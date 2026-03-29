mod backend;
mod config;
mod conversation;
mod evolution;
mod formatting;
mod inference;
mod permissions;
mod plugins;
mod rules;
mod search;
mod session;
mod skills;
mod tools;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::{ensure_ftai_dirs, load_config};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "forge", version, about = "FolkTech AI terminal coding harness")]
struct Cli {
    /// Project directory (defaults to current directory)
    #[arg(short, long)]
    project: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage local models
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },
    /// View or edit configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Subcommand)]
enum ModelAction {
    /// List installed models
    List,
    /// Install a model from HuggingFace
    Install {
        /// Model name or HuggingFace repo ID
        name: String,
    },
    /// Switch active model
    Use {
        /// Model name
        name: String,
    },
    /// Show current model and hardware info
    Info,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    /// Open config file in editor
    Edit,
}

/// Find the primary model path for a model directory.
/// For GGUF: returns the .gguf file path.
/// For MLX/safetensors: returns the directory (MLX loads from directory, not individual shards).
fn find_model_file(dir: &std::path::Path) -> Option<String> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut has_safetensors = false;
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                match ext {
                    // GGUF: single file, return the file path
                    "gguf" => return Some(path.to_string_lossy().to_string()),
                    // Safetensors: MLX loads from directory
                    "safetensors" => has_safetensors = true,
                    _ => {}
                }
            }
        }
        if has_safetensors {
            return Some(dir.to_string_lossy().to_string());
        }
    }
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Ensure ~/.ftai/ exists
    ensure_ftai_dirs()?;

    // Determine project path
    let project_path = cli.project.unwrap_or_else(|| {
        std::env::current_dir().expect("Could not determine current directory")
    });

    // Load config with precedence merging
    let config = load_config(Some(&project_path))?;

    match cli.command {
        Some(Commands::Model { action }) => match action {
            ModelAction::List => {
                println!("Installed models:");
                let models_dir = config::global_config_dir()?.join("models");
                if models_dir.exists() {
                    for entry in std::fs::read_dir(models_dir)? {
                        let entry = entry?;
                        if entry.file_type()?.is_dir() {
                            println!("  {}", entry.file_name().to_string_lossy());
                        }
                    }
                }
            }
            ModelAction::Install { name } => {
                println!("Installing model: {name}");
                let models_dir = config::global_config_dir()?.join("models");
                let dest = models_dir.join(&name);
                if dest.exists() {
                    println!("Model '{name}' already exists at {}", dest.display());
                    return Ok(());
                }
                std::fs::create_dir_all(&dest)?;

                // Try huggingface-cli first, fall back to direct download hint
                let status = std::process::Command::new("huggingface-cli")
                    .args(["download", &name, "--local-dir", &dest.to_string_lossy()])
                    .status();

                match status {
                    Ok(s) if s.success() => {
                        println!("Model installed to {}", dest.display());
                        println!("Activate with: forge model use {name}");
                    }
                    _ => {
                        // Clean up empty dir
                        let _ = std::fs::remove_dir(&dest);
                        eprintln!("huggingface-cli not found or download failed.");
                        eprintln!("Install it with: pip install huggingface-hub");
                        eprintln!("Or manually download the model to: {}", dest.display());
                        std::process::exit(1);
                    }
                }
            }
            ModelAction::Use { name } => {
                let models_dir = config::global_config_dir()?.join("models");
                let model_dir = models_dir.join(&name);

                if !model_dir.exists() {
                    // List available models
                    eprintln!("Model '{name}' not found.");
                    if models_dir.exists() {
                        let available: Vec<String> = std::fs::read_dir(&models_dir)?
                            .filter_map(|e| e.ok())
                            .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect();
                        if !available.is_empty() {
                            eprintln!("Available models:");
                            for m in &available {
                                eprintln!("  {m}");
                            }
                        } else {
                            eprintln!("No models installed. Use: forge model install <name>");
                        }
                    }
                    std::process::exit(1);
                }

                // Find the model file (GGUF or safetensors)
                let model_path = find_model_file(&model_dir)
                    .unwrap_or_else(|| model_dir.to_string_lossy().to_string());

                // Update config.toml
                let config_path = config::global_config_dir()?.join("config.toml");
                let mut config_content = std::fs::read_to_string(&config_path).unwrap_or_default();

                // Simple replacement: update or insert model.path
                if config_content.contains("path = ") {
                    // Replace existing path line under [model]
                    let re = regex::Regex::new(r#"(?m)^path\s*=\s*"[^"]*""#).unwrap();
                    config_content = re.replace(&config_content, format!(r#"path = "{model_path}""#)).to_string();
                } else if config_content.contains("[model]") {
                    config_content = config_content.replace(
                        "[model]",
                        &format!("[model]\npath = \"{model_path}\""),
                    );
                } else {
                    config_content.push_str(&format!("\n[model]\npath = \"{model_path}\"\n"));
                }

                std::fs::write(&config_path, &config_content)?;
                println!("Active model set to: {name}");
                println!("Path: {model_path}");
            }
            ModelAction::Info => {
                println!("Backend: {:?}", config.model.backend);
                println!("Context length: {}", config.model.context_length);
                println!("Temperature: {}", config.model.temperature);
                if let Some(path) = &config.model.path {
                    println!("Model path: {path}");
                } else {
                    println!("Model path: (none — will auto-detect on first run)");
                }
            }
        },
        Some(Commands::Config { action }) => match action {
            Some(ConfigAction::Show) | None => {
                let toml_str = toml::to_string_pretty(&config)?;
                println!("{toml_str}");
            }
            Some(ConfigAction::Edit) => {
                let config_path = config::global_config_dir()?.join("config.toml");
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
                    if cfg!(windows) { "notepad".to_string() } else { "vim".to_string() }
                });
                std::process::Command::new(editor)
                    .arg(&config_path)
                    .status()?;
            }
        },
        None => {
            // Default: start interactive TUI session
            let mut app = tui::TuiApp::new(config, project_path);
            app.run().await?;
        }
    }

    Ok(())
}
