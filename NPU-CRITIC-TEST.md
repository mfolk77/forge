# Forge Dual-Model (Proposer + NPU Critic) Test Sheet

Setup under test:
- **Proposer:** Devstral Small 2 (24B) on **CPU** (slow but strong)
- **Critic:** Qwen2.5-7B-Instruct on the **NPU** via lemonade (fast, separate silicon)
- Loop: proposer answers → critic reviews → on `REVISE`, proposer revises once

## Before you start — open Task Manager → Performance tab
You want to watch two graphs:
- **CPU** — spikes while the *proposer* generates (the slow part)
- **NPU** — spikes while the *critic* reviews (should be a short, fast burst)
- **Memory** — should stay well under the ceiling (no sustained 30+ GB / no disk thrash)

Launch:
```
forge
```
On startup you should see:
- `Model loading in background...`
- `Adversarial critic loading on the NPU in background.`

---

## Test 1 — Sanity (expect APPROVE)
Confirms the whole pipeline runs and the critic returns cleanly.

> Write a Rust function `fn is_even(n: i32) -> bool` and show one example call.

**Expect:** proposer answers → `Critic reviewing (round 1/1)...` → `Critic: APPROVED.`
**Watch:** a short NPU burst right after the proposer finishes.

---

## Test 2 — Bug magnet (expect REVISE)
A naive first answer divides by zero on an empty slice — the critic should catch it and trigger a revision.

> Write a Rust function `fn average(nums: &[f64]) -> f64` that returns the average of the slice.

**Expect:** proposer's first answer (likely no empty-slice guard) → `Critic: REVISE` with feedback about the empty/zero-length case → a corrected second answer that handles `nums.is_empty()`.
**This is the key test** — it demonstrates the full adversarial loop end to end.

---

## Test 3 — Security lens (expect REVISE)
The critic is prompted to look for security issues, not just bugs.

> Write a Rust function that takes a filename from user input and returns the contents of that file from a `./data` directory.

**Expect:** first answer likely joins the path naively → critic flags **path traversal** (`../`) and/or missing validation → revised answer that canonicalizes/validates the path stays within `./data`.

---

## Test 4 — Trigger gating (optional)
Confirms the critic only fires on a *final* answer, not mid-tool-loop. Ask something that makes the proposer use tools first:

> List the Rust files in the current directory, then tell me which one is largest.

**Expect:** tool calls run without a critic pass between them; the critic reviews only the **final** summary answer.

---

## What "good" looks like
- Critic messages appear as distinct system lines (`Critic reviewing…`, `Critic: APPROVED.` / `Critic: REVISE`).
- NPU graph moves during the critic pass; CPU graph moves during the proposer pass — they shouldn't both peg at once.
- After you `/quit`, no leftover `lemond.exe` / `ryzenai-server` processes (forge kills the tree). Verify:
  ```
  tasklist | findstr /i "lemond ryzenai-server llama-server"
  ```
  should print nothing.

## If something's off
- **Critic never reviews / "Critic unavailable":** check `~/.ftai/lemonade-server.log`.
- **Machine bogs down:** drop proposer `context_length` to `8192` in `~/.ftai/config.toml`.
- **Want a sharper code reviewer:** set critic `model = "Qwen2.5-Coder-7B-Instruct-NPU"` (coding-specialized) — pull it first with
  `~/.ftai/lemonade/lemonade.exe --port 13305 pull Qwen2.5-Coder-7B-Instruct-NPU`.
- **Critic too slow / want it on CPU instead:** set critic `backend = "llamacpp"` (uses the Qwen3.5-9B at the `path` already in the config).
