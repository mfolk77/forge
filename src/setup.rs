//! One-command setup: installs llama-server backend and downloads the right model.
//!
//! `forge setup` detects hardware, installs llama.cpp if needed, downloads the
//! recommended Qwen3.5 MoE model, and configures everything. One and done.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::backend::types::HardwareInfo;
use crate::config;

const LLAMACPP_REPO: &str = "ggerganov/llama.cpp";

/// Run the full setup process.
pub async fn run_setup() -> Result<()> {
    println!("┌─────────────────────────────────────┐");
    println!("│          Forge Setup                 │");
    println!("└─────────────────────────────────────┘");
    println!();

    // Step 1: Detect hardware
    println!("[1/4] Detecting hardware...");
    let hw = HardwareInfo::detect();
    println!("  Arch: {:?}", hw.arch);
    println!("  GPU:  {:?}", hw.gpu);
    println!("  RAM:  {} GB", hw.ram_gb);

    let recommendation = hw.recommended_model();
    println!("  Recommended model: {} ({} GB)", recommendation.name, recommendation.size_gb);
    println!("  Backend: {:?}", recommendation.backend);
    println!();

    // Step 2: Ensure llama-server is available (skip on Apple Silicon / MLX)
    let need_llamacpp = recommendation.backend == config::BackendType::LlamaCpp;

    if need_llamacpp {
        println!("[2/4] Checking for llama-server...");
        if find_llama_server().is_some() {
            println!("  llama-server found. Skipping install.");
        } else {
            println!("  llama-server not found. Installing...");
            install_llama_server()?;
        }
    } else {
        println!("[2/4] Using MLX backend (Apple Silicon). Skipping llama-server.");
    }
    println!();

    // Step 3: Download model
    println!("[3/4] Downloading model: {}...", recommendation.name);
    println!("  (This may take a while for large models)");
    let models_dir = config::global_config_dir()?.join("models");
    std::fs::create_dir_all(&models_dir)?;

    let model_name = hf_repo_for_recommendation(&recommendation.name);
    let model_dir = models_dir.join(recommendation.name.replace('/', "--"));

    if model_dir.exists() && has_model_files(&model_dir) {
        println!("  Model already downloaded. Skipping.");
    } else {
        download_model(&model_name, &model_dir).await?;
    }
    println!();

    // Step 4: Configure
    println!("[4/4] Configuring Forge...");
    let model_path = find_model_path(&model_dir)?;
    update_config(&recommendation, &model_path)?;
    println!("  Backend: {:?}", recommendation.backend);
    println!("  Model path: {}", model_path);
    println!();

    println!("┌─────────────────────────────────────────┐");
    println!("│  Setup complete! Run `forge` to start.   │");
    println!("└─────────────────────────────────────────┘");

    Ok(())
}

// ── llama-server detection ─────────────────────────────────────────────────

/// Find llama-server in PATH or common locations.
fn find_llama_server() -> Option<PathBuf> {
    let install_dir = install_bin_dir();
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        vec![
            install_dir.join("llama-server.exe"),
            PathBuf::from("llama-server.exe"),
        ]
    } else {
        vec![
            install_dir.join("llama-server"),
            PathBuf::from("/usr/local/bin/llama-server"),
            PathBuf::from("/opt/homebrew/bin/llama-server"),
        ]
    };

    // Check direct paths first
    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
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

// ── llama-server installation ──────────────────────────────────────────────

