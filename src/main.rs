mod backend;
mod config;
mod conversation;
mod dream;
#[cfg(feature = "evolution")]
mod evolution;
mod formatting;
mod hooks;
mod inference;
mod permissions;
mod plugins;
mod rules;
#[cfg(feature = "search")]
mod search;
mod session;
mod skills;
mod tools;
mod setup;
mod tui;
mod update;

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

    /// Resume the most recent conversation
    #[arg(short, long)]
    resume: bool,

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
    /// Initialize a new Forge project in the current directory
    Init,
    /// One-command setup: install backend, download model, configure everything
    Setup,
    /// Check system health: backends, config, hardware
    Doctor,
    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Check for and install updates
    Update {
        /// Only check, don't install
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum PluginAction {
    /// List installed plugins
    List,
    /// Search the built-in plugin catalog
    Search {
        /// Search query (matches name, description, or category)
        query: String,
    },
    /// Install a plugin by catalog name or git URL
    Install {
        /// Plugin name from catalog, or a git URL
        name_or_url: String,
    },
    /// Uninstall an installed plugin
    Uninstall {
        /// Plugin name
        name: String,
    },
    /// Show details about an installed plugin
    Info {
        /// Plugin name
        name: String,
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
                println!("Temperature: {:.2}", config.model.temperature);
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
        Some(Commands::Plugin { action }) => {
            let plugins_dir = config::global_config_dir()?.join("plugins");
            std::fs::create_dir_all(&plugins_dir)?;
            plugins::builtins::ensure_builtin_plugins(&plugins_dir)?;

            match action {
                PluginAction::List => {
                    let mut mgr = plugins::PluginManager::new(plugins_dir);
                    mgr.load_all()?;
                    let installed = mgr.list();
                    if installed.is_empty() {
                        println!("No plugins installed.");
                        println!("Browse available plugins with: forge plugin search <query>");
                    } else {
                        println!("Installed plugins:\n");
                        for p in installed {
                            let m = &p.manifest.plugin;
                            println!("  {} v{}", m.name, m.version);
                            if !m.description.is_empty() {
                                println!("    {}", m.description);
                            }
                        }
                        println!("\n{} plugin(s) installed.", installed.len());
                    }
                }
                PluginAction::Search { query } => {
                    let q = query.to_lowercase();
                    let matches: Vec<_> = plugins::catalog::catalog()
                        .into_iter()
                        .filter(|e| {
                            e.name.to_lowercase().contains(&q)
                                || e.description.to_lowercase().contains(&q)
                                || e.category.to_lowercase().contains(&q)
                        })
                        .collect();

                    if matches.is_empty() {
                        println!("No plugins found matching \"{query}\".");
                    } else {
                        println!("Catalog results for \"{query}\":\n");
                        for entry in &matches {
                            println!("  {} [{}]", entry.name, entry.category);
                            println!("    {}", entry.description);
                            println!("    by {} — {}", entry.author, entry.repo);
                            println!();
                        }
                        println!("{} result(s).", matches.len());
                    }
                }
                PluginAction::Install { name_or_url } => {
                    let url = if let Some(entry) = plugins::catalog::find_in_catalog(&name_or_url) {
                        println!("Found \"{}\" in catalog.", entry.name);
                        entry.repo
                    } else if name_or_url.starts_with("https://") || name_or_url.contains("github.com") {
                        name_or_url.clone()
                    } else {
                        eprintln!("Plugin \"{name_or_url}\" not found in catalog.");
                        eprintln!("Use a git URL to install directly, e.g.:");
                        eprintln!("  forge plugin install https://github.com/user/repo");
                        std::process::exit(1);
                    };

                    let mgr = plugins::PluginManager::new(plugins_dir);
                    match mgr.install_from_git(&url) {
                        Ok(name) => println!("Plugin \"{name}\" installed successfully."),
                        Err(e) => {
                            eprintln!("Failed to install plugin: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                PluginAction::Uninstall { name } => {
                    let mut mgr = plugins::PluginManager::new(plugins_dir);
                    mgr.load_all()?;
                    match mgr.uninstall(&name) {
                        Ok(()) => println!("Plugin \"{name}\" uninstalled."),
                        Err(e) => {
                            eprintln!("Failed to uninstall plugin: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                PluginAction::Info { name } => {
                    let mut mgr = plugins::PluginManager::new(plugins_dir);
                    mgr.load_all()?;
                    let found = mgr.list().iter().find(|p| p.manifest.plugin.name == name);
                    match found {
                        Some(plugin) => {
                            let m = &plugin.manifest.plugin;
                            println!("Plugin: {}", m.name);
                            println!("Version: {}", m.version);
                            if !m.description.is_empty() {
                                println!("Description: {}", m.description);
                            }
                            if !m.author.is_empty() {
                                println!("Author: {}", m.author);
                            }
                            println!("Path: {}", plugin.dir.display());
                            println!("Tools: {}", plugin.manifest.tools.len());
                            println!("Skills: {}", plugin.manifest.skills.len());
                            println!("Hooks: {}", plugin.manifest.hooks.len());
                        }
                        None => {
                            eprintln!("Plugin \"{name}\" is not installed.");
                            if let Some(entry) = plugins::catalog::find_in_catalog(&name) {
                                eprintln!("It is available in the catalog:");
                                eprintln!("  {} — {}", entry.name, entry.description);
                                eprintln!("Install with: forge plugin install {}", entry.name);
                            }
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Some(Commands::Init) => {
            let ftai_dir = project_path.join(".ftai");
            let config_file = ftai_dir.join("config.toml");
            let ftai_md = project_path.join("FTAI.md");

            // Create .ftai/ directory
            std::fs::create_dir_all(&ftai_dir)?;
            println!("Created {}", ftai_dir.display());

            // Create .ftai/config.toml
            let dir_name = project_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "project".to_string());
            let config_content = format!("[project]\nname = \"{dir_name}\"\n");
            std::fs::write(&config_file, &config_content)?;
            println!("Created {}", config_file.display());

            // Create FTAI.md only if it doesn't exist
            if ftai_md.exists() {
                println!("FTAI.md already exists, skipping");
            } else {
                std::fs::write(&ftai_md, config::templates::default_ftai_md())?;
                println!("Created {}", ftai_md.display());
            }

            println!("\nForge project initialized in {}", project_path.display());
        }
        Some(Commands::Setup) => {
            setup::run_setup().await?;
        }
        Some(Commands::Doctor) => {
            println!("Forge Doctor");
            println!("============\n");

            // Backend probe
            let probe = backend::BackendProbeResults::probe();
            println!("{}\n", probe.display());

            // Hardware info
            let hw = backend::manager::BackendManager::hardware_info();
            println!("Hardware");
            println!("--------");
            println!("{hw:?}\n");

            // Config validity
            println!("Configuration");
            println!("-------------");
            println!("Backend: {:?}", config.model.backend);
            println!("Context length: {}", config.model.context_length);
            match &config.model.path {
                Some(path) => {
                    let exists = std::path::Path::new(path).exists();
                    println!(
                        "Model path: {path} ({})",
                        if exists { "exists" } else { "NOT FOUND" }
                    );
                }
                None => println!("Model path: (none configured)"),
            }

            // Config file locations
            let global = config::global_config_dir()?;
            println!(
                "Global config: {} ({})",
                global.join("config.toml").display(),
                if global.join("config.toml").exists() { "exists" } else { "missing" }
            );
            let project_ftai = project_path.join(".ftai").join("config.toml");
            println!(
                "Project config: {} ({})",
                project_ftai.display(),
                if project_ftai.exists() { "exists" } else { "not present" }
            );

            println!("\nAll checks complete.");
        }
        Some(Commands::Update { check }) => {
            // self_update uses blocking HTTP — run outside the async runtime
            let result = tokio::task::spawn_blocking(move || {
                if check {
                    match update::check_for_update() {
                        Ok((current, latest, available)) => {
                            println!("Current: v{current}");
                            println!("Latest:  v{latest}");
                            if available {
                                println!("\nUpdate available! Run `forge update` to install.");
                            } else {
                                println!("\nYou're on the latest version.");
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    update::perform_update()
                }
            }).await?;

            if let Err(e) = result {
                eprintln!("Update failed: {e}");
                std::process::exit(1);
            }
        }
        None => {
            // Default: start interactive TUI session
            let mut app = tui::TuiApp::new(config, project_path);
            if cli.resume {
                app.resume_last_session();
            }
            app.run().await?;
        }
    }

    Ok(())
}
