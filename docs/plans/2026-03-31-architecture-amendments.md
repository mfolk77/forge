# Forge Architecture Update: Lessons from Modern AI Coding Assistants

**Date:** 2026-03-31
**Status:** Approved amendments to `2026-03-27-forge-architecture.md`
**Source:** Analysis of modern AI coding assistant architectures:
1. Open-source plugin ecosystems and community config toolkits
2. Reference implementations of agent loop patterns
3. Decompiled and open-source AI coding assistant internals

---

## Summary of Changes

This document specifies concrete architecture amendments to Forge based on analysis of modern AI coding assistants. Changes are organized by the section they amend in the main architecture doc.

---

## Amendment 1: Context Compaction System (amends Section 7.3)

### Problem

Section 7.3 defines a `TokenBudget` with `should_compact()` returning true at 80% usage, but never defines **what compaction does**. The current `truncate_tool_result()` only handles individual tool results. There is no strategy for when the full conversation history exceeds the budget.

A 200K token context window still needs aggressive three-tier compaction. Forge has 32K. This is survival, not optimization.

### Design: Three-Tier Compaction

```
Tier 1: Microcompact (within a turn)
  Trigger: tool result exceeds 2,000 tokens
  Action: truncate middle, keep first/last N lines
  Cost: zero model calls

Tier 2: Snip compact (between turns)
  Trigger: conversation_tokens > 80% of available budget
  Action: deterministic removal of oldest messages, keeping:
    - System prompt (always)
    - Last 3 user/assistant exchanges (recent context)
    - All tool calls from current task (continuity)
    - FTAI rules + memory (reinjected post-snip)
  Cost: zero model calls

Tier 3: Summarize compact (emergency)
  Trigger: snip compact insufficient (conversation still > 90% after snip)
  Action: model generates a 200-token summary of snipped messages
  Cost: one model call (~500 input tokens + 200 output)
  Fallback: if model call fails, hard-truncate to last 5 exchanges
```

### Implementation

Add to `src/session/compact.rs`:

```rust
pub enum CompactStrategy {
    /// Truncate individual tool results (existing behavior)
    Microcompact { max_result_tokens: usize },
    /// Deterministic removal of old messages
    Snip { keep_recent_exchanges: usize },
    /// Model-generated summary of removed messages  
    Summarize { summary_max_tokens: usize },
}

pub struct ContextCompactor {
    budget: TokenBudget,
    strategy_chain: Vec<CompactStrategy>,
}

impl ContextCompactor {
    pub fn new(budget: TokenBudget) -> Self {
        Self {
            budget,
            strategy_chain: vec![
                CompactStrategy::Microcompact { max_result_tokens: 2000 },
                CompactStrategy::Snip { keep_recent_exchanges: 3 },
                CompactStrategy::Summarize { summary_max_tokens: 200 },
            ],
        }
    }

    /// Compact messages to fit within budget. Returns (compacted_messages, reinject_context).
    /// reinject_context contains FTAI rules + memory that must be re-appended after compaction.
    pub fn compact(
        &self,
        messages: &[Message],
        conversation_tokens: usize,
    ) -> (Vec<Message>, Vec<Message>) {
        let available = self.budget.available_for_conversation();

        if conversation_tokens <= available * 80 / 100 {
            return (messages.to_vec(), vec![]);
        }

        // Tier 2: Snip
        let (snipped, removed) = self.snip(messages);
        let snipped_tokens = estimate_tokens(&snipped);

        if snipped_tokens <= available * 90 / 100 {
            let reinject = self.build_reinject_context();
            return (snipped, reinject);
        }

        // Tier 3: Summarize (handled by caller with model access)
        (snipped, self.build_reinject_context())
    }

    /// Post-compaction reinject: FTAI rules, active skills, tool definitions, memory.
    /// These are stripped during snip but must be visible to the model after compaction.
    fn build_reinject_context(&self) -> Vec<Message> {
        // Caller provides: active FTAI rules, MEMORY.md contents, tool listing
        // These become system messages appended after compacted history
        vec![]  // Populated by orchestrator with current rule/memory state
    }
}
```

