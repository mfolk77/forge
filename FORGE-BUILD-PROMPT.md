# Forge Build Prompt — Implementation Sprint

**Date:** 2026-04-01
**Working directory:** ~/Developer/forge/
**Architecture docs:** Read ALL of these before writing any code:
- `docs/plans/2026-03-27-forge-architecture.md` (main architecture)
- `docs/plans/2026-03-27-tool-calling-subsystem-design.md` (tool calling)
- `docs/plans/2026-03-31-architecture-amendments.md` (9 amendments from reference architecture analysis)
- `docs/plans/2026-03-31-agent-loop-anatomy.md` (agent loop deep-dive)

**Existing codebase:** ~/Developer/forge/ has 212 passing tests and 81 Rust source files. The modules listed as "REUSED from FTAI" already exist and work. Do NOT rewrite them. You are adding NEW modules only.

---

## What To Build

Implement the Forge architecture amendments in priority order. Use subagents for independent modules. The work breaks into 6 parallel workstreams and 3 sequential follow-ups.

---

## Phase 1 — Parallel Workstreams (use subagents)

Launch these simultaneously. They have no dependencies on each other.

### Workstream 1: Context Compaction System
**Files:** `src/session/compact.rs`, `src/session/transcript.rs`
**Tests:** `tests/session/compact_test.rs`, `tests/session/transcript_test.rs`

Build the three-tier compaction system from Amendment 1:

**Tier 1 — Microcompact:**
- Function: `micro_compact(messages: &mut Vec<Message>)`
- Replaces tool result content older than last 3 with `"[Previous: used {tool_name}]"`
- Exception: preserve `file_read` results (reference material the model needs for edits)
- Runs every loop iteration, zero model calls
- Must handle both string content and structured tool_result blocks

**Tier 2 — Snip compact:**
- Function: `snip_compact(messages: &mut Vec<Message>, keep_recent: usize) -> Vec<Message>`
- Deterministic removal of oldest messages
- Keep: system prompt (always), last N user/assistant exchanges, all tool calls from current task
- Returns removed messages (for transcript backup)
- Zero model calls

**Tier 3 — Summarize compact:**
- Function: `summarize_compact(messages: &mut Vec<Message>, backend: &dyn ModelBackend) -> Result<()>`
- Only called when snip is insufficient (>90% after snip)
- Model generates 200-token summary of removed messages
- Fallback: if model call fails, hard-truncate to last 5 exchanges

**Post-compact reinject:**
- Function: `build_reinject_context(rules: &RuleSet, skills: &SkillRegistry, tools: &ToolRegistry) -> Vec<Message>`
- After any compaction, re-append: active FTAI rules, loaded skill metadata, tool definitions, identity block
- These become system messages at the front of the compacted history

**Tier 4 — Session memory extraction (structured checkpoint):**
- Function: `extract_session_memory(messages: &[Message]) -> SessionMemory`
- Extracts structured state to `~/.ftai/sessions/<project>/<session>.memory.json`
- Schema: `{ task_description, files_touched: [], errors_encountered: [], decisions_made: [], pending_work: [], learnings: [] }`
- Written on every compact AND on session end
- Reloaded on `--continue` / `--resume` and injected as system context
- This survives ALL compaction — it's the persistent checkpoint that outlives the conversation

**Tier 5 — PTL emergency truncation:**
- Function: `ptl_truncate(messages: &mut Vec<Message>)`
- Triggered when model returns "prompt too long" error
- Drops oldest message groups (user+assistant pairs) until under budget
- Retry the model call after truncation
- Last resort — should rarely fire if Tiers 1-3 are working

**Token estimation:**
- Function: `estimate_tokens(messages: &[Message]) -> usize`
- Heuristic: `content.len() / 4` — no tokenizer needed
- Must handle all Message variants (text, tool_use, tool_result, system)

**JSONL Transcript:**
- Struct: `TranscriptWriter` with `append(&self, entry: &SessionEntry) -> Result<()>`
- Append-only JSONL at `~/.ftai/sessions/<project_hash>/<session_id>.jsonl`
- One JSON object per line: `{"seq":0,"role":"system","content":"...","ts":1711900000}`
- Function: `load_transcript(path: &Path) -> Result<Vec<Message>>` with filtering:
  - Skip `"progress"` entries (TUI-only)
  - Skip stale system context entries
