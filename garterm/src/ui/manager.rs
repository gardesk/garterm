//! Tab manager - orchestrates multiple tabs and their panes

use super::pane::PaneId;
use super::split::Direction;
use super::tab::{Tab, TabId};
use super::tabbar::{TabBar, TabBarRenderData};
use crate::config::TabBarConfig;
use anyhow::Result;
use std::collections::HashMap;

/// Manages all tabs and their panes
pub struct TabManager {
    /// All tabs
    tabs: HashMap<TabId, Tab>,
    /// Order of tabs (for tab bar display)
    tab_order: Vec<TabId>,
    /// Currently active tab
    active_tab: TabId,
    /// Tab bar state
    pub tab_bar: TabBar,
    /// Next tab ID to assign
    next_tab_id: u32,
    /// Default shell command
    shell: String,
    /// Cell dimensions for layout calculations
    cell_width: f32,
    cell_height: f32,
}

impl TabManager {
    /// Create a new tab manager with an initial tab
    ///
    /// Note: `rows` is ignored and recalculated from content height (minus tab bar)
    pub fn new(
        shell: &str,
        cols: usize,
        _rows: usize,
        width: u32,
        height: u32,
        cell_width: f32,
        cell_height: f32,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self> {
        let tab_bar = TabBar::new();
        // Initial tab: only 1 tab, so full height available (no tab bar)
        let content_height = tab_bar.content_height(height, 1);
        let content_rows = (content_height as f32 / cell_height) as usize;

        let tab_id = TabId(0);
        let tab = Tab::new(tab_id, shell, cols.max(1), content_rows.max(1), width, content_height, cwd)?;

        let mut tabs = HashMap::new();
        tabs.insert(tab_id, tab);

        Ok(Self {
            tabs,
            tab_order: vec![tab_id],
            active_tab: tab_id,
            tab_bar,
            next_tab_id: 1,
            shell: shell.to_string(),
            cell_width,
            cell_height,
        })
    }

    /// Apply tab bar configuration
    pub fn set_tab_bar_config(&mut self, config: &TabBarConfig) {
        self.tab_bar = TabBar::from_config(config);
    }

    /// Get the active tab
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(&self.active_tab)
    }

