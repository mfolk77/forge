use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::plugins::catalog::{self, CatalogEntry};
use super::modal::{Modal, ModalAction};
use super::render::Theme;

/// Which tab is active in the plugin browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTab {
    Discover,
    Installed,
    Marketplaces,
}

/// An entry in the "Installed" tab.
#[derive(Debug, Clone)]
pub struct InstalledPluginEntry {
    pub name: String,
    pub source: String,
    pub plugin_type: String,
    pub enabled: bool,
    pub description: String,
}

/// A marketplace entry displayed in the Marketplaces tab.
#[derive(Debug, Clone)]
pub struct MarketplaceEntry {
    pub name: String,
    pub repo: String,
    pub available_count: usize,
    pub installed_count: usize,
    pub last_updated: String,
    pub is_default: bool,
}

/// Detail view state when the user presses Enter on an item.
#[derive(Debug, Clone)]
pub struct DetailView {
    pub name: String,
    pub source: String,
    pub description: String,
    pub category: String,
    pub is_installed: bool,
    pub is_enabled: bool,
}

/// Interactive plugin browser modal with Discover, Installed, and Marketplaces tabs.
pub struct PluginModal {
    pub active_tab: PluginTab,
    pub discover_list: Vec<CatalogEntry>,
    pub installed_list: Vec<InstalledPluginEntry>,
    pub marketplace_list: Vec<MarketplaceEntry>,
    pub selected_index: usize,
    pub search_query: String,
    pub search_active: bool,
    pub detail_view: Option<DetailView>,
    pub scroll_offset: usize,
}

impl PluginModal {
    /// Create a new plugin modal with catalog entries and installed plugins.
    pub fn new(installed: Vec<InstalledPluginEntry>) -> Self {
        Self {
            active_tab: PluginTab::Discover,
            discover_list: catalog::catalog(),
            installed_list: installed,
            marketplace_list: Vec::new(),
            selected_index: 0,
            search_query: String::new(),
            search_active: false,
            detail_view: None,
            scroll_offset: 0,
        }
    }

    /// Create a new plugin modal with marketplace data.
    pub fn with_marketplaces(installed: Vec<InstalledPluginEntry>, marketplaces: Vec<MarketplaceEntry>) -> Self {
        Self {
            active_tab: PluginTab::Discover,
            discover_list: catalog::catalog(),
            installed_list: installed,
            marketplace_list: marketplaces,
            selected_index: 0,
            search_query: String::new(),
            search_active: false,
            detail_view: None,
            scroll_offset: 0,
        }
    }

    /// Get the filtered discover list based on search query.
    fn filtered_discover(&self) -> Vec<&CatalogEntry> {
        if self.search_query.is_empty() {
            self.discover_list.iter().collect()
        } else {
            let q = self.search_query.to_lowercase();
            self.discover_list
                .iter()
                .filter(|e| {
                    e.name.to_lowercase().contains(&q)
                        || e.description.to_lowercase().contains(&q)
                })
                .collect()
        }
    }

    /// Get the filtered installed list based on search query.
    fn filtered_installed(&self) -> Vec<&InstalledPluginEntry> {
        if self.search_query.is_empty() {
            self.installed_list.iter().collect()
        } else {
            let q = self.search_query.to_lowercase();
            self.installed_list
                .iter()
                .filter(|e| {
                    e.name.to_lowercase().contains(&q)
                        || e.description.to_lowercase().contains(&q)
                })
                .collect()
        }
    }

    /// Total number of visible items in the current tab.
    fn visible_count(&self) -> usize {
        match self.active_tab {
            PluginTab::Discover => self.filtered_discover().len(),
            PluginTab::Installed => self.filtered_installed().len(),
            PluginTab::Marketplaces => self.marketplace_list.len() + 1, // +1 for "Add Marketplace" row
        }
    }

    /// Clamp selected_index to be within bounds.
    fn clamp_selection(&mut self) {
        let count = self.visible_count();
        if count == 0 {
            self.selected_index = 0;
        } else if self.selected_index >= count {
            self.selected_index = count - 1;
        }
    }

