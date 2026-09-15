pub mod catalog;
pub mod loader;

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub content: String,
}

pub use catalog::{get_skill_catalog, load_skill};
pub use loader::discover_skills;
