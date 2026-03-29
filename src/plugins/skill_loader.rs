use std::path::Path;

/// A loaded plugin skill with its content.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    pub name: String,
    pub description: String,
    pub trigger: String,
    pub content: String,
    pub plugin_name: String,
}

/// Load a skill markdown file from a plugin directory.
pub fn load_skill(
    plugin_dir: &Path,
    plugin_name: &str,
    skill_name: &str,
    skill_file: &str,
    description: &str,
    trigger: Option<&str>,
) -> Option<LoadedSkill> {
    // Prevent path traversal (Unix and Windows patterns)
    if skill_file.contains("..") || skill_file.starts_with('/') {
        return None;
    }
    // Block Windows absolute paths (drive letter)
    if skill_file.len() >= 2
        && skill_file.as_bytes()[0].is_ascii_alphabetic()
        && skill_file.as_bytes()[1] == b':'
    {
        return None;
    }
    // Block UNC paths
    if skill_file.starts_with("\\\\") {
        return None;
    }

    let skill_path = plugin_dir.join(skill_file);

    // Verify the resolved path is within the plugin dir
    if let (Ok(canonical_skill), Ok(canonical_plugin)) =
        (skill_path.canonicalize(), plugin_dir.canonicalize())
    {
        if !canonical_skill.starts_with(&canonical_plugin) {
            return None;
        }
    }

    let content = std::fs::read_to_string(&skill_path).ok()?;
    if content.trim().is_empty() {
        return None;
    }

    let trigger_str = trigger
        .map(|t| t.to_string())
        .unwrap_or_else(|| format!("/{skill_name}"));

    Some(LoadedSkill {
        name: skill_name.to_string(),
        description: description.to_string(),
        trigger: trigger_str,
        content,
        plugin_name: plugin_name.to_string(),
    })
}

/// Format loaded skills into a system prompt section.
pub fn format_skills_for_prompt(skills: &[LoadedSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut out = String::from("The following skills are available via slash commands:\n\n");
    for skill in skills {
        out.push_str(&format!(
            "- `{}` ({}) — {}\n",
            skill.trigger, skill.plugin_name, skill.description
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_plugin_skill_loading() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("refactor.md"),
            "# Refactoring Guide\nStep 1: identify smell",
        )
        .unwrap();

        let skill = load_skill(
            tmp.path(),
            "my-plugin",
            "refactor",
            "skills/refactor.md",
            "Guided refactoring",
            Some("/refactor"),
        );

        assert!(skill.is_some());
        let skill = skill.unwrap();
        assert_eq!(skill.name, "refactor");
        assert_eq!(skill.trigger, "/refactor");
        assert!(skill.content.contains("identify smell"));
    }

    #[test]
    fn test_skill_path_traversal_blocked() {
        let tmp = TempDir::new().unwrap();
        let skill = load_skill(
            tmp.path(),
            "evil",
            "steal",
            "../../etc/passwd",
            "evil",
            None,
        );
        assert!(skill.is_none());
    }

    #[test]
    fn test_skill_windows_traversal_blocked() {
        let tmp = TempDir::new().unwrap();
        let skill = load_skill(
            tmp.path(),
            "evil",
            "steal",
            "..\\..\\Windows\\System32\\config\\SAM",
            "evil",
            None,
        );
        assert!(skill.is_none());
    }

    #[test]
    fn test_skill_windows_absolute_blocked() {
        let tmp = TempDir::new().unwrap();
        let skill = load_skill(
            tmp.path(),
            "evil",
            "steal",
            "C:\\Windows\\System32\\config\\SAM",
            "evil",
            None,
        );
        assert!(skill.is_none());
    }

    #[test]
    fn test_skill_unc_path_blocked() {
        let tmp = TempDir::new().unwrap();
        let skill = load_skill(
            tmp.path(),
            "evil",
            "steal",
            "\\\\server\\share\\secret.md",
            "evil",
            None,
        );
        assert!(skill.is_none());
    }

    #[test]
    fn test_skill_default_trigger() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("test.md"), "test content").unwrap();

        let skill = load_skill(
            tmp.path(),
            "my-plugin",
            "test",
            "skills/test.md",
            "Test skill",
            None,
        );

        assert!(skill.is_some());
        assert_eq!(skill.unwrap().trigger, "/test");
    }

    #[test]
    fn test_format_skills_for_prompt() {
        let skills = vec![
            LoadedSkill {
                name: "refactor".to_string(),
                description: "Refactoring guide".to_string(),
                trigger: "/refactor".to_string(),
                content: "content".to_string(),
                plugin_name: "my-plugin".to_string(),
            },
        ];
        let formatted = format_skills_for_prompt(&skills);
        assert!(formatted.contains("/refactor"));
        assert!(formatted.contains("my-plugin"));
    }
}
