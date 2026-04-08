# Forge

## Build & Test
- `cargo build` — build debug binary
- `cargo test` — 1,226 tests (1,115 lib + 111 integration)
- `cargo install --path . --root ~/.local --force` — install globally as `forge`
- Binary location: `~/.local/bin/forge`
- After ANY code change, always reinstall with the cargo install command above

## Architecture
- Rust monolithic binary, TUI via ratatui + crossterm
- 107 source files across 17 modules
- All modules: backend, config, conversation, dream, evolution, formatting, hooks, inference, permissions, plugins, rules, search, session, skills, tools, tui, update
- Config precedence: defaults → ~/.ftai/config.toml → ~/.ftai/projects/<encoded>/config.toml → <project>/.ftai/config.toml
- System prompt order: identity → FTAI.md → tools → rules → memory → formatting → project rules → plugin skills → active skills (on-demand)

## Architecture Docs
- Main: `docs/plans/2026-03-27-forge-architecture.md`
- Tool calling: `docs/plans/2026-03-27-tool-calling-subsystem-design.md`
- Architecture amendments: `docs/plans/2026-03-31-architecture-amendments.md`
- Agent loop deep-dive: `docs/plans/2026-03-31-agent-loop-anatomy.md`

## CLI Commands
- `forge` — start interactive TUI (default)
- `forge --resume` / `forge -r` — resume last conversation
- `forge init` — scaffold .ftai/ + FTAI.md in current project
- `forge doctor` — check backends, hardware, config
- `forge model list/install/use/info` — manage local models
- `forge plugin list/search/install/uninstall/info` — manage plugins
- `forge plugin marketplace add/list` — manage marketplace sources
- `forge update` — self-update from GitHub Releases
- `forge update --check` — check for updates without installing
- `forge config show/edit` — view/edit config

## TUI Slash Commands
- `/plugin` — interactive plugin browser (Discover/Installed/Marketplaces tabs)
- `/skill` — interactive skill browser with token estimates
- `/theme` — theme picker
- `/chat` / `/code` — switch modes
- `/model` — show current model info
- `/context init` — create FTAI.md
- `/clear` / `/compact` — manage context
- `/quit` / `/exit` — exit

## Agent Loop Design
- Loop is stateless — exits on `stop_reason != tool_use`, model decides when to stop
- Tool results are user messages — conversation always alternates user/assistant
- Errors are tool results — never crash from tool error, let model decide next action
- Pre-LLM-call checklist: drain background → micro_compact → check budget → reinject identity → call model
- Subagents get fresh `Vec<Message>`, restricted tool set, return summary only
- Chat mode: no tool definitions sent (faster prefill), skips compaction
- Streaming enabled on first turn, sync generate for continuations

## Context Management
- 32K context window, ~26K available for conversation after system overhead
- Three-tier compaction: microcompact (truncate tool results) → snip (deterministic old message removal) → summarize (emergency model call)
- Post-compaction reinject: FTAI rules, active skills, tool definitions re-attached after snip
- All tools support CancellationToken (Ctrl+C abort) and progress callbacks

## Session Persistence
- Messages saved to SQLite at `~/.ftai/sessions.db`
- Sessions auto-start on TUI launch, auto-end on quit
- `forge --resume` reloads last session's messages into engine + display
- `SessionManager` in `src/session/manager.rs` — start, end, save_message, resume_latest, list_recent

## Built-in Plugins (5 plugins, 38 skills)
- Auto-scaffold on first run via `ensure_builtin_plugins()` in `src/plugins/builtins.rs`
- Called from `ensure_ftai_dirs()` and Plugin CLI handler
- folktech-dev-toolkit (6 skills): /secure, /tdd, /perf, /audit-dep, /doc, /quality
- forge-superpowers (11 skills): /brainstorm, /plan, /execute, /tdd-workflow, /debug, /review, /review-feedback, /parallel, /subagent-dev, /verify, /finish
- forge-dev-tools (10 skills): /changelog, /frontend, /organize, /research, /webapp-test, /mcp, /create-skill, /audit-config, /enhance-image, /comms
- forge-document-tools (6 skills): /pdf, /docx, /xlsx, /pptx, /canvas, /markdown
- forge-plugin-dev (5 skills): /plugin-dev, /skill-dev, /hook-dev, /command-dev, /agent-dev

