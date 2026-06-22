# FORGE-001 — Dual-Model Adversarial Inference (Proposer → Critic)

- **Status:** Draft
- **Author:** Michael Folk
- **Date:** 2026-06-18
- **Branch:** `feature/windows-port`

## Intent

Replace Forge's single-model local inference with a two-model **adversarial** setup so answers are
reviewed before they reach the user:

- **Proposer / generator:** Devstral Small 2 (24B dense, Q4_K_M GGUF) — purpose-built for agentic
  software-engineering and tool calling.
- **Critic:** Qwen3.5-9B (Q4_K_M GGUF) — adversarially reviews the proposer's final answer for bugs,
  security issues, and missed requirements.
- **Loop:** proposer answers → critic reviews → proposer revises once if the critic flags issues.

This also fixes a concrete problem: the previous single 35B-A3B model (22.4 GB) thrashed the 32GB
machine. Devstral (14.3 GB) + Qwen3.5-9B (5.7 GB) with trimmed context fits with headroom.

## Constraints

- **Memory:** Both models stay resident on the 32GB box. Proposer context trimmed to 16K, critic to 8K,
  to keep total (weights + KV + OS + iGPU UMA) below the ceiling (~28–30 GB). Swapping models per turn is
  not acceptable (10–30s reload).
- **Backend:** Reuse the existing `BackendManager` / `LlamaCppServer` HTTP architecture. Critic runs as a
  second `llama-server` on a distinct port (8413; proposer keeps 8411).
- **Opt-in & backward-compatible:** New `[critic]` config section, all `#[serde(default)]`, `enabled =
  false` by default. With the critic disabled, behavior is identical to today (single model, no 8413).
- **Sequential, not concurrent:** The critic runs after the proposer settles, not in parallel real-time.
- **Security (CAT 7 — LLM Output Injection, P0):** The critic's output is fed back into the proposer's
  context. It MUST be sanitized before re-injection and wrapped in an unambiguous "reviewer feedback"
  data envelope. No code ships without the CAT 7 red tests (Red Test Rule).
- **Latency:** The critic fires once per *final* answer (not per tool-call turn). Overhead ≈ one 9B pass
  plus at most one proposer revision round.

## Acceptance Criteria

1. A new `[critic]` TOML section configures the critic model (path, context_length, max_rounds, trigger,
   llamacpp gpu_layers/threads). Absent or `enabled=false` ⇒ single-model behavior, no second server.
2. When enabled, launching `forge` spawns two `llama-server` processes (8411 proposer, 8413 critic) and
   both reach health-ready.
3. After the proposer's agent loop settles to `StopReason::EndOfText`, the critic reviews the final
   answer. On `APPROVE`, the answer is delivered. On `REVISE` (with rounds remaining), the sanitized
   critique is injected as a user message and the proposer produces one revised answer.
4. Revision is capped at `critic.max_rounds`.
5. Critic output is sanitized: fake tool-call blocks, instruction-injection strings, and oversized /
   null-byte payloads are neutralized before reaching the proposer's `ChatRequest`. Verified by ≥3 red
   tests that fail before the fix and pass after.
6. `forge setup` writes both `[model]` (Devstral proposer) and `[critic]` (Qwen3.5-9B) sections on a
   32GB-class machine; low-RAM machines get a single small model with the critic disabled.
7. `recommended_model()` and its unit tests reflect the new tiers with no stale Qwen3.5-vs-3.6 drift.
8. Full `cargo test` suite passes; total test count does not decrease.
9. The TUI surfaces critic activity (a "reviewing" indicator and the APPROVE/REVISE verdict).

## Out-of-Scope

- Concurrent / real-time parallel critic execution.
- Debate, consensus, or speculative-decoding modes (this spec is Generator + Critic only).
- Code signing / GitHub release tagging (tracked in `forge_windows_port.md`).
- Making the critic available over the cloud `Api` backend (local llama.cpp only for now).

## Open Questions

- Should `trigger = "code-only"` (critic only when the answer contains code) be implemented now or
  deferred? Default is `final` (review every final answer). Implement `final` + `always`; stub
  `code-only` if low-cost.
- Optimal `gpu_layers` split between proposer (iGPU) and critic (CPU) on the Radeon 820M — to be tuned
  empirically during verification; default critic to CPU (`gpu_layers = 0`).
