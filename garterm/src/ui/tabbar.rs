//! Tab bar renderer

use super::tab::TabId;
use crate::config::TabBarConfig;

/// Height of the tab bar in pixels (default, can be overridden by config)
pub const TAB_BAR_HEIGHT: u32 = 24;

/// Shorten a path for display in tab title
/// e.g., "/home/user/Projects/foo/bar" → "~/P/f/bar"
fn shorten_path(title: &str) -> String {
    // If it doesn't look like a path, return as-is
    if !title.contains('/') {
        return title.to_string();
    }

    let mut path = title.to_string();

    // Replace home directory with ~
    if let Some(home) = dirs::home_dir() {
        if let Some(home_str) = home.to_str() {
            if path.starts_with(home_str) {
                path = format!("~{}", &path[home_str.len()..]);
            }
        }
    }

    // Split into components
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        return path;
    }

    // Keep first component (~ or root indicator) and last component full
    // Shorten middle components to first char
    let mut result = String::new();

    if path.starts_with("~/") {
        result.push_str("~/");
        // Shorten all but the last component
        for (i, part) in parts[1..].iter().enumerate() {
            if i == parts.len() - 2 {
                // Last component - keep full
                result.push_str(part);
            } else {
                // Middle component - first char only
                if let Some(c) = part.chars().next() {
                    result.push(c);
                    result.push('/');
                }
            }
        }
    } else if path.starts_with('/') {
        result.push('/');
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                result.push_str(part);
            } else {
                if let Some(c) = part.chars().next() {
                    result.push(c);
                    result.push('/');
                }
            }
        }
    } else {
        return path;
    }

    result
}

/// Truncate a string to fit within max_chars, adding ellipsis if needed
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    if max_chars <= 3 {
        return "…".to_string();
    }
    let truncated: String = s.chars().take(max_chars - 1).collect();
    format!("{}…", truncated)
}

/// Tab bar state
pub struct TabBar {
    /// Whether tab bar is visible
    pub visible: bool,
    /// Tab bar position: true = top, false = bottom
    pub top: bool,
    /// Tab bar height
    pub height: u32,
    /// Show tab bar even with single tab
    pub show_single_tab: bool,
    /// Maximum tab width
    pub max_tab_width: f32,
    /// Padding inside each tab
    pub tab_padding: f32,
    /// Whether to shorten paths
    pub shorten_paths: bool,
    /// Background color
    pub background: [f32; 4],
    /// Active tab background
    pub active_bg: [f32; 4],
    /// Inactive tab background
    pub inactive_bg: [f32; 4],
    /// Active tab text color
    pub active_fg: [f32; 4],
    /// Inactive tab text color
    pub inactive_fg: [f32; 4],
}

impl Default for TabBar {
    fn default() -> Self {
        Self {
            visible: true,
            top: true,
            height: TAB_BAR_HEIGHT,
            show_single_tab: false,
            max_tab_width: 200.0,
            tab_padding: 16.0,
            shorten_paths: true,
            background: [0.08, 0.08, 0.12, 1.0],
            active_bg: [0.15, 0.15, 0.20, 1.0],
            inactive_bg: [0.10, 0.10, 0.14, 1.0],
            active_fg: [1.0, 1.0, 1.0, 1.0],
            inactive_fg: [0.7, 0.7, 0.7, 1.0],
        }
    }
}

impl TabBar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create from config
    pub fn from_config(config: &TabBarConfig) -> Self {
        Self {
            visible: true,
            top: config.position == "top",
            height: config.height,
            show_single_tab: config.show_single_tab,
            max_tab_width: config.max_tab_width,
            tab_padding: config.tab_padding,
            shorten_paths: config.shorten_paths,
            background: config.background,
            active_bg: config.active_bg,
            inactive_bg: config.inactive_bg,
            active_fg: config.active_fg,
            inactive_fg: config.inactive_fg,
        }
    }

    /// Set visibility
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Get the Y offset for terminal content
    /// Tab bar is hidden for single tab unless show_single_tab is true
    pub fn content_offset(&self, tab_count: usize) -> u32 {
        let show = self.visible && (tab_count > 1 || self.show_single_tab);
        if show && self.top {
            self.height
        } else {
            0
        }
    }

    /// Get available height for terminal content
    /// Tab bar is hidden for single tab unless show_single_tab is true
    pub fn content_height(&self, total_height: u32, tab_count: usize) -> u32 {
        let show = self.visible && (tab_count > 1 || self.show_single_tab);
        if show {
            total_height.saturating_sub(self.height)
        } else {
            total_height
        }
    }

    /// Generate vertices for tab bar background and tabs
    /// Returns (vertices, indices) for rendering
    /// Tab bar is hidden when there's only one tab (unless show_single_tab is true)
    pub fn render(
        &self,
        tabs: &[(TabId, String, bool)], // (id, title, is_active)
        width: u32,
        _height: u32,
        cell_width: f32,
        cell_height: f32,
    ) -> TabBarRenderData {
        // Hide tab bar if not visible or not enough tabs
        let show = self.visible && (tabs.len() > 1 || self.show_single_tab);
        if !show || tabs.is_empty() {
            return TabBarRenderData::default();
        }

        let mut data = TabBarRenderData::default();

        // Background color from config
        data.background = Some(TabRect {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: self.height as f32,
            color: self.background,
        });

        // Calculate tab width
        let tab_count = tabs.len() as f32;
        let tab_width = (width as f32 / tab_count).min(self.max_tab_width);

        // Calculate max chars that fit in a tab (with padding)
        let available_width = tab_width - self.tab_padding;
        let max_chars = (available_width / cell_width).floor() as usize;

        let half_padding = self.tab_padding / 2.0;

        let mut x = 0.0;
        for (id, title, is_active) in tabs {
            // Tab background and text colors from config
            let (bg_color, fg_color) = if *is_active {
                (self.active_bg, self.active_fg)
            } else {
                (self.inactive_bg, self.inactive_fg)
            };

            // Optionally shorten path and truncate to fit
            let display_title = if self.shorten_paths {
                let shortened = shorten_path(title);
                truncate_with_ellipsis(&shortened, max_chars)
            } else {
                truncate_with_ellipsis(title, max_chars)
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
                title: display_title,
                title_x: x + half_padding,
                title_y: (self.height as f32 - cell_height) / 2.0,
                is_active: *is_active,
                fg_color,
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
    pub fg_color: [f32; 4],
}

/// All data needed to render the tab bar
#[derive(Debug, Default)]
pub struct TabBarRenderData {
    pub background: Option<TabRect>,
    pub tabs: Vec<TabRenderInfo>,
}
