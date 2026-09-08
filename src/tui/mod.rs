use crate::agent::{self, AgentEvent};
use crate::theme::{ASHEN, THEME};
use crossterm::event::{self, Event, KeyCode, KeyModifiers, MouseEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use std::io::Stdout;

mod markdown;

pub async fn run(model: String) -> anyhow::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let res = app_loop(&mut terminal, model).await;

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("TUI error: {}", e);
    }
    Ok(())
}

// ── Message model ──────────────────────────────────────────────

struct Msg {
    role: String,
    content: String,
}

impl Msg {
    /// Render this message as styled ratatui Lines.
    fn render_lines(&self) -> Vec<Line<'static>> {
        match self.role.as_str() {
            "user" => {
                let mut lines = vec![Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(">>", Style::default()
                        .fg(ASHEN.slate)
                        .add_modifier(Modifier::BOLD)),
                    Span::styled(" you", Style::default()
                        .fg(ASHEN.slate)
                        .add_modifier(Modifier::BOLD)),
                ])];
                for l in self.content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("     {}", l),
                        Style::default().fg(ASHEN.bone),
                    )));
                }
                lines
            }
            "assistant" => {
                let mut lines = vec![Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled("lean", Style::default()
                        .fg(ASHEN.moss)
                        .add_modifier(Modifier::BOLD)),
                ])];
                lines.extend(markdown::render_markdown(&self.content, 5));
                lines
            }
            "thinking" => {
                // Render thinking as a single collapsed block.
                // If there's content, show first line as preview.
                let first = self.content.lines().next().unwrap_or("");
                if first.is_empty() {
                    vec![Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled("...", Style::default()
                            .fg(ASHEN.deep_ash)
                            .add_modifier(Modifier::ITALIC)),
                    ])]
                } else {
                    let preview: String = first.chars().take(80).collect();
                    vec![Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled("...", Style::default()
                            .fg(ASHEN.deep_ash)
                            .add_modifier(Modifier::ITALIC)),
                        Span::styled(format!(" {}", preview), Style::default()
                            .fg(ASHEN.deep_ash)
                            .add_modifier(Modifier::ITALIC)),
                    ])]
                }
            }
            "tool" => self.render_tool_lines(),
            "system" => self
                .content
                .lines()
                .map(|l| {
                    Line::from(Span::styled(
                        format!("   {}", l),
                        Style::default().fg(ASHEN.ember),
                    ))
                })
                .collect(),
            _ => self
                .content
                .lines()
                .map(|l| {
                    Line::from(Span::styled(
                        l.to_string(),
                        Style::default().fg(ASHEN.frost),
                    ))
                })
                .collect(),
        }
    }

    /// Render tool messages compactly.
    fn render_tool_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if let Some(pos) = self.content.find(" → ") {
            // Tool result: "tool_name → result"
            let name = self.content[..pos].trim();
            let result = &self.content[pos + " → ".len()..];

            // Truncate long results
            let result_lines: Vec<&str> = result.lines().collect();
            let max_lines = 3;
            let show = result_lines.len().min(max_lines);

            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled("▸", Style::default().fg(ASHEN.ember)),
                Span::styled(", ", Style::default().fg(ASHEN.charcoal)),
                Span::styled(name.to_string(), Style::default()
                    .fg(ASHEN.ember_glow)
                    .add_modifier(Modifier::BOLD)),
            ]));

            for l in &result_lines[..show] {
                lines.push(Line::from(Span::styled(
                    format!("      {}", l),
                    Style::default().fg(ASHEN.smoke),
                )));
            }
            if result_lines.len() > max_lines {
                lines.push(Line::from(Span::styled(
                    format!("      … +{} more lines", result_lines.len() - max_lines),
                    Style::default().fg(ASHEN.charcoal),
                )));
            }
        } else {
            // Tool invocation: "tool_name {json}"
            let first_space = self.content.find(' ');
            if let Some(pos) = first_space {
                let name = &self.content[..pos];
                let args_str = self.content[pos..].trim();

                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled("▸", Style::default().fg(ASHEN.ember)),
                    Span::styled(", ", Style::default().fg(ASHEN.charcoal)),
                    Span::styled(name.to_string(), Style::default()
                        .fg(ASHEN.ember_glow)
                        .add_modifier(Modifier::BOLD)),
                ]));

                if let Ok(v) = serde_json::from_str::<serde_json::Value>(args_str) {
                    if let Some(path) = v.get("path").and_then(|p| p.as_str()) {
                        let home = std::env::var("HOME").unwrap_or_default();
                        let display = if let Some(rest) = path.strip_prefix(&home) {
                            format!("~{}", rest)
                        } else {
                            path.to_string()
                        };
                        lines.push(Line::from(Span::styled(
                            format!("      {}", display),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    } else if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
                        lines.push(Line::from(Span::styled(
                            format!("      $ {}", cmd),
                            Style::default().fg(ASHEN.whisper),
                        )));
                    } else if let Some(q) = v.get("query").and_then(|q| q.as_str()) {
                        lines.push(Line::from(Span::styled(
                            format!("      \"{}\"", q),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("      {}", args_str),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    }
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("      {}", args_str),
                        Style::default().fg(ASHEN.smoke),
                    )));
                }
            }
        }

        if lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines
    }
}