- Write before any compaction (never lose data)

**Tests (minimum 15):**
- micro_compact preserves recent results
- micro_compact preserves file_read results
- micro_compact replaces old bash/edit results
- snip_compact keeps last N exchanges
- snip_compact preserves system prompt
- estimate_tokens returns reasonable values
- transcript append and load roundtrip
- transcript filtering skips progress entries
- reinject includes rules and skills
- full pipeline: micro → snip → summarize fallback
- empty message list doesn't panic
- single message doesn't get compacted
- large tool results get micro-compacted correctly
- snip with zero keep_recent keeps only system
- transcript handles concurrent appends (file locking)

---

### Workstream 2: Tool Abort Signals + Progress Callbacks
**Files:** Modify `src/tools/mod.rs`, `src/tools/bash.rs`, `src/tools/registry.rs`
**Tests:** `tests/tools/abort_test.rs`

Extend the existing `Tool` trait with cancellation and progress:

```rust
use tokio_util::sync::CancellationToken;
use tokio::sync::mpsc;

pub enum ToolProgress {
    Percent(u8),
    Status(String),
    PartialOutput(String),
}

// Add to existing Tool trait:
// - cancel: CancellationToken parameter on execute()
// - progress: mpsc::Sender<ToolProgress> parameter on execute()
// - classify_summary(&self, args: &Value) -> String (default impl provided)
```

Update `bash.rs` to:
- Stream stdout line-by-line via `ToolProgress::PartialOutput`
- Check `cancel.is_cancelled()` in the read loop
- Kill child process on cancellation
- Return `ToolResult::error("Cancelled by user")` on cancel

Update `registry.rs`:
- `execute_tool()` creates CancellationToken and progress channel
- Returns a `ToolExecution` handle with `.cancel()` method
- Progress channel is exposed for TUI consumption

Add `classify_summary()` default implementation:
- Returns `"{tool_name}({truncated_args})"` — max 100 chars
- Used by permission classifier for compact context

**Tests (minimum 8):**
- bash tool streams partial output
- bash tool cancels on token
- bash tool kills child on cancel
- file_read sends progress for large files
- classify_summary truncates correctly
- registry creates cancel token per execution
- cancel after completion is no-op
- progress channel drops cleanly when receiver gone

---

### Workstream 3: Glob-Matched Rule Loading
**Files:** Modify `src/rules/loader.rs`, add `src/rules/glob_matcher.rs`
**Tests:** `tests/rules/glob_test.rs`

Extend FTAI rule files to support YAML frontmatter with glob patterns:

```markdown
---
globs: ["**/*.rs", "**/*.toml"]
alwaysApply: false
---

rule "rust-unwrap-guard" { ... }
```

Implementation:
- Parse YAML frontmatter from rule files (regex: `^---\n(.*?)\n---\n(.*)`)
- `GlobRule` struct: `{ rule_set: RuleSet, globs: Vec<GlobPattern>, always_apply: bool }`
- `LazyRuleLoader::rules_for_context()` checks glob match against file path
- Use `glob` crate's `Pattern::matches_path()`
- `alwaysApply: true` → loaded at session start regardless of file
- `alwaysApply: false` + globs → loaded only when tool call targets matching file
- `alwaysApply: false` + no globs → existing directory-walk behavior (unchanged)

**Tests (minimum 8):**
- Parse frontmatter with globs
- Parse frontmatter without globs (backward compat)
- Parse file with no frontmatter (backward compat)
- Glob matching: `**/*.rs` matches `src/auth.rs`
- Glob matching: `**/*.rs` does NOT match `src/auth.py`
- alwaysApply rules load regardless of file context
- Multiple globs: any match triggers load
- Empty globs array treated as no-glob (backward compat)

---

### Workstream 4: Skills System
**Files:** `src/skills/mod.rs`, `src/skills/loader.rs`
**Tests:** `tests/skills/skills_test.rs`

New module — on-demand domain knowledge, separate from rules:

