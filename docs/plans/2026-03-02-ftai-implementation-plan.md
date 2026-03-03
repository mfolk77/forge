# FTAI Implementation Plan

**Design:** `2026-03-02-ftai-terminal-harness-design.md`
**Approach:** Bottom-up — build foundations first, layer features on top

---

## Phase 1: Project Skeleton & Config (Day 1)

### Step 1.1: Initialize Rust project
- `cargo init --name ftai`
- Set up `Cargo.toml` with core dependencies (tokio, serde, clap, toml)
- Create module structure: `config/`, `backend/`, `conversation/`, `rules/`, `tools/`, `tui/`
- Set up basic CLI arg parsing with clap (subcommands: `run`, `model`, `config`)

### Step 1.2: Config system
- Define `Config` struct (model settings, permissions, paths)
- Implement TOML loading from `~/.ftai/config.toml`
- Implement project-level config merging (`<project>/.ftai/config.toml`)
- Create `~/.ftai/` directory structure on first run
- Write tests for config loading and merging

**Deliverable:** `ftai` binary that parses args, loads config, creates `~/.ftai/`

---

## Phase 2: Model Backend (Days 2-4)

### Step 2.1: Backend trait
- Define `ModelBackend` trait (load, generate, generate_stream, tool_calling, context_length)
- Define `ChatRequest`, `ChatResponse`, `Token` types
- Define `ModelConfig` (path, backend type, context length, temperature, etc.)

### Step 2.2: llama.cpp backend
- Add `llama-cpp-rs` dependency
- Implement `LlamaCppBackend` — model loading, text generation, streaming
- Handle Metal GPU acceleration detection
- Implement GGUF model discovery from `~/.ftai/models/`
- Write tests (load model, generate tokens)

### Step 2.3: MLX backend
- Create C FFI bridge to MLX framework (`build.rs`)
- Implement `MlxBackend` — model loading, generation, streaming
- Handle Apple Silicon detection
- Write tests

### Step 2.4: Hardware detection & model recommendation
- Detect: Apple Silicon vs x86, GPU (Metal/CUDA), RAM
- Map hardware → recommended model (per the matrix in design)
- Implement `ftai model list/install/use/info` subcommands
- HuggingFace download integration (reqwest + progress bar)

**Deliverable:** `ftai` can load a local model and generate text responses

---

## Phase 3: Conversation Engine (Days 5-6)

### Step 3.1: Message history
- Define message types (System, User, Assistant, ToolCall, ToolResult)
- Implement conversation history with context window management
- Context compression (summarize old messages when approaching limit)

### Step 3.2: System prompt construction
- Build system prompt from: base instructions + tool descriptions + active rules + project context
- Load `<project>/.ftai/RULES.md` and inject into prompt
- Load memory files and inject relevant context

### Step 3.3: Tool call parsing
- Native parser — extract function calls from model's structured output
- Prompted parser — parse XML/JSON tool calls from text output
- Hybrid mode — try native, fall back to prompted
- Write tests for each parsing mode with various model output formats

**Deliverable:** Full conversation loop — user types, model responds, tool calls parsed

---

## Phase 4: Tool System (Days 7-10)

### Step 4.1: Tool trait & registry
- Define `Tool` trait (name, description, parameters schema, execute)
- Implement `ToolRegistry` — register tools, dispatch by name, collect results
- Define `ToolContext` (cwd, project, config, user confirmation callback)

### Step 4.2: File tools
- `file_read` — read file with optional line range, line numbers
- `file_write` — create/overwrite files
- `file_edit` — exact string replacement (old_string → new_string)
- Tests for each

### Step 4.3: Search tools
- `glob` — file pattern matching (use `glob` crate)
- `grep` — regex content search (use `grep` crate or shell out to `rg`)
- Tests

### Step 4.4: Bash tool
- `bash` — execute commands in persistent shell session
- Persistent working directory across calls
- Configurable timeout (default 2min, max 10min)
- Background execution mode
- Stdout/stderr capture and streaming
- Tests

### Step 4.5: Git tools
- `git_status`, `git_diff`, `git_log`, `git_commit`, `git_branch`, `git_push`
- `git_pr_create` — via `gh` CLI
- Use `git2` crate for read operations, shell out for write operations
- Tests

