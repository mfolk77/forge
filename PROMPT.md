# Forge Build Sprint

You are building Forge, a local-first AI coding assistant in Rust. The codebase already exists at ~/Developer/forge/ with 81 source files, 212 passing tests, and 14 working tools. You are adding NEW modules only — do not rewrite existing code.

## Step 1: Read these docs first (mandatory, do not skip)

1. `docs/plans/2026-03-27-forge-architecture.md` — full architecture (read sections 1-10)
2. `docs/plans/2026-03-31-architecture-amendments.md` — 9 amendments with Rust code designs
3. `docs/plans/2026-03-31-agent-loop-anatomy.md` — agent loop patterns
5. `FORGE-BUILD-PROMPT.md` — full implementation spec with struct definitions, test requirements, and appendices

Read ALL five before writing any code. The implementation spec in FORGE-BUILD-PROMPT.md is the source of truth — it has exact struct definitions, function signatures, test lists, and the orchestrator wiring code.

## Step 2: Build Phase 1 — six parallel workstreams

Launch these as parallel subagents. They have zero dependencies on each other.

### Agent 1: Context Compaction System
Build `src/session/compact.rs` and `src/session/transcript.rs`. Five-tier compaction: microcompact (replace old tool results), snip (deterministic removal), summarize (model-generated summary), session memory extraction (structured JSON checkpoint to disk), PTL emergency truncation (drop oldest on prompt-too-long). Plus JSONL transcript writer with append-only writes and filtered resume loading. Use the 9-category summary prompt structure from FORGE-BUILD-PROMPT.md Appendix A. Minimum 15 tests.

### Agent 2: Tool Abort Signals + Progress Callbacks
Modify `src/tools/mod.rs`, `src/tools/bash.rs`, `src/tools/registry.rs`. Add `CancellationToken` and `mpsc::Sender<ToolProgress>` to the `Tool` trait's `execute()` method. Update bash tool to stream stdout line-by-line via PartialOutput and kill child process on cancellation. Add `classify_summary()` default method. Add concurrency metadata: read-only tools (file_read, grep, glob, list_dir, search_semantic) run concurrently via `tokio::join!`, mutating tools (file_edit, file_write, bash) run serially. Minimum 8 tests. All 212 existing tests must still pass after trait changes.

### Agent 3: Glob-Matched Rule Loading
Modify `src/rules/loader.rs`, add `src/rules/glob_matcher.rs`. Parse YAML frontmatter from rule files (`---\nglobs: ["**/*.rs"]\nalwaysApply: false\n---`). `alwaysApply: true` loads at session start. `alwaysApply: false` with globs loads only when tool call targets a matching file. Backward compatible — files without frontmatter work unchanged. Minimum 8 tests.

### Agent 4: Skills System
New module `src/skills/mod.rs` and `src/skills/loader.rs`. SkillMeta struct with name, description, triggers, max_tokens, path. SkillRegistry scans `~/.ftai/skills/`, `<project>/.ftai/skills/`, `~/.ftai/plugins/<name>/skills/` for SKILL.md files with YAML frontmatter. Two-layer injection: metadata in system prompt (~100 tokens/skill), full body loaded on demand via `load_skill()` and wrapped in `<skill name="...">` XML tags. Trigger matching is case-insensitive keyword search. Minimum 10 tests.

### Agent 5: Permission Classifier Enhancement
Modify `src/permissions/classifier.rs`. Add `PermissionClassifier` struct with `denial_streak: HashMap<String, u32>` and `session_allowlist: HashSet<String>`. 3+ consecutive denials for same tool_key escalates to Dangerous. Session allowlist bypasses all checks. `tool_key()` groups by path for file tools, by first two words for bash, by name for others. Minimum 8 tests.

### Agent 6: Hook System
New module `src/hooks/mod.rs`. HookConfig struct with event, optional tool filter, command, description, timeout_ms. HookRunner loads from `<project>/.ftai/config.toml` `[[hooks]]` sections. Events: session_start, session_end, before_tool (blocking), after_tool, after_file_edit, before_commit (blocking). Runs hooks as `sh -c "{command}"` with env vars (FORGE_PROJECT, FORGE_TOOL_NAME, FORGE_TOOL_ARGS, etc). Timeout via `tokio::time::timeout`. Non-zero exit on blocking events = block with stderr message. Minimum 10 tests.

## Step 3: Build Phase 2 — sequential (after Phase 1)

### Step 7: Inference Backend Fallback Chain
Modify `src/inference/mod.rs`. Memoized probe: try llama.cpp FFI → try MLX subprocess → fail with diagnostic. Probe once at startup, cache in `BackendProbeResults`, never re-probe. Add `forge doctor` command.

### Step 8: Cargo Feature Gates
Modify `Cargo.toml`. Features: `default = ["llamacpp", "evolution", "search"]`, optional: `mlx`, `knowledge-sampler`. Add `#[cfg(feature = "...")]` guards to evolution/, search/, inference/knowledge_sampler.rs, inference/mlx.rs.

### Step 9: Dream System
New module `src/dream/mod.rs`, `src/dream/prompt.rs`, `src/dream/scheduler.rs`. Runs after session end when: ≥24h since last dream AND ≥3 sessions since last. Lock file prevents concurrent dreams. Five-phase prompt: orient (read memory + rules), gather (scan transcripts for problems/patterns), consolidate (update memory files), ideate (analyze unresolved problems, explore code, write findings to `.ftai/dreams/{date}-{topic}.md`), prepare (write summary to `.ftai/dreams/latest.md`). Bash is read-only during dreams. File edits allowed only in `.ftai/` directory. On next session start, inject dream results if <48h old. Feed patterns to Mitosis evolution engine. Minimum 12 tests.

## Step 4: Build Phase 3 — integration

### Step 10: Wire the Orchestrator
The pre-LLM-call checklist — this is the exact order of operations in the main loop. Full code is in FORGE-BUILD-PROMPT.md under "Step 10: Wire the Orchestrator". Key points:
1. Drain background notifications
2. Run before_model_call hook
3. Re-read FTAI.md files if mtime changed (hot reload every turn)
4. Microcompact
5. Check token budget → snip → summarize if needed → reinject identity/rules/skills
6. Match and inject relevant skills by trigger keywords
7. Build request and call model (streaming)
8. If stop_reason != tool_use → return
9. Execute tools: before_tool hook → permission check → execute (concurrent reads, serial writes) → after_tool hook → after_file_edit hook
10. Append results, log for evolution engine, loop

## Rules

1. Read ALL architecture docs before writing any code
2. Every module must have tests — minimum counts specified per workstream
3. Errors from tools are tool results, not panics — convert `Result::Err` to string at the message boundary
4. The agent loop stays simple — state belongs in messages or external managers, never in the loop
5. Don't rewrite existing modules — extend them
6. `cargo test` after each workstream — all 212 existing tests must still pass
7. `cargo clippy` — zero warnings
8. `anyhow::Result` for fallible functions, `thiserror` for custom error types
9. `safe_path()` validation on ALL file operations
10. FTAI.md re-read every turn (stat check, re-read if mtime changed)
11. Read-only tools concurrent, mutating tools serial
12. Tool result persistence: large results (>50KB) saved to disk with 500-char preview, not truncated
13. System prompt includes false-claims mitigation and thoroughness anchor from Appendix B
14. Add git context to system prompt (branch, dirty status) — Appendix G

Start with Phase 1. Launch all six workstreams as parallel subagents.