### Key insight from reference implementations

The reference `snipCompact` is **deterministic** — no model call needed. It removes old messages and re-appends system context. This is critical for Forge where every model call is expensive (local inference at 35 tok/s). The **reinject pattern** is equally important: after compaction, skills, rules, and tool definitions must be re-attached so the model doesn't lose awareness of its capabilities.

---

## Amendment 2: Tool Abort Signals + Progress Callbacks (amends Section 5)

### Problem

Section 5 defines the `Tool` trait with `execute()` returning `Result<ToolResult>`. No mechanism for:
- Cancelling a running tool (user presses Ctrl+C during a long bash command)
- Streaming progress back to the TUI (file indexing, large reads)

Modern AI coding assistants give every tool an abort signal and progress callbacks. This makes the TUI dramatically more responsive.

### Design

Extend the `Tool` trait:

```rust
// Amend src/tools/mod.rs

use tokio_util::sync::CancellationToken;

/// Progress update from a tool execution
pub enum ToolProgress {
    /// Percentage complete (0-100)
    Percent(u8),
    /// Status message (shown in TUI status bar)
    Status(String),
    /// Streaming partial output (shown in TUI as it arrives)
    PartialOutput(String),
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    fn safety_level(&self) -> SafetyLevel;

    /// Execute the tool with cancellation support and progress streaming.
    fn execute(
        &self,
        args: &Value,
        ctx: &ToolContext,
        cancel: CancellationToken,
        progress: mpsc::Sender<ToolProgress>,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + '_>>;

    /// Compact self-description for permission classifier context.
    /// Should be <50 tokens summarizing what this specific invocation does.
    fn classify_summary(&self, args: &Value) -> String {
        format!("{}({})", self.name(), truncate_args(args, 100))
    }
}
```

### Bash tool example

```rust
impl Tool for BashTool {
    fn execute(
        &self,
        args: &Value,
        ctx: &ToolContext,
        cancel: CancellationToken,
        progress: mpsc::Sender<ToolProgress>,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let cmd = args["command"].as_str().unwrap_or("");
            let mut child = Command::new("sh")
                .arg("-c").arg(cmd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            let stdout = child.stdout.take().unwrap();
            let mut reader = BufReader::new(stdout).lines();
            let mut output = String::new();

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        child.kill().await.ok();
                        return Ok(ToolResult::error("Cancelled by user"));
                    }
                    line = reader.next_line() => {
                        match line? {
                            Some(line) => {
                                output.push_str(&line);
                                output.push('\n');
                                progress.send(ToolProgress::PartialOutput(line)).await.ok();
                            }
                            None => break,
                        }
                    }
                }
            }

            let status = child.wait().await?;
            Ok(ToolResult::new(output, status.success()))
        })
    }
}
```

### Wiring to TUI

The orchestrator passes `CancellationToken` created on Ctrl+C and routes `ToolProgress` to the TUI's status bar channel.

---

## Amendment 3: Dual Storage — JSONL Transcript + SQLite Analytics (amends Section 7.1)

### Problem

Section 7.1 uses SQLite for everything: session transcripts AND evolution analytics. Reference implementations use **append-only JSONL per session** for transcripts, which is:
- Crash-safe (no transactions, append-only)
- Debuggable (`cat session.jsonl | jq`)
- Streamable (`tail -f` during debugging)
- Resume-safe (replay with filtering)

SQLite is better for cross-session queries (which Mitosis needs).

### Design: Use both

```
~/.ftai/sessions/
  <project_hash>/
    <session_id>.jsonl          # Live transcript (append-only)
    <session_id>.meta.json      # Session metadata (start time, project, summary)

~/.ftai/evolution/
  evolution.db                  # SQLite — cross-session analytics for Mitosis
```

**JSONL format** (one JSON object per line):