/// Merge all consecutive/adjacent thinking messages into one.
/// This fixes fragmented thinking from interleaved reasoning + tool events.
fn merge_thinking(messages: &[Msg]) -> Vec<Msg> {
    let mut out: Vec<Msg> = Vec::new();
    let mut thinking_buf = String::new();
    let mut in_thinking = false;

    for m in messages {
        if m.role == "thinking" {
            if in_thinking {
                thinking_buf.push_str(&m.content);
            } else {
                in_thinking = true;
                thinking_buf = m.content.clone();
            }
        } else {
            if in_thinking {
                // Flush accumulated thinking
                out.push(Msg {
                    role: "thinking".into(),
                    content: thinking_buf.clone(),
                });
                thinking_buf.clear();
                in_thinking = false;
            }
            out.push(Msg {
                role: m.role.clone(),
                content: m.content.clone(),
            });
        }
    }
    // Flush trailing thinking
    if in_thinking && !thinking_buf.is_empty() {
        out.push(Msg {
            role: "thinking".into(),
            content: thinking_buf,
        });
    }
    out
}


// ── Rendering helpers ──────────────────────────────────────────

fn draw_header(f: &mut Frame, area: Rect, model: &str, step_info: &str) {
    let inner = area;
    let width = inner.width as usize;

    // Left: app name
    let left = " lean ";
    // Right: model + step
    let right = if step_info.is_empty() {
        format!(" {} ", model)
    } else {
        format!(" {} · {} ", model, step_info)
    };
    let gap = width.saturating_sub(left.len() + right.len());
    let filler = " ".repeat(gap);
    let title = format!("{}{}{}", left, filler, right);

    let header = Paragraph::new(Line::from(Span::styled(
        title,
        Style::default()
            .fg(ASHEN.deep_ash)
            .bg(THEME.header_bg)
            .add_modifier(Modifier::BOLD),
    )));
    f.render_widget(header, inner);
}

fn draw_queue(f: &mut Frame, area: Rect, queue: &[String]) {
    let max_show = queue.len().min(2);
    let start = queue.len().saturating_sub(max_show);
    let visible = &queue[start..];
    let width = area.width as usize;

    let lines: Vec<Line<'static>> = visible
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            let n = start + i + 1;
            let prefix = format!("  {:>2} queued  ", n);
            let display: String = msg.chars().take(width.saturating_sub(prefix.chars().count())).collect();
            Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default()
                        .fg(ASHEN.charcoal)
                        .bg(THEME.input_bg),
                ),
                Span::styled(
                    display,
                    Style::default()
                        .fg(ASHEN.deep_ash)
                        .bg(THEME.input_bg),
                ),
            ])
        })
        .collect();

    let para = Paragraph::new(lines).style(Style::default().bg(THEME.input_bg));
    f.render_widget(para, area);
}

fn draw_separator(f: &mut Frame, area: Rect) {
    let sep = Paragraph::new(Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(THEME.separator).bg(THEME.page_bg),
    )));
    f.render_widget(sep, area);
}