    fn switch_tab(&mut self) {
        self.active_tab = match self.active_tab {
            PluginTab::Discover => PluginTab::Installed,
            PluginTab::Installed => PluginTab::Marketplaces,
            PluginTab::Marketplaces => PluginTab::Discover,
        };
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            if self.selected_index < self.scroll_offset {
                self.scroll_offset = self.selected_index;
            }
        }
    }

    fn move_down(&mut self) {
        let count = self.visible_count();
        if count > 0 && self.selected_index < count - 1 {
            self.selected_index += 1;
            // Scroll follows selection — assume ~20 visible rows as safe default.
            // The actual content_height is computed in render(), but we keep
            // scroll_offset roughly in sync here so the next render picks it up.
            let visible_rows = 18_usize; // conservative; render will clamp
            if self.selected_index >= self.scroll_offset + visible_rows {
                self.scroll_offset = self.selected_index + 1 - visible_rows;
            }
        }
    }

    fn open_detail(&mut self) {
        match self.active_tab {
            PluginTab::Discover => {
                let filtered = self.filtered_discover();
                if let Some(entry) = filtered.get(self.selected_index) {
                    let installed = self
                        .installed_list
                        .iter()
                        .any(|i| i.name == entry.name);
                    self.detail_view = Some(DetailView {
                        name: entry.name.clone(),
                        source: entry.repo.clone(),
                        description: entry.description.clone(),
                        category: entry.category.clone(),
                        is_installed: installed,
                        is_enabled: true,
                    });
                }
            }
            PluginTab::Installed => {
                let filtered = self.filtered_installed();
                if let Some(entry) = filtered.get(self.selected_index) {
                    self.detail_view = Some(DetailView {
                        name: entry.name.clone(),
                        source: entry.source.clone(),
                        description: entry.description.clone(),
                        category: entry.plugin_type.clone(),
                        is_installed: true,
                        is_enabled: entry.enabled,
                    });
                }
            }
            PluginTab::Marketplaces => {
                // No detail view for marketplaces — handled via u/r keys
            }
        }
    }
}

