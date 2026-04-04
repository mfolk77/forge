<p align="center">
  <img src="https://img.shields.io/badge/Forge-v0.1.0-E89C38?style=for-the-badge" alt="Forge v0.1.0" />
  <img src="https://img.shields.io/badge/Rust-2021-DEA584?style=for-the-badge&logo=rust" alt="Rust" />
  <img src="https://img.shields.io/badge/License-MIT-blue?style=for-the-badge" alt="MIT License" />
  <img src="https://img.shields.io/badge/Tests-1519_passing-brightgreen?style=for-the-badge" alt="Tests" />
</p>

<h1 align="center">Forge</h1>
<p align="center"><strong>AI terminal coding harness</strong></p>
<p align="center">
  A local-first agentic coding assistant that runs entirely on your machine.<br/>
  No API keys. No cloud. Your models, your rules, your data.
</p>

---

## Install

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/mfolk77/forge/main/install.sh | sh
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/mfolk77/forge/main/install.ps1 | iex
```

### What this does

1. Detects your OS and architecture
2. Downloads the latest pre-built binary from [GitHub Releases](https://github.com/mfolk77/forge/releases)
3. Installs it to `~/.local/bin/forge` (customizable via `FORGE_INSTALL_DIR`)
4. Adds it to your PATH if needed (Windows does this automatically)

**No Rust, no compiler, no build tools required.**

### Verify

```bash
forge --version
```

### Update

```bash
forge update
```

Forge can self-update from GitHub Releases. Run `forge update --check` to check without installing.

---

## Quick start

```bash
# 1. Install a model (Forge auto-detects your hardware and recommends one)
forge hardware
forge model install Qwen/Qwen2.5-Coder-7B-Instruct-MLX    # macOS Apple Silicon
forge model install Qwen/Qwen2.5-Coder-7B-Instruct-GGUF    # Linux / Windows

# 2. Activate it
forge model use Qwen2.5-Coder-7B-Instruct-MLX

# 3. Start coding
cd ~/my-project
forge
```

Forge auto-detects **Coding mode** (if `.git/` or `.ftai/` exists) or **Chat mode** (otherwise). In Coding mode, the full agentic loop with tools and project context is active. In Chat mode, tools are available but passive.

---

## Overview

Forge is a terminal-native AI coding assistant built in Rust. It connects to local LLMs (via MLX on Apple Silicon or llama.cpp on any platform), provides a full agentic tool-calling loop, enforces project-specific rules, and learns from your sessions over time.

```
forge                     # Start interactive session in current directory
forge --project ~/myapp   # Start in a specific project
forge --resume            # Resume last conversation
forge doctor              # Check backends, hardware, config
forge config show
```

### Key capabilities

- **Agentic loop** -- streams tokens from the model, parses tool calls (XML/JSON/hybrid), checks permissions, evaluates rules, executes tools, feeds results back, and repeats
- **10 built-in tools** -- bash, file read/write/edit, glob, grep, git, web fetch, ask user, request permissions
- **Rule engine** -- a custom DSL for project-specific constraints (`reject`, `require`, `modify` with boolean logic and built-in functions)
- **3-tier permission system** -- Safe (auto-approve) / Write (configurable) / Destructive (always confirm), with hard-block constants and a session grant cache
- **Plugin system** -- extend Forge with custom tools, hooks, skills, and rules via `plugin.toml` manifests
- **38 built-in skills** -- shipped across 5 plugins, activatable via slash commands
- **Semantic code search** -- BGE-small-en embeddings with SQLite vector store for context-aware retrieval
- **Session memory** -- persists conversations, learns patterns, auto-generates rules via the evolution engine
- **Self-update** -- `forge update` pulls latest release from GitHub
- **Cross-platform** -- macOS (Apple Silicon native via MLX), Linux (llama.cpp + CUDA), Windows (llama.cpp + cmd.exe)

---

## Hardware support

Forge auto-detects your hardware and recommends an appropriate model:

| Platform | GPU | Recommended model | Backend |
|----------|-----|-------------------|---------|
| Apple Silicon 32GB+ | Metal | Qwen2.5-Coder-32B-Q4 | MLX |
| Apple Silicon 16GB+ | Metal | Qwen2.5-Coder-7B-Q4 | MLX |
| Apple Silicon <16GB | Metal | Qwen2.5-Coder-3B-Q4 | MLX |
| NVIDIA 24GB+ VRAM | CUDA | Qwen2.5-Coder-32B-Q4 | llama.cpp |
| NVIDIA <24GB VRAM | CUDA | DeepSeek-Coder-V2-Lite-Q4 | llama.cpp |
| CPU-only (x86_64) | -- | Qwen2.5-Coder-7B-Q4 | llama.cpp |

```bash
forge hardware   # or /hardware in TUI
```

### LLM backends

Forge needs a local LLM to run. It handles the backend automatically based on your platform:

- **macOS Apple Silicon** -- MLX (native Metal acceleration). Forge starts the MLX server for you.
- **Linux / Windows** -- llama.cpp (CUDA if available, CPU fallback). Forge starts the llama.cpp server for you.
- **External server** -- connect to any OpenAI-compatible API (Ollama, LM Studio, vLLM) by setting `backend = "external"` in config.

If you already use **Ollama** or **LM Studio**, Forge can connect to them directly -- no additional setup needed. Just point it at the server:

```toml
# ~/.ftai/config.toml
[model]
backend = "external"

