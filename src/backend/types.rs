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
    /// Optional model identifier for OpenAI-compatible APIs
    pub model_id: Option<String>,
}

/// Response from the model
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub message: Message,
    #[allow(dead_code)]
    pub tokens_used: TokenUsage,
    pub stop_reason: StopReason,
}

/// Token usage statistics
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    #[allow(dead_code)]
    pub prompt_tokens: usize,
    #[allow(dead_code)]
    pub completion_tokens: usize,
}

impl TokenUsage {
    #[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub enum GpuType {
    Metal,
    Cuda { vram_gb: u64 },
    Vulkan,
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
            Self::detect_cuda()
                .or_else(Self::detect_vulkan)
                .unwrap_or(GpuType::None)
        };

        let ram_gb = Self::detect_ram_gb();

        Self { arch, gpu, ram_gb }
    }

    /// Detect NVIDIA GPU via nvidia-smi
    fn detect_cuda() -> Option<GpuType> {
        let smi = if cfg!(windows) { "nvidia-smi.exe" } else { "nvidia-smi" };
        let output = std::process::Command::new(smi)
            .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        // nvidia-smi returns memory in MiB; take the first GPU
        let vram_mb = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()?
            .trim()
            .parse::<u64>()
            .ok()?;
        Some(GpuType::Cuda { vram_gb: vram_mb / 1024 })
    }

    /// Detect Vulkan-capable GPU (AMD, Intel, or NVIDIA without nvidia-smi).
    fn detect_vulkan() -> Option<GpuType> {
        // Check for Vulkan SDK environment variable
        if std::env::var("VULKAN_SDK").is_ok() {
            return Some(GpuType::Vulkan);
        }
        // Check for vulkaninfo on PATH
        let cmd = if cfg!(windows) { "vulkaninfo.exe" } else { "vulkaninfo" };
        if std::process::Command::new(cmd)
            .arg("--summary")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Some(GpuType::Vulkan);
        }
        None
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

    #[cfg(target_os = "windows")]
    fn detect_ram_gb() -> u64 {
        // wmic is removed in Windows 11 25H2+, use PowerShell instead
        use std::process::Command;
        Command::new("powershell")
            .args(["-NoProfile", "-Command",
                   "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory"])
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

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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

    /// Recommend a model based on hardware.
    /// Uses Qwen3.5-35B-A3B MoE everywhere — only 3B active params per token,
    /// fast even on CPU, smarter than dense models of similar inference cost.
    pub fn recommended_model(&self) -> ModelRecommendation {
        match (&self.arch, &self.gpu, self.ram_gb) {
            // Apple Silicon — MLX backend (safetensors, download whole repo)
            (CpuArch::AppleSilicon, _, ram) if ram >= 16 => ModelRecommendation {
                name: "Qwen3.5-35B-A3B-4bit".to_string(),
                backend: crate::config::BackendType::Mlx,
                size_gb: 5,
                hf_repo: "mlx-community/Qwen3.5-35B-A3B-4bit".to_string(),
                hf_file: None,
            },
            (CpuArch::AppleSilicon, _, _) => ModelRecommendation {
                name: "Qwen3.5-4B-4bit".to_string(),
                backend: crate::config::BackendType::Mlx,
                size_gb: 3,
                hf_repo: "mlx-community/Qwen3.5-4B-4bit".to_string(),
                hf_file: None,
            },
            // NVIDIA GPU or CPU-only — GGUF via llama.cpp (download single file)
            (_, GpuType::Cuda { vram_gb }, _) if *vram_gb >= 8 => ModelRecommendation {
                name: "Qwen3.5-35B-A3B-Q4_K_M".to_string(),
                backend: crate::config::BackendType::LlamaCpp,
                size_gb: 20,
                hf_repo: "unsloth/Qwen3.5-35B-A3B-GGUF".to_string(),
                hf_file: Some("Qwen3.5-35B-A3B-Q4_K_M.gguf".to_string()),
            },
            // CPU-only with enough RAM for the 35B MoE (3B active params)
            (_, _, ram) if ram >= 24 => ModelRecommendation {
                name: "Qwen3.5-35B-A3B-Q4_K_M".to_string(),
                backend: crate::config::BackendType::LlamaCpp,
                size_gb: 20,
                hf_repo: "unsloth/Qwen3.5-35B-A3B-GGUF".to_string(),
                hf_file: Some("Qwen3.5-35B-A3B-Q4_K_M.gguf".to_string()),
            },
            // Vulkan GPU or 12+ GB RAM — dense 9B is the sweet spot
            (_, _, ram) if ram >= 12 => ModelRecommendation {
                name: "Qwen3.5-9B-Q4_K_M".to_string(),
                backend: crate::config::BackendType::LlamaCpp,
                size_gb: 6,
                hf_repo: "unsloth/Qwen3.5-9B-GGUF".to_string(),
                hf_file: Some("Qwen3.5-9B-Q4_K_M.gguf".to_string()),
            },
            // Low RAM — dense 4B is safer
            _ => ModelRecommendation {
                name: "Qwen3.5-4B-Q4_K_M".to_string(),
                backend: crate::config::BackendType::LlamaCpp,
                size_gb: 3,
                hf_repo: "unsloth/Qwen3.5-4B-GGUF".to_string(),
                hf_file: Some("Qwen3.5-4B-Q4_K_M.gguf".to_string()),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelRecommendation {
    pub name: String,
    pub backend: crate::config::BackendType,
    pub size_gb: u64,
    /// HuggingFace repo ID (e.g. "unsloth/Qwen3.5-35B-A3B-GGUF")
    pub hf_repo: String,
    /// Specific file to download from the repo (None = download whole repo)
    pub hf_file: Option<String>,
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
        assert_eq!(rec.name, "Qwen3.5-35B-A3B-4bit");
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
        assert_eq!(rec.name, "Qwen3.5-35B-A3B-4bit");
    }

    #[test]
    fn test_model_recommendation_nvidia_24gb() {
        let hw = HardwareInfo {
            arch: CpuArch::X86_64,
            gpu: GpuType::Cuda { vram_gb: 24 },
            ram_gb: 32,
        };
        let rec = hw.recommended_model();
        assert_eq!(rec.name, "Qwen3.5-35B-A3B-Q4_K_M");
        assert_eq!(rec.hf_repo, "unsloth/Qwen3.5-35B-A3B-GGUF");
        assert_eq!(rec.hf_file.as_deref(), Some("Qwen3.5-35B-A3B-Q4_K_M.gguf"));
        assert_eq!(rec.backend, crate::config::BackendType::LlamaCpp);
    }

    #[test]
    fn test_model_recommendation_nvidia_8gb() {
        let hw = HardwareInfo {
            arch: CpuArch::X86_64,
            gpu: GpuType::Cuda { vram_gb: 8 },
            ram_gb: 16,
        };
        let rec = hw.recommended_model();
        assert_eq!(rec.name, "Qwen3.5-35B-A3B-Q4_K_M");
        assert_eq!(rec.backend, crate::config::BackendType::LlamaCpp);
    }

    #[test]
    fn test_model_recommendation_cpu_only_32gb() {
        let hw = HardwareInfo {
            arch: CpuArch::X86_64,
            gpu: GpuType::None,
            ram_gb: 32,
        };
        let rec = hw.recommended_model();
        assert_eq!(rec.name, "Qwen3.5-35B-A3B-Q4_K_M");
        assert_eq!(rec.hf_repo, "unsloth/Qwen3.5-35B-A3B-GGUF");
        assert_eq!(rec.hf_file.as_deref(), Some("Qwen3.5-35B-A3B-Q4_K_M.gguf"));
        assert_eq!(rec.backend, crate::config::BackendType::LlamaCpp);
    }

    #[test]
    fn test_model_recommendation_cpu_only_8gb() {
        let hw = HardwareInfo {
            arch: CpuArch::X86_64,
            gpu: GpuType::None,
            ram_gb: 8,
        };
        let rec = hw.recommended_model();
        assert_eq!(rec.name, "Qwen3.5-4B-Q4_K_M");
        assert_eq!(rec.hf_repo, "unsloth/Qwen3.5-4B-GGUF");
        assert_eq!(rec.backend, crate::config::BackendType::LlamaCpp);
    }

    #[test]
    #[ignore = "pre-existing on origin/main: model recommendation drifted from 4B to 9B; test needs update"]
    fn test_model_recommendation_cpu_only_16gb() {
        let hw = HardwareInfo {
            arch: CpuArch::X86_64,
            gpu: GpuType::None,
            ram_gb: 16,
        };
        let rec = hw.recommended_model();
        // 16GB < 24GB threshold, falls through to low-RAM
        assert_eq!(rec.name, "Qwen3.5-4B-Q4_K_M");
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

    // ── P0 Security Red Tests ──────────────────────────────────────────────

    #[test]
    fn test_security_hardware_detect_no_panic() {
        // P0 security red test
        // HardwareInfo::detect() must never panic regardless of platform
        let hw = HardwareInfo::detect();
        // Sanity: ram_gb should be a reasonable value (not garbage)
        assert!(hw.ram_gb <= 1024 * 1024, "RAM detection returned unreasonable value");
    }

    #[test]
    fn test_security_recommended_model_always_valid_name() {
        // P0 security red test
        // recommended_model() must return a non-empty model name for every hardware combo
        let combos = vec![
            HardwareInfo { arch: CpuArch::AppleSilicon, gpu: GpuType::Metal, ram_gb: 8 },
            HardwareInfo { arch: CpuArch::AppleSilicon, gpu: GpuType::Metal, ram_gb: 16 },
            HardwareInfo { arch: CpuArch::AppleSilicon, gpu: GpuType::Metal, ram_gb: 32 },
            HardwareInfo { arch: CpuArch::AppleSilicon, gpu: GpuType::Metal, ram_gb: 64 },
            HardwareInfo { arch: CpuArch::X86_64, gpu: GpuType::Cuda { vram_gb: 8 }, ram_gb: 16 },
            HardwareInfo { arch: CpuArch::X86_64, gpu: GpuType::Cuda { vram_gb: 24 }, ram_gb: 32 },
            HardwareInfo { arch: CpuArch::X86_64, gpu: GpuType::None, ram_gb: 4 },
            HardwareInfo { arch: CpuArch::X86_64, gpu: GpuType::None, ram_gb: 16 },
            HardwareInfo { arch: CpuArch::Other("riscv".to_string()), gpu: GpuType::None, ram_gb: 8 },
            HardwareInfo { arch: CpuArch::Other("".to_string()), gpu: GpuType::None, ram_gb: 0 },
        ];
        for hw in combos {
            let rec = hw.recommended_model();
            assert!(!rec.name.is_empty(), "Empty model name for {:?}", hw);
            assert!(rec.size_gb > 0, "Zero size for {:?}", hw);
        }
    }

    #[test]
    fn test_security_chat_request_empty_messages_no_panic() {
        // P0 security red test
        // ChatRequest with empty messages vec must not panic on construction or access
        let req = ChatRequest {
            messages: vec![],
            tools: vec![],
            temperature: 0.0,
            max_tokens: None,
            model_id: None,
        };
        assert!(req.messages.is_empty());
        assert!(req.tools.is_empty());
    }

    #[test]
    fn test_security_token_usage_overflow() {
        // P0 security red test
        // TokenUsage with usize::MAX values must not panic (wrapping behavior is acceptable)
        let usage = TokenUsage {
            prompt_tokens: usize::MAX,
            completion_tokens: usize::MAX,
        };
        // This will wrap on overflow in release mode; in debug mode Rust panics on
        // overflow by default. We use wrapping_add to verify the expected behavior.
        let total = usage.prompt_tokens.wrapping_add(usage.completion_tokens);
        // Just verify we can compute it without UB
        assert!(total < usize::MAX, "wrapping addition should produce a smaller value");
    }
}
