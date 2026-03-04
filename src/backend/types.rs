use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tokio::sync::mpsc;

/// A single token emitted during streaming generation
#[derive(Debug, Clone)]
pub struct Token {
    pub text: String,
    pub is_final: bool,
}

/// Role in a conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A single message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Tool call requests from the assistant
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Tool call ID this message is responding to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// A tool call request from the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Tool definition provided to the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Request to generate a response
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub temperature: f64,
    pub max_tokens: Option<usize>,
}

/// Response from the model
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub message: Message,
    pub tokens_used: TokenUsage,
    pub stop_reason: StopReason,
}

/// Token usage statistics
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
}

impl TokenUsage {
    pub fn total(&self) -> usize {
        self.prompt_tokens + self.completion_tokens
    }
}

/// Why the model stopped generating
#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    EndOfText,
    MaxTokens,
    ToolCall,
}

/// Type alias for the streaming token sender
pub type TokenStream = mpsc::Receiver<Token>;

/// The core backend trait — implemented by llama.cpp and MLX
pub trait ModelBackend: Send + Sync {
    /// Load a model from the given path
    fn load_model(
        &mut self,
        model_path: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>>;

    /// Generate a complete response (non-streaming)
    fn generate(
        &self,
        request: &ChatRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ChatResponse>> + Send + '_>>;

    /// Generate a streaming response — tokens arrive on the returned channel
    fn generate_stream(
        &self,
        request: &ChatRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(TokenStream, tokio::task::JoinHandle<Result<ChatResponse>>)>> + Send + '_>>;

    /// Whether this model supports native tool/function calling
    fn supports_tool_calling(&self) -> bool;

    /// Maximum context length in tokens
    fn max_context_length(&self) -> usize;

    /// Model name/identifier
    fn model_name(&self) -> &str;

    /// Whether a model is currently loaded
    fn is_loaded(&self) -> bool;
}

/// Hardware capabilities detected at runtime
#[derive(Debug, Clone)]
pub struct HardwareInfo {
    pub arch: CpuArch,
    pub gpu: GpuType,
    pub ram_gb: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CpuArch {
    AppleSilicon,
    X86_64,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum GpuType {
    Metal,
    Cuda { vram_gb: u64 },
    None,
}

impl HardwareInfo {
    /// Detect current hardware capabilities
    pub fn detect() -> Self {
        let arch = if cfg!(target_arch = "aarch64") && cfg!(target_os = "macos") {
            CpuArch::AppleSilicon
        } else if cfg!(target_arch = "x86_64") {
            CpuArch::X86_64
        } else {
            CpuArch::Other(std::env::consts::ARCH.to_string())
        };

        let gpu = if cfg!(target_os = "macos") && arch == CpuArch::AppleSilicon {
            GpuType::Metal
        } else {
            // TODO: CUDA detection via nvidia-smi
            GpuType::None
        };

        let ram_gb = Self::detect_ram_gb();

        Self { arch, gpu, ram_gb }
    }

    #[cfg(target_os = "macos")]
    fn detect_ram_gb() -> u64 {
        use std::process::Command;
        Command::new("sysctl")
            .arg("-n")
            .arg("hw.memsize")
            .output()
            .ok()
            .and_then(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
            .map(|bytes| bytes / (1024 * 1024 * 1024))
            .unwrap_or(8)
    }

    #[cfg(not(target_os = "macos"))]
    fn detect_ram_gb() -> u64 {
        // Fallback — read /proc/meminfo on Linux
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemTotal"))
                    .and_then(|l| {
                        l.split_whitespace()
                            .nth(1)
                            .and_then(|v| v.parse::<u64>().ok())
                    })
            })
            .map(|kb| kb / (1024 * 1024))
            .unwrap_or(8)
    }

    /// Recommend a model based on hardware
    pub fn recommended_model(&self) -> ModelRecommendation {
        match (&self.arch, &self.gpu, self.ram_gb) {
            (CpuArch::AppleSilicon, _, ram) if ram >= 32 => ModelRecommendation {
                name: "Qwen2.5-Coder-32B-Q4".to_string(),
                backend: crate::config::BackendType::Mlx,
                size_gb: 18,
            },
            (CpuArch::AppleSilicon, _, ram) if ram >= 16 => ModelRecommendation {
                name: "Qwen2.5-Coder-7B-Q4".to_string(),
                backend: crate::config::BackendType::Mlx,
                size_gb: 4,
            },
            (CpuArch::AppleSilicon, _, _) => ModelRecommendation {
                name: "Qwen2.5-Coder-3B-Q4".to_string(),
                backend: crate::config::BackendType::Mlx,
                size_gb: 2,
            },
            (_, GpuType::Cuda { vram_gb }, _) if *vram_gb >= 24 => ModelRecommendation {
                name: "Qwen2.5-Coder-32B-Q4".to_string(),
                backend: crate::config::BackendType::LlamaCpp,
                size_gb: 18,
            },
            (_, GpuType::Cuda { .. }, _) => ModelRecommendation {
                name: "DeepSeek-Coder-V2-Lite-Q4".to_string(),
                backend: crate::config::BackendType::LlamaCpp,
                size_gb: 9,
            },
            _ => ModelRecommendation {
                name: "Qwen2.5-Coder-7B-Q4".to_string(),
                backend: crate::config::BackendType::LlamaCpp,
                size_gb: 4,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelRecommendation {
    pub name: String,
    pub backend: crate::config::BackendType,
    pub size_gb: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_detection() {
        let hw = HardwareInfo::detect();
        assert!(hw.ram_gb > 0);
        // On macOS with Apple Silicon
        if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            assert_eq!(hw.arch, CpuArch::AppleSilicon);
            assert_eq!(hw.gpu, GpuType::Metal);
        }
    }

    #[test]
    fn test_model_recommendation_16gb_mac() {
        let hw = HardwareInfo {
            arch: CpuArch::AppleSilicon,
            gpu: GpuType::Metal,
            ram_gb: 16,
        };
        let rec = hw.recommended_model();
        assert_eq!(rec.name, "Qwen2.5-Coder-7B-Q4");
        assert_eq!(rec.backend, crate::config::BackendType::Mlx);
    }

    #[test]
    fn test_model_recommendation_32gb_mac() {
        let hw = HardwareInfo {
            arch: CpuArch::AppleSilicon,
            gpu: GpuType::Metal,
            ram_gb: 32,
        };
        let rec = hw.recommended_model();
        assert_eq!(rec.name, "Qwen2.5-Coder-32B-Q4");
    }

    #[test]
    fn test_model_recommendation_nvidia() {
        let hw = HardwareInfo {
            arch: CpuArch::X86_64,
            gpu: GpuType::Cuda { vram_gb: 24 },
            ram_gb: 32,
        };
        let rec = hw.recommended_model();
        assert_eq!(rec.backend, crate::config::BackendType::LlamaCpp);
    }

    #[test]
    fn test_model_recommendation_cpu_only() {
        let hw = HardwareInfo {
            arch: CpuArch::X86_64,
            gpu: GpuType::None,
            ram_gb: 16,
        };
        let rec = hw.recommended_model();
        assert_eq!(rec.name, "Qwen2.5-Coder-7B-Q4");
        assert_eq!(rec.backend, crate::config::BackendType::LlamaCpp);
    }

    #[test]
    fn test_token_usage() {
        let usage = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
        };
        assert_eq!(usage.total(), 150);
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message {
            role: Role::User,
            content: "Hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(!json.contains("tool_calls")); // skip_serializing_if = None
    }

    #[test]
    fn test_tool_call_serialization() {
        let tc = ToolCall {
            id: "tc_1".to_string(),
            name: "file_read".to_string(),
            arguments: serde_json::json!({"path": "/foo/bar.rs"}),
        };
        let json = serde_json::to_string(&tc).unwrap();
        assert!(json.contains("file_read"));
        assert!(json.contains("/foo/bar.rs"));
    }
}
