use anyhow::{Context, Result};

use crate::config::{BackendType, Config};
use super::api_client::ApiClient;
use super::http_client::HttpModelClient;
use super::llamacpp::LlamaCppServer;
use super::mlx::MlxServer;
use super::types::{ChatRequest, ChatResponse, HardwareInfo, Token, ToolDefinition};
use tokio::sync::mpsc;

const LLAMACPP_PORT: u16 = 8411;
const MLX_PORT: u16 = 8412;

/// Manages the active model backend
pub enum BackendManager {
    LlamaCpp(LlamaCppServer),
    Mlx(MlxServer),
    /// External OpenAI-compatible server (Ollama, LM Studio, etc.)
    #[allow(dead_code)]
    External(HttpModelClient),
    /// Cloud API backend (Anthropic, OpenAI, etc.)
    Api(ApiClient),
}

impl BackendManager {
    /// Create a backend manager based on config.
    /// If API is enabled and key is available, uses cloud API backend.
    /// Falls back to llama.cpp if MLX is requested on a non-Apple-Silicon platform.
    pub fn from_config(config: &Config) -> Self {
        // Check if API backend is enabled and key is available
        if config.api.enabled {
            if let Some(_key) = super::api_client::resolve_api_key(&config.api) {
                match ApiClient::from_config(&config.api) {
                    Ok(client) => return BackendManager::Api(client),
                    Err(e) => {
                        eprintln!("Warning: API backend configured but failed to initialize: {e}. Falling back to local.");
                    }
                }
            } else {
                eprintln!("Warning: API backend enabled but no API key found. Falling back to local backend.");
            }
        }

        let effective_backend = if config.model.backend == BackendType::Mlx
            && !crate::backend::mlx::is_available()
        {
            eprintln!(
                "Warning: MLX backend requires Apple Silicon Mac. Falling back to llama.cpp."
            );
            BackendType::LlamaCpp
        } else {
            config.model.backend.clone()
        };

        match effective_backend {
            BackendType::LlamaCpp | BackendType::Direct => {
                BackendManager::LlamaCpp(LlamaCppServer::new(LLAMACPP_PORT))
            }
            BackendType::Mlx => BackendManager::Mlx(MlxServer::new(MLX_PORT)),
        }
    }

    /// Create a backend that connects to an external server
    #[allow(dead_code)]
    pub fn external(base_url: &str) -> Self {
        BackendManager::External(HttpModelClient::new(base_url))
    }

    /// Resolve a bare model name to a full local path.
    /// If the path already contains separators, returns it as-is.
    pub fn resolve_path(raw_path: &str) -> String {
        if !raw_path.contains('/') && !raw_path.contains('\\') {
            if let Ok(config_dir) = crate::config::global_config_dir() {
                let models_dir = config_dir.join("models").join(raw_path);
                if models_dir.exists() {
                    return resolve_model_path(&models_dir)
                        .unwrap_or_else(|| models_dir.to_string_lossy().to_string());
                }
            }
        }
        raw_path.to_string()
    }

    /// Spawn the backend server without waiting for it to be ready.
    /// Call `wait_until_ready()` later to block until the model is loaded.
    pub fn spawn_only(&mut self, config: &Config) -> Result<()> {
        match self {
            BackendManager::Api(_) | BackendManager::External(_) => Ok(()),
            _ => {
                let raw_path = config
                    .model
                    .path
                    .as_deref()
                    .context("No model path configured. Run `forge model install` or set model.path in config.")?;
                let resolved = Self::resolve_path(raw_path);
                match self {
                    BackendManager::LlamaCpp(server) => server.spawn_only(
                        &resolved,
                        config.model.llamacpp.gpu_layers,
                        config.model.llamacpp.threads,
                        config.model.context_length,
                    ),
                    _ => Ok(()),
                }
            }
        }
    }

    /// Wait for an already-spawned backend to become ready.
    pub async fn wait_until_ready(&mut self) -> Result<()> {
        match self {
            BackendManager::LlamaCpp(server) => server.wait_until_ready().await,
            _ => Ok(()),
        }
    }

