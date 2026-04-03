# Forge Architecture Document

**Date:** 2026-03-27
**Status:** Draft for Review
**Predecessor:** FTAI Terminal Harness (existing codebase at ~/Developer/ftai/)

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [System Architecture](#2-system-architecture)
3. [Inference Layer](#3-inference-layer)
4. [RTAI Adaptation for Code Search](#4-rtai-adaptation-for-code-search)
5. [Tool System](#5-tool-system)
6. [Mitosis + Self-Evolution](#6-mitosis--self-evolution)
7. [Session & Context Management](#7-session--context-management-ake-adaptation)
8. [FTAI Scalability Design](#8-ftai-scalability-design)
9. [FolkTech IDE Integration Assessment](#9-folktech-ide-integration-assessment)
10. [File Structure](#10-file-structure)
11. [Build & Distribution](#11-build--distribution)
12. [Knowledge Grounding Layer (Mitosis-RTAI Inference Integration)](#12-knowledge-grounding-layer-mitosis-rtai-inference-integration)
13. [TUI/UX Specification](#13-tuiux-specification)

---

> **2026-03-31 UPDATE:** Nine architecture amendments based on analysis of modern AI coding assistants
> have been approved. See [`2026-03-31-architecture-amendments.md`](2026-03-31-architecture-amendments.md)
> for: context compaction (P0), tool abort/progress signals (P0), JSONL transcripts,
> glob-matched rule loading, skills system, inference fallback chain, cargo feature gates,
> hook system, and permission denial tracking.

---

## 1. Executive Summary

Forge is a local-first AI development assistant. Single Rust binary. No cloud dependency. It replaces cloud-dependent AI coding assistants for users who want full control over their tooling, running Qwen 3.5 models on consumer hardware.

**What changes from FTAI to Forge:**
- FTAI's llama-server subprocess model becomes direct FFI to llama.cpp (C API)
- Add fastembed-rs for local embeddings (RTAI code search)
- Add Mitosis self-evolution loop (cross-session learning)
- Add AKE session persistence (cross-session context)
- Redesign FTAI format for large codebase scalability
- Keep everything else: rules DSL, tool system, TUI, config, permissions

**What we keep from FTAI verbatim:**
- `rules/` module (lexer, parser, evaluator, builtins) -- 100% reuse
- `tools/` module (all 10 tools + registry) -- 100% reuse
- `permissions/` module -- 100% reuse
- `config/` module -- extend, not rewrite
- `conversation/` module -- extend with AKE, keep engine/parser/prompt
- `plugins/` module -- 100% reuse
- `tui/` module -- 100% reuse
- `formatting/` module -- 100% reuse

**What we add:**
- `inference/` -- direct llama.cpp FFI (replaces `backend/llamacpp.rs` subprocess approach)
- `inference/knowledge_sampler.rs` -- Mitosis-RTAI Knowledge Grounding Layer (logit-level fact enforcement)
- `search/` -- RTAI-adapted code search with fastembed-rs
- `evolution/` -- Mitosis self-evolution engine
- `session/` -- AKE cross-session persistence
- `conversation/adapter.rs` -- ModelAdapter trait (Qwen 3.5 uses XML tool format, not Hermes JSON)
- `conversation/streaming.rs` -- Streaming tool call parser
- `conversation/recovery.rs` -- Three-attempt tool call error recovery

**CRITICAL CORRECTION (from tool calling research):**
- Qwen 3.5 uses XML tool format: `<tool_call><function=name><parameter=key>value</parameter></function></tool_call>`
- NOT the Hermes JSON format assumed in Section 3.5 (which applies to Qwen 3 / Qwen 2.5)
- The official HuggingFace Jinja template has a known bug — Forge must bundle a fixed template
- GBNF grammars constrain the ENTIRE generation (cannot activate mid-stream) — use only for retry
- See companion doc: `2026-03-27-tool-calling-subsystem-design.md` for full details

---

## 2. System Architecture

### 2.1 Component Diagram

```
+------------------------------------------------------------------+
|                         FORGE BINARY                              |
|                                                                   |
|  +----------+  +-------------+  +-----------+  +---------------+  |
|  |   TUI    |  | Conversation|  |   Rules   |  |   Plugins     |  |
|  | ratatui  |  |   Engine    |  |   DSL     |  |   System      |  |
|  +----+-----+  +------+------+  +-----+-----+  +-------+-------+  |
|       |               |               |                 |          |
|  +----v---------------v---------------v-----------------v-------+  |
|  |                    ORCHESTRATOR                               |  |
|  |  User input -> intent -> retrieve context -> prompt model     |  |
|  |  -> parse response -> execute tools -> loop until done        |  |
|  +----+---+---+---+---+---+---+---+---+---+---+---+---+--------+  |
|       |   |   |   |   |   |   |   |   |   |   |   |   |          |
|  +----v-+ | +-v-+ | +-v-+ | +-v-+ | +-v-+ | +-v-+ | +-v-------+  |
|  |Infer.| | |Srch| | |Tool| | |Sess| | |Evol| | |Perm| |Config |  |
|  |Layer | | |RTAI| | |Reg.| | |AKE | | |Mito| | |    | |       |  |
|  +---+--+ | +--+-+ | +-+--+ | +--+-+ | +--+-+ | +----+ +-------+  |
|      |    |    |    |   |    |    |    |    |    |                  |
+------+----+----+----+---+----+----+----+----+----+-----------------+
       |         |        |         |         |
  +----v---+ +---v----+ +-v------+ +v-------+ +v--------+
  |llama.cpp| |fastembed| | Shell | | SQLite | | FS/Git  |
  |  FFI    | |  ONNX   | |Procss | | (3 DBs)| | Ops     |
  |(C API)  | |         | |       | |        | |         |
  +---------+ +---------+ +-------+ +--------+ +---------+
```

### 2.2 Data Flow: "fix the bug in auth.rs"

```
1. User types: "fix the bug in auth.rs"
   |
2. TUI captures input -> Orchestrator.handle_message()
   |
3. Orchestrator:
   a. AKE check: load session summary if continuing from yesterday
   b. RTAI search: embed "bug in auth.rs" -> retrieve top-5 relevant code chunks
   c. Rules pre-check: any rules apply to this context?
   d. Build prompt:
      - System: identity + FTAI.md + tools + rules + memory
      - Context: [RTAI results injected as system context]
      - History: [previous messages]
      - User: "fix the bug in auth.rs"
   |
4. Inference Layer: generate(prompt) -> streaming tokens
   |
5. Parser: detect tool call in output
   e.g., <tool_call>{"name":"file_read","arguments":{"path":"src/auth.rs"}}</tool_call>
   |
6. Permission check: file_read is "safe" -> auto-approve
   |
7. Rules check: no rules block this file_read
   |
8. Tool executes: read src/auth.rs -> return contents
   |
9. Tool result injected as Tool message -> back to step 4
   |
10. Model sees file contents, identifies bug, emits:
    <tool_call>{"name":"file_edit","arguments":{
      "path":"src/auth.rs",
      "old_string":"if token.expired() {",
      "new_string":"if token.is_expired() {"
    }}</tool_call>
    |
11. Permission check: file_edit is "moderate" -> prompt user in TUI
    User approves.
    |
12. Tool executes: edit applied -> return success
    |
13. Model emits final text response: "Fixed the method name..."
    |
14. Evolution engine: log tool call sequence + outcome for Mitosis
    |
15. Session engine: persist conversation snapshot for AKE
```

### 2.3 Memory Budget (16GB Machine)

This is the critical constraint. Every byte matters.

```
Component                    | Budget   | Notes
-----------------------------|----------|----------------------------------
OS + Desktop + Terminal      | 3.0 GB   | macOS baseline
Qwen 3.5 MoE 35B-A3B Q4    | 4.0 GB   | Only 3B params active at a time
KV Cache (32K context, Q8)  | 1.5 GB   | 32K tokens, quantized to Q8
KV Cache (32K, Q4 future)   | 0.8 GB   | With TurboQuant/ctk when available
fastembed model (BGE-small)  | 0.13 GB  | 33M params, ONNX quantized
SQLite (RTAI index, 50K files)| 0.3 GB  | Embeddings on disk, hot pages in cache
Forge binary + runtime       | 0.1 GB   | Rust binary + heap
Headroom                     | 7.0 GB   | Available for compilation, LSP, etc.
-----------------------------|----------|----------------------------------
TOTAL (with Q8 KV)          | 9.0 GB   | Leaves 7GB headroom -- good
TOTAL (Qwen 3.5 9B dense)   | 6.5 GB   | Dense 9B Q4 = ~5GB + 1.5GB KV
```

**Tier 1 (16GB):** Qwen 3.5 35B-A3B (MoE, 3B active) or Qwen 3.5 9B dense. Both fit comfortably.
**Tier 2 (32GB):** Qwen 3.5 27B dense Q4 (~16GB model + 2GB KV). Tight but works.

**KV cache quantization config:**

```toml
# ~/.ftai/config.toml
[inference]
kv_cache_type_k = "q8_0"    # Key cache quantization (llama.cpp -ctk)
kv_cache_type_v = "q8_0"    # Value cache quantization (llama.cpp -ctv)
# Future: when TurboQuant lands in llama.cpp
# kv_cache_type_k = "q4_0"  # 6x compression vs f16
# kv_cache_type_v = "q4_0"
```

---

## 3. Inference Layer

### 3.1 Architecture Decision: Direct FFI vs Subprocess

The existing FTAI uses `LlamaCppServer` -- spawns `llama-server` as a subprocess, talks HTTP. This works but adds latency, a separate process to manage, and port conflicts.

Forge switches to **direct FFI to llama.cpp's C API**. The Rust binary links `libllama.a` at compile time.

**Why FFI wins:**
- No subprocess lifecycle management
- No HTTP overhead (saves ~5ms per call)
- Direct access to KV cache state, token probabilities, sampling
- Single process = simpler deployment
- Can implement custom sampling strategies (constrained generation for tool calls)

**Why it's harder:**
- Must build llama.cpp from source as part of `build.rs`
- C FFI is unsafe Rust -- needs careful wrapper
- Model loading blocks the thread (must be async-wrapped)

### 3.2 llama.cpp FFI Binding

We use the `llama-cpp-sys-2` crate (raw C bindings) and write our own safe wrapper on top. We do NOT use `llama-cpp-2` (the high-level crate) because we need fine-grained control over KV cache management, sampling, and grammar-constrained generation.

```rust
// src/inference/mod.rs
pub mod ffi;        // Raw FFI wrappers
pub mod context;    // LlamaContext -- safe wrapper
pub mod sampler;    // Sampling strategies
pub mod grammar;    // GBNF grammar for constrained tool call output
pub mod model;      // Model loading, info, config

// Re-export the backend trait (kept from FTAI)
pub use crate::backend::types::{ModelBackend, ChatRequest, ChatResponse, Token, TokenStream};
```

```rust
// src/inference/context.rs

use llama_cpp_sys_2 as ffi;
use std::ptr::NonNull;

/// Safe wrapper around llama_context
pub struct LlamaContext {
    ctx: NonNull<ffi::llama_context>,
    model: NonNull<ffi::llama_model>,
    n_ctx: u32,
    n_batch: u32,
}

// SAFETY: llama_context is thread-safe when accessed through the C API
// with proper synchronization. We enforce single-threaded access via &mut self.
unsafe impl Send for LlamaContext {}

impl LlamaContext {
    /// Load model and create context
    pub fn new(config: &InferenceConfig) -> Result<Self> {
        let model_params = unsafe {
            let mut params = ffi::llama_model_default_params();
            params.n_gpu_layers = config.gpu_layers;
            // KV cache quantization
            params.type_k = config.kv_type_k.to_ffi();
            params.type_v = config.kv_type_v.to_ffi();
            params
        };

        let c_path = std::ffi::CString::new(config.model_path.as_str())?;
        let model = unsafe { ffi::llama_load_model_from_file(c_path.as_ptr(), model_params) };
        let model = NonNull::new(model)
            .ok_or_else(|| anyhow::anyhow!("Failed to load model: {}", config.model_path))?;

        let ctx_params = unsafe {
            let mut params = ffi::llama_context_default_params();
            params.n_ctx = config.context_length;
            params.n_batch = config.batch_size;
            params.n_threads = config.threads;
            params.flash_attn = config.flash_attention;
            params
        };

        let ctx = unsafe { ffi::llama_new_context_with_model(model.as_ptr(), ctx_params) };
        let ctx = NonNull::new(ctx)
            .ok_or_else(|| anyhow::anyhow!("Failed to create context"))?;

        Ok(Self {
            ctx,
            model,
            n_ctx: config.context_length,
            n_batch: config.batch_size,
        })
    }

    /// Tokenize text
    pub fn tokenize(&self, text: &str, add_bos: bool) -> Vec<i32> {
        let c_text = std::ffi::CString::new(text).unwrap_or_default();
        let max_tokens = text.len() as i32 + 128;
        let mut tokens = vec![0i32; max_tokens as usize];

        let n = unsafe {
            ffi::llama_tokenize(
                self.model.as_ptr(),
                c_text.as_ptr(),
                text.len() as i32,
                tokens.as_mut_ptr(),
                max_tokens,
                add_bos,
                false, // special tokens
            )
        };

        tokens.truncate(n.max(0) as usize);
        tokens
    }

    /// Decode a batch of tokens (KV cache updated in place)
    pub fn decode_batch(&mut self, tokens: &[i32], pos: i32) -> Result<()> {
        let batch = unsafe {
            ffi::llama_batch_get_one(
                tokens.as_ptr() as *mut i32,
                tokens.len() as i32,
                pos,
                0, // seq_id
            )
        };

        let result = unsafe { ffi::llama_decode(self.ctx.as_ptr(), batch) };
        if result != 0 {
            anyhow::bail!("llama_decode failed with code {result}");
        }
        Ok(())
    }

    /// Get logits for the last token
    pub fn get_logits(&self) -> &[f32] {
        let n_vocab = unsafe { ffi::llama_n_vocab(self.model.as_ptr()) } as usize;
        unsafe {
            let ptr = ffi::llama_get_logits(self.ctx.as_ptr());
            std::slice::from_raw_parts(ptr, n_vocab)
        }
    }

    /// Clear KV cache (for new conversation)
    pub fn clear_kv_cache(&mut self) {
        unsafe { ffi::llama_kv_cache_clear(self.ctx.as_ptr()) };
    }

    /// Get number of tokens currently in KV cache
    pub fn kv_cache_used(&self) -> u32 {
        unsafe { ffi::llama_get_kv_cache_used_cells(self.ctx.as_ptr()) as u32 }
    }

    pub fn context_length(&self) -> u32 {
        self.n_ctx
    }
}

impl Drop for LlamaContext {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_free(self.ctx.as_ptr());
            ffi::llama_free_model(self.model.as_ptr());
        }
    }
}
```

### 3.3 Inference Config

```rust
// src/inference/model.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceConfig {
    pub model_path: String,
    pub context_length: u32,       // Default: 32768
    pub batch_size: u32,           // Default: 512
    pub threads: u32,              // Default: num_cpus / 2
    pub gpu_layers: i32,           // Default: -1 (all layers to GPU)
    pub flash_attention: bool,     // Default: true on Metal
    pub kv_type_k: KvQuantType,   // Default: Q8_0
    pub kv_type_v: KvQuantType,   // Default: Q8_0
    pub rope_scaling: Option<f32>, // For extended context
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum KvQuantType {
    F16,
    Q8_0,
    Q4_0,  // TurboQuant -- future
    Q4_1,  // TurboQuant -- future
}

impl KvQuantType {
    pub fn to_ffi(self) -> i32 {
        match self {
            Self::F16  => 1,  // GGML_TYPE_F16
            Self::Q8_0 => 8,  // GGML_TYPE_Q8_0
            Self::Q4_0 => 2,  // GGML_TYPE_Q4_0
            Self::Q4_1 => 3,  // GGML_TYPE_Q4_1
        }
    }
}
```

### 3.4 Streaming Response

```rust
// src/inference/sampler.rs

use tokio::sync::mpsc;
use crate::backend::types::{Token, ChatResponse, TokenUsage, StopReason};

pub struct Sampler {
    temperature: f32,
    top_p: f32,
    top_k: i32,
    repeat_penalty: f32,
    grammar: Option<GbnfGrammar>,  // For constrained generation
}

impl Sampler {
    /// Generate tokens, streaming each one through the channel
    pub async fn generate_stream(
        &self,
        ctx: &mut LlamaContext,
        prompt_tokens: &[i32],
        max_tokens: usize,
        tx: mpsc::Sender<Token>,
    ) -> Result<ChatResponse> {
        // Decode prompt
        ctx.decode_batch(prompt_tokens, 0)?;

        let mut generated_tokens = Vec::new();
        let mut pos = prompt_tokens.len() as i32;

        for _ in 0..max_tokens {
            let logits = ctx.get_logits();
            let token_id = self.sample(logits, &generated_tokens);

            // Check for EOS
            if self.is_eos(ctx, token_id) {
                tx.send(Token { text: String::new(), is_final: true }).await.ok();
                return Ok(self.build_response(ctx, prompt_tokens, &generated_tokens, StopReason::EndOfText));
            }

            // Detokenize and stream
            let text = ctx.token_to_str(token_id);
            tx.send(Token { text: text.clone(), is_final: false }).await.ok();

            generated_tokens.push(token_id);

            // Feed token back into context
            ctx.decode_batch(&[token_id], pos)?;
            pos += 1;

            // Check if tool call is complete (for early stopping)
            if self.grammar.is_some() && self.grammar_complete() {
                tx.send(Token { text: String::new(), is_final: true }).await.ok();
                return Ok(self.build_response(ctx, prompt_tokens, &generated_tokens, StopReason::ToolCall));
            }
        }

        tx.send(Token { text: String::new(), is_final: true }).await.ok();
        Ok(self.build_response(ctx, prompt_tokens, &generated_tokens, StopReason::MaxTokens))
    }
}
```

### 3.5 Tool Calling Strategy for Local Models

This is the hardest problem. Local models are worse at structured tool calling than frontier models. We use a three-layer approach:

**Layer 1: Qwen 3.5 Native Tool Calling**

Qwen 3.5 supports Hermes-style tool calling natively. The model was trained on this format:

```
<|im_start|>system
You are a helpful assistant with access to the following tools:

[{"type": "function", "function": {"name": "file_read", "description": "...", "parameters": {...}}}]

When you need to use a tool, emit:
<tool_call>
{"name": "tool_name", "arguments": {...}}
</tool_call>
<|im_end|>
```

The existing `ToolCallParser` in `src/conversation/parser.rs` already handles this format. No changes needed.

**Layer 2: GBNF Grammar Constrained Generation**

When the model starts emitting `<tool_call>`, we switch to grammar-constrained generation using llama.cpp's built-in GBNF grammar support. This forces the model to emit valid JSON matching our tool call schema.

```rust
// src/inference/grammar.rs

/// GBNF grammar that constrains output to valid tool call JSON
pub fn tool_call_grammar(tool_names: &[&str]) -> String {
    let names = tool_names.iter()
        .map(|n| format!(r#""{n}""#))
        .collect::<Vec<_>>()
        .join(" | ");

    format!(r#"
root   ::= "<tool_call>\n" toolcall "\n</tool_call>"
toolcall ::= "{{" ws "\"name\":" ws name "," ws "\"arguments\":" ws object ws "}}"
name   ::= ({names})
object ::= "{{" ws (pair ("," ws pair)*)? ws "}}"
pair   ::= string ":" ws value
value  ::= string | number | "true" | "false" | "null" | object | array
array  ::= "[" ws (value ("," ws value)*)? ws "]"
string ::= "\"" [^"\\]* "\""
number ::= "-"? [0-9]+ ("." [0-9]+)?
ws     ::= [ \t\n]*
"#)
}
```

**How it works in practice:**
1. Model generates freely until we detect `<tool_call>` in the output stream
2. At that point, activate the GBNF grammar
3. Grammar constrains all subsequent tokens to valid JSON matching our schema
4. When `</tool_call>` is emitted, deactivate grammar
5. Parse the guaranteed-valid JSON

This eliminates malformed tool calls entirely. The model only needs to decide WHEN to call a tool and WHICH tool -- the grammar handles the structural correctness.

**Layer 3: Retry with Correction**

If the model emits something that looks like a tool call but fails parsing (e.g., the grammar wasn't activated in time), we inject a correction message:

```
<|im_start|>system
Your last tool call was malformed. Here is what you tried:
[raw output]

Please try again. Use this exact format:
<tool_call>
{"name": "tool_name", "arguments": {"param": "value"}}
</tool_call>
<|im_end|>
```

Maximum 2 retries before giving up and showing the raw output to the user.

### 3.6 MLX Bridge Design for Mac

For Apple Silicon, MLX offers better performance than llama.cpp for safetensors models. Three options evaluated:

| Option | Approach | Latency | Complexity | Maintenance |
|--------|----------|---------|------------|-------------|
| A | Python subprocess (`mlx_lm.generate`) | +50ms startup | Low | Python dep |
| B | Swift subprocess via XPC | +10ms | Medium | Swift toolchain |
| C | MLX C++ API via FFI | 0ms | High | Must track MLX API |

**Decision: Option A (Python subprocess) for v1, Option C as v2 upgrade.**

Rationale: MLX's Python API is the most stable and well-documented. The +50ms startup is a one-time cost (we keep the Python process alive as a long-running server). For v2, MLX is adding a C API (`mlx-c`) which would allow direct FFI like llama.cpp.

```rust
// src/inference/mlx.rs

use tokio::process::Command;
use tokio::io::{AsyncBufReadExt, BufReader};

/// MLX backend via persistent Python subprocess
pub struct MlxBackend {
    process: Option<tokio::process::Child>,
    stdin: Option<tokio::process::ChildStdin>,
    stdout: Option<BufReader<tokio::process::ChildStdout>>,
    model_path: Option<String>,
}

impl MlxBackend {
    /// Start the MLX inference server (Python script bundled with Forge)
    pub async fn start(&mut self, model_path: &str) -> Result<()> {
        let script = self.bundled_mlx_server_path()?;
        let mut child = Command::new("python3")
            .arg(&script)
            .arg("--model").arg(model_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        self.stdin = child.stdin.take();
        self.stdout = child.stdout.take().map(BufReader::new);
        self.process = Some(child);
        self.model_path = Some(model_path.to_string());

        // Wait for "ready" signal
        self.wait_for_ready().await
    }

    /// Send a request via stdin, read streaming response from stdout
    /// Protocol: JSON-lines (one JSON object per line)
    pub async fn generate_stream(
        &mut self,
        request: &ChatRequest,
        tx: mpsc::Sender<Token>,
    ) -> Result<ChatResponse> {
        let json = serde_json::to_string(request)?;
        // Write request as single line
        let stdin = self.stdin.as_mut().unwrap();
        use tokio::io::AsyncWriteExt;
        stdin.write_all(json.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;

        // Read streaming tokens
        let stdout = self.stdout.as_mut().unwrap();
        let mut line = String::new();
        let mut full_text = String::new();

        loop {
            line.clear();
            stdout.read_line(&mut line).await?;
            let msg: MlxMessage = serde_json::from_str(line.trim())?;
            match msg {
                MlxMessage::Token { text } => {
                    full_text.push_str(&text);
                    tx.send(Token { text, is_final: false }).await.ok();
                }
                MlxMessage::Done { usage } => {
                    tx.send(Token { text: String::new(), is_final: true }).await.ok();
                    return Ok(/* build response */);
                }
                MlxMessage::Error { message } => {
                    anyhow::bail!("MLX error: {message}");
                }
            }
        }
    }
}
```

The bundled Python script (`scripts/mlx_server.py`, ~100 lines) uses `mlx_lm` to load models and stream tokens. It reads JSON requests from stdin and writes JSON-lines to stdout.

---

## 4. RTAI Adaptation for Code Search

### 4.1 Overview

RTAI (Real-Time AI Retrieval) in Serena provides sub-20ms codebase search. We adapt this for Forge using:
- **fastembed-rs** for embeddings (ONNX Runtime, no Python)
- **SQLite** for vector storage (no external vector DB)
- **notify** crate for filesystem watching

### 4.2 Indexing Strategy

```rust
// src/search/mod.rs
pub mod indexer;    // File walker, chunker, embedding pipeline
pub mod store;      // SQLite storage
pub mod query;      // Search query execution
pub mod watcher;    // File change detection

// src/search/indexer.rs

/// Chunking strategy per language
pub enum ChunkStrategy {
    /// Parse AST and chunk at function/method/class boundaries
    Semantic(Language),
    /// Fixed-size sliding window (fallback for unsupported languages)
    SlidingWindow { size: usize, overlap: usize },
}

/// A single indexed chunk
pub struct CodeChunk {
    pub file_path: String,
    pub chunk_type: ChunkType,    // Function, Class, Method, Module, Block
    pub symbol_name: Option<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    pub embedding: Vec<f32>,      // 384-dim from BGE-small
    pub file_mtime: u64,          // For incremental re-indexing
}

pub enum ChunkType {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Impl,
    Trait,
    Module,
    Block,           // Sliding window fallback
}
```

**Language-aware chunking:**

| Language | Chunking Strategy | Parser |
|----------|------------------|--------|
| Rust | `fn`, `struct`, `enum`, `impl`, `trait`, `mod` | tree-sitter-rust |
| TypeScript/JS | `function`, `class`, `const =`, `export` | tree-sitter-typescript |
| Python | `def`, `class`, `async def` | tree-sitter-python |
| Swift | `func`, `class`, `struct`, `enum`, `protocol` | tree-sitter-swift |
| Go | `func`, `type`, `interface` | tree-sitter-go |
| Other | Sliding window (40 lines, 10 overlap) | None |

We use `tree-sitter` for parsing because:
- Rust bindings are mature (`tree-sitter` crate)
- Incremental parsing (fast re-index on file change)
- Already used by many code intelligence tools
- Language grammars are small (~100KB each)

### 4.3 Embedding Pipeline

```rust
// src/search/indexer.rs

use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};

pub struct CodeIndexer {
    embedder: TextEmbedding,
    store: SearchStore,
    watcher: Option<RecommendedWatcher>,
}

impl CodeIndexer {
    pub fn new(db_path: &Path) -> Result<Self> {
        // BGE-small-en-v1.5: 384-dim, 33M params, ~130MB ONNX
        // Downloaded once to ~/.ftai/models/bge-small-en-v1.5/
        let embedder = TextEmbedding::try_new(InitOptions {
            model_name: EmbeddingModel::BGESmallENV15,
            cache_dir: dirs::home_dir().unwrap().join(".ftai/models"),
            show_download_progress: true,
            ..Default::default()
        })?;

        let store = SearchStore::open(db_path)?;
        Ok(Self { embedder, store, watcher: None })
    }

    /// Index a project directory
    pub async fn index_project(&self, root: &Path, progress: impl Fn(IndexProgress)) -> Result<()> {
        let files = self.discover_files(root)?;
        let total = files.len();

        for (i, file_path) in files.iter().enumerate() {
            // Skip if file hasn't changed since last index
            let mtime = file_mtime(file_path)?;
            if self.store.is_current(file_path, mtime)? {
                continue;
            }

            // Chunk the file
            let chunks = self.chunk_file(file_path)?;

            // Batch embed (fastembed handles batching internally)
            let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
            let embeddings = self.embedder.embed(texts, None)?;

            // Store
            for (chunk, embedding) in chunks.into_iter().zip(embeddings) {
                self.store.upsert_chunk(file_path, &chunk, &embedding, mtime)?;
            }

            progress(IndexProgress { current: i + 1, total, file: file_path.clone() });
        }

        // Delete chunks for files that no longer exist
        self.store.prune_deleted_files(root)?;
        Ok(())
    }

    fn discover_files(&self, root: &Path) -> Result<Vec<PathBuf>> {
        let walker = ignore::WalkBuilder::new(root)
            .hidden(true)           // Respect .gitignore
            .git_ignore(true)
            .git_global(true)
            .filter_entry(|e| {
                let name = e.file_name().to_str().unwrap_or("");
                // Skip known non-code directories
                !matches!(name, "node_modules" | "target" | ".git" | "build" | "dist" |
                          "__pycache__" | ".venv" | "vendor" | ".next")
            })
            .build();

        let mut files = Vec::new();
        for entry in walker {
            let entry = entry?;
            if entry.file_type().map_or(false, |t| t.is_file()) {
                if is_code_file(entry.path()) {
                    files.push(entry.path().to_path_buf());
                }
            }
        }
        Ok(files)
    }
}
```

### 4.4 SQLite Storage Schema

```sql
-- ~/.ftai/projects/<project_hash>/search.db

CREATE TABLE chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL,
    chunk_type TEXT NOT NULL,       -- 'function', 'class', etc.
    symbol_name TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB NOT NULL,        -- f32 le_bytes, 384 dims = 1536 bytes
    file_mtime INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX idx_chunks_file ON chunks(file_path);
CREATE INDEX idx_chunks_symbol ON chunks(symbol_name) WHERE symbol_name IS NOT NULL;
CREATE INDEX idx_chunks_type ON chunks(chunk_type);

-- Metadata table for index state
CREATE TABLE index_meta (
    key TEXT PRIMARY KEY,
    value TEXT
);
```

**Search implementation: brute-force cosine similarity.**

For codebases up to 100K chunks (~50K files), brute-force cosine similarity over 384-dim vectors is fast enough:
- 100K chunks * 384 dims * 4 bytes = ~150MB in memory (we load on demand from SQLite)
- Cosine similarity: 100K comparisons of 384-dim vectors takes ~5ms on Apple Silicon (SIMD)

We do NOT need HNSW or a vector database. SQLite + in-memory cosine is sufficient for the scale we're targeting.

```rust
// src/search/query.rs

pub struct SearchQuery {
    pub text: String,
    pub top_k: usize,           // Default: 10
    pub file_filter: Option<String>,  // Glob pattern
    pub chunk_type_filter: Option<ChunkType>,
}

pub struct SearchResult {
    pub file_path: String,
    pub symbol_name: Option<String>,
    pub chunk_type: ChunkType,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    pub score: f32,              // Cosine similarity 0.0 - 1.0
}

pub async fn search(
    indexer: &CodeIndexer,
    query: &SearchQuery,
) -> Result<Vec<SearchResult>> {
    // 1. Embed the query
    let query_embedding = indexer.embedder.embed(vec![query.text.as_str()], None)?;
    let query_vec = &query_embedding[0];

    // 2. Load all embeddings from SQLite (cached in memory after first load)
    let chunks = indexer.store.all_chunks_with_embeddings()?;

    // 3. Compute cosine similarity
    let mut scored: Vec<(f32, &StoredChunk)> = chunks.iter()
        .filter(|c| {
            query.file_filter.as_ref().map_or(true, |f| glob_match(f, &c.file_path))
        })
        .map(|c| (cosine_similarity(query_vec, &c.embedding), c))
        .collect();

    // 4. Sort by score descending, take top_k
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(query.top_k);

    Ok(scored.into_iter().map(|(score, chunk)| SearchResult {
        file_path: chunk.file_path.clone(),
        symbol_name: chunk.symbol_name.clone(),
        chunk_type: chunk.chunk_type,
        start_line: chunk.start_line,
        end_line: chunk.end_line,
        content: chunk.content.clone(),
        score,
    }).collect())
}

/// SIMD-friendly cosine similarity
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    dot / (norm_a.sqrt() * norm_b.sqrt() + 1e-8)
}
```

### 4.5 File Watcher (Re-indexing)

```rust
// src/search/watcher.rs

use notify::{RecommendedWatcher, RecursiveMode, Watcher, Event, EventKind};
use tokio::sync::mpsc;

pub struct FileWatcher {
    watcher: RecommendedWatcher,
    rx: mpsc::Receiver<Vec<PathBuf>>,
}

impl FileWatcher {
    pub fn new(root: &Path) -> Result<Self> {
        let (tx, rx) = mpsc::channel(100);

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                        let paths: Vec<PathBuf> = event.paths.into_iter()
                            .filter(|p| is_code_file(p))
                            .collect();
                        if !paths.is_empty() {
                            tx.blocking_send(paths).ok();
                        }
                    }
                    _ => {}
                }
            }
        })?;

        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self { watcher, rx })
    }

    /// Drain pending changed files (debounced -- waits 500ms for more changes)
    pub async fn drain_changes(&mut self) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        // Wait for first change
        if let Some(paths) = self.rx.recv().await {
            changed.extend(paths);
        }
        // Debounce: collect more changes for 500ms
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        loop {
            match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Ok(Some(paths)) => changed.extend(paths),
                _ => break,
            }
        }
        // Deduplicate
        changed.sort();
        changed.dedup();
        changed
    }
}
```

### 4.6 Scaling to 100K+ Files

**Problem:** Large monorepos (100K+ files, 500K+ chunks) make brute-force cosine slow and memory-heavy.

**Solution: Two-tier search.**

Tier 1: Coarse filter via SQLite FTS5 (text search). Fast, no embeddings needed.
Tier 2: Semantic re-rank via embeddings on the top-100 FTS results.

```sql
-- Add FTS5 virtual table alongside embeddings
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    file_path,
    symbol_name,
    content,
    content=chunks,
    content_rowid=id
);

-- Triggers to keep FTS in sync
CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO chunks_fts(rowid, file_path, symbol_name, content)
    VALUES (new.id, new.file_path, new.symbol_name, new.content);
END;
```

Search flow for large codebases:
1. FTS5 query: `chunks_fts MATCH 'auth token expired'` -> top 100 results
2. Load embeddings for those 100 chunks only
3. Cosine similarity re-rank -> top 10 results

This keeps search under 20ms even at 500K chunks.

---

## 5. Tool System

### 5.1 Existing Tools (Kept from FTAI)

All 10 existing tools in `src/tools/` are reused verbatim:

| Tool | Safety Level | Description |
|------|-------------|-------------|
| `bash` | Dangerous | Execute shell commands |
| `file_read` | Safe | Read files with line ranges |
| `file_write` | Moderate | Create/overwrite files |
| `file_edit` | Moderate | String replacement edits |
| `glob` | Safe | File pattern matching |
| `grep` | Safe | Regex content search |
| `git` | Moderate | Git operations (commit, diff, log, status) |
| `web_fetch` | Safe | HTTP GET + HTML-to-markdown |
| `ask_user` | Safe | Prompt for input |
| `request_permissions` | Safe | Request elevated permissions |

### 5.2 New Tools for Forge

```rust
// src/tools/search_semantic.rs -- NEW
pub struct SemanticSearchTool;

impl Tool for SemanticSearchTool {
    fn name(&self) -> &str { "search_semantic" }
    fn description(&self) -> &str {
        "Search the codebase using natural language. Returns relevant code chunks \
         ranked by semantic similarity. Use this when grep isn't enough -- e.g., \
         'find the authentication middleware' or 'where is rate limiting implemented'."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Natural language search query" },
                "top_k": { "type": "integer", "description": "Number of results", "default": 5 },
                "file_filter": { "type": "string", "description": "Glob pattern to filter files" }
            },
            "required": ["query"]
        })
    }
    // execute() calls search::query::search()
}

// src/tools/list_dir.rs -- NEW (split from glob for full coding assistant parity)
pub struct ListDirTool;

impl Tool for ListDirTool {
    fn name(&self) -> &str { "list_dir" }
    fn description(&self) -> &str {
        "List files and directories in a path. Returns names with type indicators \
         (/ for dirs). Use this for directory exploration."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path to list" },
                "recursive": { "type": "boolean", "default": false }
            },
            "required": ["path"]
        })
    }
}

// src/tools/agent_spawn.rs -- NEW
pub struct AgentSpawnTool;

impl Tool for AgentSpawnTool {
    fn name(&self) -> &str { "agent_spawn" }
    fn description(&self) -> &str {
        "Spawn a sub-agent with an isolated context to perform a focused task. \
         The sub-agent has access to all tools but operates in its own conversation. \
         Use for parallel investigation or when the current context is too large."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "Task description for the sub-agent" },
                "context_files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "File paths to include in the sub-agent's context"
                }
            },
            "required": ["task"]
        })
    }
}
```

### 5.3 Safety Levels

```rust
// src/permissions/classifier.rs (extended)

pub enum SafetyLevel {
    Safe,       // Auto-approve
    Moderate,   // Prompt user (with "always allow" option)
    Dangerous,  // Always prompt, no "always allow"
}

pub fn classify_tool_call(name: &str, args: &Value, ctx: &ToolContext) -> SafetyLevel {
    match name {
        // Always safe: reads only
        "file_read" | "glob" | "grep" | "list_dir" | "search_semantic" |
        "ask_user" | "request_permissions" => SafetyLevel::Safe,

        // Moderate: writes within project
        "file_write" | "file_edit" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if is_within_project(path, &ctx.project_path) {
                SafetyLevel::Moderate
            } else {
                SafetyLevel::Dangerous
            }
        }

        // Git: depends on operation
        "git" => {
            let op = args.get("operation").and_then(|v| v.as_str()).unwrap_or("");
            match op {
                "status" | "diff" | "log" | "branch_list" => SafetyLevel::Safe,
                "commit" | "checkout" | "branch_create" => SafetyLevel::Moderate,
                "push" | "reset" | "force_push" => SafetyLevel::Dangerous,
                _ => SafetyLevel::Moderate,
            }
        }

        // Bash: always dangerous (unless allow-listed pattern)
        "bash" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if is_safe_command(cmd) {
                SafetyLevel::Moderate  // cargo test, npm test, etc.
            } else {
                SafetyLevel::Dangerous
            }
        }

        // Web: safe (read-only HTTP GET)
        "web_fetch" => SafetyLevel::Safe,

        // Sub-agent: moderate (it can execute tools internally)
        "agent_spawn" => SafetyLevel::Moderate,

        // Plugin tools: moderate by default
        _ if name.starts_with("plugin:") => SafetyLevel::Moderate,

        // Unknown: dangerous
        _ => SafetyLevel::Dangerous,
    }
}

fn is_safe_command(cmd: &str) -> bool {
    let safe_patterns = [
        "cargo test", "cargo check", "cargo build", "cargo clippy",
        "npm test", "npm run test", "npx tsc",
        "python -m pytest", "go test",
        "git status", "git diff", "git log",
        "ls", "pwd", "cat", "head", "tail", "wc",
    ];
    safe_patterns.iter().any(|p| cmd.starts_with(p))
}
```

### 5.4 Tool Call Prompt Engineering

The system prompt must teach Qwen 3.5 how and when to use tools. This is injected by `build_system_prompt()`:

```
# Tool Usage Guidelines

You have access to tools for interacting with the user's codebase. Use them proactively:

1. ALWAYS read files before editing them. Never guess at file contents.
2. Use grep/glob to find files before reading them. Don't assume paths.
3. Use search_semantic when you need conceptual search ("where is auth handled").
4. Use grep when you need exact text search ("find all TODO comments").
5. After editing a file, verify the edit worked by reading the file back.
6. For multi-step tasks, plan your tool calls. Read first, then edit.

When you need to use a tool, emit it in this format:
<tool_call>
{"name": "tool_name", "arguments": {"param1": "value1"}}
</tool_call>

You can emit multiple tool calls in a single response. They will execute in order.

CRITICAL: After receiving tool results, continue your work. Do not ask the user
for confirmation unless you genuinely need input. Be autonomous.
```

---

## 6. Mitosis + Self-Evolution

### 6.1 Concept

MiniMax M2.7 demonstrated that an AI system can improve its own performance through iterative self-analysis: attempt task, analyze outcome, modify approach, store improvement, apply next time.

Forge adapts this as **cross-session learning**. Instead of modifying model weights (which we can't do with local inference), we modify the scaffold: FTAI rules, tool call patterns, prompt templates, and context injection strategies.

### 6.2 Architecture

```
Session N: User asks "fix auth bug"
  |
  v
Forge attempts: reads files, generates fix, applies edit
  |
  v
Outcome captured:
  - Tool call sequence: [grep -> file_read -> file_edit -> bash(cargo test)]
  - Success: tests pass after edit? Yes/No
  - User feedback: did user accept, reject, or modify the fix?
  - Token efficiency: how many tokens used? How many retries?
  |
  v
Evolution Engine (runs at session end, async):
  1. Analyze: "file_edit failed because old_string didn't match"
  2. Pattern: "When editing Rust files, always read the file first to get exact text"
  3. Generate FTAI rule:
     rule "read-before-edit-rust" {
       on tool:file_edit
       when extension(path) == "rs"
       require tool_was_called("file_read", path) in session
       reason "Always read Rust files before editing to ensure exact string match"
     }
  4. Store in ~/.ftai/evolution/rules/
  5. Next session: rule is loaded and enforced
```

### 6.3 Data Model

```rust
// src/evolution/mod.rs
pub mod analyzer;    // Session outcome analysis
pub mod generator;   // FTAI rule generation
pub mod store;       // Evolution history storage

// src/evolution/analyzer.rs

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionOutcome {
    pub session_id: String,
    pub project: String,
    pub timestamp: u64,
    pub task_description: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub success: OutcomeType,
    pub user_feedback: Option<UserFeedback>,
    pub total_tokens: usize,
    pub retries: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub arguments_summary: String,  // Truncated args for storage
    pub result_type: ToolResultType,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ToolResultType {
    Success,
    Error(String),
    Timeout,
    Rejected,       // User rejected the tool call
    RuleBlocked,    // Rules engine blocked it
}

#[derive(Debug, Serialize, Deserialize)]
pub enum OutcomeType {
    Success,                // Task completed, user accepted
    PartialSuccess,         // Task completed but user modified result
    Failure(String),        // Task failed
    Abandoned,              // User gave up / switched task
}

#[derive(Debug, Serialize, Deserialize)]
pub enum UserFeedback {
    Accepted,
    Modified,               // User made changes after AI's attempt
    Rejected,
    NoFeedback,             // Session ended without clear signal
}
```

### 6.4 Evolution Engine

```rust
// src/evolution/generator.rs

pub struct EvolutionEngine {
    store: EvolutionStore,
    min_sessions_for_analysis: usize,  // Don't generate rules from 1 data point
}

impl EvolutionEngine {
    /// Run at end of session (async, non-blocking)
    pub async fn analyze_and_evolve(&self, outcome: &SessionOutcome) -> Result<Vec<GeneratedRule>> {
        self.store.save_outcome(outcome)?;

        // Need at least 3 sessions with the same pattern before generating a rule
        if self.store.session_count()? < self.min_sessions_for_analysis {
            return Ok(vec![]);
        }

        let mut rules = Vec::new();

        // Pattern 1: Repeated tool failures
        rules.extend(self.detect_repeated_failures()?);

        // Pattern 2: Tool call ordering patterns
        rules.extend(self.detect_ordering_patterns()?);

        // Pattern 3: Project-specific patterns
        rules.extend(self.detect_project_patterns(outcome.project.as_str())?);

        // Write generated rules to ~/.ftai/evolution/rules/
        for rule in &rules {
            self.store.save_generated_rule(rule)?;
        }

        Ok(rules)
    }

    /// Detect patterns like "file_edit after file_read succeeds more often"
    fn detect_ordering_patterns(&self) -> Result<Vec<GeneratedRule>> {
        let sessions = self.store.recent_sessions(20)?;
        let mut patterns: HashMap<(String, String), (usize, usize)> = HashMap::new(); // (before, after) -> (success, total)

        for session in &sessions {
            for window in session.tool_calls.windows(2) {
                let key = (window[0].tool_name.clone(), window[1].tool_name.clone());
                let entry = patterns.entry(key).or_insert((0, 0));
                entry.1 += 1;
                if matches!(window[1].result_type, ToolResultType::Success) {
                    entry.0 += 1;
                }
            }
        }

        let mut rules = Vec::new();
        for ((before, after), (success, total)) in &patterns {
            if *total >= 5 {
                let success_rate = *success as f64 / *total as f64;
                let inverse = self.success_rate_without_predecessor(after, before, &sessions);

                // If doing A before B is significantly better than not doing A before B
                if success_rate > 0.8 && inverse < 0.5 {
                    rules.push(GeneratedRule {
                        name: format!("{before}-before-{after}"),
                        source: RuleSource::Evolution,
                        confidence: success_rate,
                        ftai_rule: format!(
                            "rule \"{before}-before-{after}\" {{\n  \
                               on tool:{after}\n  \
                               require tool_was_called(\"{before}\") in session\n  \
                               reason \"Pattern learned: {before} before {after} succeeds {:.0}% of the time\"\n\
                             }}", success_rate * 100.0
                        ),
                    });
                }
            }
        }
        Ok(rules)
    }

    /// Detect repeated failures with the same tool
    fn detect_repeated_failures(&self) -> Result<Vec<GeneratedRule>> {
        let sessions = self.store.recent_sessions(20)?;
        let mut failure_counts: HashMap<String, Vec<String>> = HashMap::new();

        for session in &sessions {
            for tc in &session.tool_calls {
                if let ToolResultType::Error(msg) = &tc.result_type {
                    failure_counts.entry(tc.tool_name.clone())
                        .or_default()
                        .push(msg.clone());
                }
            }
        }

        let mut rules = Vec::new();
        for (tool, errors) in &failure_counts {
            if errors.len() >= 3 {
                // Cluster similar errors
                let common_pattern = find_common_substring(errors);
                if let Some(pattern) = common_pattern {
                    rules.push(GeneratedRule {
                        name: format!("{tool}-failure-guard"),
                        source: RuleSource::Evolution,
                        confidence: 0.7,
                        ftai_rule: format!(
                            "# Auto-generated: {tool} frequently fails with: {pattern}\n\
                             # Consider adding validation before calling {tool}"
                        ),
                    });
                }
            }
        }
        Ok(rules)
    }
}

#[derive(Debug)]
pub struct GeneratedRule {
    pub name: String,
    pub source: RuleSource,
    pub confidence: f64,     // 0.0 - 1.0
    pub ftai_rule: String,   // The actual FTAI DSL rule text
}

#[derive(Debug)]
pub enum RuleSource {
    Evolution,   // Auto-generated from session analysis
    User,        // Written by user
    Plugin,      // From a plugin
}
```

### 6.5 Evolution Storage

```sql
-- ~/.ftai/evolution/evolution.db

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    task_description TEXT,
    outcome TEXT NOT NULL,       -- 'success', 'partial', 'failure', 'abandoned'
    user_feedback TEXT,
    total_tokens INTEGER,
    retries INTEGER
);

CREATE TABLE tool_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    seq INTEGER NOT NULL,        -- Order within session
    tool_name TEXT NOT NULL,
    arguments_summary TEXT,
    result_type TEXT NOT NULL,
    error_message TEXT,
    duration_ms INTEGER
);

CREATE TABLE generated_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    confidence REAL NOT NULL,
    ftai_rule TEXT NOT NULL,
    generated_at INTEGER NOT NULL,
    applied_count INTEGER DEFAULT 0,
    success_after_apply INTEGER DEFAULT 0,  -- Track if rule actually helps
    disabled INTEGER DEFAULT 0
);

CREATE INDEX idx_tc_session ON tool_calls(session_id);
CREATE INDEX idx_tc_tool ON tool_calls(tool_name);
CREATE INDEX idx_sessions_project ON sessions(project);
```

### 6.6 Safeguards

Evolution rules can go wrong. Safeguards:

1. **Confidence threshold:** Rules below 0.7 confidence are stored but not activated. They appear in `/rules` output with a "(suggested)" tag.
2. **Max active evolution rules:** 20. Oldest rules with lowest success_after_apply are pruned.
3. **User override:** User can disable any evolution rule via `forge evolution disable <rule-name>`.
4. **Rollback:** If a rule is applied and the next 3 sessions have lower success rates, it's auto-disabled.
5. **Transparency:** `forge evolution list` shows all generated rules with their stats.

---

## 7. Session & Context Management (AKE Adaptation)

### 7.1 Session Persistence

Every conversation is persisted to SQLite for cross-session continuity.

```sql
-- ~/.ftai/sessions/sessions.db

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    summary TEXT,                    -- LLM-generated summary at session end
    message_count INTEGER DEFAULT 0,
    total_tokens INTEGER DEFAULT 0
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    seq INTEGER NOT NULL,
    role TEXT NOT NULL,              -- 'system', 'user', 'assistant', 'tool'
    content TEXT NOT NULL,
    tool_calls TEXT,                 -- JSON array of tool calls
    tool_call_id TEXT,
    tokens_estimated INTEGER,
    timestamp INTEGER NOT NULL
);

CREATE INDEX idx_messages_session ON messages(session_id, seq);
CREATE INDEX idx_sessions_project ON sessions(project, started_at DESC);
```

### 7.2 Cross-Session Continuity

When starting a new session for a project, Forge loads the summary from the most recent session:

```rust
// src/session/mod.rs

pub struct SessionManager {
    db: Connection,
    current_session: Option<String>,
}

impl SessionManager {
    /// Load context from the previous session for this project
    pub fn load_previous_context(&self, project: &str) -> Option<String> {
        let row = self.db.query_row(
            "SELECT summary, ended_at FROM sessions \
             WHERE project = ?1 ORDER BY ended_at DESC LIMIT 1",
            params![project],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        ).ok()?;

        let (summary, ended_at) = row;
        let age_hours = (now_unix() - ended_at) / 3600;

        // Only inject if session was recent (< 24 hours)
        if age_hours < 24 {
            Some(format!(
                "[Previous session ({age_hours}h ago): {summary}]"
            ))
        } else {
            None
        }
    }

    /// Generate summary at session end (using the model itself)
    pub async fn end_session(&self, engine: &ConversationEngine, backend: &dyn ModelBackend) -> Result<()> {
        let messages = engine.messages();
        if messages.len() < 2 { return Ok(()); }

        // Build a summary request
        let summary_prompt = format!(
            "Summarize the following coding session in 2-3 sentences. \
             Focus on what was accomplished, what files were modified, \
             and any unresolved issues.\n\n{}",
            Self::format_messages_for_summary(messages)
        );

        let request = ChatRequest {
            messages: vec![
                Message { role: Role::System, content: "You are a session summarizer.".into(), tool_calls: None, tool_call_id: None },
                Message { role: Role::User, content: summary_prompt, tool_calls: None, tool_call_id: None },
            ],
            tools: vec![],
            temperature: 0.3,
            max_tokens: Some(256),
        };

        let response = backend.generate(&request).await?;

        self.db.execute(
            "UPDATE sessions SET summary = ?1, ended_at = ?2 WHERE id = ?3",
            params![response.message.content, now_unix(), self.current_session],
        )?;
        Ok(())
    }
}
```

### 7.3 Context Window Token Budgeting

The model has a fixed context window (typically 32K tokens for Qwen 3.5). Every token matters.

```
Total context: 32,768 tokens
  |
  |-- System prompt (identity + FTAI.md + formatting): ~1,500 tokens
  |-- Tool definitions (13 tools):                     ~2,000 tokens
  |-- Active rules summary:                            ~500 tokens
  |-- Memory (MEMORY.md):                              ~500 tokens
  |-- Previous session summary:                        ~200 tokens
  |-- RTAI search results (injected per turn):         ~2,000 tokens (budget)
  |                                                    ---------------
  |-- FIXED OVERHEAD:                                  ~6,700 tokens
  |
  |-- Available for conversation:                      ~26,000 tokens
  |   |-- User messages
  |   |-- Assistant responses
  |   |-- Tool call results (can be large)
  |
  |-- Generation headroom:                             ~4,000 tokens (max output per turn)
```

**Token budget manager:**

```rust
// src/session/budget.rs

pub struct TokenBudget {
    total: usize,               // 32768
    system_reserved: usize,     // Fixed overhead calculated at session start
    generation_reserve: usize,  // 4096 -- reserved for model output
}

impl TokenBudget {
    pub fn available_for_conversation(&self) -> usize {
        self.total - self.system_reserved - self.generation_reserve
    }

    pub fn available_for_rtai_results(&self, conversation_tokens: usize) -> usize {
        let remaining = self.available_for_conversation().saturating_sub(conversation_tokens);
        // Cap RTAI injection at 2000 tokens regardless
        remaining.min(2000)
    }

    /// Decide whether to compact based on usage
    pub fn should_compact(&self, conversation_tokens: usize) -> bool {
        conversation_tokens > self.available_for_conversation() * 80 / 100
    }
}
```

**Smart tool result truncation:**

Large tool results (file reads of 1000+ line files, bash output) are truncated to stay within budget:

```rust
pub fn truncate_tool_result(result: &str, max_tokens: usize) -> String {
    let estimated = result.len() / 4;
    if estimated <= max_tokens {
        return result.to_string();
    }

    let max_chars = max_tokens * 4;
    let half = max_chars / 2;
    let start = &result[..half];
    let end = &result[result.len() - half..];

    format!(
        "{start}\n\n[... {lines} lines truncated ({estimated} tokens -> {max_tokens} budget) ...]\n\n{end}",
        lines = result.lines().count() - start.lines().count() - end.lines().count()
    )
}
```

---

## 8. FTAI Scalability Design

### 8.1 Current State

FTAI files are flat text files in `~/.ftai/` and `<project>/.ftai/`. For small-medium projects (under 50 files, under 10 rules), this is fine. The RTAI knowledge library in Serena has ~95 .ftai files totaling ~400KB -- still trivial.

### 8.2 Scalability Concerns

For large codebases or teams with extensive rule sets:
1. **Rule count:** 100+ rules across multiple scopes means loading and evaluating all of them per tool call
2. **Knowledge files:** Teams might have 500+ .ftai knowledge files
3. **Cross-project rules:** An organization's rules repository could be large

### 8.3 Hierarchical FTAI Design

```
~/.ftai/
  rules.ftai              # Global rules (always loaded)
  evolution/rules/        # Auto-generated rules (loaded with confidence filter)
  knowledge/              # Global knowledge files (loaded on demand)
    LIBRARY_INDEX.ftai    # Index file: maps topics -> files

<project>/.ftai/
  RULES.md                # Project rules (always loaded)
  config.toml             # Project config
  knowledge/              # Project-specific knowledge
    INDEX.ftai            # Project knowledge index

<project>/src/
  module_a/.ftai/
    RULES.md              # Module-level rules (loaded when touching module_a/)
  module_b/.ftai/
    RULES.md              # Module-level rules (loaded when touching module_b/)
```

**Rule loading priority (highest wins):**
1. Module-level: `<project>/src/module/.ftai/RULES.md`
2. Project-level: `<project>/.ftai/RULES.md`
3. User project-level: `~/.ftai/projects/<project>/rules.ftai`
4. Evolution rules: `~/.ftai/evolution/rules/` (confidence >= 0.7)
5. Global: `~/.ftai/rules.ftai`

### 8.4 Lazy Loading

Only load rules and knowledge files relevant to the current operation:

```rust
// src/rules/loader.rs

pub struct LazyRuleLoader {
    global_rules: RuleSet,          // Always loaded
    project_rules: RuleSet,         // Loaded at session start
    module_cache: HashMap<PathBuf, RuleSet>,  // Loaded on demand
}

impl LazyRuleLoader {
    /// Get rules relevant to a specific tool call
    pub fn rules_for_context(&mut self, tool_name: &str, file_path: Option<&Path>) -> Vec<&Rule> {
        let mut applicable = Vec::new();

        // Global rules always apply
        applicable.extend(self.global_rules.rules.iter()
            .filter(|r| r.event_matches(tool_name)));

        // Project rules always apply
        applicable.extend(self.project_rules.rules.iter()
            .filter(|r| r.event_matches(tool_name)));

        // Module rules: only if we're touching a file in that module
        if let Some(path) = file_path {
            if let Some(module_rules) = self.load_module_rules(path) {
                applicable.extend(module_rules.rules.iter()
                    .filter(|r| r.event_matches(tool_name)));
            }
        }

        applicable
    }

    fn load_module_rules(&mut self, file_path: &Path) -> Option<&RuleSet> {
        // Walk up from file_path looking for .ftai/RULES.md
        let mut dir = file_path.parent()?;
        loop {
            let rules_path = dir.join(".ftai").join("RULES.md");
            if rules_path.exists() {
                if !self.module_cache.contains_key(dir) {
                    if let Ok(rules) = parse_rules_file(&rules_path) {
                        self.module_cache.insert(dir.to_path_buf(), rules);
                    }
                }
                return self.module_cache.get(dir);
            }
            dir = dir.parent()?;
            // Stop at project root
            if dir == self.project_root { break; }
        }
        None
    }
}
```

### 8.5 SQLite Backend for Large-Scale FTAI

For organizations with 1000+ rules, the file-based approach won't scale. Provide an optional SQLite backend:

```toml
# ~/.ftai/config.toml
[ftai]
storage = "sqlite"  # Default: "file". Set "sqlite" for large rule sets.
```

```sql
-- ~/.ftai/ftai.db (only created when storage = "sqlite")

CREATE TABLE rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    scope TEXT,                  -- NULL = global, path = scoped
    event TEXT NOT NULL,
    condition TEXT,              -- Serialized expression
    action_type TEXT NOT NULL,   -- 'reject', 'require', 'modify'
    action_expr TEXT NOT NULL,   -- Serialized expression
    unless_expr TEXT,
    reason TEXT,
    source TEXT NOT NULL,        -- 'user', 'project', 'evolution', 'plugin'
    enabled INTEGER DEFAULT 1,
    priority INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE knowledge (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    topic TEXT NOT NULL,
    content TEXT NOT NULL,
    scope TEXT,                  -- NULL = global, project path = scoped
    source TEXT NOT NULL,
    embedding BLOB,             -- Optional: for semantic retrieval of knowledge
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_rules_event ON rules(event) WHERE enabled = 1;
CREATE INDEX idx_rules_scope ON rules(scope) WHERE enabled = 1;
CREATE INDEX idx_knowledge_topic ON knowledge(topic);
```

The SQLite backend is a drop-in replacement for the file loader. Same `RuleSet` struct, different source.

---

## 9. FolkTech IDE Integration Assessment

### 9.1 Option A: Standalone CLI (`forge`)

**Pros:**
- Simple distribution: single binary, no GUI dependencies
- Works in any terminal (SSH, tmux, bare metal)
- Faster iteration: no Tauri/React build overhead
- Matches the terminal-first UX model used by modern AI coding assistants
- Lower memory footprint (no Electron/WebView)

**Cons:**
- No rich UI (code preview, file tree, visual diffs)
- Duplicate effort: IDE already has tool execution, agent system, MCP
- Two separate products to maintain

### 9.2 Option B: Integrate into FolkTech IDE

**Pros:**
- Rich UI for code viewing, diffs, previews
- Reuse IDE's LSP, semantic search, MCP infrastructure
- Single product

**Cons:**
- IDE is Electron-weight (Tauri is lighter but still a WebView)
- IDE has its own agent system (TypeScript) -- duplicates Forge's Rust agent
- IDE's llama-cpp-2 integration is already working but less flexible than direct FFI
- IDE is 92% functional but has known crash issues (DAS, model loading)
- TypeScript frontend is hard to ship as a single binary

### 9.3 Option C: Both (Recommended)

**Architecture:**

```
forge (Rust binary)
  |-- CLI mode (default): TUI with ratatui
  |-- Server mode (--serve): Unix socket / TCP server
  |
  v
FolkTech IDE (Tauri app)
  |-- Uses forge as backend via Tauri sidecar
  |-- Replaces current LLM/Agent TypeScript code with forge calls
  |-- Keeps: Monaco editor, UI, LSP frontend, MCP frontend
```

**How it works:**

1. `forge` ships as a standalone CLI. This is the primary product.
2. `forge --serve` starts a JSON-RPC server on a Unix socket.
3. FolkTech IDE bundles `forge` as a Tauri sidecar binary.
4. IDE's TypeScript agent code becomes a thin client that calls `forge` via JSON-RPC.
5. IDE keeps its Monaco editor, UI components, LSP frontend bindings.
6. IDE's Rust backend (`src-tauri/`) becomes thinner: removes `llm/`, `search/`, replaces with forge sidecar calls.

**Shared code:**
- `forge` crate becomes a library (`forge-core`) + binary (`forge-cli`)
- IDE's Rust backend depends on `forge-core` as a library crate
- All inference, search, tools, rules, evolution logic lives in `forge-core`
- IDE just provides the GUI and routes user actions to `forge-core`

**Development complexity:**
- Phase 1: Build `forge` CLI standalone (this document)
- Phase 2: Extract `forge-core` library crate
- Phase 3: Integrate `forge-core` into FolkTech IDE's Tauri backend

This gives us the terminal product fast, then the IDE integration later without rewriting anything.

### 9.4 JSON-RPC Server Protocol (for IDE integration)

```rust
// forge --serve protocol (future, Phase 2)

// Request: start a session
{"jsonrpc": "2.0", "id": 1, "method": "session/start", "params": {"project": "/path/to/project"}}

// Request: send a message
{"jsonrpc": "2.0", "id": 2, "method": "message/send", "params": {"text": "fix the auth bug"}}

// Notification: streaming token
{"jsonrpc": "2.0", "method": "stream/token", "params": {"text": "Let me", "session_id": "abc"}}

// Notification: tool call pending approval
{"jsonrpc": "2.0", "method": "tool/approve", "params": {"id": "tc_1", "tool": "file_edit", "args": {...}}}

// Request: approve/reject tool call
{"jsonrpc": "2.0", "id": 3, "method": "tool/respond", "params": {"id": "tc_1", "approved": true}}
```

---

## 10. File Structure

### 10.1 Cargo Workspace

For Phase 1, keep it as a single crate (monolith). Extract `forge-core` in Phase 2.

```
forge/                              # Renamed from ftai/
├── Cargo.toml
├── build.rs                        # Builds llama.cpp from source
├── CLAUDE.md
├── docs/
│   └── plans/
│       ├── 2026-03-02-ftai-terminal-harness-design.md
│       ├── 2026-03-02-ftai-implementation-plan.md
│       ├── 2026-03-27-forge-architecture.md          # This document
│       ├── 2026-03-31-architecture-amendments.md     # Architecture amendments
│       └── 2026-03-31-agent-loop-anatomy.md          # Agent loop deep-dive
├── scripts/
│   └── mlx_server.py              # MLX inference subprocess script
├── vendor/
│   └── llama.cpp/                 # Git submodule
├── src/
│   ├── main.rs                    # CLI entry point (clap)
│   ├── lib.rs                     # Library root (for future forge-core extraction)
│   │
│   ├── config/                    # REUSED from FTAI
│   │   ├── mod.rs
│   │   └── loader.rs
│   │
│   ├── inference/                 # NEW -- replaces backend/
│   │   ├── mod.rs
│   │   ├── context.rs             # LlamaContext safe FFI wrapper
│   │   ├── sampler.rs             # Sampling strategies
│   │   ├── grammar.rs             # GBNF grammar for tool calls
│   │   ├── model.rs               # Model loading, config, hardware detection
│   │   ├── mlx.rs                 # MLX subprocess backend
│   │   └── knowledge_sampler.rs   # NEW -- Mitosis-RTAI Knowledge Grounding Layer
│   │
│   ├── backend/                   # KEPT for trait definitions
│   │   ├── mod.rs
│   │   ├── types.rs               # ModelBackend trait, ChatRequest, etc.
│   │   ├── manager.rs             # Backend selection logic
│   │   ├── http_client.rs         # DEPRECATED (kept for fallback to llama-server)
│   │   └── llamacpp.rs            # DEPRECATED (subprocess approach, kept as fallback)
│   │
│   ├── conversation/              # REUSED + expanded
│   │   ├── mod.rs
│   │   ├── engine.rs              # ConversationEngine
│   │   ├── parser.rs              # ToolCallParser (existing)
│   │   ├── prompt.rs              # System prompt builder
│   │   ├── adapter.rs             # NEW -- ModelAdapter trait (Qwen 3.5 XML vs Qwen 3 Hermes)
│   │   ├── streaming.rs           # NEW -- StreamingToolCallParser
│   │   ├── validator.rs           # NEW -- ToolCallValidator
│   │   ├── recovery.rs            # NEW -- Three-attempt error recovery pipeline
│   │   └── grammar.rs             # NEW -- GBNF grammar for constrained retry only
│   │
│   ├── rules/                     # REUSED from FTAI
│   │   ├── mod.rs
│   │   ├── lexer.rs
│   │   ├── parser.rs
│   │   ├── evaluator.rs
│   │   ├── builtins.rs
│   │   └── loader.rs              # NEW -- lazy loading, module-level rules
│   │
│   ├── tools/                     # REUSED + extended
│   │   ├── mod.rs
│   │   ├── registry.rs            # Tool trait, ToolRegistry
│   │   ├── bash.rs
│   │   ├── file_read.rs
│   │   ├── file_write.rs
│   │   ├── file_edit.rs
│   │   ├── glob_tool.rs
│   │   ├── grep_tool.rs
│   │   ├── git.rs
│   │   ├── web_fetch.rs
│   │   ├── ask_user.rs
│   │   ├── request_permissions.rs
│   │   ├── list_dir.rs            # NEW
│   │   ├── search_semantic.rs     # NEW -- RTAI semantic search
│   │   └── agent_spawn.rs         # NEW -- sub-agent spawning
│   │
│   ├── search/                    # NEW -- RTAI adaptation
│   │   ├── mod.rs
│   │   ├── indexer.rs             # File walker, chunker, embedding pipeline
│   │   ├── store.rs               # SQLite storage
│   │   ├── query.rs               # Search execution
│   │   └── watcher.rs             # File change detection (notify crate)
│   │
│   ├── evolution/                 # NEW -- Mitosis adaptation
│   │   ├── mod.rs
│   │   ├── analyzer.rs            # Session outcome analysis
│   │   ├── generator.rs           # FTAI rule generation
│   │   └── store.rs               # SQLite evolution history
│   │
│   ├── session/                   # NEW -- AKE adaptation
│   │   ├── mod.rs                 # SessionManager
│   │   ├── budget.rs              # Token budget management
│   │   ├── compact.rs             # NEW (CC-amendment) -- Three-tier context compaction
│   │   └── transcript.rs          # NEW (CC-amendment) -- JSONL append-only transcript
│   │
│   ├── skills/                    # NEW (CC-amendment) -- On-demand domain knowledge
│   │   ├── mod.rs                 # SkillRegistry, trigger matching
│   │   └── loader.rs              # YAML frontmatter parser, lazy loading
│   │
│   ├── hooks/                     # NEW (CC-amendment) -- Event-driven shell automation
│   │   └── mod.rs                 # HookRunner, event dispatch
│   │
│   ├── permissions/               # REUSED from FTAI
│   │   ├── mod.rs
│   │   ├── classifier.rs
│   │   ├── grants.rs
│   │   └── patterns.rs
│   │
│   ├── plugins/                   # REUSED from FTAI
│   │   ├── mod.rs
│   │   ├── hooks.rs
│   │   ├── manager.rs
│   │   ├── manifest.rs
│   │   ├── registry.rs
│   │   ├── skill_loader.rs
│   │   └── tool_bridge.rs
│   │
│   ├── formatting/                # REUSED from FTAI
│   │   ├── mod.rs
│   │   └── loader.rs
│   │
│   └── tui/                       # REUSED from FTAI
│       ├── mod.rs
│       ├── app.rs
│       ├── input.rs
│       └── render.rs
│
├── tests/
│   ├── integration.rs             # Full pipeline tests
│   ├── inference/                  # NEW
│   │   ├── grammar_test.rs
│   │   └── tool_calling_test.rs
│   ├── search/                     # NEW
│   │   ├── indexer_test.rs
│   │   └── query_test.rs
│   ├── evolution/                  # NEW
│   │   └── analyzer_test.rs
│   └── session/                    # NEW
│       └── budget_test.rs
│
└── resources/
    └── grammars/
        └── tool_call.gbnf         # GBNF grammar for constrained tool generation
```

### 10.2 Updated Cargo.toml

```toml
[package]
name = "forge"
version = "0.2.0"
edition = "2021"
description = "Local-first AI development assistant"
license = "MIT"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# CLI
clap = { version = "4", features = ["derive"] }

# TUI
ratatui = "0.29"
crossterm = "0.28"

# HTTP (kept for web_fetch tool + model download)
reqwest = { version = "0.12", features = ["json", "stream"] }

# Code tools
regex = "1"
glob = "0.3"
git2 = "0.19"

# Rendering
syntect = "5"
pulldown-cmark = "0.12"

# Error handling
anyhow = "1"
thiserror = "2"

# Async streams
futures-util = "0.3"

# Misc
dirs = "6"
sha2 = "0.10"
uuid = { version = "1", features = ["v4"] }

# NEW: Inference
llama-cpp-sys-2 = "0.1"           # Raw FFI bindings to llama.cpp

# NEW: Embeddings
fastembed = "4"                    # ONNX-based text embeddings

# NEW: Code parsing
tree-sitter = "0.24"
tree-sitter-rust = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-python = "0.23"
tree-sitter-go = "0.23"

# NEW: SQLite (sessions, search, evolution)
rusqlite = { version = "0.32", features = ["bundled"] }

# NEW: File watching
notify = "7"

# NEW: File walking (respects .gitignore)
ignore = "0.4"

# NEW: Chrono for timestamps
chrono = "0.4"

[dev-dependencies]
tempfile = "3"

[build-dependencies]
cc = "1"                           # For building llama.cpp from source
```

---

## 11. Build & Distribution

### 11.1 build.rs: Compiling llama.cpp

```rust
// build.rs

fn main() {
    // Build llama.cpp from vendor/ submodule
    let mut build = cc::Build::new();

    build
        .cpp(true)
        .file("vendor/llama.cpp/src/llama.cpp")
        .file("vendor/llama.cpp/src/llama-vocab.cpp")
        .file("vendor/llama.cpp/src/llama-grammar.cpp")
        .file("vendor/llama.cpp/src/llama-sampling.cpp")
        .file("vendor/llama.cpp/ggml/src/ggml.c")
        .file("vendor/llama.cpp/ggml/src/ggml-alloc.c")
        .file("vendor/llama.cpp/ggml/src/ggml-backend.c")
        .include("vendor/llama.cpp/include")
        .include("vendor/llama.cpp/ggml/include")
        .flag("-std=c++17")
        .flag("-O3")
        .define("NDEBUG", None);

    // Platform-specific acceleration
    if cfg!(target_os = "macos") {
        // Metal GPU acceleration
        build
            .file("vendor/llama.cpp/ggml/src/ggml-metal.m")
            .define("GGML_USE_METAL", None)
            .flag("-framework").flag("Foundation")
            .flag("-framework").flag("Metal")
            .flag("-framework").flag("MetalKit");

        // Accelerate framework for BLAS
        build
            .define("GGML_USE_ACCELERATE", None)
            .flag("-framework").flag("Accelerate");

        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=MetalKit");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=Accelerate");
    }

    if cfg!(target_os = "linux") || cfg!(target_os = "windows") {
        // Check for CUDA
        if std::env::var("CUDA_PATH").is_ok() || std::path::Path::new("/usr/local/cuda").exists() {
            // CUDA build -- requires nvcc
            build.define("GGML_USE_CUDA", None);
            println!("cargo:rustc-link-lib=cuda");
            println!("cargo:rustc-link-lib=cublas");
        }
    }

    build.compile("llama");

    // Link C++ runtime
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }
}
```

### 11.2 Cross-Platform Build

**macOS (Apple Silicon):**
```bash
# Universal binary (arm64 only -- we don't support Intel Macs)
cargo build --release --target aarch64-apple-darwin

# Output: target/aarch64-apple-darwin/release/forge
# Size: ~15-20MB (includes llama.cpp statically linked)
```

**Windows (x86_64):**
```bash
# Cross-compile from Mac (requires cross or Windows CI)
# Or build natively on Windows:
cargo build --release --target x86_64-pc-windows-msvc

# With CUDA (if CUDA toolkit installed):
CUDA_PATH="C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.x" cargo build --release
```

**Linux (x86_64):**
```bash
cargo build --release --target x86_64-unknown-linux-gnu
```

### 11.3 Model Download Flow

```
forge model install
  |
  v
1. Detect hardware: HardwareInfo::detect()
   -> "Apple Silicon, 16GB, Metal"
  |
  v
2. Recommend model: "Qwen3.5-35B-A3B-4bit (MLX)"
   "This model uses 4GB RAM. Recommended for your hardware."
   "Download? [Y/n]"
  |
  v
3. Download from Hugging Face:
   https://huggingface.co/Qwen/Qwen3.5-35B-A3B-4bit/resolve/main/
   -> ~/.ftai/models/Qwen3.5-35B-A3B-4bit/
   Progress: [=========>     ] 2.1GB / 4.0GB  52%  12 MB/s  ETA 2m30s
  |
  v
4. Also download embedding model (if not present):
   BGE-small-en-v1.5 ONNX -> ~/.ftai/models/bge-small-en-v1.5/
   (130MB, fast download)
  |
  v
5. Verify checksums
  |
  v
6. Update config:
   ~/.ftai/config.toml
   [model]
   path = "~/.ftai/models/Qwen3.5-35B-A3B-4bit"
   backend = "mlx"
```

### 11.4 First-Run Experience

```
$ forge

  Welcome to Forge v0.2.0

  First-time setup detected. Let's get you configured.

  Hardware detected:
    CPU:  Apple M2 Pro
    RAM:  16 GB unified memory
    GPU:  Metal (Apple Silicon)

  Recommended model: Qwen3.5-35B-A3B-4bit (MoE, 3B active params)
    Size: 4.0 GB
    Backend: MLX
    Context: 32K tokens

  Download now? [Y/n] y

  Downloading Qwen3.5-35B-A3B-4bit...
  [================================] 4.0 GB  Done.

  Downloading BGE-small-en-v1.5 (embedding model)...
  [================================] 130 MB  Done.

  Configuration saved to ~/.ftai/config.toml
  Models saved to ~/.ftai/models/

  Ready. Type 'forge' in any project directory to start.
  Type 'forge --help' for all options.

$ cd ~/Developer/my-project
$ forge

  forge v0.2.0 | Qwen3.5-35B-A3B | MLX | 32K ctx
  Project: ~/Developer/my-project
  Indexing project... 1,247 files, 8,432 chunks indexed. (3.2s)

  > _
```

---

## 12. Knowledge Grounding Layer (Mitosis-RTAI Inference Integration)

### 12.1 Overview

This section defines a novel inference-time layer that makes RTAI verified facts non-negotiable in the model's output. Instead of injecting facts into the context window (RAG) where the model can hallucinate on top of them, this layer operates at the logit level — modifying the token probability distribution during generation to force verified facts into the output.

**This is not RAG.** RAG says "here's some context, good luck." This says "when you state a fact that RTAI knows, you WILL use RTAI's value."

**Core principle:** The model controls language, structure, and reasoning. RTAI controls facts. They never cross-contaminate. RTAI's determinism (zero hallucination) is preserved because facts are injected at the sampling level, not interpreted by the model.

### 12.2 Architecture: Parallel Processing During Inference

```
Standard LLM inference:
  Forward pass → logits → sampling → next token

Forge inference with Knowledge Grounding:
  Forward pass → logits → KnowledgeSampler → modified logits → sampling → next token
                              │
                              ├── Entity Trie Walker (detect entities in token stream)
                              ├── Mitosis Tree Navigator (traverse knowledge hierarchy)
                              ├── RTAI Fact Retriever (get verified facts at leaf nodes)
                              └── Logit Modifier (boost fact tokens, suppress contradictions)
```

The KnowledgeSampler runs at every generated token. It costs ~55-105 microseconds per token against a budget of ~28.5ms per token (at 35 tok/s). This is 0.2-0.4% overhead — imperceptible.

### 12.3 The Mitosis Knowledge Tree

Entities and facts form a navigable tree that Mitosis traverses during generation. Each token narrows the active path. Leaf nodes are RTAI-verified facts.

```
Root
├── Michelle (person)
│   ├── medications
│   │   ├── metoprolol
│   │   │   ├── dose: "50mg"             ← RTAI fact (verified, deterministic)
│   │   │   ├── frequency: "daily AM"    ← RTAI fact
│   │   │   └── prescriber: "Dr. Patel"  ← RTAI fact
│   │   └── lisinopril
│   │       ├── dose: "10mg"
│   │       └── frequency: "daily PM"
│   ├── allergies
│   │   └── sulfa: "confirmed 2024"
│   └── appointments
│       └── PT: "Thursday 5pm"
│
├── project:forge (code project)
│   ├── language: "Rust"
│   ├── framework: "llama.cpp FFI"
│   └── modules
│       ├── inference: "src/inference/"
│       └── search: "src/search/"
```

### 12.4 Token-Level Entity Detection

The model generates BPE sub-word tokens, not whole words. "Metoprolol" might tokenize as ["Met", "ro", "pol", "ol"]. We detect entities by walking a pre-tokenized trie:

```rust
// Built at init time: tokenize every entity name with the model's tokenizer.
// Store as a trie of token-ID sequences.
//
// "Metoprolol" → token IDs [15439, 307, 1159, 337]
// "Michelle"   → token IDs [35879, 4912]
//
// Trie structure:
// 15439 → 307 → 1159 → 337 → ENTITY("metoprolol")
// 35879 → 4912 → ENTITY("michelle")

pub struct EntityTrie {
    children: HashMap<i32, EntityTrie>,   // token_id → subtree
    entity: Option<EntityMatch>,           // populated at leaf = entity fully matched
}

pub struct EntityMatch {
    pub entity_id: String,
    pub tree_path: Vec<String>,  // ["michelle", "medications", "metoprolol"]
}

impl EntityTrie {
    /// Feed a generated token. Returns entity match if trie path completes.
    pub fn step(&mut self, token_id: i32) -> Option<&EntityMatch> {
        // O(1) per token — single HashMap lookup
    }
}
```

### 12.5 The Cascade: Context Narrows With Each Token

```
Token stream:  "Michelle"  →  "takes"  →  "metoprolol"  →  [NEXT]

Mitosis:       activate       context      drill           INJECT FACT
               cluster:       narrows:     deeper:         "50mg" logits
               [Michelle]     [Michelle+   [Michelle→      boosted to
                               medication]  medications→    near-certainty
                                           metoprolol]

RTAI depth:    Level 0        Level 1      Level 2         Level 3: FACT
```

Each entity token narrows the Mitosis path. When we reach a depth where RTAI facts exist, the logits for the next tokens are modified to favor the verified value.

### 12.6 KnowledgeSampler Implementation

Implemented as a custom `llama_sampler_i` (llama.cpp's sampler interface). Plugs into the existing sampler chain:

```
logits → [KnowledgeSampler] → top_k → top_p → temperature → dist → sample
```

```rust
// src/inference/knowledge_sampler.rs

/// State machine for knowledge-grounded sampling
#[derive(Debug)]
enum SamplerState {
    /// No active entity detection. Model generates freely.
    Idle,
    /// Entity partially matched in trie. Watching for completion.
    Matching { trie_position: TrieNodeId },
    /// Entity fully matched. Watching for fact-statement context.
    EntityMatched { entity: EntityMatch, tokens_since: usize },
    /// Actively injecting a verified fact. Boosting fact tokens.
    InjectingFact {
        fact_tokens: Vec<i32>,    // Remaining tokens to boost
        position: usize,          // Current position in fact sequence
        boost: f32,               // Logit boost amount
    },
}

pub struct KnowledgeSampler {
    state: SamplerState,
    entity_trie: EntityTrie,
    knowledge_tree: MitosisTree,
    token_buffer: Vec<i32>,           // Recent tokens for context detection
    suppression_tokens: HashSet<i32>, // Tokens for "unlike", "compared to", etc.

    // Config
    boost_amount: f32,      // Default: +12.0 (strong bias, not hard override)
    max_context_window: usize,  // How many tokens after entity to watch for fact context
}

impl KnowledgeSampler {
    /// Called by llama.cpp after each token is accepted into the sequence.
    /// Updates entity detection state.
    fn accept(&mut self, token: i32) {
        self.token_buffer.push(token);
        if self.token_buffer.len() > 50 { self.token_buffer.remove(0); }

        match &self.state {
            SamplerState::Idle => {
                // Check if this token starts or continues an entity match
                if let Some(next) = self.entity_trie.step(token) {
                    if let Some(entity) = &next.entity {
                        self.state = SamplerState::EntityMatched {
                            entity: entity.clone(),
                            tokens_since: 0
                        };
                    } else {
                        self.state = SamplerState::Matching {
                            trie_position: next.id
                        };
                    }
                }
            }
            SamplerState::Matching { trie_position } => {
                // Continue trie walk
                if let Some(next) = self.entity_trie.step_from(*trie_position, token) {
                    if let Some(entity) = &next.entity {
                        self.state = SamplerState::EntityMatched {
                            entity: entity.clone(),
                            tokens_since: 0
                        };
                    }
                } else {
                    // Trie walk failed — not an entity
                    self.state = SamplerState::Idle;
                }
            }
            SamplerState::EntityMatched { tokens_since, .. } => {
                // Check for suppression signals ("unlike", "compared to")
                if self.suppression_tokens.contains(&token) {
                    self.state = SamplerState::Idle;
                    return;
                }
                // Timeout: if too many tokens pass without fact context, release
                if *tokens_since > self.max_context_window {
                    self.state = SamplerState::Idle;
                }
            }
            SamplerState::InjectingFact { position, fact_tokens, .. } => {
                // Advance fact injection
                if *position >= fact_tokens.len() {
                    self.state = SamplerState::Idle;
                }
            }
        }
    }

    /// Called by llama.cpp before sampling. Modifies logit distribution.
    fn apply(&mut self, candidates: &mut LlamaTokenDataArray) {
        match &mut self.state {
            SamplerState::EntityMatched { entity, tokens_since } => {
                *tokens_since += 1;

                // Check if recent tokens suggest a fact statement follows
                if self.is_fact_context(&self.token_buffer) {
                    // Look up the RTAI fact for this entity
                    if let Some(fact) = self.knowledge_tree.get_fact(&entity.tree_path) {
                        let fact_tokens = self.tokenize_fact(&fact.value);
                        self.state = SamplerState::InjectingFact {
                            fact_tokens,
                            position: 0,
                            boost: self.boost_amount,
                        };
                        // Apply boost for first token immediately
                        self.boost_token(candidates);
                    }
                }
            }
            SamplerState::InjectingFact { .. } => {
                self.boost_token(candidates);
            }
            _ => {} // Idle or Matching — no logit modification
        }
    }

    /// Boost the target fact token's logit in the candidate array
    fn boost_token(&mut self, candidates: &mut LlamaTokenDataArray) {
        if let SamplerState::InjectingFact { fact_tokens, position, boost } = &mut self.state {
            if *position < fact_tokens.len() {
                let target_token = fact_tokens[*position];
                for candidate in candidates.iter_mut() {
                    if candidate.id == target_token {
                        candidate.logit += *boost;  // Boost, don't override
                    }
                }
                *position += 1;
            }
        }
    }

    /// Detect if recent tokens suggest the model is about to state a fact
    fn is_fact_context(&self, buffer: &[i32]) -> bool {
        // Heuristic: look for patterns like "is", "dose", "takes", "=", ":"
        // in the last few tokens after the entity
        //
        // More sophisticated: check logit entropy (low entropy = confident claim)
        //
        // Start simple, iterate to 95%+ accuracy
        let recent_text = self.detokenize_buffer(&buffer[buffer.len().saturating_sub(5)..]);
        let fact_patterns = [" is ", " are ", " was ", " dose ", " takes ", ": ", " = "];
        fact_patterns.iter().any(|p| recent_text.contains(p))
    }
}
```

### 12.7 Logit Boosting vs Hard Override

**Design decision: boost, not override.**

| Approach | Effect | Risk |
|----------|--------|------|
| Hard override (logit = +∞, all others = -∞) | Fact token guaranteed | Can break grammar mid-sentence |
| Soft boost (logit += 12.0) | Fact token ~99.9% likely | Model can still route around if grammatically necessary |

A logit boost of +12.0 means the fact token's probability increases by ~e^12 ≈ 162,000x relative to other tokens. In practice this makes it near-certain while preserving the model's ability to maintain sentence structure.

For safety-critical domains (medical dosages), the boost can be increased to +20.0 or higher, approaching hard override behavior.

### 12.8 Integration with FTAI Components

**Component responsibilities remain separate:**

```
Mitosis (LEARNS)
  │  Discovers patterns, builds/refines the knowledge tree structure
  │  Proposes new entities and relationships
  │
  ▼
AKE (VERIFIES)
  │  Validates facts before they enter RTAI
  │  Confidence scoring, correction flow
  │  Nothing enters RTAI without AKE approval
  │
  ▼
RTAI (STORES)
  │  Deterministic fact store (FTAI Data files)
  │  Clustered by domain, routed by Mitosis tree
  │  Zero interpretation, zero generation
  │
  ▼
KnowledgeSampler (ENFORCES)
     Reads Mitosis tree + RTAI facts during inference
     Modifies logits to ground model output in verified facts
     Does NOT learn, verify, or store — only enforces
```

Four components, four jobs. The trust chain is strictly ordered.

### 12.9 FTAI Data Cluster Routing

RTAI facts are organized in clustered FTAI Data files with a SQLite routing layer:

```
Query contains "metoprolol"
  │
  ▼
Keyword routing (<1ms):
  "metoprolol" → cluster: "healthcare/medications"
  │
  ▼
Load cluster FTAI files (cached after first access):
  medications.ftai, dosage_charts.ftai
  │
  ▼
Linear scan within cluster (<1ms):
  "metoprolol_dose" → "50mg daily AM"
```

**Cluster inheritance** for shared knowledge:
```
clusters/
  _base/
    common_units.ftai            # Shared: mg, mL, kg
  healthcare/
    inherits: _base
    medications.ftai
    protocols.ftai
  ems_field/
    inherits: healthcare         # Gets healthcare + _base
    field_protocols.ftai
```

**Routing modes:**
- Fast path (90%): keyword match to cluster via HashMap — <1ms
- Slow path (10%): embed query, cosine similarity over cluster descriptions — ~15ms
- Both imperceptible to users

### 12.10 For Forge (Code Context)

In Forge, the knowledge tree stores code project facts:

```
Root
├── project:forge
│   ├── language: "Rust"
│   ├── build: "cargo build --release"
│   ├── modules
│   │   ├── inference: "src/inference/ — llama.cpp FFI"
│   │   ├── search: "src/search/ — RTAI code search"
│   │   └── tools: "src/tools/ — 13 tools"
│   └── patterns
│       ├── error_handling: "anyhow for application, thiserror for libraries"
│       └── naming: "snake_case functions, PascalCase types"
```

When the model generates code and mentions a module name, the KnowledgeSampler can ground path references, API patterns, and naming conventions in verified project facts.

### 12.11 Failure Modes and Mitigations

| Failure | Cause | Mitigation |
|---------|-------|------------|
| False entity match | "Metropolitan" triggers "metoprolol" | Require full trie completion, not prefix |
| Wrong injection timing | "Unlike metoprolol..." triggers fact injection | Suppression signals: "unlike", "compared to", "rather than" |
| Grammatical incoherence | Forced token breaks sentence flow | Logit boost (not override) preserves model's grammar |
| Multi-token fact split | "50mg" = ["50", "mg"] needs 2-step injection | State machine tracks remaining fact tokens across steps |
| Model fights override | Model's belief contradicts RTAI fact | Higher boost wins. Log conflict for review. |
| Unknown entity | Entity not in trie | System passes through silently. No degradation. |

### 12.12 Level 1 vs Level 2

**Level 1 (Forge v1 — buildable now):**
- Standard Qwen 3.5 model, unmodified
- KnowledgeSampler bolted onto llama.cpp sampler chain
- Entity detection via pre-tokenized BPE trie
- ~1,500 lines of Rust

**Level 2 (SerenaLM / Jarvis — research):**
- Custom model with semantic tokenizer (entities as first-class tokens)
- Mitosis clustering built into attention mechanism
- RTAI as non-parametric memory (kNN-LM style, native to the model)
- No bolt-on needed — the model natively navigates the knowledge tree

Level 1 validates every concept. Level 2 bakes it into the architecture. Everything built for Level 1 informs Level 2.

### 12.13 Implementation Estimate

| Component | Lines (est.) | Complexity |
|-----------|-------------|------------|
| EntityTrie (pre-tokenized entity trie) | ~200 | Low |
| MitosisTree (hierarchical knowledge navigation) | ~300 | Medium |
| KnowledgeSampler (llama_sampler_i impl) | ~500 | Medium |
| Fact injection state machine | ~300 | Medium |
| Context detection heuristics | ~200 | Medium (iterative) |
| **Total** | **~1,500** | |

---

## 13. TUI/UX Specification

### 13.1 Design Philosophy: Best of Terminal AI Assistants

**Do first, explain when it matters.**

- Safe operations (file reads, grep, git status) execute silently — show result, not narration
- Edits show a compact diff, not "Let me edit that file for you"
- Only stop for confirmation on moderate/dangerous operations
- When there's a real decision (multiple approaches, ambiguous intent), discuss
- Errors get full explanation. Successes get one line.

The user should feel like they have a fast, competent pair programmer that works quietly and speaks up when something matters.

### 13.2 Terminal Layout

```
┌─ forge v0.2.0 │ Qwen3.5-35B-A3B │ MLX │ 32K ctx ──────────────────────┐
│                                                                          │
│ ● src/auth.rs modified (uncommitted)                                    │
│                                                                          │
│ > fix the token expiration bug                                          │
│                                                                          │
│   Reading src/auth.rs...                                                │
│   Found: token.expired() should be token.is_expired()                   │
│                                                                          │
│   ── src/auth.rs ──────────────────────────────────────                 │
│   @@ -45,1 +45,1 @@                                                    │
│   - if token.expired() {                                                │
│   + if token.is_expired() {                                             │
│   ──────────────────────────── Apply? [y/N/a] ─                         │
│                                                                          │
├──────────────────────────────────────────────────────────────────────────┤
│ forge │ main │ 1 staged │ 245 tok │ 12.3 tok/s │ session: 3m          │
└──────────────────────────────────────────────────────────────────────────┘
```

**Three zones:**
1. **Chat area** (scrollable) — conversation + inline diffs + inline approvals
2. **Input line** (bottom of chat area) — user types here, `>` prompt
3. **Status bar** (fixed bottom) — model, git, tokens, speed, session time

### 13.3 Message Display Patterns

**User message:**
```
> fix the token expiration bug
```

**Agent working (collapsed — Aider style):**
```
  Reading src/auth.rs... grep for "expired"... found 3 matches
```

One line. No sub-agent tree. No task IDs. Just what happened.

**Agent working (expanded — on error or verbose mode):**
```
  ├─ grep_search "expired" src/ ............. 3 matches [12ms]
  ├─ read_file src/auth.rs .................. 2450 chars [8ms]
  ├─ read_file src/auth/token.rs ............ 890 chars [5ms]
  └─ edit_file src/auth.rs .................. PENDING APPROVAL
```

Toggle between collapsed/expanded with `Ctrl+V` (verbose mode).

**Agent final response:**
```
  Fixed: `token.expired()` → `token.is_expired()` in src/auth.rs:45.
  The method was renamed in the Token trait but this call site wasn't updated.
```

Short. Factual. No "I've made the change for you" padding.

### 13.4 Inline Diff Display

When the agent wants to edit a file, show the diff inline:

```
  ── src/auth.rs ──────────────────────────────────────
  @@ -45,1 +45,1 @@
  - if token.expired() {
  + if token.is_expired() {
  ──────────────────────────── Apply? [y/N/a/d] ──────
```

**Keys:**
- `y` — apply edit
- `N` — reject (default)
- `a` — always approve edits to this file for this session
- `d` — show full diff context (expand to ±10 lines)

If the diff is large (>20 lines), show a summary first:
```
  ── src/auth.rs (+23 -15) ────────────────────────────
  38 lines changed across 3 hunks. [d] to view full diff
  ──────────────────────────── Apply? [y/N/a/d] ──────
```

### 13.5 Approval Flow

Matches FolkTech IDE's three-tier system:

**Safe tools (auto-approve, silent):**
```
  Reading src/auth.rs... grep for "expired"... 3 matches
```
User sees the result. No prompt. No delay.

**Moderate tools (inline prompt):**
```
  ── src/auth.rs ──────────────────────────────────
  @@ -45,1 +45,1 @@
  - if token.expired() {
  + if token.is_expired() {
  ──────────────────────── Apply? [y/N/a] ────────
```

**Dangerous tools (highlighted prompt with warning):**
```
  ⚠ Shell command:
  │ cargo test --release
  │
  │ This will execute a shell command.
  └─ [y] Run  [N] Cancel
```

For destructive patterns (`rm -rf`, `git push --force`, `DROP TABLE`):
```
  ⚠⚠ DESTRUCTIVE COMMAND:
  │ rm -rf target/
  │
  │ This cannot be undone.
  └─ Type 'yes' to confirm, anything else cancels:
```

### 13.6 Trust Mode

`Ctrl+T` toggles trust mode for the session:
```
  ⚠ Trust mode ENABLED — all operations auto-approved
  Git checkpoint created: forge-checkpoint-a1b2c3
```

In trust mode:
- All moderate operations auto-approve
- Dangerous operations still prompt (but with [y] as default)
- Git checkpoint created at start for rollback safety

### 13.7 Status Bar

Fixed at bottom. Single line. Dense information:

```
forge │ main ↑2 │ 1 staged 3 modified │ 245/32K tok │ 12.3 tok/s │ 3m
```

Fields:
- `forge` — app name
- `main ↑2` — git branch + commits ahead of remote
- `1 staged 3 modified` — git working tree status
- `245/32K tok` — tokens used / context window size
- `12.3 tok/s` — current generation speed
- `3m` — session duration

When model is generating, status bar shows:
```
forge │ main │ ████████░░ 78% │ generating... 342 tok │ 12.3 tok/s │ 3m
```

### 13.8 Commands

Typed at the `>` prompt with `/` prefix:

| Command | Action |
|---------|--------|
| `/help` | Show available commands |
| `/clear` | Clear conversation (keep session) |
| `/compact` | Summarize old messages, free context window |
| `/trust` | Toggle trust mode |
| `/verbose` | Toggle verbose tool display |
| `/diff <file>` | Show git diff for file |
| `/undo` | Undo last file edit (git checkout) |
| `/rollback` | Restore to session start checkpoint |
| `/index` | Rebuild project search index |
| `/search <query>` | Semantic search the codebase |
| `/model` | Show current model info |
| `/session` | Show session stats |
| `/evolution` | Show auto-generated rules |
| `/quit` | End session (generates summary for AKE) |

### 13.9 Streaming Response

Tokens appear character by character as they generate. The cursor blinks at the end of the stream:

```
  Fixed: `token.expired()` → `token.is_expired()` in src/auth.rs:45.█
```

When the model emits a tool call mid-response, the streaming pauses, tool executes, result is injected, and streaming resumes. The user sees:

```
  Let me check the file...
  Reading src/auth.rs... found the issue on line 45.
  The method was renamed...█
```

Not:
```
  Let me check the file.
  <tool_call>{"name":"file_read"...}</tool_call>
  [Tool result: 2450 chars]
  The method was renamed...
```

The tool call mechanics are hidden. The user sees a natural conversation.

### 13.10 Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Submit message |
| `Ctrl+C` | Cancel current generation |
| `Ctrl+D` | Quit (with session save) |
| `Ctrl+T` | Toggle trust mode |
| `Ctrl+V` | Toggle verbose mode |
| `Ctrl+L` | Clear screen |
| `↑/↓` | Scroll conversation history |
| `Tab` | Autocomplete file paths in message |
| `Esc` | Cancel current approval prompt |

### 13.11 First Run Experience

```
$ forge

  ╔═══════════════════════════════════════════╗
  ║  forge v0.2.0 — local AI dev assistant   ║
  ╚═══════════════════════════════════════════╝

  Hardware detected:
    CPU:  Apple M2 Pro
    RAM:  16 GB unified memory
    GPU:  Metal (Apple Silicon)

  Recommended model: Qwen3.5-35B-A3B-4bit (MoE, 3B active)
    Size: 4.0 GB │ Backend: MLX │ Context: 32K tokens

  Download now? [Y/n] █

  Downloading Qwen3.5-35B-A3B-4bit...
  [████████████████████████████████] 4.0 GB  Done.

  Downloading BGE-small-en-v1.5 (embedding model)...
  [████████████████████████████████] 130 MB  Done.

  Ready. Run 'forge' in any project directory to start.
```

### 13.12 Orchestrator Loop (Wiring Everything Together)

This is the main loop that connects all modules:

```
1. User types message at > prompt
2. SessionManager saves user message
3. Gather context:
   a. RTAI search: embed query, retrieve relevant code chunks
   b. AKE: load previous session summary
   c. Evolution: load active rules
   d. KnowledgeSampler: preload entity trie for detected entities
4. Build system prompt:
   a. Identity + behavioral rules
   b. Tool definitions (13 tools)
   c. Active FTAI Rules
   d. Retrieved code context (max 2000 tokens)
   e. Previous session summary
5. Call inference engine:
   a. Forward pass through llama.cpp (or MLX)
   b. KnowledgeSampler modifies logits per token
   c. Stream tokens to TUI
6. Parse response:
   a. ModelAdapter detects tool calls (Qwen 3.5 XML)
   b. StreamingToolCallParser handles chunk boundaries
   c. ToolCallValidator checks schema
   d. If parse fails: RecoveryPipeline (3 attempts)
7. If tool calls found:
   a. Classify safety level (safe/moderate/dangerous)
   b. Check FTAI Rules (may block or modify)
   c. If needs approval: show inline prompt, wait for user
   d. Execute tool, capture result
   e. Truncate result if needed (TokenBudget)
   f. Inject result as Tool message
   g. Go to step 5 (continue generation)
   h. Max 10 iterations
8. If no tool calls (final response):
   a. Display response in chat area
   b. SessionManager saves assistant message
   c. Evolution engine logs tool call sequence + outcome
9. Wait for next user message (go to step 1)
10. On /quit:
    a. Generate session summary (using the model)
    b. Save to SessionManager (AKE)
    c. Evolution engine analyzes session outcomes
    d. Git checkpoint cleanup
```

### 13.13 ratatui Widget Mapping

| IDE Component | ratatui Equivalent |
|--------------|-------------------|
| AIAgentPanel (chat) | Paragraph widget with Line spans for colors |
| ApprovalModal | Popup Clear area + bordered Block |
| DiffViewer | Scrollable List with red/green Spans |
| StatusBar | Bottom Paragraph with fixed layout |
| ConsoleOutput | Scrollable Paragraph with styled lines |
| BackgroundTasksPanel | Not needed for v1 (single-threaded tool execution) |
| Progress bar | Gauge widget in status bar area |
| Input line | Input widget at bottom of chat area |

### 13.14 Color Scheme

```
Background:        terminal default (respects user theme)
User message:      bold white
Agent response:    default (dim white)
Tool execution:    dim cyan (collapsed) / cyan (expanded)
Diff added:        green
Diff removed:      red
Diff context:      dim
Approval prompt:   yellow
Dangerous warning: bold red
Status bar:        inverse (bg: white, fg: black)
Error:             bold red
Success:           green
Progress bar:      cyan
```

---

## Appendix A: Database Summary

Forge uses three SQLite databases, all stored per-project:

| Database | Path | Purpose |
|----------|------|---------|
| `search.db` | `~/.ftai/projects/<hash>/search.db` | Code search index (chunks + embeddings) |
| `sessions.db` | `~/.ftai/sessions/sessions.db` | Conversation history (AKE) |
| `evolution.db` | `~/.ftai/evolution/evolution.db` | Self-evolution data (Mitosis) |
| `ftai.db` | `~/.ftai/ftai.db` | FTAI rules/knowledge (optional, large-scale mode only) |

All databases use WAL mode for concurrent read/write.

---

## Appendix B: Bottlenecks and Mitigation

| Bottleneck | Impact | Mitigation |
|------------|--------|------------|
| Model loading time | 5-15s cold start | Keep model loaded between sessions. Warm start via `forge daemon`. |
| First token latency | 200-500ms (prompt processing) | Batch prompt tokens. Use flash attention. Cache system prompt prefix in KV. |
| Token generation speed | 15-40 tok/s on 16GB Mac | MoE model (3B active) is faster than dense equivalent. Q4 quantization helps. |
| Large file reads | Tool results blow context budget | Truncation with middle-out strategy. Read specific line ranges. |
| Code indexing (first run) | 5-30s for large projects | Background indexing. Progressive -- search works after first 100 files. |
| Re-indexing on file change | Blocks search briefly | Async re-index. Queue changes, batch-process every 2s. |
| Evolution analysis | CPU at session end | Fully async. Non-blocking. Skipped if < 3 sessions. |
| GBNF grammar activation | Must detect `<tool_call>` prefix in stream | Simple string prefix matching on accumulated output buffer. ~0 overhead. |
| KV cache OOM on 16GB | 32K context with F16 KV = 3GB+ | Default to Q8_0 KV (1.5GB). Future: Q4 (0.8GB). |

---

## Appendix C: Security Considerations

All security patterns from FTAI's FolkTech Secure Coding Standard apply:

1. **Path traversal:** All file tools resolve paths through `canonicalize()` and check against project root. Same as existing `src/tools/` implementations.
2. **Shell injection:** Bash tool uses `Command::new("bash").arg("-c").arg(&cmd)` -- no shell expansion of user input.
3. **LLM output injection:** Tool results are clearly delimited in the conversation. The model cannot inject system-level instructions via tool output.
4. **Plugin sandboxing:** Plugin tools are namespaced (`plugin:<name>:<tool>`) and go through the full permission pipeline. 30s execution timeout.
5. **Model download verification:** SHA-256 checksums verified after download. Model files are read-only after installation.
6. **Evolution rule injection:** Auto-generated rules go through the same parser as user rules. No arbitrary code execution. Confidence threshold prevents spurious rules.
7. **RTAI index security:** Search index is per-project. No cross-project data leakage. Index files are user-readable only (0600 permissions).

---

## Appendix D: Migration Path from FTAI

Since Forge is built on top of the existing FTAI codebase:

1. **Rename:** `ftai` -> `forge` in Cargo.toml, binary name, CLI output
2. **Keep `ftai` as alias:** `forge` binary also responds to `ftai` (symlink or clap alias)
3. **Config compatibility:** `~/.ftai/` directory name stays. Config format stays. Users don't need to migrate anything.
4. **Test compatibility:** All 212 existing tests must pass after rename. New modules add ~150 more tests.
5. **Model compatibility:** Existing models in `~/.ftai/models/` work unchanged.

Total estimated new code: ~6,500 lines:
  - inference/ (FFI + sampler): ~1,000 lines
  - inference/knowledge_sampler.rs (Knowledge Grounding): ~1,500 lines
  - search/ (RTAI code search): ~800 lines
  - evolution/ (Mitosis self-evolution): ~700 lines
  - session/ (AKE persistence): ~500 lines
  - conversation/ expansions (adapter, streaming, recovery, grammar): ~1,000 lines
  - tests: ~1,000 lines
Total reused code: ~8,000 lines (everything in the existing FTAI codebase).
