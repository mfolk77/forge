use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use tokio::time::{sleep, Duration};

use super::http_client::HttpModelClient;

/// Manages a llama-server process for llama.cpp inference
pub struct LlamaCppServer {
    process: Option<Child>,
    port: u16,
    client: HttpModelClient,
    model_path: Option<String>,
}

impl LlamaCppServer {
    pub fn new(port: u16) -> Self {
        let client = HttpModelClient::new(&format!("http://127.0.0.1:{port}"));
        Self {
            process: None,
            port,
            client,
            model_path: None,
        }
    }

    /// Find the llama-server binary
    fn find_server_binary() -> Result<PathBuf> {
        // Check common locations
        let candidates = [
            "llama-server",
            "llama.cpp/build/bin/llama-server",
            "/usr/local/bin/llama-server",
            "/opt/homebrew/bin/llama-server",
        ];

        for candidate in &candidates {
            if let Ok(output) = Command::new("which").arg(candidate).output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    return Ok(PathBuf::from(path));
                }
            }
            let path = PathBuf::from(candidate);
            if path.exists() {
                return Ok(path);
            }
        }

        anyhow::bail!(
            "llama-server not found. Install llama.cpp: brew install llama.cpp\n\
             Or build from source: https://github.com/ggerganov/llama.cpp"
        )
    }

    /// Start the llama-server process with the given model
    pub async fn start(
        &mut self,
        model_path: &str,
        gpu_layers: i32,
        threads: usize,
        context_length: usize,
    ) -> Result<()> {
        // Stop any existing server
        self.stop();

        let server_bin = Self::find_server_binary()?;

        let mut cmd = Command::new(server_bin);
        cmd.arg("-m")
            .arg(model_path)
            .arg("--port")
            .arg(self.port.to_string())
            .arg("-ngl")
            .arg(gpu_layers.to_string())
            .arg("-t")
            .arg(threads.to_string())
            .arg("-c")
            .arg(context_length.to_string())
            .arg("--host")
            .arg("127.0.0.1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = cmd.spawn().context("Failed to start llama-server")?;
        self.process = Some(child);
        self.model_path = Some(model_path.to_string());

        // Wait for server to be ready
        for i in 0..30 {
            if self.client.health_check().await {
                return Ok(());
            }
            if i > 0 && i % 10 == 0 {
                // Check if process is still alive
                if let Some(ref mut proc) = self.process {
                    match proc.try_wait() {
                        Ok(Some(status)) => {
                            self.process = None;
                            anyhow::bail!("llama-server exited with status: {status}");
                        }
                        Ok(None) => {} // still running
                        Err(e) => anyhow::bail!("Failed to check llama-server status: {e}"),
                    }
                }
            }
            sleep(Duration::from_millis(500)).await;
        }

        self.stop();
        anyhow::bail!("llama-server failed to start within 15 seconds")
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

impl Drop for LlamaCppServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_server() {
        let server = LlamaCppServer::new(8081);
        assert_eq!(server.port, 8081);
        assert!(!server.is_running());
        assert!(server.model_path().is_none());
    }
}
