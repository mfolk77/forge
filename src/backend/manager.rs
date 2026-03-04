use anyhow::{Context, Result};

use crate::config::{BackendType, Config};
use super::http_client::HttpModelClient;
use super::llamacpp::LlamaCppServer;
use super::mlx::MlxServer;
use super::types::{ChatRequest, ChatResponse, HardwareInfo, Token};
use tokio::sync::mpsc;

const LLAMACPP_PORT: u16 = 8411;
const MLX_PORT: u16 = 8412;

/// Manages the active model backend
pub enum BackendManager {
    LlamaCpp(LlamaCppServer),
    Mlx(MlxServer),
    /// External OpenAI-compatible server (Ollama, LM Studio, etc.)
    External(HttpModelClient),
}

impl BackendManager {
    /// Create a backend manager based on config
    pub fn from_config(config: &Config) -> Self {
        match config.model.backend {
            BackendType::LlamaCpp => BackendManager::LlamaCpp(LlamaCppServer::new(LLAMACPP_PORT)),
            BackendType::Mlx => BackendManager::Mlx(MlxServer::new(MLX_PORT)),
        }
    }

    /// Create a backend that connects to an external server
    pub fn external(base_url: &str) -> Self {
        BackendManager::External(HttpModelClient::new(base_url))
    }

    /// Load and start the model
    pub async fn start(&mut self, config: &Config) -> Result<()> {
        let model_path = config
            .model
            .path
            .as_deref()
            .context("No model path configured. Run `ftai model install` or set model.path in config.")?;

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
        }
    }

    /// Stop the backend server
    pub fn stop(&mut self) {
        match self {
            BackendManager::LlamaCpp(server) => server.stop(),
            BackendManager::Mlx(server) => server.stop(),
            BackendManager::External(_) => {}
        }
    }

    fn client(&self) -> &HttpModelClient {
        match self {
            BackendManager::LlamaCpp(server) => server.client(),
            BackendManager::Mlx(server) => server.client(),
            BackendManager::External(client) => client,
        }
    }

    /// Generate a complete response
    pub async fn generate(&self, request: &ChatRequest) -> Result<ChatResponse> {
        self.client().generate(request).await
    }

    /// Generate a streaming response
    pub async fn generate_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<(mpsc::Receiver<Token>, tokio::task::JoinHandle<Result<ChatResponse>>)> {
        self.client().generate_stream(request).await
    }

    /// Check if backend is available
    pub async fn health_check(&self) -> bool {
        self.client().health_check().await
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_config_mlx() {
        let config = Config::default();
        let manager = BackendManager::from_config(&config);
        assert_eq!(manager.backend_name(), "MLX");
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
}
