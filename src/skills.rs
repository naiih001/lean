use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dirs::home_dir;
use walkdir::WalkDir;

const CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub content: String,
}

// Cache state
static CACHE: OnceLock<std::sync::Mutex<Option<(Vec<Skill>, Instant)>>> = OnceLock::new();

fn cache_lock() -> &'static std::sync::Mutex<Option<(Vec<Skill>, Instant)>> {
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

fn parse_frontmatter(raw: &str) -> (Option<String>, Option<String>, String) {
    if !raw.starts_with("---") {
        return (None, None, raw.to_string());
    }
    let end = raw[3..].find("\n---").map(|i| i + 3);
    let end = match end {
        Some(i) => i,
        None => return (None, None, raw.to_string()),
    };
    let fm = &raw[3..end].trim();
    let body = raw[end + 4..].trim_start();

    let mut name = None;
    let mut description = None;
    for line in fm.lines() {
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val = val.trim();
            let val = val
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| val.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(val)
                .trim();
            match key {
                "name" => name = Some(val.to_string()),
                "description" => description = Some(val.to_string()),
                _ => {}
            }
        }
    }
    (name, description, body.to_string())
}

async fn scan_dir(base: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    let read_dir = match tokio::fs::read_dir(base).await {
        Ok(rd) => rd,
        Err(_) => return skills,
    };
    let mut entries = read_dir;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let file_type = match entry.file_type().await {
            Ok(ft) => ft,
            Err(_) => {
                if let Ok(meta) = std::fs::metadata(entry.path()) {
                    meta.file_type()
                } else {
                    continue;
                }
            }
        };
        if !file_type.is_dir() {
            continue;
        }
        let skill_path = entry.path().join("SKILL.md");
        if !skill_path.exists() {
            continue;
        }
        let raw = match tokio::fs::read_to_string(&skill_path).await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let (name_opt, description_opt, body) = parse_frontmatter(&raw);
        let skill_name = name_opt.unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
        let desc = description_opt.unwrap_or_else(|| {
            body.lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .chars()
                .take(140)
                .collect()
        });
        skills.push(Skill {
            name: skill_name,
            description: desc,
            path: skill_path,
            content: raw,
        });
    }
    skills
}

pub async fn discover_skills() -> Vec<Skill> {
    {
        let mut lock = cache_lock().lock().unwrap();
        if let Some((cached, time)) = lock.as_ref() {
            if time.elapsed() < CACHE_TTL {
                return cached.clone();
            }
        }
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = home_dir().unwrap_or_else(|| PathBuf::from("~"));

    let bases = vec![
        cwd.join("skills"),
        cwd.join(".lean").join("skills"),
        home.join(".agents").join("skills"),
    ];

    let mut all: HashMap<String, Skill> = HashMap::new();
    for base in &bases {
        let found = scan_dir(base).await;
        let is_local = base.starts_with(&cwd);
        for skill in found {
            if !all.contains_key(&skill.name) || is_local {
                all.insert(skill.name.clone(), skill);
            }
        }
    }
    let mut result: Vec<Skill> = all.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));

    {
        let mut lock = cache_lock().lock().unwrap();
        *lock = Some((result.clone(), Instant::now()));
    }

    result
}

pub async fn get_skill_catalog() -> String {
    let skills = discover_skills().await;
    // Filter pi-internal meta-skills that should not auto-trigger for every prompt
    let filtered: Vec<&Skill> = skills.iter().filter(|s| s.name != "using-superpowers").collect();
    if filtered.is_empty() {
        return "No skills installed. Use `pi skill add <skill>` to install to ~/.agents/skills.".to_string();
    }
    filtered
        .iter()
        .map(|s| format!("- {}: {} (path: {})", s.name, s.description, s.path.display()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn load_skill(name: &str) -> Result<String> {
    let skills = discover_skills().await;
    let found = skills.iter().find(|s| s.name == name);
    match found {
        Some(skill) => Ok(skill.content.clone()),
        None => {
            let avail: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            anyhow::bail!(
                "Skill not found: {}. Available: {}",
                name,
                avail.join(", ")
            );
        }
    }
}