```jsonl
{"seq":0,"role":"system","content":"You are Forge...","ts":1711900000}
{"seq":1,"role":"user","content":"fix the auth bug","ts":1711900001}
{"seq":2,"role":"assistant","content":"Let me read...","tool_calls":[{"name":"file_read","args":{"path":"src/auth.rs"}}],"ts":1711900002}
{"seq":3,"role":"tool","tool_call_id":"tc_1","content":"[file contents]","ts":1711900003,"tokens_est":450}
```

**Resume filtering** (skip on replay):

```rust
pub fn load_session(path: &Path) -> Result<Vec<Message>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let entry: SessionEntry = serde_json::from_str(&line?)?;

        // Skip ephemeral entries on resume
        match entry.role.as_str() {
            "progress" => continue,     // TUI-only progress updates
            "system" if entry.is_stale_context() => continue,  // Old injected context
            _ => messages.push(entry.into_message()),
        }
    }
    Ok(messages)
}
```

**Evolution analytics** still use SQLite (Section 6.5 unchanged). At session end, the JSONL is parsed and tool call records are written to `evolution.db` for Mitosis analysis.

---

## Amendment 4: Glob-Matched Rule Loading (amends Section 8.3-8.4)

### Problem

Section 8.4's `LazyRuleLoader` walks directory parents looking for `.ftai/RULES.md`. This works for module-level scoping but doesn't support **file-type-specific rules** (e.g., "Rust rules only load when working on .rs files").

Community config toolkits use glob matchers in YAML frontmatter:

```yaml
---
description: "Rust-specific coding guidelines"
globs: ["**/*.rs"]
alwaysApply: false
---
```

### Design: Add glob matching to FTAI Rules

Extend FTAI rule file format to support an optional glob header:

```markdown
---
globs: ["**/*.rs", "**/*.toml"]
alwaysApply: false
---

rule "rust-unwrap-guard" {
  on tool:file_edit
  when extension(path) == "rs"
  require not contains(new_string, ".unwrap()")
  unless contains(new_string, "// SAFETY:") or contains(new_string, "test")
  reason "Avoid .unwrap() in production Rust code — use ? or expect() with context"
}
```

**Loading behavior:**

| `alwaysApply` | `globs` | When loaded |
|---------------|---------|-------------|
| `true` | ignored | Always loaded at session start |
| `false` | present | Loaded when any tool call targets a file matching the glob |
| `false` | absent | Loaded when directory-walk hits the rule file (existing behavior) |

```rust
// Amend src/rules/loader.rs

pub struct GlobRule {
    pub rule_set: RuleSet,
    pub globs: Vec<GlobPattern>,
    pub always_apply: bool,
}

impl LazyRuleLoader {
    pub fn rules_for_context(&mut self, tool_name: &str, file_path: Option<&Path>) -> Vec<&Rule> {
        let mut applicable = Vec::new();

        for glob_rule in &self.all_rules {
            if glob_rule.always_apply {
                applicable.extend(glob_rule.rule_set.rules.iter()
                    .filter(|r| r.event_matches(tool_name)));
                continue;
            }

            // Check glob match against the file being operated on
            if let Some(path) = file_path {
                if glob_rule.globs.iter().any(|g| g.matches_path(path)) {
                    applicable.extend(glob_rule.rule_set.rules.iter()
                        .filter(|r| r.event_matches(tool_name)));
                }
            }
        }

        applicable
    }
}
```

This is cheap (glob matching is microseconds) and prevents Forge from loading 100+ rules when only 5 apply to the current file type.

---

## Amendment 5: Skills as Separate from Rules (new subsection under Section 8)

### Problem

Forge's architecture treats FTAI Rules and knowledge files as the only extension points. Modern AI coding assistants separate **rules** (always-on, small, convention enforcement) from **skills** (on-demand, can be large, domain knowledge + workflows). With a 32K context window, injecting all knowledge upfront is not viable.

### Design: FTAI Skills

