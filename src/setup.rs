//! One-command setup: installs llama-server backend and downloads the right model.
//!
//! `forge setup` detects hardware, installs llama.cpp if needed, downloads the
//! model, and configures everything. One and done.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::backend::types::{GpuType, HardwareInfo};
use crate::config;

const LLAMACPP_REPO: &str = "ggerganov/llama.cpp";

// ── Hardcoded model URLs — no HuggingFace CLI, no auth, just curl ──────────
// These are public Apache-2.0 models hosted on HuggingFace.

/// Small model for CPU-only / low RAM. Dense 4B, ~3GB download.
const MODEL_SMALL_URL: &str = "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf";
const MODEL_SMALL_FILE: &str = "Qwen3.5-4B-Q4_K_M.gguf";
const MODEL_SMALL_NAME: &str = "Qwen3.5-4B";
const MODEL_SMALL_SIZE: &str = "~3 GB";

/// Large model for GPU or high-RAM systems. 35B MoE (3B active), ~20GB download.
const MODEL_LARGE_URL: &str = "https://huggingface.co/unsloth/Qwen3.5-35B-A3B-GGUF/resolve/main/Qwen3.5-35B-A3B-Q4_K_M.gguf";
const MODEL_LARGE_FILE: &str = "Qwen3.5-35B-A3B-Q4_K_M.gguf";
const MODEL_LARGE_NAME: &str = "Qwen3.5-35B-A3B";
const MODEL_LARGE_SIZE: &str = "~20 GB";

/// Pick the right model based on hardware.
fn pick_model(hw: &HardwareInfo) -> (&'static str, &'static str, &'static str, &'static str) {
    let has_gpu = !matches!(hw.gpu, GpuType::None);

    // GPU with 8GB+ VRAM or native system with 48GB+ RAM → large model
    // CPU-only or low RAM → small model (the 35B needs ~22GB, won't fit in WSL default memory)
    if has_gpu {
        (MODEL_LARGE_URL, MODEL_LARGE_FILE, MODEL_LARGE_NAME, MODEL_LARGE_SIZE)
    } else if hw.ram_gb >= 48 {
        (MODEL_LARGE_URL, MODEL_LARGE_FILE, MODEL_LARGE_NAME, MODEL_LARGE_SIZE)
    } else {
        (MODEL_SMALL_URL, MODEL_SMALL_FILE, MODEL_SMALL_NAME, MODEL_SMALL_SIZE)
    }
}

