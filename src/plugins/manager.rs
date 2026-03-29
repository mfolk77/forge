use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::hooks::ResolvedHook;
use super::manifest::{self, PluginManifest};
use super::skill_loader::{self, LoadedSkill};
use super::tool_bridge::PluginTool;

/// A fully loaded plugin with resolved paths.
#[derive(Debug)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub dir: PathBuf,
}

/// Manages plugin discovery, loading, installation, and uninstallation.
pub struct PluginManager {
    plugins_dir: PathBuf,
    plugins: Vec<LoadedPlugin>,
}

impl PluginManager {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self {
            plugins_dir,
            plugins: Vec::new(),
        }
    }

    /// Discover and load all plugins from the plugins directory.
    pub fn load_all(&mut self) -> Result<usize> {
        self.plugins.clear();

        if !self.plugins_dir.exists() {
            return Ok(0);
        }

        let entries = std::fs::read_dir(&self.plugins_dir)
            .with_context(|| format!("Failed to read plugins dir: {}", self.plugins_dir.display()))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("plugin.toml");
            if !manifest_path.exists() {
                continue;
            }

            match manifest::load_manifest(&path) {
                Ok(m) => {
                    self.plugins.push(LoadedPlugin {
                        manifest: m,
                        dir: path,
                    });
                }
                Err(e) => {
                    eprintln!(
                        "Warning: failed to load plugin at {}: {e}",
                        path.display()
                    );
                }
            }
        }

        Ok(self.plugins.len())
    }

    /// Install a plugin from a local path (copy into plugins dir).
    pub fn install_from_path(&mut self, source: &Path) -> Result<String> {
        let manifest = manifest::load_manifest(source)
            .with_context(|| format!("Invalid plugin at {}", source.display()))?;

        let dest = self.plugins_dir.join(&manifest.plugin.name);
        if dest.exists() {
            anyhow::bail!("Plugin '{}' is already installed", manifest.plugin.name);
        }

        copy_dir_recursive(source, &dest)?;

        let name = manifest.plugin.name.clone();
        self.plugins.push(LoadedPlugin {
            manifest,
            dir: dest,
        });

        Ok(name)
    }

    /// Install a plugin from a git URL.
    pub fn install_from_git(&self, url: &str) -> Result<String> {
        // Validate URL doesn't contain shell injection characters
        if url.contains(';') || url.contains('&') || url.contains('|') || url.contains('`') {
            anyhow::bail!("Invalid characters in URL");
        }

        let tmp_dir = self.plugins_dir.join(".tmp-clone");
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir)?;

        let status = std::process::Command::new("git")
            .args(["clone", "--depth=1", url, &tmp_dir.to_string_lossy()])
            .status()
            .context("Failed to run git clone")?;

        if !status.success() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            anyhow::bail!("git clone failed for {url}");
        }

        let manifest = manifest::load_manifest(&tmp_dir)
            .context("Cloned repo is not a valid ftai plugin");

        let manifest = match manifest {
            Ok(m) => m,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return Err(e);
            }
        };

        let dest = self.plugins_dir.join(&manifest.plugin.name);
        if dest.exists() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            anyhow::bail!("Plugin '{}' is already installed", manifest.plugin.name);
        }

        copy_dir_recursive(&tmp_dir, &dest)?;
        let _ = std::fs::remove_dir_all(&tmp_dir);

        Ok(manifest.plugin.name)
    }

    /// Uninstall a plugin by name.
    pub fn uninstall(&mut self, name: &str) -> Result<()> {
        let dir = self.plugins_dir.join(name);
        if !dir.exists() {
            anyhow::bail!("Plugin '{name}' is not installed");
        }

        // Verify it's actually within our plugins dir (prevent traversal)
        let canonical_dir = dir.canonicalize()?;
        let canonical_plugins = self.plugins_dir.canonicalize()?;
        if !canonical_dir.starts_with(&canonical_plugins) {
            anyhow::bail!("Plugin path escapes plugins directory");
        }

        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("Failed to remove plugin directory: {}", dir.display()))?;

        self.plugins.retain(|p| p.manifest.plugin.name != name);

        Ok(())
    }

    /// List all loaded plugins.
    pub fn list(&self) -> &[LoadedPlugin] {
        &self.plugins
    }

    /// Get all tool definitions from all plugins.
    pub fn get_tools(&self) -> Vec<PluginTool> {
        let mut tools = Vec::new();
        for plugin in &self.plugins {
            for tool_def in &plugin.manifest.tools {
                tools.push(PluginTool::new(
                    format!("plugin:{}:{}", plugin.manifest.plugin.name, tool_def.name),
                    tool_def.description.clone(),
                    tool_def.command.clone(),
                    plugin.dir.clone(),
                    tool_def.params.clone(),
                ));
            }
        }
        tools
    }

    /// Get all loaded skills from all plugins.
    pub fn get_skills(&self) -> Vec<LoadedSkill> {
        let mut skills = Vec::new();
        for plugin in &self.plugins {
            for skill_def in &plugin.manifest.skills {
                if let Some(skill) = skill_loader::load_skill(
                    &plugin.dir,
                    &plugin.manifest.plugin.name,
                    &skill_def.name,
                    &skill_def.file,
                    &skill_def.description,
                    skill_def.trigger.as_deref(),
                ) {
                    skills.push(skill);
                }
            }
        }
        skills
    }

    /// Get all hooks matching a specific event (e.g., "pre:bash").
    pub fn get_hooks(&self, event: &str) -> Vec<ResolvedHook> {
        let mut hooks = Vec::new();
        for plugin in &self.plugins {
            for hook_def in &plugin.manifest.hooks {
                if hook_def.event == event {
                    // Validate command doesn't escape (Unix and Windows patterns)
                    if hook_def.command.contains("..")
                        || hook_def.command.starts_with('/')
                        || hook_def.command.starts_with("\\\\")
                        || (hook_def.command.len() >= 2
                            && hook_def.command.as_bytes()[0].is_ascii_alphabetic()
                            && hook_def.command.as_bytes()[1] == b':')
                    {
                        continue;
                    }
                    hooks.push(ResolvedHook {
                        event: hook_def.event.clone(),
                        command_path: plugin.dir.join(&hook_def.command),
                        plugin_dir: plugin.dir.clone(),
                        plugin_name: plugin.manifest.plugin.name.clone(),
                    });
                }
            }
        }
        hooks
    }

    /// Get all plugin-provided .ftai rules content.
    pub fn get_rules(&self) -> Vec<(String, String)> {
        let mut rules = Vec::new();
        for plugin in &self.plugins {
            let rules_file = plugin.dir.join("rules.ftai");
            if rules_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&rules_file) {
                    rules.push((plugin.manifest.plugin.name.clone(), content));
                }
            }
        }
        rules
    }
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            // Skip .git directory
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use tempfile::TempDir;

    /// Platform-aware script name and content for plugin test scripts.
    #[cfg(unix)]
    fn test_script() -> (&'static str, &'static str) {
        ("hello.sh", "#!/bin/bash\necho hello")
    }

    #[cfg(windows)]
    fn test_script() -> (&'static str, &'static str) {
        ("hello.bat", "@echo off\r\necho hello")
    }

    fn create_test_plugin(dir: &Path, name: &str) {
        let plugin_dir = dir.join(name);
        std::fs::create_dir_all(plugin_dir.join("tools")).unwrap();
        std::fs::create_dir_all(plugin_dir.join("skills")).unwrap();
        std::fs::create_dir_all(plugin_dir.join("hooks")).unwrap();

        let (script_name, script_content) = test_script();

        std::fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"
[plugin]
name = "{name}"
version = "0.1.0"
description = "Test plugin"

[[tools]]
name = "hello"
description = "Says hello"
command = "tools/{script_name}"

[[skills]]
name = "guide"
file = "skills/guide.md"
description = "A guide"
trigger = "/guide"
"#
            ),
        )
        .unwrap();

        let script = plugin_dir.join(format!("tools/{script_name}"));
        std::fs::write(&script, script_content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        std::fs::write(
            plugin_dir.join("skills/guide.md"),
            "# Guide\nStep 1: do thing",
        )
        .unwrap();
    }

    #[test]
    fn test_load_all_plugins() {
        let tmp = TempDir::new().unwrap();
        create_test_plugin(tmp.path(), "test-plugin");

        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        let count = mgr.load_all().unwrap();
        assert_eq!(count, 1);
        assert_eq!(mgr.list()[0].manifest.plugin.name, "test-plugin");
    }

    #[test]
    fn test_get_tools() {
        let tmp = TempDir::new().unwrap();
        create_test_plugin(tmp.path(), "test-plugin");

        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        mgr.load_all().unwrap();

        let tools = mgr.get_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "plugin:test-plugin:hello");
    }

    #[test]
    fn test_get_skills() {
        let tmp = TempDir::new().unwrap();
        create_test_plugin(tmp.path(), "test-plugin");

        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        mgr.load_all().unwrap();

        let skills = mgr.get_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "guide");
        assert_eq!(skills[0].trigger, "/guide");
    }

    #[test]
    fn test_install_from_path() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();

        let source = tmp.path().join("source-plugin");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("plugin.toml"),
            r#"
