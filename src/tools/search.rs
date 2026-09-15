use std::path::Path;
use crate::tools::fs::{truncate_output, TruncateStrategy};

pub async fn grep(pattern: &str, path: Option<&str>) -> Result<String, String> {
    let base = path.unwrap_or(".");
    let base_path = Path::new(base);
    if !base_path.exists() {
        return Err(format!("Path not found: {}", base));
    }
    let mut results = Vec::new();
    let walker = walkdir::WalkDir::new(base_path)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with(".git") && name != "target" && name != "node_modules"
        });
    for entry in walker.filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                for (idx, line) in content.lines().enumerate() {
                    if line.contains(pattern) {
                        results.push(format!(
                            "{}:{}: {}",
                            entry.path().display(),
                            idx + 1,
                            line.trim()
                        ));
                        if results.len() >= 200 {
                            break;
                        }
                    }
                }
            }
            if results.len() >= 200 {
                break;
            }
        }
    }
    if results.is_empty() {
        Ok(format!("No matches for '{}' in {}", pattern, base))
    } else {
        Ok(truncate_output(&results.join("\n"), TruncateStrategy::Head))
    }
}

pub async fn find(pattern: &str, path: Option<&str>) -> Result<String, String> {
    let base = path.unwrap_or(".");
    let base_path = Path::new(base);
    if !base_path.exists() {
        return Err(format!("Path not found: {}", base));
    }
    let mut results = Vec::new();
    let walker = walkdir::WalkDir::new(base_path)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with(".git") && name != "target"
        });
    let matcher = wildmatch::WildMatch::new(pattern);
    for entry in walker.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy();
        if matcher.matches(&name) || entry.path().to_string_lossy().contains(pattern) {
            results.push(entry.path().display().to_string());
            if results.len() >= 200 {
                break;
            }
        }
    }
    if results.is_empty() {
        Ok(format!("No files matching '{}' in {}", pattern, base))
    } else {
        Ok(truncate_output(&results.join("\n"), TruncateStrategy::Head))
    }
}

pub async fn ls(path: Option<&str>) -> Result<String, String> {
    let base = path.unwrap_or(".");
    let p = Path::new(base);
    if !p.exists() {
        return Err(format!("Path not found: {}", base));
    }
    if p.is_file() {
        return Ok(p.display().to_string());
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(p).map_err(|e| format!("read dir error: {}", e))? {
        let e = entry.map_err(|e| format!("entry error: {}", e))?;
        let ft = e
            .file_type()
            .map_err(|e| format!("file type error: {}", e))?;
        let name = e.file_name().to_string_lossy().to_string();
        let suffix = if ft.is_dir() { "/" } else { "" };
        entries.push(format!("{}{}", name, suffix));
    }
    entries.sort();
    Ok(truncate_output(&entries.join("\n"), TruncateStrategy::Head))
}

