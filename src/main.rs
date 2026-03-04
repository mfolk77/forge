mod backend;
mod config;
mod conversation;
mod formatting;
mod permissions;
mod rules;
mod tools;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::{ensure_ftai_dirs, load_config};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ftai", version, about = "FolkTech AI terminal coding harness")]
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
                println!("(Model download not yet implemented)");
            }
            ModelAction::Use { name } => {
                println!("Switching to model: {name}");
                println!("(Model switching not yet implemented)");
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
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
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