/// Wrap lines to fit within a given width.
fn wrap_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return lines;
    }
    let mut out = Vec::new();
    for line in lines {
        // Calculate total text width of this line
        let total: usize = line
            .spans
            .iter()
            .map(|s| s.content.chars().count())
            .sum();
        if total <= width {
            out.push(line);
            continue;
        }
        // Split into segments that fit
        let mut remaining = width;
        let mut seg_spans: Vec<Span<'static>> = Vec::new();
        for span in &line.spans {
            let chars: Vec<char> = span.content.chars().collect();
            let mut pos = 0;
            while pos < chars.len() && remaining > 0 {
                let take = chars.len().min(pos + remaining);
                let chunk: String = chars[pos..take].iter().collect();
                seg_spans.push(Span::styled(chunk, span.style));
                remaining -= take - pos;
                pos = take;
                if remaining == 0 {
                    out.push(Line::from(seg_spans));
                    seg_spans = Vec::new();
                    remaining = width;
                }
            }
        }
        if !seg_spans.is_empty() {
            out.push(Line::from(seg_spans));
        }
    }
    out
}

/// Build the full list of styled lines from messages (without wrapping).
fn build_content_lines(messages: &[Msg]) -> Vec<Line<'static>> {
    let mut all_lines: Vec<Line<'static>> = Vec::new();
    let merged = merge_thinking(messages);

    for (i, m) in merged.iter().enumerate() {
        // Add a thin separator between tool blocks and other messages
        if i > 0
            && (m.role == "tool" || merged[i - 1].role == "tool")
            && m.role != merged[i - 1].role
        {
            all_lines.push(Line::from(Span::styled(
                format!("   {}", "─".repeat(6)),
                Style::default().fg(ASHEN.charcoal),
            )));
        }
        all_lines.extend(m.render_lines());
        // Blank line between messages (but not after the last one)
        if i + 1 < merged.len() {
            all_lines.push(Line::from(""));
        }
    }

    // If empty, show a welcome line
    if messages.is_empty() {
        all_lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "    lean",
                Style::default()
                    .fg(ASHEN.moss)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "    light coding assistant",
                Style::default().fg(ASHEN.deep_ash),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "    type a message below. /help for commands.",
                Style::default().fg(ASHEN.deep_ash),
            )),
            Line::from(""),
        ];
    }

    all_lines
}


fn draw_input(f: &mut Frame, area: Rect, input_text: &str) {
    let width = area.width as usize;
    let prompt = " ▸ ";
    let available = width.saturating_sub(prompt.len());

    let display_input = if input_text.is_empty() {
        // Show placeholder with dim style
        Span::styled(
            format!("{}{}", prompt, " ".repeat(available)),
            Style::default().fg(ASHEN.charcoal).bg(THEME.input_bg),
        )
    } else if input_text.chars().count() >= available {
        // Truncate from left, show cursor at end
        let char_count = input_text.chars().count();
        let skip = char_count - available + 1;
        let truncated: String = input_text.chars().skip(skip).collect();
        Span::styled(
            format!("▸{}█", truncated),
            Style::default().fg(ASHEN.bone).bg(THEME.input_bg),
        )
    } else {
        Span::styled(
            format!("{}{}█", prompt, input_text),
            Style::default().fg(ASHEN.bone).bg(THEME.input_bg),
        )
    };

    let line = Line::from(display_input);

    let para = Paragraph::new(line).style(Style::default().bg(THEME.input_bg));
    f.render_widget(para, area);
}

fn draw_footer(f: &mut Frame, area: Rect, model: &str, msg_count: usize, cwd: &str, agent_busy: bool, spinner_tick: usize) {
    let width = area.width as usize;

    let spinner = ["\u{280b}", "\u{2819}", "\u{2813}", "\u{2827}", "\u{2836}", "\u{2834}", "\u{2826}", "\u{282e}"];
    let left = if agent_busy {
        let ch = spinner[spinner_tick % spinner.len()];
        format!(" {} {} ", ch, model)
    } else {
        format!(" {} ", model)
    };
    let right = format!(" {} msgs ", msg_count);

    // Shorten cwd to show last 2 components
    let short_cwd = {
        let parts: Vec<&str> = cwd.rsplit('/').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>();
        if parts.len() >= 2 {
            format!("{}/{}", parts[0], parts[1])
        } else {
            parts.join("/")
        }
    };
    let center = format!(" {} ", short_cwd);

    let used = left.len() + center.len() + right.len();
    let gap = if used < width {
        width - used
    } else {
        0
    };
    let gap_left = gap / 2;
    let gap_right = gap - gap_left;

    let footer_text = format!(
        "{}{}{}{}{}",
        left,
        " ".repeat(gap_left),
        center,
        " ".repeat(gap_right),
        right,
    );

    let footer = Paragraph::new(Line::from(Span::styled(
        footer_text,
        Style::default().fg(ASHEN.charcoal).bg(THEME.page_bg),
    )));
    f.render_widget(footer, area);
}

