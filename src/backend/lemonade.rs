//! Lemonade Server (`lemond.exe`) sidecar — runs the critic model on the AMD NPU
//! via the Ryzen AI runtime. lemonade exposes an OpenAI-compatible API under the
//! `/api/v1` prefix, so the critic reuses forge's existing `HttpModelClient`
//! (pointed at `http://127.0.0.1:<port>/api`).
//!
//! Lifecycle mirrors `LlamaCppServer`: spawn the process, poll health, load the
//! model onto the NPU, then serve generations over HTTP. Killed on `stop()`/drop.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use tokio::time::{sleep, Duration};

use super::http_client::HttpModelClient;

pub struct LemonadeServer {
    process: Option<Child>,
    port: u16,
    model_name: String,
    /// OpenAI-compatible client. base_url includes the `/api` prefix so the
    /// client's `/v1/chat/completions` resolves to lemonade's `/api/v1/chat/completions`.
    client: HttpModelClient,
}

impl LemonadeServer {
    pub fn new(port: u16, model_name: String) -> Self {
        let client = HttpModelClient::new(&format!("http://127.0.0.1:{port}/api"));
        Self {
            process: None,
            port,
            model_name,
            client,
        }
    }

    pub fn client(&self) -> &HttpModelClient {
        &self.client
    }

    /// Locate `lemond.exe`. Checks forge's bundled location first, then PATH.
    fn find_lemond() -> Result<PathBuf> {
        // 1. Forge's stable install location (~/.ftai/lemonade/lemond.exe)
        if let Ok(dir) = crate::config::global_config_dir() {
            let p = dir.join("lemonade").join("lemond.exe");
            if p.is_file() {
                return Ok(p);
            }
        }
        // 2. PATH via `where` (Windows) / `which` (Unix)
        let which = if cfg!(windows) { "where" } else { "which" };
        let name = if cfg!(windows) { "lemond.exe" } else { "lemond" };
        if let Ok(out) = Command::new(which).arg(name).output() {
            if out.status.success() {
                if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
                    let p = PathBuf::from(line.trim());
                    if p.is_file() {
                        return Ok(p);
                    }
                }
            }
        }
        bail!("lemond.exe not found (install the lemonade server under ~/.ftai/lemonade/)")
    }

    /// Spawn the lemond.exe process. Does not wait for readiness or load a model.
    pub fn spawn_only(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(());
        }
        let exe = Self::find_lemond()?;
        let log_path = crate::config::global_config_dir()
            .map(|d| d.join("lemonade-server.log"))
            .unwrap_or_else(|_| PathBuf::from("lemonade-server.log"));

        let mut cmd = Command::new(&exe);
        cmd.args(["--port", &self.port.to_string()]);
        if let Ok(file) = std::fs::File::create(&log_path) {
            if let Ok(err_file) = file.try_clone() {
                cmd.stdout(Stdio::from(file)).stderr(Stdio::from(err_file));
            }
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn lemond.exe at {}", exe.display()))?;
        self.process = Some(child);
        Ok(())
    }

    /// Wait for the server to become healthy, then load the model onto the NPU.
    pub async fn wait_until_ready(&mut self) -> Result<()> {
        // Poll health for up to ~90s (NPU runtime init can be slow on first start).
        for _ in 0..180 {
            if self.client.health_check().await {
                return self.load_model().await;
            }
            if let Some(child) = self.process.as_mut() {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    bail!("lemonade server exited during startup (see ~/.ftai/lemonade-server.log)");
                }
            }
            sleep(Duration::from_millis(500)).await;
        }
        bail!("lemonade server did not become ready within 90s")
    }

    /// Load the configured model onto the NPU via POST /api/v1/load.
    async fn load_model(&self) -> Result<()> {
        let url = format!("http://127.0.0.1:{}/api/v1/load", self.port);
        let resp = reqwest::Client::new()
            .post(&url)
            .json(&serde_json::json!({ "model_name": self.model_name }))
            .send()
            .await
            .context("failed to call lemonade /api/v1/load")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("lemonade model load failed ({status}): {body}");
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.process.take() {
            // lemond.exe spawns `ryzenai-server` worker children that hold the model
            // (and ~GBs of RAM). A plain kill() orphans them, so on Windows kill the
            // whole process tree via taskkill /T before reaping the handle.
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/T", "/F", "/PID", &child.id().to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for LemonadeServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lemonade_server_construction() {
        // The /api prefix is load-bearing: HttpModelClient appends /v1/chat/completions,
        // which must resolve to lemonade's /api/v1/chat/completions.
        let server = LemonadeServer::new(13305, "Qwen2.5-7B-Instruct-NPU".to_string());
        assert_eq!(server.port, 13305);
        assert_eq!(server.model_name, "Qwen2.5-7B-Instruct-NPU");
        assert!(server.process.is_none());
    }
}
