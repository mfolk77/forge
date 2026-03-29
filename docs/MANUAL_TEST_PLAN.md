# Forge Manual Test Plan

Run these tests in your terminal after building with `cargo build --release`.
The binary is at `./target/release/forge` or `~/.local/bin/forge` if installed.

Check off each test as you go. Expected results are listed for each step.

---

## Phase 1: CLI Commands (no TUI)

These tests verify the command-line interface works without entering the interactive TUI.

### 1.1 Version and help

```bash
forge --version
```
- [ ] Prints `forge 0.1.0`

```bash
forge --help
```
- [ ] Shows usage with `model`, `config` subcommands and `--project` flag

```bash
forge model --help
```
- [ ] Shows `list`, `install`, `use`, `info` subcommands

---

### 1.2 Model management

```bash
forge model list
```
- [ ] Lists installed models (may be empty or show previously downloaded models)

```bash
forge model info
```
- [ ] Shows backend, context length, temperature, and model path

```bash
forge model use nonexistent-model-xyz
```
- [ ] Prints error: "Model 'nonexistent-model-xyz' not found."
- [ ] Exits with non-zero status

---

### 1.3 Configuration

```bash
forge config show
```
- [ ] Prints valid TOML with `[model]`, `[permissions]`, `[plugins]` sections
- [ ] Backend shows your configured backend (mlx or llamacpp)

```bash
forge config edit
```
- [ ] Opens `~/.ftai/config.toml` in your `$EDITOR` (or vim/notepad)
- [ ] After closing editor, returns to shell cleanly

---

### 1.4 Project flag

```bash
mkdir -p /tmp/forge-test-project
forge --project /tmp/forge-test-project model info
```
- [ ] Works without error (uses the project path for context)

```bash
rm -rf /tmp/forge-test-project
```

---

## Phase 2: TUI Startup and Splash

### 2.1 Launch in a git repo

```bash
cd ~/Developer/forge   # or any git repo
forge
```
- [ ] TUI appears with alternate screen (full terminal takeover)
- [ ] Top status bar shows: model name, backend, project path
- [ ] Splash screen shows the FORGE ASCII logo
- [ ] Bottom shows token count and mode
- [ ] Mode shows "coding" (because .git/ exists)

**Exit:** Press `Ctrl+D`
- [ ] Returns to normal terminal cleanly

---

### 2.2 Launch outside a git repo

```bash
mkdir -p /tmp/forge-chat-test
cd /tmp/forge-chat-test
forge
```
- [ ] TUI launches
- [ ] Mode shows "chat" (no .git/ or .ftai/ present)

**Exit:** Press `Ctrl+D`

```bash
rm -rf /tmp/forge-chat-test
```

---

### 2.3 Launch with no model configured

```bash
# Temporarily break the model path
forge config show | grep "^path"   # note current path
```

