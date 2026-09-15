use anyhow::Result;
use crate::services::skills::loader::discover_skills;

pub async fn get_skill_catalog() -> String {
    let skills = discover_skills().await;
    let filtered: Vec<_> = skills
        .iter()
        .filter(|s| s.name != "using-superpowers")
        .collect();
    if filtered.is_empty() {
        return "No skills installed. Use `pi skill add <skill>` to install to ~/.agents/skills."
            .to_string();
    }
    filtered
        .iter()
        .map(|s| {
            format!(
                "- {}: {} (path: {})",
                s.name,
                s.description,
                s.path.display()
            )
        })
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
            anyhow::bail!("Skill not found: {}. Available: {}", name, avail.join(", "));
        }
    }
}