```
~/.ftai/skills/                       # User skills
  rust-async.md                       # Domain knowledge, loaded on demand
  react-hooks.md
  
<project>/.ftai/skills/               # Project skills
  deployment-process.md               # Team workflows
  api-conventions.md

~/.ftai/plugins/<name>/skills/        # Plugin-provided skills
  tdd-workflow.md
```

**Skill format:**

```markdown
---
name: rust-async-patterns
description: "Async Rust patterns — tokio, channels, select!, error propagation"
triggers: ["async", "tokio", "spawn", "channel", "select!", "Future", ".await"]
max_tokens: 1500
---

# Async Rust Patterns

## When to use tokio::spawn vs direct .await
[... domain knowledge ...]

## Error handling in async contexts
[... patterns ...]
```

**Loading strategy:**

1. At session start, load only skill **metadata** (name, description, triggers) — ~50 tokens per skill
2. When the model's response or user input contains trigger keywords, inject the full skill content
3. Skills are injected as system messages, subject to compaction like everything else
4. After compaction, only re-inject skills that are still relevant (trigger re-evaluation)

```rust
// New: src/skills/mod.rs

pub struct SkillRegistry {
    skills: Vec<SkillMeta>,
    loaded_cache: HashMap<String, String>,  // name -> full content
}

pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub max_tokens: usize,
    pub path: PathBuf,
}

impl SkillRegistry {
    /// Check if any skills should be activated based on text content
    pub fn match_skills(&self, text: &str) -> Vec<&SkillMeta> {
        let text_lower = text.to_lowercase();
        self.skills.iter()
            .filter(|s| s.triggers.iter().any(|t| text_lower.contains(&t.to_lowercase())))
            .collect()
    }

    /// Load full skill content (lazy, cached)
    pub fn load_skill(&mut self, name: &str) -> Option<&str> {
        if !self.loaded_cache.contains_key(name) {
            let meta = self.skills.iter().find(|s| s.name == name)?;
            let content = std::fs::read_to_string(&meta.path).ok()?;
            self.loaded_cache.insert(name.to_string(), content);
        }
        self.loaded_cache.get(name).map(|s| s.as_str())
    }
}
```

### Rules vs Skills distinction

| | Rules | Skills |
|---|---|---|
| **Purpose** | Enforce conventions, block bad patterns | Provide domain knowledge, guide workflows |
| **Size** | Small (< 200 tokens each) | Can be large (500-2000 tokens) |
| **Loading** | Always-on or glob-matched | On-demand via trigger keywords |
| **Execution** | Evaluated per tool call | Injected into system context |
| **Format** | FTAI DSL (`rule "name" { ... }`) | Markdown with YAML frontmatter |
| **User-editable** | Yes, but requires DSL knowledge | Yes, plain markdown |

---

## Amendment 6: Inference Backend Fallback Chain (amends Section 3)

### Problem

Section 3 describes llama.cpp FFI as primary and MLX as Mac-specific alternative, but doesn't define runtime selection or fallback behavior.

Reference implementations use a **memoized probe pattern**: try each backend at startup, cache the first working one, never re-probe.

### Design

```rust
// Amend src/inference/mod.rs

pub struct InferenceManager {
    active_backend: Box<dyn ModelBackend>,
    probe_results: BackendProbeResults,
}

struct BackendProbeResults {
    llamacpp_available: Option<bool>,   // None = not probed yet
    mlx_available: Option<bool>,
}

impl InferenceManager {
    /// Probe backends in priority order, memoize results
    pub async fn initialize(config: &InferenceConfig) -> Result<Self> {
        // Priority 1: llama.cpp FFI (fastest, most portable)
        if let Ok(backend) = LlamaCppBackend::try_new(config).await {
            return Ok(Self {
                active_backend: Box::new(backend),
                probe_results: BackendProbeResults {
                    llamacpp_available: Some(true),
                    mlx_available: None,  // Don't probe if not needed
                },
            });
        }

        // Priority 2: MLX (Apple Silicon only)
        if cfg!(target_os = "macos") {
            if let Ok(backend) = MlxBackend::try_new(config).await {
                return Ok(Self {
                    active_backend: Box::new(backend),
                    probe_results: BackendProbeResults {
                        llamacpp_available: Some(false),
                        mlx_available: Some(true),
                    },
                });
            }
        }

        Err(anyhow::anyhow!(
            "No inference backend available.\n\
             llama.cpp: failed to load model\n\
             MLX: {}\n\
             Run `forge doctor` for diagnostics.",
            if cfg!(target_os = "macos") { "failed to start" } else { "not available (macOS only)" }
        ))
    }
}
```