// ── Autocomplete ─────────────────────────────────────────────

const COMMANDS: &[&str] = &["/help", "/clear", "/exit", "/quit", "/model"];

/// Filter commands matching the current input prefix.
fn autocomplete_matches(input: &str) -> Vec<&'static str> {
    if input.is_empty() || !input.starts_with('/') {
        return Vec::new();
    }
    COMMANDS
        .iter()
        .copied()
        .filter(|cmd| cmd.starts_with(input))
        .collect()
}

/// Draw a scrollable autocomplete popup above the input area.
fn draw_autocomplete(
    f: &mut Frame,
    input_area: Rect,
    matches: &[&str],
    selected: usize,
    scroll_offset: usize,
) {
    if matches.is_empty() {
        return;
    }

    let max_cmd_len = matches.iter().map(|c| c.len()).max().unwrap_or(10);
    let popup_width = (max_cmd_len + 4) as u16;

    // Cap height to available space above input
    let max_visible = input_area.y.saturating_sub(1) as usize; // leave 1 row gap
    let visible_items = matches.len().min(max_visible.max(1));
    let popup_height = (visible_items + 2) as u16; // +2 for borders

    let x = input_area.x;
    let y = input_area.y.saturating_sub(popup_height);

    let area = Rect {
        x,
        y,
        width: popup_width.min(input_area.width),
        height: popup_height,
    };

    if area.height < 2 || area.width < 4 {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ASHEN.charcoal))
        .style(Style::default().bg(THEME.header_bg));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Render only visible items
    let visible_h = inner.height as usize;
    let start = scroll_offset;
    let end = (start + visible_h).min(matches.len());

    let items: Vec<Line<'static>> = matches[start..end]
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let real_idx = start + i;
            let style = if real_idx == selected {
                Style::default()
                    .fg(ASHEN.bone)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ASHEN.smoke)
            };
            Line::from(Span::styled(format!(" {} ", cmd), style))
        })
        .collect();

    let para = Paragraph::new(items);
    f.render_widget(para, inner);
}

// ── App loop ───────────────────────────────────────────────────

