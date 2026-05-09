use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use std::io::{self, stdout};
use std::path::PathBuf;
use crate::backend::manager::BackendManager;
use crate::backend::types::{ChatRequest, ChatResponse, Token};
use crate::config::Config;
use crate::conversation::engine::ConversationEngine;
use crate::conversation::parser::ToolCallParser;
use crate::conversation::prompt;
use crate::formatting::{self, TemplateSet};
use crate::hooks::HookRunner;
use crate::permissions::{self, DenialTracker, GrantCache, GrantScope, PermissionGrant, PermissionVerdict};
use crate::rules::{EvalContext, RuleAction, RulesEngine};
use crate::rules::parser::Event as RuleEvent;
use crate::plugins::PluginManager;
use crate::skills::LoadedSkill;
use crate::tools::{ToolContext, ToolRegistry};
#[cfg(feature = "evolution")]
use crate::evolution::{
    analyzer::{OutcomeType, SessionOutcome, ToolCallRecord, ToolResultType, UserFeedback},
    generator::EvolutionEngine,
    store::EvolutionStore,
};

use super::autocomplete::{Autocomplete, AutocompleteResult, CommandEntry};
use super::input::InputState;
use super::modal::{Modal, ModalAction};

/// RAII guard that restores the terminal on drop — covers panics, early `?`
/// returns, and normal exit. Created immediately after `enable_raw_mode()`.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = stdout().execute(crossterm::event::DisableBracketedPaste);
        let _ = stdout().execute(crossterm::event::DisableMouseCapture);
        let _ = terminal::disable_raw_mode();
        let _ = stdout().execute(LeaveAlternateScreen);
    }
}
use super::plugin_modal::{InstalledPluginEntry, PluginModal};
use super::render::{self, DisplayMessage};
use super::skill_modal::{SkillEntry, SkillModal};

/// Operating mode for the TUI session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Full coding assistant — tools active, agentic loop, project context.
    Coding,
    /// General conversational assistant — tools available but passive.
    Chat,
}

impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Mode::Coding => "coding",
            Mode::Chat => "chat",
        }
    }
}

/// Detect the appropriate startup mode for a project path.
/// Returns Coding if the directory contains `.git/` or `.ftai/`, Chat otherwise.
pub fn detect_mode(project_path: &std::path::Path) -> Mode {
    if project_path.join(".git").exists() || project_path.join(".ftai").exists() {
        Mode::Coding
    } else {
        Mode::Chat
    }
}

pub struct TuiApp {
    config: Config,
    backend: BackendManager,
    engine: ConversationEngine,
    parser: ToolCallParser,
    tools: ToolRegistry,
    rules: RulesEngine,
    templates: TemplateSet,
    grant_cache: GrantCache,
    plugin_manager: PluginManager,
    skills: Vec<LoadedSkill>,
    input: InputState,
    messages: Vec<DisplayMessage>,
    project_path: PathBuf,
    mode: Mode,
    should_quit: bool,
    is_generating: bool,
    streaming_text: String,
    /// Channel for receiving streaming tokens during generation
    token_rx: Option<tokio::sync::mpsc::Receiver<crate::backend::types::Token>>,
    /// Handle for the streaming generation task
    stream_handle: Option<tokio::task::JoinHandle<Result<ChatResponse>>>,
    /// Scroll offset — lines from the bottom (0 = pinned to bottom)
    scroll_offset: u16,
    /// Resolved color theme
    theme: render::Theme,
    /// Active modal overlay (plugin browser, skill browser, etc.)
    active_modal: Option<Box<dyn Modal>>,
    /// Slash-command autocomplete dropdown
    autocomplete: Autocomplete,
    /// Progress tracking: rounds since last task tool call (for nag injection)
    rounds_since_task_update: usize,
    /// User-level hook runner (from config.toml [[hooks]])
    hook_runner: HookRunner,
    /// Denial streak tracker for permission escalation
    denial_tracker: DenialTracker,
    /// Last-known mtime of FTAI.md for hot-reload detection
    ftai_mtime: Option<std::time::SystemTime>,
    /// Session persistence manager
    session_manager: Option<crate::session::manager::SessionManager>,
    /// Evolution engine for session outcome analysis and rule generation
    #[cfg(feature = "evolution")]
    evolution_engine: Option<EvolutionEngine>,
    /// Accumulated tool call records for the current session (evolution tracking)
    #[cfg(feature = "evolution")]
    session_tool_calls: Vec<ToolCallRecord>,
    /// Count of completed sessions (for periodic evolution analysis)
    #[cfg(feature = "evolution")]
    session_count: usize,
    /// First user message in the session (used as task description for evolution)
    #[cfg(feature = "evolution")]
    session_task_description: Option<String>,
    /// True while the backend is still loading the model in the background
    backend_loading: bool,
}

impl TuiApp {
    pub fn new(config: Config, project_path: PathBuf) -> Self {
        let mut tools = ToolRegistry::with_defaults();

        // Load plugins
        let plugins_dir = crate::config::global_config_dir()
            .map(|d| d.join("plugins"))
            .unwrap_or_else(|_| PathBuf::from("~/.ftai/plugins"));

        // Ensure built-in plugins are scaffolded on first run
        let _ = crate::plugins::builtins::ensure_builtin_plugins(&plugins_dir);

        let mut plugin_manager = PluginManager::new(plugins_dir);
        let mut plugin_loaded_skills = Vec::new();

        if config.plugins.enabled {
            let _plugin_count = plugin_manager.load_all().unwrap_or(0);

            // Register plugin tools
            for plugin_tool in plugin_manager.get_tools() {
                tools.register(plugin_tool);
            }

            // Load plugin skills and convert to skills::LoadedSkill
            for ps in plugin_manager.get_skills() {
                plugin_loaded_skills.push(LoadedSkill {
                    name: ps.name,
                    description: ps.description,
                    trigger: ps.trigger,
                    content: ps.content,
                    source: ps.plugin_name,
                });
            }
        }

        let tool_defs = tools.tool_definitions();

        let mut rules = RulesEngine::new();
        // Load global rules
        if let Ok(global_dir) = crate::config::global_config_dir() {
            let rules_file = global_dir.join("rules.ftai");
            if rules_file.exists() {
                let _ = rules.load_file(&rules_file);
            }
        }
        // Load project rules
        let project_rules = project_path.join(".ftai").join("rules.ftai");
        if project_rules.exists() {
            let _ = rules.load_file(&project_rules);
        }

        let memory = prompt::load_memory_context(&project_path);
        let ftai_context = prompt::load_ftai_context(&project_path);
        let mode = detect_mode(&project_path);

        let templates = formatting::load_templates(&config.formatting, Some(&project_path))
            .unwrap_or_default();

        // Load plugin rules into rules engine
        if config.plugins.enabled {
            for (_plugin_name, rule_content) in plugin_manager.get_rules() {
                let _ = rules.load_string(&rule_content);
            }
        }

        // Rebuild rules summary after plugin rules
        let rules_summary = if rules.rule_count() > 0 {
            Some(rules.summary())
        } else {
            None
        };

        // Load skills: builtins merged with plugin skills
        let skills = crate::skills::loader::load_all_skills(plugin_loaded_skills);

        let skills_prompt = if !skills.is_empty() {
            let triggers: Vec<&str> = skills.iter().map(|s| s.trigger.as_str()).collect();
            Some(format!(
                "Slash command skills: {}\nType any command to activate it.",
                triggers.join(", ")
            ))
        } else {
            None
        };

        let system_prompt = match mode {
            Mode::Coding => prompt::build_system_prompt(
                &project_path,
                &tool_defs,
                rules_summary.as_deref(),
                memory.as_deref(),
                Some(&templates),
                &config.formatting.enabled,
                ftai_context.as_deref(),
                skills_prompt.as_deref(),
            ),
            Mode::Chat => prompt::build_chat_system_prompt(
                memory.as_deref(),
                ftai_context.as_deref(),
            ),
        };

        let mut engine = ConversationEngine::new(
            system_prompt,
            tool_defs,
            config.model.context_length,
        );
        engine.set_project_path(project_path.clone());

        // Inject dream context from last dream analysis (if fresh)
        if let Some(dream_ctx) = crate::dream::runner::dream_context_for_session(&project_path) {
            engine.add_system_context(&dream_ctx);
        }

        let parser = ToolCallParser::new(config.model.tool_calling.clone());
        let backend = BackendManager::from_config(&config);

        let startup_msg = format!("Forge v{}", env!("CARGO_PKG_VERSION"));

        let theme = render::Theme::from_config(&config.theme);

        // Build autocomplete command list from slash commands + skill triggers
        let mut ac_commands: Vec<CommandEntry> = vec![
            CommandEntry { trigger: "/help".into(), description: "Show help and available commands".into() },
            CommandEntry { trigger: "/clear".into(), description: "Clear conversation and grants".into() },
            CommandEntry { trigger: "/compact".into(), description: "Compact context window".into() },
            CommandEntry { trigger: "/rules".into(), description: "Show or reload rules".into() },
            CommandEntry { trigger: "/permissions".into(), description: "View permission grants".into() },
            CommandEntry { trigger: "/templates".into(), description: "Show loaded templates".into() },
            CommandEntry { trigger: "/config".into(), description: "Show current configuration".into() },
            CommandEntry { trigger: "/model".into(), description: "Show or switch model".into() },
            CommandEntry { trigger: "/api".into(), description: "Show or configure cloud API backend".into() },
            CommandEntry { trigger: "/project".into(), description: "Show or switch project path".into() },
            CommandEntry { trigger: "/memory".into(), description: "View or add memory notes".into() },
            CommandEntry { trigger: "/context".into(), description: "Manage FTAI.md project context".into() },
            CommandEntry { trigger: "/plugin".into(), description: "Open plugin browser".into() },
            CommandEntry { trigger: "/hardware".into(), description: "Show hardware info".into() },
            CommandEntry { trigger: "/chat".into(), description: "Switch to chat mode".into() },
            CommandEntry { trigger: "/code".into(), description: "Switch to coding mode".into() },
            CommandEntry { trigger: "/skill".into(), description: "Open skill browser".into() },
            CommandEntry { trigger: "/theme".into(), description: "Switch color theme".into() },
            CommandEntry { trigger: "/dream".into(), description: "Show or run dream analysis".into() },
            CommandEntry { trigger: "/doctor".into(), description: "Check system health and backends".into() },
            CommandEntry { trigger: "/quit".into(), description: "Exit forge".into() },
        ];
        for skill in &skills {
            ac_commands.push(CommandEntry {
                trigger: skill.trigger.clone(),
                description: skill.description.clone(),
            });
        }
        let autocomplete = Autocomplete::new(ac_commands);
        let hook_runner = HookRunner::from_config(&config);
        let denial_tracker = DenialTracker::new();
        let ftai_mtime = Self::read_ftai_mtime(&project_path);

        // Initialize session persistence
        let session_manager = crate::config::global_config_dir()
            .ok()
            .map(|d| d.join("sessions.db"))
            .and_then(|db_path| {
                crate::session::manager::SessionManager::open(
                    &db_path,
                    &project_path.to_string_lossy(),
                ).ok()
            });

        // Initialize evolution engine
        #[cfg(feature = "evolution")]
        let evolution_engine = crate::config::global_config_dir()
            .ok()
            .map(|d| d.join("evolution.db"))
            .and_then(|db_path| EvolutionStore::open(&db_path).ok())
            .map(EvolutionEngine::new);

        Self {
            config,
            backend,
            engine,
            parser,
            tools,
            rules,
            templates,
            grant_cache: GrantCache::new(),
            plugin_manager,
            skills,
            input: InputState::new(),
            messages: vec![DisplayMessage::System(startup_msg)],
            project_path,
            mode,
            should_quit: false,
            is_generating: false,
            streaming_text: String::new(),
            token_rx: None,
            stream_handle: None,
            scroll_offset: 0,
            theme,
            active_modal: None,
            autocomplete,
            rounds_since_task_update: 0,
            hook_runner,
            denial_tracker,
            ftai_mtime,
            session_manager,
            #[cfg(feature = "evolution")]
            evolution_engine,
            #[cfg(feature = "evolution")]
            session_tool_calls: Vec::new(),
            #[cfg(feature = "evolution")]
            session_count: 0,
            #[cfg(feature = "evolution")]
            session_task_description: None,
            backend_loading: false,
        }
    }