/// Run the full setup process.
pub async fn run_setup() -> Result<()> {
    println!();
    println!("  Forge Setup");
    println!("  ===========");
    println!();

    // Step 1: Detect hardware
    println!("[1/4] Detecting hardware...");
    let hw = HardwareInfo::detect();
    let has_gpu = !matches!(hw.gpu, GpuType::None);
    println!("  RAM:  {} GB", hw.ram_gb);
    println!("  GPU:  {}", if has_gpu { "yes" } else { "none (CPU-only)" });

    let (model_url, model_file, model_name, model_size) = pick_model(&hw);
    println!("  Model: {} ({})", model_name, model_size);
    println!();

    // Step 2: Ensure llama-server is available
    println!("[2/4] Checking for llama-server...");
    if find_llama_server().is_some() {
        println!("  Found. Skipping install.");
    } else {
        println!("  Not found. Installing...");
        install_llama_server()?;
    }
    println!();

    // Step 3: Download model
    let models_dir = config::global_config_dir()?.join("models");
    std::fs::create_dir_all(&models_dir)?;
    let gguf_path = models_dir.join(model_file);

    println!("[3/4] Downloading model...");
    if gguf_path.exists() && gguf_path.metadata().map(|m| m.len() > 100_000_000).unwrap_or(false) {
        println!("  Already downloaded: {}", gguf_path.display());
    } else {
        // Delete any partial/empty file
        let _ = std::fs::remove_file(&gguf_path);
        println!("  Downloading {} ({})", model_file, model_size);
        println!("  This will take a while...");
        download_file(model_url, &gguf_path)?;

        // Verify the file actually downloaded
        let size = gguf_path.metadata().map(|m| m.len()).unwrap_or(0);
        if size < 100_000_000 {
            let _ = std::fs::remove_file(&gguf_path);
            bail!("Download failed — file too small ({} bytes). Check your internet connection.", size);
        }
        println!("  Done. ({} MB)", size / (1024 * 1024));
    }
    println!();

    // Step 4: Write config
    println!("[4/4] Writing config...");
    let gguf_path_str = gguf_path.to_string_lossy().to_string();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4);
    let gpu_layers: i32 = if has_gpu { -1 } else { 0 };

    let config_path = config::global_config_dir()?.join("config.toml");
    let config_content = format!(
        r#"[model]
backend = "llamacpp"
path = "{gguf_path_str}"
context_length = 32768
temperature = 0.3
tool_calling = "hybrid"

[model.llamacpp]
gpu_layers = {gpu_layers}
threads = {threads}

[model.mlx]
quantization = "q4"

[permissions]
mode = "auto"

[plugins]
enabled = true
auto_update = false
"#
    );
    std::fs::write(&config_path, &config_content)?;
    println!("  Model: {}", gguf_path_str);
    println!("  GPU layers: {} ({})", gpu_layers, if gpu_layers == 0 { "CPU-only" } else { "GPU" });
    println!("  Threads: {}", threads);
    println!();

    println!("  Setup complete! Run `forge` to start.");
    println!();

    Ok(())
}

// ── llama-server ───────────────────────────────────────────────────────────

fn find_llama_server() -> Option<PathBuf> {
    let install_dir = install_bin_dir();

    // Check our install dir first
    let ours = if cfg!(windows) {
        install_dir.join("llama-server.exe")
    } else {
        install_dir.join("llama-server")
    };
    if ours.exists() {
        return Some(ours);
    }

    // Check PATH
    let which_cmd = if cfg!(windows) { "where" } else { "which" };
    let name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    if let Ok(output) = Command::new(which_cmd).arg(name).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    None
}

fn install_llama_server() -> Result<()> {
    let version = get_latest_llamacpp_version()?;
    println!("  llama.cpp release: {}", version);

    let asset_name = llama_asset_name(&version)?;
    let url = format!(
        "https://github.com/{LLAMACPP_REPO}/releases/download/{version}/{asset_name}"
    );

    let install_dir = install_bin_dir();
    std::fs::create_dir_all(&install_dir)?;

    let tmp_dir = install_dir.join("_llama_tmp");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;

    println!("  Downloading: {}", asset_name);
    let archive_path = tmp_dir.join(&asset_name);
    download_file(&url, &archive_path)?;

    println!("  Extracting...");
    extract_archive(&archive_path, &tmp_dir)?;

    let server_name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let server_bin = find_file_recursive(&tmp_dir, server_name)
        .with_context(|| format!("{server_name} not found in archive"))?;

    let dest = install_dir.join(server_name);
    std::fs::copy(&server_bin, &dest)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }

    copy_shared_libs(&tmp_dir, &install_dir)?;
    let _ = std::fs::remove_dir_all(&tmp_dir);

    println!("  Installed: {}", dest.display());
    Ok(())
}

fn get_latest_llamacpp_version() -> Result<String> {
    let output = Command::new("curl")
        .args(["-sI", &format!("https://github.com/{LLAMACPP_REPO}/releases/latest")])
        .output()
        .context("curl not available")?;

    let headers = String::from_utf8_lossy(&output.stdout);
    for line in headers.lines() {
        if line.to_lowercase().starts_with("location:") {
            if let Some(tag) = line.rsplit('/').next() {
                let tag = tag.trim();
                if !tag.is_empty() {
                    return Ok(tag.to_string());
                }
            }
        }
    }
    bail!("Could not determine latest llama.cpp release")
}

