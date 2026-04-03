# FTAI Terminal Coding Harness — Design Document

**Date:** 2026-03-02
**Status:** Approved

## Overview

`ftai` is a Rust CLI terminal coding assistant powered by local models (llama.cpp + MLX) with a custom rules DSL for governance. It provides full tool capabilities while giving the user full control over the AI's behavior through enforceable rules.

## Architecture

Monolithic Rust binary. Single `cargo build`, no runtime dependencies.

```
ftai CLI binary
├── tui/          — ratatui terminal interface
├── conversation/ — message history, context management, tool call parsing
├── rules/        — DSL parser, evaluator, enforcement hooks
├── backend/      — llama.cpp + MLX inference (trait-based)
├── tools/        — 11 tools at full coding assistant parity
└── config/       — TOML config loading, merging
```

## Model Backend

### Trait

```rust
pub trait ModelBackend: Send + Sync {
    fn load_model(&mut self, config: &ModelConfig) -> Result<()>;
    fn generate(&self, request: &ChatRequest) -> Result<ChatResponse>;
    fn generate_stream(&self, request: &ChatRequest) -> Result<Box<dyn Stream<Item = Token>>>;
    fn supports_tool_calling(&self) -> bool;
    fn max_context_length(&self) -> usize;
}
```

### Backends

- **llama.cpp** — via `llama-cpp-rs`, loads GGUF models, Metal GPU acceleration on macOS
- **MLX** — via C FFI, loads safetensors, Apple Silicon optimized

### Model Strategy

Auto-detect hardware at first run, recommend best coding model:

| Hardware | RAM | Model | Backend | Size |
|----------|-----|-------|---------|------|
| Apple Silicon | 8GB | Qwen2.5-Coder-3B-Q4 | MLX | ~2GB |
| Apple Silicon | 16GB | Qwen2.5-Coder-7B-Q4 | MLX | ~4GB |
| Apple Silicon | 32GB+ | Qwen2.5-Coder-32B-Q4 | MLX | ~18GB |
| NVIDIA GPU | 8GB+ VRAM | DeepSeek-Coder-V2-Lite-Q4 | llama.cpp | ~9GB |
| NVIDIA GPU | 24GB+ VRAM | Qwen2.5-Coder-32B-Q4 | llama.cpp | ~18GB |
| CPU only | 16GB+ | Qwen2.5-Coder-7B-Q4 | llama.cpp | ~4GB |

Users can BYO any GGUF or safetensors model via config.

### Tool Calling

- **Native** — models with OpenAI-style function calling (Qwen, DeepSeek)
- **Prompted** — inject tool descriptions into system prompt, parse XML/JSON output
- **Hybrid** — try native, fall back to prompted

## Rules DSL

### Syntax

```ftai
rule "no-co-author" {
  on commit
  reject contains(message, "Co-Authored-By")
  reason "Never add co-author lines to commits"
}

rule "security-tests-required" {
  on commit
  when project in ["Serena", "FolkOS", "MacroAI"]
  require files_match("*RedTests*") in staged_files
  reason "FolkTech codebases require security red tests"
}

rule "block-destructive-shell" {
  on tool:bash
  reject matches(command, "rm -rf|git reset --hard|git push --force")
  unless confirmed_by_user
  reason "Destructive commands require confirmation"
}

scope "~/Developer/Serena" {
  rule "swift-conventions" {
    on tool:file_write
    when extension(path) == "swift"
    require !contains(content, "force try") && !contains(content, "force unwrap")
    reason "No force try/unwrap in Serena codebase"
  }
}
```

### Grammar Elements

| Element | Purpose |
|---------|---------|
| `rule "name" {}` | Define a named rule |
| `on <event>` | Trigger: `commit`, `pr_create`, `tool:<name>`, `response`, `session_start` |
| `when <condition>` | Optional guard condition |
| `reject <expr>` | Block if expression is true |
| `require <expr>` | Block if expression is false |
| `modify <action>` | Transform the action |
| `unless <condition>` | Override escape hatch |
| `reason "..."` | Error message on rule fire |
| `scope "path" {}` | Project-scoped rules |

