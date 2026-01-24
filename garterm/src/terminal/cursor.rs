use super::cell::{CellAttrs, CellColor};

/// Cursor state
#[derive(Debug, Clone)]
pub struct Cursor {
    /// Row position (0-indexed)
    pub row: usize,
    /// Column position (0-indexed)
    pub col: usize,
    /// Cursor style
    pub style: CursorStyle,
    /// Cursor visibility
    pub visible: bool,
    /// Cursor blink state (for rendering)
    pub blink_on: bool,
    /// Saved cursor state (for DECSC/DECRC)
    saved: Option<SavedCursor>,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            style: CursorStyle::Block,
            visible: true,
            blink_on: true,
            saved: None,
        }
    }
}

impl Cursor {
    /// Create a new cursor at the origin
    pub fn new() -> Self {
        Self::default()
    }

    /// Move cursor to the specified position
    pub fn goto(&mut self, row: usize, col: usize) {
        self.row = row;
        self.col = col;
    }

    /// Move cursor up by n rows (stops at top)
    pub fn move_up(&mut self, n: usize) {
        self.row = self.row.saturating_sub(n);
    }

    /// Move cursor down by n rows
    pub fn move_down(&mut self, n: usize, max_row: usize) {
        self.row = (self.row + n).min(max_row);
    }

    /// Move cursor left by n columns (stops at left edge)
    pub fn move_left(&mut self, n: usize) {
        self.col = self.col.saturating_sub(n);
    }

    /// Move cursor right by n columns
    pub fn move_right(&mut self, n: usize, max_col: usize) {
        self.col = (self.col + n).min(max_col);
    }

    /// Save cursor state (DECSC)
    pub fn save(&mut self, attrs: CellAttrs, fg: CellColor, bg: CellColor, origin_mode: bool, autowrap: bool) {
        self.saved = Some(SavedCursor {
            row: self.row,
            col: self.col,
            attrs,
            fg,
            bg,
            origin_mode,
            autowrap,
        });
    }

    /// Restore cursor state (DECRC)
    pub fn restore(&mut self) -> Option<(CellAttrs, CellColor, CellColor, bool, bool)> {
        self.saved.take().map(|s| {
            self.row = s.row;
            self.col = s.col;
            (s.attrs, s.fg, s.bg, s.origin_mode, s.autowrap)
        })
    }

    /// Check if cursor has saved state
    pub fn has_saved(&self) -> bool {
        self.saved.is_some()
    }
}

/// Cursor rendering style
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

/// Saved cursor state for DECSC/DECRC
#[derive(Debug, Clone)]
struct SavedCursor {
    row: usize,
    col: usize,
    attrs: CellAttrs,
    fg: CellColor,
    bg: CellColor,
    origin_mode: bool,
    autowrap: bool,
}
