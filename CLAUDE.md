# FTAI

## Build & Test
- `cargo build` — build debug binary
- `cargo test` — 212 tests (182 lib + 30 integration)
- `cargo install --path . --root ~/.local --force` — install globally as `ftai`
- Binary location: `~/.local/bin/ftai`

## Architecture
- Rust monolithic binary, TUI via ratatui + crossterm
- Modules: backend, config, conversation, formatting, permissions, plugins, rules, tools, tui
- Config precedence: defaults → ~/.ftai/config.toml → ~/.ftai/projects/<encoded>/config.toml → <project>/.ftai/config.toml
- System prompt order: identity → FTAI.md → tools → rules → memory → formatting → project rules → plugin skills

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