impl Modal for PluginModal {
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        // Clear the area
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_style(Style::default());
                }
            }
        }

        if area.height < 6 || area.width < 30 {
            return;
        }

        // Layout: tabs (1) + search (1) + content (rest - 1 for footer)
        let content_start = area.y + 2;
        let footer_y = area.y + area.height - 1;
        let content_height = footer_y.saturating_sub(content_start) as usize;

        // ── Tab bar ────────────────────────────────────────────────────
        let tab_style = |tab: PluginTab| -> Style {
            if self.active_tab == tab {
                Style::default().fg(theme.accent).bold()
            } else {
                Style::default().fg(theme.dim)
            }
        };
        let sep = Span::styled("  |  ", Style::default().fg(theme.dim));

        let tab_line = Line::from(vec![
            Span::styled("  Discover", tab_style(PluginTab::Discover)),
            sep.clone(),
            Span::styled("Installed", tab_style(PluginTab::Installed)),
            sep,
            Span::styled("Marketplaces", tab_style(PluginTab::Marketplaces)),
        ]);
        buf.set_line(area.x, area.y, &tab_line, area.width);

        // ── Search bar ─────────────────────────────────────────────────
        let search_text = if self.search_active || !self.search_query.is_empty() {
            format!("  / {}", self.search_query)
        } else {
            "  / type to search...".to_string()
        };
        let search_style = if self.search_active {
            Style::default().fg(theme.user_input)
        } else {
            Style::default().fg(theme.dim).italic()
        };
        let search_line = Line::from(Span::styled(search_text, search_style));
        buf.set_line(area.x, area.y + 1, &search_line, area.width);

        // ── Detail view ────────────────────────────────────────────────
        if let Some(detail) = &self.detail_view {
            render_detail_view(detail, theme, area, content_start, content_height, buf);
            // Footer for detail view
            let footer = Line::from(Span::styled(
                "  Enter: install/uninstall · Space: toggle · Esc: back",
                Style::default().fg(theme.dim),
            ));
            buf.set_line(area.x, footer_y, &footer, area.width);
            return;
        }

        // ── List view ──────────────────────────────────────────────────
        match self.active_tab {
            PluginTab::Discover => {
                let filtered = self.filtered_discover();
                let total = filtered.len();

                if total == 0 {
                    let empty = Line::from(Span::styled(
                        "  No plugins found.",
                        Style::default().fg(theme.dim),
                    ));
                    buf.set_line(area.x, content_start, &empty, area.width);
                } else {
                    let start = self.scroll_offset.min(total.saturating_sub(1));
                    let end = (start + content_height).min(total);

                    for (i, idx) in (start..end).enumerate() {
                        let entry = filtered[idx];
                        let y = content_start + i as u16;
                        let is_selected = idx == self.selected_index;

                        let style = if is_selected {
                            Style::default().fg(Color::Black).bg(theme.accent)
                        } else {
                            Style::default().fg(theme.assistant_text)
                        };

                        let cat_style = if is_selected {
                            Style::default().fg(Color::Black).bg(theme.accent)
                        } else {
                            Style::default().fg(theme.dim)
                        };

                        let name_width = 20.min(area.width as usize / 3);
                        let cat_width = 16.min(area.width as usize / 5);
                        let desc_width = (area.width as usize)
                            .saturating_sub(name_width + cat_width + 6);

                        let name_str = format!("  {:<width$}", entry.name, width = name_width);
                        let desc_str: String = entry
                            .description
                            .chars()
                            .take(desc_width)
                            .collect();
                        let desc_padded = format!("{:<width$}", desc_str, width = desc_width);
                        let cat_str = format!("[{}]", entry.category);

                        let line = Line::from(vec![
                            Span::styled(name_str, style),
                            Span::styled(desc_padded, style),
                            Span::styled(format!("  {cat_str}"), cat_style),
                        ]);
                        buf.set_line(area.x, y, &line, area.width);
                    }
                }
            }
            PluginTab::Installed => {
                let filtered = self.filtered_installed();
                let total = filtered.len();

                if total == 0 {
                    let empty = Line::from(Span::styled(
                        "  No plugins installed.",
                        Style::default().fg(theme.dim),
                    ));
                    buf.set_line(area.x, content_start, &empty, area.width);
                } else {
                    let start = self.scroll_offset.min(total.saturating_sub(1));
                    let end = (start + content_height).min(total);

                    for (i, idx) in (start..end).enumerate() {
                        let entry = filtered[idx];
                        let y = content_start + i as u16;
                        let is_selected = idx == self.selected_index;

                        let style = if is_selected {
                            Style::default().fg(Color::Black).bg(theme.accent)
                        } else {
                            Style::default().fg(theme.assistant_text)
                        };

                        let status = if entry.enabled {
                            "\u{2713} enabled"
                        } else {
                            "\u{25CB} disabled"
                        };

                        let text = format!(
                            "  {}  {} \u{00B7} {} \u{00B7} {}",
                            entry.name, entry.plugin_type, entry.source, status
                        );
                        let line = Line::from(Span::styled(text, style));
                        buf.set_line(area.x, y, &line, area.width);
                    }
                }
            }
            PluginTab::Marketplaces => {
                // Header
                let header = Line::from(Span::styled(
                    "  Manage marketplaces",
                    Style::default().fg(theme.assistant_text).bold(),
                ));
                buf.set_line(area.x, content_start, &header, area.width);

                let mut y = content_start + 1;

                // "+ Add Marketplace" row (index 0)
                let add_selected = self.selected_index == 0;
                let add_style = if add_selected {
                    Style::default().fg(Color::Black).bg(theme.accent)
                } else {
                    Style::default().fg(theme.accent)
                };
                let add_line = Line::from(Span::styled(
                    "  \u{203A} + Add Marketplace",
                    add_style,
                ));
                buf.set_line(area.x, y, &add_line, area.width);
                y += 1;

                // Marketplace entries
                for (i, mp) in self.marketplace_list.iter().enumerate() {
                    if y + 3 >= footer_y {
                        break;
                    }

                    let entry_index = i + 1; // offset by 1 for "Add" row
                    let is_selected = self.selected_index == entry_index;
                    y += 1; // blank line separator

                    // Bullet + name
                    let bullet = if mp.is_default { "  \u{2731} " } else { "  \u{2022} " };
                    let name_style = if is_selected {
                        Style::default().fg(Color::Black).bg(theme.accent).bold()
                    } else {
                        Style::default().fg(theme.assistant_text).bold()
                    };
                    let suffix = if mp.is_default { " \u{2731}" } else { "" };
                    let name_line = Line::from(Span::styled(
                        format!("{}{}{}", bullet, mp.name, suffix),
                        name_style,
                    ));
                    buf.set_line(area.x, y, &name_line, area.width);
                    y += 1;

                    // Repo + stats
                    let info_style = if is_selected {
                        Style::default().fg(Color::Black).bg(theme.accent)
                    } else {
                        Style::default().fg(theme.dim)
                    };
                    let info_text = format!(
                        "    {} \u{00B7} {} available \u{00B7} {} installed \u{00B7} Updated {}",
                        mp.repo, mp.available_count, mp.installed_count, mp.last_updated
                    );
                    let info_line = Line::from(Span::styled(info_text, info_style));
                    buf.set_line(area.x, y, &info_line, area.width);
                    y += 1;
                }
            }
        }

        // ── Footer ─────────────────────────────────────────────────────
        let footer_text = match self.active_tab {
            PluginTab::Marketplaces =>
                "  Tab: switch \u{00B7} \u{2191}\u{2193}: navigate \u{00B7} Enter: select \u{00B7} u: update \u{00B7} r: remove \u{00B7} Esc: back",
            _ =>
                "  Tab: switch \u{00B7} \u{2191}\u{2193}: navigate \u{00B7} Enter: details \u{00B7} Space: toggle \u{00B7} /: search \u{00B7} Esc: back",
        };
        let footer = Line::from(Span::styled(
            footer_text,
            Style::default().fg(theme.dim),
        ));
        buf.set_line(area.x, footer_y, &footer, area.width);
    }

    fn handle_key(&mut self, key: KeyEvent) -> ModalAction {
        // If in search mode, capture typed characters
        if self.search_active {
            match key.code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                    self.selected_index = 0;
                    self.scroll_offset = 0;
                    return ModalAction::Continue;
                }
                KeyCode::Backspace => {
                    if self.search_query.is_empty() {
                        self.search_active = false;
                    } else {
                        self.search_query.pop();
                        self.clamp_selection();
                    }
                    return ModalAction::Continue;
                }
                KeyCode::Enter => {
                    self.search_active = false;
                    return ModalAction::Continue;
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.clamp_selection();
                    return ModalAction::Continue;
                }
                _ => return ModalAction::Continue,
            }
        }

        // If detail view is showing
        if let Some(detail) = &self.detail_view {
            match key.code {
                KeyCode::Esc => {
                    self.detail_view = None;
                    return ModalAction::Continue;
                }
                KeyCode::Enter => {
                    let name = detail.name.clone();
                    if detail.is_installed {
                        self.detail_view = None;
                        return ModalAction::UninstallPlugin(name);
                    } else {
                        self.detail_view = None;
                        return ModalAction::InstallPlugin(name);
                    }
                }
                KeyCode::Char(' ') => {
                    if detail.is_installed {
                        let name = detail.name.clone();
                        self.detail_view = None;
                        return ModalAction::TogglePlugin(name);
                    }
                    return ModalAction::Continue;
                }
                _ => return ModalAction::Continue,
            }
        }

        // Normal list mode
        match key.code {
            KeyCode::Esc => ModalAction::Close,
            KeyCode::Tab | KeyCode::BackTab => {
                self.switch_tab();
                ModalAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                ModalAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                ModalAction::Continue
            }
            KeyCode::Enter => {
                if self.active_tab == PluginTab::Marketplaces {
                    if self.selected_index == 0 {
                        return ModalAction::AddMarketplace;
                    }
                    // Enter on a marketplace entry does nothing (use u/r)
                    return ModalAction::Continue;
                }
                self.open_detail();
                ModalAction::Continue
            }
            KeyCode::Char('u') if self.active_tab == PluginTab::Marketplaces => {
                let mp_idx = self.selected_index.saturating_sub(1);
                if let Some(mp) = self.marketplace_list.get(mp_idx) {
                    return ModalAction::UpdateMarketplace(mp.name.clone());
                }
                ModalAction::Continue
            }
            KeyCode::Char('r') if self.active_tab == PluginTab::Marketplaces => {
                let mp_idx = self.selected_index.saturating_sub(1);
                if let Some(mp) = self.marketplace_list.get(mp_idx) {
                    return ModalAction::RemoveMarketplace(mp.name.clone());
                }
                ModalAction::Continue
            }
            KeyCode::Char(' ') => {
                if self.active_tab == PluginTab::Installed {
                    let filtered = self.filtered_installed();
                    if let Some(entry) = filtered.get(self.selected_index) {
                        return ModalAction::TogglePlugin(entry.name.clone());
                    }
                }
                ModalAction::Continue
            }
            KeyCode::Char('/') => {
                self.search_active = true;
                self.search_query.clear();
                ModalAction::Continue
            }
            _ => ModalAction::Continue,
        }
    }

    fn input_hint(&self) -> &str {
        if self.search_active {
            "Type to filter, Enter to confirm, Esc to cancel"
        } else if self.detail_view.is_some() {
            "Enter: install/uninstall, Space: toggle, Esc: back"
        } else if self.active_tab == PluginTab::Marketplaces {
            "Enter: select, u: update, r: remove, Tab: switch, Esc: close"
        } else {
            "Tab: switch, Up/Down: navigate, Enter: details, /: search, Esc: close"
        }
    }
}

