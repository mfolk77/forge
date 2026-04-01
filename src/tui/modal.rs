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
    CreatePlugin(String),
    /// Inject skill content into the conversation context.
    ActivateSkill { name: String, content: String },
    /// Switch the color theme.
    SelectTheme(String),
}

/// Trait implemented by all modal overlays (plugin browser, skill browser, etc.).
pub trait Modal {
    /// Render the modal into the given area.
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer);
    /// Handle a key event, returning an action for the app to process.
    fn handle_key(&mut self, key: KeyEvent) -> ModalAction;
    /// Hint text shown in the status line while this modal is active.
    fn input_hint(&self) -> &str;
}
