use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, Serializer};
use std::path::{Path, PathBuf};

use crate::formatting::FormattingConfig;

/// Serialize f64 with limited precision to avoid IEEE 754 noise (e.g. 0.3 → 0.30000001192092896)
fn serialize_f64_clean<S: Serializer>(val: &f64, ser: S) -> std::result::Result<S::Ok, S::Error> {
    // Round to 4 decimal places for clean display
    let rounded = (*val * 10000.0).round() / 10000.0;
    ser.serialize_f64(rounded)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub model: ModelConfig,
    pub permissions: PermissionConfig,
    pub paths: PathsConfig,
    pub formatting: FormattingConfig,
    pub plugins: PluginsConfig,
    pub theme: ThemeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub backend: BackendType,
    pub path: Option<String>,
    pub context_length: usize,
    #[serde(serialize_with = "serialize_f64_clean")]
    pub temperature: f64,
    pub llamacpp: LlamaCppConfig,
    pub mlx: MlxConfig,
    pub tool_calling: ToolCallingMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    LlamaCpp,
    Mlx,
    /// Direct FFI inference via llama.cpp — no HTTP server required
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallingMode {
    Native,
    Prompted,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    Ask,
    Auto,
    Yolo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlamaCppConfig {
    pub gpu_layers: i32,
    pub threads: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MlxConfig {
    pub quantization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionConfig {
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsConfig {
    pub models_dir: Option<String>,
    pub rules_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    pub enabled: bool,
    pub registry_url: Option<String>,
    pub auto_update: bool,
}

/// Theme configuration. Pick a preset or override individual colors.
///
/// In config.toml:
/// ```toml
/// [theme]
/// preset = "dark"     # "dark", "light", "high-contrast", "solarized", "dracula"
///
/// # Optional: override any individual color (hex "#RRGGBB" or named color)
/// # accent = "#E89C38"
/// # user_input = "#B4C8FF"
/// # assistant_text = "#DCDCE1"
/// # system_text = "#C8C8D2"
/// # error = "#DC5050"
/// # warning = "#DCB43C"
/// # tool_border = "#64A0DC"
/// # status_bar_fg = "#000000"
/// # status_bar_bg = "#00CCCC"
/// # status_line_fg = "#FFFFFF"
/// # status_line_bg = "#404040"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub preset: ThemePreset,
    pub accent: Option<String>,
    pub user_input: Option<String>,
    pub assistant_text: Option<String>,
    pub system_text: Option<String>,
    pub error: Option<String>,
    pub warning: Option<String>,
    pub tool_border: Option<String>,
    pub status_bar_fg: Option<String>,
    pub status_bar_bg: Option<String>,
    pub status_line_fg: Option<String>,
    pub status_line_bg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePreset {
    Dark,
    Light,
    HighContrast,
    Solarized,
    Dracula,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            preset: ThemePreset::Dark,
            accent: None,
            user_input: None,
            assistant_text: None,
            system_text: None,
            error: None,
            warning: None,
            tool_border: None,
            status_bar_fg: None,
            status_bar_bg: None,
            status_line_fg: None,
            status_line_bg: None,
        }
    }
}

// Defaults

impl Default for Config {
    fn default() -> Self {
        Self {
            model: ModelConfig::default(),
            permissions: PermissionConfig::default(),
            paths: PathsConfig::default(),
            formatting: FormattingConfig::default(),
            plugins: PluginsConfig::default(),
            theme: ThemeConfig::default(),
        }
    }
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            registry_url: None,
            auto_update: false,
        }
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            backend: BackendType::Mlx,
            path: None,
            context_length: 32768,
            temperature: 0.3_f64,
            llamacpp: LlamaCppConfig::default(),
            mlx: MlxConfig::default(),
            tool_calling: ToolCallingMode::Hybrid,
        }
    }
}

impl Default for LlamaCppConfig {
    fn default() -> Self {
        Self {
            gpu_layers: -1,
            threads: 8,
        }
    }
}