If you have a model configured, this tests the error-recovery flow when the backend fails to start:
- [ ] TUI still launches (doesn't crash)
- [ ] Shows "Backend error" or "Running in offline mode" message
- [ ] You can still type `/help` and see commands

---

## Phase 3: TUI Slash Commands

Launch Forge in a git repo (`cd ~/Developer/forge && forge`), then test each command.

### 3.1 Help

Type: `/help` then Enter
- [ ] Shows list of available commands

---

### 3.2 Hardware info

Type: `/hardware` then Enter
- [ ] Shows CPU architecture (e.g., AppleSilicon)
- [ ] Shows GPU type (e.g., Metal)
- [ ] Shows RAM in GB
- [ ] Shows recommended model

---

### 3.3 Skills

Type: `/skill` then Enter
- [ ] Lists 6+ skills: /commit, /review, /refactor, /debug, /test, /security
- [ ] Each shows trigger, description, and source [builtin]

Type: `/skill commit` then Enter
- [ ] Shows the commit skill content (git commit best practices)

Type: `/commit` then Enter (bare trigger)
- [ ] Shows "Loaded skill: Git commit best practices"

---

### 3.4 Mode switching

Type: `/chat` then Enter
- [ ] Shows "Switched to chat mode"
- [ ] Bottom status shows "chat"

Type: `/code` then Enter
- [ ] Shows "Switched to coding mode"
- [ ] Bottom status shows "coding"

---

### 3.5 Rules

Type: `/rules` then Enter
- [ ] Shows "No rules loaded" or lists active rules

---

### 3.6 Permissions

Type: `/permissions` then Enter
- [ ] Shows permission mode (e.g., "Auto")
- [ ] Shows "No active grants" or lists grants

---

### 3.7 Config

Type: `/config` then Enter
- [ ] Shows full TOML configuration inline

---

### 3.8 Model info

Type: `/model` then Enter
- [ ] Shows backend, model path, context length

---

### 3.9 Memory

Type: `/memory` then Enter
- [ ] Shows "No memory notes found" or existing notes

Type: `/memory this is a test note` then Enter
- [ ] Shows "Saved to memory: this is a test note"

Type: `/memory` then Enter
- [ ] Now shows the saved note

---

### 3.10 Context (FTAI.md)

Type: `/context` then Enter
- [ ] Shows FTAI.md content if it exists, or "No FTAI.md found"

---

### 3.11 Plugin management

Type: `/plugin list` then Enter
- [ ] Shows installed plugins or "No plugins installed"

---

### 3.12 Clear

Type: `/clear` then Enter
- [ ] Message area clears
- [ ] Shows "Conversation cleared. Permission grants cleared."

---

### 3.13 Project

Type: `/project` then Enter
- [ ] Shows current project path

---

## Phase 4: TUI Input and Navigation

### 4.1 Text input

- [ ] Typing characters appears in the input area
- [ ] Cursor is visible and positioned correctly
- [ ] Backspace deletes characters

---

### 4.2 Multi-line input

Type some text, then press `Shift+Enter`, then type more
- [ ] A newline is inserted (doesn't submit)

Press `Enter` to submit
- [ ] The full multi-line message is submitted

---

### 4.3 History navigation

Submit a message (e.g., "hello"), then submit another (e.g., "world")

Press `Up` arrow
- [ ] Previous message "world" appears in input

Press `Up` again
- [ ] "hello" appears

Press `Down`
- [ ] "world" appears again

---

### 4.4 Scrolling

If the message area has enough content:

Press `Shift+Up`
- [ ] Messages scroll up (older messages visible)

Press `Shift+Down`
- [ ] Messages scroll back down

Press `PageUp` / `PageDown`
- [ ] Scrolls in larger increments

---

### 4.5 Cancel and clear

Type something, then press `Esc`
- [ ] Input area clears

Type something, then press `Ctrl+C`
- [ ] Input area clears (if input is non-empty)

With empty input, press `Ctrl+C`
- [ ] Exits Forge

---

## Phase 5: Agentic Loop (requires working backend)

**Prerequisite:** You need a model loaded and the backend running. If the backend failed to start, skip this phase.

### 5.1 Simple conversation

Type: `What is 2 + 2?` then Enter
- [ ] Model streams a response (tokens appear progressively)
- [ ] Response contains "4"
- [ ] Token count updates in the status bar

---

### 5.2 Tool calling (Coding mode only)

Type: `List the files in the current directory` then Enter
- [ ] Model calls the `bash` or `glob` tool
- [ ] Tool call box appears with tool name and result
- [ ] File listing is visible in the result
- [ ] Model responds with a summary after seeing tool results

---

### 5.3 File operations

Type: `Read the first 10 lines of Cargo.toml` then Enter
- [ ] Model calls `file_read` tool
- [ ] Cargo.toml content visible in tool result
- [ ] Model summarizes what it found

---

### 5.4 Permission prompt

Type: `Create a file called /tmp/forge-test-output.txt with the content "hello from forge"` then Enter
- [ ] Depending on permission mode:
  - **Auto mode:** file_write executes automatically
  - **Ask mode:** Permission prompt appears, type `y` to approve
- [ ] File is created

Verify:
```bash
cat /tmp/forge-test-output.txt
```
- [ ] Contains "hello from forge"

Clean up:
```bash
rm /tmp/forge-test-output.txt
```

---

### 5.5 Multi-turn tool use

Type: `Create a file /tmp/forge-multi.txt with "step 1", then read it back to confirm` then Enter
- [ ] Model makes multiple tool calls (write then read)
- [ ] Each tool call shows in its own box
- [ ] Model confirms the content

Clean up:
```bash
rm /tmp/forge-multi.txt
```

---

### 5.6 Skill + conversation

Type: `/commit` then Enter (activates skill)
Then type: `Help me write a commit message for the changes in this repo` then Enter
- [ ] Model uses the commit skill context to give structured commit advice
- [ ] Response references commit best practices

---

### 5.7 Hard block

Type: `Run rm -rf /` then Enter
- [ ] **BLOCKED** message appears in red
- [ ] Tool is NOT executed
- [ ] Model receives "HARD BLOCKED" feedback

---

### 5.8 Context compaction

After several exchanges:

Type: `/compact` then Enter
- [ ] Shows "Context compacted. Tokens: ~N" with a reduced count

---

### 5.9 Cancel generation

Start a long request, then press `Ctrl+C` while tokens are streaming
- [ ] Generation stops
- [ ] Partial response is visible
- [ ] Input becomes active again

---

## Phase 6: Edge Cases

### 6.1 Rapid input during generation

While the model is generating, type characters
- [ ] Characters are ignored (input shows "generating...")
- [ ] No crash or garbled output

---

### 6.2 Empty submit

Press `Enter` with empty input
- [ ] Nothing happens (no message sent)

---

### 6.3 Very long input

Paste a large block of text (500+ characters) and submit
- [ ] Message is displayed correctly
- [ ] Model receives and responds to it

---

### 6.4 Terminal resize

While Forge is running, resize the terminal window
- [ ] TUI redraws correctly
- [ ] No crash or garbled layout

---

### 6.5 Unknown command

Type: `/foobar` then Enter
- [ ] Shows "Unknown command: /foobar"

---

### 6.6 File paths not confused with commands

Type: `/Users/michaelfolk/some/path` then Enter
- [ ] Treated as a normal message (not a slash command)
- [ ] Sent to the model as user input

---

## Results Summary

| Phase | Tests | Passed | Failed | Notes |
|-------|-------|--------|--------|-------|
| 1. CLI Commands | 9 | | | |
| 2. TUI Startup | 3 | | | |
| 3. Slash Commands | 13 | | | |
| 4. Input/Navigation | 5 | | | |
| 5. Agentic Loop | 9 | | | |
| 6. Edge Cases | 6 | | | |
| **Total** | **45** | | | |

---

## Automated tests

To run all 1,632 automated tests:

```bash
cargo test
```

To run specific test modules:

```bash
cargo test --test integration cli_commands     # CLI tests
cargo test --test integration tool_execution   # Tool E2E tests
cargo test --test integration permission_pipeline  # Permission tests
cargo test --lib skills                        # Skills unit tests
cargo test --lib rules                         # Rules unit tests
cargo test --lib conversation                  # Conversation tests
cargo test --lib "security"                    # All security red tests
```