The key pattern: **probe once, cache result, don't re-probe**. Backend switching mid-session is not supported — if the backend fails, the session ends with an error.

---

## Amendment 7: Cargo Feature Gates (amends Section 11)

### Problem

Section 11 describes the build as a single monolithic binary. Some subsystems are optional or platform-specific but always compiled.

### Design

```toml
# Amend Cargo.toml

[features]
default = ["llamacpp", "evolution", "search"]
llamacpp = ["llama-cpp-sys-2"]
mlx = []                              # MLX Python subprocess bridge (macOS only)
evolution = []                        # Mitosis self-evolution engine
search = ["fastembed", "tree-sitter"] # RTAI code search
knowledge-sampler = []                # KnowledgeSampler logit-level enforcement
```

Use `cfg` guards in code:

```rust
#[cfg(feature = "evolution")]
pub mod evolution;

#[cfg(feature = "search")]
pub mod search;

#[cfg(feature = "knowledge-sampler")]
use crate::inference::knowledge_sampler::KnowledgeSampler;
```

This enables:
- Minimal builds for constrained environments: `cargo build --no-default-features --features llamacpp`
- Platform-specific builds: `cargo build --features mlx` on macOS
- Debug builds without heavy subsystems: faster compile times during development

---

## Amendment 8: Hook System (new subsection under Section 5)

### Problem

Forge has no event-driven automation. Modern AI coding assistants implement hooks — shell commands triggered by events like tool calls, file edits, or session lifecycle.

### Design

```toml
# <project>/.ftai/config.toml

[[hooks]]
event = "before_tool"
tool = "bash"
command = "echo $FORGE_TOOL_ARGS | jq -r '.command' | grep -qE 'rm -rf|sudo' && echo 'BLOCK: dangerous command' && exit 1 || exit 0"
description = "Block dangerous bash commands"
timeout_ms = 5000

[[hooks]]
event = "after_file_edit"
command = "rustfmt $FORGE_FILE_PATH 2>/dev/null || true"
description = "Auto-format Rust files after edit"
timeout_ms = 10000

[[hooks]]
event = "session_start"
command = "git status --porcelain | head -5"
description = "Show uncommitted changes at session start"
timeout_ms = 3000
```

**Events:**

| Event | Env vars | Can block? |
|-------|----------|------------|
| `session_start` | `FORGE_PROJECT` | No |
| `session_end` | `FORGE_PROJECT`, `FORGE_SESSION_ID` | No |
| `before_tool` | `FORGE_TOOL_NAME`, `FORGE_TOOL_ARGS` (JSON) | Yes (exit 1 = block) |
| `after_tool` | `FORGE_TOOL_NAME`, `FORGE_TOOL_RESULT` | No |
| `after_file_edit` | `FORGE_FILE_PATH` | No |
| `before_commit` | `FORGE_COMMIT_MSG` | Yes |

