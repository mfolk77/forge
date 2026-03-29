use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
    #[serde(default)]
    pub registry: Option<RegistryMeta>,
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
    #[serde(default)]
    pub skills: Vec<PluginSkillDef>,
    #[serde(default)]
    pub hooks: Vec<PluginHookDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistryMeta {
    pub repo: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginToolDef {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default = "default_params")]
    pub params: serde_json::Value,
}

fn default_params() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}})
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginSkillDef {
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub trigger: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginHookDef {
    pub event: String,
    pub command: String,
}

/// Check if a path string contains traversal or absolute path patterns
/// on any platform (Unix forward-slash and Windows backslash variants).
fn is_path_escape(path: &str) -> bool {
    // Block Unix-style path traversal
    if path.contains("..") {
        return true;
    }
    // Block Unix absolute paths
    if path.starts_with('/') {
        return true;
    }
    // Block Windows-style backslash traversal (..\ anywhere)
    if path.contains("..\\") {
        return true;
    }
    // Block Windows absolute paths: drive letter like C:\ or C:/
    if path.len() >= 2 && path.as_bytes()[0].is_ascii_alphabetic() && path.as_bytes()[1] == b':' {
        return true;
    }
    // Block UNC paths (\\server\share)
    if path.starts_with("\\\\") {
        return true;
    }
    false
}

/// Load and parse a plugin.toml manifest from a directory.
pub fn load_manifest(plugin_dir: &Path) -> Result<PluginManifest> {
    let manifest_path = plugin_dir.join("plugin.toml");
    let content = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let manifest: PluginManifest = toml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", manifest_path.display()))?;

    // Sanitize: plugin name must be alphanumeric + hyphens only
    if !manifest.plugin.name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!(
            "Invalid plugin name '{}': must contain only alphanumeric, hyphen, or underscore",
            manifest.plugin.name
        );
    }

    // Sanitize: tool commands must not escape the plugin directory
    for tool in &manifest.tools {
        if is_path_escape(&tool.command) {
            anyhow::bail!(
                "Invalid tool command '{}' in plugin '{}': must be relative to plugin directory",
                tool.command, manifest.plugin.name
            );
        }
    }

    // Sanitize: hook commands must not escape
    for hook in &manifest.hooks {
        if is_path_escape(&hook.command) {
            anyhow::bail!(
                "Invalid hook command '{}' in plugin '{}': must be relative to plugin directory",
                hook.command, manifest.plugin.name
            );
        }
    }

    // Sanitize: skill file paths must not escape
    for skill in &manifest.skills {
        if is_path_escape(&skill.file) {
            anyhow::bail!(
                "Invalid skill file '{}' in plugin '{}': must be relative to plugin directory",
                skill.file, manifest.plugin.name
            );
        }
    }

    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_plugin_manifest() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "test-plugin"
version = "0.1.0"
description = "A test plugin"
author = "tester"

[[tools]]
name = "greet"
description = "Says hello"
command = "tools/greet.sh"

[[skills]]
name = "refactor"
file = "skills/refactor.md"
description = "Refactoring skill"
trigger = "/refactor"

[[hooks]]
event = "pre:bash"
command = "hooks/pre_bash.sh"
"#,
        )
        .unwrap();

        let manifest = load_manifest(tmp.path()).unwrap();
        assert_eq!(manifest.plugin.name, "test-plugin");
        assert_eq!(manifest.plugin.version, "0.1.0");
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "greet");
        assert_eq!(manifest.skills.len(), 1);
        assert_eq!(manifest.skills[0].trigger, Some("/refactor".to_string()));
        assert_eq!(manifest.hooks.len(), 1);
        assert_eq!(manifest.hooks[0].event, "pre:bash");
    }

    #[test]
    fn test_plugin_manifest_injection() {
        let tmp = TempDir::new().unwrap();

        // Malicious name
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "evil/../../plugin"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = load_manifest(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_plugin_tool_path_escape() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "escape-test"
version = "0.1.0"

[[tools]]
name = "evil"
description = "Tries to escape"
command = "../../etc/passwd"
"#,
        )
        .unwrap();

        let result = load_manifest(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be relative"));
    }

    #[test]
    fn test_plugin_absolute_path_blocked() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "abs-test"
version = "0.1.0"

[[tools]]
name = "evil"
description = "Absolute path"
command = "/usr/bin/rm"
"#,
        )
        .unwrap();

        let result = load_manifest(tmp.path());
        assert!(result.is_err());
    }

    // P0 security: Windows-style path traversal with backslashes
    #[test]
    fn test_windows_backslash_traversal_blocked() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "win-escape"
version = "0.1.0"

[[tools]]
name = "evil"
description = "Windows traversal"
command = "..\\..\\..\\Windows\\System32\\cmd.exe"
"#,
        )
        .unwrap();

        let result = load_manifest(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be relative"));
    }

    // P0 security: Windows drive letter absolute paths
    #[test]
    fn test_windows_drive_letter_path_blocked() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "win-abs"
version = "0.1.0"

[[tools]]
name = "evil"
description = "Windows absolute"
command = "C:\\Windows\\System32\\cmd.exe"
"#,
        )
        .unwrap();

        let result = load_manifest(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be relative"));
    }

    // P0 security: Windows drive letter with forward slash
    #[test]
    fn test_windows_drive_letter_forward_slash_blocked() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "win-abs-fwd"
version = "0.1.0"

[[tools]]
name = "evil"
description = "Windows absolute forward slash"
command = "C:/Windows/System32/cmd.exe"
"#,
        )
        .unwrap();

        let result = load_manifest(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be relative"));
    }

    // P0 security: UNC path blocked
    #[test]
    fn test_unc_path_blocked() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "unc-test"
version = "0.1.0"

[[tools]]
name = "evil"
description = "UNC path"
command = "\\\\server\\share\\evil.exe"
"#,
        )
        .unwrap();

        let result = load_manifest(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be relative"));
    }

    // P0 security: Hook command with Windows traversal
    #[test]
    fn test_hook_windows_traversal_blocked() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "hook-escape"
version = "0.1.0"

[[hooks]]
event = "pre:bash"
command = "..\\..\\..\\Windows\\System32\\cmd.exe"
"#,
        )
        .unwrap();

        let result = load_manifest(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be relative"));
    }

    // P0 security: Skill file with Windows traversal
    #[test]
    fn test_skill_windows_traversal_blocked() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("plugin.toml"),
            r#"
[plugin]
name = "skill-escape"
version = "0.1.0"

[[skills]]
name = "evil"
file = "..\\..\\..\\etc\\passwd"
description = "evil skill"
"#,
        )
        .unwrap();

        let result = load_manifest(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be relative"));
    }

    #[test]
    fn test_is_path_escape() {
        // Unix traversal
        assert!(is_path_escape("../../etc/passwd"));
        assert!(is_path_escape("../foo"));
        // Unix absolute
        assert!(is_path_escape("/usr/bin/rm"));
        // Windows backslash traversal
        assert!(is_path_escape("..\\..\\Windows\\System32\\cmd.exe"));
        // Windows drive letter
        assert!(is_path_escape("C:\\Windows\\System32"));
        assert!(is_path_escape("D:/some/path"));
        // UNC path
        assert!(is_path_escape("\\\\server\\share"));
        // Valid relative paths
        assert!(!is_path_escape("tools/hello.sh"));
        assert!(!is_path_escape("hooks/pre_bash.sh"));
        assert!(!is_path_escape("skills/guide.md"));
    }
}
