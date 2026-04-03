use anyhow::{Context, Result};
use std::process::{Child, Command, Stdio};
use tokio::time::{sleep, Duration};

use super::http_client::HttpModelClient;

/// Returns true if MLX is available on this platform (Apple Silicon only)
pub fn is_available() -> bool {
    cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")
}

/// Manages an MLX LM server process for Apple Silicon inference
pub struct MlxServer {
    process: Option<Child>,
    port: u16,
    client: HttpModelClient,
    model_path: Option<String>,
}

impl MlxServer {
    pub fn new(port: u16) -> Self {
        let client = HttpModelClient::new(&format!("http://127.0.0.1:{port}"));
        Self {
            process: None,
            port,
            client,
            model_path: None,
        }
    }

    /// Find the mlx_lm.server executable.
    /// Tries: standalone binary (Homebrew) first, then python3 module fallback.
    /// MLX is macOS-only; on other platforms this returns an error.
    fn find_server() -> Result<(String, Vec<String>)> {
        if !is_available() {
            anyhow::bail!("MLX requires Apple Silicon Mac");
        }

        // 1. Standalone binary (Homebrew: brew install mlx-lm)
        let brew_path = "/opt/homebrew/bin/mlx_lm.server";
        if std::path::Path::new(brew_path).exists() {
            return Ok((brew_path.to_string(), vec![]));
        }

        // 2. In PATH
        if let Ok(output) = Command::new("which").arg("mlx_lm.server").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok((path, vec![]));
                }
            }
        }

        // 3. Python module fallback
        let python_cmd = if cfg!(windows) { "python" } else { "python3" };
        let output = Command::new(python_cmd)
            .args(["-c", "import mlx_lm"])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                return Ok(("python3".to_string(), vec!["-m".to_string(), "mlx_lm.server".to_string()]));
            }
        }

        anyhow::bail!(
            "mlx_lm not found. Install it:\n  brew install mlx-lm  (recommended)\n  pip install mlx-lm   (alternative)\nRequires Apple Silicon Mac."
        )
    }

    /// Start the MLX LM server with the given model.
    /// Passes `--max-kv-size` to cap KV cache memory usage, preventing
    /// Metal OOM crashes on memory-constrained machines (e.g. 16GB).
    pub async fn start(&mut self, model_path: &str, context_length: usize) -> Result<()> {
        self.stop();

        let (exe, prefix_args) = Self::find_server()?;

        // Cap KV cache steps to limit memory. On 16GB machines with 14B models,
        // 32K context can exhaust unified memory. Use the configured context_length
        // but cap at a safe maximum based on available RAM.
        let hw = super::types::HardwareInfo::detect();
        let safe_kv_steps = if hw.ram_gb <= 16 {
            context_length.min(8192)
        } else if hw.ram_gb <= 32 {
            context_length.min(16384)
        } else {
            context_length
        };

        let mut cmd = Command::new(&exe);
        cmd.args(&prefix_args);
        cmd.args([
            "--model",
            model_path,
            "--port",
            &self.port.to_string(),
            "--host",
            "127.0.0.1",
        ]);

        // Limit prompt cache to control memory usage
        let cache_str = safe_kv_steps.to_string();
        cmd.args(["--prompt-cache-size", &cache_str]);

        // Capture stderr to a log file for debugging MLX crashes
        let log_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".ftai")
            .join("mlx-server.log");
        cmd.stdout(Stdio::null());
        match std::fs::File::create(&log_path) {
            Ok(file) => { cmd.stderr(Stdio::from(file)); }
            Err(_) => { cmd.stderr(Stdio::null()); }
        }

        let child = cmd.spawn().context("Failed to start mlx_lm.server")?;
        self.process = Some(child);
        self.model_path = Some(model_path.to_string());

        // Wait for server to be ready
        for i in 0..60 {
            if self.client.health_check().await {
                return Ok(());
            }
            if i > 0 && i % 10 == 0 {
                if let Some(ref mut proc) = self.process {
                    match proc.try_wait() {
                        Ok(Some(status)) => {
                            self.process = None;
                            anyhow::bail!("mlx_lm.server exited with status: {status}");
                        }
                        Ok(None) => {}
                        Err(e) => anyhow::bail!("Failed to check mlx_lm.server status: {e}"),
                    }
                }
            }
            sleep(Duration::from_millis(500)).await;
        }

        self.stop();
        anyhow::bail!("mlx_lm.server failed to start within 30 seconds")
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn client(&self) -> &HttpModelClient {
        &self.client
    }

    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }

    pub fn model_path(&self) -> Option<&str> {
        self.model_path.as_deref()
    }
}

impl Drop for MlxServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_server() {
        let server = MlxServer::new(8082);
        assert_eq!(server.port, 8082);
        assert!(!server.is_running());
    }
}