    /// Get the active tab mutably
    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(&self.active_tab)
    }

    /// Get the focused pane in the active tab
    pub fn focused_pane(&self) -> Option<&super::pane::Pane> {
        self.active_tab().and_then(|t| t.focused_pane())
    }

    /// Get the focused pane mutably
    pub fn focused_pane_mut(&mut self) -> Option<&mut super::pane::Pane> {
        self.active_tab_mut().and_then(|t| t.focused_pane_mut())
    }

    /// Create a new tab
    pub fn new_tab(
        &mut self,
        width: u32,
        height: u32,
        cwd: Option<&std::path::Path>,
    ) -> Result<TabId> {
        self.new_tab_with_command(width, height, cwd, None)
    }

    /// Create a new tab with an optional startup command
    pub fn new_tab_with_command(
        &mut self,
        width: u32,
        height: u32,
        cwd: Option<&std::path::Path>,
        startup_cmd: Option<&str>,
    ) -> Result<TabId> {
        // After adding this tab, we'll have tabs.len() + 1 tabs
        let new_tab_count = self.tabs.len() + 1;
        let content_height = self.tab_bar.content_height(height, new_tab_count);
        let cols = (width as f32 / self.cell_width) as usize;
        let rows = (content_height as f32 / self.cell_height) as usize;

        let tab_id = TabId(self.next_tab_id);
        self.next_tab_id += 1;

        let tab = Tab::new_with_command(
            tab_id, &self.shell, cols, rows, width, content_height, cwd, startup_cmd
        )?;
        self.tabs.insert(tab_id, tab);
        self.tab_order.push(tab_id);
        self.active_tab = tab_id;

        Ok(tab_id)
    }

    /// Close the current tab
    pub fn close_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false; // Can't close the last tab
        }

        let idx = self.tab_order.iter().position(|id| *id == self.active_tab);
        self.tabs.remove(&self.active_tab);
        if let Some(idx) = idx {
            self.tab_order.remove(idx);
            // Switch to next tab or previous if at end
            let new_idx = idx.min(self.tab_order.len() - 1);
            self.active_tab = self.tab_order[new_idx];
        }

        true
    }

    /// Switch to a specific tab by ID
    pub fn switch_to(&mut self, id: TabId) {
        if self.tabs.contains_key(&id) {
            self.active_tab = id;
        }
    }

    /// Switch to next tab
    pub fn next_tab(&mut self) {
        if let Some(idx) = self.tab_order.iter().position(|id| *id == self.active_tab) {
            let next_idx = (idx + 1) % self.tab_order.len();
            self.active_tab = self.tab_order[next_idx];
        }
    }

    /// Switch to previous tab
    pub fn prev_tab(&mut self) {
        if let Some(idx) = self.tab_order.iter().position(|id| *id == self.active_tab) {
            let prev_idx = if idx == 0 { self.tab_order.len() - 1 } else { idx - 1 };
            self.active_tab = self.tab_order[prev_idx];
        }
    }

    /// Switch to tab by index (1-based for user-facing)
    pub fn switch_to_tab(&mut self, index: usize) {
        if index > 0 && index <= self.tab_order.len() {
            self.active_tab = self.tab_order[index - 1];
        }
    }

    /// Split the focused pane horizontally
    pub fn split_horizontal(
        &mut self,
        cwd: Option<&std::path::Path>,
    ) -> Result<Option<PaneId>> {
        self.split_horizontal_with_command(cwd, None)
    }

    /// Split the focused pane horizontally with an optional startup command
    pub fn split_horizontal_with_command(
        &mut self,
        cwd: Option<&std::path::Path>,
        startup_cmd: Option<&str>,
    ) -> Result<Option<PaneId>> {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            let pane_id = tab.split_with_command(
                super::split::SplitDirection::Horizontal,
                &self.shell,
                self.cell_width,
                self.cell_height,
                cwd,
                startup_cmd,
            )?;
            Ok(Some(pane_id))
        } else {
            Ok(None)
        }
    }

    /// Split the focused pane vertically
    pub fn split_vertical(
        &mut self,
        cwd: Option<&std::path::Path>,
    ) -> Result<Option<PaneId>> {
        self.split_vertical_with_command(cwd, None)
    }

    /// Split the focused pane vertically with an optional startup command
    pub fn split_vertical_with_command(
        &mut self,
        cwd: Option<&std::path::Path>,
        startup_cmd: Option<&str>,
    ) -> Result<Option<PaneId>> {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            let pane_id = tab.split_with_command(
                super::split::SplitDirection::Vertical,
                &self.shell,
                self.cell_width,
                self.cell_height,
                cwd,
                startup_cmd,
            )?;
            Ok(Some(pane_id))
        } else {
            Ok(None)
        }
    }

    /// Close the focused pane (or tab if last pane)
    pub fn close_pane(&mut self) -> bool {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            let focused = tab.focused;
            if tab.panes.len() > 1 {
                tab.close_pane(focused);
                true
            } else {
                // Last pane in tab - close the tab instead
                self.close_tab()
            }
        } else {
            false
        }
    }

    /// Focus pane in direction
    pub fn focus_direction(&mut self, direction: Direction, width: u32, height: u32) {
        let content_height = self.tab_bar.content_height(height, self.tabs.len());
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            tab.focus_direction(direction, width, content_height);
        }
    }

    /// Relayout all panes after resize
    pub fn relayout(&mut self, width: u32, height: u32) -> Result<()> {
        let content_height = self.tab_bar.content_height(height, self.tabs.len());
        for tab in self.tabs.values_mut() {
            tab.relayout(width, content_height, self.cell_width, self.cell_height)?;
        }
        Ok(())
    }

    /// Update cell dimensions (e.g., after font change)
    pub fn set_cell_size(&mut self, width: f32, height: f32) {
        self.cell_width = width;
        self.cell_height = height;
    }

    /// Get tab bar render data
    pub fn render_tab_bar(&self, width: u32, height: u32) -> TabBarRenderData {
        let tabs: Vec<_> = self.tab_order
            .iter()
            .filter_map(|id| {
                self.tabs.get(id).map(|tab| {
                    (*id, tab.title.clone(), *id == self.active_tab)
                })
            })
            .collect();

        self.tab_bar.render(&tabs, width, height, self.cell_width, self.cell_height)
    }

    /// Handle a click at pixel coordinates
    /// Returns true if the click was in the tab bar and handled
    pub fn handle_click(&mut self, x: i16, y: i16, width: u32) -> bool {
        // Only handle if tab bar is visible (more than 1 tab)
        if self.tabs.len() <= 1 {
            return false;
        }

        // Check if click is in tab bar area
        let tab_bar_height = self.tab_bar.height;
        if y < 0 || y as u32 >= tab_bar_height {
            return false;
        }

        // Find which tab was clicked
        let tab_count = self.tab_order.len() as f32;
        let max_tab_width = 200.0f32;
        let tab_width = (width as f32 / tab_count).min(max_tab_width);

        let click_x = x as f32;
        let tab_index = (click_x / tab_width) as usize;

        if tab_index < self.tab_order.len() {
            let tab_id = self.tab_order[tab_index];
            if tab_id != self.active_tab {
                self.switch_to(tab_id);
                self.mark_all_dirty();
            }
            true
        } else {
            false
        }
    }

    /// Get content Y offset (below tab bar)
    /// Returns 0 if only one tab (tab bar hidden)
    pub fn content_offset(&self) -> u32 {
        self.tab_bar.content_offset(self.tabs.len())
    }

    /// Check for exited panes in all tabs
    pub fn check_exits(&mut self) -> Vec<(TabId, PaneId)> {
        let mut exited = Vec::new();
        for (tab_id, tab) in &mut self.tabs {
            for pane_id in tab.check_exits() {
                exited.push((*tab_id, pane_id));
            }
        }
        exited
    }

    /// Handle exited panes (close them)
    /// Returns true if any pane or tab was closed (caller should relayout)
    pub fn handle_exits(&mut self) -> bool {
        let exits = self.check_exits();
        let had_exits = !exits.is_empty();

        for (tab_id, pane_id) in exits {
            if let Some(tab) = self.tabs.get_mut(&tab_id) {
                // Clean up terminal before closing (restores primary screen if needed)
                // This handles TUI programs that exit without sending rmcup
                if let Some(pane) = tab.panes.get_mut(&pane_id) {
                    pane.terminal.cleanup_on_exit();
                }

                if tab.panes.len() > 1 {
                    tab.close_pane(pane_id);
                } else {
                    // Last pane in tab - mark tab for removal
                }
            }
        }

        // Clean up empty tabs
        let empty_tabs: Vec<_> = self.tabs
            .iter()
            .filter(|(_, tab)| tab.panes.is_empty())
            .map(|(id, _)| *id)
            .collect();

        let had_empty = !empty_tabs.is_empty();

        for id in empty_tabs {
            if self.tabs.len() > 1 {
                self.tabs.remove(&id);
                self.tab_order.retain(|tid| *tid != id);
                if self.active_tab == id && !self.tab_order.is_empty() {
                    self.active_tab = self.tab_order[0];
                }
            }
        }

        had_exits || had_empty
    }

    /// Mark all panes in active tab as dirty
    pub fn mark_all_dirty(&mut self) {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            for pane in tab.panes.values_mut() {
                pane.mark_dirty();
            }
        }
    }

    /// Get number of tabs
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Check if there are any tabs left
    pub fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    /// Iterate over all panes in the active tab
    pub fn active_panes(&self) -> impl Iterator<Item = &super::pane::Pane> {
        self.active_tab()
            .map(|t| t.panes.values())
            .into_iter()
            .flatten()
    }

    /// Iterate mutably over all panes in the active tab
    pub fn active_panes_mut(&mut self) -> impl Iterator<Item = &mut super::pane::Pane> {
        self.active_tab_mut()
            .map(|t| t.panes.values_mut())
            .into_iter()
            .flatten()
    }

    /// Get all PTY file descriptors for polling
    pub fn pty_fds(&self) -> Vec<(TabId, PaneId, std::os::fd::RawFd)> {
        let mut fds = Vec::new();
        for (tab_id, tab) in &self.tabs {
            for (pane_id, pane) in &tab.panes {
                fds.push((*tab_id, *pane_id, pane.pty_fd()));
            }
        }
        fds
    }

    /// Update tab title from focused pane's terminal title
    pub fn update_titles(&mut self) {
        for tab in self.tabs.values_mut() {
            tab.update_title();
        }
    }

    /// Set a custom title for a tab
    pub fn set_tab_title(&mut self, tab_id: TabId, title: String) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.set_title(title);
        }
    }

    /// Find which tab contains a pane
    pub fn find_pane_tab(&self, pane_id: PaneId) -> Option<TabId> {
        for (tab_id, tab) in &self.tabs {
            if tab.panes.contains_key(&pane_id) {
                return Some(*tab_id);
            }
        }
        None
    }

    /// Get a pane by ID (searches all tabs)
    pub fn get_pane(&self, pane_id: PaneId) -> Option<&super::pane::Pane> {
        for tab in self.tabs.values() {
            if let Some(pane) = tab.panes.get(&pane_id) {
                return Some(pane);
            }
        }
        None
    }

    /// Get a pane mutably by ID (searches all tabs)
    pub fn get_pane_mut(&mut self, pane_id: PaneId) -> Option<&mut super::pane::Pane> {
        for tab in self.tabs.values_mut() {
            if let Some(pane) = tab.panes.get_mut(&pane_id) {
                return Some(pane);
            }
        }
        None
    }

    /// Focus a specific pane by ID (switches tab if needed)
    pub fn focus_pane(&mut self, pane_id: PaneId) -> bool {
        if let Some(tab_id) = self.find_pane_tab(pane_id) {
            self.active_tab = tab_id;
            if let Some(tab) = self.tabs.get_mut(&tab_id) {
                tab.focus(pane_id);
                return true;
            }
        }
        false
    }

    /// Get the active tab ID
    pub fn active_tab_id(&self) -> TabId {
        self.active_tab
    }

    /// Resize the focused pane by adjusting split ratio
    /// delta > 0 makes the pane larger, delta < 0 makes it smaller
    pub fn resize_focused_pane(&mut self, delta: f32) -> bool {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            tab.resize_focused_pane(delta)
        } else {
            false
        }
    }
}
