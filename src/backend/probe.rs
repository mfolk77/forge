use std::fmt;

/// Results of probing available inference backends.
/// Probe once at startup, cache for the session lifetime.
pub struct BackendProbeResults {
    pub mlx_available: bool,
    pub mlx_reason: Option<String>,
    pub llamacpp_available: bool,
    pub llamacpp_reason: Option<String>,
    pub recommended: String,
}

impl BackendProbeResults {
    /// Probe all backends and return cached results.
    pub fn probe() -> Self {
        let (mlx_available, mlx_reason) = probe_mlx();
        let (llamacpp_available, llamacpp_reason) = probe_llamacpp();

        let recommended = if mlx_available {
            "mlx".to_string()
        } else if llamacpp_available {
            "llamacpp".to_string()
        } else {
            "none".to_string()
        };

        Self {
            mlx_available,
            mlx_reason,
            llamacpp_available,
            llamacpp_reason,
            recommended,
        }
    }

    /// Format results for `forge doctor` output.
    pub fn display(&self) -> String {
        let mut lines = Vec::new();
        lines.push("Backend Probe Results".to_string());
        lines.push("---------------------".to_string());

        lines.push(format!(
            "MLX:       {} {}",
            if self.mlx_available { "available" } else { "unavailable" },
            self.mlx_reason.as_deref().map(|r| format!("({})", r)).unwrap_or_default()
        ));

        lines.push(format!(
            "llama.cpp: {} {}",
            if self.llamacpp_available { "available" } else { "unavailable" },
            self.llamacpp_reason.as_deref().map(|r| format!("({})", r)).unwrap_or_default()
        ));

        lines.push(format!("Recommended: {}", self.recommended));

        lines.join("\n")
    }
}

impl fmt::Display for BackendProbeResults {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// Check if MLX is available (Apple Silicon + mlx_lm installed).
fn probe_mlx() -> (bool, Option<String>) {
    if !super::mlx::is_available() {
        return (false, Some("Requires Apple Silicon Mac".to_string()));
    }

    // Check if mlx_lm.server binary exists
    let brew_path = std::path::Path::new("/opt/homebrew/bin/mlx_lm.server");
    if brew_path.exists() {
        return (true, None);
    }

    // Check PATH
    if let Ok(output) = std::process::Command::new("which")
        .arg("mlx_lm.server")
        .output()
    {
        if output.status.success() {
            return (true, None);
        }
    }

    // Check python module
    if let Ok(output) = std::process::Command::new("python3")
        .args(["-c", "import mlx_lm"])
        .output()
    {
        if output.status.success() {
            return (true, Some("via python3 -m mlx_lm.server".to_string()));
        }
    }

    (false, Some("mlx_lm not installed".to_string()))
}

/// Check if llama.cpp server binary is available.
fn probe_llamacpp() -> (bool, Option<String>) {
    let candidates: &[&str] = if cfg!(windows) {
        &["llama-server.exe", "llama-server"]
    } else {
        &[
            "llama-server",
            "/usr/local/bin/llama-server",
            "/opt/homebrew/bin/llama-server",
        ]
    };

    let which_cmd = if cfg!(windows) { "where" } else { "which" };

    for candidate in candidates {
        if let Ok(output) = std::process::Command::new(which_cmd)
            .arg(candidate)
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path.is_empty() {
                    return (true, Some(format!("at {path}")));
                }
            }
        }
        let path = std::path::Path::new(candidate);
        if path.exists() {
            return (true, Some(format!("at {}", path.display())));
        }
    }

    (false, Some("llama-server not found in PATH".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_returns_valid_results() {
        let results = BackendProbeResults::probe();
        // recommended must be one of these three values
        assert!(
            matches!(results.recommended.as_str(), "mlx" | "llamacpp" | "none"),
            "unexpected recommended: {}",
            results.recommended
        );
    }

    #[test]
    fn test_probe_display_contains_sections() {
        let results = BackendProbeResults::probe();
        let output = results.display();
        assert!(output.contains("MLX:"));
        assert!(output.contains("llama.cpp:"));
        assert!(output.contains("Recommended:"));
    }

    #[test]
    fn test_probe_display_format_trait() {
        let results = BackendProbeResults::probe();
        let via_display = format!("{results}");
        let via_method = results.display();
        assert_eq!(via_display, via_method);
    }

    #[test]
    fn test_probe_consistency() {
        let results = BackendProbeResults::probe();
        // If MLX is available, recommended should be mlx
        if results.mlx_available {
            assert_eq!(results.recommended, "mlx");
        }
        // If only llamacpp is available, recommended should be llamacpp
        if !results.mlx_available && results.llamacpp_available {
            assert_eq!(results.recommended, "llamacpp");
        }
        // If neither, recommended should be none
        if !results.mlx_available && !results.llamacpp_available {
            assert_eq!(results.recommended, "none");
        }
    }
}
