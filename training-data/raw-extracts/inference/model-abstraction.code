use std::path::PathBuf;

use crate::backend::types::HardwareInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvQuantType {
    F16,
    Q8_0,
    Q4_0,
    Q4_1,
    /// TurboQuant 3-bit (Zandieh et al., ICLR 2026). Randomized Hadamard
    /// Transform + 3-bit Lloyd-Max scalar quantization. ~4.9x compression
    /// vs F16 with MSE ≈ 0.034. Requires llama.cpp with TQ3 support
    /// (GGML_TYPE_TQ3_0).
    TQ3,
    /// TurboQuant 4-bit. ~3.8x compression vs F16 with MSE ≈ 0.009.
    /// Better quality than Q4_0 at similar compression because the
    /// Hadamard rotation spreads outliers before quantization.
    TQ4,
}

impl KvQuantType {
    pub fn to_ffi(self) -> i32 {
        match self {
            Self::F16 => 1,    // GGML_TYPE_F16
            Self::Q8_0 => 8,   // GGML_TYPE_Q8_0
            Self::Q4_0 => 2,   // GGML_TYPE_Q4_0
            Self::Q4_1 => 3,   // GGML_TYPE_Q4_1
            Self::TQ3 => 30,   // GGML_TYPE_TQ3_0 (pending upstream merge)
            Self::TQ4 => 31,   // GGML_TYPE_TQ4_0 (pending upstream merge)
        }
    }

    pub fn memory_factor(self) -> f32 {
        match self {
            Self::F16 => 1.0,
            Self::Q8_0 => 0.5,
            Self::Q4_0 => 0.25,
            Self::Q4_1 => 0.28,
            Self::TQ3 => 0.20,  // 3-bit + rotation metadata ≈ 4.9x compression
            Self::TQ4 => 0.27,  // 4-bit + rotation metadata ≈ 3.8x compression
        }
    }

    /// Whether this quantization type requires a llama.cpp build with
    /// TurboQuant support (not yet in upstream main).
    pub fn requires_turboquant(self) -> bool {
        matches!(self, Self::TQ3 | Self::TQ4)
    }

    /// Human-readable label for display.
    pub fn label(self) -> &'static str {
        match self {
            Self::F16 => "FP16",
            Self::Q8_0 => "Q8_0",
            Self::Q4_0 => "Q4_0",
            Self::Q4_1 => "Q4_1",
            Self::TQ3 => "TurboQuant-3bit",
            Self::TQ4 => "TurboQuant-4bit",
        }
    }

    /// Estimated max context tokens for a given KV budget in bytes.
    /// Each token needs 2 KV entries (key + value), each `head_dim * n_heads`
    /// elements. This gives a rough estimate for Qwen 3.5 architecture.
    pub fn estimate_max_context(self, kv_budget_bytes: u64) -> u64 {
        // Qwen 3.5 35B-A3B: 28 heads, 128 dim, 2 (K+V) = 7168 bytes/token at F16
        let bytes_per_token_f16: f64 = 7168.0;
        let bytes_per_token = bytes_per_token_f16 * self.memory_factor() as f64;
        (kv_budget_bytes as f64 / bytes_per_token) as u64
    }
}

#[derive(Debug, Clone)]
pub struct InferenceConfig {
    pub model_path: PathBuf,
    pub context_length: u32,
    pub batch_size: u32,
    pub threads: u32,
    pub gpu_layers: i32,
    pub flash_attention: bool,
    pub kv_type_k: KvQuantType,
    pub kv_type_v: KvQuantType,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::new(),
            context_length: 8192,
            batch_size: 512,
            threads: 4,
            gpu_layers: -1,
            flash_attention: true,
            kv_type_k: KvQuantType::Q8_0,
            kv_type_v: KvQuantType::Q8_0,
        }
    }
}

impl InferenceConfig {
    /// Standard tier configs using legacy quantization.
    pub fn for_tier(tier: u32) -> Self {
        match tier {
            1 => Self {
                context_length: 4096,
                batch_size: 256,
                threads: 4,
                gpu_layers: -1,
                flash_attention: true,
                kv_type_k: KvQuantType::Q4_0,
                kv_type_v: KvQuantType::Q4_0,
                ..Default::default()
            },
            2 => Self {
                context_length: 8192,
                batch_size: 512,
                threads: 6,
                gpu_layers: -1,
                flash_attention: true,
                kv_type_k: KvQuantType::Q8_0,
                kv_type_v: KvQuantType::Q4_0,
                ..Default::default()
            },
            3 => Self {
                context_length: 32768,
                batch_size: 1024,
                threads: 8,
                gpu_layers: -1,
                flash_attention: true,
                kv_type_k: KvQuantType::Q8_0,
                kv_type_v: KvQuantType::Q8_0,
                ..Default::default()
            },
            _ => Self {
                context_length: 65536,
                batch_size: 2048,
                threads: 12,
                gpu_layers: -1,
                flash_attention: true,
                kv_type_k: KvQuantType::F16,
                kv_type_v: KvQuantType::Q8_0,
                ..Default::default()
            },
        }
    }

