//! Skills loader for agent capabilities.

use std::path::{Path, PathBuf};
use tracing::debug;

/// A loaded skill.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub path: PathBuf,
    pub mode: String,      // "always", "available", etc.
    pub category: String,
    pub description: String,
    pub available: bool,
}

/// Loads and manages agent skills from workspace/skills/.
pub struct SkillsLoader {
    skills_dir: PathBuf,
    skills: Vec<Skill>,
}

impl SkillsLoader {
    pub fn new(workspace: &Path) -> Self {
        let skills_dir = workspace.join("skills");
        let skills = Self::scan_skills(&skills_dir);
        Self { skills_dir, skills }
    }

    fn scan_skills(skills_dir: &Path) -> Vec<Skill> {
        if !skills_dir.exists() {
            return Vec::new();
        }

        let mut skills = Vec::new();
        let entries = std::fs::read_dir(skills_dir).ok();
        if let Some(entries) = entries {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let skill_md = path.join("SKILL.md");
                if !skill_md.exists() {
                    continue;
                }

                let content = std::fs::read_to_string(&skill_md).unwrap_or_default();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();

                let mode = extract_frontmatter(&content, "mode")
                    .unwrap_or_else(|| "available".to_string());
                let category = extract_frontmatter(&content, "category")
                    .unwrap_or_else(|| "general".to_string());
                let description = extract_frontmatter(&content, "description")
                    .unwrap_or_else(|| name.clone());
                let available = extract_frontmatter(&content, "available")
                    .map(|v| v.to_lowercase() != "false")
                    .unwrap_or(true);

                skills.push(Skill {
                    name,
                    path: skill_md,
                    mode,
                    category,
                    description,
                    available,
                });
            }
        }

        skills.sort_by(|a, b| a.name.cmp(&b.name));
        debug!("Loaded {} skills", skills.len());
        skills
    }

    /// Get skills with mode="always".
    pub fn get_always_skills(&self) -> Vec<&Skill> {
        self.skills.iter().filter(|s| s.mode == "always").collect()
    }

    /// Get all skills for the registry summary.
    pub fn build_skills_summary(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        self.skills
            .iter()
            .map(|s| {
                let avail = if s.available { "✓" } else { "✗" };
                format!("- {} [{}] {}: {}", avail, s.category, s.name, s.description)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Load the content of a skill by name.
    pub fn load_skill_content(&self, name: &str) -> Option<String> {
        self.skills
            .iter()
            .find(|s| s.name == name)
            .and_then(|s| std::fs::read_to_string(&s.path).ok())
    }

    /// Load always-mode skills for injection into the system prompt.
    pub fn load_skills_for_context(&self, skills: &[&Skill]) -> String {
        skills
            .iter()
            .filter_map(|s| std::fs::read_to_string(&s.path).ok())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Extract a value from markdown-style frontmatter or metadata comments.
/// Looks for lines like: `<!-- mode: always -->` or `mode: always`
fn extract_frontmatter(content: &str, key: &str) -> Option<String> {
    for line in content.lines().take(20) {
        let line = line.trim();
        // Handle <!-- key: value --> style
        if line.starts_with("<!--") && line.ends_with("-->") {
            let inner = &line[4..line.len() - 3].trim();
            if let Some(val) = parse_kv(inner, key) {
                return Some(val);
            }
        }
        // Handle "key: value" style
        if let Some(val) = parse_kv(line, key) {
            return Some(val);
        }
    }
    None
}

fn parse_kv(line: &str, key: &str) -> Option<String> {
    let prefix = format!("{}:", key);
    if line.to_lowercase().starts_with(&prefix.to_lowercase()) {
        let val = line[prefix.len()..].trim().to_string();
        if !val.is_empty() {
            return Some(val);
        }
    }
    None
}