impl Default for MlxConfig {
    fn default() -> Self {
        Self {
            quantization: "q4".to_string(),
        }
    }
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            mode: PermissionMode::Auto,
        }
    }
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            models_dir: None,
            rules_file: None,
        }
    }
}

/// Returns the global ftai config directory (~/.ftai/)
pub fn global_config_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".ftai"))
}

/// Returns the project-specific config directory (~/.ftai/projects/<encoded-path>/)
pub fn project_config_dir(project_path: &Path) -> Result<PathBuf> {
    let global = global_config_dir()?;
    let encoded = project_path
        .to_string_lossy()
        .replace(['/', '\\'], "-")
        .trim_start_matches('-')
        .to_string();
    Ok(global.join("projects").join(encoded))
}

/// Create ~/.ftai/ directory structure if it doesn't exist
pub fn ensure_ftai_dirs() -> Result<()> {
    let global = global_config_dir()?;
    std::fs::create_dir_all(global.join("models"))?;
    std::fs::create_dir_all(global.join("memory"))?;
    std::fs::create_dir_all(global.join("projects"))?;
    std::fs::create_dir_all(global.join("plugins"))?;

    // Create default config if it doesn't exist
    let config_path = global.join("config.toml");
    if !config_path.exists() {
        let default_config = Config::default();
        let toml_str = toml::to_string_pretty(&default_config)
            .context("Failed to serialize default config")?;
        std::fs::write(&config_path, toml_str)?;
    }

    // Create default rules file if it doesn't exist
    let rules_path = global.join("rules.ftai");
    if !rules_path.exists() {
        std::fs::write(
            &rules_path,
            "# FTAI Rules\n# See docs for DSL syntax: https://folktech.ai/ftai/rules\n\n",
        )?;
    }

    // Create default templates
    let templates_dir = global.join("templates");
    crate::formatting::write_default_templates(&templates_dir)?;

    Ok(())
}

/// Load config with precedence: project .ftai/ > user project override > global
pub fn load_config(project_path: Option<&Path>) -> Result<Config> {
    let global_dir = global_config_dir()?;
    let global_config_path = global_dir.join("config.toml");

    // Start with defaults
    let mut config = Config::default();

    // Layer 1: Global config
    if global_config_path.exists() {
        let contents = std::fs::read_to_string(&global_config_path)
            .context("Failed to read global config")?;
        config = toml::from_str(&contents).context("Failed to parse global config")?;
    }

    if let Some(project) = project_path {
        // Layer 2: User's per-project override (~/.ftai/projects/<project>/config.toml)
        let user_project_dir = project_config_dir(project)?;
        let user_project_config = user_project_dir.join("config.toml");
        if user_project_config.exists() {
            let contents = std::fs::read_to_string(&user_project_config)
                .context("Failed to read user project config")?;
            let override_config: Config =
                toml::from_str(&contents).context("Failed to parse user project config")?;
            config = merge_config(config, override_config);
        }

        // Layer 3: In-repo config (<project>/.ftai/config.toml)
        let repo_config = project.join(".ftai").join("config.toml");
        if repo_config.exists() {
            let contents = std::fs::read_to_string(&repo_config)
                .context("Failed to read project repo config")?;
            let override_config: Config =
                toml::from_str(&contents).context("Failed to parse project repo config")?;
            config = merge_config(config, override_config);
        }
    }

    Ok(config)
}

