//! Tab bar renderer

use super::tab::TabId;

/// Height of the tab bar in pixels
pub const TAB_BAR_HEIGHT: u32 = 24;

/// Tab bar state
pub struct TabBar {
    /// Whether tab bar is visible
    pub visible: bool,
    /// Tab bar position: true = top, false = bottom
    pub top: bool,
    /// Tab bar height
    pub height: u32,
}

impl Default for TabBar {
    fn default() -> Self {
        Self {
            visible: true,
            top: true,
            height: TAB_BAR_HEIGHT,
        }
    }
}

impl TabBar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set visibility
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Get the Y offset for terminal content
    /// Tab bar is hidden for single tab, so offset is 0 in that case
    pub fn content_offset(&self, tab_count: usize) -> u32 {
        if self.visible && self.top && tab_count > 1 {
            self.height
        } else {
            0
        }
    }

    /// Get available height for terminal content
    /// Tab bar is hidden for single tab, so full height is available
    pub fn content_height(&self, total_height: u32, tab_count: usize) -> u32 {
        if self.visible && tab_count > 1 {
            total_height.saturating_sub(self.height)
        } else {
            total_height
        }
    }

    /// Generate vertices for tab bar background and tabs
    /// Returns (vertices, indices) for rendering
    /// Tab bar is hidden when there's only one tab
    pub fn render(
        &self,
        tabs: &[(TabId, String, bool)], // (id, title, is_active)
        width: u32,
        _height: u32,
        cell_width: f32,
    ) -> TabBarRenderData {
        // Hide tab bar if not visible, no tabs, or only one tab
        if !self.visible || tabs.len() <= 1 {
            return TabBarRenderData::default();
        }

        let mut data = TabBarRenderData::default();

        // Background color (darker than terminal)
        data.background = Some(TabRect {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: self.height as f32,
            color: [0.08, 0.08, 0.12, 1.0], // Dark background
        });

        // Calculate tab width
        let tab_count = tabs.len() as f32;
        let max_tab_width = 200.0f32;
        let tab_width = (width as f32 / tab_count).min(max_tab_width);

        let mut x = 0.0;
        for (id, title, is_active) in tabs {
            // Tab background
            let bg_color = if *is_active {
                [0.15, 0.15, 0.20, 1.0] // Active tab
            } else {
                [0.10, 0.10, 0.14, 1.0] // Inactive tab
            };

            data.tabs.push(TabRenderInfo {
                id: *id,
                rect: TabRect {
                    x,
                    y: 0.0,
                    width: tab_width,
                    height: self.height as f32,
                    color: bg_color,
                },
                title: title.clone(),
                title_x: x + 8.0, // Padding
                title_y: (self.height as f32 - cell_width) / 2.0,
                is_active: *is_active,
            });

            x += tab_width;
        }

        data
    }
}

/// Rectangle for rendering
#[derive(Debug, Clone, Default)]
pub struct TabRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: [f32; 4],
}

/// Information for rendering a single tab
#[derive(Debug, Clone)]
pub struct TabRenderInfo {
    pub id: TabId,
    pub rect: TabRect,
    pub title: String,
    pub title_x: f32,
    pub title_y: f32,
    pub is_active: bool,
}

/// All data needed to render the tab bar
#[derive(Debug, Default)]
pub struct TabBarRenderData {
    pub background: Option<TabRect>,
    pub tabs: Vec<TabRenderInfo>,
}