### Step 4.6: Web tools
- `web_fetch` — HTTP GET, HTML→markdown conversion
- `web_search` — pluggable search backend (start with DuckDuckGo or SearXNG)
- Tests

### Step 4.7: Agent & interaction tools
- `agent_spawn` — launch sub-conversation with isolated context
- `ask_user` — prompt user for text input or multiple choice
- Tests

**Deliverable:** All 11 tools working, callable from conversation loop

---

## Phase 5: Rules DSL Engine (Days 11-14)

### Step 5.1: Lexer
- Tokenize `.ftai` files: keywords (rule, on, when, reject, require, modify, unless, reason, scope), strings, identifiers, operators, braces
- Handle comments (#)
- Tests for tokenization

### Step 5.2: Parser
- Parse tokens into AST: `RuleSet` → `Rule` / `Scope` → `Event`, `Condition`, `Action`, `Reason`
- Expression parser for conditions (boolean logic, function calls, operators)
- Error reporting with line/column numbers
- Tests for parsing valid and invalid DSL

### Step 5.3: Built-in functions
- String: `contains()`, `matches()`
- File: `files_match()`, `files_exist()`, `extension()`, `dirname()`, `line_count()`
- Context: `staged_files`, `changed_files`, `project`, `adds_lines_matching()`
- Interaction: `confirmed_by_user`
- Tests for each function

### Step 5.4: Evaluator
- Evaluate rule conditions against tool call context
- Pre-execution check: run all matching rules, collect REJECT/MODIFY/ALLOW
- Post-execution check: filter/transform output
- Rule precedence handling (project > user-project > global)
- Tests for evaluation logic

### Step 5.5: Integration with tool system
- Wire rules engine into tool dispatch pipeline
- Pre-check before every tool execution
- Post-check after every tool execution
- Display rule violations in TUI
- Hot-reload rules from disk (`/rules reload`)
- Integration tests

**Deliverable:** Full rules DSL — parse, evaluate, enforce on every tool call

---

## Phase 6: Terminal UI (Days 15-18)

### Step 6.1: Basic TUI scaffold
- `ratatui` + `crossterm` setup
- Main event loop (input events, render cycle)
- Layout: status bar, message area, input area

### Step 6.2: Input handling
- Multi-line text input with cursor movement
- Input history (up/down arrows)
- Enter to submit, Shift+Enter for newline
- Ctrl+C cancel, Ctrl+D exit

### Step 6.3: Message rendering
- Markdown rendering (pulldown-cmark → ratatui widgets)
- Syntax highlighting for code blocks (syntect)
- Tool call display — collapsible boxes with tool name, params, result
- Rule violation inline warnings

### Step 6.4: Status displays
- Top status bar: model, backend, cwd
- Bottom status line: task count, token usage, active rules
- Token generation progress / streaming indicator

### Step 6.5: Slash commands
- `/model`, `/rules`, `/config`, `/clear`, `/compact`, `/project`, `/help`
- Tab completion for commands

**Deliverable:** Polished terminal UI with full interaction

---

## Phase 7: Polish & Integration (Days 19-21)

### Step 7.1: Memory system
- `~/.ftai/memory/MEMORY.md` — persistent across sessions
- Per-project memory in `~/.ftai/projects/<project>/memory/`
- Load into system prompt context

### Step 7.2: First-run experience
- Hardware detection
- Model recommendation and download prompt
- Create `~/.ftai/` with example `config.toml` and `rules.ftai`

### Step 7.3: Error handling & edge cases
- Graceful model loading failures
- Network errors for web tools
- Tool timeout handling
- Context window overflow handling

### Step 7.4: End-to-end testing
- Full conversation flow tests
- Rules enforcement integration tests
- Tool + rules interaction tests

**Deliverable:** Shippable v0.1.0

---

## Implementation Notes

- Build and test each phase before moving to the next
- Each step should have tests before moving on
- The rules DSL is the most novel piece — spend extra time on its parser/evaluator
- MLX backend may need significant C FFI work — consider llama.cpp-only for v0.1 if MLX proves slow to integrate
- Tool call parsing quality depends heavily on the model — test with multiple models early