**SkillMeta struct:**
```rust
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub max_tokens: usize,
    pub path: PathBuf,
}
```

**SkillRegistry:**
- `new(dirs: &[PathBuf]) -> Self` — scans directories for SKILL.md files
- `get_descriptions() -> String` — one-line per skill for system prompt (~100 tokens each)
- `match_skills(text: &str) -> Vec<&SkillMeta>` — keyword trigger matching (case-insensitive)
- `load_skill(name: &str) -> Option<String>` — lazy load, cached in HashMap
- `get_content(name: &str) -> String` — returns `<skill name="...">\n{body}\n</skill>`

**Skill file format** (YAML frontmatter + markdown body):
```yaml
---
name: rust-async-patterns
description: "Async Rust patterns — tokio, channels, select!"
triggers: ["async", "tokio", "spawn", "channel", ".await"]
max_tokens: 1500
---

# Content here...
```

**Directories scanned (in order):**
1. `~/.ftai/skills/` (user skills)
2. `<project>/.ftai/skills/` (project skills)
3. `~/.ftai/plugins/<name>/skills/` (plugin skills)

**Tests (minimum 10):**
- Parse skill frontmatter correctly
- Missing frontmatter fields use defaults
- Trigger matching is case-insensitive
- Multiple triggers: any match returns the skill
- No triggers: skill never auto-matches (load by name only)
- get_descriptions formats correctly
- load_skill caches content
- load_skill returns None for unknown name
- get_content wraps in XML skill tags
- Scanner finds skills in nested directories

---

### Workstream 5: Permission Classifier Enhancement
**Files:** Modify `src/permissions/classifier.rs`
**Tests:** `tests/permissions/classifier_test.rs`

Add denial tracking and session allowlist to the existing static classifier:

**PermissionClassifier struct:**
```rust
pub struct PermissionClassifier {
    denial_streak: HashMap<String, u32>,
    session_allowlist: HashSet<String>,
}
```

Methods:
- `classify()` — checks allowlist first, then static classification, then denial escalation
- `add_to_session_allowlist(name, args)` — called when user approves with "always allow"
- `record_denial(name, args)` — increments denial streak for similar calls
- `reset_denials(name, args)` — called on approval to reset streak
- `tool_key(name, args) -> String` — stable grouping key:
  - file tools: `"{tool}:{path}"`
  - bash: `"bash:{first_two_words}"`
  - others: `"{tool}"`

Behavior:
- 3+ consecutive denials for same key → escalate to `SafetyLevel::Dangerous`
- Session allowlist bypasses all checks → `SafetyLevel::Safe`
- Allowlist is session-scoped (cleared on session end)

**Tests (minimum 8):**
- Default classification unchanged (backward compat)
- Session allowlist bypasses classification
- 3 denials escalates to Dangerous
- Approval resets denial streak
- tool_key groups file operations by path
- tool_key groups bash by command prefix
- Allowlist cleared on reset
- Denial tracking is per-key, not global

---

### Workstream 6: Hook System
**Files:** `src/hooks/mod.rs`
**Tests:** `tests/hooks/hooks_test.rs`

Event-driven shell automation:

**HookConfig struct:**
```rust
pub struct HookConfig {
    pub event: String,
    pub tool: Option<String>,  // filter to specific tool (for before_tool/after_tool)
    pub command: String,
    pub description: String,
    pub timeout_ms: u64,
}
```

**HookRunner:**
- `new(hooks: Vec<HookConfig>) -> Self`
- `load_from_config(path: &Path) -> Result<Self>` — parse from `<project>/.ftai/config.toml`
- `run(event: &str, env: &HashMap<String, String>) -> Result<()>`
  - Filters hooks by event name and optional tool filter
  - Runs each matching hook as `sh -c "{command}"` with env vars
  - Timeout via `tokio::time::timeout`
  - For blocking events (`before_tool`, `before_commit`): non-zero exit = block with stderr message
  - For non-blocking events: log errors but don't block