/// Install llama-server from pre-built GitHub releases.
fn install_llama_server() -> Result<()> {
    // Get the latest release tag
    let version = get_latest_llamacpp_version()?;
    println!("  Latest llama.cpp release: {}", version);

    let asset_name = llama_asset_name(&version)?;
    let url = format!(
        "https://github.com/{LLAMACPP_REPO}/releases/download/{version}/{asset_name}"
    );

    let install_dir = install_bin_dir();
    std::fs::create_dir_all(&install_dir)?;

    // Create a temporary directory for extraction
    let tmp_dir = install_dir.join("_llama_setup_tmp");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;

    println!("  Downloading: {}", asset_name);
    let archive_path = tmp_dir.join(&asset_name);
    download_file(&url, &archive_path)?;

    println!("  Extracting...");
    extract_archive(&archive_path, &tmp_dir)?;

    // Find llama-server binary in extracted contents
    let server_name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let server_bin = find_file_recursive(&tmp_dir, server_name)
        .with_context(|| format!("{server_name} not found in extracted archive"))?;

    let dest = install_dir.join(server_name);
    std::fs::copy(&server_bin, &dest)?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }

    // Also copy any shared libraries (CUDA runtime DLLs, etc.)
    copy_shared_libs(&tmp_dir, &install_dir)?;

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_dir);

    println!("  Installed: {}", dest.display());

    // Verify
    if find_llama_server().is_some() {
        println!("  llama-server verified and ready.");
    } else {
        println!();
        println!("  llama-server installed but not in PATH.");
        println!("  Add this to your shell profile:");
        println!("    export PATH=\"{}:$PATH\"", install_dir.display());
    }

    Ok(())
}

/// Get the latest llama.cpp release tag from GitHub.
fn get_latest_llamacpp_version() -> Result<String> {
    // Try curl + GitHub API
    let output = Command::new("curl")
        .args([
            "-sI",
            &format!("https://github.com/{LLAMACPP_REPO}/releases/latest"),
        ])
        .output()
        .context("Failed to check llama.cpp releases (curl not available)")?;

    let headers = String::from_utf8_lossy(&output.stdout);
    for line in headers.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("location:") {
            if let Some(tag) = line.rsplit('/').next() {
                let tag = tag.trim();
                if !tag.is_empty() {
                    return Ok(tag.to_string());
                }
            }
        }
    }

    bail!("Could not determine latest llama.cpp release. Check your internet connection.")
}

/// Build the correct asset filename for this platform.
fn llama_asset_name(version: &str) -> Result<String> {
    if cfg!(target_os = "windows") {
        if has_nvidia_gpu() {
            Ok(format!("llama-{version}-bin-win-cuda-12.4-x64.zip"))
        } else {
            Ok(format!("llama-{version}-bin-win-cpu-x64.zip"))
        }
    } else if cfg!(target_os = "linux") {
        if has_nvidia_gpu() {
            // Try CUDA 12.4 (most common)
            Ok(format!("llama-{version}-bin-ubuntu-x64.tar.gz"))
            // Note: CUDA version is separate, users can install cudart package
        } else {
            Ok(format!("llama-{version}-bin-ubuntu-x64.tar.gz"))
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            Ok(format!("llama-{version}-bin-macos-arm64.tar.gz"))
        } else {
            Ok(format!("llama-{version}-bin-macos-x64.tar.gz"))
        }
    } else {
        bail!("Unsupported platform for llama-server auto-install")
    }
}

/// Check if nvidia-smi is available (indicates NVIDIA GPU).
fn has_nvidia_gpu() -> bool {
    let smi = if cfg!(windows) { "nvidia-smi.exe" } else { "nvidia-smi" };
    Command::new(smi)
        .arg("--query-gpu=name")
        .arg("--format=csv,noheader")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── File download / extraction helpers ─────────────────────────────────────

/// Download a file. Uses curl (available on Linux, macOS, and modern Windows).
fn download_file(url: &str, dest: &Path) -> Result<()> {
    // curl is available on all target platforms (Win10+, Linux, macOS)
    let status = Command::new("curl")
        .args(["-fSL", "--progress-bar", url, "-o"])
        .arg(dest)
        .status();

    match status {
        Ok(s) if s.success() => return Ok(()),
        _ => {}
    }

    // Windows fallback: PowerShell
    if cfg!(windows) {
        let status = Command::new("powershell")
            .args([
                "-Command",
                &format!(
                    "Invoke-WebRequest -Uri '{}' -OutFile '{}'",
                    url,
                    dest.display()
                ),
            ])
            .status()
            .context("Download failed (neither curl nor PowerShell worked)")?;

        if status.success() {
            return Ok(());
        }
    }

    bail!("Download failed: {}", url)
}

/// Extract an archive (.tar.gz or .zip).
fn extract_archive(archive: &Path, dest_dir: &Path) -> Result<()> {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let status = Command::new("tar")
            .args(["xzf"])
            .arg(archive)
            .arg("-C")
            .arg(dest_dir)
            .status()
            .context("Failed to run tar")?;
        if !status.success() {
            bail!("tar extraction failed");
        }
    } else if name.ends_with(".zip") {
        if cfg!(windows) {
            let status = Command::new("powershell")
                .args([
                    "-Command",
                    &format!(
                        "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                        archive.display(),
                        dest_dir.display()
                    ),
                ])
                .status()
                .context("Failed to extract zip")?;
            if !status.success() {
                bail!("zip extraction failed");
            }
        } else {
            let status = Command::new("unzip")
                .args(["-o", "-q"])
                .arg(archive)
                .arg("-d")
                .arg(dest_dir)
                .status()
                .context("Failed to run unzip")?;
            if !status.success() {
                bail!("zip extraction failed");
            }
        }
    } else {
        bail!("Unknown archive format: {}", name);
    }

    Ok(())
}

