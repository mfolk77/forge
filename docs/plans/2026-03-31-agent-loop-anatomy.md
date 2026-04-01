# The Anatomy of an Agent Loop: Lessons from learn-claude-code

**Date:** 2026-03-31
**Source:** [shareAI-lab/learn-claude-code](https://github.com/shareAI-lab/learn-claude-code) — a minimal Python distillation of Claude Code's architecture into ~11 composable sessions
**Purpose:** Extract every load-bearing pattern from the agent loop for Forge's Rust implementation

---

## The Core Thesis

> The harness is the world the model inhabits. The model is the intelligence. The harness is not.

An AI coding agent is not a model. It's a **while loop** with a model inside it. Everything else — tools, memory, permissions, teams, autonomy — is layered on top of that loop without changing its fundamental shape. learn-claude-code proves this by building the full architecture in 11 progressive sessions, where the loop itself never changes.

---

## Layer 0: The Loop (s01)

The entire secret in 10 lines:

```python
def agent_loop(messages: list):
    while True:
        response = LLM(messages, tools)
        messages.append({"role": "assistant", "content": response.content})
        if response.stop_reason != "tool_use":
            return
        results = []
        for block in response.content:
            if block.type == "tool_use":
                output = execute(block.name, block.input)
                results.append({"type": "tool_result", "tool_use_id": block.id, "content": output})
        messages.append({"role": "user", "content": results})
```

### What makes this work

1. **The model decides when to stop.** Not the harness. The loop exits when `stop_reason != "tool_use"`. The model emits text instead of a tool call when it believes the task is done. This is the single most important design decision — it means the harness doesn't need to understand task completion.

2. **Tool results are user messages.** The Anthropic API treats tool results as user-role messages containing `tool_result` blocks. This means the conversation flow is always `user → assistant → user → assistant → ...` even during multi-step tool use. The model sees tool results exactly like it sees human input.

3. **Messages are mutable and shared.** The `messages` list is passed by reference. Everything appended inside the loop is visible to the caller. This is how the outer REPL maintains conversation history across multiple `agent_loop()` invocations.

4. **The loop is stateless.** No iteration counter, no state machine, no mode flags. The only state is the `messages` list. This makes the loop trivially testable and composable.

### Forge implication

Forge's orchestrator should follow this exact shape. The temptation will be to add state (turn counters, mode flags, retry tracking) to the loop itself. Resist it. State belongs in the message history or in external managers, never in the loop.

```rust
// The Forge loop should be this simple:
pub async fn agent_loop(messages: &mut Vec<Message>, backend: &dyn ModelBackend, tools: &ToolRegistry) -> Result<()> {
    loop {
        let response = backend.generate(&build_request(messages, tools)).await?;
        messages.push(response.into_message());
        if response.stop_reason != StopReason::ToolUse {
            return Ok(());
        }
        let results = execute_tool_calls(&response, tools).await;
        messages.push(Message::tool_results(results));
    }
}
```

---

## Layer 1: Tool Dispatch (s02)

### The pattern: Handler map + schema array

```python
TOOL_HANDLERS = {
    "bash":       lambda **kw: run_bash(kw["command"]),
    "read_file":  lambda **kw: run_read(kw["path"], kw.get("limit")),
    "write_file": lambda **kw: run_write(kw["path"], kw["content"]),
    "edit_file":  lambda **kw: run_edit(kw["path"], kw["old_text"], kw["new_text"]),
}

TOOLS = [
    {"name": "bash", "description": "Run a shell command.",
     "input_schema": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}},
    # ...
]
```

Two parallel data structures:
- **TOOLS** — JSON schema sent to the model (describes what tools exist)
- **TOOL_HANDLERS** — execution map (runs the tool when called)

### Critical insight: The loop doesn't change

Adding tools never modifies the agent loop. The loop dispatches generically:

```python
handler = TOOL_HANDLERS.get(block.name)
output = handler(**block.input) if handler else f"Unknown tool: {block.name}"
```

This is why the loop from s01 is identical in s11 (the final session). Tools are plugged in, not hardcoded.

### Safety boundary: Path validation

```python
def safe_path(p: str) -> Path:
    path = (WORKDIR / p).resolve()
    if not path.is_relative_to(WORKDIR):
        raise ValueError(f"Path escapes workspace: {p}")
    return path
```

Every file operation runs through `safe_path()`. This is the **only** path validation — no secondary checks, no redundant verification. One choke point, one place to audit.

### Forge implication

Forge already has a `ToolRegistry` with this pattern. The lesson is: keep it this simple. Don't add middleware chains, aspect-oriented hooks, or dynamic dispatch layers. A `HashMap<String, Box<dyn Tool>>` is sufficient. Tool lookup is O(1), there are never more than ~20 tools, and the dispatch code should be readable by someone who has never seen the codebase.

---

## Layer 2: Progress Tracking (s03)

### The pattern: Structured state + nag injection

```python
class TodoManager:
    def update(self, items: list) -> str:
        # Constraint: only 1 in_progress at a time
        # Returns ASCII rendering: [ ] pending  [>] in_progress  [x] completed
```

The model tracks its own work via a todo tool. The harness enforces **one constraint**: only one item can be `in_progress` at a time. This prevents the model from "multitasking" (which really means losing focus).

### The nag: Soft intervention

```python
rounds_since_todo += 1
if rounds_since_todo >= 3:
    # Inject reminder as text content alongside tool results
    results.append({"type": "text", "text": "<reminder>Update your todos.</reminder>"})
    rounds_since_todo = 0
```

If the model goes 3 rounds without calling the todo tool, a reminder is injected **as content within the tool results message**. Not as a system prompt change. Not as a separate message. As inline text that the model reads naturally.

### Why this matters

The model will drift. It will start a 5-step task, complete step 2, and then rabbit-hole into an edge case it found. The todo + nag pattern keeps it on track without scripting the work. The model still decides what to do — it just can't forget to report what it's doing.

### Forge implication

Forge's `TodoManager` (if implemented) should be a simple in-memory state machine, not a database. The nag injection pattern maps to Forge's prompt builder — inject a `<reminder>` block after N tool calls without todo updates. The key is making it a **soft** intervention: the model can ignore the reminder, and sometimes that's correct (when the task is simple enough not to need tracking).

---

## Layer 3: Subagents (s04)

### The pattern: Fresh context, restricted tools, summary return

```python
def run_subagent(prompt: str) -> str:
    sub_messages = [{"role": "user", "content": prompt}]  # Fresh context!
    for _ in range(30):  # safety limit
        response = client.messages.create(
            model=MODEL, system=SUBAGENT_SYSTEM, messages=sub_messages,
            tools=CHILD_TOOLS, max_tokens=8000,
        )
        # ... standard loop ...
    return "".join(b.text for b in response.content if hasattr(b, "text")) or "(no summary)"
```

Three critical design decisions:

1. **Fresh message list.** `sub_messages = []` — the subagent has zero context from the parent. It knows nothing about what the parent was doing. This is intentional: exploration and subtasks should not be biased by the parent's history.

2. **Restricted tool set.** `CHILD_TOOLS` excludes the `task` tool itself — no recursive spawning. The subagent can read, write, and execute, but cannot spawn further subagents.

3. **Summary-only return.** Only the final text response returns to the parent. The entire subagent conversation (all tool calls, all intermediate results) is **discarded**. The parent's context stays clean.

### Why not share context?

Because context is the scarcest resource. Every token in the parent's history costs attention. If the parent sends "investigate the auth module" as a subagent task, the subagent might read 15 files and make 30 tool calls. If those all appeared in the parent's history, the parent would have consumed most of its context window on exploration that produced a 3-sentence summary.

### Forge implication

Forge's `agent_spawn` tool should create a new `Vec<Message>` and run the same `agent_loop()` function with it. In Rust, this is naturally scoped — the subagent's messages are stack-allocated (or in a separate heap allocation) and dropped when the function returns.

The 30-iteration safety limit is important. Without it, a subagent could loop forever on a broken tool call. Forge should make this configurable but default to 30.

```rust
pub async fn run_subagent(prompt: &str, backend: &dyn ModelBackend, tools: &ToolRegistry) -> Result<String> {
    let mut messages = vec![Message::user(prompt)];
    let restricted_tools = tools.without(&["agent_spawn"]); // No recursive spawning
    
    for _ in 0..30 {
        let response = backend.generate(&build_request(&messages, &restricted_tools)).await?;
        messages.push(response.into_message());
        if response.stop_reason != StopReason::ToolUse {
            return Ok(response.text_content());
        }
        let results = execute_tool_calls(&response, &restricted_tools).await;
        messages.push(Message::tool_results(results));
    }
    Ok("(subagent reached iteration limit)".into())
}
```

---

## Layer 4: Skills — On-Demand Knowledge (s05)

### The pattern: Two-layer injection

**Layer 1 — Metadata in system prompt (~100 tokens per skill):**

```python
SYSTEM = f"""You are a coding agent at {WORKDIR}.
Skills available:
  - pdf: Process PDF files [pdf,document]
  - code-review: Review code quality [review,quality]
"""
```

**Layer 2 — Full body via tool result (on demand):**

```python
def get_content(self, name: str) -> str:
    return f"<skill name=\"{name}\">\n{skill['body']}\n</skill>"
```

The model sees a list of available skills in its system prompt. When it needs one, it calls `load_skill("pdf")` and gets the full body back as a tool result.

### Why two layers?

System prompt tokens are permanent — they're consumed on every model call for the entire session. A skill with 1,500 tokens of instructions costs 1,500 tokens × every turn. With 10 skills, that's 15,000 tokens of permanent overhead — nearly half of Forge's 32K context window.

Two-layer injection costs ~100 tokens per skill permanently (just the name and description), and 1,500 tokens only when needed (loaded into a single tool result that gets compacted away later).

### Skill file format

```yaml
---
name: pdf
description: Process PDF files
tags: pdf,document
---

# PDF Processing Steps
1. Install poppler if not present
2. Use pdftotext for extraction
...
```

YAML frontmatter for metadata, markdown body for content. Parsed with a simple regex:

```python
match = re.match(r"^---\n(.*?)\n---\n(.*)", text, re.DOTALL)
meta = yaml.safe_load(match.group(1))
body = match.group(2).strip()
```

### Forge implication

This directly validates Forge's Amendment 5 (Skills system) from the Claude Code lessons. The implementation should be nearly identical — YAML frontmatter parsed at startup, bodies loaded lazily. The `<skill>` XML wrapper tag helps the model distinguish skill content from regular tool output.

Forge's trigger-based loading (matching keywords in user input) is an optimization on top of this. learn-claude-code uses explicit `load_skill()` calls, which is simpler but requires the model to know when to load a skill. Triggers automate that decision.

---

## Layer 5: Context Compaction (s06)

### The pattern: Three-tier compression pipeline

This is the most critical layer for Forge. Without it, any agent dies after ~10 turns on a 32K context window.

**Tier 1 — Microcompact (silent, every turn, zero model calls):**

```python
def micro_compact(messages: list) -> list:
    # Find all tool_result entries
    # Keep the last 3
    # Replace older ones with: "[Previous: used {tool_name}]"
    # Exception: preserve read_file results (reference material)
```

This runs **before every LLM call**. It's invisible to the model — old tool results are replaced with one-line placeholders. The model knows it used `bash` earlier, but doesn't see the full output anymore.

The `read_file` exception is critical: file contents are reference material that the model needs to edit files correctly. Compacting them forces re-reads, which wastes turns.

**Tier 2 — Auto-compact (token-triggered, one model call):**

```python
def auto_compact(messages: list) -> list:
    # 1. Save full transcript to .transcripts/{timestamp}.jsonl
    # 2. Ask LLM to summarize the conversation
    # 3. Replace ALL messages with: [summary]
    return [{"role": "user", "content": f"[Compressed]\n\n{summary}"}]
```

When `estimate_tokens(messages) > 50000`, the entire history is:
1. Saved to disk (never lost)
2. Summarized by the model itself (200-token summary)
3. Replaced with a single user message containing the summary

The model loses specific details but retains awareness of what it did and where it is.

**Tier 3 — Manual compact (model-triggered):**

The model can call the `compact` tool to force immediate summarization. Useful when the model realizes it's been exploring and wants to "reset" before starting focused work.

### Token estimation

```python
def estimate_tokens(messages: list) -> int:
    return len(str(messages)) // 4  # ~4 chars per token
```

No tokenizer needed. This heuristic is within 20% of real token counts and costs zero computation.

### Transcript preservation

```python
TRANSCRIPT_DIR.mkdir(exist_ok=True)
transcript_path = TRANSCRIPT_DIR / f"transcript_{int(time.time())}.jsonl"
with open(transcript_path, "w") as f:
    for msg in messages:
        f.write(json.dumps(msg, default=str) + "\n")
```

Before any compaction, the full transcript is written to disk. This means:
- No data is ever permanently lost
- The transcript can be replayed for debugging
- The evolution engine can analyze it post-session
- The user can `cat .transcripts/` to see what happened

### Forge implication

This is the strongest validation of Forge's Amendment 1 (Context Compaction). The three tiers map directly:

| learn-claude-code | Forge Amendment 1 |
|---|---|
| micro_compact (replace old tool_results) | Tier 1: Microcompact |
| auto_compact (LLM summarize) | Tier 3: Summarize compact |
| (missing) | Tier 2: Snip compact (deterministic, no model call) |

Forge adds a **middle tier** (snip compact) that learn-claude-code doesn't have. This is because Forge uses local models where every model call is expensive (~35 tok/s). Snip compact — deterministic removal of old messages without summarization — fills the gap between "free but shallow" microcompact and "expensive but thorough" summarization.

The `read_file` preservation rule is important and should be carried over to Forge. When the model reads a file, it's building working memory for an edit. Compacting that forces a re-read, which wastes a turn. Forge should preserve the last N `file_read` results (where N = 3-5) during microcompact.

---

## Layer 6: Persistent Tasks (s07)

### The pattern: File-per-task with dependency graph

```python
class TaskManager:
    # Each task: .tasks/task_{id}.json
    # Schema: {id, subject, description, status, blockedBy: [int], owner: str}
    
    def _clear_dependency(self, completed_id: int):
        for f in self.dir.glob("task_*.json"):
            task = json.loads(f.read_text())
            if completed_id in task["blockedBy"]:
                task["blockedBy"].remove(completed_id)
                f.write_text(json.dumps(task, indent=2))
```

### Why separate from messages?

Tasks survive context compaction. When auto_compact replaces all messages with a summary, the task board on disk is untouched. The model can call `task_list` after compaction and immediately see what work remains.

This is the critical difference between **todos** (in-memory, ephemeral, die with context) and **tasks** (on-disk, persistent, survive compaction and even session restarts).

### The dependency graph

`blockedBy: [3, 5]` means this task can't start until tasks 3 and 5 are completed. When task 3 completes, `_clear_dependency(3)` removes it from all blockers. When both 3 and 5 complete, the task becomes unblocked.

This enables the model to plan work with dependencies:
```
Task 1: Set up database schema
Task 2: Write API endpoints (blocked by 1)
Task 3: Write tests (blocked by 2)
```

### Forge implication

Forge's task system should use individual JSON files, not SQLite rows, for tasks. Reasons:
- Atomic writes (rename is atomic on POSIX)
- Debuggable (`cat .tasks/task_1.json`)
- Multi-agent safe (file locking is simpler than SQLite WAL for this use case)
- Git-friendly (tasks can be committed for shared team boards)

The dependency graph should be implemented as described. It's ~20 lines of code and enables non-trivial planning behavior.

---

## Layer 7: Background Execution (s08)

### The pattern: Fire-and-forget + notification drain

```python
class BackgroundManager:
    def run(self, command: str) -> str:
        task_id = str(uuid.uuid4())[:8]
        thread = threading.Thread(target=self._execute, args=(task_id, command), daemon=True)
        thread.start()
        return f"Background task {task_id} started"
    
    def drain_notifications(self) -> list:
        with self._lock:
            notifs = list(self._notification_queue)
            self._notification_queue.clear()
        return notifs
```

The key is the **drain point**: notifications are collected **before** the LLM call, not after.

```python
def agent_loop(messages: list):
    while True:
        # DRAIN FIRST
        notifs = BG.drain_notifications()
        if notifs:
            messages.append({"role": "user", "content": f"<background-results>\n{notif_text}\n</background-results>"})
        # THEN call LLM
        response = client.messages.create(...)
```

This means:
1. Agent spawns `cargo test` in background
2. Agent continues other work (reads files, makes edits)
3. At the start of the next loop iteration, `cargo test` results appear
4. The model sees them and can react

### Why not await?

`cargo test` on a large project takes 30-120 seconds. If the agent blocks, it wastes that time doing nothing. Background execution lets the agent do useful work while long commands run.

### Forge implication

Forge should implement this with `tokio::spawn` instead of threads:

```rust
pub struct BackgroundManager {
    tasks: Arc<Mutex<HashMap<String, BackgroundTask>>>,
    notifications: Arc<Mutex<Vec<Notification>>>,
}

impl BackgroundManager {
    pub fn spawn(&self, command: String) -> String {
        let id = Uuid::new_v4().to_string()[..8].to_string();
        let notifications = Arc::clone(&self.notifications);
        tokio::spawn(async move {
            let output = Command::new("sh").arg("-c").arg(&command).output().await;
            notifications.lock().unwrap().push(Notification { id, output });
        });
        format!("Background task {id} started")
    }
    
    pub fn drain(&self) -> Vec<Notification> {
        std::mem::take(&mut *self.notifications.lock().unwrap())
    }
}
```

The drain point is critical: call `drain()` at the **top** of the agent loop, before calling the model. This ensures the model always sees the latest results.

---

## Layer 8: Multi-Agent Teams (s09-s10)

### The pattern: JSONL inboxes + protocol FSMs

**MessageBus:**

```python
class MessageBus:
    def send(self, sender, to, content, msg_type="message", extra=None):
        msg = {"type": msg_type, "from": sender, "content": content, "timestamp": time.time()}
        inbox_path = self.dir / f"{to}.jsonl"
        with open(inbox_path, "a") as f:
            f.write(json.dumps(msg) + "\n")
    
    def read_inbox(self, name):
        # Read all messages, truncate file (drain semantics)
```

Each teammate has a file: `.team/inbox/coder.jsonl`. Messages are appended as JSON lines. When read, the file is truncated (drain semantics — messages are consumed, not peeked).

**Teammate spawning:**

Each teammate is a **thread** running its own agent loop with its own message list. Teammates share:
- The filesystem (WORKDIR)
- The task board (.tasks/)
- The message bus (.team/inbox/)

Teammates do NOT share:
- Message history (each has `messages = []`)
- System prompt (each gets a role-specific prompt)
- Tool set (teammates get limited tools — no recursive spawning)

### Protocol FSMs

**Shutdown protocol:**

```
Lead:      shutdown_request {request_id: "abc"} → coder's inbox
Coder:     reads inbox, decides
Coder:     shutdown_response {request_id: "abc", approve: true} → lead's inbox
Lead:      reads inbox, checks tracker[abc], marks "approved"
```

**Plan approval:**

```
Coder:     plan_approval {request_id: "xyz", plan: "I'll refactor auth.rs..."} → lead's inbox
Lead:      reads inbox, reviews plan
Lead:      plan_approval_response {request_id: "xyz", approve: true} → coder's inbox
```

Both use **request IDs** for correlation. Multiple in-flight requests can be tracked independently.

### Forge implication

Forge doesn't need multi-agent teams in v1. But the **message bus pattern** is reusable for any async coordination. If Forge ever adds background agents (Phase 2 IDE integration), the JSONL inbox pattern is the right approach:
- Append-only (crash-safe)
- Drain semantics (no duplicate processing)
- File-based (debuggable, no IPC complexity)
- Lock-free for writers (OS handles append atomicity for small writes)

---

## Layer 9: Autonomy (s11)

### The pattern: Idle polling + task board scanning + identity re-injection

This is the most advanced layer. Teammates become self-directed:

```python
# WORK PHASE: standard agent loop
for _ in range(50):
    # drain inbox, call LLM, execute tools
    if idle_tool_called:
        break

# IDLE PHASE: poll for work
deadline = time.time() + 60  # 60s timeout
while time.time() < deadline:
    time.sleep(5)
    
    # Check 1: inbox messages?
    inbox = BUS.read_inbox(name)
    if inbox:
        # Resume WORK with inbox messages in context
        break
    
    # Check 2: unclaimed tasks on the board?
    for task_file in TASKS_DIR.glob("task_*.json"):
        task = json.loads(task_file.read_text())
        if task["status"] == "pending" and not task["owner"] and not task["blockedBy"]:
            claim_task(task["id"], agent_name)
            # Resume WORK with claimed task in context
            break
    
    # Check 3: timeout → shutdown
```

### Task claiming (atomic)

```python
_claim_lock = threading.Lock()

def claim_task(task_id: int, owner: str) -> str:
    with _claim_lock:  # atomic check-and-set
        task = json.loads(path.read_text())
        if task.get("owner"):
            return "Error: already claimed"
        task["owner"] = owner
        task["status"] = "in_progress"
        path.write_text(json.dumps(task, indent=2))
```

The lock prevents two teammates from claiming the same task. This is the simplest possible distributed coordination — a mutex around a file read-write.

### Identity re-injection

After context compaction, the model forgets who it is. learn-claude-code solves this:

```python
def make_identity_block(name, role, team_name):
    return {
        "role": "user",
        "content": f"<identity>You are '{name}', role: {role}, team: {team_name}. "
                   f"Use task_list to find work. Use send_message to communicate.</identity>"
    }
# Inserted at messages[0] when list gets short
```

This is the **post-compaction reinject** pattern from Forge's Amendment 1. After summarization replaces all messages, the identity block is inserted at the front so the model knows its name, role, and capabilities.

### Forge implication

Forge's Mitosis self-evolution engine can learn from this. The idle → scan → claim → work cycle is exactly how autonomous agents should discover work. The task board is the coordination layer — no central scheduler, no message-passing overhead. Agents find work by reading the filesystem.

For Forge v1 (single agent), the relevant takeaway is **identity re-injection**: after context compaction, reinject the system identity, active rules, and tool list as the first message. Without this, the model loses awareness of its capabilities.

---

## The Pre-LLM-Call Checklist

The full integration (s_full.py) reveals the order of operations before every model call:

```
1. Drain background notifications → inject as user message
2. Drain inbox messages → inject as user message  
3. Run micro_compact (replace old tool results with placeholders)
4. Check token threshold → run auto_compact if exceeded
5. (Post-compact) reinject identity + rules + skill list
6. Call LLM
7. Parse response → execute tools → collect results
8. Append results to messages
9. Loop back to step 1
```

Steps 1-5 are **housekeeping**. They happen every iteration, silently. The model never sees the mechanics — it just gets a clean, right-sized context with the latest information.

### Forge implementation

This checklist maps directly to Forge's orchestrator:

```rust
pub async fn agent_loop(ctx: &mut AgentContext) -> Result<()> {
    loop {
        // 1. Drain background notifications
        let notifs = ctx.background.drain();
        if !notifs.is_empty() {
            ctx.messages.push(Message::background_results(notifs));
        }
        
        // 2. Micro-compact (replace old tool results)
        ctx.compactor.micro_compact(&mut ctx.messages);
        
        // 3. Check token budget, auto-compact if needed
        if ctx.compactor.should_compact(&ctx.messages) {
            ctx.compactor.snip_or_summarize(&mut ctx.messages, &ctx.backend).await?;
            // 4. Post-compact reinject
            ctx.messages.insert(0, ctx.build_reinject_context());
        }
        
        // 5. Call model
        let request = build_request(&ctx.messages, &ctx.tools, &ctx.rules, &ctx.skills);
        let response = ctx.backend.generate(&request).await?;
        ctx.messages.push(response.into_message());
        
        if response.stop_reason != StopReason::ToolUse {
            return Ok(());
        }
        
        // 6. Execute tools with cancellation support
        let results = execute_tools(&response, &ctx.tools, &ctx.cancel_token).await;
        ctx.messages.push(Message::tool_results(results));
        
        // 7. Log for evolution engine
        ctx.evolution.record_tool_calls(&response);
    }
}
```

---

## Error Handling Philosophy

learn-claude-code has a consistent error handling strategy:

**Errors are tool results, not exceptions.**

```python
try:
    output = handler(**block.input)
except Exception as e:
    output = f"Error: {e}"
results.append({"type": "tool_result", "tool_use_id": block.id, "content": str(output)})
```

Every error is caught, stringified, and returned as a tool result. The model sees `"Error: File not found"` and decides what to do next. The harness never crashes from a tool error.

**No retry logic in the harness.** If a tool fails, the model sees the error and decides whether to retry, try a different approach, or ask the user. The harness doesn't retry — that's a policy decision for the model.

**No error classification.** Errors are just strings. The model reads them. There's no error taxonomy, no error codes, no structured error types. This is intentional — the model is better at interpreting error messages than any hardcoded classifier.

### Forge implication

Forge should follow this pattern. Tool errors should be `Result::Err` at the Rust level but converted to string tool results before being appended to messages. The model should never see a Rust error type — it should see a human-readable error string.

The one exception: **cancellation**. If the user presses Ctrl+C, the tool should return `"Cancelled by user"` and the harness should consider whether to exit the loop entirely. This is a harness policy, not a model decision.

---

## State Persistence Strategy

learn-claude-code uses four persistence mechanisms:

| What | Where | Survives compaction? | Survives session end? |
|------|-------|---------------------|----------------------|
| Messages | In-memory list | No (replaced by summary) | No |
| Transcripts | `.transcripts/*.jsonl` | N/A (they ARE the backup) | Yes |
| Tasks | `.tasks/task_*.json` | Yes | Yes |
| Team state | `.team/config.json` + `.team/inbox/*.jsonl` | Yes | Yes |
| Skills | `skills/*/SKILL.md` | Reload on demand | Yes (static files) |

The key insight: **anything that must survive compaction lives on disk, not in messages.** Tasks, team state, and skills are all file-based. Messages are ephemeral.

### Forge mapping

| learn-claude-code | Forge equivalent |
|---|---|
| `.transcripts/*.jsonl` | `~/.ftai/sessions/<project>/<session>.jsonl` |
| `.tasks/task_*.json` | Same (file-per-task in project dir) |
| `.team/inbox/*.jsonl` | Future: multi-agent coordination |
| `skills/*/SKILL.md` | `~/.ftai/skills/` + `<project>/.ftai/skills/` |
| In-memory messages | In-memory `Vec<Message>` |
| Evolution data | `~/.ftai/evolution/evolution.db` (SQLite, not in learn-claude-code) |

---

## What learn-claude-code Doesn't Have (and Forge Needs)

1. **Streaming.** All LLM calls are blocking. Forge needs token-by-token streaming for TUI responsiveness.

2. **Permission prompting.** All tools auto-execute. Forge needs approval UI for Moderate/Dangerous tools.

3. **GBNF grammar constraints.** Tool calls are parsed from model output, not grammar-constrained. Forge needs this for reliable local model tool calling.

4. **Abort/cancellation.** No Ctrl+C handling. Forge needs `CancellationToken` per tool execution.

5. **Progress callbacks.** No partial output streaming from tools. Forge needs `ToolProgress` for long-running operations.

6. **Self-evolution.** No cross-session learning. Forge has Mitosis for this.

7. **Rule enforcement.** No FTAI rules. Forge has the full rules DSL.

8. **Knowledge grounding.** No logit-level enforcement. Forge has the KnowledgeSampler.

These are all **additive** — they layer on top of the loop without changing it. The loop from learn-claude-code's s01 is the same loop in Forge. Everything else is policy.

---

## Summary: The 10 Laws of Agent Loop Design

1. **The model decides when to stop.** The loop exits on `stop_reason != tool_use`.
2. **Tool results are user messages.** The conversation always alternates user/assistant.
3. **The loop is stateless.** All state lives in messages or external managers.
4. **Tools are plugged in, not hardcoded.** Adding tools never changes the loop.
5. **Errors are tool results.** Never crash from a tool error; let the model decide.
6. **Context is the scarcest resource.** Compact aggressively, reinject strategically.
7. **Subagents get fresh context.** Only the summary returns to the parent.
8. **Background results are drained, not awaited.** Non-blocking execution.
9. **Persistent state lives on disk.** Anything that must survive compaction is a file.
10. **The harness is not the intelligence.** Don't script the model's behavior — give it tools and let it work.