    /// Load and start the model (blocking — waits for model to be ready).
    /// Used by non-TUI paths (CLI commands, tests). The TUI uses spawn_only + wait_until_ready.
    #[allow(dead_code)]
    pub async fn start(&mut self, config: &Config) -> Result<()> {
        match self {
            BackendManager::Api(_) => {
                // Cloud API — no local server to start
                Ok(())
            }
            _ => {
                let raw_path = config
                    .model
                    .path
                    .as_deref()
                    .filter(|p| !p.is_empty())
                    .map(|p| p.to_string())
                    .or_else(discover_first_installed_model)
                    .context(
                        "No model path configured. Run `forge model install` then `forge model use <name>`, or set model.path in config.",
                    )?;

                let resolved = Self::resolve_path(&raw_path);
                let model_path = resolved.as_str();

                match self {
                    BackendManager::LlamaCpp(server) => {
                        server
                            .start(
                                model_path,
                                config.model.llamacpp.gpu_layers,
                                config.model.llamacpp.threads,
                                config.model.context_length,
                            )
                            .await
                    }
                    BackendManager::Mlx(server) => {
                        server.start(model_path, config.model.context_length).await
                    }
                    BackendManager::External(_) => {
                        // External server is already running
                        Ok(())
                    }
                    BackendManager::Api(_) => unreachable!(),
                }
            }
        }
    }

    /// Stop the backend server
    #[allow(dead_code)]
    pub fn stop(&mut self) {
        match self {
            BackendManager::LlamaCpp(server) => server.stop(),
            BackendManager::Mlx(server) => server.stop(),
            BackendManager::External(_) => {}
            BackendManager::Api(_) => {}
        }
    }

    fn http_client(&self) -> Option<&HttpModelClient> {
        match self {
            BackendManager::LlamaCpp(server) => Some(server.client()),
            BackendManager::Mlx(server) => Some(server.client()),
            BackendManager::External(client) => Some(client),
            BackendManager::Api(_) => None,
        }
    }

    /// Generate a complete response
    pub async fn generate(&self, request: &ChatRequest) -> Result<ChatResponse> {
        match self {
            BackendManager::Api(client) => client.generate(request).await,
            _ => self.http_client().unwrap().generate(request).await,
        }
    }

    /// Generate a streaming response
    pub async fn generate_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<(mpsc::Receiver<Token>, tokio::task::JoinHandle<Result<ChatResponse>>)> {
        match self {
            BackendManager::Api(client) => client.generate_stream(request).await,
            _ => self.http_client().unwrap().generate_stream(request).await,
        }
    }

    /// Check if backend is available
    pub async fn health_check(&self) -> bool {
        match self {
            BackendManager::Api(client) => client.health_check().await,
            _ => self.http_client().unwrap().health_check().await,
        }
    }

