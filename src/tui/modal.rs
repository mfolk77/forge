use crossterm::event::KeyEvent;
use ratatui::prelude::*;

use super::render::Theme;

/// Actions that a modal can request from the application.
#[derive(Debug, Clone)]
pub enum ModalAction {
    /// Keep the modal open, no side effects.
    Continue,
    /// Close the modal.
    Close,
    /// Install a plugin by name (looked up in catalog for repo URL).
    InstallPlugin(String),
    /// Uninstall a plugin by name.
    UninstallPlugin(String),
    /// Toggle a plugin's enabled/disabled state.
    TogglePlugin(String),
    /// Scaffold a new plugin directory.
    #[allow(dead_code)]
    CreatePlugin(String),
    /// Inject skill content into the conversation context.
    ActivateSkill { name: String, content: String },
    /// Switch the color theme.
    SelectTheme(String),
    /// Add a new marketplace source (prompts user for repo).
    AddMarketplace,
    /// Update a marketplace by name (git pull).
    UpdateMarketplace(String),
    /// Remove a marketplace by name.
    RemoveMarketplace(String),
}

/// Trait implemented by all modal overlays (plugin browser, skill browser, etc.).
pub trait Modal {
    /// Render the modal into the given area.
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer);
    /// Handle a key event, returning an action for the app to process.
    fn handle_key(&mut self, key: KeyEvent) -> ModalAction;
    /// Hint text shown in the status line while this modal is active.
    #[allow(dead_code)]
    fn input_hint(&self) -> &str;
}