fn render_detail_view(
    detail: &DetailView,
    theme: &Theme,
    area: Rect,
    content_start: u16,
    _content_height: usize,
    buf: &mut Buffer,
) {
    let mut y = content_start;

    // Name header
    let name_line = Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(&detail.name, Style::default().fg(theme.accent).bold()),
    ]);
    buf.set_line(area.x, y, &name_line, area.width);
    y += 1;

    // Category / Type
    let cat_line = Line::from(vec![
        Span::styled("  Category: ", Style::default().fg(theme.dim)),
        Span::styled(&detail.category, Style::default().fg(theme.assistant_text)),
    ]);
    buf.set_line(area.x, y, &cat_line, area.width);
    y += 1;

    // Source / repo
    let source_line = Line::from(vec![
        Span::styled("  Source: ", Style::default().fg(theme.dim)),
        Span::styled(&detail.source, Style::default().fg(theme.assistant_text)),
    ]);
    buf.set_line(area.x, y, &source_line, area.width);
    y += 2;

    // Description
    let desc_line = Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(&detail.description, Style::default().fg(theme.assistant_text)),
    ]);
    buf.set_line(area.x, y, &desc_line, area.width);
    y += 2;

    // Action hints
    if detail.is_installed {
        let status = if detail.is_enabled {
            "enabled"
        } else {
            "disabled"
        };
        let status_line = Line::from(vec![
            Span::styled("  Status: ", Style::default().fg(theme.dim)),
            Span::styled(
                status,
                Style::default().fg(if detail.is_enabled {
                    Color::Green
                } else {
                    theme.warning
                }),
            ),
        ]);
        buf.set_line(area.x, y, &status_line, area.width);
        y += 1;

        let action_line = Line::from(Span::styled(
            "  [Enter] Uninstall  [Space] Toggle",
            Style::default().fg(theme.accent),
        ));
        buf.set_line(area.x, y, &action_line, area.width);
    } else {
        let action_line = Line::from(Span::styled(
            "  [Enter] Install",
            Style::default().fg(theme.accent),
        ));
        buf.set_line(area.x, y, &action_line, area.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn make_modal() -> PluginModal {
        let installed = vec![
            InstalledPluginEntry {
                name: "test-plugin".into(),
                source: "local".into(),
                plugin_type: "Plugin".into(),
                enabled: true,
                description: "A test plugin".into(),
            },
        ];
        PluginModal::new(installed)
    }

    #[test]
    fn test_new_modal_starts_on_discover() {
        let modal = make_modal();
        assert_eq!(modal.active_tab, PluginTab::Discover);
        assert_eq!(modal.selected_index, 0);
        assert!(modal.detail_view.is_none());
        assert!(!modal.search_active);
    }

    #[test]
    fn test_tab_cycles_through_all_tabs() {
        let mut modal = make_modal();
        assert_eq!(modal.active_tab, PluginTab::Discover);
        modal.handle_key(key(KeyCode::Tab));
        assert_eq!(modal.active_tab, PluginTab::Installed);
        modal.handle_key(key(KeyCode::Tab));
        assert_eq!(modal.active_tab, PluginTab::Marketplaces);
        modal.handle_key(key(KeyCode::Tab));
        assert_eq!(modal.active_tab, PluginTab::Discover);
    }

    #[test]
    fn test_up_down_navigation() {
        let mut modal = make_modal();
        assert_eq!(modal.selected_index, 0);

        // Move down
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.selected_index, 1);

        // Move down again
        modal.handle_key(key(KeyCode::Down));
        assert_eq!(modal.selected_index, 2);

        // Move up
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.selected_index, 1);

        // Move up again
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.selected_index, 0);

        // Moving up at 0 stays at 0
        modal.handle_key(key(KeyCode::Up));
        assert_eq!(modal.selected_index, 0);
    }

    #[test]
    fn test_j_k_navigation() {
        let mut modal = make_modal();
        modal.handle_key(key(KeyCode::Char('j')));
        assert_eq!(modal.selected_index, 1);
        modal.handle_key(key(KeyCode::Char('k')));
        assert_eq!(modal.selected_index, 0);
    }

    #[test]
    fn test_search_filtering() {
        let mut modal = make_modal();

        // Activate search
        modal.handle_key(key(KeyCode::Char('/')));
        assert!(modal.search_active);

        // Type "docker"
        for c in "docker".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(modal.search_query, "docker");

        let filtered = modal.filtered_discover();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "docker-tools");
    }

    #[test]
    fn test_search_backspace() {
        let mut modal = make_modal();
        modal.handle_key(key(KeyCode::Char('/')));
        modal.handle_key(key(KeyCode::Char('a')));
        modal.handle_key(key(KeyCode::Char('b')));
        assert_eq!(modal.search_query, "ab");

        modal.handle_key(key(KeyCode::Backspace));
        assert_eq!(modal.search_query, "a");

        modal.handle_key(key(KeyCode::Backspace));
        assert_eq!(modal.search_query, "");

        // Backspace on empty deactivates search
        modal.handle_key(key(KeyCode::Backspace));
        assert!(!modal.search_active);
    }

    #[test]
    fn test_esc_layering_search_then_close() {
        let mut modal = make_modal();

        // Enter search
        modal.handle_key(key(KeyCode::Char('/')));
        assert!(modal.search_active);

        // Esc closes search first
        let action = modal.handle_key(key(KeyCode::Esc));
        assert!(!modal.search_active);
        assert!(matches!(action, ModalAction::Continue));

        // Esc again closes modal
        let action = modal.handle_key(key(KeyCode::Esc));
        assert!(matches!(action, ModalAction::Close));
    }

    #[test]
    fn test_esc_layering_detail_then_close() {
        let mut modal = make_modal();

        // Open detail
        modal.handle_key(key(KeyCode::Enter));
        assert!(modal.detail_view.is_some());

        // Esc closes detail first
        let action = modal.handle_key(key(KeyCode::Esc));
        assert!(modal.detail_view.is_none());
        assert!(matches!(action, ModalAction::Continue));

        // Esc closes modal
        let action = modal.handle_key(key(KeyCode::Esc));
        assert!(matches!(action, ModalAction::Close));
    }

    #[test]
    fn test_enter_opens_detail_view() {
        let mut modal = make_modal();
        modal.handle_key(key(KeyCode::Enter));
        assert!(modal.detail_view.is_some());
        let detail = modal.detail_view.as_ref().unwrap();
        assert_eq!(detail.name, "commit-helper");
    }

    #[test]
    fn test_install_from_detail() {
        let mut modal = make_modal();
        // Select an entry not in installed
        modal.handle_key(key(KeyCode::Enter));
        let action = modal.handle_key(key(KeyCode::Enter));
        // commit-helper is not in installed_list, so it should trigger install
        assert!(matches!(action, ModalAction::InstallPlugin(ref name) if name == "commit-helper"));
    }

    #[test]
    fn test_space_toggle_on_installed_tab() {
        let mut modal = make_modal();
        modal.handle_key(key(KeyCode::Tab)); // Switch to installed
        let action = modal.handle_key(key(KeyCode::Char(' ')));
        assert!(matches!(action, ModalAction::TogglePlugin(ref name) if name == "test-plugin"));
    }

    #[test]
    fn test_render_no_panic_discover() {
        let modal = make_modal();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        modal.render(&theme, area, &mut buf);
    }

    #[test]
    fn test_render_no_panic_installed() {
        let mut modal = make_modal();
        modal.active_tab = PluginTab::Installed;
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        modal.render(&theme, area, &mut buf);
    }

    #[test]
    fn test_render_no_panic_detail() {
        let mut modal = make_modal();
        modal.handle_key(key(KeyCode::Enter));
        assert!(modal.detail_view.is_some());
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        modal.render(&theme, area, &mut buf);
    }

    #[test]
    fn test_render_no_panic_tiny_area() {
        let modal = make_modal();
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        modal.render(&theme, area, &mut buf);
    }

    #[test]
    fn test_render_no_panic_empty_installed() {
        let modal = PluginModal::new(vec![]);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        // Render both tabs
        modal.render(&theme, area, &mut buf);
    }

    #[test]
    fn test_search_case_insensitive() {
        let mut modal = make_modal();
        modal.handle_key(key(KeyCode::Char('/')));
        for c in "DOCKER".chars() {
            modal.handle_key(key(KeyCode::Char(c)));
        }
        let filtered = modal.filtered_discover();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "docker-tools");
    }

    #[test]
    fn test_down_does_not_exceed_bounds() {
        let mut modal = make_modal();
        let count = modal.visible_count();
        for _ in 0..count + 5 {
            modal.handle_key(key(KeyCode::Down));
        }
        assert_eq!(modal.selected_index, count - 1);
    }
}
