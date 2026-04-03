# FTAI Tool Calling Subsystem -- Technical Design Document

**Date:** 2026-03-27
**Status:** Draft
**Prerequisite:** `2026-03-02-ftai-terminal-harness-design.md`
**Scope:** Complete tool-calling pipeline from prompt construction to result extraction

---

## 1. Problem Statement

Tool calling is the single hardest problem in building a local AI coding assistant. Frontier models (Claude, GPT-4) have tool calling fine-tuned into the weights and use structured output APIs. Local models -- even strong ones like Qwen 3.5 -- are significantly less reliable at producing well-formed tool calls. The gap is not small. A coding assistant that cannot reliably invoke tools is useless.

This document designs the complete tool-calling subsystem for FTAI, targeting Qwen 3.5 (9B, 27B, 35B-A3B) running via llama.cpp and MLX backends.

---

## 2. Qwen 3.5 Tool Calling Capabilities

### 2.1 Native Format

Qwen 3.5 was trained on the **Qwen3-Coder XML format**, distinct from the Hermes-style JSON format used by Qwen3 and Qwen 2.5.

**Qwen 3.5 native tool call format:**

```xml
<tool_call>
<function=function_name>
<parameter=param_name>
value
</parameter>
<parameter=param_name2>
value2
</parameter>
</function>
</tool_call>
```

This is NOT the Hermes JSON format. Using Hermes-style with Qwen 3.5 will produce garbage.

| Model Family | Tool Call Format | Parser |
|---|---|---|
| Qwen 2.5 | Hermes JSON | hermes |
| Qwen 3 | Hermes JSON | hermes |
| Qwen 3.5 | XML (function/parameter tags) | qwen3_coder |
| Qwen 3-Coder | XML (same as Qwen 3.5) | qwen3_coder |

### 2.2 Known Issues