async fn app_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<Stdout>>,
    model: String,
) -> anyhow::Result<()> {
    let mut messages: Vec<Msg> = Vec::new();
    let mut scroll: u16 = 0;
    let mut auto_scroll = true;
    let mut step_info = String::new();
    let mut cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    // input history
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut input_text = String::new();

    // autocomplete
    let mut ac_matches: Vec<&str> = Vec::new();
    let mut ac_idx: usize = 0;
    let mut ac_scroll: usize = 0;

    // message queue
    let mut msg_queue: Vec<String> = Vec::new();
    let mut agent_busy = false;
    let mut spinner_tick: usize = 0;

    loop {
        let term_size = terminal.size()?;
        let queue_rows = msg_queue.len().min(2) as u16;
        let overhead = 5 + queue_rows; // header + sep + sep + queue + input + footer
        // Content area: starts at row 2 (after header + sep), height is the rest
        let content_area = Rect {
            x: 0,
            y: 2,
            width: term_size.width,
            height: term_size.height.saturating_sub(overhead).max(1),
        };

        // Pre-compute layout so we know content area dimensions
        let chunks = {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // header
                    Constraint::Length(1), // separator
                    Constraint::Min(1),   // content
                    Constraint::Length(1), // separator
                    Constraint::Length(queue_rows), // queue (0-2)
                    Constraint::Length(1), // input
                    Constraint::Length(1), // footer
                ])
                .split(Rect {
                    x: 0,
                    y: 0,
                    width: term_size.width,
                    height: term_size.height,
                });
            chunks
        };
        let content_height = chunks[2].height as usize;

        // Build content lines once, wrap once, use for both counting and rendering
        let content_lines = build_content_lines(&messages);
        let wrapped_content = wrap_lines(content_lines, chunks[2].width as usize);
        let total_lines = wrapped_content.len();
        if auto_scroll {
            scroll = total_lines.saturating_sub(content_height) as u16;
        }

        // Single render pass with correct scroll
        terminal.draw(|f| {
            let bg_block = Block::default().style(Style::default().bg(THEME.page_bg));
            f.render_widget(bg_block, f.area());

            // Header
            draw_header(f, chunks[0], &model, &step_info);

            // Separator
            draw_separator(f, chunks[1]);

            // Content — use pre-wrapped lines
            let para = Paragraph::new(wrapped_content.clone()).scroll((scroll, 0));
            f.render_widget(para, chunks[2]);

            // Separator
            draw_separator(f, chunks[3]);

            // Queue
            draw_queue(f, chunks[4], &msg_queue);

            // Input
            draw_input(f, chunks[5], &input_text);

            // Autocomplete popup
            if !ac_matches.is_empty() {
                draw_autocomplete(f, chunks[5], &ac_matches, ac_idx, ac_scroll);
            }

            // Footer
            draw_footer(f, chunks[6], &model, messages.len(), &cwd, agent_busy, spinner_tick);
        })?;

        // Handle keyboard and mouse events
        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Mouse(m) => {
                    let in_content = m.row >= content_area.y
                        && m.row < content_area.y + content_area.height;
                    if in_content {
                        match m.kind {
                            MouseEventKind::ScrollUp => {
                                scroll = scroll.saturating_sub(3);
                                auto_scroll = false;
                            }
                            MouseEventKind::ScrollDown => {
                                let max_scroll = total_lines.saturating_sub(content_height) as u16;
                                scroll = (scroll + 3).min(max_scroll);
                                if scroll >= max_scroll {
                                    auto_scroll = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::Key(k) => {
                match k.code {
                    KeyCode::Esc => {
                        if !ac_matches.is_empty() {
                            ac_matches.clear();
                            ac_idx = 0;
                        }
                    }
                    KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Enter if k.modifiers.contains(KeyModifiers::SHIFT) => {
                        input_text.push('\n');
                        ac_matches = autocomplete_matches(&input_text);
                        ac_idx = 0;
                    }
                    KeyCode::Enter => {
                        // If autocomplete is visible, complete first
                        if !ac_matches.is_empty() {
                            input_text = ac_matches[ac_idx].to_string();
                            ac_matches.clear();
                            ac_idx = 0;
                        }
                        let prompt = input_text.trim().to_string();
                        if prompt.is_empty() {
                            continue;
                        }
                        ac_matches.clear();
                        ac_idx = 0;
                        if prompt.starts_with('/') {
                            match prompt.as_str() {
                                "/exit" | "/quit" => break,
                                "/clear" => {
                                    messages.clear();
                                    scroll = 0;
                                    auto_scroll = true;
                                    step_info.clear();
                                    msg_queue.clear();
                                    agent_busy = false;
                                }
                                "/help" => {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: "/help /model <name> /clear /exit  ·  Enter send · Shift+Enter newline · Up/Down history · PgUp/PgDn scroll".into(),
                                    });
                                }
                                _ if prompt.starts_with("/model ") => {
                                    let m = prompt.strip_prefix("/model ").unwrap().trim();
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: format!("model: {} (restart to apply)", m),
                                    });
                                }
                                _ => {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: format!("unknown command: {}", prompt),
                                    });
                                }
                            }
                            input_text.clear();
                            hist_idx = None;
                            continue;
                        }
                        // Send user message
                        history.push(prompt.clone());
                        hist_idx = None;
                        step_info = "...".into();
                        input_text.clear();
                        auto_scroll = true;

                        if agent_busy {
                            // Queue for later — don't add to messages yet
                            msg_queue.push(prompt);
                        } else {
                            // Send immediately
                            messages.push(Msg {
                                role: "user".into(),
                                content: prompt.clone(),
                            });
                            agent_busy = true;
                            let tx_clone = tx.clone();
                            let model_clone = model.clone();
                            tokio::spawn(async move {
                                let stream = agent::run_agent(prompt, model_clone, 100);
                                use futures::StreamExt;
                                let mut s = Box::pin(stream);
                                while let Some(ev) = s.next().await {
                                    let _ = tx_clone.send(ev);
                                }
                            });
                        }
                    }
                    KeyCode::Up => {
                        if !ac_matches.is_empty() {
                            ac_idx = if ac_idx == 0 { ac_matches.len() - 1 } else { ac_idx - 1 };
                        } else if !history.is_empty() {
                            let idx = hist_idx
                                .map(|i| if i == 0 { 0 } else { i - 1 })
                                .unwrap_or(history.len() - 1);
                            hist_idx = Some(idx);
                            input_text = history[idx].clone();
                        }
                    }
                    KeyCode::Down => {
                        if !ac_matches.is_empty() {
                            ac_idx = (ac_idx + 1) % ac_matches.len();
                        } else if let Some(idx) = hist_idx {
                            if idx + 1 < history.len() {
                                hist_idx = Some(idx + 1);
                                input_text = history[idx + 1].clone();
                            } else {
                                hist_idx = None;
                                input_text.clear();
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        input_text.push(c);
                        ac_matches = autocomplete_matches(&input_text);
                        ac_idx = 0;
                    }
                    KeyCode::Backspace => {
                        input_text.pop();
                        ac_matches = autocomplete_matches(&input_text);
                        ac_idx = 0;
                    }
                    KeyCode::Tab => {
                        if !ac_matches.is_empty() {
                            input_text = ac_matches[ac_idx].to_string();
                            ac_matches.clear();
                            ac_idx = 0;
                        }
                    }
                    KeyCode::PageUp => {
                        scroll = scroll.saturating_sub(content_height as u16);
                        auto_scroll = false;
                    }
                    KeyCode::PageDown => {
                        let max_scroll = total_lines.saturating_sub(content_height) as u16;
                        scroll = (scroll + content_height as u16).min(max_scroll);
                        if scroll >= max_scroll {
                            auto_scroll = true;
                        }
                    }
                    KeyCode::Home => {
                        scroll = 0;
                        auto_scroll = false;
                    }
                    KeyCode::End => {
                        auto_scroll = true;
                    }
                    _ => {}
                }
                }
                _ => {}
            }
        }

        // Poll agent events (non-blocking)
        while let Ok(ev) = rx.try_recv() {
            match ev {
                AgentEvent::Text { delta } => {
                    if delta.is_empty() {
                        continue;
                    }
                    if let Some(last) = messages.last_mut() {
                        if last.role == "assistant" {
                            last.content.push_str(&delta);
                        } else {
                            messages.push(Msg {
                                role: "assistant".into(),
                                content: delta,
                            });
                        }
                    } else {
                        messages.push(Msg {
                            role: "assistant".into(),
                            content: delta,
                        });
                    }
                    step_info = "".into();
                }
                AgentEvent::Reasoning { delta } => {
                    if let Some(last) = messages.last_mut() {
                        if last.role == "thinking" {
                            last.content.push_str(&delta);
                        } else {
                            messages.push(Msg {
                                role: "thinking".into(),
                                content: delta,
                            });
                        }
                    } else {
                        messages.push(Msg {
                            role: "thinking".into(),
                            content: delta,
                        });
                    }
                }
                AgentEvent::TextDone { text } => {
                    let _ = text;
                }
                AgentEvent::ToolStart { name, args, id: _ } => {
                    messages.push(Msg {
                        role: "tool".into(),
                        content: format!(
                            "{} {}",
                            name,
                            serde_json::to_string(&args).unwrap_or_default()
                        ),
                    });
                }
                AgentEvent::ToolResult { name, result, id: _ } => {
                    let truncated: String = result.chars().take(2000).collect();
                    let display = if result.chars().count() > 2000 {
                        format!(
                            "{}… [truncated {} chars]",
                            truncated,
                            result.chars().count() - 2000
                        )
                    } else {
                        result
                    };
                    messages.push(Msg {
                        role: "tool".into(),
                        content: format!("{} → {}", name, display),
                    });
                    if name == "bash" {
                        if let Ok(new_cwd) = std::env::current_dir() {
                            cwd = new_cwd.display().to_string();
                        }
                    }
                }
                AgentEvent::Step { n } => {
                    step_info = format!("step {}", n);
                }
                AgentEvent::Done { text } => {
                    step_info = "done".into();
                    let _ = text;

                    // Send next queued message if any
                    if !msg_queue.is_empty() {
                        let next = msg_queue.remove(0);
                        messages.push(Msg {
                            role: "user".into(),
                            content: next.clone(),
                        });
                        step_info = "...".into();
                        let tx_clone = tx.clone();
                        let model_clone = model.clone();
                        tokio::spawn(async move {
                            let stream = agent::run_agent(next, model_clone, 100);
                            use futures::StreamExt;
                            let mut s = Box::pin(stream);
                            while let Some(ev) = s.next().await {
                                let _ = tx_clone.send(ev);
                            }
                        });
                    } else {
                        agent_busy = false;
                    }
                }
            }
            auto_scroll = true;
        }

        // Advance spinner
        spinner_tick = spinner_tick.wrapping_add(1);
    }

    Ok(())
}