[model.external]
url = "http://localhost:11434/v1"   # Ollama default
```

---

## Architecture

```
                                    +---------------+
                                    |   TUI App     |
                                    |  (ratatui)    |
                                    +-------+-------+
                                            |
               +----------------------------+----------------------------+
               |                            |                            |
        +------+------+          +---------+---------+           +------+------+
        | Conversation|          |  Backend Manager   |          |   Tools     |
        |   Engine    |          | (MLX/llama.cpp/    |          |  Registry   |
        |             |          |  External)         |          | (10 tools)  |
        +------+------+          +---------+---------+           +------+------+
               |                           |                           |
    +----------+----------+     +----------+----------+    +---------+---------+
    |          |          |     |          |          |    |         |         |
 Adapter   Parser    Recovery  MLX     llama.cpp  HTTP   Bash   File ops   Git
 (Qwen/    (XML/     Pipeline Server   Server    Client
 Hermes/   JSON/
 Generic)  Hybrid)
               |
    +----------+----------------------+
    |          |          |           |
 Permissions  Rules    Plugins    Skills
 (3-tier)    (DSL)   (manifest)  (builtin)
```

### Module map

| Module | Purpose | Key types |
|--------|---------|-----------|
| `backend/` | LLM server lifecycle, hardware detection | `BackendManager`, `HardwareInfo`, `HttpModelClient` |
| `config/` | Multi-layer config precedence | `Config`, `BackendType`, `PermissionMode` |
| `conversation/` | Message history, tool parsing, prompt building | `ConversationEngine`, `ToolCallParser`, `ModelAdapter` |
| `evolution/` | Session pattern learning, auto-rule generation | `EvolutionEngine`, `SkillBuilder` |
| `formatting/` | Output template system | `TemplateSet`, `FormattingConfig` |
| `inference/` | Direct model inference (FFI, sampling) | `LlamaContext`, `Sampler`, `KnowledgeSampler` |
| `permissions/` | Tool execution gating | `PermissionTier`, `GrantCache`, `hard_block_check` |
| `plugins/` | External extension system | `PluginManager`, `PluginTool`, `ResolvedHook` |
| `rules/` | Custom constraint DSL | `RulesEngine`, `EvalContext`, `RuleAction` |
| `search/` | Semantic code indexing | `CodeIndexer`, `SearchStore`, `SearchEngine` |
| `session/` | Persistence, token budgeting, memory | `SessionManager`, `TokenBudget`, `MemoryManager` |
| `skills/` | Built-in and plugin skill system | `LoadedSkill`, `builtin_skills()` |
| `tools/` | Executable tool implementations | `Tool` trait, `ToolRegistry`, `BashTool` |
| `tui/` | Terminal UI with ratatui | `TuiApp`, `InputState`, `DisplayMessage` |

---

## Configuration

Forge uses layered TOML configuration with this precedence (highest wins):

```
<project>/.ftai/config.toml          # Project-local overrides
~/.ftai/projects/<encoded>/config.toml  # Per-project user prefs
~/.ftai/config.toml                   # Global defaults
Built-in defaults                     # Hardcoded fallback
```

### Default configuration

```toml
[model]
backend = "mlx"           # "mlx", "llamacpp", or "external"
context_length = 32768     # Token context window
temperature = 0.3          # Sampling temperature
tool_calling = "hybrid"    # "native", "prompted", or "hybrid"

[model.llamacpp]
gpu_layers = -1            # -1 = offload all layers to GPU
threads = 8

[model.mlx]
quantization = "q4"

[permissions]
mode = "auto"              # "ask", "auto", or "yolo"