[plugin]
name = "local-plugin"
version = "0.1.0"
"#,
        )
        .unwrap();

        let mut mgr = PluginManager::new(plugins_dir.clone());
        let name = mgr.install_from_path(&source).unwrap();
        assert_eq!(name, "local-plugin");
        assert!(plugins_dir.join("local-plugin").join("plugin.toml").exists());
    }

    #[test]
    fn test_uninstall() {
        let tmp = TempDir::new().unwrap();
        create_test_plugin(tmp.path(), "removeme");

        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        mgr.load_all().unwrap();
        assert_eq!(mgr.list().len(), 1);

        mgr.uninstall("removeme").unwrap();
        assert_eq!(mgr.list().len(), 0);
        assert!(!tmp.path().join("removeme").exists());
    }

    #[test]
    fn test_uninstall_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        let result = mgr.uninstall("nope");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_plugins_dir() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        let count = mgr.load_all().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_missing_plugins_dir() {
        let non_existent = if cfg!(windows) {
            PathBuf::from("C:\\nonexistent\\plugins")
        } else {
            PathBuf::from("/nonexistent/plugins")
        };
        let mut mgr = PluginManager::new(non_existent);
        let count = mgr.load_all().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_plugin_tool_hard_block() {
        // Plugin tools are namespaced as "plugin:name:tool" — they go through
        // the same permission pipeline in process_response, including hard_block_check.
        // This is tested at the integration level in app.rs, but we verify the naming here.
        let tmp = TempDir::new().unwrap();
        create_test_plugin(tmp.path(), "test-plugin");
        let mut mgr = PluginManager::new(tmp.path().to_path_buf());
        mgr.load_all().unwrap();
        let tools = mgr.get_tools();
        // All plugin tools are prefixed — they can't masquerade as built-in tools
        assert!(tools[0].name().starts_with("plugin:"));
    }
}