1. **Broken HuggingFace template**: The official Qwen3.5 tokenizer_config.json has a Jinja bug -- `arguments | items` filter fails on mappings. Community fix (QwenLM/Qwen3#1831) uses `.items()` with mapping validation.

2. **Thinking block contamination**: Qwen3.5 9B with `enable_thinking=true` emits tool calls INSIDE `<think>` blocks. The PEG parser then fails. **Fix: auto-disable thinking when tools are active.**

3. **Parallel call interleaving**: Original template interleaved parallel calls. Fix: double-newline delimiters between `<tool_call>` blocks.

4. **Reliability**: Expect ~85-90% well-formed calls at 27B, ~70-80% at 9B. Error recovery is mandatory.

### 2.3 Recommended Inference Parameters

```
enable_thinking: false  (when tools are active)
temperature: 0.6
top_p: 0.95
top_k: 20
min_p: 0.0
presence_penalty: 0.0
```

Use the fixed chat template (unsloth or QwenLM#1831 fix), NOT the default HuggingFace template.

---

## 3. llama.cpp Tool Calling Support

### 3.1 Server-Side Native Support

llama.cpp server supports OpenAI-compatible tool calling with the `--jinja` flag:

1. Server reads chat template from GGUF or `--chat-template-file` override
2. Tools passed via OpenAI `tools` parameter in `/v1/chat/completions`
3. Native format handlers for recognized templates, generic fallback for others
4. Returns `tool_calls` array with `finish_reason: "tool_calls"`

**Server launch:**
```bash
llama-server \
  -m path/to/qwen3.5.gguf \
  --jinja \
  --chat-template-file path/to/fixed-qwen3.5-template.jinja \
  --port 8080 --host 127.0.0.1 \
  -ngl 99 -c 32768 -n 8192 \
  --temp 0.6 --top-k 20 --top-p 0.95
```

Do NOT use extreme KV quantization (`-ctk q4_0`) -- it degrades tool calling.

### 3.2 What llama.cpp Handles

- Injecting tool definitions into the prompt via Jinja template
- Formatting conversation history with role tags
- Parsing tool calls from model output (native or generic)
- Returning structured tool_calls in OpenAI-compatible response

### 3.3 What llama.cpp Does NOT Handle

- Error recovery for malformed output
- Validation that tool names/params match our schema
- Robust streaming accumulation (string fragments are fragile)
- Fallback when native parsing fails silently
- Retry logic

### 3.4 Architecture: Dual-Layer Parsing

```
Model generates response
  --> llama.cpp parses (native handler via --jinja)
     --> Has structured tool_calls? Use them directly.
     --> Plain text only? Our parser extracts tool calls from text.
  --> FTAI validates all tool calls against known tool schema
  --> Invalid? Error recovery pipeline (Section 7).
```

---

## 4. Prompt Engineering Strategy

### 4.1 Format Decision

**Primary**: Qwen 3.5 native XML format via `--jinja` + fixed template.
**Fallback**: Prompted XML format with same structure, parsed by FTAI.

Rejected alternatives:
- Hermes JSON -- Qwen 3.5 was not trained on it
- ReAct (Thought/Action/Observation) -- more tokens, lower reliability on small models
- Freeform JSON in code blocks -- too unreliable

### 4.2 Why Not ReAct?

Aider benchmarks show plain text edit formats outperform structured/function-calling formats. ReAct adds formatting overhead that smaller models struggle with. The XML format is in Qwen 3.5's training data, giving us reliability without extra prompting cost.

### 4.3 System Prompt Design

Two modes:

**Native Mode** (`--jinja` handles tool injection): System prompt contains only identity + behavior instructions + project context. Tool definitions injected by the Jinja template. This is the existing `build_system_prompt()` in `src/conversation/prompt.rs` with tool-calling behavior instructions added.

**Prompted Mode** (fallback): System prompt includes identity + behavior + tool schemas + few-shot examples. Tool schemas rendered in compact human-readable format (not full JSON Schema) to save tokens.

### 4.4 Few-Shot Examples (prompted mode)

Two examples embedded in the system prompt showing correct tool use. This is critical for smaller models that need demonstration to follow the format reliably.

**Example 1 -- Single tool call:**

```
User: What files are in the src directory?

Assistant: Let me check.

<tool_call>
<function=bash>
<parameter=command>ls -la /path/to/project/src/</parameter>
<parameter=description>List files in src directory</parameter>
</function>
</tool_call>
```

**Example 2 -- Multiple tool calls:**

```
User: Read both config files.

Assistant: I will read both files.

<tool_call>
<function=file_read>
<parameter=file_path>/path/to/project/config.toml</parameter>
</function>
</tool_call>

<tool_call>
<function=file_read>
<parameter=file_path>/path/to/project/settings.json</parameter>
</function>
</tool_call>
```

**Example 3 -- No tool needed:**

```
User: What does the pub keyword mean in Rust?

Assistant: The pub keyword in Rust makes an item public, allowing it to be
accessed from outside its defining module. Without pub, items are private
to their module by default.
```

---

## 5. Tool Call Parsing in Rust

### 5.1 Parser Architecture

The parser handles three input sources:

1. **Native API tool calls** -- structured `tool_calls` from llama.cpp OpenAI response
2. **XML tool calls in text** -- `<tool_call>...</tool_call>` blocks in plain text
3. **Hermes JSON in text** -- `<tool_call>\n{"name":...}\n</tool_call>` (for Qwen 2.5/3 compat)

### 5.2 Parsing Pipeline

```rust
pub enum ParsedResponse {
    /// Pure text, no tool calls
    Text(String),
    /// Text before tool calls + the tool calls themselves
    ToolCalls {
        preamble: String,
        calls: Vec<ToolCall>,
    },
}

pub struct ToolCallParser {
    known_tools: HashSet<String>,
    mode: ToolCallingMode,
}

impl ToolCallParser {
    /// Parse a complete response (non-streaming)
    pub fn parse_response(
        &self,
        api_tool_calls: Option<Vec<ToolCall>>,
        text: &str,
    ) -> ParsedResponse {
        // Layer 1: If API returned structured tool calls, use them
        if let Some(calls) = api_tool_calls {
            if !calls.is_empty() {
                return ParsedResponse::ToolCalls {
                    preamble: text.to_string(),
                    calls: self.validate_calls(calls),
                };
            }
        }

        // Layer 2: Parse XML tool calls from text
        if let Some(parsed) = self.parse_xml_tool_calls(text) {
            return parsed;
        }

        // Layer 3: Parse Hermes JSON tool calls from text
        if let Some(parsed) = self.parse_hermes_tool_calls(text) {
            return parsed;
        }

        // No tool calls found
        ParsedResponse::Text(text.to_string())
    }
}
```

### 5.3 XML Parser (Primary -- Qwen 3.5)

```rust
/// Parse Qwen3-Coder XML format tool calls from text.
/// Handles: single calls, multiple calls, mixed text + calls.
fn parse_xml_tool_calls(&self, text: &str) -> Option<ParsedResponse> {
    let re = Regex::new(
        r"(?s)<tool_call>\s*<function=(\w+)>(.*?)</function>\s*</tool_call>"
    ).unwrap();

    let mut calls = Vec::new();
    let mut preamble = text.to_string();

    for cap in re.captures_iter(text) {
        let full_match = cap.get(0).unwrap().as_str();
        let func_name = cap.get(1).unwrap().as_str().to_string();
        let params_block = cap.get(2).unwrap().as_str();

        let arguments = self.parse_xml_parameters(params_block);
        preamble = preamble.replace(full_match, "");

        calls.push(ToolCall {
            id: format!("tc_{}", calls.len() + 1),
            name: func_name,
            arguments,
        });
    }

    if calls.is_empty() {
        return None;
    }
    Some(ParsedResponse::ToolCalls {
        preamble: preamble.trim().to_string(),
        calls,
    })
}

/// Parse <parameter=name>value</parameter> pairs from a function block.
fn parse_xml_parameters(&self, block: &str) -> serde_json::Value {
    let param_re = Regex::new(
        r"(?s)<parameter=(\w+)>\s*(.*?)\s*</parameter>"
    ).unwrap();

    let mut map = serde_json::Map::new();
    for cap in param_re.captures_iter(block) {
        let key = cap.get(1).unwrap().as_str().to_string();
        let value = cap.get(2).unwrap().as_str().trim().to_string();

        // Try to parse as JSON value (number, bool, object)
        // Fall back to string
        let json_val = serde_json::from_str(&value)
            .unwrap_or(serde_json::Value::String(value));
        map.insert(key, json_val);
    }
    serde_json::Value::Object(map)
}
```

### 5.4 Streaming Tool Call Parser

Streaming is harder because tokens arrive one at a time and tool call XML may span many tokens.

```rust
pub struct StreamingToolCallParser {
    buffer: String,
    state: StreamState,
    current_call: Option<PartialToolCall>,
    completed_calls: Vec<ToolCall>,
    preamble: String,
}

enum StreamState {
    /// Accumulating text before any tool call
    Text,
    /// Seen '<tool_call>', accumulating function/params
    InToolCall,
    /// Seen '<function=', accumulating function name
    InFunctionName,
    /// Inside a parameter block
    InParameter { name: String },
}

struct PartialToolCall {
    func_name: String,
    params: serde_json::Map<String, serde_json::Value>,
}

impl StreamingToolCallParser {
    /// Feed a new token into the parser.
    /// Returns any text that should be displayed to the user
    /// (preamble text before tool calls).
    pub fn feed(&mut self, token: &str) -> Option<String> {
        self.buffer.push_str(token);

        match self.state {
            StreamState::Text => {
                // Check if buffer ends with start of "<tool_call>"
                if self.buffer.contains("<tool_call>") {
                    let idx = self.buffer.find("<tool_call>").unwrap();
                    let text_before = self.buffer[..idx].to_string();
                    self.preamble.push_str(&text_before);
                    self.buffer = self.buffer[idx + "<tool_call>".len()..].to_string();
                    self.state = StreamState::InToolCall;
                    if !text_before.is_empty() {
                        return Some(text_before);
                    }
                    return None;
                }
                // Check for potential partial match at end of buffer
                // e.g., buffer ends with "<tool" -- hold it back
                if let Some(partial_idx) = find_partial_tag_start(&self.buffer, "<tool_call>") {
                    let safe = self.buffer[..partial_idx].to_string();
                    self.buffer = self.buffer[partial_idx..].to_string();
                    self.preamble.push_str(&safe);
                    if !safe.is_empty() {
                        return Some(safe);
                    }
                    return None;
                }
                // No tool call starting -- flush buffer as text
                let text = std::mem::take(&mut self.buffer);
                self.preamble.push_str(&text);
                Some(text)
            }
            StreamState::InToolCall => {
                // Look for <function= to start parsing
                if let Some(idx) = self.buffer.find("<function=") {
                    let after = &self.buffer[idx + "<function=".len()..];
                    if let Some(end) = after.find(">") {
                        let name = after[..end].to_string();
                        self.current_call = Some(PartialToolCall {
                            func_name: name,
                            params: serde_json::Map::new(),
                        });
                        self.buffer = after[end + 1..].to_string();
                        self.state = StreamState::InFunctionName;
                    }
                }
                None // Tool call content not shown to user
            }
            StreamState::InFunctionName => {
                // Look for <parameter= or </function>
                if let Some(idx) = self.buffer.find("<parameter=") {
                    let after = &self.buffer[idx + "<parameter=".len()..];
                    if let Some(end) = after.find(">") {
                        let param_name = after[..end].to_string();
                        self.buffer = after[end + 1..].to_string();
                        self.state = StreamState::InParameter {
                            name: param_name,
                        };
                    }
                } else if self.buffer.contains("</function>") {
                    self.finalize_current_call();
                    self.buffer = self.buffer.split("</function>")
                        .last().unwrap_or("").to_string();
                }
                None
            }
            StreamState::InParameter { ref name } => {
                if let Some(idx) = self.buffer.find("</parameter>") {
                    let value = self.buffer[..idx].trim().to_string();
                    let name = name.clone();
                    if let Some(ref mut call) = self.current_call {
                        let json_val = serde_json::from_str(&value)
                            .unwrap_or(serde_json::Value::String(value));
                        call.params.insert(name, json_val);
                    }
                    self.buffer = self.buffer[idx + "</parameter>".len()..].to_string();
                    self.state = StreamState::InFunctionName;
                }
                None
            }
        }
    }

    /// Call when generation is complete. Returns all parsed tool calls.
    pub fn finalize(mut self) -> ParsedResponse {
        self.finalize_current_call();
        if self.completed_calls.is_empty() {
            ParsedResponse::Text(self.preamble)
        } else {
            ParsedResponse::ToolCalls {
                preamble: self.preamble.trim().to_string(),
                calls: self.completed_calls,
            }
        }
    }

    fn finalize_current_call(&mut self) {
        if let Some(call) = self.current_call.take() {
            self.completed_calls.push(ToolCall {
                id: format!("tc_{}", self.completed_calls.len() + 1),
                name: call.func_name,
                arguments: serde_json::Value::Object(call.params),
            });
            self.state = StreamState::Text;
        }
    }
}

/// Find the start index of a potential partial tag match at end of buffer.
/// e.g. buffer="hello <too" with tag="<tool_call>" returns Some(6)
fn find_partial_tag_start(buffer: &str, tag: &str) -> Option<usize> {
    for i in 1..tag.len() {
        if buffer.ends_with(&tag[..i]) {
            return Some(buffer.len() - i);
        }
    }
    None
}
```

### 5.5 Validation Layer

After parsing, every tool call is validated before execution:

```rust
pub struct ToolCallValidator {
    known_tools: HashMap<String, ToolSchema>,
}

pub enum ValidationResult {
    Valid(ToolCall),
    UnknownTool { name: String, call: ToolCall },
    MissingRequiredParam { tool: String, param: String, call: ToolCall },
    WrongParamType { tool: String, param: String, expected: String, call: ToolCall },
    Malformed { reason: String, raw_text: String },
}

impl ToolCallValidator {
    pub fn validate(&self, call: ToolCall) -> ValidationResult {
        // 1. Check tool exists
        let schema = match self.known_tools.get(&call.name) {
            Some(s) => s,
            None => return ValidationResult::UnknownTool {
                name: call.name.clone(), call
            },
        };

        // 2. Check required parameters
        for param in &schema.required_params {
            if !call.arguments.get(param).is_some() {
                return ValidationResult::MissingRequiredParam {
                    tool: call.name.clone(),
                    param: param.clone(),
                    call,
                };
            }
        }

        // 3. Type check parameters (best-effort)
        for (name, expected_type) in &schema.param_types {
            if let Some(val) = call.arguments.get(name) {
                if !type_matches(val, expected_type) {
                    return ValidationResult::WrongParamType {
                        tool: call.name.clone(),
                        param: name.clone(),
                        expected: expected_type.clone(),
                        call,
                    };
                }
            }
        }

        ValidationResult::Valid(call)
    }
}
```

---

## 6. Constrained Generation (GBNF Grammar)

### 6.1 Can We Use It?

llama.cpp supports GBNF grammars for constrained decoding. In theory, we could force the model to produce valid tool call XML. In practice, there are significant tradeoffs.

**Pros:**
- Guarantees syntactically valid XML tool calls
- Eliminates malformed output entirely
- No need for error recovery on syntax errors

**Cons:**
- Grammar applies to the ENTIRE generation, not just tool call portions
- Cannot mix free-text responses with tool calls under a single grammar
- Model must decide at token 0 whether to use a tool or respond in text
- Significantly constrains the model's ability to reason before calling tools
- Performance overhead from grammar sampling
- llama.cpp's grammar-based sampling does NOT apply to tool calling via --jinja (grammars are for completions, not chat)

### 6.2 Decision: Do NOT Use GBNF for Primary Tool Calling

The constraint that grammar applies to the entire output is a dealbreaker. We need the model to freely mix text ("Let me check that file.") with tool calls. GBNF cannot express "output arbitrary text, then optionally emit one or more well-formed tool call XML blocks."

### 6.3 GBNF Grammar for Retry Mode Only

When the model fails to produce a valid tool call on the first attempt, we can use GBNF on the RETRY to force valid XML. The retry prompt explicitly asks for a tool call, so constraining the entire output to XML is acceptable.

**GBNF grammar for a single Qwen3-Coder tool call:**

```gbnf
root ::= "<tool_call>\n" function-block "\n</tool_call>"

function-block ::= "<function=" tool-name ">\n" parameter-list "</function>"

tool-name ::= [a-z_]+

parameter-list ::= (parameter "\n")*

parameter ::= "<parameter=" param-name ">\n" param-value "\n</parameter>"

param-name ::= [a-z_]+

param-value ::= [^<]+
```

**GBNF grammar constrained to known tool names:**

```gbnf
root ::= "<tool_call>\n" function-block "\n</tool_call>"

function-block ::= "<function=" known-tool ">\n" parameter-list "</function>"

known-tool ::= "bash" | "file_read" | "file_write" | "file_edit" | "glob" | "grep" | "git_status" | "git_diff" | "git_commit" | "git_log" | "ask_user"

parameter-list ::= (parameter "\n")*

parameter ::= "<parameter=" param-name ">\n" param-value "\n</parameter>"

param-name ::= [a-z_]+

param-value ::= [^<]+
```

This grammar is dynamically generated at runtime from the tool registry, ensuring it always matches the available tools.

---

## 7. Error Recovery

### 7.1 Failure Taxonomy

| Failure Type | Frequency (est.) | Recovery Strategy |
|---|---|---|
| Malformed XML (unclosed tags) | 5-10% | Regex repair, then re-parse |
| Unknown tool name | 2-5% | Fuzzy match against known tools |
| Missing required parameter | 5-8% | Re-prompt with specific instruction |
| Wrong parameter type | 2-3% | Type coercion (string to int, etc.) |
| Tool call inside think block | 10-15% (9B only) | Extract from think block |
| Hermes JSON instead of XML | 3-5% | Parse as Hermes, convert |
| No tool call when one was expected | 5-10% | Re-prompt with nudge |
| Hallucinated parameters | 3-5% | Strip unknown params, validate |

### 7.2 Recovery Pipeline

```rust
pub struct ErrorRecovery {
    max_retries: usize,  // default: 2
    known_tools: HashMap<String, ToolSchema>,
}

impl ErrorRecovery {
    pub fn recover(
        &self,
        failure: ValidationResult,
    ) -> RecoveryAction {
        match failure {
            // Try to fix it ourselves
            ValidationResult::UnknownTool { name, call } => {
                if let Some(closest) = fuzzy_match(&name, &self.known_tools) {
                    RecoveryAction::FixAndRetry(ToolCall {
                        name: closest,
                        ..call
                    })
                } else {
                    RecoveryAction::RepromptModel(format!(
                        "The tool '{}' does not exist. Available tools: {}. Please try again.",
                        name,
                        self.known_tools.keys().join(", ")
                    ))
                }
            }

            // Ask the model to fix it
            ValidationResult::MissingRequiredParam { tool, param, .. } => {
                RecoveryAction::RepromptModel(format!(
                    "Your {} call is missing the required '{}'parameter. Please emit the tool call again with all required parameters.",
                    tool, param
                ))
            }

            // Try type coercion
            ValidationResult::WrongParamType { tool, param, expected, call } => {
                if let Some(fixed) = try_coerce_type(
                    call.arguments.get(&param),
                    &expected
                ) {
                    let mut args = call.arguments.clone();
                    args[&param] = fixed;
                    RecoveryAction::FixAndRetry(ToolCall {
                        arguments: args,
                        ..call
                    })
                } else {
                    RecoveryAction::RepromptModel(format!(
                        "Parameter '{}' for {} should be type {}.",
                        param, tool, expected
                    ))
                }
            }

            // Malformed -- try regex repair or re-prompt
            ValidationResult::Malformed { reason, raw_text } => {
                if let Some(repaired) = attempt_xml_repair(&raw_text) {
                    RecoveryAction::ReparseText(repaired)
                } else {
                    RecoveryAction::RepromptModel(format!(
                        "Your tool call was malformed: {}. Please try again using the exact format.",
                        reason
                    ))
                }
            }
        }
    }
}

pub enum RecoveryAction {
    /// We fixed it ourselves, re-validate and execute
    FixAndRetry(ToolCall),
    /// Re-parse the repaired text through the full pipeline
    ReparseText(String),
    /// Send a message back to the model asking it to fix
    RepromptModel(String),
    /// Give up after max retries
    GiveUp(String),
}
```

### 7.3 XML Repair Heuristics

```rust
fn attempt_xml_repair(text: &str) -> Option<String> {
    let mut repaired = text.to_string();

    // Fix 1: Close unclosed </tool_call>
    if repaired.contains("<tool_call>") && !repaired.contains("</tool_call>") {
        repaired.push_str("\n</function>\n</tool_call>");
    }

    // Fix 2: Close unclosed </function>
    if repaired.contains("<function=") && !repaired.contains("</function>") {
        // Find last </parameter> and append </function>
        if let Some(idx) = repaired.rfind("</parameter>") {
            repaired.insert_str(idx + "</parameter>".len(), "\n</function>");
        }
    }

    // Fix 3: Close unclosed </parameter>
    let open_count = repaired.matches("<parameter=").count();
    let close_count = repaired.matches("</parameter>").count();
    if open_count > close_count {
        repaired.push_str("\n</parameter>");
    }

    // Fix 4: Extract tool call from <think> block
    if repaired.contains("<think>") && repaired.contains("<tool_call>") {
        if let Some(start) = repaired.find("<tool_call>") {
            if let Some(end) = repaired.find("</tool_call>") {
                let extracted = &repaired[start..end + "</tool_call>".len()];
                return Some(extracted.to_string());
            }
        }
    }

    // Fix 5: Handle Hermes JSON format as fallback
    if repaired.contains("<tool_call>") && repaired.contains("\"name\"") {
        // Delegate to Hermes parser
        return None;  // Let the Hermes parser handle it
    }

    // Re-validate the repaired text
    Some(repaired)
}
```

### 7.4 Retry Protocol

```
Attempt 1: Normal generation
  --> Parse fails?
    --> Attempt XML repair (no model call)
    --> Repair succeeds? Re-parse and validate.

Attempt 2: Re-prompt with error message
  --> Add system message: "Your previous tool call was malformed: [reason]."
  --> Normal generation (no grammar constraint)
  --> Parse fails again?

Attempt 3: Constrained retry with GBNF grammar
  --> Switch to completion endpoint with GBNF grammar
  --> Prefix the generation with the tool call opening tag
  --> Grammar forces valid XML
  --> This WILL produce valid XML but may have wrong content

Attempt 4: Give up
  --> Log the failure
  --> Show user: "Failed to execute tool call after 3 attempts."
  --> Continue conversation without the tool result
```

---

## 8. Streaming Architecture

### 8.1 Token Flow

```
llama.cpp SSE stream
  --> HttpModelClient.generate_stream()
     --> Token channel (mpsc)
        --> StreamingToolCallParser.feed(token)
           --> Displayable text? Send to TUI immediately.
           --> Tool call being built? Buffer silently.
        --> On stream end: parser.finalize()
           --> Returns ParsedResponse with tool calls
```

### 8.2 Dual-Path Streaming

When llama.cpp native tool calling is active, the server accumulates tool calls in the streaming response via `delta.tool_calls` chunks. Our `HttpModelClient` already handles this (see `generate_stream()` in `http_client.rs`). The streaming tool call parser is the fallback for when we need to parse from text.

### 8.3 UI Feedback During Tool Calls

While a tool call is being accumulated (parser is in `InToolCall`/`InFunctionName`/`InParameter` state), the TUI should show a spinner or "Preparing tool call..." indicator. Once the call is complete and validated, show the tool name and parameters in a collapsible block.

---

## 9. Model-Specific Adapters

### 9.1 Adapter Trait

Different models need different prompt formats and parsers. Abstract this behind an adapter:

```rust
pub trait ModelAdapter: Send + Sync {
    /// Name of this adapter (for config/logging)
    fn name(&self) -> &str;

    /// Whether this model supports native tool calling via the server API
    fn supports_native_tool_calling(&self) -> bool;

    /// Build the system prompt for this model
    fn build_system_prompt(
        &self,
        context: &PromptContext,
        tools: &[ToolDefinition],
    ) -> String;

    /// Parse tool calls from model output
    fn parse_tool_calls(
        &self,
        api_calls: Option<Vec<ToolCall>>,
        text: &str,
    ) -> ParsedResponse;

    /// Create a streaming parser for this model
    fn streaming_parser(&self) -> Box<dyn StreamingParser>;

    /// Inference parameters optimized for this model
    fn inference_params(&self) -> InferenceParams;

    /// GBNF grammar for constrained retry (if supported)
    fn retry_grammar(&self, tools: &[ToolDefinition]) -> Option<String>;
}
```

### 9.2 Adapters

| Adapter | Models | Native TC | Format | Thinking |
|---|---|---|---|---|
| `Qwen35Adapter` | Qwen 3.5 (all sizes) | Yes (with fixed template) | XML (qwen3_coder) | Auto-disabled with tools |
| `Qwen3Adapter` | Qwen 3, Qwen 2.5 | Yes | Hermes JSON | Supported |
| `GenericAdapter` | Any model | No | Prompted XML | N/A |
| `DeepSeekAdapter` | DeepSeek Coder V2/V3 | Yes | Native | Supported |

The adapter is selected based on model name pattern matching during model load.

---

## 10. Integration with Existing Code

### 10.1 Changes to `src/conversation/parser.rs`

The existing `ToolCallParser` is a good start but needs significant expansion:

1. Add Qwen3-Coder XML parsing (the `<function=...><parameter=...>` format)
2. Add the streaming parser
3. Add the validation layer
4. Add error recovery
5. Support both native and prompted modes in the same parser

### 10.2 Changes to `src/conversation/prompt.rs`

1. Add tool-calling behavior instructions to native mode prompt
2. Add full prompted mode prompt with tool schemas and few-shot examples
3. Generate prompted tool schemas from `ToolDefinition` structs
4. Dynamic few-shot example generation based on available tools

### 10.3 Changes to `src/conversation/engine.rs`

1. After receiving a response, run it through the dual-layer parser
2. On validation failure, invoke error recovery
3. On retry, add the error message to conversation and re-generate
4. Track retry count per turn, enforce max_retries

### 10.4 Changes to `src/backend/http_client.rs`

1. The existing streaming tool call accumulation is solid
2. Add: pass `parse_tool_calls: true` in request body when using --jinja
3. Add: GBNF grammar parameter for constrained retry mode
4. Add: separate endpoint call for completion mode (vs chat mode) for grammar retry

### 10.5 New Files

```
src/conversation/
  adapter.rs        -- ModelAdapter trait + implementations
  streaming.rs      -- StreamingToolCallParser
  validator.rs      -- ToolCallValidator
  recovery.rs       -- ErrorRecovery pipeline
  grammar.rs        -- GBNF grammar generation for retry mode
```

---

## 11. Benchmarks and References

### 11.1 How Others Handle This

**Aider**: Does NOT use function calling at all. Uses text-based edit formats (whole file, search/replace blocks, unified diffs). Found that function calling performed worse than plain text. This is instructive -- for code editing specifically, text formats may be better. But FTAI needs tool calling for bash, git, grep, etc., not just file edits.

**llama.cpp PR #16932**: Generalized XML-style parser supporting 7 model families. Uses a builder pattern for incremental parsing, supports streaming, handles interleaved reasoning blocks. This is the closest reference implementation to what we need.

**Ollama**: Had a critical bug where Qwen 3.5 was wired to the wrong parser (hermes instead of qwen3-coder). Demonstrates that format detection must be correct or tool calling silently fails.

**vLLM**: Supports `--tool-call-parser qwen3_coder` flag. The fact that they needed a separate parser for Qwen 3.5 confirms our finding that it uses a different format from Qwen 3.

### 11.2 Expected Reliability

Based on community reports and llama.cpp issue tracker:

| Model | Size | Format | Expected Reliability |
|---|---|---|---|
| Qwen3.5-27B | 27B | Native XML | ~90% first-attempt, ~98% with retry |
| Qwen3.5-35B-A3B | 35B (3B active) | Native XML | ~85% first-attempt, ~95% with retry |
| Qwen3.5-9B | 9B | Native XML | ~75% first-attempt, ~90% with retry |
| Qwen3.5-4B | 4B | Native XML | ~55% first-attempt, ~75% with retry |

The retry pipeline with GBNF constrained generation is expected to close most of the gap. The remaining failures will be semantic (wrong tool, wrong params) rather than syntactic.

---

## 12. Implementation Priority

### Phase 1 (Critical Path)
1. XML parser for Qwen3-Coder format (complete + streaming)
2. Dual-layer parsing (native API + text fallback)
3. Tool call validation against schema
4. Error recovery with re-prompt
5. Fixed Qwen 3.5 chat template bundled with FTAI

### Phase 2 (Reliability)
6. GBNF grammar generation for constrained retry
7. XML repair heuristics
8. Thinking block extraction (for models that leak tool calls into think blocks)
9. Fuzzy tool name matching

### Phase 3 (Multi-Model)
10. ModelAdapter trait + Qwen35Adapter
11. GenericAdapter for BYO models
12. Hermes JSON adapter for Qwen 3/2.5

---

## 13. Testing Strategy

### 13.1 Unit Tests (no model required)

```
tests/conversation/
  test_xml_parser.rs          -- Parse well-formed XML tool calls
  test_xml_parser_malformed.rs -- Parse broken XML, verify repair
  test_hermes_parser.rs       -- Parse Hermes JSON format
  test_streaming_parser.rs    -- Feed tokens one at a time, verify result
  test_validator.rs           -- Validate tool calls against schema
  test_recovery.rs            -- Error recovery actions
  test_grammar.rs             -- GBNF grammar generation
```

### 13.2 Integration Tests (model required)

```
tests/integration/
  test_tool_calling_qwen35.rs   -- End-to-end with Qwen 3.5
  test_tool_calling_retry.rs    -- Verify retry pipeline works
  test_streaming_tool_call.rs   -- Streaming + tool call detection
```

### 13.3 Corpus Tests

Build a corpus of real model outputs (both good and bad) from actual Qwen 3.5 generations. Run the parser against this corpus in CI. Every time we encounter a new failure mode in production, add it to the corpus.

```
tests/corpus/
  good/          -- Well-formed tool calls from real generations
  malformed/     -- Broken tool calls with expected repairs
  edge_cases/    -- Think block leaks, mixed formats, etc.
```

---

## 14. Key Risks

1. **Qwen 3.5 chat template instability**: The template has been broken on HuggingFace and fixed by the community multiple times. We should bundle our own verified template rather than relying on the GGUF-embedded one.

2. **llama.cpp parser changes**: The tool calling support in llama.cpp is under active development. Parser behavior may change between versions. Pin to a known-good llama.cpp version and test before upgrading.

3. **9B model reliability**: At 9B parameters, tool calling reliability drops significantly. The 35B-A3B MoE model (3B active parameters, 4GB on disk) may actually be less reliable than the dense 9B despite having more total parameters. Test both extensively.

4. **Context window pressure**: Tool definitions + few-shot examples + conversation history + tool results consume a lot of context. On a 32K context window, this leaves less room for actual code. Monitor context usage and compress aggressively.

5. **Silent failures**: The worst case is when llama.cpp's native parser silently drops a tool call or misparses it. The dual-layer approach mitigates this -- if the API returns no tool calls but the text contains tool call XML, we parse from text.

---

## Appendix A: Fixed Qwen 3.5 Chat Template

The fixed Jinja template should be downloaded from the QwenLM/Qwen3#1831 fix or unsloth's corrected GGUFs and stored at `~/.ftai/templates/qwen3.5-tool-calling.jinja`. Key fixes applied:

1. Replace `arguments | items` with `.items()` method + mapping guard
2. Auto-disable thinking when tools are present (`auto_disable_thinking_with_tools`)
3. Separate parallel tool calls with double-newline delimiters
4. Add existence and non-null guards for `reasoning_content`
5. Proper `tojson` serialization for complex argument values

## Appendix B: Source References

- Qwen function calling docs: https://qwen.readthedocs.io/en/latest/framework/function_call.html
- llama.cpp function calling docs: https://github.com/ggml-org/llama.cpp/blob/master/docs/function-calling.md
- Qwen 3.5 template fix: https://github.com/QwenLM/Qwen3/issues/1831
- Qwen 3.5 template bug (HF): https://huggingface.co/Qwen/Qwen3.5-35B-A3B/discussions/4
- Qwen 3.5 thinking block bug: https://github.com/ggml-org/llama.cpp/issues/20837
- llama.cpp XML parser PR: https://github.com/ggml-org/llama.cpp/pull/16932
- llama.cpp GBNF grammar docs: https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md
- Aider edit formats: https://aider.chat/docs/more/edit-formats.html
- Unsloth Qwen 3.5 local guide: https://unsloth.ai/docs/models/qwen3.5
- Ollama Qwen 3.5 tool calling bug: https://github.com/ollama/ollama/issues/14493
- vLLM tool calling docs: https://docs.vllm.ai/en/latest/features/tool_calling/
