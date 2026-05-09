# Troubleshooting Forge

Known issues, fixes, and what Forge does automatically to keep your session alive. Aimed at Mike + Michelle on a fresh laptop install — if you hit something not covered here, open an issue with the contents of `~/.ftai/mlx-server.log` (or `~/.ftai/llamacpp-server.log` on non-Apple-Silicon).

---

## Fresh install checklist

After running `curl … install.sh | sh`:

1. `forge --version` — confirm the binary is on PATH.
2. `forge setup` — installs the local backend (llama.cpp) and downloads the recommended model for your hardware.
3. `forge doctor` — verifies backends, hardware, config.
4. `cd <your-project>` and run `forge` — should drop you into a TUI session.

If any of these fail, jump to the matching section below.

---

## Apple Silicon (MLX) extra setup

`forge setup` installs llama.cpp (which works everywhere) but **does not** install `mlx-lm`. On macOS Apple Silicon you'll get faster, more memory-efficient inference by switching to the MLX backend:

```bash
brew install mlx-lm
forge model install Qwen/Qwen3.5-4B-4bit
forge config edit
# In config.toml under [model], set:
#   backend = "mlx"
#   path = "/Users/<you>/.ftai/models/Qwen3.5-4B-4bit"
#   tool_calling = "hybrid"
```

You should not need any other manual steps. If long MLX sessions used to crash with `ValueError: No function provided.`, that's now handled automatically — see "Auto-patches Forge applies" below.

---

## What Forge auto-fixes for you (so you don't have to)

These are workarounds the harness applies on its own. You don't have to know they exist, but if you ever wonder *why* something works, the answer is here.

### 1. MLX `qwen3_coder` parser disable (`tool_parser_type: null`)

**Problem**: MLX's `mlx_lm.server` ships a brittle `qwen3_coder` tool parser that raises `ValueError: No function provided.` on any malformed `<tool_call>` block in the model's output. Small Qwen3.5-Coder models under context pressure occasionally emit malformed blocks, killing the session with HTTP 500.

**Fix** (codified, automatic): On every MLX startup, Forge edits `<model_path>/tokenizer_config.json` to set `"tool_parser_type": null`. This disables MLX's native parser. Forge's own `extract_inline_tool_calls` (in `src/backend/http_client.rs`) lifts tool calls from the raw response content instead — more tolerant of malformed output.

**Idempotent**: if the field is already null, the file is not rewritten.
**Atomic**: temp file + rename, so a crash mid-write can't corrupt the model config.
**Safe**: the function never *creates* a tokenizer_config.json that didn't already exist (CAT 2 — Path & File Security).

If you want to verify it's working, look at `~/.ftai/mlx-server.log` after a long session — there should be no `ValueError` lines.

### 2. Inline tool-call extraction fallback

**Problem**: With MLX's native parser disabled (above), MLX returns model output verbatim. But Forge's response handler used to read `tool_calls` from the OAI-format structured field — which is now empty. Tools would be visible as raw `<tool_call>` XML text in the chat instead of executing.

**Fix** (codified, automatic): When the structured `tool_calls` field is `None` and the response content contains `<tool_call>` markers, Forge calls `parse_qwen35_xml` on the content, lifts the calls into structured `ToolCall`s, and strips the XML from the visible content.

This works for both streaming and non-streaming responses.

### 3. Shrink-before-retry on backend disconnect

**Problem**: If the local model server hits memory pressure or transient OOM and disconnects mid-request, Forge previously restarted the backend and retried the **same** large request — which would just fail the same way.

**Fix** (codified, automatic): After detecting a transport disconnect (`Failed to connect`, `connection refused`, `Stream read error`, etc.), Forge:

1. Restarts the local model server.
2. Forces a Tier-2 snip compaction on the conversation (keeps last 6 messages, drops older).
3. Rebuilds a smaller request.
4. Retries once.

You'll see `Backend disconnected (...). Restarting and shrinking request...` in the chat when this fires.

### 4. KV cache memory cap

**Problem**: MLX's `--prompt-cache-size` flag is the *count* of distinct caches, not a token limit. Without a bytes cap, large KV caches accumulate until macOS Metal OOMs.

**Fix** (existing): `src/backend/mlx.rs` passes `--prompt-cache-bytes` sized by detected RAM (768 MB ≤16 GB, 1.5 GB ≤32 GB, 3 GB otherwise). Cache count flag is mostly ignored by mlx-lm but kept for forward-compat.

---

## Common failure modes

### "Failed to connect to model server"

**Likely cause**: backend not running yet, or it crashed.

**Diagnostics**:
1. `forge doctor` — checks if MLX/llama.cpp is installed and reachable.
2. `tail -100 ~/.ftai/mlx-server.log` (Apple Silicon) — look for tracebacks.
3. `tail -100 ~/.ftai/llamacpp-server.log` (other) — same.
4. If MLX log shows `mlx_lm not found` → run `brew install mlx-lm` (Apple Silicon only).

If forge auto-restarts and the second attempt also fails, the error message will say so. The shrink-before-retry path handles only one retry per request.

### "Tools emit `<tool_call>` text but don't execute"

**Was a known regression** before A1+A2 codification. **Fixed automatically** in current builds — if you see this on a current binary, please open an issue.

If you're on an older binary, run `forge update`.

### Splash banner repeats during the first response

Cosmetic only, self-corrects after the first turn. Tracked but not yet reproduced reliably for fixing — if you can capture it consistently on a fresh `forge` cold-start, please grab a screenshot and the order of operations that produced it.

### Session memory growing without bound

Long-running sessions trigger Forge's 5-tier compaction system automatically:

- Tier 1 (microcompact) — runs before every LLM call, drops old tool-result bodies.
- Tier 2 (snip) — at 70% of usable context, drops oldest messages keeping last 10.
- Tier 3 (summarize) — at 85%, replaces all but the last few with a model-generated summary.
- Tier 5 (emergency truncate) — at 95%, hard-keeps system + last 3.

You can also run `/compact` manually in the TUI to force Tier 2 immediately.

### Plugin won't install

```bash
forge plugin info <name>      # check that the source URL resolves
forge plugin list             # confirm what's currently installed
```

Plugin manifests must use only alphanumeric/hyphen/underscore for their `name` field — names with `..`, `/`, or absolute paths are rejected at install for path traversal protection.

---

## Logs and where they live

| Log | Path | What's in it |
|---|---|---|
| MLX server stderr | `~/.ftai/mlx-server.log` | Apple Silicon backend output, prompt processing progress, tracebacks |
| llama.cpp server stderr | `~/.ftai/llamacpp-server.log` | Other-platform backend output |
| Conversations | `~/.ftai/sessions.db` (SQLite) | All session messages — open with `sqlite3` or `forge --resume` |
| Execution log | `~/.ftai/logs/` | Tool execution traces |

For privacy: `sessions.db` is currently stored unencrypted. If you keep credentials in your conversations, treat the file as sensitive (don't sync it to a backup that other people can read). Encryption-at-rest is on the roadmap.

---

## Reporting issues

If something here didn't help, open an issue with:

1. The exact command you ran.
2. The output of `forge doctor`.
3. The last 50 lines of the relevant log (`mlx-server.log` or `llamacpp-server.log`).
4. Your `~/.ftai/config.toml` (redact any API keys before sharing).

Issues: https://github.com/mfolk77/forge/issues
