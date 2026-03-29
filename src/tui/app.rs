use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use std::io::{self, stdout};
use std::path::PathBuf;
use crate::backend::manager::BackendManager;
use crate::backend::types::ChatResponse;
use crate::config::Config;
use crate::conversation::engine::ConversationEngine;
use crate::conversation::parser::ToolCallParser;
use crate::conversation::prompt;
use crate::formatting::{self, TemplateSet};
use crate::permissions::{self, GrantCache, GrantScope, PermissionGrant, PermissionVerdict};
use crate::rules::{EvalContext, RuleAction, RulesEngine};
use crate::rules::parser::Event as RuleEvent;
use crate::plugins::PluginManager;
use crate::skills::LoadedSkill;
use crate::tools::{ToolContext, ToolRegistry};

use super::input::InputState;
use super::render::{self, DisplayMessage};

/// Error type for interruptible backend startup.
enum StartErr {
    UserQuit,
    BackendFailed(String),
}

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
}

impl TuiApp {
    pub fn new(config: Config, project_path: PathBuf) -> Self {
        let mut tools = ToolRegistry::with_defaults();

        // Load plugins
        let plugins_dir = crate::config::global_config_dir()
            .map(|d| d.join("plugins"))
            .unwrap_or_else(|_| PathBuf::from("~/.ftai/plugins"));

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
            let mut out = String::from("Available skills via slash commands:\n\n");
            for skill in &skills {
                out.push_str(&format!(
                    "- `{}` ({}) — {}\n",
                    skill.trigger, skill.source, skill.description
                ));
            }
            Some(out)
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

        let engine = ConversationEngine::new(
            system_prompt,
            tool_defs,
            config.model.context_length,
        );

        let parser = ToolCallParser::new(config.model.tool_calling.clone());
        let backend = BackendManager::from_config(&config);

        let startup_msg = format!(
            "Forge v{} | Project: {} | mode: {}",
            env!("CARGO_PKG_VERSION"),
            project_path.display(),
            mode.label(),
        );

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
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // Enter TUI mode FIRST so the user can see the splash and exit
        terminal::enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

        // Show splash immediately
        terminal.draw(|frame| self.render(frame))?;

        // Start the backend — this can take a while for large models.
        // We poll for quit keys during the wait so the user isn't trapped.
        self.messages.push(DisplayMessage::System(
            format!("Starting {} backend...", self.backend.backend_name()),
        ));
        terminal.draw(|frame| self.render(frame))?;

        let start_result = self.start_backend_interruptible(&mut terminal).await;

        match start_result {
            Ok(()) => {}
            Err(StartErr::UserQuit) => {
                terminal::disable_raw_mode()?;
                stdout().execute(LeaveAlternateScreen)?;
                return Ok(());
            }
            Err(StartErr::BackendFailed(e)) => {
                self.messages
                    .push(DisplayMessage::System(format!("Backend error: {e}")));
                self.messages.push(DisplayMessage::System(
                    "Running in offline mode. Use /model to configure.".to_string(),
                ));
            }
        }

        let result = self.main_loop(&mut terminal).await;

        // Restore terminal
        terminal::disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;

        // Intentionally DO NOT stop the backend server here.
        // Keeping it warm means the next `forge` invocation connects instantly
        // instead of waiting 30-60s for model reload. The server uses idle
        // memory that macOS will reclaim under pressure anyway.

        result
    }

