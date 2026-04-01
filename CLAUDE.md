# Forge (formerly FTAI)

## Build & Test
- `cargo build` — build debug binary
- `cargo test` — 212 tests (182 lib + 30 integration)
- `cargo install --path . --root ~/.local --force` — install globally as `forge`
- Binary location: `~/.local/bin/forge`

## Architecture
- Rust monolithic binary, TUI via ratatui + crossterm
- Existing modules: backend, config, conversation, formatting, permissions, plugins, rules, tools, tui
- New modules (planned): inference, search, evolution, session, skills, hooks
- Config precedence: defaults → ~/.ftai/config.toml → ~/.ftai/projects/<encoded>/config.toml → <project>/.ftai/config.toml
- System prompt order: identity → FTAI.md → tools → rules → memory → formatting → project rules → plugin skills → active skills (on-demand)

## Architecture Docs
- Main: `docs/plans/2026-03-27-forge-architecture.md`
- Tool calling: `docs/plans/2026-03-27-tool-calling-subsystem-design.md`
- Claude Code amendments: `docs/plans/2026-03-31-claude-code-lessons-learned.md`
- Agent loop deep-dive: `docs/plans/2026-03-31-agent-loop-anatomy.md`

## Agent Loop Design (from learn-claude-code analysis)
- Loop is stateless — exits on `stop_reason != tool_use`, model decides when to stop
- Tool results are user messages — conversation always alternates user/assistant
- Errors are tool results — never crash from tool error, let model decide next action
- Pre-LLM-call checklist: drain background → micro_compact → check budget → reinject identity → call model
- Subagents get fresh `Vec<Message>`, restricted tool set, return summary only
- Persistent state (tasks, team) lives on disk, survives context compaction
- Background execution: fire-and-forget spawn, drain notification queue at top of loop

## Context Management (from CC analysis)
- 32K context window, ~26K available for conversation after system overhead
- Three-tier compaction: microcompact (truncate tool results) → snip (deterministic old message removal) → summarize (emergency model call)
- Post-compaction reinject: FTAI rules, active skills, tool definitions re-attached after snip
- All tools support CancellationToken (Ctrl+C abort) and progress callbacks

## Session Storage
- Live transcripts: JSONL append-only at `~/.ftai/sessions/<project>/<session>.jsonl`
- Cross-session analytics: SQLite at `~/.ftai/evolution/evolution.db` (feeds Mitosis)
- Resume filtering: skip progress/stale-context entries on replay

## Skills vs Rules
- Rules: always-on or glob-matched, small (<200 tokens), enforce conventions via FTAI DSL
- Skills: on-demand via trigger keywords, can be large (500-2000 tokens), markdown with YAML frontmatter
- Skills loaded lazily — only metadata at session start, full content when triggers match

## Model System
- Models stored in `~/.ftai/models/<name>/`
- MLX (safetensors): `find_model_file()` returns directory path, not individual shards
- GGUF: returns the .gguf file path
- Active model: Qwen3.5-35B-A3B-4bit (MoE, 3B active, MLX backend)
- Hardware detection in `src/backend/types.rs` — `HardwareInfo::detect()` and `recommended_model()`

## Plugin System
- Plugins in `~/.ftai/plugins/<name>/plugin.toml`
- Plugin tools namespaced as `plugin:<name>:<tool>` — go through full permission pipeline
- Plugin names: alphanumeric/hyphen/underscore only (sanitized in manifest.rs)
- Tool bridge: 30s timeout, hooks: 10s timeout
- `tempfile` is dev-dependency only — use manual tmp dirs in non-test code

## Gotchas
- ~40 dead code warnings expected — backend types defined but inference loop not yet wired
- `build_system_prompt()` has 8 params — update all call sites when changing signature (including tests/integration.rs)
- Old Python `ftai` may exist at `~/.local/bin/ftai` via pipx — `cargo install --force` overwrites it
- TUI requires real terminal — tests that call `app.run()` will fail without TTY

## Security (FolkTech Secure Coding Standard)
- Every code change requires security red tests — P0: input injection, path traversal, auth bypass, LLM output injection
- Plugin manifest: path traversal blocked (`..`, absolute paths), name injection sanitized
- Hook/tool commands validated against canonical paths before execution