fn llama_asset_name(version: &str) -> Result<String> {
    if cfg!(target_os = "windows") {
        Ok(format!("llama-{version}-bin-win-cpu-x64.zip"))
    } else if cfg!(target_os = "linux") {
        Ok(format!("llama-{version}-bin-ubuntu-x64.tar.gz"))
    } else if cfg!(target_os = "macos") {
        Ok(format!("llama-{version}-bin-macos-arm64.tar.gz"))
    } else {
        bail!("Unsupported platform")
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn download_file(url: &str, dest: &Path) -> Result<()> {
    // curl -L follows redirects, --progress-bar shows progress
    let status = Command::new("curl")
        .args(["-L", "--progress-bar", "-o"])
        .arg(dest)
        .arg(url)
        .status();

    match status {
        Ok(s) if s.success() => return Ok(()),
        Ok(s) => {
            // Show the exit code for debugging
            eprintln!("  curl exited with code: {}", s.code().unwrap_or(-1));
        }
        Err(e) => {
            eprintln!("  curl error: {}", e);
        }
    }

    // Windows fallback: PowerShell
    if cfg!(windows) {
        println!("  Trying PowerShell...");
        let status = Command::new("powershell")
            .args([
                "-Command",
                &format!("Invoke-WebRequest -Uri '{}' -OutFile '{}'", url, dest.display()),
            ])
            .status()
            .context("PowerShell failed")?;
        if status.success() {
            return Ok(());
        }
    }

    bail!("Download failed: {}", url)
}

fn extract_archive(archive: &Path, dest_dir: &Path) -> Result<()> {
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let status = Command::new("tar")
            .arg("xzf").arg(archive).arg("-C").arg(dest_dir)
            .status().context("tar failed")?;
        if !status.success() { bail!("tar extraction failed"); }
    } else if name.ends_with(".zip") {
        if cfg!(windows) {
            let status = Command::new("powershell")
                .args(["-Command", &format!(
                    "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                    archive.display(), dest_dir.display()
                )])
                .status().context("zip extraction failed")?;
            if !status.success() { bail!("zip extraction failed"); }
        } else {
            let status = Command::new("unzip")
                .args(["-o", "-q"]).arg(archive).arg("-d").arg(dest_dir)
                .status().context("unzip failed")?;
            if !status.success() { bail!("zip extraction failed"); }
        }
    } else {
        bail!("Unknown archive format: {}", name);
    }
    Ok(())
}

fn copy_shared_libs(src_dir: &Path, dest_dir: &Path) -> Result<()> {
    let exts: &[&str] = if cfg!(windows) { &["dll"] } else if cfg!(target_os = "macos") { &["dylib"] } else { &["so"] };
    copy_files_by_ext(src_dir, dest_dir, exts)
}

fn copy_files_by_ext(dir: &Path, dest: &Path, exts: &[&str]) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()); };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if exts.iter().any(|&e| ext.starts_with(e)) {
                    let _ = std::fs::copy(&path, dest.join(path.file_name().unwrap()));
                }
            }
        } else if path.is_dir() {
            copy_files_by_ext(&path, dest, exts)?;
        }
    }
    Ok(())
}

fn find_file_recursive(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.exists() { return Some(direct); }

    let Ok(entries) = std::fs::read_dir(dir) else { return None; };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().map(|n| n == name).unwrap_or(false) {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, name) {
                return Some(found);
            }
        }
    }
    None
}

fn install_bin_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FORGE_INSTALL_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    {
        // Windows: %LOCALAPPDATA%\forge\bin (e.g. C:\Users\Michelle\AppData\Local\forge\bin)
        let base = std::env::var("LOCALAPPDATA")
            .or_else(|_| std::env::var("APPDATA"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        base.join("forge").join("bin")
    }
    #[cfg(not(windows))]
    {
        // Unix: ~/.local/bin
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".local").join("bin")
    }
}
