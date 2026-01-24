use super::cell::Cell;
use std::collections::VecDeque;
use std::ops::{Index, IndexMut};

/// A single line in the terminal
#[derive(Debug, Clone)]
pub struct Line {
    cells: Vec<Cell>,
    /// Line is wrapped from previous line
    pub wrapped: bool,
}

impl Line {
    /// Create a new line with the given number of columns
    pub fn new(cols: usize) -> Self {
        Self {
            cells: vec![Cell::default(); cols],
            wrapped: false,
        }
    }

    /// Get the number of columns
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Resize the line to the given number of columns
    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::default());
    }

    /// Clear the line (reset all cells to default)
    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            cell.reset();
        }
        self.wrapped = false;
    }

    /// Clear from column to end of line
    pub fn clear_from(&mut self, col: usize) {
        for cell in self.cells.iter_mut().skip(col) {
            cell.reset();
        }
    }

    /// Clear from start of line to column (inclusive)
    pub fn clear_to(&mut self, col: usize) {
        for cell in self.cells.iter_mut().take(col + 1) {
            cell.reset();
        }
    }

    /// Get iterator over cells
    pub fn iter(&self) -> impl Iterator<Item = &Cell> {
        self.cells.iter()
    }

    /// Get mutable iterator over cells
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Cell> {
        self.cells.iter_mut()
    }
}

impl Index<usize> for Line {
    type Output = Cell;

    fn index(&self, col: usize) -> &Self::Output {
        &self.cells[col]
    }
}

impl IndexMut<usize> for Line {
    fn index_mut(&mut self, col: usize) -> &mut Self::Output {
        &mut self.cells[col]
    }
}

/// Terminal grid with active display and scrollback
#[derive(Debug)]
pub struct Grid {
    /// Active display lines
    lines: Vec<Line>,
    /// Scrollback buffer
    scrollback: VecDeque<Line>,
    /// Number of columns
    cols: usize,
    /// Number of rows
    rows: usize,
    /// Maximum scrollback lines
    max_scrollback: usize,
    /// Current scroll offset (0 = at bottom, showing active display)
    scroll_offset: usize,
}