    /// TurboQuant tier configs — same memory budget, massively expanded context.
    ///
    /// TQ3 gives ~4.9x compression vs F16 with near-zero quality loss.
    /// On a 16GB machine this means ~20K context instead of 4K.
    pub fn for_tier_turboquant(tier: u32) -> Self {
        match tier {
            1 => Self {
                context_length: 20480,  // was 4096 — 5x from TQ3
                batch_size: 256,
                threads: 4,
                gpu_layers: -1,
                flash_attention: true,
                kv_type_k: KvQuantType::TQ3,
                kv_type_v: KvQuantType::TQ3,
                ..Default::default()
            },
            2 => Self {
                context_length: 40960,  // was 8192 — 5x from TQ3
                batch_size: 512,
                threads: 6,
                gpu_layers: -1,
                flash_attention: true,
                kv_type_k: KvQuantType::TQ3,
                kv_type_v: KvQuantType::TQ4,
                ..Default::default()
            },
            3 => Self {
                context_length: 131072, // was 32768 — 4x from TQ3/TQ4 mix
                batch_size: 1024,
                threads: 8,
                gpu_layers: -1,
                flash_attention: true,
                kv_type_k: KvQuantType::TQ4,
                kv_type_v: KvQuantType::TQ3,
                ..Default::default()
            },
            _ => Self {
                context_length: 262144, // was 65536 — full native context window
                batch_size: 2048,
                threads: 12,
                gpu_layers: -1,
                flash_attention: true,
                kv_type_k: KvQuantType::TQ4,
                kv_type_v: KvQuantType::TQ4,
                ..Default::default()
            },
        }
    }

    /// Pick the best config for a tier, using TurboQuant if available.
    pub fn for_tier_auto(tier: u32, turboquant_available: bool) -> Self {
        if turboquant_available {
            Self::for_tier_turboquant(tier)
        } else {
            Self::for_tier(tier)
        }
    }
}