    /// Pre-warm the prompt cache by sending the system prompt to the server.
    /// This processes and caches the system prompt so the first real user message
    /// only needs to process the new tokens. Fire-and-forget — errors are ignored.
    pub async fn warm_up_prompt(&self, system_prompt: &str, tools: Vec<ToolDefinition>) {
        let request = ChatRequest {
            messages: vec![
                crate::backend::types::Message {
                    role: crate::backend::types::Role::System,
                    content: system_prompt.to_string(),
                    tool_calls: None,
                    tool_call_id: None,
                },
                crate::backend::types::Message {
                    role: crate::backend::types::Role::User,
                    content: "hi".to_string(),
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            temperature: 0.0,
            max_tokens: Some(1),
            model_id: None,
            tools,
        };
        let _ = self.generate(&request).await;
    }

    /// Get hardware info and model recommendation
    pub fn hardware_info() -> HardwareInfo {
        HardwareInfo::detect()
    }

    pub fn backend_name(&self) -> &str {
        match self {
            BackendManager::LlamaCpp(_) => "llama.cpp",
            BackendManager::Mlx(_) => "MLX",
            BackendManager::External(_) => "external",
            BackendManager::Api(_) => "api",
        }
    }

    /// Get a mutable reference to the Api client, if this is an API backend
    pub fn api_client_mut(&mut self) -> Option<&mut ApiClient> {
        match self {
            BackendManager::Api(client) => Some(client),
            _ => None,
        }
    }

    /// Get a reference to the Api client, if this is an API backend
    #[allow(dead_code)]
    pub fn api_client(&self) -> Option<&ApiClient> {
        match self {
            BackendManager::Api(client) => Some(client),
            _ => None,
        }
    }
}

/// If `model.path` is unset, use the first usable model under `~/.ftai/models/`
/// (alphabetical by directory name). Matches the "first run" expectation when
/// the user ran `forge model install` but not `forge model use`.
fn discover_first_installed_model() -> Option<String> {
    let models_dir = crate::config::global_config_dir().ok()?.join("models");
    let mut dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&models_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for dir in dirs {
        if let Some(path) = resolve_model_path(&dir) {
            return Some(path);
        }
    }
    None
}

/// Resolve a model directory to the appropriate path for the backend.
/// Safetensors/MLX models (directory) take priority over GGUF (single file),
/// because if both exist, MLX is the preferred backend on Apple Silicon.
fn resolve_model_path(dir: &std::path::Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut has_safetensors = false;
    let mut gguf_path: Option<String> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            match ext {
                "safetensors" => has_safetensors = true,
                "gguf" if gguf_path.is_none() => {
                    gguf_path = Some(path.to_string_lossy().to_string());
                }
                _ => {}
            }
        }
    }
    // Prefer safetensors (MLX directory) over GGUF
    if has_safetensors {
        return Some(dir.to_string_lossy().to_string());
    }
    gguf_path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_config_mlx() {
        let config = Config::default();
        let manager = BackendManager::from_config(&config);
        if crate::backend::mlx::is_available() {
            assert_eq!(manager.backend_name(), "MLX");
        } else {
            assert_eq!(manager.backend_name(), "llama.cpp");
        }
    }

    #[test]
    fn test_from_config_llamacpp() {
        let mut config = Config::default();
        config.model.backend = BackendType::LlamaCpp;
        let manager = BackendManager::from_config(&config);
        assert_eq!(manager.backend_name(), "llama.cpp");
    }

    #[test]
    fn test_external_backend() {
        let manager = BackendManager::external("http://localhost:11434");
        assert_eq!(manager.backend_name(), "external");
    }

    #[test]
    fn test_api_backend_from_config_when_enabled() {
        std::env::set_var("TEST_MGR_API_KEY", "sk-test-manager-key");
        let mut config = Config::default();
        config.api.enabled = true;
        config.api.api_key_env = Some("TEST_MGR_API_KEY".into());
        config.api.provider = "anthropic".into();

        let manager = BackendManager::from_config(&config);
        assert_eq!(manager.backend_name(), "api");
        assert!(manager.api_client().is_some());

        std::env::remove_var("TEST_MGR_API_KEY");
    }

    #[test]
    fn test_api_backend_fallback_when_no_key() {
        let mut config = Config::default();
        config.api.enabled = true;
        config.api.api_key = None;
        config.api.api_key_env = Some("NONEXISTENT_MGR_KEY_99999".into());

        let manager = BackendManager::from_config(&config);
        // Should fall back to local backend, not api
        assert_ne!(manager.backend_name(), "api");
    }

    #[test]
    fn test_api_backend_disabled_uses_local() {
        std::env::set_var("TEST_MGR_DISABLED_KEY", "sk-test-key");
        let mut config = Config::default();
        config.api.enabled = false;
        config.api.api_key_env = Some("TEST_MGR_DISABLED_KEY".into());

        let manager = BackendManager::from_config(&config);
        assert_ne!(manager.backend_name(), "api");

        std::env::remove_var("TEST_MGR_DISABLED_KEY");
    }
}
