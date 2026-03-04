use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Default commit template — conventional commits style
const DEFAULT_COMMIT: &str = r#"Format commit messages using conventional commits:

<type>(<scope>): <subject>

<body>

Types: feat, fix, refactor, docs, test, chore, perf, ci, style, build
- Subject: imperative mood, lowercase, no period, max 72 chars
- Body: optional, wrap at 80 chars, explain WHY not WHAT
- Scope: optional, the area of the codebase affected

Examples:
  feat(auth): add OAuth2 login flow
  fix(parser): handle empty input without panic
  refactor(config): simplify merge logic
"#;

/// Default PR template — structured sections
const DEFAULT_PR: &str = r#"Format pull request descriptions with these sections:

## What changed
[2-3 sentences explaining what this does and why]

## Files
**New:**
- `FileName` — what it does

**Changed:**
- `FileName` — what changed

## Tests
[X] total — [Y] passing, [Z] failing

**What was tested:**
- [list of areas covered]

**Recommended follow-up tests:**
- [ ] [manual or edge case tests worth running]

## Notes
[gotchas, permissions, limitations, anything worth calling out]
"#;

/// Default comment style — concise, explain-why
const DEFAULT_COMMENTS: &str = r#"When writing code comments:
- Explain WHY, not WHAT — the code shows what, comments explain intent
- Keep inline comments short (1 line preferred)
- Use doc comments (/// or /**) for public APIs with param/return descriptions
- Don't comment obvious code — trust the reader
- Mark temporary workarounds with TODO(reason)
"#;

/// Default chat style — direct, no filler
const DEFAULT_CHAT: &str = r#"When responding to the user:
- Be direct and concise — no filler phrases like "Sure!", "Great question!", "Let me..."
- Use code blocks with language tags for all code snippets
- When showing file changes, reference the file path
- Lead with the answer, then explain if needed
- Use bullet points for lists, not paragraphs
"#;

/// Configuration for the formatting system (lives in config.toml under [formatting])
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FormattingConfig {
    /// Custom templates directory (overrides default lookup paths)
    pub templates_dir: Option<String>,
    /// Inline commit format override (highest precedence)
    pub commit_format: Option<String>,
    /// Inline PR format override
    pub pr_format: Option<String>,
    /// Inline comment style override
    pub comment_style: Option<String>,
    /// Inline chat style override
    pub chat_style: Option<String>,
    /// Which templates are active — empty means all enabled
    pub enabled: Vec<String>,
}

impl Default for FormattingConfig {
    fn default() -> Self {
        Self {
            templates_dir: None,
            commit_format: None,
            pr_format: None,
            comment_style: None,
            chat_style: None,
            enabled: vec![],
        }
    }
}

/// Loaded template set ready for injection into the system prompt
#[derive(Debug, Clone)]
pub struct TemplateSet {
    pub commit: String,
    pub pr: String,
    pub comments: String,
    pub chat: String,
}

impl Default for TemplateSet {
    fn default() -> Self {
        Self {
            commit: DEFAULT_COMMIT.to_string(),
            pr: DEFAULT_PR.to_string(),
            comments: DEFAULT_COMMENTS.to_string(),
            chat: DEFAULT_CHAT.to_string(),
        }
    }
}

/// Load templates with precedence chain:
/// defaults → global ~/.ftai/templates/ → user-project ~/.ftai/projects/<encoded>/templates/
/// → in-repo <project>/.ftai/templates/ → inline config overrides (highest)
pub fn load_templates(config: &FormattingConfig, project_path: Option<&Path>) -> Result<TemplateSet> {
    let mut templates = TemplateSet::default();

    // Layer 1: Global templates
    if let Ok(global_dir) = crate::config::global_config_dir() {
        let templates_dir = global_dir.join("templates");
        load_from_dir(&templates_dir, &mut templates);
    }

    if let Some(project) = project_path {
        // Layer 2: User-project templates (~/.ftai/projects/<encoded>/templates/)
        if let Ok(project_dir) = crate::config::project_config_dir(project) {
            let templates_dir = project_dir.join("templates");
            load_from_dir(&templates_dir, &mut templates);
        }

        // Layer 3: In-repo templates (<project>/.ftai/templates/)
        let repo_templates = project.join(".ftai").join("templates");
        load_from_dir(&repo_templates, &mut templates);
    }

    // Layer 4: Custom templates_dir from config
    if let Some(ref custom_dir) = config.templates_dir {
        let custom_path = Path::new(custom_dir);
        if custom_path.is_absolute() {
            load_from_dir(custom_path, &mut templates);
        }
    }

    // Layer 5 (highest): Inline config overrides
    if let Some(ref fmt) = config.commit_format {
        templates.commit = fmt.clone();
    }
    if let Some(ref fmt) = config.pr_format {
        templates.pr = fmt.clone();
    }
    if let Some(ref fmt) = config.comment_style {
        templates.comments = fmt.clone();
    }
    if let Some(ref fmt) = config.chat_style {
        templates.chat = fmt.clone();
    }

    // Substitute meta variables
    if let Some(project) = project_path {
        substitute_meta_variables(&mut templates, project);
    }

    Ok(templates)
}

/// Load template files from a directory, overriding any that exist
fn load_from_dir(dir: &Path, templates: &mut TemplateSet) {
    if !dir.is_dir() {
        return;
    }

    if let Ok(content) = std::fs::read_to_string(dir.join("commit.md")) {
        templates.commit = content;
    }
    if let Ok(content) = std::fs::read_to_string(dir.join("pr.md")) {
        templates.pr = content;
    }
    if let Ok(content) = std::fs::read_to_string(dir.join("comments.md")) {
        templates.comments = content;
    }
    if let Ok(content) = std::fs::read_to_string(dir.join("chat.md")) {
        templates.chat = content;
    }
}

/// Replace {{project_name}} and {{project_path}} placeholders
fn substitute_meta_variables(templates: &mut TemplateSet, project_path: &Path) {
    let project_name = project_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let project_path_str = project_path.to_string_lossy().to_string();

    for template in [
        &mut templates.commit,
        &mut templates.pr,
        &mut templates.comments,
        &mut templates.chat,
    ] {
        *template = template.replace("{{project_name}}", &project_name);
        *template = template.replace("{{project_path}}", &project_path_str);
    }
}

/// Write default template files to a directory
pub fn write_default_templates(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).context("Failed to create templates directory")?;

    let files = [
        ("commit.md", DEFAULT_COMMIT),
        ("pr.md", DEFAULT_PR),
        ("comments.md", DEFAULT_COMMENTS),
        ("chat.md", DEFAULT_CHAT),
    ];

    for (name, content) in files {
        let path = dir.join(name);
        if !path.exists() {
            std::fs::write(&path, content)
                .with_context(|| format!("Failed to write default template {name}"))?;
        }
    }

    Ok(())
}