/// Merge override config into base. Override values take precedence when set.
fn merge_config(base: Config, override_cfg: Config) -> Config {
    Config {
        model: ModelConfig {
            backend: override_cfg.model.backend,
            path: override_cfg.model.path.or(base.model.path),
            context_length: override_cfg.model.context_length,
            temperature: override_cfg.model.temperature,
            llamacpp: override_cfg.model.llamacpp,
            mlx: override_cfg.model.mlx,
            tool_calling: override_cfg.model.tool_calling,
        },
        permissions: override_cfg.permissions,
        paths: PathsConfig {
            models_dir: override_cfg.paths.models_dir.or(base.paths.models_dir),
            rules_file: override_cfg.paths.rules_file.or(base.paths.rules_file),
        },
        formatting: FormattingConfig {
            templates_dir: override_cfg.formatting.templates_dir.or(base.formatting.templates_dir),
            commit_format: override_cfg.formatting.commit_format.or(base.formatting.commit_format),
            pr_format: override_cfg.formatting.pr_format.or(base.formatting.pr_format),
            comment_style: override_cfg.formatting.comment_style.or(base.formatting.comment_style),
            chat_style: override_cfg.formatting.chat_style.or(base.formatting.chat_style),
            enabled: if override_cfg.formatting.enabled.is_empty() {
                base.formatting.enabled
            } else {
                override_cfg.formatting.enabled
            },
        },
        plugins: PluginsConfig {
            enabled: override_cfg.plugins.enabled,
            registry_url: override_cfg.plugins.registry_url.or(base.plugins.registry_url),
            auto_update: override_cfg.plugins.auto_update,
        },
        theme: override_cfg.theme,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config_serializes() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("context_length"));
        assert!(toml_str.contains("temperature"));
    }

    #[test]
    fn test_config_roundtrip() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.model.context_length, 32768);
        assert_eq!(parsed.permissions.mode, PermissionMode::Auto);
    }

    #[test]
    fn test_config_from_toml() {
        let toml_str = r#"
[model]
backend = "llamacpp"
context_length = 16384
temperature = 0.7

[model.llamacpp]
gpu_layers = 32
threads = 4

[permissions]
mode = "yolo"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.model.backend, BackendType::LlamaCpp);
        assert_eq!(config.model.context_length, 16384);
        assert_eq!(config.model.llamacpp.gpu_layers, 32);
        assert_eq!(config.permissions.mode, PermissionMode::Yolo);
    }

    #[test]
    fn test_merge_config() {
        let base = Config::default();
        let override_toml = r#"
[model]
backend = "llamacpp"
path = "/custom/model.gguf"
context_length = 8192
temperature = 0.5

[permissions]
mode = "ask"
"#;
        let override_cfg: Config = toml::from_str(override_toml).unwrap();
        let merged = merge_config(base, override_cfg);

        assert_eq!(merged.model.backend, BackendType::LlamaCpp);
        assert_eq!(merged.model.path, Some("/custom/model.gguf".to_string()));
        assert_eq!(merged.model.context_length, 8192);
        assert_eq!(merged.permissions.mode, PermissionMode::Ask);
    }

    #[test]
    fn test_project_config_dir_encoding() {
        let path = Path::new("/Users/michael/Developer/Serena");
        let dir = project_config_dir(path).unwrap();
        let dir_name = dir.file_name().unwrap().to_string_lossy();
        assert!(dir_name.contains("Users-michael-Developer-Serena"));
        assert!(!dir_name.starts_with('-'));
    }

    #[test]
    fn test_ensure_ftai_dirs() {
        // This test creates real directories in ~/.ftai/ — safe because it's idempotent
        ensure_ftai_dirs().unwrap();
        let global = global_config_dir().unwrap();
        assert!(global.join("models").exists());
        assert!(global.join("memory").exists());
        assert!(global.join("projects").exists());
        assert!(global.join("config.toml").exists());
        assert!(global.join("rules.ftai").exists());
    }

    #[test]
    fn test_load_config_defaults() {
        let config = load_config(None).unwrap();
        assert_eq!(config.model.context_length, 32768);
    }

    #[test]
    fn test_formatting_config_defaults() {
        let config = Config::default();
        assert!(config.formatting.templates_dir.is_none());
        assert!(config.formatting.commit_format.is_none());
        assert!(config.formatting.enabled.is_empty());
    }

    #[test]
    fn test_formatting_config_from_toml() {
        let toml_str = r#"
[formatting]
commit_format = "type: subject"
enabled = ["commit", "pr"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.formatting.commit_format, Some("type: subject".to_string()));
        assert_eq!(config.formatting.enabled, vec!["commit", "pr"]);
    }

    #[test]
    fn test_merge_formatting_config() {
        let base = Config {
            formatting: FormattingConfig {
                commit_format: Some("base commit".to_string()),
                pr_format: Some("base pr".to_string()),
                enabled: vec!["commit".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };
        let override_cfg = Config {
            formatting: FormattingConfig {
                commit_format: Some("override commit".to_string()),
                enabled: vec!["commit".to_string(), "pr".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_config(base, override_cfg);
        assert_eq!(merged.formatting.commit_format, Some("override commit".to_string()));
        assert_eq!(merged.formatting.pr_format, Some("base pr".to_string()));
        assert_eq!(merged.formatting.enabled, vec!["commit", "pr"]);
    }

    #[test]
    fn test_load_config_with_project_override() {
        let tmp = TempDir::new().unwrap();
        let project_ftai = tmp.path().join(".ftai");
        std::fs::create_dir_all(&project_ftai).unwrap();
        std::fs::write(
            project_ftai.join("config.toml"),
            r#"
[model]
context_length = 4096

[permissions]
mode = "ask"
"#,
        )
        .unwrap();

        let config = load_config(Some(tmp.path())).unwrap();
        assert_eq!(config.model.context_length, 4096);
        assert_eq!(config.permissions.mode, PermissionMode::Ask);
    }

    #[test]
    fn test_project_config_dir_windows_style_path() {
        let path = Path::new("C:\\Users\\foo\\project");
        let dir = project_config_dir(path).unwrap();
        let dir_name = dir.file_name().unwrap().to_string_lossy();
        assert!(!dir_name.contains('\\'));
        assert!(!dir_name.contains('/'));
        assert!(!dir_name.starts_with('-'));
    }

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_security_path_traversal_in_model_path() {
        // P0 security red test
        // Config with path traversal in model path loads without executing anything
        let toml_str = r#"
[model]
path = "../../etc/passwd"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.model.path, Some("../../etc/passwd".to_string()));
        // The path is stored but never executed at config-load time — verification
        // happens at model-load time. This is safe by design.
    }

    #[test]
    fn test_security_extremely_long_strings_no_panic() {
        // P0 security red test
        // Config with 1MB strings doesn't panic during parse
        let long_string = "a".repeat(1_000_000);
        let toml_str = format!(
            r#"
[model]
path = "{long_string}"
"#
        );
        let config: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(config.model.path.as_ref().unwrap().len(), 1_000_000);
    }

    #[test]
    fn test_security_yolo_mode_parses() {
        // P0 security red test
        // PermissionMode::Yolo loads correctly — hard block enforcement is tested
        // at the permission gate layer, not here, but we verify it round-trips.
        let toml_str = r#"
[permissions]
mode = "yolo"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.permissions.mode, PermissionMode::Yolo);
    }

    #[test]
    fn test_security_null_bytes_in_project_path() {
        // P0 security red test
        // project_config_dir with null bytes in path doesn't panic
        let path = Path::new("/some/path\x00with/null");
        let result = project_config_dir(path);
        // Should succeed — the null byte gets replaced during lossy conversion
        assert!(result.is_ok());
        let dir = result.unwrap();
        let dir_name = dir.file_name().unwrap().to_string_lossy();
        // Must not contain path separators
        assert!(!dir_name.contains('/'));
    }

    #[test]
    fn test_security_malformed_toml_returns_error() {
        // P0 security red test
        // Malformed TOML returns Err, never panics
        let unclosed_string = "x = \"".to_string();
        let bad_inputs = vec![
            "[model",                           // unclosed section
            "key = ",                            // missing value
            "[[[nested]]]",                      // triple bracket
            "= value",                           // missing key
            "\x00\x01\x02",                      // binary garbage
            &unclosed_string,                    // unclosed string
            "[model]\nbackend = 42",             // wrong type for enum
        ];
        for input in bad_inputs {
            let result: Result<Config, _> = toml::from_str(input);
            assert!(result.is_err(), "Expected error for malformed TOML: {:?}", input);
        }
    }
}