```rust
// New: src/hooks/mod.rs

pub struct HookRunner {
    hooks: Vec<HookConfig>,
}

impl HookRunner {
    /// Run hooks for an event. Returns Err if a blocking hook exits non-zero.
    pub async fn run(&self, event: &str, env: &HashMap<String, String>) -> Result<()> {
        let matching: Vec<_> = self.hooks.iter()
            .filter(|h| h.event == event)
            .filter(|h| h.tool.as_ref().map_or(true, |t| {
                env.get("FORGE_TOOL_NAME").map_or(false, |n| n == t)
            }))
            .collect();

        for hook in matching {
            let output = Command::new("sh")
                .arg("-c")
                .arg(&hook.command)
                .envs(env.iter())
                .timeout(Duration::from_millis(hook.timeout_ms))
                .output()
                .await?;

            if !output.status.success() && hook.can_block(event) {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow::anyhow!("Hook blocked: {}", stderr.trim()));
            }
        }
        Ok(())
    }
}
```

---

## Amendment 9: Contextual Permission Classifier (amends Section 5.3)

### Problem

Section 5.3 uses static safety levels. Reference implementations add a **contextual classifier** that considers the conversation context when deciding permissions. For Forge, we can't call a cloud classifier, but we can use the local model itself.

### Design: Lightweight local classification

Instead of a full model call, use a **heuristic + denial tracking** approach:

```rust
// Amend src/permissions/classifier.rs

pub struct PermissionClassifier {
    /// Count of consecutive user denials for similar tool calls
    denial_streak: HashMap<String, u32>,
    /// Tool calls auto-approved in this session (for "always allow" feature)
    session_allowlist: HashSet<String>,
}

impl PermissionClassifier {
    pub fn classify(
        &mut self,
        name: &str,
        args: &Value,
        ctx: &ToolContext,
    ) -> SafetyLevel {
        // Check session allowlist first
        let key = self.tool_key(name, args);
        if self.session_allowlist.contains(&key) {
            return SafetyLevel::Safe;
        }

        // Static classification (existing behavior)
        let base = classify_tool_call(name, args, ctx);

        // Denial tracking: if user has denied 3+ similar calls, escalate
        if let Some(denials) = self.denial_streak.get(&key) {
            if *denials >= 3 {
                return SafetyLevel::Dangerous; // Force explicit approval
            }
        }

        base
    }

    /// Called when user approves with "always allow"
    pub fn add_to_session_allowlist(&mut self, name: &str, args: &Value) {
        self.session_allowlist.insert(self.tool_key(name, args));
    }

    /// Called when user denies a tool call
    pub fn record_denial(&mut self, name: &str, args: &Value) {
        let key = self.tool_key(name, args);
        *self.denial_streak.entry(key).or_insert(0) += 1;
    }

    /// Generate a stable key for similar tool calls
    /// e.g., "file_edit:src/auth.rs" or "bash:cargo*"
    fn tool_key(&self, name: &str, args: &Value) -> String {
        match name {
            "file_read" | "file_write" | "file_edit" =>
                format!("{}:{}", name, args.get("path").and_then(|v| v.as_str()).unwrap_or("?")),
            "bash" => {
                let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let prefix = cmd.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
                format!("bash:{}", prefix)
            }
            _ => name.to_string(),
        }
    }
}
```

This avoids burning model tokens on classification while still providing:
- Session-scoped "always allow" (user approves once, similar calls auto-approved)
- Denial escalation (3 rejections → escalate to Dangerous)
- Stable grouping of similar tool calls

---

## Priority Implementation Order

| Phase | Amendment | Effort | Blocked by |
|-------|-----------|--------|------------|
| **P0** | 1. Context compaction | Medium | Nothing |
| **P0** | 2. Tool abort + progress | Low | Nothing |
| **P1** | 3. Dual storage (JSONL + SQLite) | Low | Nothing |
| **P1** | 4. Glob-matched rule loading | Low | Nothing |
| **P1** | 9. Permission classifier (denial tracking) | Low | Nothing |
| **P2** | 5. Skills system | Medium | Glob matching (#4) |
| **P2** | 6. Backend fallback chain | Low | Nothing |
| **P2** | 7. Cargo feature gates | Low | Nothing |
| **P3** | 8. Hook system | Medium | Nothing |

P0 items should be implemented before Forge's first usable build — they're essential for a functional 32K-context assistant.
