//! Tab - a collection of panes in a split layout

use super::pane::{Pane, PaneId};
use super::split::{Direction, PaneLayout, SplitDirection, SplitNode};
use anyhow::Result;
use std::collections::HashMap;

/// Unique identifier for a tab
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u32);

/// A tab containing one or more panes
pub struct Tab {
    /// Unique identifier
    pub id: TabId,
    /// Tab title (from focused pane or custom)
    pub title: String,
    /// Split tree layout
    pub layout: SplitNode,
    /// All panes in this tab
    pub panes: HashMap<PaneId, Pane>,
    /// Currently focused pane
    pub focused: PaneId,
    /// Next pane ID to assign
    next_pane_id: u32,
}

impl Tab {
    /// Create a new tab with an initial pane
    pub fn new(
        id: TabId,
        shell: &str,
        cols: usize,
        rows: usize,
        width: u32,
        height: u32,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self> {
        Self::new_with_command(id, shell, cols, rows, width, height, cwd, None)
    }

    /// Create a new tab with an initial pane and optional startup command
    pub fn new_with_command(
        id: TabId,
        shell: &str,
        cols: usize,
        rows: usize,
        width: u32,
        height: u32,
        cwd: Option<&std::path::Path>,
        startup_cmd: Option<&str>,
    ) -> Result<Self> {
        let pane_id = PaneId(0);
        let mut pane = Pane::new_with_command(
            pane_id, shell, cols, rows, width, height, cwd, startup_cmd
        )?;
        pane.focused = true;

        let mut panes = HashMap::new();
        panes.insert(pane_id, pane);

        Ok(Self {
            id,
            title: "shell".into(),
            layout: SplitNode::leaf(pane_id),
            panes,
            focused: pane_id,
            next_pane_id: 1,
        })
    }

    /// Get the focused pane
    pub fn focused_pane(&self) -> Option<&Pane> {
        self.panes.get(&self.focused)
    }

    /// Get the focused pane mutably
    pub fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(&self.focused)
    }

    /// Split the focused pane
    pub fn split(
        &mut self,
        direction: SplitDirection,
        shell: &str,
        cell_width: f32,
        cell_height: f32,
        cwd: Option<&std::path::Path>,
    ) -> Result<PaneId> {
        self.split_with_command(direction, shell, cell_width, cell_height, cwd, None)
    }

    /// Split the focused pane with an optional startup command
    pub fn split_with_command(
        &mut self,
        direction: SplitDirection,
        shell: &str,
        cell_width: f32,
        cell_height: f32,
        cwd: Option<&std::path::Path>,
        startup_cmd: Option<&str>,
    ) -> Result<PaneId> {
        let focused_pane = self.panes.get(&self.focused).ok_or_else(|| {
            anyhow::anyhow!("No focused pane")
        })?;

        // Calculate new pane dimensions (half of current)
        let (new_width, new_height) = match direction {
            SplitDirection::Horizontal => (focused_pane.width / 2, focused_pane.height),
            SplitDirection::Vertical => (focused_pane.width, focused_pane.height / 2),
        };

        let cols = (new_width as f32 / cell_width) as usize;
        let rows = (new_height as f32 / cell_height) as usize;

        // Create new pane
        let new_id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;

        let new_pane = Pane::new_with_command(
            new_id, shell, cols, rows, new_width, new_height, cwd, startup_cmd
        )?;
        self.panes.insert(new_id, new_pane);

        // Update layout tree
        if let Some(node) = self.layout.find_mut(self.focused) {
            node.split(direction, new_id, true);
        }

        // Focus the new pane
        self.focus(new_id);

        Ok(new_id)
    }

    /// Close a pane
    pub fn close_pane(&mut self, id: PaneId) -> Option<PaneId> {
        // Can't close if it's the only pane
        if self.panes.len() <= 1 {
            return None;
        }

        // Remove from layout and get sibling to focus
        let sibling = self.layout.remove(id);

        // Remove the pane
        self.panes.remove(&id);

        // Focus sibling if we closed the focused pane
        if self.focused == id {
            if let Some(new_focus) = sibling.or_else(|| self.layout.first_pane()) {
                self.focus(new_focus);
            }
        }

        sibling
    }

    /// Focus a pane
    pub fn focus(&mut self, id: PaneId) {
        if !self.panes.contains_key(&id) {
            return;
        }

        // Unfocus old pane
        if let Some(old) = self.panes.get_mut(&self.focused) {
            old.focused = false;
        }

        // Focus new pane
        self.focused = id;
        if let Some(new) = self.panes.get_mut(&id) {
            new.focused = true;
            new.mark_dirty();
        }
    }

    /// Focus pane in direction
    pub fn focus_direction(&mut self, direction: Direction, width: u32, height: u32) {
        let layouts = self.layout.layout(0, 0, width, height);
        if let Some(neighbor) = self.layout.find_neighbor(self.focused, direction, &layouts) {
            self.focus(neighbor);
        }
    }

    /// Recalculate layouts for all panes
    pub fn relayout(&mut self, width: u32, height: u32, cell_width: f32, cell_height: f32) -> Result<()> {
        let layouts = self.layout.layout(0, 0, width, height);

        for layout in layouts {
            if let Some(pane) = self.panes.get_mut(&layout.id) {
                let cols = (layout.width as f32 / cell_width) as usize;
                let rows = (layout.height as f32 / cell_height) as usize;

                pane.set_position(layout.x, layout.y);
                pane.resize(cols.max(1), rows.max(1), layout.width, layout.height)?;
            }
        }

        Ok(())
    }

    /// Get layout for all panes
    pub fn get_layouts(&self, width: u32, height: u32) -> Vec<PaneLayout> {
        self.layout.layout(0, 0, width, height)
    }

    /// Check if any pane has exited
    pub fn check_exits(&mut self) -> Vec<PaneId> {
        let exited: Vec<_> = self
            .panes
            .iter()
            .filter(|(_, p)| !p.is_alive())
            .map(|(id, _)| *id)
            .collect();

        exited
    }

    /// Get all pane IDs
    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.layout.all_panes()
    }

    /// Update tab title from focused pane's terminal title
    pub fn update_title(&mut self) {
        if let Some(pane) = self.panes.get(&self.focused) {
            let title = pane.terminal.title();
            if !title.is_empty() {
                self.title = title.to_string();
            }
        }
    }
}