    /// Start the backend with Ctrl+C support via tokio::select.
    async fn start_backend_interruptible(
        &mut self,
        _terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> std::result::Result<(), StartErr> {
        // Quick check — maybe a server is already running
        if self.backend.health_check().await {
            return Ok(());
        }

        self.messages.push(DisplayMessage::System(
            "Loading model... (Ctrl+C to cancel)".to_string(),
        ));

        // Use tokio::select to race backend start against Ctrl+C signal
        let config = self.config.clone();
        tokio::select! {
            result = self.backend.start(&config) => {
                match result {
                    Ok(()) => Ok(()),
                    Err(e) => Err(StartErr::BackendFailed(e.to_string())),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                self.backend.stop();
                Err(StartErr::UserQuit)
            }
        }
    }

    async fn main_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            // Render
            terminal.draw(|frame| self.render(frame))?;

            // Poll for key events (non-blocking)
            if event::poll(std::time::Duration::from_millis(16))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key).await?;
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

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),        // Status bar
                Constraint::Min(5),           // Messages
                Constraint::Length(1),        // Status line
                Constraint::Length(3),        // Input
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
            layout[0],
            frame.buffer_mut(),
        );

        // Messages — include streaming text as a live assistant message
        let mut display_msgs = self.messages.clone();
        if self.is_generating && !self.streaming_text.is_empty() {
            display_msgs.push(DisplayMessage::Assistant(
                format!("{}▊", self.streaming_text),
            ));
        }
        render::render_messages(&display_msgs, self.mode.label(), layout[1], frame.buffer_mut());

        // Status line
        render::render_status_line(
            self.engine.estimated_tokens(),
            self.config.model.context_length,
            self.rules.rule_count(),
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
            }
            _ => {}
        }

        Ok(())
    }

    /// Known slash commands — anything starting with / that isn't in this
    /// list gets treated as normal user input (e.g. file paths).
    const SLASH_COMMANDS: &'static [&'static str] = &[
        "/help", "/clear", "/compact", "/rules", "/permissions", "/templates",
        "/config", "/model", "/project", "/memory", "/context", "/plugin",
        "/hardware", "/chat", "/code", "/skill", "/quit", "/exit",
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

        // Add user message
        self.messages.push(DisplayMessage::User(text.clone()));
        self.engine.add_user_message(&text);

        // Start the agentic loop
        self.run_agentic_loop().await
    }

    /// Agentic loop: generate → parse tool calls → execute → feed results → repeat.
    /// Uses streaming for the first turn (user sees tokens live), then falls back
    /// to synchronous generate for tool-result continuations (speed over UX for
    /// intermediate turns).
    async fn run_agentic_loop(&mut self) -> Result<()> {
        const MAX_TURNS: usize = 25;

        for turn in 0..MAX_TURNS {
            self.engine.compact();
            self.is_generating = true;
            let request = self.engine.build_request(&self.config);

            if turn == 0 {
                // First turn: try streaming for live token display
                match self.backend.generate_stream(&request).await {
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
            let response = match self.backend.generate(&request).await {
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
                self.engine.compact();
                let request = self.engine.build_request(&self.config);

                let response = match self.backend.generate(&request).await {
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
                self.messages.push(DisplayMessage::Assistant(content));
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

        self.engine.add_assistant_message(response);

        // Process tool calls (from native or parsed)
        let all_calls = tool_calls
            .unwrap_or_default()
            .into_iter()
            .chain(parsed_calls)
            .collect::<Vec<_>>();

        for call in all_calls {
            // Handle request_permissions specially (pre-flight batch approval)
            if call.name == "request_permissions" {
                let result = self.handle_permission_request(&call.arguments);
                self.engine.add_tool_result(&call.id, &result);
                continue;
            }

            // Step 1: Hard-block check (compile-time constants, no override)
            if let Some(reason) = permissions::hard_block_check(&call.name, &call.arguments) {
                self.messages.push(DisplayMessage::PermissionBlocked {
                    tool: call.name.clone(),
                    reason: reason.clone(),
                });
                self.engine.add_tool_result(&call.id, &format!("HARD BLOCKED: {reason}"));
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
                        self.messages.push(DisplayMessage::PermissionDenied {
                            tool: call.name.clone(),
                        });
                        self.engine.add_tool_result(&call.id, "DENIED: User declined permission");
                        continue;
                    }
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

                    let ctx = ToolContext {
                        cwd: self.project_path.clone(),
                        project_path: self.project_path.clone(),
                    };

                    let args_summary = summarize_args(&call.arguments);
                    match self.tools.execute(&call.name, call.arguments.clone(), &ctx).await {
                        Ok(result) => {
                            self.messages.push(DisplayMessage::ToolCall {
                                name: call.name.clone(),
                                args_summary: args_summary.clone(),
                                result: result.output.clone(),
                                is_error: result.is_error,
                            });
                            self.engine.add_tool_result(&call.id, &result.output);

                            // Run post-hooks
                            let post_event = format!("post:{}", call.name);
                            let post_hooks = self.plugin_manager.get_hooks(&post_event);
                            for hook in &post_hooks {
                                let _ = crate::plugins::hooks::run_post_hook(
                                    hook,
                                    &call.name,
                                    &params_json,
                                    &result.output,
                                    &self.project_path,
                                ).await;
                            }
                        }
                        Err(e) => {
                            let err = format!("Tool error: {e}");
                            self.messages.push(DisplayMessage::ToolCall {
                                name: call.name.clone(),
                                args_summary,
                                result: err.clone(),
                                is_error: true,
                            });
                            self.engine.add_tool_result(&call.id, &err);
                        }
                    }
                }
            }
        }

        Ok(())
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
                    let mut out = String::from("Available skills via slash commands:\n\n");
                    for skill in &self.skills {
                        out.push_str(&format!(
                            "- `{}` ({}) — {}\n",
                            skill.trigger, skill.source, skill.description
                        ));
                    }
                    Some(out)
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
                    "Commands: /help /clear /compact /rules /permissions /templates /config /model /project /memory /context /plugin /hardware /skill /chat /code /quit".to_string(),
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
                if parts.len() > 1 {
                    // /memory <text> — append to project memory
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
                    // /memory — show current memory
                    let memory = crate::conversation::prompt::load_memory_context(&self.project_path);
                    match memory {
                        Some(m) => self.messages.push(DisplayMessage::System(m)),
                        None => self.messages.push(DisplayMessage::System("No memory notes found.".to_string())),
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
                    _ => {
                        self.messages.push(DisplayMessage::System(
                            "Usage: /plugin <list|install|uninstall|search|info>".to_string(),
                        ));
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
                    let mut info = String::from("Available skills:\n");
                    for skill in &self.skills {
                        info.push_str(&format!(
                            "  {} — {} [{}]\n",
                            skill.trigger, skill.description, skill.source
                        ));
                    }
                    self.messages.push(DisplayMessage::System(info));
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
}