/// Copy shared libraries (.so, .dll, .dylib) from extracted dir to install dir.
fn copy_shared_libs(src_dir: &Path, dest_dir: &Path) -> Result<()> {
    let lib_extensions: &[&str] = if cfg!(windows) {
        &["dll"]
    } else if cfg!(target_os = "macos") {
        &["dylib"]
    } else {
        &["so"]
    };

    copy_files_with_extensions(src_dir, dest_dir, lib_extensions)
}

fn copy_files_with_extensions(src_dir: &Path, dest_dir: &Path, extensions: &[&str]) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(src_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if extensions.iter().any(|&e| ext.starts_with(e)) {
                    let dest = dest_dir.join(path.file_name().unwrap());
                    let _ = std::fs::copy(&path, &dest);
                }
            }
        } else if path.is_dir() {
            copy_files_with_extensions(&path, dest_dir, extensions)?;
        }
    }
    Ok(())
}

/// Recursively find a file by name in a directory.
fn find_file_recursive(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.exists() {
        return Some(direct);
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
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

// ── Model download ─────────────────────────────────────────────────────────

/// Map a recommendation name to a HuggingFace repo ID.
fn hf_repo_for_recommendation(name: &str) -> String {
    match name {
        "Qwen3.5-35B-A3B-4bit" => "Qwen/Qwen3.5-35B-A3B-4bit".to_string(),
        "Qwen3.5-8B-A3B-4bit" => "Qwen/Qwen3.5-8B-A3B-4bit".to_string(),
        "Qwen3.5-35B-A3B-Q4_K_M-GGUF" => "Qwen/Qwen3.5-35B-A3B-Q4_K_M-GGUF".to_string(),
        "Qwen3.5-27B-A7B-Q4_K_M-GGUF" => "Qwen/Qwen3.5-27B-A7B-Q4_K_M-GGUF".to_string(),
        "Qwen3.5-8B-A3B-Q4_K_M-GGUF" => "Qwen/Qwen3.5-8B-A3B-Q4_K_M-GGUF".to_string(),
        other => format!("Qwen/{other}"),
    }
}

/// Download a model. Tries huggingface-cli first, then falls back to built-in downloader.
async fn download_model(model_name: &str, model_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(model_dir)?;

    // Try huggingface-cli first (fastest, handles auth + resume)
    if try_huggingface_cli(model_name, model_dir) {
        return Ok(());
    }

    // Try installing huggingface-cli and retrying
    println!("  Installing huggingface-hub...");
    let pip_installed = install_pip_package("huggingface-hub");
    if pip_installed && try_huggingface_cli(model_name, model_dir) {
        return Ok(());
    }

    // Fallback: use Forge's built-in async downloader
    println!("  Using built-in downloader...");
    let downloader = crate::inference::download::ModelDownloader::new()?;
    let progress = |downloaded: u64, total: u64| {
        if total > 0 {
            let pct = (downloaded as f64 / total as f64 * 100.0) as u64;
            let downloaded_mb = downloaded / (1024 * 1024);
            let total_mb = total / (1024 * 1024);
            eprint!(
                "\r  Downloading: {} / {} MB ({}%)    ",
                downloaded_mb, total_mb, pct
            );
        }
    };

    let models_dir = model_dir.parent().context("invalid model dir")?;
    downloader
        .download_model(model_name, models_dir, progress)
        .await?;
    eprintln!();
    println!("  Model downloaded successfully.");
    Ok(())
}

/// Try downloading via huggingface-cli. Returns true on success.
fn try_huggingface_cli(model_name: &str, model_dir: &Path) -> bool {
    println!("  Trying huggingface-cli download...");
    let status = Command::new("huggingface-cli")
        .args([
            "download",
            model_name,
            "--local-dir",
            &model_dir.to_string_lossy(),
        ])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("  Model downloaded successfully.");
            true
        }
        _ => {
            println!("  huggingface-cli not available or failed.");
            false
        }
    }
}

