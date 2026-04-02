mod loader;
pub mod templates;

pub use loader::{Config, ModelConfig, BackendType, PermissionMode, ToolCallingMode, PluginsConfig, ThemeConfig, ThemePreset, ApiConfig, load_config, ensure_ftai_dirs, global_config_dir, project_config_dir};