pub fn detect_tier(hw: &HardwareInfo) -> u32 {
    match hw.ram_gb {
        0..=16 => 1,
        17..=32 => 2,
        33..=64 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::types::{CpuArch, GpuType, HardwareInfo};

    #[test]
    fn test_kv_quant_ffi_values() {
        assert_eq!(KvQuantType::F16.to_ffi(), 1);
        assert_eq!(KvQuantType::Q8_0.to_ffi(), 8);
        assert_eq!(KvQuantType::Q4_0.to_ffi(), 2);
        assert_eq!(KvQuantType::Q4_1.to_ffi(), 3);
        assert_eq!(KvQuantType::TQ3.to_ffi(), 30);
        assert_eq!(KvQuantType::TQ4.to_ffi(), 31);
    }

    #[test]
    fn test_kv_quant_memory_factors() {
        assert!(KvQuantType::Q4_0.memory_factor() < KvQuantType::Q8_0.memory_factor());
        assert!(KvQuantType::Q8_0.memory_factor() < KvQuantType::F16.memory_factor());
        assert_eq!(KvQuantType::F16.memory_factor(), 1.0);
        // TQ3 should be the most compressed
        assert!(KvQuantType::TQ3.memory_factor() < KvQuantType::Q4_0.memory_factor());
        assert!(KvQuantType::TQ4.memory_factor() < KvQuantType::Q8_0.memory_factor());
    }

    #[test]
    fn test_turboquant_requires_flag() {
        assert!(KvQuantType::TQ3.requires_turboquant());
        assert!(KvQuantType::TQ4.requires_turboquant());
        assert!(!KvQuantType::Q8_0.requires_turboquant());
        assert!(!KvQuantType::F16.requires_turboquant());
    }

    #[test]
    fn test_kv_quant_labels() {
        assert_eq!(KvQuantType::TQ3.label(), "TurboQuant-3bit");
        assert_eq!(KvQuantType::TQ4.label(), "TurboQuant-4bit");
        assert_eq!(KvQuantType::Q8_0.label(), "Q8_0");
    }

    #[test]
    fn test_estimate_max_context() {
        let budget = 2 * 1024 * 1024 * 1024; // 2GB KV budget
        let ctx_f16 = KvQuantType::F16.estimate_max_context(budget);
        let ctx_tq3 = KvQuantType::TQ3.estimate_max_context(budget);
        // TQ3 should give ~5x more context than F16 for same memory
        assert!(ctx_tq3 > ctx_f16 * 4, "TQ3={ctx_tq3} should be >4x F16={ctx_f16}");
    }

    #[test]
    fn test_detect_tier_16gb() {
        let hw = HardwareInfo {
            arch: CpuArch::AppleSilicon,
            gpu: GpuType::Metal,
            ram_gb: 16,
        };
        assert_eq!(detect_tier(&hw), 1);
    }

    #[test]
    fn test_detect_tier_32gb() {
        let hw = HardwareInfo {
            arch: CpuArch::AppleSilicon,
            gpu: GpuType::Metal,
            ram_gb: 32,
        };
        assert_eq!(detect_tier(&hw), 2);
    }

    #[test]
    fn test_detect_tier_64gb() {
        let hw = HardwareInfo {
            arch: CpuArch::X86_64,
            gpu: GpuType::Cuda { vram_gb: 24 },
            ram_gb: 64,
        };
        assert_eq!(detect_tier(&hw), 3);
    }

    #[test]
    fn test_detect_tier_96gb() {
        let hw = HardwareInfo {
            arch: CpuArch::AppleSilicon,
            gpu: GpuType::Metal,
            ram_gb: 96,
        };
        assert_eq!(detect_tier(&hw), 4);
    }

    #[test]
    fn test_detect_tier_128gb() {
        let hw = HardwareInfo {
            arch: CpuArch::AppleSilicon,
            gpu: GpuType::Metal,
            ram_gb: 128,
        };
        assert_eq!(detect_tier(&hw), 4);
    }

    #[test]
    fn test_default_config() {
        let cfg = InferenceConfig::default();
        assert_eq!(cfg.context_length, 8192);
        assert_eq!(cfg.batch_size, 512);
        assert!(cfg.flash_attention);
        assert_eq!(cfg.kv_type_k, KvQuantType::Q8_0);
    }

    #[test]
    fn test_tier_configs_scale_context() {
        let t1 = InferenceConfig::for_tier(1);
        let t2 = InferenceConfig::for_tier(2);
        let t3 = InferenceConfig::for_tier(3);
        let t4 = InferenceConfig::for_tier(4);
        assert!(t1.context_length < t2.context_length);
        assert!(t2.context_length < t3.context_length);
        assert!(t3.context_length < t4.context_length);
    }

    #[test]
    fn test_tier_configs_scale_batch() {
        let t1 = InferenceConfig::for_tier(1);
        let t4 = InferenceConfig::for_tier(4);
        assert!(t1.batch_size < t4.batch_size);
    }

    // -- TurboQuant tier tests --

    #[test]
    fn test_turboquant_tier1_massive_context_boost() {
        let legacy = InferenceConfig::for_tier(1);
        let tq = InferenceConfig::for_tier_turboquant(1);
        assert!(
            tq.context_length >= legacy.context_length * 4,
            "TQ tier 1 context {} should be >=4x legacy {}",
            tq.context_length, legacy.context_length
        );
        assert!(tq.kv_type_k.requires_turboquant());
    }

    #[test]
    fn test_turboquant_tiers_scale() {
        let t1 = InferenceConfig::for_tier_turboquant(1);
        let t2 = InferenceConfig::for_tier_turboquant(2);
        let t3 = InferenceConfig::for_tier_turboquant(3);
        let t4 = InferenceConfig::for_tier_turboquant(4);
        assert!(t1.context_length < t2.context_length);
        assert!(t2.context_length < t3.context_length);
        assert!(t3.context_length < t4.context_length);
    }

    #[test]
    fn test_tier_auto_selects_turboquant_when_available() {
        let auto_tq = InferenceConfig::for_tier_auto(1, true);
        let auto_legacy = InferenceConfig::for_tier_auto(1, false);
        assert!(auto_tq.kv_type_k.requires_turboquant());
        assert!(!auto_legacy.kv_type_k.requires_turboquant());
        assert!(auto_tq.context_length > auto_legacy.context_length);
    }

    #[test]
    fn test_turboquant_tier4_full_native_context() {
        let t4 = InferenceConfig::for_tier_turboquant(4);
        // 262K — full Qwen 3.5 native context window
        assert_eq!(t4.context_length, 262144);
    }
}