**Events:**
| Event | Env vars | Can block? |
|-------|----------|------------|
| `session_start` | `FORGE_PROJECT` | No |
| `session_end` | `FORGE_PROJECT`, `FORGE_SESSION_ID` | No |
| `before_tool` | `FORGE_TOOL_NAME`, `FORGE_TOOL_ARGS` | Yes |
| `after_tool` | `FORGE_TOOL_NAME`, `FORGE_TOOL_RESULT` | No |
| `after_file_edit` | `FORGE_FILE_PATH` | No |
| `before_commit` | `FORGE_COMMIT_MSG` | Yes |

**TOML config format:**
```toml
[[hooks]]
event = "after_file_edit"
command = "rustfmt $FORGE_FILE_PATH 2>/dev/null || true"
description = "Auto-format Rust files after edit"
timeout_ms = 10000
```

**Tests (minimum 10):**
- Hook runs shell command with env vars
- Hook timeout kills process
- Blocking hook stops on non-zero exit
- Non-blocking hook logs error but continues
- Tool filter matches correctly
- Multiple hooks for same event all run
- Missing command returns error
- Empty hooks list is no-op
- Load from TOML parses correctly
- Env vars are passed to subprocess

---

## Phase 2 — Sequential (after Phase 1 completes)

### Step 7: Inference Backend Fallback Chain
**Files:** Modify `src/inference/mod.rs`

Add memoized probe pattern:
- Try llama.cpp FFI → try MLX subprocess → fail with diagnostic message
- Probe once at startup, cache result in `BackendProbeResults`
- Never re-probe mid-session
- `forge doctor` command shows probe results

### Step 8: Cargo Feature Gates
**Files:** Modify `Cargo.toml`

```toml
[features]
default = ["llamacpp", "evolution", "search"]
llamacpp = ["llama-cpp-sys-2"]
mlx = []
evolution = []
search = ["fastembed", "tree-sitter"]
knowledge-sampler = []
```

Add `#[cfg(feature = "...")]` guards to: evolution/, search/, inference/knowledge_sampler.rs, inference/mlx.rs

### Step 9: Dream System
**Files:** `src/dream/mod.rs`, `src/dream/prompt.rs`, `src/dream/scheduler.rs`
**This is the big one.** Depends on: transcript system (workstream 1), skills (workstream 4), session persistence.

**DreamScheduler:**
- Runs as background task after session ends (or on cron if session is long-lived)
- Gate: ≥24 hours since last dream AND ≥3 sessions since last dream
- Lock file prevents concurrent dreams

**DreamPrompt (4 phases + ideation):**
```
Phase 1 — Orient
  Read memory directory, existing rules, skill list

Phase 2 — Gather signal
  Read session transcripts since last dream
  Identify: what was the user working on? What were they stuck on?
  What patterns repeated? What errors occurred?

Phase 3 — Consolidate memory
  Update/create memory files, resolve contradictions, prune stale

Phase 4 — Ideate
  For each unresolved problem from recent sessions:
  - Read the relevant source files
  - Analyze the error or blocker
  - Explore alternative approaches (grep for patterns, read related code)
  - Write findings to .ftai/dreams/{date}-{topic}.md

Phase 5 — Prepare
  Write a dream summary to .ftai/dreams/latest.md
  Include: what was consolidated, what problems were analyzed,
  what solutions are ready to propose
```

**Dream output format** (`.ftai/dreams/2026-04-01-auth-fix.md`):
```markdown
---
session_refs: ["session_abc123", "session_def456"]
topic: "Token refresh failing on v2 API"
confidence: high
status: ready
---

## Problem
User was stuck on token refresh in src/auth/refresh.rs.
Error: "invalid_grant" from /oauth/token endpoint.

## Analysis
- Current code uses v1 API endpoint (/oauth/token)
- v2 endpoint is /oauth/token/refresh with different payload
- Found working pattern in src/auth/initial_auth.rs:42

## Proposed Fix
Replace the refresh endpoint and update the request body:
- File: src/auth/refresh.rs
- Old: POST /oauth/token with grant_type=refresh_token
- New: POST /oauth/token/refresh with { refresh_token, client_id }

## Ready to implement?
Yes — say "implement the auth fix" to apply.
```

