//! Text selection handling for terminal

use crate::terminal::Grid;

/// Selection mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionMode {
    /// Normal character selection (follows line wrapping)
    #[default]
    Normal,
    /// Line selection (selects entire lines)
    Line,
    /// Block/rectangular selection
    Block,
}

/// A point in the terminal grid
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionPoint {
    pub row: usize,
    pub col: usize,
}

impl SelectionPoint {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }
}

/// Text selection state
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// Selection start point (anchor)
    start: Option<SelectionPoint>,
    /// Selection end point (cursor)
    end: Option<SelectionPoint>,
    /// Selection mode
    mode: SelectionMode,
    /// Whether selection is active (mouse button held)
    active: bool,
}

impl Selection {
    /// Create a new empty selection
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new selection at the given point
    pub fn start(&mut self, row: usize, col: usize, mode: SelectionMode) {
        self.start = Some(SelectionPoint::new(row, col));
        self.end = Some(SelectionPoint::new(row, col));
        self.mode = mode;
        self.active = true;
    }

    /// Update selection end point (during drag)
    pub fn update(&mut self, row: usize, col: usize) {
        if self.active {
            self.end = Some(SelectionPoint::new(row, col));
        }
    }

    /// Finish selection (mouse button released)
    pub fn finish(&mut self) {
        self.active = false;
    }

    /// Clear the selection
    pub fn clear(&mut self) {
        self.start = None;
        self.end = None;
        self.active = false;
    }

    /// Check if there is an active selection
    pub fn is_empty(&self) -> bool {
        self.start.is_none() || self.end.is_none()
    }

    /// Check if selection is currently being made (dragging)
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Get selection mode
    pub fn mode(&self) -> SelectionMode {
        self.mode
    }

    /// Get normalized selection bounds (start <= end)
    pub fn bounds(&self) -> Option<(SelectionPoint, SelectionPoint)> {
        let start = self.start?;
        let end = self.end?;

        // Normalize so start is before end
        let (start, end) = if (start.row, start.col) <= (end.row, end.col) {
            (start, end)
        } else {
            (end, start)
        };

        Some((start, end))
    }

    /// Check if a cell is within the selection
    pub fn contains(&self, row: usize, col: usize, _cols: usize) -> bool {
        let Some((start, end)) = self.bounds() else {
            return false;
        };

        match self.mode {
            SelectionMode::Normal => {
                // Normal selection: continuous from start to end
                if row < start.row || row > end.row {
                    return false;
                }
                if row == start.row && row == end.row {
                    // Single line
                    col >= start.col && col <= end.col
                } else if row == start.row {
                    // First line
                    col >= start.col
                } else if row == end.row {
                    // Last line
                    col <= end.col
                } else {
                    // Middle lines - entire line selected
                    true
                }
            }
            SelectionMode::Line => {
                // Line selection: entire lines
                row >= start.row && row <= end.row
            }
            SelectionMode::Block => {
                // Block selection: rectangular
                let (min_col, max_col) = if start.col <= end.col {
                    (start.col, end.col)
                } else {
                    (end.col, start.col)
                };
                row >= start.row && row <= end.row && col >= min_col && col <= max_col
            }
        }
    }