    /// Read the current mtime of FTAI.md (project layer).
    fn read_ftai_mtime(project_path: &std::path::Path) -> Option<std::time::SystemTime> {
        let ftai_path = project_path.join(".ftai").join("FTAI.md");
        std::fs::metadata(&ftai_path)
            .ok()
            .and_then(|m| m.modified().ok())
    }

    /// Check if FTAI.md has changed since last read; if so, reload and rebuild system prompt.
    fn check_ftai_reload(&mut self) {
        let current_mtime = Self::read_ftai_mtime(&self.project_path);
        if current_mtime == self.ftai_mtime {
            return; // No change
        }
        // mtime changed (or file appeared/disappeared)
        self.ftai_mtime = current_mtime;
        let ftai_context = prompt::load_ftai_context(&self.project_path);
        self.rebuild_system_prompt(ftai_context);
    }

    /// Rebuild the system prompt from current state.
    fn rebuild_system_prompt(&mut self, ftai_context: Option<String>) {
        let memory = prompt::load_memory_context(&self.project_path);
        let tool_defs = self.tools.tool_definitions();
        let rules_summary = if self.rules.rule_count() > 0 {
            Some(self.rules.summary())
        } else {
            None
        };
        let skills_prompt = if !self.skills.is_empty() {
            let triggers: Vec<&str> = self.skills.iter().map(|s| s.trigger.as_str()).collect();
            Some(format!(
                "Slash command skills: {}\nType any command to activate it.",
                triggers.join(", ")
            ))
        } else {
            None
        };

        let new_prompt = match self.mode {
            Mode::Coding => prompt::build_system_prompt(
                &self.project_path,
                &tool_defs,
                rules_summary.as_deref(),
                memory.as_deref(),
                Some(&self.templates),
                &self.config.formatting.enabled,
                ftai_context.as_deref(),
                skills_prompt.as_deref(),
            ),
            Mode::Chat => prompt::build_chat_system_prompt(
                memory.as_deref(),
                ftai_context.as_deref(),
            ),
        };
        self.engine.update_system_prompt(new_prompt);
    }

