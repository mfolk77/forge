# Forge Mesh, Specialization & Hackathon Readiness

**Date:** 2026-04-01
**Status:** Approved — Build in parallel with Forge Core (Option C)
**Authors:** Mike Folk
**Repo:** https://github.com/mfolk77/forge

---

## Table of Contents

1. [Overview](#1-overview)
2. [Forge Mesh — Multi-Instance Communication](#2-forge-mesh--multi-instance-communication)
3. [Role-Specialized Fine-Tuned Models](#3-role-specialized-fine-tuned-models)
4. [Hybrid Inference — Local + API Mid-Session Switching](#4-hybrid-inference--local--api-mid-session-switching)
5. [Cross-Platform — Mac + Windows](#5-cross-platform--mac--windows)
6. [Hardware Upgrade Path](#6-hardware-upgrade-path)
7. [Hackathon Readiness Checklist](#7-hackathon-readiness-checklist)
8. [Build Order & Workstreams](#8-build-order--workstreams)
9. [Security Requirements](#9-security-requirements)

---

## 1. Overview

Forge evolves from a single-user CLI into a **team-capable, role-specialized AI development system**. Two Forge instances — one backend-specialist (Mike), one frontend-specialist (Michelle) — communicate over a shared DAS as a four-entity squad: two humans + two specialized AIs.

**Key upgrades from base Forge architecture:**
- Forge Mesh: P2P secure communication between instances over shared DAS
- Role-specific fine-tuned models (not general-purpose coding models)
- Hybrid inference with mid-session local ↔ API switching
- Cross-platform support (macOS + Windows)
- Hardware upgrade to support larger specialized models

**Build approach:** Option C — all three workstreams (Core, Mesh, Specialization) run in parallel.

---

## 2. Forge Mesh — Multi-Instance Communication

### 2.1 Physical Architecture

```
┌──────────────────────────┐                    ┌──────────────────────────┐
│  MIKE'S MACHINE (macOS)  │                    │  MICHELLE'S MACHINE (Win)│
│                          │                    │                          │
│  Forge (Backend Role)    │                    │  Forge (Frontend Role)   │
│  - Fine-tuned backend    │                    │  - Fine-tuned frontend   │
│    model (MLX/Metal)     │                    │    model (llama.cpp/CUDA)│
│  - RTAI + Mitosis        │                    │  - RTAI + Mitosis        │
│  - FTAI Rules            │                    │  - FTAI Rules            │
│  - Security red tests    │                    │  - Security red tests    │
│                          │                    │                          │
└────────────┬─────────────┘                    └────────────┬─────────────┘
             │  Thunderbolt / USB-C                          │
             │                                               │
     ┌───────▼───────────────────────────────────────────────▼───────┐
     │                        SHARED DAS                             │
     │                                                               │
     │  /forge-bus/          Encrypted signed message queue          │
     │  /forge-state/        Shared task board + API contracts       │
     │  /repo/               Local git repository                   │
     │  /models/             Fine-tuned model weights (both roles)  │
     │  /training-data/      Fine-tuning datasets per role          │
     │  /ftai/               Shared FTAI rules + red test templates │
     │                                                               │
     └───────────────────────────────────────────────────────────────┘
```

### 2.2 Why DAS Over Network

- **No network stack** — Thunderbolt/USB-C direct to storage. No TCP, DNS, firewall, or venue WiFi isolation.
- **No cloud dependency** — Everything on hardware you physically carry.
- **Speed** — DAS read/write speeds crush any network sync.
- **Physical security** — Attacker needs physical access to the DAS.
- **Shared git is just a local path** — `git remote add shared /Volumes/DAS/repo`.
- **Shared model weights** — Both machines load from same DAS path, no duplicate downloads.
- **Works at hackathons** — No WiFi dependency, no client isolation problems.

### 2.3 Identity & Authentication

Each Forge instance generates an Ed25519 keypair at `forge init`:

```
~/.ftai/identity/
  forge.pub        # Public key (shared with peers)
  forge.key        # Private key (never leaves machine, encrypted at rest)
  peers.toml       # Known peer public keys
```

- Keypair generated via `ed25519-dalek` or `ring` crate
- Peer registration: manual key exchange at first connection (TOFU — Trust On First Use)
- Instance ID: SHA-256 of public key, truncated to 16 chars

### 2.4 Message Protocol

Every message on the bus is a signed, encrypted envelope:

```
┌─────────────────────────────────────┐
│  ENVELOPE                           │
│  ┌─────────────────────────────┐    │
│  │ header:                     │    │
│  │   sender_id: <instance_id>  │    │
│  │   sequence: <monotonic u64> │    │
│  │   timestamp: <unix_ms>      │    │
│  │   type: <message_type>      │    │
│  ├─────────────────────────────┤    │
│  │ payload: <encrypted blob>   │    │
│  ├─────────────────────────────┤    │
│  │ signature: <Ed25519 sig>    │    │
│  └─────────────────────────────┘    │
└─────────────────────────────────────┘
```

**Message types:**
- `task_update` — "Auth API complete, endpoint is /api/auth, schema attached"
- `context_share` — "Working on database layer, here's current schema"
- `request` — "Need a component that accepts these props"
- `alert` — "Changed API response format, update your fetch calls"
- `heartbeat` — Instance alive + current working state
- `ack` — Receipt confirmation

**Security layers:**

| Layer | Implementation | Purpose |
|---|---|---|
| Instance identity | Ed25519 keypair | Know who sent it |
| Message signing | Ed25519 signature over header+payload | Tamper-proof |
| Encryption at rest | XChaCha20-Poly1305 (shared key derived from ECDH) | DAS data protected |
| Sequence numbers | Monotonic counter per sender | Replay protection |
| Context sanitization | `InputSanitizer` pass before LLM ingestion | No prompt injection via bus |
| Message TTL | Configurable expiry (default 1 hour) | Stale messages auto-purged |

### 2.5 Bus Implementation

File-system based message queue on the DAS:

```
/forge-bus/
  outbox/
    <instance_id>/
      000001.msg
      000002.msg
  inbox/
    <instance_id>/
      (symlinks or copies from peer outbox)
  ack/
    <sequence>.ack
```

- **Writer:** Forge writes to its own `outbox/` directory
- **Reader:** Forge watches peer's `outbox/` via `notify` crate (fsnotify)
  - macOS: kqueue
  - Windows: ReadDirectoryChangesW
  - Both handled transparently by `notify` crate
- **Delivery:** At-least-once with ack deduplication
- **Ordering:** Sequence numbers guarantee per-sender ordering
- **Cleanup:** Messages older than TTL auto-purged on read

### 2.6 Shared State (`/forge-state/`)

Single source of truth for project-wide state:

```toml
# /forge-state/project.toml

[tasks]
# Task board — both Forges read/write
[tasks.auth-api]
status = "complete"
owner = "backend"
description = "JWT auth endpoints"
api_contract = "POST /api/auth/login → { token: string, expires: u64 }"
completed_at = "2026-04-01T14:30:00Z"

[tasks.login-ui]
status = "in_progress"
owner = "frontend"
description = "Login form component"
depends_on = ["auth-api"]
started_at = "2026-04-01T14:35:00Z"

[contracts]
# API contracts — backend publishes, frontend consumes
[contracts.auth]
endpoint = "POST /api/auth/login"
request = '{ "email": "string", "password": "string" }'
response = '{ "token": "string", "user": { "id": "string", "name": "string" } }'
updated_by = "backend"
updated_at = "2026-04-01T14:30:00Z"
```

- File-level locking via `fs2` crate (cross-platform advisory locks)
- Both Forges can read anytime, write with lock
- Changes trigger bus notifications to peer

### 2.7 Autonomy Levels (Configurable)

```toml
# forge.toml
[mesh]
autonomy = "suggest"  # "notify" | "suggest" | "auto-act"
```

| Level | Behavior |
|---|---|
| `notify` | "Your teammate finished the auth API. Here's the contract." |
| `suggest` | "Your teammate finished the auth API. I recommend generating the fetch client. Want me to?" |
| `auto-act` | "Your teammate finished the auth API. I've generated the fetch client and types." |

Autonomy level can be changed mid-session via `forge mesh autonomy <level>`.

### 2.8 Forge Roles

```toml
# forge.toml
[identity]
role = "backend"  # or "frontend"
name = "Mike's Forge"
```

Role affects:
- **System prompt** — role-specific instructions injected
- **Bus message interpretation** — backend Forge knows API tasks are its job
- **Task assignment** — unassigned tasks get routed by role affinity
- **Model selection** — loads role-appropriate fine-tuned model

---

## 3. Role-Specialized Fine-Tuned Models

### 3.1 Philosophy

A 27B model fine-tuned exclusively on backend patterns will outperform a 70B general coding model on backend tasks. Breadth traded for depth. Under hackathon constraints where you know exactly what each person builds, this is pure upside.

### 3.2 Backend Model (Mike's Forge)

**Base:** Qwen 3.5 (size TBD based on hardware upgrade)

**Training data sources:**
- Rust idioms, API design patterns, database schemas
- Forge and Serena backend code (Mike's own patterns)
- Security-first patterns from red test corpus
- Auth flows, system architecture decisions
- Error handling patterns (Rust Result/Option idioms)
- SQL/database migration patterns
- REST + GraphQL API design
- Protobuf/gRPC patterns

**What to exclude:**
- Frontend code, CSS, HTML, React
- UI/UX patterns
- Deployment/CI configuration

### 3.3 Frontend Model (Michelle's Forge)

**Base:** Qwen 3.5 (same family, different fine-tune)

**Training data sources:**
- React, Next.js component patterns
- Tailwind CSS, responsive design
- State management (Zustand, Jotai, Redux)
- TypeScript frontend patterns
- Accessibility (WCAG compliance)
- Deploy pipelines (Vercel, Netlify)
- Fetch/API client patterns
- Michelle's own project code

**What to exclude:**
- Backend infrastructure, database schemas
- Rust, system programming
- Server-side auth implementation details

### 3.4 Continuous Specialization Stack

Fine-tuned weights provide **general domain expertise**. On top of that:

```
Layer 1: Fine-tuned model weights (static, trained offline)
         ↓
Layer 2: FTAI Rules (cross-project conventions, loaded at session start)
         ↓
Layer 3: RTAI (runtime learning, adapts during session)
         ↓
Layer 4: Mitosis (cross-session evolution, generates new FTAI rules)
         ↓
Layer 5: KnowledgeSampler (logit-level fact enforcement during generation)
```

Each layer compounds the one below it. Over time, the combination of fine-tuned weights + RTAI/Mitosis personal specialization creates something no generic tool can match.

### 3.5 Training Data Curation (Start Now)

**No hardware needed — pure time value.**

Mike's dataset:
- [ ] Extract best Rust patterns from Forge source (~81 files)
- [ ] Extract backend patterns from Serena's service layer
- [ ] Compile API design examples from FolkTech projects
- [ ] Collect security red test patterns as training examples
- [ ] Curate open-source Rust/API datasets (filtered for quality)

Michelle's dataset:
- [ ] Collect her React/Next.js project code
- [ ] Curate component patterns she uses frequently
- [ ] Compile Tailwind/CSS patterns
- [ ] Collect deploy pipeline configurations
- [ ] Curate open-source frontend datasets (filtered for quality)

---

## 4. Hybrid Inference — Local + API Mid-Session Switching

### 4.1 Design

Forge supports two inference backends simultaneously, switchable mid-session:

```
┌─────────────────────────────────┐
│  Forge Inference Router         │
│                                 │
│  ┌───────────┐  ┌───────────┐  │
│  │ Local     │  │ API       │  │
│  │ (MLX or   │  │ (Claude,  │  │
│  │ llama.cpp)│  │ OpenAI,   │  │
│  │           │  │ etc.)     │  │
│  └───────────┘  └───────────┘  │
│                                 │
│  Mode: auto | local | api      │
└─────────────────────────────────┘
```

### 4.2 Modes

| Mode | Behavior |
|---|---|
| `local` | Always use local model. Fail if unavailable. |
| `api` | Always use API. Fail if no key configured. |
| `auto` | Local for fast iterations, API for complex reasoning. Forge decides based on task complexity heuristic. |

Switch mid-session: `forge model switch <mode>` or automatic based on:
- Token count of current context (large context → API may handle better)
- Task type (security review → API for thoroughness, quick edit → local for speed)
- Local model confidence score (low confidence → escalate to API)

### 4.3 Conversation Continuity

When switching backends mid-session:
- Conversation history is preserved (JSONL transcript)
- System prompt is re-injected with current context
- Tool definitions carry over unchanged
- RTAI/Mitosis state persists (lives on disk, not in model context)

---

## 5. Cross-Platform — Mac + Windows

### 5.1 Platform Abstraction Module

```
src/platform/
  mod.rs          // Trait definitions (InferenceBackend, FileWatcher, PathResolver)
  macos.rs        // MLX Metal, kqueue, /Volumes/ paths
  windows.rs      // llama.cpp CUDA/Vulkan, ReadDirectoryChangesW, drive letters
```

Conditional compilation:
```rust
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
```

### 5.2 Platform Differences

| Component | macOS (Mike) | Windows (Michelle) |
|---|---|---|
| Inference | MLX (Metal GPU) | llama.cpp (CUDA or CPU) |
| File paths | `/Volumes/DAS/forge-bus/` | `E:\forge-bus\` |
| DAS mount | Thunderbolt auto-mount | Thunderbolt, drive letter |
| Shell | zsh | PowerShell / cmd |
| Model accel | Metal | CUDA / Vulkan |
| fsnotify | kqueue | ReadDirectoryChangesW |

### 5.3 Build Strategy

- **Development:** Cross-compile from Mac using `cargo build --target x86_64-pc-windows-gnu`
- **Release:** GitHub Actions CI builds both platform binaries on every push
- **llama.cpp FFI:** Use `cross` crate container for Windows builds with C toolchain
- **Testing:** Get basic `forge --version` on Michelle's machine early

### 5.4 DAS Path Configuration

```toml
# forge.toml (Mac)
[mesh]
bus_path = "/Volumes/ForgeDAS/forge-bus"
state_path = "/Volumes/ForgeDAS/forge-state"
repo_path = "/Volumes/ForgeDAS/repo"

# forge.toml (Windows)
[mesh]
bus_path = "E:\\forge-bus"
state_path = "E:\\forge-state"
repo_path = "E:\\repo"
```

Rust `std::path::PathBuf` handles slash differences automatically.

---

## 6. Hardware Upgrade Path

### 6.1 Current Constraint

16GB unified memory MacBook limits:
- Model size (Qwen 35B-A3B MoE barely fits)
- Can't run Xcode + AI coding assistant + local LLM + browser simultaneously
- Fine-tuning impossible locally

### 6.2 Target Specs

| RAM | What It Unlocks |
|---|---|
| 32GB | Qwen 27B dense comfortably, Forge + Xcode + browser |
| 64GB | 70B-class models quantized, multiple services, fine-tuning small models |
| 96-128GB | Full-size coding models, no compromises, local fine-tuning |

**Recommendation:** 64GB+ unified on M4 Pro/Max for hackathon-grade performance.

### 6.3 Michelle's Machine (Windows)

Needs:
- NVIDIA GPU with sufficient VRAM for fine-tuned frontend model (16GB+ VRAM ideal)
- Thunderbolt port for DAS connection
- Enough RAM for Node.js dev + local model inference

---

## 7. Hackathon Readiness Checklist

### Before First Hackathon

- [ ] Forge core loop functional (inference + tool calling + FTAI rules)
- [ ] Hybrid model switching working (local ↔ API)
- [ ] Cross-platform build passing (Mac + Windows binaries from CI)
- [ ] Michelle running Forge on Windows successfully
- [ ] Forge Mesh bus working over DAS
- [ ] Shared task board functional
- [ ] At least one fine-tuning run completed per role
- [ ] `forge init --template hackathon` scaffolds a full project
- [ ] Deploy hook working (one-command push to Vercel/Railway)
- [ ] 4-hour dry run completed as a team
- [ ] Hardware upgraded

### Day-Of Checklist

- [ ] DAS formatted and tested with both machines
- [ ] Both Forge instances paired (keys exchanged)
- [ ] Fine-tuned models loaded on DAS
- [ ] Git repo initialized on DAS
- [ ] FTAI rules + red test templates on DAS
- [ ] API keys configured (fallback for hybrid inference)
- [ ] `forge mesh status` shows both instances connected
- [ ] Test message sent and received between Forges

### Hackathon Workflow

```
Hour 0:     Plug in DAS, forge mesh connect, brainstorm idea
Hour 0-1:   Define API contracts in shared state, split tasks
Hour 1-8:   Build — Forges communicate task updates automatically
Hour 8-10:  Polish, deploy, prepare demo
Hour 10:    Demo — Forges have been learning all session via RTAI
Post-hack:  Mitosis generates rules from everything learned
```

---

## 8. Build Order & Workstreams

### Option C: All Three in Parallel

**Workstream 1 — Forge Core** (already in progress)
- Complete the 6 parallel + 3 sequential workstreams from build sprint
- Get inference + tool calling + FTAI rules working end-to-end
- Reference: `FORGE-BUILD-PROMPT.md`

**Workstream 2 — Forge Mesh**
- Design message protocol and bus format
- Implement Ed25519 identity at `forge init`
- Build DAS watcher (`notify` crate)
- Define shared state schema
- Build `forge mesh` CLI subcommands
- Implement cross-platform path handling

**Workstream 3 — Forge Specialization**
- Curate training datasets (start immediately — no hardware needed)
- Research fine-tuning approaches for Qwen 3.5 family
- Build `forge train` pipeline (when hardware arrives)
- Validate fine-tuned models against baseline benchmarks

**Workstream 4 — Cross-Platform**
- Add `src/platform/` module with trait abstractions
- Set up CI for dual-platform builds
- Get Forge running on Michelle's Windows machine
- Test DAS connectivity from Windows

### Dependencies

```
Core ──────────► Mesh (needs working Forge to add mesh layer)
Core ──────────► Cross-Platform (needs working Forge to port)
Nothing ───────► Specialization/Data Curation (start today)
Hardware ──────► Fine-Tuning Runs
Mesh + Platform ► Team Dry Run
All ───────────► First Hackathon
```

---

## 9. Security Requirements

All Forge Mesh components follow FolkTech Secure Coding Standard.

### P0 (Never Skipped)

- **Bus message injection** — Unsigned/forged messages must be rejected
- **Replay attacks** — Sequence number validation on every message
- **Prompt injection via bus** — All shared context passes through `InputSanitizer` before LLM ingestion
- **DAS tampering** — Encryption at rest, signature verification on read
- **Key exfiltration** — Private keys encrypted at rest, never written to bus or shared state
- **Path traversal via shared state** — All file paths in contracts/tasks validated and canonicalized

### P1 (Within 48 Hours)

- **Denial of service via bus flooding** — Rate limiting per sender
- **Stale message exploitation** — TTL enforcement, auto-purge
- **Model poisoning via shared training data** — Training data integrity checks (checksums)
- **Privilege escalation via role spoofing** — Role bound to keypair, not configurable by peer

### Minimum Red Tests

- Forge Mesh bus: 12 tests (message signing, encryption, replay, injection, TTL, flooding)
- Shared state: 8 tests (lock contention, path traversal, schema validation, tamper detection)
- Identity system: 6 tests (key generation, peer verification, TOFU, spoofing)
- Context sanitization: 8 tests (prompt injection via task updates, contract payloads, alerts)

---

## Related Documents

- [Forge Architecture](2026-03-27-forge-architecture.md) — Base system design
- [Tool Calling Subsystem](2026-03-27-tool-calling-subsystem-design.md) — Tool system details
- [Architecture Amendments](2026-03-31-architecture-amendments.md) — 9 architecture amendments
- [Agent Loop Anatomy](2026-03-31-agent-loop-anatomy.md) — Loop design deep-dive
