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
use crate::tools::{ToolContext, ToolRegistry};

use super::input::InputState;
use super::render::{self, DisplayMessage};

pub struct TuiApp {
    config: Config,
    backend: BackendManager,
    engine: ConversationEngine,
    parser: ToolCallParser,
    tools: ToolRegistry,
    rules: RulesEngine,
    templates: TemplateSet,
    grant_cache: GrantCache,
    input: InputState,
    messages: Vec<DisplayMessage>,
    project_path: PathBuf,
    should_quit: bool,
    is_generating: bool,
    streaming_text: String,
}

impl TuiApp {
    pub fn new(config: Config, project_path: PathBuf) -> Self {
        let tools = ToolRegistry::with_defaults();
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
        let rules_summary = if rules.rule_count() > 0 {
            Some(rules.summary())
        } else {
            None
        };

        let templates = formatting::load_templates(&config.formatting, Some(&project_path))
            .unwrap_or_default();

        let system_prompt = prompt::build_system_prompt(
            &project_path,
            &tool_defs,
            rules_summary.as_deref(),
            memory.as_deref(),
            Some(&templates),
            &config.formatting.enabled,
        );

        let engine = ConversationEngine::new(
            system_prompt,
            tool_defs,
            config.model.context_length,
        );

        let parser = ToolCallParser::new(config.model.tool_calling.clone());
        let backend = BackendManager::from_config(&config);

        Self {
            config,
            backend,
            engine,
            parser,
            tools,
            rules,
            templates,
            grant_cache: GrantCache::new(),
            input: InputState::new(),
            messages: vec![DisplayMessage::System(format!(
                "ftai v{} | Project: {}",
                env!("CARGO_PKG_VERSION"),
                project_path.display()
            ))],
            project_path,
            should_quit: false,
            is_generating: false,
            streaming_text: String::new(),
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // Start the backend
        self.messages.push(DisplayMessage::System(
            format!("Starting {} backend...", self.backend.backend_name()),
        ));

        if let Err(e) = self.backend.start(&self.config).await {
            self.messages
                .push(DisplayMessage::System(format!("Backend error: {e}")));
            self.messages.push(DisplayMessage::System(
                "Running in offline mode. Use /model to configure.".to_string(),
            ));
        }

        // Enter TUI mode
        terminal::enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

        let result = self.main_loop(&mut terminal).await;

        // Restore terminal
        terminal::disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;

        result
    }

    async fn main_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            // Render
            terminal.draw(|frame| self.render(frame))?;

            // Handle input
            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key).await?;
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

        // Messages
        render::render_messages(&self.messages, layout[1], frame.buffer_mut());

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
        render::render_input(input_text, self.input.cursor_col, layout[3], frame.buffer_mut());
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
            KeyCode::Up => self.input.history_up(),
            KeyCode::Down => self.input.history_down(),
            KeyCode::Esc => {
                self.input = InputState::new();
            }
            KeyCode::Char(c) => self.input.insert_char(c),
            _ => {}
        }

        Ok(())
    }

    async fn handle_submit(&mut self, text: String) -> Result<()> {
        // Check for slash commands
        if text.starts_with('/') {
            return self.handle_slash_command(&text).await;
        }

        // Add user message
        self.messages.push(DisplayMessage::User(text.clone()));
        self.engine.add_user_message(&text);

        // Generate response
        self.is_generating = true;
        let request = self.engine.build_request(&self.config);

        match self.backend.generate(&request).await {
            Ok(response) => {
                self.process_response(response).await?;
            }
            Err(e) => {
                self.messages
                    .push(DisplayMessage::System(format!("Error: {e}")));
            }
        }

        self.is_generating = false;
        Ok(())
    }

    async fn process_response(&mut self, response: ChatResponse) -> Result<()> {
        let content = response.message.content.clone();
        let tool_calls = response.message.tool_calls.clone();

        // Parse tool calls from text (for prompted mode)
        let (display_text, parsed_calls) = self.parser.parse(&content);

        if !display_text.trim().is_empty() {
            self.messages
                .push(DisplayMessage::Assistant(display_text));
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
                    let ctx = ToolContext {
                        cwd: self.project_path.clone(),
                        project_path: self.project_path.clone(),
                    };

                    match self.tools.execute(&call.name, call.arguments.clone(), &ctx).await {
                        Ok(result) => {
                            self.messages.push(DisplayMessage::ToolCall {
                                name: call.name.clone(),
                                result: result.output.clone(),
                                is_error: result.is_error,
                            });
                            self.engine.add_tool_result(&call.id, &result.output);
                        }
                        Err(e) => {
                            let err = format!("Tool error: {e}");
                            self.messages.push(DisplayMessage::ToolCall {
                                name: call.name.clone(),
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

    async fn handle_slash_command(&mut self, text: &str) -> Result<()> {
        let parts: Vec<&str> = text.trim().split_whitespace().collect();
        let cmd = parts[0];

        match cmd {
            "/help" => {
                self.messages.push(DisplayMessage::System(
                    "Commands: /help /clear /compact /rules /permissions /templates /config /model /project /memory /hardware /quit".to_string(),
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
            "/quit" | "/exit" => {
                self.should_quit = true;
            }
            _ => {
                self.messages.push(DisplayMessage::System(
                    format!("Unknown command: {cmd}. Type /help for available commands."),
                ));
            }
        }

        Ok(())
    }
}