[plugins]
enabled = true
auto_update = false
```

### Permission modes

| Mode | Behavior |
|------|----------|
| `ask` | Prompt for every non-safe tool call |
| `auto` | Auto-grant Safe + Write; confirm Destructive |
| `yolo` | Grant everything without prompting |

### Permission tiers

| Tier | Examples | Default |
|------|----------|---------|
| **Safe** | file_read, glob, grep, web_fetch | Always approved |
| **Write** | file_write, file_edit, bash (non-destructive) | Based on permission mode |
| **Destructive** | git push, bash rm, force operations | Always requires confirmation |

Hard-blocked operations (no override): `rm -rf /`, writes to `/etc/passwd`, `/System`, etc.

---

## Tools

Forge ships with 10 built-in tools that the model can invoke:

| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands (bash on Unix, cmd.exe on Windows) with timeout + RLIMIT_CPU sandboxing |
| `file_read` | Read files with optional offset/limit for large files |
| `file_write` | Create or overwrite files, auto-creates parent directories |
| `file_edit` | Exact string replacement with uniqueness enforcement |
| `glob` | Find files by glob pattern (e.g., `**/*.rs`) |
| `grep` | Search file contents by regex with context lines |
| `git` | Git operations: status, diff, log, add, commit, branch, push, PR create |
| `web_fetch` | HTTP GET with HTML stripping and 50KB truncation |
| `ask_user` | Interactive question with optional multiple-choice |
| `request_permissions` | Pre-flight batch permission approval for multi-step tasks |

Plugin tools are namespaced as `plugin:<name>:<tool>` and go through the full permission pipeline.

---

## Rules DSL

Forge includes a custom rule language for enforcing project constraints. Rules are loaded from `~/.ftai/rules.ftai`, `<project>/.ftai/rules.ftai`, and plugin rule files.

### Syntax

```
rule "rule-name" {
  on <event>
  [when <condition>]
  <action> <expression>
  [unless <override>]
  [reason "explanation"]
}

scope "path/pattern" {
  rule "scoped-rule" { ... }
}
```

### Events

`commit`, `pr_create`, `tool:bash`, `tool:file_write`, `response`, `session_start`, `any`

### Actions

- `reject <expr>` -- block if expression is true
- `require <expr>` -- require expression to be true
- `modify <expr>` -- transform output

### Built-in functions

| Function | Description |
|----------|-------------|
| `contains(haystack, needle)` | Substring check |
| `matches(text, pattern)` | Regex match |
| `extension(path)` | File extension |
| `dirname(path)` | Directory path |
| `files_exist(path)` | File/directory exists |
| `files_match(pattern, files)` | Glob match on file list |
| `line_count(path)` | Line count |
| `adds_lines_matching(pattern, diff)` | Check diff additions |

### Examples

```
# Block dangerous shell commands
rule "no-rm-rf" {
  on tool:bash
  reject matches(command, "rm\\s+-rf")
  reason "Destructive file deletion blocked"
}

# Require tests in commits
rule "need-tests" {
  on commit
  require files_match("*test*", staged_files)
  reason "All commits must include test files"
}

# Project-scoped rule
scope "~/Developer/my-app" {
  rule "rust-only" {
    on tool:file_write
    require extension(path) == "rs"
    reason "Only Rust files in this project"
  }
}
```

---

## Skills

Skills are markdown guides injected into the model's context via slash commands.

### Built-in skills (38 across 5 plugins)

| Plugin | Skills |
|--------|--------|
| **folktech-dev-toolkit** | `/secure`, `/tdd`, `/perf`, `/audit-dep`, `/doc`, `/quality` |
| **forge-superpowers** | `/brainstorm`, `/plan`, `/execute`, `/tdd-workflow`, `/debug`, `/review`, `/review-feedback`, `/parallel`, `/subagent-dev`, `/verify`, `/finish` |
| **forge-dev-tools** | `/changelog`, `/frontend`, `/organize`, `/research`, `/webapp-test`, `/mcp`, `/create-skill`, `/audit-config`, `/enhance-image`, `/comms` |
| **forge-document-tools** | `/pdf`, `/docx`, `/xlsx`, `/pptx`, `/canvas`, `/markdown` |
| **forge-plugin-dev** | `/plugin-dev`, `/skill-dev`, `/hook-dev`, `/command-dev`, `/agent-dev` |

```
/skill              # List all available skills
/skill commit       # Show the commit skill content
/commit             # Activate the commit skill (inject into context)
```

Plugin skills override built-in skills with matching triggers.

---

## Plugins

Forge is extensible via plugins installed to `~/.ftai/plugins/`.

### Plugin manifest (`plugin.toml`)

```toml
[plugin]
name = "my-plugin"
version = "1.0.0"
description = "My custom plugin"
author = "Your Name"

[[tools]]
name = "lint"
description = "Run project linter"
command = "tools/lint.sh"
params = { type = "object" }

[[skills]]
name = "deploy"
file = "skills/deploy.md"
description = "Deployment guide"
trigger = "/deploy"

[[hooks]]
event = "pre:bash"
command = "hooks/validate.sh"

[registry]
repo = "https://github.com/you/my-plugin"
```

### Plugin commands

```
/plugin list                    # List installed plugins
/plugin install <git-url>       # Install from Git repo
/plugin install <local-path>    # Install from local directory
/plugin uninstall <name>        # Remove a plugin
/plugin search <query>          # Search plugin registry
/plugin info <name>             # Show plugin details
```

### Hook lifecycle

- **Pre-hooks** (`pre:<tool>`) run before tool execution. Non-zero exit blocks the tool.
- **Post-hooks** (`post:<tool>`) run after execution. Fire-and-forget with result in environment.
- Hooks execute with a 10-second timeout. Tools have a 30-second timeout.

---

## TUI Commands

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/clear` | Clear conversation and permission grants |
| `/compact` | Compress context window |
| `/rules` | Show active rules (`/rules reload` to reload) |
| `/permissions` | Show permission mode and grants (`/permissions clear`) |
| `/templates` | Show formatting templates |
| `/config` | Show current configuration |
| `/model` | Show active model info |
| `/project` | Show or switch project directory |
| `/memory` | Show or append to project memory |
| `/context` | Show, init, or edit FTAI.md (`/context init`, `/context edit`) |
| `/plugin` | Plugin management |
| `/hardware` | Show hardware info and model recommendation |
| `/skill` | List or activate skills |
| `/chat` | Switch to Chat mode |
| `/code` | Switch to Coding mode |
| `/quit` | Exit |

### Keyboard shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Submit message |
| `Shift+Enter` | New line |
| `Ctrl+C` | Cancel generation / clear input / quit |
| `Ctrl+D` | Quit |
| `Shift+Up/Down` | Scroll message history |
| `PageUp/PageDown` | Scroll fast |
| `Esc` | Clear input |
| `Up/Down` | Input history navigation |

---

## Project context (`FTAI.md`)

Create a `.ftai/FTAI.md` in your project root to give Forge project-specific instructions:

```bash
forge init           # Creates .ftai/ directory + template FTAI.md
/context init        # Same thing, from inside the TUI
/context edit        # Opens FTAI.md in $EDITOR
```

```markdown
# Project: my-app

## Stack
Rust, PostgreSQL, React frontend

## Conventions
- snake_case for all identifiers
- Error types use thiserror

## Architecture
- src/api/ -- HTTP handlers
- src/db/ -- Database layer
- src/core/ -- Business logic

## Testing
cargo test -- runs 500+ tests
Integration tests require DATABASE_URL

## Gotchas
- The auth module uses a custom JWT implementation
- Never modify migration files after they ship
```

Content is capped at 10,000 characters and included in the system prompt.

---

## Build from source

Only needed if you want to develop Forge itself or can't use the pre-built binaries.

### Prerequisites

- **Rust** 1.75+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)

### Build

```bash
git clone https://github.com/mfolk77/forge.git
cd forge
cargo build --release
```

### Install globally

```bash
cargo install --path . --root ~/.local --force
# Binary: ~/.local/bin/forge
```

### Run tests

```
cargo test              # 1,519 tests
```

---

## Directory structure

```
~/.ftai/
  config.toml          # Global configuration
  models/              # Downloaded model files
  memory/              # Global memory store
  plugins/             # Installed plugins
  projects/            # Per-project config overrides
  rules.ftai           # Global rules
  logs/                # Execution logs
  sessions.db          # Conversation history

<project>/
  .ftai/
    config.toml        # Project-local config
    rules.ftai         # Project-local rules
    memory/
      MEMORY.md        # Project memory notes
    FTAI.md            # Project context document
```

---

## Security model

Forge follows the **FolkTech Secure Coding Standard**:

- **P0 (Critical)**: Input injection, path traversal, auth bypass, LLM output injection -- tested on every code change
- **P1 (High)**: DoS, information leakage, unsafe memory operations
- **P2 (Medium)**: Logic bugs, state confusion, resource exhaustion

Key protections:
- Rust's `Command::arg()` prevents shell injection (arguments are never interpreted by the shell)
- Plugin paths validated against traversal on both Unix (`../`) and Windows (`..\`, `C:\`, UNC `\\`)
- Hard-blocked operations cannot be overridden by any permission mode
- Tool results truncated to prevent context window poisoning (2,000 token cap, middle-out truncation)
- Model download validates filenames from HuggingFace weight indices (blocks shard traversal)
- GBNF grammar generation validates tool names before interpolation (prevents grammar injection)

---

## License

MIT -- see [LICENSE](LICENSE) for details.

---

<p align="center">
  <strong>Built by <a href="https://github.com/mfolk77">FolkTech AI</a></strong>
</p>