    /// Get selected text from the grid
    pub fn get_text(&self, grid: &Grid, cols: usize) -> String {
        let Some((start, end)) = self.bounds() else {
            return String::new();
        };

        let mut result = String::new();

        match self.mode {
            SelectionMode::Normal => {
                for row in start.row..=end.row {
                    if let Some(line) = grid.line(row) {
                        let start_col = if row == start.row { start.col } else { 0 };
                        let end_col = if row == end.row { end.col } else { cols - 1 };

                        for col in start_col..=end_col.min(cols - 1) {
                            let c = line[col].c;
                            if c != '\0' {
                                result.push(c);
                            }
                        }

                        // Add newline between lines (but not if line is wrapped)
                        if row != end.row && !line.wrapped {
                            result.push('\n');
                        }
                    }
                }
            }
            SelectionMode::Line => {
                for row in start.row..=end.row {
                    if let Some(line) = grid.line(row) {
                        // Find last non-space character
                        let mut last_non_space = 0;
                        for col in 0..cols {
                            if line[col].c != ' ' && line[col].c != '\0' {
                                last_non_space = col;
                            }
                        }

                        for col in 0..=last_non_space {
                            let c = line[col].c;
                            if c != '\0' {
                                result.push(c);
                            }
                        }

                        if row != end.row {
                            result.push('\n');
                        }
                    }
                }
            }
            SelectionMode::Block => {
                let (min_col, max_col) = if start.col <= end.col {
                    (start.col, end.col)
                } else {
                    (end.col, start.col)
                };

                for row in start.row..=end.row {
                    if let Some(line) = grid.line(row) {
                        for col in min_col..=max_col.min(cols - 1) {
                            let c = line[col].c;
                            if c != '\0' {
                                result.push(c);
                            }
                        }

                        if row != end.row {
                            result.push('\n');
                        }
                    }
                }
            }
        }

        // Trim trailing whitespace from each line
        result
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Expand selection to word boundaries
    pub fn select_word(&mut self, row: usize, col: usize, grid: &Grid, cols: usize) {
        let Some(line) = grid.line(row) else {
            return;
        };

        // Find word boundaries
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

        let mut start_col = col;
        let mut end_col = col;

        // Expand left
        while start_col > 0 && is_word_char(line[start_col - 1].c) {
            start_col -= 1;
        }

        // Expand right
        while end_col < cols - 1 && is_word_char(line[end_col + 1].c) {
            end_col += 1;
        }

        self.start = Some(SelectionPoint::new(row, start_col));
        self.end = Some(SelectionPoint::new(row, end_col));
        self.mode = SelectionMode::Normal;
        self.active = false;
    }

    /// Select entire line
    pub fn select_line(&mut self, row: usize, cols: usize) {
        self.start = Some(SelectionPoint::new(row, 0));
        self.end = Some(SelectionPoint::new(row, cols - 1));
        self.mode = SelectionMode::Line;
        self.active = false;
    }

    /// Select all text
    pub fn select_all(&mut self, rows: usize, cols: usize) {
        self.start = Some(SelectionPoint::new(0, 0));
        self.end = Some(SelectionPoint::new(rows - 1, cols - 1));
        self.mode = SelectionMode::Normal;
        self.active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_contains_normal() {
        let mut sel = Selection::new();
        sel.start(1, 5, SelectionMode::Normal);
        sel.update(3, 10);
        sel.finish();

        // First line
        assert!(!sel.contains(1, 4, 80));
        assert!(sel.contains(1, 5, 80));
        assert!(sel.contains(1, 79, 80));

        // Middle line
        assert!(sel.contains(2, 0, 80));
        assert!(sel.contains(2, 79, 80));

        // Last line
        assert!(sel.contains(3, 0, 80));
        assert!(sel.contains(3, 10, 80));
        assert!(!sel.contains(3, 11, 80));

        // Outside
        assert!(!sel.contains(0, 0, 80));
        assert!(!sel.contains(4, 0, 80));
    }

    #[test]
    fn test_selection_contains_block() {
        let mut sel = Selection::new();
        sel.start(1, 5, SelectionMode::Block);
        sel.update(3, 10);
        sel.finish();

        // Inside block
        assert!(sel.contains(1, 5, 80));
        assert!(sel.contains(2, 7, 80));
        assert!(sel.contains(3, 10, 80));

        // Outside block
        assert!(!sel.contains(1, 4, 80));
        assert!(!sel.contains(1, 11, 80));
        assert!(!sel.contains(2, 4, 80));
        assert!(!sel.contains(2, 11, 80));
    }
}
