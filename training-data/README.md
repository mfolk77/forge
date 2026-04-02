# Forge Backend Training Data — Distillation Pipeline

## Overview

Fine-tuning dataset for Forge's backend-specialist model. Uses **distillation** from a frontier teacher model (Claude Opus) to generate rich instruction-response pairs and multi-turn conversations from FolkTech's production codebase.

**Target:** 5,000–15,000 high-quality examples
**Format:** Mixed — instruction-response (Format A) + multi-turn conversation (Format B, ChatML)
**Base model:** Qwen 3.5 family (size TBD based on hardware)

## Directory Structure

```
training-data/
├── raw-extracts/          # Code extracted from Forge + Serena, organized by category
│   ├── architecture/      # Tool systems, compaction, permissions, plugins, rules engine
│   ├── security/          # Red tests, sanitizers, validators, path traversal
│   ├── api-services/      # HTTP clients, JSON-RPC, OAuth, streaming, MCP
│   ├── data-persistence/  # SQLite, JSONL, memory systems, fact storage
│   ├── rust-idioms/       # anyhow, traits, async, cfg, builders, lifetimes
│   └── testing-methodology/ # Red test patterns, two-stage testing, async testing
│
├── distilled/             # Teacher model (Claude) output — instruction/response pairs + convos
│   ├── architecture/
│   ├── security/
│   ├── api-services/
│   ├── data-persistence/
│   ├── rust-idioms/
│   └── testing-methodology/
│
├── final/                 # Quality-filtered, deduplicated, ready for fine-tuning
│   ├── architecture/
│   ├── security/
│   ├── api-services/
│   ├── data-persistence/
│   ├── rust-idioms/
│   └── testing-methodology/
│
├── scripts/               # Extraction + distillation scripts
│   ├── extract.py         # Pulls code patterns from source repos
│   ├── distill.py         # Sends to teacher model, collects output
│   ├── filter.py          # Quality checks, dedup, compile verification
│   └── merge.py           # Combines all categories into final JSONL
│
├── source-map.toml        # Maps source files → categories → extraction strategy
├── distill-prompts.toml   # Teacher model prompts per category
└── README.md              # This file
```

## Pipeline

```
Step 1: EXTRACT    → Pull code patterns from Forge + Serena into raw-extracts/
Step 2: DISTILL    → Feed to Claude Opus with category-specific prompts → distilled/
Step 3: FILTER     → Quality check, compile verify, dedup → final/
Step 4: MERGE      → Combine all final/ into single train.jsonl + val.jsonl (90/10 split)
Step 5: FINE-TUNE  → Qwen fine-tuning run (when hardware is ready)
```

## Source Codebases

| Source | Location | Lines | Language | Categories |
|--------|----------|-------|----------|------------|
| Forge src | ~/Developer/forge/src/ | 39,718 | Rust | All |
| Forge tests | ~/Developer/forge/tests/ | 2,712 | Rust | Testing |
| Forge inline security tests | ~/Developer/forge/src/*/security_tests.rs | ~3,000 | Rust | Security |
| Serena services | ~/Developer/Serena/Sources/SerenaCore/Services/ | ~50,000 | Swift | API, Architecture, Security |
| Serena models | ~/Developer/Serena/Sources/SerenaCore/Models/ | ~2,500 | Swift | Data, Architecture |
| Serena red tests | ~/Developer/Serena/Tests/SerenaCoreTests/RedTests/ | 18,221 | Swift | Security, Testing |
| Serena backend tests | ~/Developer/Serena/Tests/SerenaCoreTests/ | 10,274 | Swift | Testing |

## Distillation Strategy Per Category

### Architecture (target: 800-1,200 examples)
- Extract: Tool trait, compactor, permission classifier, plugin system, rules engine, config loader
- Distill: Design reasoning, alternative approaches, multi-turn "build this system" conversations
- Negative examples: Over-engineering, wrong abstraction boundaries

### Security (target: 1,500-2,500 examples)
- Extract: All red tests, sanitizers, validators, path traversal, injection prevention
- Distill: Attack scenarios, "is this secure?" review conversations, red test generation from code
- Negative examples: Common vulnerability patterns, "this looks safe but isn't"

### API & Services (target: 800-1,200 examples)
- Extract: HTTP clients, JSON-RPC, OAuth, streaming, MCP, inference engines
- Distill: API design conversations, error handling strategies, retry/timeout patterns
- Negative examples: Naive implementations without timeout/retry/auth

### Data Persistence (target: 500-800 examples)
- Extract: SQLite patterns, JSONL transcripts, memory systems, fact storage
- Distill: Schema design conversations, migration strategies, consistency patterns
- Negative examples: N+1 queries, missing WAL mode, no connection pooling

### Rust Idioms (target: 1,000-1,500 examples)
- Extract: Error handling, async patterns, trait design, lifetime usage, cfg compilation
- Distill: "What's the idiomatic way to..." conversations, refactoring from bad → good
- Negative examples: Fighting the borrow checker, unnecessary clones, blocking in async

### Testing Methodology (target: 500-1,000 examples)
- Extract: Red test structure, two-stage testing, async test patterns, permission test patterns
- Distill: "How do I test this?" conversations, test-first development flows
- Negative examples: Tests that pass but don't actually verify anything