## Plugin System
- Plugins in `~/.ftai/plugins/<name>/plugin.toml`
- Plugin tools namespaced as `plugin:<name>:<tool>` — go through full permission pipeline
- Plugin names: alphanumeric/hyphen/underscore only (sanitized in manifest.rs)
- Tool bridge: 30s timeout, hooks: 10s timeout
- Marketplace registry in `src/plugins/registry.rs` — supports both Forge (plugin.toml) and CC (.claude-plugin/plugin.json) formats
- `tempfile` is dev-dependency only — use manual tmp dirs in non-test code

## Skills vs Rules
- Rules: always-on or glob-matched, small (<200 tokens), enforce conventions via FTAI DSL
- Skills: on-demand via trigger keywords, can be large (500-2000 tokens), markdown with YAML frontmatter
- Skills loaded lazily — only metadata at session start, full content when triggers match

## Model System
- Models stored in `~/.ftai/models/<name>/`
- MLX (safetensors): `find_model_file()` returns directory path, not individual shards
- GGUF: returns the .gguf file path
- Active model: Qwen3.5-9B-4bit (dense 9B, ~6GB disk, ~7-8GB RAM, MLX backend)
- Hardware detection in `src/backend/types.rs` — `HardwareInfo::detect()` and `recommended_model()`
- MLX server logs to `~/.ftai/mlx-server.log` for crash debugging
- Prompt cache capped by RAM: 8192 on 16GB, 16384 on 32GB (prevents Metal OOM)
- MLX is macOS Apple Silicon only — `is_available()` returns false on other platforms

## Self-Update
- `forge update` downloads latest release from GitHub Releases (mfolk77/forge)
- Uses `self_update` crate with `spawn_blocking` (blocking HTTP inside async runtime)
- GitHub Actions workflow at `.github/workflows/release.yml` builds 4 targets on tag push:
  macOS arm64, macOS x86_64, Linux x86_64, Windows x86_64
- To publish: `git tag v0.2.0 && git push origin v0.2.0`

## Windows Parity
- Hooks use `cmd.exe /C` on Windows (not `sh -c`)
- Validator falls back to brace-balance check for shell (no `bash -n` on Windows)
- File tools use `Path::is_absolute()` (handles `C:\` paths)
- Dangerous command detection includes: `rd /s /q`, `format C:`, `Remove-Item -Recurse`, `IEX`, `reg delete`
- llama.cpp uses `where` on Windows, `which` on Unix
- MLX skipped entirely on non-Apple Silicon

## Security (FolkTech Secure Coding Standard)
- Every code change requires security red tests — P0: input injection, path traversal, auth bypass, LLM output injection
- Plugin manifest: path traversal blocked (`..`, absolute paths), name injection sanitized
- Hook/tool commands validated against canonical paths before execution
- Built-in security rules in `src/rules/builtins.rs`: `check_dangerous_command()`, `scan_for_secrets()`
- Built-in hook prompts: confidence gate, TDD reminder, perf check, mental model checkpoint

## Gotchas
- Zero warnings — test-only modules use `#[allow(dead_code)]` on `mod` declarations, don't remove those
- `build_system_prompt()` has 8 params — update all call sites when changing signature (including tests/integration.rs)
- TUI requires real terminal — tests that call `app.run()` will fail without TTY
- MLX on 16GB: use 9B or smaller dense models. 14B+ dense models will Metal OOM crash.
- After editing src/, always `cargo install --path . --root ~/.local --force` to update the live binary

## Git & GitHub
- Remote: `origin` → `https://github.com/mfolk77/forge.git`
- Branch: `main`
- No AI attribution in commits or PRs
- No "Co-Authored-By" lines
