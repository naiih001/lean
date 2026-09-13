//! Voice dictation for lean — parity with PI `dictate` extension.
//! Toggle: Alt+M, Cancel: Alt+N. Backend: ffmpeg (pulse) → WAV → Groq Whisper.

use std::process::Stdio;

pub const GROQ_API_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
pub const GROQ_MODEL: &str = "whisper-large-v3-turbo";
pub const SAMPLE_RATE: u32 = 16000;
pub const CHANNELS: u16 = 1;
pub const BITS_PER_SAMPLE: u16 = 16;

pub const METER_CELLS: usize = 6;
pub const METER_FLOOR_DB: f64 = -50.0;
pub const METER_CEILING_DB: f64 = -10.0;
pub const PEAK_BLOCKS: &[&str] = &["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Recording,
    Transcribing,
}

/// Compute normalized RMS (0..1) over a buffer of signed 16-bit little-endian PCM.
pub fn rms_from_pcm16(buf: &[u8]) -> f32 {
    let sample_count = buf.len() / 2;
    if sample_count == 0 {
        return 0.0;
    }
    let mut sum_squares: f64 = 0.0;
    for i in 0..sample_count {
        let off = i * 2;
        let s = i16::from_le_bytes([buf[off], buf[off + 1]]) as f64;
        sum_squares += s * s;
    }
    let rms = (sum_squares / sample_count as f64).sqrt() / 32768.0;
    rms as f32
}

/// Map a normalized RMS value to one of PEAK_BLOCKS via dB.
pub fn rms_to_block(rms: f32) -> String {
    if rms <= 0.0 {
        return PEAK_BLOCKS[0].to_string();
    }
    let db = 20.0 * (rms as f64).log10();
    let t = ((db - METER_FLOOR_DB) / (METER_CEILING_DB - METER_FLOOR_DB)).clamp(0.0, 1.0);
    let idx = (t * (PEAK_BLOCKS.len() - 1) as f64).floor() as usize;
    PEAK_BLOCKS[idx].to_string()
}

/// Render the 6-cell meter ring as a string.
pub fn meter_string(meter: &[f32]) -> String {
    meter
        .iter()
        .map(|v| rms_to_block(*v))
        .collect::<Vec<_>>()
        .join("")
}

/// Build a WAV file (44-byte header + PCM data) from raw chunks.
pub fn build_wav(chunks: &[Vec<u8>]) -> Vec<u8> {
    let pcm_len: usize = chunks.iter().map(|c| c.len()).sum();
    let byte_rate = SAMPLE_RATE * CHANNELS as u32 * (BITS_PER_SAMPLE as u32 / 8);
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);
    let mut header = vec![0u8; 44];
    // RIFF
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + pcm_len as u32).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    // fmt
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&CHANNELS.to_le_bytes());
    header[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    // data
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&(pcm_len as u32).to_le_bytes());

    let mut out = Vec::with_capacity(44 + pcm_len);
    out.extend_from_slice(&header);
    for c in chunks {
        out.extend_from_slice(c);
    }
    out
}

/// Send WAV bytes to Groq Whisper and return transcript.
pub async fn transcribe_with_groq(wav: Vec<u8>) -> Result<String, String> {
    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY not set in environment".to_string())?;
    if api_key.trim().is_empty() {
        return Err("GROQ_API_KEY is empty".to_string());
    }
    let client = reqwest::Client::new();
    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", GROQ_MODEL)
        .text("language", "en")
        .text("response_format", "json");

    let resp = client
        .post(GROQ_API_URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Groq request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Groq API {}: {}", status, body));
    }
    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Groq parse failed: {}", e))?;
    let text = data
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(text)
}

/// Attempt to copy text to clipboard via wl-copy / xclip / xsel / pbcopy.
/// Returns true if any succeeded.
pub fn try_copy_clipboard(text: &str) -> bool {
    // wl-copy (Wayland)
    if try_clipboard_cmd("wl-copy", text) {
        return true;
    }
    if try_clipboard_cmd("xclip", text) {
        return true;
    }
    if try_clipboard_cmd("xsel", text) {
        return true;
    }
    if try_clipboard_cmd("pbcopy", text) {
        return true;
    }
    false
}

fn try_clipboard_cmd(cmd: &str, text: &str) -> bool {
    let args: &[&str] = match cmd {
        "xclip" => &["-selection", "clipboard"],
        "xsel" => &["--clipboard", "--input"],
        _ => &[],
    };
    let Ok(mut child) = std::process::Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(text.as_bytes());
        // stdin dropped here closes pipe
    }
    match child.wait() {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

/// Spawn ffmpeg capturing pulse default 16kHz mono s16le to stdout.
/// Returns the child process (stdout piped).
pub fn spawn_ffmpeg() -> Result<tokio::process::Child, String> {
    let child = tokio::process::Command::new("ffmpeg")
        .args([
            "-f", "pulse", "-i", "default", "-ar", "16000", "-ac", "1", "-f", "s16le", "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn 'ffmpeg' (is it installed?): {}", e))?;
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_silence_is_zero() {
        let buf = vec![0u8; 100];
        assert_eq!(rms_from_pcm16(&buf), 0.0);
    }

    #[test]
    fn rms_max_maps_to_top_block() {
        // max i16 = 32767 -> rms ~1.0 -> top block
        let mut buf = Vec::new();
        for _ in 0..10 {
            buf.extend_from_slice(&32767i16.to_le_bytes());
        }
        let rms = rms_from_pcm16(&buf);
        assert!(rms > 0.9);
        assert_eq!(rms_to_block(rms), "█");
    }

    #[test]
    fn build_wav_has_correct_header() {
        let chunks = vec![vec![0u8; 100], vec![1u8; 50]];
        let wav = build_wav(&chunks);
        assert_eq!(wav.len(), 44 + 150);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }
}
