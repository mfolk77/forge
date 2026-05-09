use anyhow::{Context, Result};
use std::process::{Child, Command, Stdio};
use tokio::time::{sleep, Duration};

use super::http_client::HttpModelClient;

/// Disable MLX's native tool parser by setting `tool_parser_type: null` in the
/// model's `tokenizer_config.json`. The Qwen3-Coder template that ships with
/// Qwen3.5 models binds MLX to a parser that raises `ValueError` on any
/// malformed `<tool_call>` block in the model's output, killing long sessions
/// once the small model emits even one slightly off block. Forge's
/// `http_client::extract_inline_tool_calls` lifts tool calls from raw content
/// itself, so MLX's native parser is unwanted.
///
/// Idempotent: no-op if already null. Silent no-op if `model_path` points at
/// a GGUF file (single-file path) or a directory without `tokenizer_config.json`.
///
/// SECURITY (CAT 2 — Path & File Security):
/// - ONLY modifies an existing `tokenizer_config.json`; never creates one.
///   This prevents a hostile `model_path` from causing arbitrary file writes.
/// - Atomic write (temp file + rename) so a crash mid-write can't corrupt
///   the model config — partial-write would leave the model unloadable.
fn ensure_tool_parser_disabled(model_path: &str) -> Result<()> {
    let model_dir = std::path::Path::new(model_path);
    if !model_dir.is_dir() {
        return Ok(());
    }
    let config_path = model_dir.join("tokenizer_config.json");
    if !config_path.is_file() {
        return Ok(());
    }

    let bytes = std::fs::read(&config_path)
        .with_context(|| format!("read tokenizer_config.json at {config_path:?}"))?;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse tokenizer_config.json at {config_path:?}"))?;
    let obj = value
        .as_object_mut()
        .context("tokenizer_config.json root is not a JSON object")?;

    if matches!(obj.get("tool_parser_type"), Some(serde_json::Value::Null)) {
        return Ok(());
    }

    obj.insert("tool_parser_type".to_string(), serde_json::Value::Null);

    let serialized = serde_json::to_vec_pretty(&value)
        .context("re-serialize tokenizer_config.json")?;

    let tmp_path = config_path.with_extension("json.forge.tmp");
    std::fs::write(&tmp_path, &serialized)
        .with_context(|| format!("write {tmp_path:?}"))?;
    std::fs::rename(&tmp_path, &config_path)
        .with_context(|| format!("rename {tmp_path:?} -> {config_path:?}"))?;

    Ok(())
}

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
    #[allow(dead_code)]
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
    /// Caps MLX prompt cache memory so stale request KV caches do not
    /// accumulate until unified memory pressure destabilizes the server.
    pub async fn start(&mut self, model_path: &str, context_length: usize) -> Result<()> {
        self.stop();

        // Disable MLX's native tool parser before spawning. Best-effort: a
        // permission error here is non-fatal, but the user will see degraded
        // long-session stability and a clear stderr warning explaining why.
        if let Err(e) = ensure_tool_parser_disabled(model_path) {
            eprintln!(
                "[forge] warning: could not disable MLX tool_parser_type for {model_path}: {e}\n[forge] long sessions may hit qwen3_coder ValueError on malformed <tool_call> blocks. See docs/troubleshooting.md."
            );
        }

        let (exe, prefix_args) = Self::find_server()?;

        // MLX's --prompt-cache-size is the number of distinct cached prompts,
        // not a token limit.
        let hw = super::types::HardwareInfo::detect();
        let _safe_kv_steps = if hw.ram_gb <= 16 {
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

        // Limit prompt cache to control memory usage. Passing the token budget
        // here would let MLX retain thousands of large KV caches.
        let cache_count = if hw.ram_gb <= 16 { 1 } else { 2 };
        let cache_bytes: u64 = if hw.ram_gb <= 16 {
            768 * 1024 * 1024
        } else if hw.ram_gb <= 32 {
            1536 * 1024 * 1024
        } else {
            3072 * 1024 * 1024
        };
        cmd.args(["--prompt-cache-size", &cache_count.to_string()]);
        cmd.args(["--prompt-cache-bytes", &cache_bytes.to_string()]);

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

        // Wait for server to be ready (90s — large models on 16GB can take 30-60s)
        for i in 0..180 {
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
        anyhow::bail!("mlx_lm.server failed to start within 90 seconds")
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

    // ── ensure_tool_parser_disabled tests (companion to A1) ──────────────────
    //
    // These tests cover the MLX startup auto-patch that codifies what was
    // previously a manual hand-edit of the model's tokenizer_config.json.
    // Production-readiness requirement: a fresh install must Just Work without
    // the user editing model JSON files.

    fn write_config(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("tokenizer_config.json");
        std::fs::write(&path, content).unwrap();
        path
    }

    fn read_config(path: &std::path::Path) -> serde_json::Value {
        let bytes = std::fs::read(path).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn test_ensure_disables_when_field_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(dir.path(), r#"{"model_max_length": 32768}"#);

        ensure_tool_parser_disabled(dir.path().to_str().unwrap()).unwrap();

        let v = read_config(&path);
        assert!(v.as_object().unwrap().contains_key("tool_parser_type"));
        assert_eq!(v["tool_parser_type"], serde_json::Value::Null);
        // Other fields preserved
        assert_eq!(v["model_max_length"], 32768);
    }

    #[test]
    fn test_ensure_overrides_inferred_string() {
        // The exact failure mode: MLX inferred `qwen3_coder` from chat template.
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(
            dir.path(),
            r#"{"tool_parser_type": "qwen3_coder", "model_max_length": 32768}"#,
        );

        ensure_tool_parser_disabled(dir.path().to_str().unwrap()).unwrap();

        let v = read_config(&path);
        assert_eq!(v["tool_parser_type"], serde_json::Value::Null);
    }

    #[test]
    fn test_ensure_idempotent_when_already_null() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(
            dir.path(),
            r#"{"tool_parser_type": null, "model_max_length": 32768}"#,
        );
        let mtime_before = std::fs::metadata(&path).unwrap().modified().unwrap();

        // Sleep briefly so any rewrite would change mtime
        std::thread::sleep(std::time::Duration::from_millis(50));
        ensure_tool_parser_disabled(dir.path().to_str().unwrap()).unwrap();

        let mtime_after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "idempotent path must not rewrite the file when already null"
        );

        let v = read_config(&path);
        assert_eq!(v["tool_parser_type"], serde_json::Value::Null);
    }

    #[test]
    fn test_ensure_skips_when_directory_missing_config() {
        let dir = tempfile::tempdir().unwrap();
        // No tokenizer_config.json in this dir — common for GGUF-only models.
        ensure_tool_parser_disabled(dir.path().to_str().unwrap()).unwrap();
    }

    #[test]
    fn test_ensure_skips_when_model_path_is_file() {
        // GGUF models pass a single .gguf file path, not a directory.
        let dir = tempfile::tempdir().unwrap();
        let gguf = dir.path().join("model.gguf");
        std::fs::write(&gguf, b"GGUF mock").unwrap();

        ensure_tool_parser_disabled(gguf.to_str().unwrap()).unwrap();
    }

    /// SECURITY (CAT 2 — Path & File Security):
    /// The function must NEVER create a tokenizer_config.json that didn't
    /// already exist. If a hostile config supplies model_path pointing at
    /// a directory the user didn't intend, this prevents Forge from writing
    /// arbitrary content there.
    #[test]
    fn test_ensure_security_no_create_at_arbitrary_path() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("tokenizer_config.json");
        assert!(!config_path.exists());

        ensure_tool_parser_disabled(dir.path().to_str().unwrap()).unwrap();

        assert!(
            !config_path.exists(),
            "must NOT create tokenizer_config.json at arbitrary paths (CAT 2 — Path & File Security)"
        );
    }

    /// SECURITY (CAT 2 — Path & File Security):
    /// Atomic-write semantics: the temp file should not survive a successful
    /// run. If it did, an attacker observing the file system mid-write could
    /// read the partially-written config (or future code might consume the
    /// temp path).
    #[test]
    fn test_ensure_atomic_no_temp_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), r#"{"model_max_length": 32768}"#);

        ensure_tool_parser_disabled(dir.path().to_str().unwrap()).unwrap();

        let temp_path = dir.path().join("tokenizer_config.json.forge.tmp");
        assert!(!temp_path.exists(), "temp file must be renamed away on success");
    }

    #[test]
    fn test_ensure_rejects_non_object_root() {
        // Defensive: tokenizer_config.json with non-object root (very unusual,
        // but we shouldn't blow up — we should error cleanly).
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), r#"["not", "an", "object"]"#);

        let err = ensure_tool_parser_disabled(dir.path().to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("not a JSON object"));
    }
}