**Session start injection:**
- On new session, check `.ftai/dreams/latest.md`
- If exists and <48h old, inject as system context:
  `"[Dream results available — I analyzed problems from your recent sessions. Key findings: {summary}. Say 'show dreams' for details or 'implement {topic}' to apply.]"`

**Mitosis integration:**
- Dream Phase 2 feeds session patterns to Mitosis analyzer
- If Mitosis generates a new rule, Dream Phase 3 writes it to `~/.ftai/evolution/rules/`

**Bash constraints during dream:**
- Read-only: `ls`, `find`, `grep`, `cat`, `stat`, `wc`, `head`, `tail`
- Block writes, redirects, state modifications
- File edits allowed (for memory files in `.ftai/` only)

**Tests (minimum 12):**
- Dream scheduler respects time gate
- Dream scheduler respects session count gate
- Dream lock prevents concurrent execution
- Dream prompt includes recent session summaries
- Dream output writes correctly formatted markdown
- Session start injects dream results
- Old dream results (>48h) not injected
- Bash read-only constraint blocks writes
- File edits allowed in .ftai/ directory
- File edits blocked outside .ftai/ during dream
- Mitosis integration passes patterns correctly
- Dream summary truncated if too long

---

## Phase 3 — Integration

### Step 10: Wire the Orchestrator
**Files:** Modify the main orchestrator (likely `src/conversation/engine.rs` or equivalent)

The pre-LLM-call checklist — this is the order of operations:

```rust
pub async fn agent_loop(ctx: &mut AgentContext) -> Result<()> {
    loop {
        // 1. Drain background notifications
        let notifs = ctx.background.drain();
        if !notifs.is_empty() {
            ctx.messages.push(Message::background_results(notifs));
        }
        
        // 2. Run hooks (before_tool handled per-tool, this is loop-level)
        ctx.hooks.run("before_model_call", &ctx.env()).await.ok();
        
        // 3. Micro-compact (silent, every turn)
        ctx.compactor.micro_compact(&mut ctx.messages);
        
        // 4. Check token budget
        if ctx.compactor.should_compact(&ctx.messages) {
            // Save transcript before compacting
            ctx.transcript.flush(&ctx.messages).await?;
            // Snip first (no model call)
            ctx.compactor.snip_compact(&mut ctx.messages, 3);
            // If still too big, summarize
            if ctx.compactor.should_compact(&ctx.messages) {
                ctx.compactor.summarize_compact(&mut ctx.messages, &ctx.backend).await?;
            }
            // Reinject identity, rules, skills, tools
            let reinject = ctx.build_reinject_context();
            ctx.messages.insert(0, reinject);
        }
        
        // 5. Match and inject relevant skills
        if let Some(last_user_msg) = ctx.messages.last_user_text() {
            for skill in ctx.skills.match_skills(last_user_msg) {
                let content = ctx.skills.load_skill(&skill.name);
                if let Some(content) = content {
                    ctx.messages.push(Message::system(content));
                }
            }
        }
        
        // 6. Build request and call model
        let request = build_request(&ctx.messages, &ctx.tools, &ctx.rules);
        let response = ctx.backend.generate_stream(&request, ctx.token_tx.clone()).await?;
        ctx.messages.push(response.into_message());
        ctx.transcript.append(&response).await?;
        
        // 7. Check stop condition
        if response.stop_reason != StopReason::ToolUse {
            return Ok(());
        }
        
        // 8. Execute tools with cancellation + progress + hooks
        let mut results = Vec::new();
        for tool_call in response.tool_calls() {
            // Pre-tool hook
            let env = tool_call_env(&tool_call);
            if let Err(blocked) = ctx.hooks.run("before_tool", &env).await {
                results.push(ToolResult::error(format!("Blocked by hook: {}", blocked)));
                continue;
            }
            
            // Permission check
            let safety = ctx.permissions.classify(&tool_call.name, &tool_call.args, &ctx.tool_ctx);
            match safety {
                SafetyLevel::Dangerous | SafetyLevel::Moderate => {
                    if !ctx.tui.prompt_approval(&tool_call).await {
                        ctx.permissions.record_denial(&tool_call.name, &tool_call.args);
                        results.push(ToolResult::error("Rejected by user"));
                        continue;
                    }
                }
                SafetyLevel::Safe => {}
            }
            
            // Execute with cancel token + progress
            let (cancel, progress_rx) = ctx.tools.create_execution_context();
            ctx.tui.stream_progress(progress_rx);
            let result = ctx.tools.execute(&tool_call, cancel).await;
            
            // Post-tool hook
            ctx.hooks.run("after_tool", &tool_result_env(&tool_call, &result)).await.ok();
            
            // after_file_edit hook
            if tool_call.name == "file_edit" || tool_call.name == "file_write" {
                if let Some(path) = tool_call.args.get("path") {
                    let mut env = HashMap::new();
                    env.insert("FORGE_FILE_PATH".into(), path.to_string());
                    ctx.hooks.run("after_file_edit", &env).await.ok();
                }
            }
            
            results.push(result);
        }
        
        ctx.messages.push(Message::tool_results(results));
        ctx.transcript.append_tool_results(&results).await?;
        
        // 9. Log for evolution engine
        ctx.evolution.record_tool_calls(&response);
    }
}
```