/// Filter a TemplateSet to only include enabled templates.
/// Returns (label, content) pairs for injection into the system prompt.
pub fn enabled_templates<'a>(templates: &'a TemplateSet, enabled: &[String]) -> Vec<(&'static str, &'a str)> {
    let all_templates = [
        ("commit", "Commit Messages", templates.commit.as_str()),
        ("pr", "Pull Requests", templates.pr.as_str()),
        ("comments", "Code Comments", templates.comments.as_str()),
        ("chat", "Chat Responses", templates.chat.as_str()),
    ];

    if enabled.is_empty() {
        // All enabled by default
        return all_templates
            .iter()
            .map(|(_, label, content)| (*label, *content))
            .collect();
    }

    all_templates
        .iter()
        .filter(|(key, _, _)| enabled.iter().any(|e| e == key))
        .map(|(_, label, content)| (*label, *content))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_templates() {
        let config = FormattingConfig::default();
        let templates = load_templates(&config, None).unwrap();
        assert!(templates.commit.contains("conventional commits"));
        assert!(templates.pr.contains("What changed"));
        assert!(templates.comments.contains("WHY"));
        assert!(templates.chat.contains("direct"));
    }

    #[test]
    fn test_global_override() {
        let tmp = TempDir::new().unwrap();
        let templates_dir = tmp.path().join("templates");
        std::fs::create_dir_all(&templates_dir).unwrap();
        std::fs::write(templates_dir.join("commit.md"), "Custom commit format").unwrap();

        let config = FormattingConfig {
            templates_dir: Some(templates_dir.to_string_lossy().to_string()),
            ..Default::default()
        };
        let templates = load_templates(&config, None).unwrap();
        assert_eq!(templates.commit, "Custom commit format");
        // Others should remain defaults
        assert!(templates.pr.contains("What changed"));
    }

    #[test]
    fn test_project_override() {
        let tmp = TempDir::new().unwrap();
        let project_templates = tmp.path().join(".ftai").join("templates");
        std::fs::create_dir_all(&project_templates).unwrap();
        std::fs::write(project_templates.join("pr.md"), "Project PR format").unwrap();

        let config = FormattingConfig::default();
        let templates = load_templates(&config, Some(tmp.path())).unwrap();
        assert_eq!(templates.pr, "Project PR format");
        // Commit should remain default
        assert!(templates.commit.contains("conventional commits"));
    }

    #[test]
    fn test_inline_override_highest_precedence() {
        let tmp = TempDir::new().unwrap();
        // Set up a project template
        let project_templates = tmp.path().join(".ftai").join("templates");
        std::fs::create_dir_all(&project_templates).unwrap();
        std::fs::write(project_templates.join("commit.md"), "Project commit").unwrap();

        // Inline should override project
        let config = FormattingConfig {
            commit_format: Some("Inline commit override".to_string()),
            ..Default::default()
        };
        let templates = load_templates(&config, Some(tmp.path())).unwrap();
        assert_eq!(templates.commit, "Inline commit override");
    }

    #[test]
    fn test_missing_dir_uses_defaults() {
        let config = FormattingConfig {
            templates_dir: Some("/nonexistent/path/templates".to_string()),
            ..Default::default()
        };
        let templates = load_templates(&config, None).unwrap();
        assert!(templates.commit.contains("conventional commits"));
    }

    #[test]
    fn test_meta_substitution() {
        let tmp = TempDir::new().unwrap();
        let project_templates = tmp.path().join(".ftai").join("templates");
        std::fs::create_dir_all(&project_templates).unwrap();
        std::fs::write(
            project_templates.join("chat.md"),
            "You are working on {{project_name}} at {{project_path}}",
        )
        .unwrap();

        let config = FormattingConfig::default();
        let templates = load_templates(&config, Some(tmp.path())).unwrap();
        let dir_name = tmp.path().file_name().unwrap().to_string_lossy();
        assert!(templates.chat.contains(&*dir_name));
        assert!(templates.chat.contains(&tmp.path().to_string_lossy().to_string()));
    }

    #[test]
    fn test_enabled_filter() {
        let templates = TemplateSet::default();

        // All enabled when empty
        let all = enabled_templates(&templates, &[]);
        assert_eq!(all.len(), 4);

        // Filter to just commit and chat
        let filtered = enabled_templates(&templates, &["commit".to_string(), "chat".to_string()]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|(label, _)| *label == "Commit Messages"));
        assert!(filtered.iter().any(|(label, _)| *label == "Chat Responses"));
    }
}
