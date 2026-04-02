use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Legacy registry client (used by TUI /plugin search)
// ---------------------------------------------------------------------------

const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/FolkTechAI/ftai-registry/main/registry.json";

#[derive(Debug, Clone, Deserialize)]
pub struct RegistryIndex {
    pub plugins: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistryEntry {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub struct RegistryClient {
    url: String,
}

impl RegistryClient {
    pub fn new(url: Option<&str>) -> Self {
        Self {
            url: url.unwrap_or(DEFAULT_REGISTRY_URL).to_string(),
        }
    }

    /// Fetch the registry index from the remote URL.
    pub async fn fetch_index(&self) -> Result<RegistryIndex> {
        let resp = reqwest::get(&self.url)
            .await
            .context("Failed to fetch plugin registry")?;

        let body = resp
            .text()
            .await
            .context("Failed to read registry response")?;

        let index: RegistryIndex =
            serde_json::from_str(&body).context("Failed to parse plugin registry JSON")?;

        Ok(index)
    }

    /// Search the registry for plugins matching a query.
    pub async fn search(&self, query: &str) -> Result<Vec<RegistryEntry>> {
        let index = self.fetch_index().await?;
        let query_lower = query.to_lowercase();

        let results: Vec<RegistryEntry> = index
            .plugins
            .into_iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query_lower)
                    || p.description.to_lowercase().contains(&query_lower)
                    || p.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect();

        Ok(results)
    }

    /// Get info about a specific plugin from the registry.
    pub async fn fetch_info(&self, name: &str) -> Result<Option<RegistryEntry>> {
        let index = self.fetch_index().await?;
        Ok(index.plugins.into_iter().find(|p| p.name == name))
    }
}

// ---------------------------------------------------------------------------
// Marketplace data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSource {
    pub name: String,
    pub repo: String, // "owner/repo" format
}

#[derive(Debug, Clone)]
pub struct MarketplacePlugin {
    pub name: String,
    pub version: String,
    pub description: String,
    pub source_name: String,
    pub install_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MarketplacesConfig {
    #[serde(default)]
    marketplace: Vec<MarketplaceSource>,
}

pub struct MarketplaceRegistry {
    config_path: PathBuf,
    cache_dir: PathBuf,
    sources: Vec<MarketplaceSource>,
}

// ---------------------------------------------------------------------------
// CC-format plugin.json structures
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CcPluginJson {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: Option<CcAuthor>,
}

#[derive(Debug, Deserialize)]
struct CcAuthor {
    #[allow(dead_code)]
    name: String,
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Validate a marketplace name: non-empty, alphanumeric + hyphens only, no
/// path traversal, no shell-injection chars.
fn is_valid_marketplace_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !name.contains("..")
}

/// Validate a repo string: must match `owner/repo` with safe characters.
fn is_valid_repo(repo: &str) -> bool {
    let re = Regex::new(r"^[a-zA-Z0-9_-]+/[a-zA-Z0-9_.\-]+$").expect("valid regex");
    re.is_match(repo)
}

// ---------------------------------------------------------------------------
// MarketplaceRegistry implementation
// ---------------------------------------------------------------------------

impl MarketplaceRegistry {
    /// Load (or create) a registry from the given config directory.
    ///
    /// - Config file: `config_dir/marketplaces.toml`
    /// - Cache dir:   `config_dir/marketplaces/`
    pub fn new(config_dir: &Path) -> Result<Self> {
        let config_path = config_dir.join("marketplaces.toml");
        let cache_dir = config_dir.join("marketplaces");

        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("Failed to create cache dir {}", cache_dir.display()))?;

        let sources = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read {}", config_path.display()))?;
            let cfg: MarketplacesConfig = toml::from_str(&content)
                .with_context(|| format!("Failed to parse {}", config_path.display()))?;
            cfg.marketplace
        } else {
            Vec::new()
        };