---

## Rules

1. Read ALL architecture docs before writing any code
2. Every module must have tests — no exceptions
3. Errors from tools are tool results, not panics. Use `Result` internally but convert to string at the message boundary.
4. The agent loop itself must stay simple — 10 lines of core logic. Everything else is pre/post processing.
5. Don't rewrite existing modules (rules/, tools/, permissions/, config/, conversation/, tui/, formatting/, plugins/). Extend them.
6. Follow existing code patterns — check how existing tools implement the `Tool` trait before modifying it
7. Run `cargo test` after each workstream. All 212 existing tests must still pass.
8. Run `cargo clippy` — zero warnings.
9. Use `anyhow::Result` for fallible functions, `thiserror` for custom error types.
10. Security: `safe_path()` validation on ALL file operations. Dream bash is read-only. Hook commands run with timeout.
11. FTAI.md files are re-read EVERY turn (stat check, re-read if mtime changed). User can edit mid-session.
12. Read-only tools (file_read, grep, glob, list_dir, search_semantic) execute concurrently. Mutating tools (file_edit, file_write, bash) execute serially. Use `tokio::join!` for read batches.

---

## Appendix: Reference Architecture Patterns (final extraction)

### A. Summarize Compact Prompt Structure

When Tier 3 (summarize compact) runs, use this 9-category structure in the summarization prompt:

```
Summarize this conversation for continuity. Preserve these categories in order:

1. Primary Request and Intent — the user's explicit requests in detail
2. Key Technical Concepts — technologies, frameworks, patterns discussed
3. Files and Code Sections — specific files examined or modified, with key snippets
4. Errors and Fixes — all errors encountered and how they were resolved
5. Problem Solving — problems solved and ongoing troubleshooting
6. User Messages — capture the user's own words for important instructions
7. Pending Tasks — explicitly assigned work that remains incomplete
8. Current Work — precisely what was happening immediately before this summary
9. Next Step — the single next action in line with the user's most recent request

Be concise but preserve critical technical details. Output only the summary.
```

### B. System Prompt Additions

Add these to `build_system_prompt()`:

**False-claims mitigation:**
```
Report outcomes faithfully: if tests fail, say so with the relevant output; if you
did not run a verification step, say that rather than implying it succeeded. Never
claim "all tests pass" when output shows failures. Equally, when a check did pass,
state it plainly — do not hedge confirmed results with unnecessary disclaimers.
```

**Thoroughness anchor:**
```
Before reporting a task complete, verify it actually works: run the test, execute
the script, check the output. If you can't verify (no test exists, can't run the
code), say so explicitly rather than claiming success.
```

**Output efficiency (for local models):**
```
Go straight to the point. Lead with the action, not the reasoning. Skip filler
words and preamble. Do not narrate each step or list every file you read.
If you can say it in one sentence, don't use three.
```

### C. Tool Result Persistence (replaces truncation)

Instead of truncating large tool results, persist to disk and replace with reference:

```rust
// src/tools/result_storage.rs

const MAX_RESULT_CHARS: usize = 50_000;
const PREVIEW_CHARS: usize = 500;

pub fn maybe_persist_result(result: &str, tool_name: &str) -> String {
    if result.len() <= MAX_RESULT_CHARS {
        return result.to_string();
    }
    
    // Persist full result to disk
    let path = persist_to_disk(result, tool_name);
    let preview = &result[..PREVIEW_CHARS.min(result.len())];
    
    format!(
        "<persisted-output>\nOutput too large ({} chars). Full output saved to: {}\n\nPreview (first {} chars):\n{}\n</persisted-output>",
        result.len(), path.display(), preview.len(), preview
    )
}
```

The model gets a preview to decide if it needs more. It can `file_read` the persisted file if needed. No data loss, no re-execution.

### D. Token Budget Enforcement

Add to the orchestrator loop:

```rust
const COMPLETION_THRESHOLD: f32 = 0.9;   // 90% = consider stopping
const DIMINISHING_THRESHOLD: usize = 500; // <500 new tokens/turn = diminishing returns
const MAX_DIMINISHING_CONTINUATIONS: u32 = 3;

struct TokenBudgetTracker {
    continuation_count: u32,
    last_delta_tokens: usize,
}

impl TokenBudgetTracker {
    fn should_stop(&mut self, turn_tokens: usize, budget: usize) -> bool {
        let pct = turn_tokens as f32 / budget as f32;
        if pct >= COMPLETION_THRESHOLD {
            return true;
        }
        if self.last_delta_tokens > 0 && self.last_delta_tokens < DIMINISHING_THRESHOLD {
            self.continuation_count += 1;
            if self.continuation_count >= MAX_DIMINISHING_CONTINUATIONS {
                return true;
            }
        } else {
            self.continuation_count = 0;
        }
        self.last_delta_tokens = turn_tokens;
        false
    }
}
```

When the budget tracker says stop, inject: `"Stopped at {pct}% of token target. Keep working — do not summarize."`

### E. Conversation Recovery (Session Resume)

When loading a transcript for resume, filter these before replaying:

```rust
pub fn filter_for_resume(messages: Vec<Message>) -> Vec<Message> {
    messages.into_iter().filter(|msg| {
        // Skip orphaned tool_results (no matching tool_use in prior assistant msg)
        // Skip whitespace-only assistant messages (streaming artifacts)  
        // Skip thinking-only assistant messages (can confuse local models)
        // Skip stale system context injections
        match msg.role {
            Role::Assistant => !msg.is_empty() && !msg.is_thinking_only(),
            Role::System if msg.is_stale_context() => false,
            _ => true,
        }
    }).collect()
}

// If conversation ends on user message (not tool_result), append:
// "Continue from where you left off."
```

### F. KV Cache Reuse for Subagents

When spawning a subagent with the same system prompt prefix as the parent, reuse KV cache:

```rust
impl LlamaContext {
    /// Fork the KV cache for a subagent. The subagent shares cached tokens
    /// for the system prompt prefix, avoiding re-computation.
    pub fn fork_kv_cache(&self, shared_prefix_tokens: usize) -> Result<Self> {
        // Create new context sharing the model
        // Copy KV cache entries [0..shared_prefix_tokens]
        // Subagent starts decoding from shared_prefix_tokens onward
    }
}
```

This makes subagent startup near-instant for the shared prefix (system prompt + tool definitions + rules). Only the subagent's unique prompt needs new inference.

### G. Git Context Injection

Add to system prompt assembly:

```rust
pub fn build_git_context(project_root: &Path) -> Option<String> {
    let git_root = find_git_root(project_root)?;
    let branch = read_head_branch(&git_root)?;
    let is_dirty = !Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .status().ok()?.success();
    
    let diff_stats = if is_dirty {
        // git diff HEAD --numstat (cheap, no content)
        let output = Command::new("git")
            .args(["diff", "HEAD", "--shortstat"])
            .output().ok()?;
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        "clean".to_string()
    };
    
    Some(format!(
        "Git: branch={branch}, status={diff_stats}, root={}", 
        git_root.display()
    ))
}
```

Injected as environment context in the system prompt. Cheap (one git command), high value (model knows what branch it's on and whether there are uncommitted changes).