impl Grid {
    /// Create a new grid with the given dimensions
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        let lines = (0..rows).map(|_| Line::new(cols)).collect();
        Self {
            lines,
            scrollback: VecDeque::new(),
            cols,
            rows,
            max_scrollback,
            scroll_offset: 0,
        }
    }

    /// Get grid dimensions
    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Get current scroll offset
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Get total scrollback lines
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Resize the grid
    pub fn resize(&mut self, cols: usize, rows: usize) {
        // Resize existing lines
        for line in &mut self.lines {
            line.resize(cols);
        }

        // Add or remove rows
        if rows > self.rows {
            // Add new rows at bottom
            for _ in 0..rows - self.rows {
                self.lines.push(Line::new(cols));
            }
        } else if rows < self.rows {
            // Move excess lines to scrollback
            let excess: Vec<_> = self.lines.drain(..self.rows - rows).collect();
            for line in excess {
                self.push_scrollback(line);
            }
        }

        self.cols = cols;
        self.rows = rows;
        self.scroll_offset = self.scroll_offset.min(self.scrollback.len());
    }

    /// Get a cell reference
    pub fn cell(&self, row: usize, col: usize) -> Option<&Cell> {
        self.lines.get(row).and_then(|line| {
            if col < line.len() {
                Some(&line[col])
            } else {
                None
            }
        })
    }

    /// Get a mutable cell reference
    pub fn cell_mut(&mut self, row: usize, col: usize) -> Option<&mut Cell> {
        self.lines.get_mut(row).and_then(|line| {
            if col < line.len() {
                Some(&mut line[col])
            } else {
                None
            }
        })
    }

    /// Get a line reference
    pub fn line(&self, row: usize) -> Option<&Line> {
        self.lines.get(row)
    }

    /// Get a mutable line reference
    pub fn line_mut(&mut self, row: usize) -> Option<&mut Line> {
        self.lines.get_mut(row)
    }

    /// Scroll the grid up by n lines within a scroll region
    /// Lines scrolled out of the top go to scrollback
    pub fn scroll_up(&mut self, n: usize, top: usize, bottom: usize) {
        let n = n.min(bottom - top + 1);

        if top == 0 {
            // Lines going to scrollback
            let scrolled: Vec<_> = self.lines.drain(..n).collect();
            for line in scrolled {
                self.push_scrollback(line);
            }
            // Add new blank lines at bottom of region
            for _ in 0..n {
                self.lines.insert(bottom - n + 1, Line::new(self.cols));
            }
        } else {
            // Scroll within region only (no scrollback)
            for _ in 0..n {
                self.lines.remove(top);
                self.lines.insert(bottom, Line::new(self.cols));
            }
        }
    }

    /// Scroll the grid down by n lines within a scroll region
    pub fn scroll_down(&mut self, n: usize, top: usize, bottom: usize) {
        let n = n.min(bottom - top + 1);

        for _ in 0..n {
            self.lines.remove(bottom);
            self.lines.insert(top, Line::new(self.cols));
        }
    }

    /// Clear a region of the grid
    pub fn clear_region(&mut self, top: usize, left: usize, bottom: usize, right: usize) {
        for row in top..=bottom.min(self.rows - 1) {
            if let Some(line) = self.lines.get_mut(row) {
                for col in left..=right.min(self.cols - 1) {
                    line[col].reset();
                }
            }
        }
    }

    /// Clear entire grid
    pub fn clear(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
    }

    /// Clear grid and scrollback
    pub fn clear_all(&mut self) {
        self.clear();
        self.scrollback.clear();
        self.scroll_offset = 0;
    }

    /// Set scroll offset for viewing scrollback
    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset.min(self.scrollback.len());
    }

    /// Scroll viewport up (into scrollback)
    pub fn scroll_viewport_up(&mut self, n: usize) {
        self.scroll_offset = (self.scroll_offset + n).min(self.scrollback.len());
    }

    /// Scroll viewport down (toward bottom)
    pub fn scroll_viewport_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Reset viewport to bottom
    pub fn reset_viewport(&mut self) {
        self.scroll_offset = 0;
    }

    /// Check if viewport is at bottom
    pub fn is_at_bottom(&self) -> bool {
        self.scroll_offset == 0
    }

    /// Get visible lines (accounts for scroll offset)
    pub fn visible_lines(&self) -> impl Iterator<Item = &Line> {
        let scrollback_visible = self.scroll_offset.min(self.scrollback.len());
        let active_skip = if self.scroll_offset > self.scrollback.len() {
            self.scroll_offset - self.scrollback.len()
        } else {
            0
        };

        // Lines from scrollback (if scrolled back)
        let scrollback_start = self.scrollback.len().saturating_sub(scrollback_visible);
        let scrollback_lines = self.scrollback.range(scrollback_start..).take(scrollback_visible);

        // Lines from active display
        let active_lines = self.lines.iter().skip(active_skip).take(self.rows - scrollback_visible);

        scrollback_lines.chain(active_lines)
    }

    /// Push a line to scrollback
    fn push_scrollback(&mut self, line: Line) {
        self.scrollback.push_back(line);
        while self.scrollback.len() > self.max_scrollback {
            self.scrollback.pop_front();
        }
    }

    /// Insert n blank lines at row, pushing content down
    pub fn insert_lines(&mut self, row: usize, n: usize, bottom: usize) {
        let n = n.min(bottom - row + 1);
        for _ in 0..n {
            self.lines.remove(bottom);
            self.lines.insert(row, Line::new(self.cols));
        }
    }

    /// Delete n lines at row, pulling content up
    pub fn delete_lines(&mut self, row: usize, n: usize, bottom: usize) {
        let n = n.min(bottom - row + 1);
        for _ in 0..n {
            self.lines.remove(row);
            self.lines.insert(bottom, Line::new(self.cols));
        }
    }
}

impl Index<usize> for Grid {
    type Output = Line;

    fn index(&self, row: usize) -> &Self::Output {
        &self.lines[row]
    }
}

impl IndexMut<usize> for Grid {
    fn index_mut(&mut self, row: usize) -> &mut Self::Output {
        &mut self.lines[row]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_new() {
        let grid = Grid::new(80, 24, 1000);
        assert_eq!(grid.cols(), 80);
        assert_eq!(grid.rows(), 24);
    }

    #[test]
    fn test_scroll_up() {
        let mut grid = Grid::new(80, 24, 1000);
        grid[0][0].c = 'A';
        grid.scroll_up(1, 0, 23);
        assert_eq!(grid.scrollback_len(), 1);
        assert_eq!(grid[0][0].c, ' ');
    }
}