### Built-in Functions

- `contains(str, pattern)`, `matches(str, regex)`
- `files_match(glob)`, `files_exist(path)`
- `extension(path)`, `dirname(path)`
- `staged_files`, `changed_files`, `project`
- `confirmed_by_user` — prompts y/n
- `line_count(path)`, `adds_lines_matching(pattern)`

### Enforcement Flow

```
Model requests tool call
  → Rules Engine pre-check → REJECT / MODIFY / ALLOW
  → Tool executes
  → Rules Engine post-check (filter output)
  → Result back to model
```

## Tool System

### Trait

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult>;
}
```

### Tools (Day 1)

| Tool | Description |
|------|-------------|
| `bash` | Execute bash commands, persistent cwd, timeout, background mode |
| `file_read` | Read files with optional line ranges |
| `file_write` | Create/overwrite files |
| `file_edit` | String replacement edits |
| `glob` | File pattern matching |
| `grep` | Regex content search (ripgrep-style) |
| `git_*` | commit, diff, log, status, branch, push, pr_create |
| `web_fetch` | HTTP GET + HTML→markdown |
| `web_search` | Web search (pluggable backend) |
| `agent_spawn` | Sub-agent with isolated context |
| `ask_user` | Prompt for input/choice |

### Permission Modes

- `ask` — prompt before every tool call
- `auto` — auto-approve reads, prompt for writes/shell
- `yolo` — auto-approve all except rules-blocked

## Terminal UI

`ratatui` + `crossterm`.

### Layout

- **Status bar** (top) — model, backend, cwd, project
- **Message stream** — markdown rendered, syntax highlighted, collapsible tool calls
- **Rule violations** — inline warnings
- **Status line** (bottom) — task count, token usage, active rules
- **Input** — multi-line, history, tab completion

### Slash Commands

- `/model <name>` — switch model
- `/rules` — show active rules
- `/rules reload` — hot-reload rules
- `/config` — show/edit config
- `/clear` — clear conversation
- `/compact` — compress context
- `/project <path>` — switch project
- `/help` — show commands

### Key Bindings

- `Enter` — submit (configurable)
- `Shift+Enter` — newline
- `Ctrl+C` — cancel generation
- `Ctrl+D` — exit
- `Esc` — cancel tool approval

## Directory Structure

### User-facing

```
~/.ftai/
├── config.toml           # Global settings
├── rules.ftai            # Global rules
├── memory/               # Persistent memory
├── models/               # Downloaded models
└── projects/             # Per-project overrides

<project>/.ftai/
├── RULES.md              # In-repo rules (team-shared)
└── config.toml           # Project defaults
```

### Rule Precedence (highest wins)

1. `<project>/.ftai/RULES.md`
2. `~/.ftai/projects/<project>/rules.ftai`
3. `~/.ftai/rules.ftai`

## Rust Project Structure

```
ftai/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── app.rs
│   ├── config/       — TOML loading, merging
│   ├── backend/      — ModelBackend trait, llamacpp.rs, mlx.rs
│   ├── conversation/ — engine.rs, prompt.rs, parser.rs
│   ├── rules/        — lexer.rs, parser.rs, evaluator.rs, builtins.rs
│   ├── tools/        — Tool trait, registry, 11 tool implementations
│   └── tui/          — app.rs, render.rs, input.rs, markdown.rs
├── tests/
│   ├── rules/
│   ├── tools/
│   └── integration/
└── build.rs
```

## Key Dependencies

```toml
ratatui, crossterm, tokio, serde, serde_json, toml, clap,
reqwest, syntect, pulldown-cmark, regex, glob, git2, llama-cpp-rs
```