        Ok(Self {
            config_path,
            cache_dir,
            sources,
        })
    }

    /// Add a new marketplace source.
    ///
    /// Validates the name and repo format, checks for duplicates, persists to
    /// TOML, and clones the repo with `--depth=1` into the cache directory.
    pub fn add_source(&mut self, name: &str, repo: &str) -> Result<()> {
        if !is_valid_marketplace_name(name) {
            bail!(
                "Invalid marketplace name '{}': must be alphanumeric and hyphens only",
                name
            );
        }
        if !is_valid_repo(repo) {
            bail!(
                "Invalid repo format '{}': must match owner/repo with safe characters",
                repo
            );
        }
        if self.sources.iter().any(|s| s.name == name) {
            bail!("Marketplace source '{}' already exists", name);
        }

        let source = MarketplaceSource {
            name: name.to_string(),
            repo: repo.to_string(),
        };
        self.sources.push(source);
        self.save_config()?;

        // Clone into cache
        let dest = self.cache_dir.join(name);
        let url = format!("https://github.com/{}.git", repo);
        let status = std::process::Command::new("git")
            .args(["clone", "--depth=1", &url])
            .arg(&dest)
            .status()
            .context("Failed to run git clone")?;

        if !status.success() {
            // Roll back the source we just added
            self.sources.retain(|s| s.name != name);
            self.save_config()?;
            bail!("git clone failed for {}", url);
        }

        Ok(())
    }

    /// Remove a marketplace source by name.
    pub fn remove_source(&mut self, name: &str) -> Result<()> {
        let before = self.sources.len();
        self.sources.retain(|s| s.name != name);
        if self.sources.len() == before {
            bail!("Marketplace source '{}' not found", name);
        }
        self.save_config()?;

        let cache = self.cache_dir.join(name);
        if cache.exists() {
            std::fs::remove_dir_all(&cache)
                .with_context(|| format!("Failed to remove cache dir {}", cache.display()))?;
        }

        Ok(())
    }

    /// Return all registered marketplace sources.
    pub fn list_sources(&self) -> &[MarketplaceSource] {
        &self.sources
    }

    /// Pull latest changes for every cached marketplace source.
    pub fn update_all(&self) -> Result<()> {
        for source in &self.sources {
            let dir = self.cache_dir.join(&source.name);
            if !dir.exists() {
                continue;
            }
            let status = std::process::Command::new("git")
                .args(["pull"])
                .current_dir(&dir)
                .status()
                .with_context(|| format!("Failed to git pull in {}", dir.display()))?;

            if !status.success() {
                eprintln!("Warning: git pull failed for marketplace '{}'", source.name);
            }
        }
        Ok(())
    }

    /// Search all cached marketplaces for plugins matching a query.
    ///
    /// Matches case-insensitively against plugin name and description.
    pub fn search(&self, query: &str) -> Vec<MarketplacePlugin> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for source in &self.sources {
            let source_dir = self.cache_dir.join(&source.name);
            if !source_dir.is_dir() {
                continue;
            }
            self.scan_source_dir(&source_dir, source, &query_lower, &mut results);
        }

        results
    }

    /// Find a plugin by exact name across all cached sources.
    pub fn find_plugin(&self, name: &str) -> Option<MarketplacePlugin> {
        for source in &self.sources {
            let source_dir = self.cache_dir.join(&source.name);
            if !source_dir.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(&source_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if let Some(plugin) = self.try_read_plugin(&path, source) {
                    if plugin.name == name {
                        return Some(plugin);
                    }
                }
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn save_config(&self) -> Result<()> {
        let cfg = MarketplacesConfig {
            marketplace: self.sources.clone(),
        };
        let content = toml::to_string_pretty(&cfg).context("Failed to serialize config")?;
        std::fs::write(&self.config_path, content)
            .with_context(|| format!("Failed to write {}", self.config_path.display()))?;
        Ok(())
    }

    fn scan_source_dir(
        &self,
        source_dir: &Path,
        source: &MarketplaceSource,
        query_lower: &str,
        results: &mut Vec<MarketplacePlugin>,
    ) {
        let entries = match std::fs::read_dir(source_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(plugin) = self.try_read_plugin(&path, source) {
                if plugin.name.to_lowercase().contains(query_lower)
                    || plugin.description.to_lowercase().contains(query_lower)
                {
                    results.push(plugin);
                }
            }
        }
    }

    /// Try to read a plugin from a subdirectory. Checks Forge format first
    /// (`plugin.toml`), then CC format (`.claude-plugin/plugin.json`).
    fn try_read_plugin(&self, dir: &Path, source: &MarketplaceSource) -> Option<MarketplacePlugin> {
        // Forge format: plugin.toml
        let forge_manifest = dir.join("plugin.toml");
        if forge_manifest.is_file() {
            if let Ok(content) = std::fs::read_to_string(&forge_manifest) {
                if let Ok(manifest) = toml::from_str::<crate::plugins::manifest::PluginManifest>(&content) {
                    let subdir_name = dir.file_name()?.to_str()?;
                    return Some(MarketplacePlugin {
                        name: manifest.plugin.name.clone(),
                        version: manifest.plugin.version.clone(),
                        description: manifest.plugin.description.clone(),
                        source_name: source.name.clone(),
                        install_url: format!(
                            "https://github.com/{}/tree/main/{}",
                            source.repo, subdir_name
                        ),
                    });
                }
            }
        }

        // CC format: .claude-plugin/plugin.json
        let cc_manifest = dir.join(".claude-plugin").join("plugin.json");
        if cc_manifest.is_file() {
            if let Ok(content) = std::fs::read_to_string(&cc_manifest) {
                if let Ok(cc) = serde_json::from_str::<CcPluginJson>(&content) {
                    let subdir_name = dir.file_name()?.to_str()?;
                    return Some(MarketplacePlugin {
                        name: cc.name.clone(),
                        version: if cc.version.is_empty() {
                            "0.0.0".to_string()
                        } else {
                            cc.version
                        },
                        description: cc.description,
                        source_name: source.name.clone(),
                        install_url: format!(
                            "https://github.com/{}/tree/main/{}",
                            source.repo, subdir_name
                        ),
                    });
                }
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a minimal Forge-format plugin in a subdirectory.
    fn create_forge_plugin(parent: &Path, subdir: &str, name: &str, desc: &str) {
        let dir = parent.join(subdir);
        std::fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"[plugin]
name = "{name}"
version = "1.0.0"
description = "{desc}"
author = "test"
"#
        );
        std::fs::write(dir.join("plugin.toml"), toml).unwrap();
    }

    /// Helper: create a CC-format plugin in a subdirectory.
    fn create_cc_plugin(parent: &Path, subdir: &str, name: &str, desc: &str) {
        let dir = parent.join(subdir).join(".claude-plugin");
        std::fs::create_dir_all(&dir).unwrap();
        let json = serde_json::json!({
            "name": name,
            "version": "2.0.0",
            "description": desc,
            "author": {"name": "cc-author"}
        });
        std::fs::write(dir.join("plugin.json"), json.to_string()).unwrap();
    }

    /// Helper: set up a registry with a fake cached source (no git).
    fn setup_registry_with_source(
        tmp: &TempDir,
        source_name: &str,
        repo: &str,
    ) -> MarketplaceRegistry {
        let config_dir = tmp.path();

        // Write config
        let cfg = format!(
            r#"[[marketplace]]
name = "{source_name}"
repo = "{repo}"
"#
        );
        std::fs::write(config_dir.join("marketplaces.toml"), cfg).unwrap();

        // Create cache dir for the source
        std::fs::create_dir_all(config_dir.join("marketplaces").join(source_name)).unwrap();

        MarketplaceRegistry::new(config_dir).unwrap()
    }

    // -----------------------------------------------------------------------
    // Core functionality
    // -----------------------------------------------------------------------

    #[test]
    fn test_new_missing_config_creates_empty_registry() {
        let tmp = TempDir::new().unwrap();
        let reg = MarketplaceRegistry::new(tmp.path()).unwrap();

        assert!(reg.sources.is_empty());
        assert!(reg.cache_dir.exists());
        assert!(!reg.config_path.exists()); // not created until first write
    }

    #[test]
    fn test_add_source_saves_to_toml() {
        let tmp = TempDir::new().unwrap();
        let mut reg = MarketplaceRegistry::new(tmp.path()).unwrap();

        // We can't actually git clone, so we test the validation and TOML write
        // by pre-creating the cache dir and calling save_config directly.
        reg.sources.push(MarketplaceSource {
            name: "test-market".to_string(),
            repo: "owner/repo".to_string(),
        });
        reg.save_config().unwrap();

        let content = std::fs::read_to_string(tmp.path().join("marketplaces.toml")).unwrap();
        assert!(content.contains("test-market"));
        assert!(content.contains("owner/repo"));

        // Reload and verify
        let reg2 = MarketplaceRegistry::new(tmp.path()).unwrap();
        assert_eq!(reg2.sources.len(), 1);
        assert_eq!(reg2.sources[0].name, "test-market");
        assert_eq!(reg2.sources[0].repo, "owner/repo");
    }

    #[test]
    fn test_remove_source_removes_from_toml() {
        let tmp = TempDir::new().unwrap();
        let mut reg = setup_registry_with_source(&tmp, "to-remove", "owner/repo");

        assert_eq!(reg.sources.len(), 1);
        reg.remove_source("to-remove").unwrap();
        assert!(reg.sources.is_empty());

        // Reload and verify
        let reg2 = MarketplaceRegistry::new(tmp.path()).unwrap();
        assert!(reg2.sources.is_empty());
    }

    #[test]
    fn test_list_sources_returns_all() {
        let tmp = TempDir::new().unwrap();
        let cfg = r#"[[marketplace]]
name = "alpha"
repo = "a/alpha"

[[marketplace]]
name = "beta"
repo = "b/beta"
"#;
        std::fs::write(tmp.path().join("marketplaces.toml"), cfg).unwrap();
        std::fs::create_dir_all(tmp.path().join("marketplaces")).unwrap();

        let reg = MarketplaceRegistry::new(tmp.path()).unwrap();
        let sources = reg.list_sources();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].name, "alpha");
        assert_eq!(sources[1].name, "beta");
    }

    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    #[test]
    fn test_search_no_cache_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let reg = MarketplaceRegistry::new(tmp.path()).unwrap();
        let results = reg.search("anything");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_finds_forge_format_plugins() {
        let tmp = TempDir::new().unwrap();
        let reg = setup_registry_with_source(&tmp, "forge-market", "owner/forge-market");

        let source_cache = tmp.path().join("marketplaces").join("forge-market");
        create_forge_plugin(&source_cache, "my-tool", "my-tool", "A useful tool for devs");
        create_forge_plugin(&source_cache, "other", "other-plugin", "Something else");

        let results = reg.search("useful");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "my-tool");
        assert_eq!(results[0].version, "1.0.0");
        assert_eq!(results[0].source_name, "forge-market");
        assert!(results[0].install_url.contains("my-tool"));
    }

    #[test]
    fn test_search_finds_cc_format_plugins() {
        let tmp = TempDir::new().unwrap();
        let reg = setup_registry_with_source(&tmp, "cc-market", "anthropic/cc-market");

        let source_cache = tmp.path().join("marketplaces").join("cc-market");
        create_cc_plugin(
            &source_cache,
            "cc-tool",
            "cc-tool",
            "Claude Code compatible tool",
        );

        let results = reg.search("claude");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "cc-tool");
        assert_eq!(results[0].version, "2.0.0");
        assert_eq!(results[0].source_name, "cc-market");
        assert!(results[0]
            .install_url
            .contains("anthropic/cc-market/tree/main/cc-tool"));
    }

    #[test]
    fn test_search_is_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        let reg = setup_registry_with_source(&tmp, "market", "owner/market");

        let source_cache = tmp.path().join("marketplaces").join("market");
        create_forge_plugin(&source_cache, "linter", "SuperLinter", "A GREAT linting tool");

        // Search with different cases
        assert_eq!(reg.search("superlinter").len(), 1);
        assert_eq!(reg.search("SUPERLINTER").len(), 1);
        assert_eq!(reg.search("great linting").len(), 1);
        assert_eq!(reg.search("GREAT LINTING").len(), 1);
    }

    // -----------------------------------------------------------------------
    // find_plugin
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_plugin_exact_match() {
        let tmp = TempDir::new().unwrap();
        let reg = setup_registry_with_source(&tmp, "market", "owner/market");

        let source_cache = tmp.path().join("marketplaces").join("market");
        create_forge_plugin(&source_cache, "target", "target-plugin", "The one we want");
        create_forge_plugin(&source_cache, "decoy", "decoy-plugin", "Not this one");

        let found = reg.find_plugin("target-plugin");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "target-plugin");
    }

    #[test]
    fn test_find_plugin_returns_none_for_missing() {
        let tmp = TempDir::new().unwrap();
        let reg = setup_registry_with_source(&tmp, "market", "owner/market");

        let source_cache = tmp.path().join("marketplaces").join("market");
        create_forge_plugin(&source_cache, "exists", "exists-plugin", "I exist");

        assert!(reg.find_plugin("nonexistent").is_none());
    }

    // -----------------------------------------------------------------------
    // Validation — security red tests (P0)
    // -----------------------------------------------------------------------

    #[test]
    fn test_invalid_marketplace_name_rejected() {
        assert!(!is_valid_marketplace_name(""));
        assert!(!is_valid_marketplace_name("../etc"));
        assert!(!is_valid_marketplace_name("foo/bar"));
        assert!(!is_valid_marketplace_name("foo bar"));
        assert!(!is_valid_marketplace_name("foo;rm"));
        assert!(!is_valid_marketplace_name("$(whoami)"));
        assert!(!is_valid_marketplace_name("`id`"));
        assert!(!is_valid_marketplace_name("a\0b"));
        assert!(!is_valid_marketplace_name("has.dot"));
        assert!(!is_valid_marketplace_name("has_underscore")); // underscores not allowed per spec

        // Valid names
        assert!(is_valid_marketplace_name("my-market"));
        assert!(is_valid_marketplace_name("market123"));
        assert!(is_valid_marketplace_name("a"));
    }

    #[test]
    fn test_invalid_repo_format_rejected() {
        assert!(!is_valid_repo(""));
        assert!(!is_valid_repo("noslash"));
        assert!(!is_valid_repo("too/many/slashes"));
        assert!(!is_valid_repo("owner/repo;rm -rf /"));
        assert!(!is_valid_repo("owner/repo$(whoami)"));
        assert!(!is_valid_repo("owner/repo`id`"));
        assert!(!is_valid_repo("owner/repo|cat /etc/passwd"));
        assert!(!is_valid_repo("owner/repo&echo pwned"));
        assert!(!is_valid_repo("owner/repo\nnewline"));
        assert!(!is_valid_repo("../traversal/repo"));
        assert!(!is_valid_repo(" spaces/repo"));

        // Valid repos
        assert!(is_valid_repo("owner/repo"));
        assert!(is_valid_repo("My-Org/my_repo.rs"));
        assert!(is_valid_repo("a/b"));
        assert!(is_valid_repo("FolkTechAI/forge-plugins"));
    }

    #[test]
    fn test_duplicate_marketplace_name_rejected() {
        let tmp = TempDir::new().unwrap();
        let mut reg = setup_registry_with_source(&tmp, "existing", "owner/repo");

        // Trying to add with same name should fail (even though add_source would
        // try to git clone, the duplicate check comes first)
        let result = reg.add_source("existing", "other/repo");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
    }

    #[test]
    fn test_add_source_rejects_bad_name() {
        let tmp = TempDir::new().unwrap();
        let mut reg = MarketplaceRegistry::new(tmp.path()).unwrap();

        let result = reg.add_source("../escape", "owner/repo");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid marketplace name"));
    }

    #[test]
    fn test_add_source_rejects_bad_repo() {
        let tmp = TempDir::new().unwrap();
        let mut reg = MarketplaceRegistry::new(tmp.path()).unwrap();

        let result = reg.add_source("valid-name", "not-a-valid-repo");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid repo format"));
    }

    #[test]
    fn test_remove_nonexistent_source_errors() {
        let tmp = TempDir::new().unwrap();
        let mut reg = MarketplaceRegistry::new(tmp.path()).unwrap();

        let result = reg.remove_source("ghost");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_search_ignores_non_plugin_subdirs() {
        let tmp = TempDir::new().unwrap();
        let reg = setup_registry_with_source(&tmp, "market", "owner/market");

        let source_cache = tmp.path().join("marketplaces").join("market");

        // Create a subdir with no manifest — should be silently ignored
        std::fs::create_dir_all(source_cache.join("random-dir")).unwrap();
        std::fs::write(source_cache.join("random-dir").join("README.md"), "hi").unwrap();

        // Create a file (not a dir) — should be ignored
        std::fs::write(source_cache.join("not-a-dir.txt"), "nope").unwrap();

        // Create one valid plugin
        create_forge_plugin(&source_cache, "valid", "valid-plugin", "I am valid");

        let results = reg.search("valid");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "valid-plugin");
    }

    #[test]
    fn test_cc_plugin_without_version_gets_default() {
        let tmp = TempDir::new().unwrap();
        let reg = setup_registry_with_source(&tmp, "market", "owner/market");

        let source_cache = tmp.path().join("marketplaces").join("market");
        let dir = source_cache.join("no-ver").join(".claude-plugin");
        std::fs::create_dir_all(&dir).unwrap();
        let json = serde_json::json!({
            "name": "no-ver-plugin",
            "description": "Missing version field"
        });
        std::fs::write(dir.join("plugin.json"), json.to_string()).unwrap();

        let found = reg.find_plugin("no-ver-plugin");
        assert!(found.is_some());
        assert_eq!(found.unwrap().version, "0.0.0");
    }
}