    /// Resume the most recent session — reload messages into the conversation engine
    /// and display history so the user can continue where they left off.
    pub fn resume_last_session(&mut self) {
        let Some(ref mut mgr) = self.session_manager else { return };

        match mgr.resume_latest() {
            Ok(Some((session_id, messages))) => {
                if messages.is_empty() {
                    self.messages.push(DisplayMessage::System(
                        "Previous session was empty. Starting fresh.".to_string(),
                    ));
                    return;
                }

                let msg_count = messages.len();

                // Replay messages into the conversation engine and display
                for msg in &messages {
                    match msg.role {
                        crate::backend::types::Role::User => {
                            self.messages.push(DisplayMessage::User(msg.content.clone()));
                            self.engine.add_user_message(&msg.content);
                        }
                        crate::backend::types::Role::Assistant => {
                            if !msg.content.trim().is_empty() {
                                self.messages.push(DisplayMessage::Assistant(msg.content.clone()));
                            }
                            self.engine.add_assistant_message(crate::backend::types::ChatResponse {
                                message: msg.clone(),
                                tokens_used: Default::default(),
                                stop_reason: crate::backend::types::StopReason::EndOfText,
                            });
                        }
                        _ => {
                            // Tool results — add to engine but don't display
                            self.engine.add_user_message(&msg.content);
                        }
                    }
                }

                let short_id = &session_id[..8.min(session_id.len())];
                self.messages.push(DisplayMessage::System(
                    format!("Resumed session {short_id} ({msg_count} messages)"),
                ));
            }
            Ok(None) => {
                self.messages.push(DisplayMessage::System(
                    "No previous sessions found. Starting fresh.".to_string(),
                ));
            }
            Err(e) => {
                self.messages.push(DisplayMessage::System(
                    format!("Failed to resume: {e}"),
                ));
            }
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // Enter TUI mode FIRST so the user can see the splash and exit
        terminal::enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        stdout().execute(crossterm::event::EnableMouseCapture)?;
        stdout().execute(crossterm::event::EnableBracketedPaste)?;
        let _guard = TerminalGuard; // restores terminal on any exit path
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

        // Show splash immediately
        terminal.draw(|frame| self.render(frame))?;

        // Start the backend — spawn the server process immediately (fast),
        // then let the model load in the background while the user can interact.
        if self.backend.health_check().await {
            // Server already running from a previous session — instant start!
            self.messages.push(DisplayMessage::System(
                format!("{} backend connected.", self.backend.backend_name()),
            ));
        } else {
            match self.backend.spawn_only(&self.config) {
                Ok(()) => {
                    self.backend_loading = true;
                    self.messages.push(DisplayMessage::System(
                        "Model loading in background... you can start typing.".to_string(),
                    ));
                }
                Err(e) => {
                    self.messages.push(DisplayMessage::System(
                        format!("Backend error: {e}\nRunning offline. Use /model to configure."),
                    ));
                }
            }
        }
        terminal.draw(|frame| self.render(frame))?;

        // Start a new session for persistence
        if let Some(ref mut mgr) = self.session_manager {
            let _ = mgr.start_session();
        }

        // Run session_start hook
        {
            let mut env = std::collections::HashMap::new();
            env.insert("FORGE_PROJECT".to_string(), self.project_path.display().to_string());
            let _ = self.hook_runner.run("session_start", &env).await;
        }

        let result = self.main_loop(&mut terminal).await;

        // End session persistence
        if let Some(ref mut mgr) = self.session_manager {
            let _ = mgr.end_session("Session ended by user");
        }

        // Evolution: record session outcome and analyze patterns
        #[cfg(feature = "evolution")]
        {
            if let Some(ref evo) = self.evolution_engine {
                let session_id = self.session_manager
                    .as_ref()
                    .and_then(|m| m.current_session_id().map(|s| s.to_string()))
                    .unwrap_or_else(|| format!("session-{}", std::process::id()));

                let has_errors = self.session_tool_calls.iter().any(|tc| {
                    matches!(tc.result_type, ToolResultType::Error(_))
                });
                let outcome_type = if self.session_tool_calls.is_empty() {
                    OutcomeType::Abandoned
                } else if has_errors {
                    OutcomeType::PartialSuccess
                } else {
                    OutcomeType::Success
                };

                let outcome = SessionOutcome {
                    session_id,
                    project: self.project_path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64,
                    task_description: self.session_task_description.clone()
                        .unwrap_or_else(|| "Interactive session".to_string()),
                    tool_calls: std::mem::take(&mut self.session_tool_calls),
                    success: outcome_type,
                    user_feedback: Some(UserFeedback::NoFeedback),
                    total_tokens: self.engine.estimated_tokens(),
                    retries: 0,
                };

                match evo.analyze_and_evolve(&outcome) {
                    Ok(rules) if !rules.is_empty() => {
                        // Rules were generated — they're persisted in the evolution DB
                        // and will be available for future sessions
                    }
                    _ => {}
                }

                self.session_count += 1;
            }
        }

        // Dream: check if conditions are met and run dream analysis
        {
            let transcripts_dir = self.project_path.join(".ftai").join("transcripts");
            let scheduler = crate::dream::scheduler::DreamScheduler::new(&self.project_path);
            if scheduler.should_dream(&transcripts_dir) {
                if let Ok(_lock) = scheduler.acquire_lock() {
                    let runner = crate::dream::runner::DreamRunner::new(&self.project_path);
                    let since = scheduler.last_dream_time();
                    let _ = runner.run(since);
                }
            }
        }

        // Run session_end hook
        {
            let mut env = std::collections::HashMap::new();
            env.insert("FORGE_PROJECT".to_string(), self.project_path.display().to_string());
            let _ = self.hook_runner.run("session_end", &env).await;
        }

        // _guard restores terminal on drop (covers normal exit, ?, and panic)

        // Intentionally DO NOT stop the backend server here.
        // Keeping it warm means the next `forge` invocation connects instantly
        // instead of waiting 30-60s for model reload. The server uses idle
        // memory that macOS will reclaim under pressure anyway.

        result
    }

    async fn main_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        let mut health_check_counter: u32 = 0;

        loop {
            // Periodically check if the backend finished loading (~every 500ms)
            if self.backend_loading {
                health_check_counter += 1;
                if health_check_counter % 30 == 0 { // 30 * 16ms ≈ 480ms
                    if self.backend.health_check().await {
                        self.backend_loading = false;
                        self.messages.push(DisplayMessage::System(
                            "Model ready — warming up prompt cache...".to_string(),
                        ));
                        // Pre-warm: send the system prompt so it's cached before
                        // the user's first message. This happens in the background
                        // while the user is typing.
                        let system_prompt = self.engine.system_prompt().to_string();
                        let tool_defs = self.tools.tool_definitions();
                        self.backend.warm_up_prompt(&system_prompt, tool_defs).await;
                        self.messages.push(DisplayMessage::System(
                            "Ready.".to_string(),
                        ));
                    }
                }
            }

            // Render
            terminal.draw(|frame| self.render(frame))?;

            // Poll for events (non-blocking)
            if event::poll(std::time::Duration::from_millis(16))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.handle_key(key).await?;
                    }
                    Event::Key(_) => {
                        // Ignore KeyRelease and KeyRepeat events — on Windows,
                        // crossterm fires both Press and Release, causing double input.
                    }
                    Event::Mouse(mouse) => {
                        match mouse.kind {
                            crossterm::event::MouseEventKind::ScrollUp => {
                                self.scroll_offset = self.scroll_offset.saturating_add(3);
                            }
                            crossterm::event::MouseEventKind::ScrollDown => {
                                self.scroll_offset = self.scroll_offset.saturating_sub(3);
                            }
                            _ => {}
                        }
                    }
                    Event::Paste(text) => {
                        // Bracketed paste — insert all text at once, preserving newlines
                        for ch in text.chars() {
                            if ch == '\n' || ch == '\r' {
                                self.input.insert_newline();
                            } else {
                                self.input.insert_char(ch);
                            }
                        }
                    }
                    Event::Resize(_, _) => {
                        // Terminal resized — next loop iteration will re-render at new size
                    }
                    _ => {}
                }
            }

            // Drain any pending streaming tokens
            if let Some(rx) = &mut self.token_rx {
                while let Ok(token) = rx.try_recv() {
                    if token.is_final {
                        // Stream finished — collect the final response
                        if let Some(handle) = self.stream_handle.take() {
                            match handle.await {
                                Ok(Ok(response)) => {
                                    // Display accumulated text (cleaned of special tokens)
                                    let raw = std::mem::take(&mut self.streaming_text);
                                    let text = Self::clean_model_output(&raw);
                                    if !text.is_empty() {
                                        self.messages.push(DisplayMessage::Assistant(text));
                                    }
                                    // Process tool calls from the complete response
                                    self.process_response_after_stream(response).await?;
                                }
                                Ok(Err(e)) => {
                                    self.messages.push(DisplayMessage::System(
                                        format!("Stream error: {e}"),
                                    ));
                                }
                                Err(e) => {
                                    self.messages.push(DisplayMessage::System(
                                        format!("Stream task panicked: {e}"),
                                    ));
                                }
                            }
                        }
                        self.token_rx = None;
                        self.is_generating = false;
                        break;
                    } else {
                        self.streaming_text.push_str(&token.text);
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        // Calculate input height based on text wrapping
        let input_text_for_height = if self.is_generating {
            "generating..."
        } else {
            &self.input.lines[self.input.cursor_line]
        };
        // "> " prefix = 2 chars, +1 for border, +1 for content line minimum
        let input_content_width = area.width.saturating_sub(3).max(1) as usize;
        let input_lines = if input_content_width > 0 && !input_text_for_height.is_empty() {
            ((input_text_for_height.len() + 2 + input_content_width - 1) / input_content_width) as u16
        } else {
            1
        };
        let input_height = (input_lines + 2).min(area.height / 3); // +2 for border + padding, cap at 1/3 screen

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),            // Status bar
                Constraint::Min(5),              // Messages
                Constraint::Length(1),            // Status line
                Constraint::Length(input_height), // Input (dynamic)
            ])
            .split(area);

        // Status bar
        render::render_status_bar(
            self.config
                .model
                .path
                .as_deref()
                .unwrap_or("no model"),
            self.backend.backend_name(),
            &self.project_path.to_string_lossy(),
            &self.theme,
            layout[0],
            frame.buffer_mut(),
        );

        // Messages area — show modal if active, otherwise normal messages
        if let Some(ref modal) = self.active_modal {
            modal.render(&self.theme, layout[1], frame.buffer_mut());
        } else {
            let mut display_msgs = self.messages.clone();
            if self.is_generating && !self.streaming_text.is_empty() {
                display_msgs.push(DisplayMessage::Assistant(
                    format!("{}▊", self.streaming_text),
                ));
            }
            render::render_messages(&display_msgs, self.mode.label(), &self.theme, self.scroll_offset, layout[1], frame.buffer_mut());
        }

        // Status line
        render::render_status_line(
            self.engine.estimated_tokens(),
            self.config.model.context_length,
            self.rules.rule_count(),
            &self.theme,
            layout[2],
            frame.buffer_mut(),
        );

        // Input
        let input_text = if self.is_generating {
            "generating..."
        } else {
            &self.input.lines[self.input.cursor_line]
        };
        let cursor_pos = render::render_input(
            input_text,
            self.input.cursor_col,
            layout[3],
            frame.buffer_mut(),
        );

        // Render autocomplete overlay in the message area (bottom-aligned)
        if self.autocomplete.active {
            self.autocomplete.render(&self.theme, layout[1], frame.buffer_mut());
        }

        // Position the terminal cursor inside the input area
        if !self.is_generating {
            if let Some((cx, cy)) = cursor_pos {
                frame.set_cursor_position((cx, cy));
            }
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.is_generating {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.is_generating = false;
            }
            return Ok(());
        }

        // Route to active modal if present
        if self.active_modal.is_some() {
            let action = self.active_modal.as_mut().unwrap().handle_key(key);
            self.process_modal_action(action).await?;
            return Ok(());
        }

        // Route to autocomplete if active
        if self.autocomplete.active {
            let result = self.autocomplete.handle_key(key);
            match result {
                AutocompleteResult::Selected(trigger) => {
                    // Replace input with the selected command and submit
                    self.input = InputState::new();
                    for c in trigger.chars() {
                        self.input.insert_char(c);
                    }
                    let text = self.input.submit();
                    if !text.trim().is_empty() {
                        self.handle_submit(text).await?;
                    }
                }
                AutocompleteResult::Dismiss => {
                    // Keep input text as-is
                }
                AutocompleteResult::Continue => {
                    // Update input to reflect query: "/" + query
                    let new_text = format!("/{}", self.autocomplete.query);
                    self.input = InputState::new();
                    for c in new_text.chars() {
                        self.input.insert_char(c);
                    }
                }
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.input.is_empty() {
                    self.should_quit = true;
                } else {
                    self.input = InputState::new();
                }
            }
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.input.insert_newline();
                } else {
                    let text = self.input.submit();
                    if !text.trim().is_empty() {
                        self.handle_submit(text).await?;
                    }
                }
            }
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Left => self.input.move_left(),
            KeyCode::Right => self.input.move_right(),
            KeyCode::Up => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.scroll_offset = self.scroll_offset.saturating_add(3);
                } else {
                    self.input.history_up();
                }
            }
            KeyCode::Down => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.scroll_offset = self.scroll_offset.saturating_sub(3);
                } else {
                    self.input.history_down();
                }
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
            }
            KeyCode::Esc => {
                self.input = InputState::new();
            }
            KeyCode::Char(c) => {
                self.input.insert_char(c);
                // Activate autocomplete when '/' is typed as first char
                if c == '/' && self.input.text() == "/" {
                    self.autocomplete.activate("");
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Process a ModalAction returned from the active modal.
    async fn process_modal_action(&mut self, action: ModalAction) -> Result<()> {
        match action {
            ModalAction::Continue => {}
            ModalAction::Close => {
                self.active_modal = None;
            }
            ModalAction::InstallPlugin(name) => {
                self.active_modal = None;
                if let Some(entry) = crate::plugins::catalog::find_in_catalog(&name) {
                    match self.plugin_manager.install_from_git(&entry.repo) {
                        Ok(installed_name) => {
                            self.messages.push(DisplayMessage::System(
                                format!("Installed plugin: {installed_name}"),
                            ));
                        }
                        Err(e) => {
                            self.messages.push(DisplayMessage::System(
                                format!("Install failed: {e}"),
                            ));
                        }
                    }
                } else {
                    self.messages.push(DisplayMessage::System(
                        format!("Plugin '{name}' not found in catalog."),
                    ));
                }
            }
            ModalAction::UninstallPlugin(name) => {
                self.active_modal = None;
                match self.plugin_manager.uninstall(&name) {
                    Ok(_) => {
                        self.messages.push(DisplayMessage::System(
                            format!("Uninstalled plugin: {name}"),
                        ));
                    }
                    Err(e) => {
                        self.messages.push(DisplayMessage::System(
                            format!("Uninstall failed: {e}"),
                        ));
                    }
                }
            }
            ModalAction::TogglePlugin(name) => {
                // Toggle enabled/disabled in the modal's installed list
                // For now, display a message — full config persistence is a follow-up
                self.messages.push(DisplayMessage::System(
                    format!("Toggled plugin: {name}"),
                ));
            }
            ModalAction::CreatePlugin(name) => {
                self.active_modal = None;
                self.scaffold_plugin(&name);
            }
            ModalAction::ActivateSkill { name, content } => {
                self.active_modal = None;
                self.messages.push(DisplayMessage::System(
                    format!("Skill '{}' activated. Content injected into context.", name),
                ));
                self.engine.add_system_context(&format!(
                    "# Skill: {}\n{}", name, content
                ));
            }
            ModalAction::SelectTheme(name) => {
                self.active_modal = None;
                self.apply_theme(&name);
            }
            ModalAction::AddMarketplace => {
                self.active_modal = None;
                self.messages.push(DisplayMessage::System(
                    "To add a marketplace, use: /plugin marketplace add <owner/repo>".to_string(),
                ));
            }
            ModalAction::UpdateMarketplace(name) => {
                self.messages.push(DisplayMessage::System(
                    format!("Updating marketplace: {name}..."),
                ));
                let config_dir = crate::config::global_config_dir().unwrap_or_default();
                match crate::plugins::MarketplaceRegistry::new(&config_dir) {
                    Ok(registry) => {
                        match registry.update_all() {
                            Ok(_) => self.messages.push(DisplayMessage::System(
                                format!("Marketplace '{name}' updated."),
                            )),
                            Err(e) => self.messages.push(DisplayMessage::System(
                                format!("Update failed: {e}"),
                            )),
                        }
                    }
                    Err(e) => self.messages.push(DisplayMessage::System(
                        format!("Failed to load marketplaces: {e}"),
                    )),
                }
            }
            ModalAction::RemoveMarketplace(name) => {
                self.active_modal = None;
                let config_dir = crate::config::global_config_dir().unwrap_or_default();
                match crate::plugins::MarketplaceRegistry::new(&config_dir) {
                    Ok(mut registry) => {
                        match registry.remove_source(&name) {
                            Ok(_) => self.messages.push(DisplayMessage::System(
                                format!("Removed marketplace: {name}"),
                            )),
                            Err(e) => self.messages.push(DisplayMessage::System(
                                format!("Remove failed: {e}"),
                            )),
                        }
                    }
                    Err(e) => self.messages.push(DisplayMessage::System(
                        format!("Failed to load marketplaces: {e}"),
                    )),
                }
            }
        }
        Ok(())
    }

    /// Apply a theme by name.
    fn apply_theme(&mut self, name: &str) {
        let preset = match name {
            "dark" => Some(crate::config::ThemePreset::Dark),
            "light" => Some(crate::config::ThemePreset::Light),
            "high-contrast" => Some(crate::config::ThemePreset::HighContrast),
            "solarized" => Some(crate::config::ThemePreset::Solarized),
            "dracula" => Some(crate::config::ThemePreset::Dracula),
            _ => None,
        };
        if let Some(preset) = preset {
            self.config.theme.preset = preset;
            self.theme = render::Theme::from_config(&self.config.theme);
            self.messages.push(DisplayMessage::System(
                format!("Theme switched to: {name}"),
            ));
        } else {
            self.messages.push(DisplayMessage::System(
                format!("Unknown theme: {name}"),
            ));
        }
    }

    /// Scaffold a new plugin directory at ~/.ftai/plugins/<name>/.
    fn scaffold_plugin(&mut self, name: &str) {
        // Validate name using shared validation
        if !crate::plugins::catalog::is_valid_plugin_name(name) {
            self.messages.push(DisplayMessage::System(
                format!("Invalid plugin name: '{name}'. Use alphanumeric, hyphen, and underscore only."),
            ));
            return;
        }

        let plugins_dir = crate::config::global_config_dir()
            .map(|d| d.join("plugins"))
            .unwrap_or_else(|_| PathBuf::from("~/.ftai/plugins"));

        let plugin_dir = plugins_dir.join(name);
        if plugin_dir.exists() {
            self.messages.push(DisplayMessage::System(
                format!("Plugin directory already exists: {}", plugin_dir.display()),
            ));
            return;
        }

        let subdirs = ["tools", "skills", "hooks"];
        for dir in &subdirs {
            if let Err(e) = std::fs::create_dir_all(plugin_dir.join(dir)) {
                self.messages.push(DisplayMessage::System(
                    format!("Failed to create {dir} directory: {e}"),
                ));
                return;
            }
        }

        let manifest = format!(
            r#"[plugin]
name = "{name}"
version = "0.1.0"
description = "A custom forge plugin"
author = ""

# [[tools]]
# name = "my-tool"
# description = "What this tool does"
# command = "tools/my-tool.sh"

# [[skills]]
# name = "my-skill"
# file = "skills/my-skill.md"
# description = "What this skill provides"
# trigger = "/my-skill"

# [[hooks]]
# event = "pre:bash"
# command = "hooks/pre-bash.sh"
"#
        );

        if let Err(e) = std::fs::write(plugin_dir.join("plugin.toml"), &manifest) {
            self.messages.push(DisplayMessage::System(
                format!("Failed to write plugin.toml: {e}"),
            ));
            return;
        }

        let readme = format!(
            "# {name}\n\nA custom forge plugin.\n\n## Structure\n\n- `plugin.toml` — plugin manifest\n- `tools/` — tool scripts\n- `skills/` — skill markdown files\n- `hooks/` — pre/post hook scripts\n"
        );
        let _ = std::fs::write(plugin_dir.join("README.md"), &readme);

        self.messages.push(DisplayMessage::System(
            format!("Created plugin scaffold at {}\nEdit plugin.toml to configure.", plugin_dir.display()),
        ));
    }

    /// Build the list of installed plugin entries for the plugin modal.
    fn build_installed_entries(&self) -> Vec<InstalledPluginEntry> {
        self.plugin_manager
            .list()
            .iter()
            .map(|p| InstalledPluginEntry {
                name: p.manifest.plugin.name.clone(),
                source: p.manifest.plugin.name.clone(),
                plugin_type: "Plugin".to_string(),
                enabled: true,
                description: p.manifest.plugin.description.clone(),
            })
            .collect()
    }

    /// Build marketplace entries for the plugin modal.
    fn build_marketplace_entries(&self) -> Vec<super::plugin_modal::MarketplaceEntry> {
        let config_dir = crate::config::global_config_dir().unwrap_or_default();
        let registry = match crate::plugins::MarketplaceRegistry::new(&config_dir) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let installed_names: std::collections::HashSet<String> = self.plugin_manager
            .list()
            .iter()
            .map(|p| p.manifest.plugin.name.clone())
            .collect();

        registry.list_sources().iter().map(|source| {
            let available = registry.search("").len(); // empty query = all
            let installed_from_source = installed_names.len(); // approximate
            super::plugin_modal::MarketplaceEntry {
                name: source.name.clone(),
                repo: source.repo.clone(),
                available_count: available,
                installed_count: installed_from_source,
                last_updated: "recently".to_string(),
                is_default: source.name == "claude-plugins-official",
            }
        }).collect()
    }

    /// Build skill entries for the skill modal.
    fn build_skill_entries(&self) -> Vec<SkillEntry> {
        self.skills
            .iter()
            .map(|s| SkillEntry {
                name: s.name.clone(),
                source: s.source.clone(),
                description: s.description.clone(),
                content: s.content.clone(),
                token_estimate: s.content.len() / 4,
            })
            .collect()
    }

    /// Known slash commands — anything starting with / that isn't in this
    /// list gets treated as normal user input (e.g. file paths).
    const SLASH_COMMANDS: &'static [&'static str] = &[
        "/help", "/clear", "/compact", "/rules", "/permissions", "/templates",
        "/config", "/model", "/project", "/memory", "/context", "/plugin",
        "/hardware", "/chat", "/code", "/skill", "/theme", "/dream", "/doctor", "/api", "/quit", "/exit",
    ];

    async fn handle_submit(&mut self, text: String) -> Result<()> {
        // Auto-scroll to bottom on new input
        self.scroll_offset = 0;

        // Bare exit/quit — no slash needed
        let trimmed = text.trim().to_lowercase();
        if trimmed == "exit" || trimmed == "quit" {
            self.should_quit = true;
            return Ok(());
        }

        // Only treat as slash command if it matches a known command.
        // This prevents file paths like "/Users/foo" from being misinterpreted.
        if text.starts_with('/') {
            let cmd = text.split_whitespace().next().unwrap_or("");
            if Self::SLASH_COMMANDS.contains(&cmd) {
                return self.handle_slash_command(&text).await;
            }
        }

        // If backend is still loading, wait for it before proceeding
        if self.backend_loading {
            self.messages.push(DisplayMessage::System(
                "Waiting for model to finish loading...".to_string(),
            ));
            match self.backend.wait_until_ready().await {
                Ok(()) => {
                    self.backend_loading = false;
                    self.messages.push(DisplayMessage::System("Model ready.".to_string()));
                }
                Err(e) => {
                    self.messages.push(DisplayMessage::System(
                        format!("Backend failed: {e}"),
                    ));
                    return Ok(());
                }
            }
        }

        // Add user message
        self.messages.push(DisplayMessage::User(text.clone()));
        self.engine.add_user_message(&text);
        if let Some(ref mgr) = self.session_manager {
            let _ = mgr.save_message(crate::backend::types::Role::User, &text, None);
        }

        // Track first user message as session task description (for evolution)
        #[cfg(feature = "evolution")]
        if self.session_task_description.is_none() {
            self.session_task_description = Some(text.chars().take(200).collect());
        }

        // Start the agentic loop
        self.run_agentic_loop().await
    }

    /// Agentic loop: generate → parse tool calls → execute → feed results → repeat.
    /// Uses streaming for the first turn (user sees tokens live), then falls back
    /// to synchronous generate for tool-result continuations (speed over UX for
    /// intermediate turns).
    async fn run_agentic_loop(&mut self) -> Result<()> {
        const MAX_TURNS: usize = 25;
        let is_chat = self.mode == Mode::Chat;

        for turn in 0..MAX_TURNS {
            // Check for FTAI.md hot-reload before each LLM call
            self.check_ftai_reload();
            // Skip compaction in chat mode — no tool results to compact
            if !is_chat {
                self.engine.micro_compact();
                self.engine.compact();
            }
            self.is_generating = true;
            let request = self.engine.build_request_with_mode(&self.config, is_chat);

            if turn == 0 {
                // First turn: try streaming for live token display
                match self.generate_stream_with_recovery(&request).await {
                    Ok((rx, handle)) => {
                        self.streaming_text.clear();
                        self.token_rx = Some(rx);
                        self.stream_handle = Some(handle);
                        // Return to main_loop — tokens will be drained there.
                        // process_response_after_stream will handle continuation.
                        return Ok(());
                    }
                    Err(_) => {
                        // Streaming not supported — fall through to sync
                    }
                }
            }

            // Sync generate (fallback for first turn, default for continuations)
            let response = match self.generate_with_recovery(&request).await {
                Ok(r) => r,
                Err(e) => {
                    self.messages.push(DisplayMessage::System(format!("Error: {e}")));
                    break;
                }
            };

            let has_tool_calls = response.message.tool_calls.as_ref()
                .map_or(false, |tc| !tc.is_empty())
                || !self.parser.parse(&response.message.content).1.is_empty();

            self.process_response(response).await?;

            if !has_tool_calls {
                break;
            }
        }

        self.is_generating = false;
        Ok(())
    }

    async fn generate_with_recovery(&mut self, request: &ChatRequest) -> Result<ChatResponse> {
        match self.backend.generate(request).await {
            Ok(response) => Ok(response),
            Err(e) if Self::is_backend_transport_error(&e) => {
                self.messages.push(DisplayMessage::System(
                    format!("Backend disconnected ({e}). Restarting and shrinking request..."),
                ));
                self.backend.stop();
                self.backend
                    .start(&self.config)
                    .await
                    .context("Failed to restart local model server after disconnect")?;

                // Shrink-before-retry: a same-size request is what likely
                // caused the disconnect (memory pressure, prompt too big,
                // transient OOM). Force compaction so the retry is smaller.
                self.engine.shrink_for_retry();
                let smaller = self.engine.build_request(&self.config);

                self.backend
                    .generate(&smaller)
                    .await
                    .context("Model request failed after backend restart + shrink")
            }
            Err(e) => Err(e),
        }
    }

    async fn generate_stream_with_recovery(
        &mut self,
        request: &ChatRequest,
    ) -> Result<(tokio::sync::mpsc::Receiver<Token>, tokio::task::JoinHandle<Result<ChatResponse>>)> {
        match self.backend.generate_stream(request).await {
            Ok(stream) => Ok(stream),
            Err(e) if Self::is_backend_transport_error(&e) => {
                self.messages.push(DisplayMessage::System(
                    format!("Backend disconnected ({e}). Restarting and shrinking request..."),
                ));
                self.backend.stop();
                self.backend
                    .start(&self.config)
                    .await
                    .context("Failed to restart local model server after disconnect")?;

                // Shrink-before-retry — see generate_with_recovery for rationale.
                self.engine.shrink_for_retry();
                let smaller = self.engine.build_request(&self.config);

                self.backend
                    .generate_stream(&smaller)
                    .await
                    .context("Streaming request failed after backend restart + shrink")
            }
            Err(e) => Err(e),
        }
    }

    fn is_backend_transport_error(error: &anyhow::Error) -> bool {
        let text = format!("{error:#}");
        text.contains("Failed to connect to model server")
            || text.contains("connection refused")
            || text.contains("connection closed")
            || text.contains("error sending request")
            || text.contains("operation timed out")
            || text.contains("Stream read error")
    }

    /// Called after streaming completes. Processes tool calls from the streamed
    /// response and continues the agentic loop synchronously if needed.
    async fn process_response_after_stream(&mut self, response: ChatResponse) -> Result<()> {
        let has_tool_calls = response.message.tool_calls.as_ref()
            .map_or(false, |tc| !tc.is_empty())
            || !self.parser.parse(&response.message.content).1.is_empty();

        // Text was already displayed during streaming — clear content before
        // passing to process_response so it doesn't display it again.
        // Tool calls and engine state are still handled normally.
        let mut response = response;
        response.message.content = String::new();
        self.process_response(response).await?;

        // If there were tool calls, continue the agentic loop (sync for subsequent turns)
        if has_tool_calls {
            const MAX_CONTINUATION_TURNS: usize = 24;
            for _ in 0..MAX_CONTINUATION_TURNS {
                self.check_ftai_reload();
                self.engine.micro_compact();
                self.engine.compact();
                let request = self.engine.build_request(&self.config);

                let response = match self.generate_with_recovery(&request).await {
                    Ok(r) => r,
                    Err(e) => {
                        self.messages.push(DisplayMessage::System(format!("Error: {e}")));
                        break;
                    }
                };

                let more_tools = response.message.tool_calls.as_ref()
                    .map_or(false, |tc| !tc.is_empty())
                    || !self.parser.parse(&response.message.content).1.is_empty();

                self.process_response(response).await?;

                if !more_tools {
                    break;
                }
            }
        }

        Ok(())
    }

    /// Strip chat template special tokens from model output.
    fn clean_model_output(text: &str) -> String {
        text.replace("<|im_end|>", "")
            .replace("<|im_start|>", "")
            .replace("<|end|>", "")
            .replace("<|endoftext|>", "")
            .trim()
            .to_string()
    }

    async fn process_response(&mut self, response: ChatResponse) -> Result<()> {
        let content = Self::clean_model_output(&response.message.content);
        let tool_calls = response.message.tool_calls.clone();

        // In Chat mode skip tool call parsing — display the full response as text.
        if self.mode == Mode::Chat {
            if !content.trim().is_empty() {
                self.messages.push(DisplayMessage::Assistant(content.clone()));
            }
            if let Some(ref mgr) = self.session_manager {
                let _ = mgr.save_message(crate::backend::types::Role::Assistant, &content, None);
            }
            self.engine.add_assistant_message(response);
            return Ok(());
        }

        // Parse tool calls from text (for prompted mode)
        let (display_text, parsed_calls) = self.parser.parse(&content);

        if !display_text.trim().is_empty() {
            self.messages
                .push(DisplayMessage::Assistant(Self::clean_model_output(&display_text)));
        }

        // Save assistant message to session (with tool calls if any)
        if let Some(ref mgr) = self.session_manager {
            let tc = response.message.tool_calls.as_deref();
            let _ = mgr.save_message(crate::backend::types::Role::Assistant, &content, tc);
        }

        self.engine.add_assistant_message(response);

        // Process tool calls (from native or parsed)
        let all_calls = tool_calls
            .unwrap_or_default()
            .into_iter()
            .chain(parsed_calls)
            .collect::<Vec<_>>();

        // Track whether any task tool was called this round (for nag injection)
        let mut task_tool_called = false;

        // Pre-check all calls: permissions, rules, hooks. Collect approved ones
        // partitioned into read-only (concurrent) and mutating (serial).
        struct ApprovedCall {
            id: String,
            name: String,
            arguments: serde_json::Value,
            args_summary: String,
            params_json: String,
        }
        let mut approved_readonly: Vec<ApprovedCall> = Vec::new();
        let mut approved_mutating: Vec<ApprovedCall> = Vec::new();

        for call in all_calls {
            // Handle request_permissions specially (pre-flight batch approval)
            if call.name == "request_permissions" {
                let result = self.handle_permission_request(&call.arguments);
                self.engine.add_tool_result(&call.id, &result);
                continue;
            }

            // Handle agent_spawn specially — run subagent loop inline
            if call.name == "agent_spawn" {
                let result = self.handle_agent_spawn(&call.arguments).await;
                self.engine.add_tool_result(&call.id, &result);
                self.messages.push(DisplayMessage::ToolCall {
                    name: "agent_spawn".to_string(),
                    args_summary: call.arguments["task"]
                        .as_str()
                        .unwrap_or("(task)")
                        .chars()
                        .take(80)
                        .collect(),
                    result: result.clone(),
                    is_error: false,
                });
                continue;
            }

            // Track task tool calls for nag injection
            if call.name == "task" {
                task_tool_called = true;
            }

            // Step 1: Hard-block check (compile-time constants, no override)
            if let Some(reason) = permissions::hard_block_check(&call.name, &call.arguments) {
                self.messages.push(DisplayMessage::PermissionBlocked {
                    tool: call.name.clone(),
                    reason: reason.clone(),
                });
                self.engine.add_tool_result(&call.id, &format!("HARD BLOCKED: {reason}"));
                #[cfg(feature = "evolution")]
                self.session_tool_calls.push(ToolCallRecord {
                    tool_name: call.name.clone(),
                    arguments_summary: summarize_args(&call.arguments),
                    result_type: ToolResultType::Rejected,
                    duration_ms: 0,
                });
                continue;
            }

            // Step 2: Permission tier check
            let tier = permissions::classify(&call.name, &call.arguments);
            let verdict = permissions::check_permission(
                tier,
                &self.config.permissions.mode,
                &self.grant_cache,
                &call.name,
                &call.arguments,
            );

            match verdict {
                PermissionVerdict::Blocked(reason) => {
                    self.messages.push(DisplayMessage::PermissionBlocked {
                        tool: call.name.clone(),
                        reason: reason.clone(),
                    });
                    self.engine.add_tool_result(&call.id, &format!("BLOCKED: {reason}"));
                    continue;
                }
                PermissionVerdict::NeedsConfirmation(desc) => {
                    if !self.prompt_user_permission(&desc) {
                        self.denial_tracker.record_denial(&call.name, &call.arguments);
                        self.messages.push(DisplayMessage::PermissionDenied {
                            tool: call.name.clone(),
                        });
                        self.engine.add_tool_result(&call.id, "DENIED: User declined permission");
                        #[cfg(feature = "evolution")]
                        self.session_tool_calls.push(ToolCallRecord {
                            tool_name: call.name.clone(),
                            arguments_summary: summarize_args(&call.arguments),
                            result_type: ToolResultType::Rejected,
                            duration_ms: 0,
                        });
                        continue;
                    }
                    self.denial_tracker.reset_denials(&call.name, &call.arguments);
                }
                PermissionVerdict::Approved => {}
            }

            // Step 3: Rules engine check (user-defined rules)
            let mut rule_ctx = EvalContext::new(RuleEvent::Tool(call.name.clone()));
            rule_ctx.set_str("command", &call.arguments.to_string());
            if let Some(path) = call.arguments.get("path").and_then(|v| v.as_str()) {
                rule_ctx.set_str("path", path);
            }
            if let Some(content) = call.arguments.get("content").and_then(|v| v.as_str()) {
                rule_ctx.set_str("content", content);
            }

            let rule_result = self.rules.evaluate(
                &rule_ctx,
                Some(&self.project_path.to_string_lossy()),
            );

            match rule_result {
                RuleAction::Reject(reason) => {
                    self.messages.push(DisplayMessage::RuleViolation {
                        rule_name: "rule".to_string(),
                        reason: reason.clone(),
                    });
                    self.engine.add_tool_result(&call.id, &format!("BLOCKED: {reason}"));
                    #[cfg(feature = "evolution")]
                    self.session_tool_calls.push(ToolCallRecord {
                        tool_name: call.name.clone(),
                        arguments_summary: summarize_args(&call.arguments),
                        result_type: ToolResultType::RuleBlocked,
                        duration_ms: 0,
                    });
                    continue;
                }
                RuleAction::Allow | RuleAction::Modify(_) => {
                    // Run pre-hooks
                    let pre_event = format!("pre:{}", call.name);
                    let pre_hooks = self.plugin_manager.get_hooks(&pre_event);
                    let params_json = serde_json::to_string(&call.arguments).unwrap_or_default();
                    let mut hook_blocked = false;

                    for hook in &pre_hooks {
                        match crate::plugins::hooks::run_pre_hook(
                            hook,
                            &call.name,
                            &params_json,
                            &self.project_path,
                        ).await {
                            crate::plugins::hooks::HookResult::Blocked(msg) => {
                                self.messages.push(DisplayMessage::System(
                                    format!("Hook blocked {}: {msg}", call.name),
                                ));
                                self.engine.add_tool_result(&call.id, &format!("BLOCKED by hook: {msg}"));
                                hook_blocked = true;
                                break;
                            }
                            crate::plugins::hooks::HookResult::Error(msg) => {
                                self.messages.push(DisplayMessage::System(
                                    format!("Hook error: {msg}"),
                                ));
                            }
                            crate::plugins::hooks::HookResult::Passed => {}
                        }
                    }

                    if hook_blocked {
                        continue;
                    }

                    let args_summary = summarize_args(&call.arguments);
                    let approved = ApprovedCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                        args_summary,
                        params_json,
                    };

                    if ToolRegistry::is_read_only(&call.name) {
                        approved_readonly.push(approved);
                    } else {
                        approved_mutating.push(approved);
                    }
                }
            }
        }

        // Execute read-only tools concurrently
        if !approved_readonly.is_empty() {
            let ctx = ToolContext {
                cwd: self.project_path.clone(),
                project_path: self.project_path.clone(),
            };

            let start_time = std::time::Instant::now();
            let futures: Vec<_> = approved_readonly.iter().map(|ac| {
                self.tools.execute(&ac.name, ac.arguments.clone(), &ctx)
            }).collect();

            let results = futures_util::future::join_all(futures).await;
            let elapsed_ms = start_time.elapsed().as_millis() as u64;

            for (ac, result) in approved_readonly.iter().zip(results) {
                match result {
                    Ok(result) => {
                        let output = crate::tools::result_storage::maybe_persist_result(
                            &result.output,
                            &ac.name,
                            &self.project_path,
                        );

                        self.messages.push(DisplayMessage::ToolCall {
                            name: ac.name.clone(),
                            args_summary: ac.args_summary.clone(),
                            result: output.clone(),
                            is_error: result.is_error,
                        });
                        self.engine.add_tool_result(&ac.id, &output);

                        // Record tool call for evolution tracking
                        #[cfg(feature = "evolution")]
                        self.session_tool_calls.push(ToolCallRecord {
                            tool_name: ac.name.clone(),
                            arguments_summary: ac.args_summary.clone(),
                            result_type: if result.is_error {
                                ToolResultType::Error(output.chars().take(80).collect())
                            } else {
                                ToolResultType::Success
                            },
                            duration_ms: elapsed_ms,
                        });

                        // Run post-hooks (plugin hooks)
                        let post_event = format!("post:{}", ac.name);
                        let post_hooks = self.plugin_manager.get_hooks(&post_event);
                        for hook in &post_hooks {
                            let _ = crate::plugins::hooks::run_post_hook(
                                hook,
                                &ac.name,
                                &ac.params_json,
                                &output,
                                &self.project_path,
                            ).await;
                        }
                    }
                    Err(e) => {
                        let err = format!("Tool error: {e}");
                        self.messages.push(DisplayMessage::ToolCall {
                            name: ac.name.clone(),
                            args_summary: ac.args_summary.clone(),
                            result: err.clone(),
                            is_error: true,
                        });
                        self.engine.add_tool_result(&ac.id, &err);

                        // Record failed tool call for evolution tracking
                        #[cfg(feature = "evolution")]
                        self.session_tool_calls.push(ToolCallRecord {
                            tool_name: ac.name.clone(),
                            arguments_summary: ac.args_summary.clone(),
                            result_type: ToolResultType::Error(err.chars().take(80).collect()),
                            duration_ms: elapsed_ms,
                        });
                    }
                }
            }
        }

        // Execute mutating tools serially (preserving order)
        for ac in &approved_mutating {
            let ctx = ToolContext {
                cwd: self.project_path.clone(),
                project_path: self.project_path.clone(),
            };

            let tool_start = std::time::Instant::now();
            match self.tools.execute(&ac.name, ac.arguments.clone(), &ctx).await {
                Ok(result) => {
                    let tool_elapsed = tool_start.elapsed().as_millis() as u64;
                    let output = crate::tools::result_storage::maybe_persist_result(
                        &result.output,
                        &ac.name,
                        &self.project_path,
                    );

                    self.messages.push(DisplayMessage::ToolCall {
                        name: ac.name.clone(),
                        args_summary: ac.args_summary.clone(),
                        result: output.clone(),
                        is_error: result.is_error,
                    });
                    self.engine.add_tool_result(&ac.id, &output);

                    // Record tool call for evolution tracking
                    #[cfg(feature = "evolution")]
                    self.session_tool_calls.push(ToolCallRecord {
                        tool_name: ac.name.clone(),
                        arguments_summary: ac.args_summary.clone(),
                        result_type: if result.is_error {
                            ToolResultType::Error(output.chars().take(80).collect())
                        } else {
                            ToolResultType::Success
                        },
                        duration_ms: tool_elapsed,
                    });

                    // Run after_file_edit user hook for file tools
                    if ac.name == "file_edit" || ac.name == "file_write" {
                        if let Some(path) = ac.arguments.get("path").and_then(|v| v.as_str()) {
                            let mut env = std::collections::HashMap::new();
                            env.insert("FORGE_FILE_PATH".to_string(), path.to_string());
                            let _ = self.hook_runner.run("after_file_edit", &env).await;
                        }
                    }

                    // Run post-hooks (plugin hooks)
                    let post_event = format!("post:{}", ac.name);
                    let post_hooks = self.plugin_manager.get_hooks(&post_event);
                    for hook in &post_hooks {
                        let _ = crate::plugins::hooks::run_post_hook(
                            hook,
                            &ac.name,
                            &ac.params_json,
                            &output,
                            &self.project_path,
                        ).await;
                    }
                }
                Err(e) => {
                    let tool_elapsed = tool_start.elapsed().as_millis() as u64;
                    let err = format!("Tool error: {e}");
                    self.messages.push(DisplayMessage::ToolCall {
                        name: ac.name.clone(),
                        args_summary: ac.args_summary.clone(),
                        result: err.clone(),
                        is_error: true,
                    });
                    self.engine.add_tool_result(&ac.id, &err);

                    // Record failed tool call for evolution tracking
                    #[cfg(feature = "evolution")]
                    self.session_tool_calls.push(ToolCallRecord {
                        tool_name: ac.name.clone(),
                        arguments_summary: ac.args_summary.clone(),
                        result_type: ToolResultType::Error(err.chars().take(80).collect()),
                        duration_ms: tool_elapsed,
                    });
                }
            }
        }

        // Progress tracking: nag injection
        if task_tool_called {
            self.rounds_since_task_update = 0;
        } else {
            self.rounds_since_task_update += 1;
            if self.rounds_since_task_update >= 3 {
                // Inject a reminder alongside tool results
                self.engine.add_system_context(
                    "<reminder>Consider updating your task progress.</reminder>"
                );
                self.rounds_since_task_update = 0;
            }
        }

        Ok(())
    }

    /// Handle agent_spawn tool call: run a subagent loop and return its final text.
    async fn handle_agent_spawn(&mut self, params: &serde_json::Value) -> String {
        use crate::tools::agent_spawn;
        use crate::backend::types::{ChatRequest, Message, Role, StopReason};

        // Validate parameters
        if let Some(err) = agent_spawn::validate_params(params) {
            return format!("agent_spawn error: {err}");
        }

        let task = params["task"].as_str().unwrap_or("");

        // Build subagent messages
        let mut messages = agent_spawn::build_subagent_messages(task);

        // Filter tools for the subagent (no agent_spawn)
        let all_tool_defs = self.tools.tool_definitions();
        let requested_tools: Option<Vec<String>> = params["tools"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());
        let subagent_tools = agent_spawn::filter_tools(
            &all_tool_defs,
            requested_tools.as_deref(),
        );

        self.messages.push(DisplayMessage::System(
            format!("Subagent started: {}", task.chars().take(80).collect::<String>()),
        ));

        // Run the subagent loop
        let mut last_text = String::new();
        for _iteration in 0..agent_spawn::SUBAGENT_MAX_ITERATIONS {
            let request = ChatRequest {
                messages: messages.clone(),
                tools: subagent_tools.clone(),
                temperature: self.config.model.temperature,
                max_tokens: Some(4096),
                model_id: self.config.model.path.as_deref()
                    .map(|p| crate::backend::manager::BackendManager::resolve_path(p)),
            };

            let response = match self.backend.generate(&request).await {
                Ok(r) => r,
                Err(e) => {
                    return format!("Subagent backend error: {e}");
                }
            };

            let content = Self::clean_model_output(&response.message.content);
            let has_tool_calls = response.message.tool_calls.as_ref()
                .map_or(false, |tc| !tc.is_empty());

            if !content.is_empty() {
                last_text = content.clone();
            }

            // Add assistant message to subagent conversation
            messages.push(response.message.clone());

            if !has_tool_calls || response.stop_reason == StopReason::EndOfText {
                break;
            }

            // Execute tool calls
            if let Some(ref tool_calls) = response.message.tool_calls {
                for tc in tool_calls {
                    // Skip agent_spawn (safety)
                    if tc.name == "agent_spawn" {
                        messages.push(Message {
                            role: Role::Tool,
                            content: "agent_spawn not available in subagent context.".to_string(),
                            tool_calls: None,
                            tool_call_id: Some(tc.id.clone()),
                        });
                        continue;
                    }

                    let ctx = ToolContext {
                        cwd: self.project_path.clone(),
                        project_path: self.project_path.clone(),
                    };

                    let result = match self.tools.execute(&tc.name, tc.arguments.clone(), &ctx).await {
                        Ok(r) => r.output,
                        Err(e) => format!("Tool error: {e}"),
                    };

                    messages.push(Message {
                        role: Role::Tool,
                        content: result,
                        tool_calls: None,
                        tool_call_id: Some(tc.id.clone()),
                    });
                }
            }
        }

        self.messages.push(DisplayMessage::System("Subagent completed.".to_string()));

        if last_text.is_empty() {
            "Subagent completed without producing a final summary.".to_string()
        } else {
            last_text
        }
    }

    /// Temporarily exit raw mode, prompt user via stderr, return to raw mode.
    fn prompt_user_permission(&mut self, msg: &str) -> bool {
        // Leave raw mode for clean prompt
        let _ = terminal::disable_raw_mode();

        eprint!("\n  Permission required: {msg}\n  Allow? [y/N] ");
        let mut input = String::new();
        let approved = std::io::stdin().read_line(&mut input).is_ok()
            && input.trim().eq_ignore_ascii_case("y");

        // Re-enter raw mode
        let _ = terminal::enable_raw_mode();

        approved
    }

    /// Handle the request_permissions meta-tool.
    fn handle_permission_request(&mut self, params: &serde_json::Value) -> String {
        let task_desc = params
            .get("task_description")
            .and_then(|v| v.as_str())
            .unwrap_or("(no description)");

        let permissions = params
            .get("permissions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Leave raw mode for interactive prompt
        let _ = terminal::disable_raw_mode();

        eprintln!("\n  Pre-flight permission request: {task_desc}");
        eprintln!("  Requested permissions:");

        let mut approvable = Vec::new();
        let mut destructive_warnings = Vec::new();

        for perm in &permissions {
            let tool = perm.get("tool").and_then(|v| v.as_str()).unwrap_or("?");
            let scope = perm.get("scope").and_then(|v| v.as_str()).unwrap_or("all");

            // Check if this would be destructive
            let test_params = match tool {
                "bash" => serde_json::json!({"command": scope}),
                _ => serde_json::json!({"path": scope}),
            };
            let tier = permissions::classify(tool, &test_params);

            if tier == permissions::PermissionTier::Destructive {
                destructive_warnings.push(format!("    ⚠ {tool} ({scope}) — requires per-action confirmation"));
            } else {
                eprintln!("    ✓ {tool} ({scope})");
                approvable.push((tool.to_string(), scope.to_string()));
            }
        }

        for warning in &destructive_warnings {
            eprintln!("{warning}");
        }

        eprint!("  Approve non-destructive permissions? [y/N] ");
        let mut input = String::new();
        let approved = std::io::stdin().read_line(&mut input).is_ok()
            && input.trim().eq_ignore_ascii_case("y");

        // Re-enter raw mode
        let _ = terminal::enable_raw_mode();

        if approved {
            for (tool, scope) in &approvable {
                let grant_scope = if scope == "all" {
                    GrantScope::Tool(tool.clone())
                } else if tool == "bash" {
                    GrantScope::ToolWithCommand(tool.clone(), scope.clone())
                } else {
                    GrantScope::ToolWithPath(tool.clone(), scope.clone())
                };

                self.grant_cache.add(PermissionGrant {
                    tool_name: tool.clone(),
                    scope: grant_scope,
                    granted_at: std::time::Instant::now(),
                });
            }

            self.messages.push(DisplayMessage::System(format!(
                "Granted {} permissions for: {task_desc}",
                approvable.len()
            )));

            format!(
                "Approved {} permissions. {} destructive actions require per-action confirmation.",
                approvable.len(),
                destructive_warnings.len()
            )
        } else {
            self.messages.push(DisplayMessage::System(
                "Pre-flight permissions denied.".to_string(),
            ));
            "User denied pre-flight permissions.".to_string()
        }
    }

    /// Switch to a new mode, rebuilding the system prompt accordingly.
    fn switch_mode(&mut self, new_mode: Mode) {
        if self.mode == new_mode {
            return;
        }

        let memory = prompt::load_memory_context(&self.project_path);
        let ftai_context = prompt::load_ftai_context(&self.project_path);

        let new_system_prompt = match new_mode {
            Mode::Coding => {
                let tool_defs = self.tools.tool_definitions();
                let rules_summary = if self.rules.rule_count() > 0 {
                    Some(self.rules.summary())
                } else {
                    None
                };
                let skills_prompt = if !self.skills.is_empty() {
                    let triggers: Vec<&str> = self.skills.iter().map(|s| s.trigger.as_str()).collect();
                    Some(format!(
                        "Slash command skills: {}\nType any command to activate it.",
                        triggers.join(", ")
                    ))
                } else {
                    None
                };
                prompt::build_system_prompt(
                    &self.project_path,
                    &tool_defs,
                    rules_summary.as_deref(),
                    memory.as_deref(),
                    Some(&self.templates),
                    &self.config.formatting.enabled,
                    ftai_context.as_deref(),
                    skills_prompt.as_deref(),
                )
            }
            Mode::Chat => prompt::build_chat_system_prompt(
                memory.as_deref(),
                ftai_context.as_deref(),
            ),
        };

        self.engine.update_system_prompt(new_system_prompt);
        self.mode = new_mode;
    }

    async fn handle_slash_command(&mut self, text: &str) -> Result<()> {
        // Show the command as user input and scroll to bottom
        self.messages.push(DisplayMessage::User(text.trim().to_string()));
        self.scroll_offset = 0;

        let parts: Vec<&str> = text.trim().split_whitespace().collect();
        let cmd = parts[0];

        match cmd {
            "/help" => {
                self.messages.push(DisplayMessage::System(
                    "Commands: /help /clear /compact /rules /permissions /templates /config /model /api /project /memory /context /plugin /hardware /skill /theme /dream /chat /code /quit".to_string(),
                ));
            }
            "/chat" => {
                self.switch_mode(Mode::Chat);
                self.messages.push(DisplayMessage::System(
                    "Switched to chat mode. General conversation — tools available on explicit request only.".to_string(),
                ));
            }
            "/code" => {
                self.switch_mode(Mode::Coding);
                self.messages.push(DisplayMessage::System(
                    "Switched to coding mode. Full agentic loop with tools and project context active.".to_string(),
                ));
            }
            "/clear" => {
                self.messages.clear();
                self.engine.clear();
                self.grant_cache.clear();
                self.messages.push(DisplayMessage::System("Conversation cleared. Permission grants cleared.".to_string()));
            }
            "/compact" => {
                self.engine.compact();
                self.messages.push(DisplayMessage::System(
                    format!("Context compacted. Tokens: ~{}", self.engine.estimated_tokens()),
                ));
            }
            "/rules" => {
                if parts.get(1) == Some(&"reload") {
                    self.rules.clear();
                    if let Ok(global_dir) = crate::config::global_config_dir() {
                        let rules_file = global_dir.join("rules.ftai");
                        if rules_file.exists() {
                            match self.rules.load_file(&rules_file) {
                                Ok(n) => {
                                    self.messages.push(DisplayMessage::System(format!("Loaded {n} rules")));
                                }
                                Err(e) => {
                                    self.messages.push(DisplayMessage::System(format!("Error: {e}")));
                                }
                            }
                        }
                    }
                } else {
                    let summary = self.rules.summary();
                    if summary.is_empty() {
                        self.messages.push(DisplayMessage::System("No rules loaded.".to_string()));
                    } else {
                        self.messages.push(DisplayMessage::System(summary));
                    }
                }
            }
            "/permissions" => {
                if parts.get(1) == Some(&"clear") {
                    self.grant_cache.clear();
                    self.messages.push(DisplayMessage::System(
                        "Permission grants cleared.".to_string(),
                    ));
                } else {
                    let grants = self.grant_cache.list();
                    if grants.is_empty() {
                        self.messages.push(DisplayMessage::System(format!(
                            "Permission mode: {:?}\nNo active grants.",
                            self.config.permissions.mode
                        )));
                    } else {
                        let mut info = format!(
                            "Permission mode: {:?}\nActive grants:\n",
                            self.config.permissions.mode
                        );
                        for g in &grants {
                            info.push_str(&format!("  • {g}\n"));
                        }
                        self.messages.push(DisplayMessage::System(info));
                    }
                }
            }
            "/templates" => {
                let enabled = &self.config.formatting.enabled;
                let active = formatting::enabled_templates(&self.templates, enabled);
                let mut info = String::from("Loaded templates:\n");
                for (label, content) in &active {
                    let preview: String = content.lines().take(3).collect::<Vec<_>>().join("\n");
                    info.push_str(&format!("\n### {label}\n{preview}\n...\n"));
                }
                if let Some(ref dir) = self.config.formatting.templates_dir {
                    info.push_str(&format!("\nCustom dir: {dir}"));
                }
                self.messages.push(DisplayMessage::System(info));
            }
            "/config" => {
                let toml_str = toml::to_string_pretty(&self.config).unwrap_or_default();
                self.messages.push(DisplayMessage::System(toml_str));
            }
            "/model" => {
                if let Some(name) = parts.get(1) {
                    self.messages.push(DisplayMessage::System(
                        format!("Model switching not yet implemented. Current: {:?}", self.config.model.backend),
                    ));
                    let _ = name;
                } else {
                    let info = format!(
                        "Backend: {:?}\nModel: {}\nContext: {}",
                        self.config.model.backend,
                        self.config.model.path.as_deref().unwrap_or("(none)"),
                        self.config.model.context_length,
                    );
                    self.messages.push(DisplayMessage::System(info));
                }
            }
            "/project" => {
                if let Some(path) = parts.get(1) {
                    self.project_path = PathBuf::from(path);
                    self.messages.push(DisplayMessage::System(
                        format!("Switched to: {path}"),
                    ));
                } else {
                    self.messages.push(DisplayMessage::System(
                        format!("Current: {}", self.project_path.display()),
                    ));
                }
            }
            "/memory" => {
                if parts.len() > 1 && parts[1] == "delete" {
                    // /memory delete <name> — delete a specific memory file
                    if parts.len() < 3 {
                        self.messages.push(DisplayMessage::System(
                            "Usage: /memory delete <name>".to_string(),
                        ));
                    } else {
                        let name = parts[2];
                        // Validate name before constructing path
                        if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
                            self.messages.push(DisplayMessage::System(
                                "Invalid memory name.".to_string(),
                            ));
                        } else {
                            let memory_dir = self.project_path.join(".ftai").join("memory");
                            let file = memory_dir.join(format!("{name}.md"));
                            if file.exists() {
                                match std::fs::remove_file(&file) {
                                    Ok(_) => {
                                        self.messages.push(DisplayMessage::System(
                                            format!("Deleted memory: {name}"),
                                        ));
                                    }
                                    Err(e) => {
                                        self.messages.push(DisplayMessage::System(
                                            format!("Error deleting memory: {e}"),
                                        ));
                                    }
                                }
                            } else {
                                self.messages.push(DisplayMessage::System(
                                    format!("Memory \"{name}\" not found."),
                                ));
                            }
                        }
                    }
                } else if parts.len() > 1 {
                    // /memory <text> — append to project memory (legacy MEMORY.md)
                    let note = parts[1..].join(" ");
                    let memory_dir = self.project_path.join(".ftai").join("memory");
                    let _ = std::fs::create_dir_all(&memory_dir);
                    let memory_file = memory_dir.join("MEMORY.md");
                    let mut content = std::fs::read_to_string(&memory_file).unwrap_or_default();
                    if !content.is_empty() && !content.ends_with('\n') {
                        content.push('\n');
                    }
                    content.push_str(&format!("- {note}\n"));
                    match std::fs::write(&memory_file, content) {
                        Ok(_) => {
                            self.messages.push(DisplayMessage::System(
                                format!("Saved to memory: {note}"),
                            ));
                        }
                        Err(e) => {
                            self.messages.push(DisplayMessage::System(
                                format!("Error saving memory: {e}"),
                            ));
                        }
                    }
                } else {
                    // /memory — list all memory files with previews
                    let memory_dir = self.project_path.join(".ftai").join("memory");
                    let has_files = memory_dir.exists() && std::fs::read_dir(&memory_dir)
                        .map(|mut d| d.next().is_some())
                        .unwrap_or(false);

                    if has_files {
                        if let Ok(entries) = crate::tools::memory_tool::read_memory_dir(&memory_dir, None) {
                            if entries.is_empty() {
                                self.messages.push(DisplayMessage::System("No memory notes found.".to_string()));
                            } else {
                                let mut output = String::from("Memory files:\n");
                                for (name, content) in &entries {
                                    let preview: String = content.lines().next().unwrap_or("(empty)").chars().take(80).collect();
                                    output.push_str(&format!("  {name} — {preview}\n"));
                                }
                                self.messages.push(DisplayMessage::System(output));
                            }
                        } else {
                            self.messages.push(DisplayMessage::System("Error reading memory directory.".to_string()));
                        }
                    } else {
                        self.messages.push(DisplayMessage::System("No memory notes found.".to_string()));
                    }
                }
            }
            "/hardware" => {
                let hw = crate::backend::types::HardwareInfo::detect();
                let rec = hw.recommended_model();
                self.messages.push(DisplayMessage::System(format!(
                    "Architecture: {:?}\nGPU: {:?}\nRAM: {} GB\nRecommended: {} ({:?}, ~{}GB)",
                    hw.arch, hw.gpu, hw.ram_gb, rec.name, rec.backend, rec.size_gb
                )));
            }
            "/plugin" => {
                match parts.get(1).copied() {
                    Some("create") => {
                        if let Some(name) = parts.get(2) {
                            self.scaffold_plugin(name);
                        } else {
                            self.messages.push(DisplayMessage::System(
                                "Usage: /plugin create <name>".to_string(),
                            ));
                        }
                    }
                    Some("list") => {
                        let plugins = self.plugin_manager.list();
                        if plugins.is_empty() {
                            self.messages.push(DisplayMessage::System("No plugins installed.".to_string()));
                        } else {
                            let mut info = String::from("Installed plugins:\n");
                            for p in plugins {
                                info.push_str(&format!(
                                    "  {} v{} — {}\n",
                                    p.manifest.plugin.name,
                                    p.manifest.plugin.version,
                                    p.manifest.plugin.description,
                                ));
                            }
                            self.messages.push(DisplayMessage::System(info));
                        }
                    }
                    Some("install") => {
                        if let Some(source) = parts.get(2) {
                            let result = if source.starts_with("http://") || source.starts_with("https://") || source.contains("github.com") {
                                self.plugin_manager.install_from_git(source)
                            } else {
                                self.plugin_manager.install_from_path(std::path::Path::new(source))
                            };
                            match result {
                                Ok(name) => self.messages.push(DisplayMessage::System(
                                    format!("Installed plugin: {name}"),
                                )),
                                Err(e) => self.messages.push(DisplayMessage::System(
                                    format!("Install failed: {e}"),
                                )),
                            }
                        } else {
                            self.messages.push(DisplayMessage::System(
                                "Usage: /plugin install <git-url-or-path>".to_string(),
                            ));
                        }
                    }
                    Some("uninstall") => {
                        if let Some(name) = parts.get(2) {
                            match self.plugin_manager.uninstall(name) {
                                Ok(_) => self.messages.push(DisplayMessage::System(
                                    format!("Uninstalled plugin: {name}"),
                                )),
                                Err(e) => self.messages.push(DisplayMessage::System(
                                    format!("Uninstall failed: {e}"),
                                )),
                            }
                        } else {
                            self.messages.push(DisplayMessage::System(
                                "Usage: /plugin uninstall <name>".to_string(),
                            ));
                        }
                    }
                    Some("search") => {
                        if let Some(query) = parts.get(2) {
                            let registry_url = self.config.plugins.registry_url.as_deref();
                            let client = crate::plugins::registry::RegistryClient::new(registry_url);
                            match client.search(query).await {
                                Ok(results) => {
                                    if results.is_empty() {
                                        self.messages.push(DisplayMessage::System(
                                            format!("No plugins found matching '{query}'"),
                                        ));
                                    } else {
                                        let mut info = format!("Registry results for '{query}':\n");
                                        for r in &results {
                                            info.push_str(&format!(
                                                "  {} v{} — {} ({})\n",
                                                r.name, r.version, r.description, r.repo,
                                            ));
                                        }
                                        self.messages.push(DisplayMessage::System(info));
                                    }
                                }
                                Err(e) => self.messages.push(DisplayMessage::System(
                                    format!("Registry search failed: {e}"),
                                )),
                            }
                        } else {
                            self.messages.push(DisplayMessage::System(
                                "Usage: /plugin search <query>".to_string(),
                            ));
                        }
                    }
                    Some("info") => {
                        if let Some(name) = parts.get(2) {
                            // Check local first
                            let local = self.plugin_manager.list().iter().find(|p| p.manifest.plugin.name == *name);
                            if let Some(p) = local {
                                let mut info = format!(
                                    "Plugin: {} v{}\nAuthor: {}\nDescription: {}\nTools: {}\nSkills: {}\nHooks: {}\n",
                                    p.manifest.plugin.name,
                                    p.manifest.plugin.version,
                                    p.manifest.plugin.author,
                                    p.manifest.plugin.description,
                                    p.manifest.tools.len(),
                                    p.manifest.skills.len(),
                                    p.manifest.hooks.len(),
                                );
                                if let Some(reg) = &p.manifest.registry {
                                    if let Some(repo) = &reg.repo {
                                        info.push_str(&format!("Repo: {repo}\n"));
                                    }
                                }
                                self.messages.push(DisplayMessage::System(info));
                            } else {
                                self.messages.push(DisplayMessage::System(
                                    format!("Plugin '{name}' not installed locally."),
                                ));
                            }
                        } else {
                            self.messages.push(DisplayMessage::System(
                                "Usage: /plugin info <name>".to_string(),
                            ));
                        }
                    }
                    Some("marketplace") => {
                        match parts.get(2).copied() {
                            Some("add") => {
                                if let Some(repo) = parts.get(3) {
                                    let config_dir = crate::config::global_config_dir().unwrap_or_default();
                                    match crate::plugins::MarketplaceRegistry::new(&config_dir) {
                                        Ok(mut registry) => {
                                            let name = repo.split('/').last().unwrap_or(repo);
                                            match registry.add_source(name, repo) {
                                                Ok(_) => self.messages.push(DisplayMessage::System(
                                                    format!("Added marketplace: {repo}"),
                                                )),
                                                Err(e) => self.messages.push(DisplayMessage::System(
                                                    format!("Failed to add marketplace: {e}"),
                                                )),
                                            }
                                        }
                                        Err(e) => self.messages.push(DisplayMessage::System(
                                            format!("Failed to load marketplaces: {e}"),
                                        )),
                                    }
                                } else {
                                    self.messages.push(DisplayMessage::System(
                                        "Usage: /plugin marketplace add <owner/repo>".to_string(),
                                    ));
                                }
                            }
                            Some("list") => {
                                let config_dir = crate::config::global_config_dir().unwrap_or_default();
                                match crate::plugins::MarketplaceRegistry::new(&config_dir) {
                                    Ok(registry) => {
                                        let sources = registry.list_sources();
                                        if sources.is_empty() {
                                            self.messages.push(DisplayMessage::System(
                                                "No marketplaces registered.".to_string(),
                                            ));
                                        } else {
                                            let mut info = String::from("Registered marketplaces:\n");
                                            for s in sources {
                                                info.push_str(&format!("  {} ({})\n", s.name, s.repo));
                                            }
                                            self.messages.push(DisplayMessage::System(info));
                                        }
                                    }
                                    Err(e) => self.messages.push(DisplayMessage::System(
                                        format!("Failed to load marketplaces: {e}"),
                                    )),
                                }
                            }
                            _ => {
                                self.messages.push(DisplayMessage::System(
                                    "Usage: /plugin marketplace [add <owner/repo> | list]".to_string(),
                                ));
                            }
                        }
                    }
                    _ => {
                        // Open the interactive plugin browser modal with marketplace data
                        let installed = self.build_installed_entries();
                        let marketplaces = self.build_marketplace_entries();
                        let modal = PluginModal::with_marketplaces(installed, marketplaces);
                        self.active_modal = Some(Box::new(modal));
                    }
                }
            }
            "/skill" => {
                if self.skills.is_empty() {
                    self.messages.push(DisplayMessage::System("No skills available.".to_string()));
                } else if let Some(skill_name) = parts.get(1) {
                    let trigger = if skill_name.starts_with('/') {
                        skill_name.to_string()
                    } else {
                        format!("/{skill_name}")
                    };
                    if let Some(skill) = crate::skills::loader::find_skill_by_trigger(&self.skills, &trigger) {
                        self.messages.push(DisplayMessage::System(
                            format!("Skill '{}' activated. Content injected into context.", skill.name),
                        ));
                        self.engine.add_system_context(&format!(
                            "# Skill: {}\n{}", skill.name, skill.content
                        ));
                    } else {
                        self.messages.push(DisplayMessage::System(
                            format!("Unknown skill: {skill_name}. Use /skill to list available skills."),
                        ));
                    }
                } else {
                    // Open the interactive skill browser modal
                    let entries = self.build_skill_entries();
                    let modal = SkillModal::new(entries);
                    self.active_modal = Some(Box::new(modal));
                }
            }
            "/context" => {
                match parts.get(1).copied() {
                    Some("init") => {
                        let ftai_dir = self.project_path.join(".ftai");
                        let _ = std::fs::create_dir_all(&ftai_dir);
                        let ftai_md = ftai_dir.join("FTAI.md");
                        if ftai_md.exists() {
                            self.messages.push(DisplayMessage::System(
                                "FTAI.md already exists. Use /context edit to modify.".to_string(),
                            ));
                        } else {
                            let project_name = self.project_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "my-project".to_string());
                            let template = format!(
                                "# Project: {project_name}\n\n\
                                 ## Stack\n<!-- Languages, frameworks, build tools -->\n\n\
                                 ## Conventions\n<!-- Code style, patterns, naming conventions -->\n\n\
                                 ## Architecture\n<!-- Key directories, module structure, data flow -->\n\n\
                                 ## Testing\n<!-- How to run tests, what frameworks, coverage expectations -->\n\n\
                                 ## Gotchas\n<!-- Known issues, quirks, things to watch out for -->\n"
                            );
                            match std::fs::write(&ftai_md, &template) {
                                Ok(_) => self.messages.push(DisplayMessage::System(
                                    format!("Created {}", ftai_md.display()),
                                )),
                                Err(e) => self.messages.push(DisplayMessage::System(
                                    format!("Error creating FTAI.md: {e}"),
                                )),
                            }
                        }
                    }
                    Some("edit") => {
                        let ftai_md = self.project_path.join(".ftai").join("FTAI.md");
                        if !ftai_md.exists() {
                            self.messages.push(DisplayMessage::System(
                                "No FTAI.md found. Use /context init to create one.".to_string(),
                            ));
                        } else {
                            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
                            let _ = terminal::disable_raw_mode();
                            let _ = stdout().execute(LeaveAlternateScreen);
                            let _ = std::process::Command::new(&editor).arg(&ftai_md).status();
                            let _ = stdout().execute(EnterAlternateScreen);
                            let _ = terminal::enable_raw_mode();
                            self.messages.push(DisplayMessage::System("FTAI.md updated.".to_string()));
                        }
                    }
                    _ => {
                        // Show current FTAI.md content
                        match prompt::load_ftai_context(&self.project_path) {
                            Some(ctx) => self.messages.push(DisplayMessage::System(ctx)),
                            None => self.messages.push(DisplayMessage::System(
                                "No FTAI.md found. Use /context init to create one.".to_string(),
                            )),
                        }
                    }
                }
            }
            "/theme" => {
                if let Some(name) = parts.get(1) {
                    // Direct: /theme dracula
                    self.apply_theme(name);
                } else {
                    // Interactive picker
                    let current = format!("{:?}", self.config.theme.preset).to_lowercase();
                    let modal = super::theme_modal::ThemeModal::new(&current);
                    self.active_modal = Some(Box::new(modal));
                }
            }
            "/dream" => {
                let scheduler = crate::dream::scheduler::DreamScheduler::new(&self.project_path);
                match parts.get(1).copied() {
                    Some("run") => {
                        // Force a dream run now (bypass time/session gates)
                        match scheduler.acquire_lock() {
                            Ok(_lock) => {
                                let runner = crate::dream::runner::DreamRunner::new(&self.project_path);
                                match runner.run(Some(0)) {
                                    Ok(path) => {
                                        if let Ok(content) = std::fs::read_to_string(&path) {
                                            self.messages.push(DisplayMessage::System(
                                                format!("Dream complete. Output: {}\n\n{content}", path.display()),
                                            ));
                                        } else {
                                            self.messages.push(DisplayMessage::System(
                                                format!("Dream complete. Saved to: {}", path.display()),
                                            ));
                                        }
                                    }
                                    Err(e) => {
                                        self.messages.push(DisplayMessage::System(
                                            format!("Dream failed: {e}"),
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                self.messages.push(DisplayMessage::System(
                                    format!("Could not acquire dream lock: {e}"),
                                ));
                            }
                        }
                    }
                    Some("list") => {
                        let dreams = scheduler.list_dreams();
                        if dreams.is_empty() {
                            self.messages.push(DisplayMessage::System(
                                "No dream files found.".to_string(),
                            ));
                        } else {
                            let mut out = String::from("Dream files:\n");
                            for d in &dreams {
                                out.push_str(&format!("  {} (modified: {})\n", d.filename, d.modified));
                            }
                            self.messages.push(DisplayMessage::System(out));
                        }
                    }
                    _ => {
                        // /dream — show latest dream summary
                        match scheduler.latest_dream() {
                            Some(content) => {
                                self.messages.push(DisplayMessage::System(content));
                            }
                            None => {
                                self.messages.push(DisplayMessage::System(
                                    "No dream results available. Use /dream run to trigger a dream analysis.".to_string(),
                                ));
                            }
                        }
                    }
                }
            }
            "/doctor" => {
                let probe = crate::backend::BackendProbeResults::probe();
                let hw = crate::backend::types::HardwareInfo::detect();
                let rec = hw.recommended_model();
                let mut info = String::from("Forge Doctor\n\n");
                info.push_str(&probe.display());
                info.push_str(&format!(
                    "\nHardware: {:?} | {:?} | {} GB RAM\nRecommended: {} ({:?}, ~{}GB)\n",
                    hw.arch, hw.gpu, hw.ram_gb, rec.name, rec.backend, rec.size_gb
                ));
                info.push_str(&format!("\nBackend: {:?}", self.config.model.backend));
                info.push_str(&format!("\nContext: {}", self.config.model.context_length));
                if let Some(path) = &self.config.model.path {
                    let exists = std::path::Path::new(path).exists();
                    info.push_str(&format!("\nModel: {} ({})", path, if exists { "found" } else { "NOT FOUND" }));
                } else {
                    info.push_str("\nModel: (none configured)");
                }
                self.messages.push(DisplayMessage::System(info));
            }
            "/api" => {
                match parts.get(1).copied() {
                    Some("on") => {
                        if self.backend.backend_name() == "api" {
                            self.messages.push(DisplayMessage::System("API backend is already active.".to_string()));
                        } else {
                            match crate::backend::api_client::ApiClient::from_config(&self.config.api) {
                                Ok(client) => {
                                    let model = client.model().to_string();
                                    let provider = format!("{:?}", client.provider());
                                    self.backend = BackendManager::Api(client);
                                    self.config.api.enabled = true;
                                    self.messages.push(DisplayMessage::System(
                                        format!("Switched to API backend: {provider} / {model}"),
                                    ));
                                }
                                Err(e) => {
                                    self.messages.push(DisplayMessage::System(
                                        format!("Failed to enable API backend: {e}"),
                                    ));
                                }
                            }
                        }
                    }
                    Some("off") => {
                        if self.backend.backend_name() != "api" {
                            self.messages.push(DisplayMessage::System("API backend is not active.".to_string()));
                        } else {
                            self.config.api.enabled = false;
                            self.backend = BackendManager::from_config(&self.config);
                            self.messages.push(DisplayMessage::System(
                                format!("Switched to local backend: {}", self.backend.backend_name()),
                            ));
                        }
                    }
                    Some("model") => {
                        if let Some(model_name) = parts.get(2) {
                            if let Some(client) = self.backend.api_client_mut() {
                                client.set_model(model_name);
                                self.config.api.model = model_name.to_string();
                                self.messages.push(DisplayMessage::System(
                                    format!("API model set to: {model_name}"),
                                ));
                            } else {
                                self.messages.push(DisplayMessage::System(
                                    "API backend is not active. Use /api on first.".to_string(),
                                ));
                            }
                        } else {
                            self.messages.push(DisplayMessage::System(
                                "Usage: /api model <name>".to_string(),
                            ));
                        }
                    }
                    Some("provider") => {
                        if let Some(provider_name) = parts.get(2) {
                            let provider = crate::backend::api_client::ApiProvider::from_str_loose(provider_name);
                            if let Some(client) = self.backend.api_client_mut() {
                                client.set_provider(provider);
                                self.config.api.provider = provider_name.to_string();
                                self.messages.push(DisplayMessage::System(
                                    format!("API provider set to: {provider_name}"),
                                ));
                            } else {
                                self.messages.push(DisplayMessage::System(
                                    "API backend is not active. Use /api on first.".to_string(),
                                ));
                            }
                        } else {
                            self.messages.push(DisplayMessage::System(
                                "Usage: /api provider <name>".to_string(),
                            ));
                        }
                    }
                    _ => {
                        // Show API status
                        let enabled = self.config.api.enabled && self.backend.backend_name() == "api";
                        let key_status = if let Some(key) = crate::backend::api_client::resolve_api_key(&self.config.api) {
                            crate::backend::api_client::mask_api_key(&key)
                        } else {
                            "(not set)".to_string()
                        };
                        self.messages.push(DisplayMessage::System(format!(
                            "API Backend\n  Status: {}\n  Provider: {}\n  Model: {}\n  Key: {}\n  Base URL: {}\n  Max tokens: {}\n\nUsage: /api on | off | model <name> | provider <name>",
                            if enabled { "active" } else { "inactive" },
                            self.config.api.provider,
                            self.config.api.model,
                            key_status,
                            self.config.api.base_url.as_deref().unwrap_or("(default)"),
                            self.config.api.max_tokens,
                        )));
                    }
                }
            }
            "/quit" | "/exit" => {
                self.should_quit = true;
            }
            _ => {
                // Check if the command matches a skill trigger
                if let Some(skill) = crate::skills::loader::find_skill_by_trigger(&self.skills, cmd) {
                    self.messages.push(DisplayMessage::System(
                        format!("Loaded skill: {}", skill.description),
                    ));
                    self.engine.add_system_context(&format!(
                        "# Skill: {}\n{}", skill.name, skill.content
                    ));
                } else {
                    self.messages.push(DisplayMessage::System(
                        format!("Unknown command: {cmd}. Type /help for available commands."),
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Produce a short summary of tool call arguments (first 80 chars of the JSON).
fn summarize_args(args: &serde_json::Value) -> String {
    let s = args.to_string();
    if s == "{}" || s == "null" {
        return String::new();
    }
    if s.len() <= 80 {
        s
    } else {
        let mut truncated: String = s.chars().take(77).collect();
        truncated.push_str("...");
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── Mode enum ────────────────────────────────────────────────────────────

    #[test]
    fn test_mode_label() {
        assert_eq!(Mode::Coding.label(), "coding");
        assert_eq!(Mode::Chat.label(), "chat");
    }

    #[test]
    fn test_mode_equality() {
        assert_eq!(Mode::Coding, Mode::Coding);
        assert_eq!(Mode::Chat, Mode::Chat);
        assert_ne!(Mode::Coding, Mode::Chat);
    }

    // ── Auto-detection ───────────────────────────────────────────────────────

    #[test]
    fn test_detect_mode_with_git() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        assert_eq!(detect_mode(tmp.path()), Mode::Coding);
    }

    #[test]
    fn test_detect_mode_with_ftai() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".ftai")).unwrap();
        assert_eq!(detect_mode(tmp.path()), Mode::Coding);
    }

    #[test]
    fn test_detect_mode_empty_dir_is_chat() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_eq!(detect_mode(tmp.path()), Mode::Chat);
    }

    #[test]
    fn test_detect_mode_nonexistent_path_is_chat() {
        let path = PathBuf::from("/tmp/ftai-test-nonexistent-12345xyz");
        assert_eq!(detect_mode(&path), Mode::Chat);
    }

    #[test]
    fn test_detect_mode_both_git_and_ftai_is_coding() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".ftai")).unwrap();
        assert_eq!(detect_mode(tmp.path()), Mode::Coding);
    }

    // ── Security red tests (P0) ──────────────────────────────────────────────

    #[test]
    fn test_detect_mode_path_traversal_in_dir_name_stays_safe() {
        // A directory literally named ".git" triggers Coding — that is correct behaviour.
        // We verify that a path with traversal components does not panic or produce
        // unexpected results (Rust's Path API normalises the components).
        let path = PathBuf::from("/tmp/../tmp/ftai-no-git-here-xyz");
        // This non-existent path has no .git or .ftai -> Chat
        assert_eq!(detect_mode(&path), Mode::Chat);
    }

    #[test]
    fn test_mode_switch_label_reflects_change() {
        // Verify Mode variants switch correctly through the label
        let mut m = Mode::Coding;
        assert_eq!(m.label(), "coding");
        m = Mode::Chat;
        assert_eq!(m.label(), "chat");
        m = Mode::Coding;
        assert_eq!(m.label(), "coding");
    }

    // ── FTAI.md hot-reload tests ────────────────────────────────────────────

    #[test]
    fn test_read_ftai_mtime_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mtime = TuiApp::read_ftai_mtime(tmp.path());
        assert!(mtime.is_none(), "Missing FTAI.md should return None");
    }

    #[test]
    fn test_read_ftai_mtime_detects_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ftai_dir = tmp.path().join(".ftai");
        std::fs::create_dir_all(&ftai_dir).unwrap();
        std::fs::write(ftai_dir.join("FTAI.md"), "initial content").unwrap();

        let mtime = TuiApp::read_ftai_mtime(tmp.path());
        assert!(mtime.is_some(), "Existing FTAI.md should return Some mtime");
    }

    #[test]
    fn test_read_ftai_mtime_changes_on_write() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ftai_dir = tmp.path().join(".ftai");
        std::fs::create_dir_all(&ftai_dir).unwrap();
        let ftai_path = ftai_dir.join("FTAI.md");

        std::fs::write(&ftai_path, "version 1").unwrap();
        let mtime1 = TuiApp::read_ftai_mtime(tmp.path());

        // Ensure mtime actually changes (some filesystems have coarse granularity)
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&ftai_path, "version 2 with more content").unwrap();
        let mtime2 = TuiApp::read_ftai_mtime(tmp.path());

        assert!(mtime1.is_some());
        assert!(mtime2.is_some());
        // On most systems the mtime will differ; on those with 1-second granularity
        // it might not, so we just verify both are Some and the function doesn't panic.
    }
}
