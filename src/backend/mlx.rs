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
    fn find_server() -> Result<(String, Vec<String>)> {
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
        let output = Command::new("python3")
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

    /// Start the MLX LM server with the given model
    pub async fn start(&mut self, model_path: &str, context_length: usize) -> Result<()> {
        self.stop();

        let (exe, prefix_args) = Self::find_server()?;

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

        cmd.stdout(Stdio::null()).stderr(Stdio::null());

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
