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

    /// Find the llama-server binary.
    /// Checks: 1) Forge's own install dir, 2) PATH via where/which, 3) common locations.
    fn find_server_binary() -> Result<PathBuf> {
        let server_name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };

        // 1) Check Forge's install directory first (where `forge setup` puts it)
        if let Ok(config_dir) = crate::config::global_config_dir() {
            let install_dir = config_dir.parent()
                .map(|p| p.join("bin"))
                .unwrap_or_else(|| config_dir.join("bin"));
            let ours = install_dir.join(server_name);
            if ours.exists() {
                return Ok(ours);
            }
        }
        #[cfg(windows)]
        {
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                let ours = PathBuf::from(&local).join("forge").join("bin").join(server_name);
                if ours.exists() {
                    return Ok(ours);
                }
                // Also check llamacpp subdirectory (manual install location)
                let llamacpp = PathBuf::from(&local).join("forge").join("llamacpp").join(server_name);
                if llamacpp.exists() {
                    return Ok(llamacpp);
                }
            }
        }

        // 2) Check PATH via where/which
        let which_cmd = if cfg!(windows) { "where" } else { "which" };
        if let Ok(output) = Command::new(which_cmd).arg(server_name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }

        // 3) Check common locations
        let extra_candidates: Vec<&str> = if cfg!(windows) {
            vec![]
        } else {
            vec![
                "llama.cpp/build/bin/llama-server",
                "/usr/local/bin/llama-server",
                "/opt/homebrew/bin/llama-server",
            ]
        };
        for candidate in &extra_candidates {
            let path = PathBuf::from(candidate);
            if path.exists() {
                return Ok(path);
            }
        }

        let install_hint = if cfg!(windows) {
            "llama-server not found. Run `forge setup` to install automatically,\n\
             or download from https://github.com/ggerganov/llama.cpp/releases"
        } else {
            "llama-server not found. Run `forge setup` to install automatically,\n\
             or: brew install llama.cpp"
        };

        anyhow::bail!(install_hint)
    }

    /// Spawn the llama-server process WITHOUT waiting for it to become ready.
    /// Returns as soon as the process is started. Call `wait_until_ready()` later.
    pub fn spawn_only(
        &mut self,
        model_path: &str,
        gpu_layers: i32,
        threads: usize,
        context_length: usize,
    ) -> Result<()> {
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
            .arg("--jinja")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = cmd.spawn().context("Failed to start llama-server")?;
        self.process = Some(child);
        self.model_path = Some(model_path.to_string());
        Ok(())
    }

    /// Wait for an already-spawned server to become ready (up to 90s).
    pub async fn wait_until_ready(&mut self) -> Result<()> {
        for i in 0..180 {
            if self.client.health_check().await {
                return Ok(());
            }
            if i > 0 && i % 10 == 0 {
                if let Some(ref mut proc) = self.process {
                    match proc.try_wait() {
                        Ok(Some(status)) => {
                            self.process = None;
                            anyhow::bail!("llama-server exited with status: {status}");
                        }
                        Ok(None) => {}
                        Err(e) => anyhow::bail!("Failed to check llama-server status: {e}"),
                    }
                }
            }
            sleep(Duration::from_millis(500)).await;
        }
        self.stop();
        anyhow::bail!("llama-server failed to start within 90 seconds")
    }

    /// Start the llama-server process with the given model (spawn + wait).
    pub async fn start(
        &mut self,
        model_path: &str,
        gpu_layers: i32,
        threads: usize,
        context_length: usize,
    ) -> Result<()> {
        self.spawn_only(model_path, gpu_layers, threads, context_length)?;
        self.wait_until_ready().await
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

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }

    #[allow(dead_code)]
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
