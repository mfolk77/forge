# Interactive Plugin & Skill Modal System

**Date:** 2026-03-30
**Status:** Design

## Overview

Add an interactive modal overlay system to Forge's TUI, starting with two modals:
1. **`/plugin`** — tabbed plugin browser (Discover, Installed) with search, toggle, install, detail views
2. **`/skill`** — scrollable skill list (User skills, Plugin skills, Builtin) with token counts

Modeled after Claude Code's plugin/skill UI.

## Architecture

### Modal System (new: `src/tui/modal.rs`)

A `Modal` trait with `render()` and `handle_key()` methods. TuiApp gains an `active_modal: Option<Box<dyn Modal>>` field. When set:
- `render()` draws the modal instead of messages
- `handle_key()` routes to the modal's handler
- `Esc` closes the modal
- Input area shows the modal's hint text (e.g., "type to search")

```rust
pub trait Modal {
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer);
    fn handle_key(&mut self, key: KeyEvent) -> ModalAction;
    fn input_hint(&self) -> &str;
}

pub enum ModalAction {
    Continue,           // stay in modal
    Close,              // close modal, return to chat
    InstallPlugin(String),  // trigger plugin install
    UninstallPlugin(String),
    TogglePlugin(String),
    CreatePlugin(String),
}
```

### Plugin Modal (new: `src/tui/plugin_modal.rs`)

**Tabs:** `Discover` | `Installed`

**Discover tab:**
- Scrollable list from built-in registry catalog
- Each entry: `name  description  [category]`
- Enter on an entry → detail view with description + "Install" action
- Search filters the list by name/description

**Installed tab:**
- Shows installed plugins grouped by: Project, User, Builtin
- Each entry: `name  Plugin/MCP · source · ✓ enabled / ○ disabled`
- Space toggles enabled/disabled
- Enter → detail view with description + Uninstall option
- Search filters

**Detail view (sub-state within modal):**
- Plugin name, source, description
- Tools/skills/hooks counts
- Actions: Install (discover) or Uninstall + Toggle (installed)
- Esc → back to list

**Keyboard:**
- `Tab` / `Shift+Tab` — switch tabs
- `Up/Down` or `j/k` — navigate list
- `Enter` — open detail / confirm action
- `Space` — toggle enable/disable (installed tab)
- `/` or typing — search filter
- `Esc` — close detail or close modal

### Skill Modal (new: `src/tui/skill_modal.rs`)

Single scrollable list, grouped:
- **Builtin skills** (6 shipped skills)
- **Plugin skills** (from installed plugins)

Each entry: `name · source · ~N description tokens`

**Keyboard:**
- `Up/Down` — navigate
- `Enter` — activate skill (inject into context, close modal)
- `Esc` — close

### Built-in Plugin Registry (new: `src/plugins/catalog.rs`)

Compiled-in Vec of registry entries:

```rust
pub struct CatalogEntry {
    pub name: &'static str,
    pub description: &'static str,
    pub category: CatalogCategory,
    pub author: &'static str,
    pub repo: &'static str,
}

pub enum CatalogCategory {
    FolkTechCore,
    Workflow,
    Utility,
    Integration,
}
```

**Initial catalog (~12 plugins):**

FolkTech Core:
- `commit-helper` — Guided commit workflow with staged diff review
- `pr-review` — Automated code review pipeline
- `test-runner` — Run tests and format results
- `deploy-checklist` — Pre-deployment verification checklists

Workflow:
- `memory` — Persistent cross-session memory system
- `web-search` — Web search and page fetching
- `notebook` — Jupyter notebook read/edit support
- `mcp-bridge` — Connect to MCP servers

Utility:
- `docker-tools` — Dockerfile and compose generation
- `git-workflow` — Branch conventions and PR templates
- `python-tools` — Python linting and formatting helpers
- `rust-tools` — Cargo commands and clippy integration

### Plugin Create Command

`/plugin create <name>` scaffolds a new plugin:

```
~/.ftai/plugins/<name>/
  plugin.toml        # manifest with name, version, description
  tools/             # empty, ready for tool scripts
  skills/            # empty, ready for skill markdown
  hooks/             # empty, ready for hook scripts
  README.md          # basic readme
```

Opens the manifest in `$EDITOR` after scaffolding.

### Slash Command Autocomplete

When the user types `/` in the input:
- A dropdown/popup appears above the input showing matching commands
- Typing more characters filters the list
- `Up/Down` navigates, `Enter` selects, `Esc` dismisses
- Each entry: `/command-name     Description text...`

This requires a new `Autocomplete` widget state in TuiApp, rendered as an overlay between the message area and input.

## Files to Create/Modify

**New:**
- `src/tui/modal.rs` — Modal trait and ModalAction enum
- `src/tui/plugin_modal.rs` — Plugin browser modal
- `src/tui/skill_modal.rs` — Skill list modal
- `src/tui/autocomplete.rs` — Slash command autocomplete dropdown
- `src/plugins/catalog.rs` — Built-in plugin registry

**Modify:**
- `src/tui/mod.rs` — export new modules
- `src/tui/app.rs` — add `active_modal` field, route keys/render through modal, add autocomplete state, add `/plugin create` handler
- `src/tui/render.rs` — no changes (modal renders itself)
- `src/plugins/mod.rs` — export catalog
- `src/plugins/manager.rs` — add `toggle_plugin()` and `create_plugin()` methods

## Non-Goals (for this iteration)

- Marketplace management (add/remove registries) — future
- Plugin auto-update — future
- MCP server status indicators — future
- Remote registry fetching — future (currently all compiled-in)

## Testing

- Unit tests for catalog entries (non-empty, valid names)
- Unit tests for modal key handling (tab switch, navigation, search filter)
- Unit tests for plugin scaffold generation
- P0 security: plugin create name validation, catalog repo URL validation