/// Try to install a pip package. Returns true on success.
fn install_pip_package(package: &str) -> bool {
    // Try pip3 first, then pip
    for pip in &["pip3", "pip"] {
        let args = if cfg!(target_os = "linux") {
            // Linux often needs --break-system-packages for system Python
            vec!["install", "--break-system-packages", "-q", package]
        } else {
            vec!["install", "-q", package]
        };

        if let Ok(status) = Command::new(pip).args(&args).status() {
            if status.success() {
                return true;
            }
        }
    }
    false
}

// ── Model file detection ───────────────────────────────────────────────────

/// Check if a model directory has actual model files.
fn has_model_files(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    has_model_files_recursive(dir, 0)
}

fn has_model_files_recursive(dir: &Path, depth: u32) -> bool {
    if depth > 2 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "gguf" || ext == "safetensors" {
                    return true;
                }
            }
        } else if path.is_dir() && depth < 2 {
            if has_model_files_recursive(&path, depth + 1) {
                return true;
            }
        }
    }
    false
}

/// Find the model file path (GGUF file or directory for MLX).
fn find_model_path(model_dir: &Path) -> Result<String> {
    // Check for GGUF files
    if let Ok(entries) = std::fs::read_dir(model_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("gguf") {
                return Ok(path.to_string_lossy().to_string());
            }
        }
    }

    // Check for safetensors (MLX format — return directory)
    if model_dir.join("config.json").exists() {
        return Ok(model_dir.to_string_lossy().to_string());
    }

    // Recurse one level (huggingface-cli sometimes nests files)
    if let Ok(entries) = std::fs::read_dir(model_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(found) = find_model_path(&path) {
                    return Ok(found);
                }
            }
        }
    }

    bail!("No model files found in {}", model_dir.display())
}

// ── Config update ──────────────────────────────────────────────────────────

/// Where to install binaries — ~/.local/bin on Unix, %USERPROFILE%\.local\bin on Windows.
fn install_bin_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FORGE_INSTALL_DIR") {
        return PathBuf::from(dir);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".local").join("bin")
}

/// Update ~/.ftai/config.toml with the model path and backend.
fn update_config(
    recommendation: &crate::backend::types::ModelRecommendation,
    model_path: &str,
) -> Result<()> {
    let config_path = config::global_config_dir()?.join("config.toml");
    let mut content = std::fs::read_to_string(&config_path).unwrap_or_default();

    let backend_str = match recommendation.backend {
        config::BackendType::Mlx => "mlx",
        config::BackendType::LlamaCpp | config::BackendType::Direct => "llamacpp",
    };

    // Update backend
    if content.contains("backend = ") {
        let re = regex::Regex::new(r#"(?m)^backend\s*=\s*"[^"]*""#).unwrap();
        content = re
            .replace(&content, format!(r#"backend = "{backend_str}""#))
            .to_string();
    }

    // Update or insert model.path
    if content.contains("path = ") {
        let re = regex::Regex::new(r#"(?m)^path\s*=\s*"[^"]*""#).unwrap();
        content = re
            .replace(&content, format!(r#"path = "{model_path}""#))
            .to_string();
    } else if content.contains("[model]") {
        content = content.replace(
            "[model]",
            &format!("[model]\npath = \"{model_path}\""),
        );
    } else {
        content.push_str(&format!(
            "\n[model]\nbackend = \"{backend_str}\"\npath = \"{model_path}\"\n"
        ));
    }

    std::fs::write(&config_path, &content)?;
    Ok(())
}
