mod cell;
mod cursor;
mod grid;
mod modes;

pub use cell::{Cell, CellAttrs, CellColor, UnderlineStyle};
pub use cursor::{Cursor, CursorStyle};
pub use grid::{Grid, Line};
pub use modes::{MouseEncoding, MouseMode, TerminalModes};

use tracing::trace;

/// Terminal state machine
pub struct Terminal {
    /// Grid with cells and scrollback
    grid: Grid,
    /// Alternate screen buffer
    alt_grid: Option<Grid>,
    /// Cursor state
    cursor: Cursor,
    /// Current cell attributes (for new characters)
    attrs: CellAttrs,
    /// Current foreground color
    fg: CellColor,
    /// Current background color
    bg: CellColor,
    /// Terminal modes
    modes: TerminalModes,
    /// Scroll region (top, bottom) - 0-indexed, inclusive
    scroll_region: (usize, usize),
    /// Tab stops
    tabs: Vec<bool>,
    /// Terminal title
    title: String,
    /// VTE parser
    parser: vte::Parser,
    /// Dimensions
    cols: usize,
    rows: usize,
    /// Dirty flag (needs re-render)
    dirty: bool,
}

impl Terminal {
    /// Create a new terminal with the given dimensions
    pub fn new(cols: usize, rows: usize) -> Self {
        let mut tabs = vec![false; cols];
        // Default tab stops every 8 columns
        for i in (8..cols).step_by(8) {
            tabs[i] = true;
        }

        Self {
            grid: Grid::new(cols, rows, 10000),
            alt_grid: None,
            cursor: Cursor::new(),
            attrs: CellAttrs::default(),
            fg: CellColor::Default,
            bg: CellColor::Default,
            modes: TerminalModes::new(),
            scroll_region: (0, rows - 1),
            tabs,
            title: String::new(),
            parser: vte::Parser::new(),
            cols,
            rows,
            dirty: true,
        }
    }

    /// Get terminal dimensions
    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Get the grid
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Get the cursor
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Get terminal modes
    pub fn modes(&self) -> &TerminalModes {
        &self.modes
    }

    /// Get terminal title
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Check and clear dirty flag
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// Mark terminal as dirty
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Resize terminal
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.grid.resize(cols, rows);
        if let Some(ref mut alt) = self.alt_grid {
            alt.resize(cols, rows);
        }

        // Update tab stops
        self.tabs.resize(cols, false);
        for i in (8..cols).step_by(8) {
            self.tabs[i] = true;
        }

        // Clamp cursor
        self.cursor.col = self.cursor.col.min(cols.saturating_sub(1));
        self.cursor.row = self.cursor.row.min(rows.saturating_sub(1));

        // Update scroll region
        self.scroll_region = (0, rows - 1);

        self.cols = cols;
        self.rows = rows;
        self.dirty = true;
    }

    /// Process input bytes from PTY
    pub fn input(&mut self, bytes: &[u8]) {
        // Take parser temporarily to avoid borrow conflict
        let mut parser = std::mem::take(&mut self.parser);
        for byte in bytes {
            let mut performer = Performer { term: self };
            parser.advance(&mut performer, *byte);
        }
        self.parser = parser;
    }

    /// Write a character at the cursor position
    fn write_char(&mut self, c: char) {
        // Handle autowrap
        if self.cursor.col >= self.cols {
            if self.modes.autowrap {
                self.carriage_return();
                self.linefeed();
                if let Some(line) = self.grid.line_mut(self.cursor.row.saturating_sub(1)) {
                    line.wrapped = true;
                }
            } else {
                self.cursor.col = self.cols - 1;
            }
        }

        // Write the character
        if let Some(cell) = self.grid.cell_mut(self.cursor.row, self.cursor.col) {
            cell.c = c;
            cell.attrs = self.attrs;
            cell.fg = self.fg;
            cell.bg = self.bg;
            cell.extra = None;
        }

        self.cursor.col += 1;
        self.dirty = true;
    }

    /// Carriage return
    fn carriage_return(&mut self) {
        self.cursor.col = 0;
    }

    /// Line feed (with optional carriage return in LNM mode)
    fn linefeed(&mut self) {
        if self.cursor.row >= self.scroll_region.1 {
            // At bottom of scroll region, scroll up
            self.grid.scroll_up(1, self.scroll_region.0, self.scroll_region.1);
        } else {
            self.cursor.row += 1;
        }

        if self.modes.linefeed {
            self.carriage_return();
        }

        self.dirty = true;
    }

    /// Reverse index (move up, scroll down if at top)
    fn reverse_index(&mut self) {
        if self.cursor.row <= self.scroll_region.0 {
            self.grid.scroll_down(1, self.scroll_region.0, self.scroll_region.1);
        } else {
            self.cursor.row -= 1;
        }
        self.dirty = true;
    }

    /// Move to next tab stop
    fn tab(&mut self) {
        let next_tab = self.tabs.iter()
            .enumerate()
            .skip(self.cursor.col + 1)
            .find(|&(_, is_tab)| *is_tab)
            .map(|(i, _)| i)
            .unwrap_or(self.cols - 1);

        self.cursor.col = next_tab.min(self.cols - 1);
    }

    /// Backspace
    fn backspace(&mut self) {
        self.cursor.col = self.cursor.col.saturating_sub(1);
    }

    /// Set cursor position (1-indexed input, handles origin mode)
    fn set_cursor_pos(&mut self, row: usize, col: usize) {
        let row = row.saturating_sub(1);
        let col = col.saturating_sub(1);

        let (row_offset, max_row) = if self.modes.origin {
            (self.scroll_region.0, self.scroll_region.1)
        } else {
            (0, self.rows - 1)
        };

        self.cursor.row = (row + row_offset).min(max_row);
        self.cursor.col = col.min(self.cols - 1);
    }

    /// Erase in display
    fn erase_in_display(&mut self, mode: u16) {
        match mode {
            0 => {
                // Erase from cursor to end of screen
                if let Some(line) = self.grid.line_mut(self.cursor.row) {
                    line.clear_from(self.cursor.col);
                }
                for row in self.cursor.row + 1..self.rows {
                    if let Some(line) = self.grid.line_mut(row) {
                        line.clear();
                    }
                }
            }
            1 => {
                // Erase from start of screen to cursor
                for row in 0..self.cursor.row {
                    if let Some(line) = self.grid.line_mut(row) {
                        line.clear();
                    }
                }
                if let Some(line) = self.grid.line_mut(self.cursor.row) {
                    line.clear_to(self.cursor.col);
                }
            }
            2 => {
                // Erase entire screen
                self.grid.clear();
            }
            3 => {
                // Erase entire screen + scrollback
                self.grid.clear_all();
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// Erase in line
    fn erase_in_line(&mut self, mode: u16) {
        if let Some(line) = self.grid.line_mut(self.cursor.row) {
            match mode {
                0 => line.clear_from(self.cursor.col),
                1 => line.clear_to(self.cursor.col),
                2 => line.clear(),
                _ => {}
            }
        }
        self.dirty = true;
    }

    /// Insert blank characters at cursor
    fn insert_blank(&mut self, n: usize) {
        let n = n.min(self.cols - self.cursor.col);
        if let Some(line) = self.grid.line_mut(self.cursor.row) {
            // Shift cells right
            for col in (self.cursor.col + n..self.cols).rev() {
                let src_col = col - n;
                let cell = line[src_col].clone();
                line[col] = cell;
            }
            // Clear inserted cells
            for col in self.cursor.col..self.cursor.col + n {
                line[col].reset();
            }
        }
        self.dirty = true;
    }

    /// Delete characters at cursor
    fn delete_chars(&mut self, n: usize) {
        let n = n.min(self.cols - self.cursor.col);
        if let Some(line) = self.grid.line_mut(self.cursor.row) {
            // Shift cells left
            for col in self.cursor.col..self.cols - n {
                let cell = line[col + n].clone();
                line[col] = cell;
            }
            // Clear vacated cells
            for col in self.cols - n..self.cols {
                line[col].reset();
            }
        }
        self.dirty = true;
    }

    /// Insert lines at cursor
    fn insert_lines(&mut self, n: usize) {
        if self.cursor.row >= self.scroll_region.0 && self.cursor.row <= self.scroll_region.1 {
            self.grid.insert_lines(self.cursor.row, n, self.scroll_region.1);
            self.dirty = true;
        }
    }

    /// Delete lines at cursor
    fn delete_lines(&mut self, n: usize) {
        if self.cursor.row >= self.scroll_region.0 && self.cursor.row <= self.scroll_region.1 {
            self.grid.delete_lines(self.cursor.row, n, self.scroll_region.1);
            self.dirty = true;
        }
    }

    /// Set scroll region
    fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let top = top.saturating_sub(1).min(self.rows - 1);
        let bottom = bottom.saturating_sub(1).min(self.rows - 1);

        if top < bottom {
            self.scroll_region = (top, bottom);
            // Move cursor to home (or origin if origin mode)
            self.set_cursor_pos(1, 1);
        }
    }

    /// Switch to alternate screen buffer
    fn switch_to_alt_screen(&mut self) {
        if self.alt_grid.is_none() {
            self.alt_grid = Some(Grid::new(self.cols, self.rows, 0));
        }
        std::mem::swap(&mut self.grid, self.alt_grid.as_mut().unwrap());
        self.grid.clear();
        self.dirty = true;
    }

    /// Switch to primary screen buffer
    fn switch_to_primary_screen(&mut self) {
        if let Some(ref mut alt) = self.alt_grid {
            std::mem::swap(&mut self.grid, alt);
            self.dirty = true;
        }
    }

    /// Apply SGR (Select Graphic Rendition) parameters
    fn apply_sgr(&mut self, params: &vte::Params) {
        let mut iter = params.iter();

        while let Some(param) = iter.next() {
            let code = param.first().copied().unwrap_or(0);

            match code {
                0 => {
                    // Reset all attributes
                    self.attrs = CellAttrs::default();
                    self.fg = CellColor::Default;
                    self.bg = CellColor::Default;
                }
                1 => self.attrs.bold = true,
                2 => self.attrs.dim = true,
                3 => self.attrs.italic = true,
                4 => {
                    // Underline - check for subparameter
                    if param.len() > 1 {
                        self.attrs.underline = match param[1] {
                            0 => UnderlineStyle::None,
                            1 => UnderlineStyle::Single,
                            2 => UnderlineStyle::Double,
                            3 => UnderlineStyle::Curly,
                            4 => UnderlineStyle::Dotted,
                            5 => UnderlineStyle::Dashed,
                            _ => UnderlineStyle::Single,
                        };
                    } else {
                        self.attrs.underline = UnderlineStyle::Single;
                    }
                }
                5 => self.attrs.blink = true,
                7 => self.attrs.inverse = true,
                8 => self.attrs.hidden = true,
                9 => self.attrs.strikethrough = true,
                21 => self.attrs.underline = UnderlineStyle::Double,
                22 => {
                    self.attrs.bold = false;
                    self.attrs.dim = false;
                }
                23 => self.attrs.italic = false,
                24 => self.attrs.underline = UnderlineStyle::None,
                25 => self.attrs.blink = false,
                27 => self.attrs.inverse = false,
                28 => self.attrs.hidden = false,
                29 => self.attrs.strikethrough = false,

                // Foreground colors
                30..=37 => self.fg = CellColor::Indexed((code - 30) as u8),
                38 => {
                    if let Some(color) = self.parse_color(&mut iter) {
                        self.fg = color;
                    }
                }
                39 => self.fg = CellColor::Default,

                // Background colors
                40..=47 => self.bg = CellColor::Indexed((code - 40) as u8),
                48 => {
                    if let Some(color) = self.parse_color(&mut iter) {
                        self.bg = color;
                    }
                }
                49 => self.bg = CellColor::Default,

                // Bright foreground colors
                90..=97 => self.fg = CellColor::Indexed((code - 90 + 8) as u8),

                // Bright background colors
                100..=107 => self.bg = CellColor::Indexed((code - 100 + 8) as u8),

                _ => {} // Unknown, ignore
            }
        }
    }

    /// Parse extended color (256 or RGB)
    fn parse_color<'a>(&self, iter: &mut impl Iterator<Item = &'a [u16]>) -> Option<CellColor> {
        let mode = iter.next()?.first()?;

        match mode {
            2 => {
                // RGB color: 38;2;r;g;b
                let r = iter.next()?.first()? & 0xFF;
                let g = iter.next()?.first()? & 0xFF;
                let b = iter.next()?.first()? & 0xFF;
                Some(CellColor::Rgb(r as u8, g as u8, b as u8))
            }
            5 => {
                // 256 color: 38;5;n
                let n = iter.next()?.first()? & 0xFF;
                Some(CellColor::Indexed(n as u8))
            }
            _ => None,
        }
    }
}

/// vte::Perform implementation for Terminal
struct Performer<'a> {
    term: &'a mut Terminal,
}

impl vte::Perform for Performer<'_> {
    fn print(&mut self, c: char) {
        trace!("print: {:?}", c);
        self.term.write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        trace!("execute: 0x{:02x}", byte);
        match byte {
            0x07 => {} // BEL - TODO: handle bell
            0x08 => self.term.backspace(),
            0x09 => self.term.tab(),
            0x0A | 0x0B | 0x0C => self.term.linefeed(),
            0x0D => self.term.carriage_return(),
            _ => {}
        }
    }

    fn hook(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, action: char) {
        trace!("hook: {:?} {:?} {} {:?}", params, intermediates, ignore, action);
    }

    fn put(&mut self, byte: u8) {
        trace!("put: 0x{:02x}", byte);
    }

    fn unhook(&mut self) {
        trace!("unhook");
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        trace!("osc: {:?} bell={}", params, bell_terminated);

        if params.is_empty() {
            return;
        }

        match params[0] {
            // Set window title
            b"0" | b"2" => {
                if params.len() > 1 {
                    if let Ok(title) = std::str::from_utf8(params[1]) {
                        self.term.title = title.to_string();
                    }
                }
            }
            // Set icon name (ignored)
            b"1" => {}
            // TODO: OSC 7 (CWD), OSC 8 (hyperlinks), OSC 52 (clipboard), etc.
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, action: char) {
        trace!("csi: {:?} {:?} {} {:?}", params, intermediates, ignore, action);

        if ignore {
            return;
        }

        let param = |n: usize, default: u16| -> u16 {
            params.iter().nth(n).and_then(|p| p.first().copied()).unwrap_or(default)
        };

        match (action, intermediates) {
            // Cursor movement
            ('A', []) => self.term.cursor.move_up(param(0, 1) as usize),
            ('B', []) => self.term.cursor.move_down(param(0, 1) as usize, self.term.rows - 1),
            ('C', []) => self.term.cursor.move_right(param(0, 1) as usize, self.term.cols - 1),
            ('D', []) => self.term.cursor.move_left(param(0, 1) as usize),
            ('E', []) => {
                self.term.cursor.move_down(param(0, 1) as usize, self.term.rows - 1);
                self.term.carriage_return();
            }
            ('F', []) => {
                self.term.cursor.move_up(param(0, 1) as usize);
                self.term.carriage_return();
            }
            ('G', []) => self.term.cursor.col = (param(0, 1) as usize).saturating_sub(1).min(self.term.cols - 1),
            ('H', []) | ('f', []) => self.term.set_cursor_pos(param(0, 1) as usize, param(1, 1) as usize),
            ('d', []) => self.term.cursor.row = (param(0, 1) as usize).saturating_sub(1).min(self.term.rows - 1),

            // Erase
            ('J', []) => self.term.erase_in_display(param(0, 0)),
            ('K', []) => self.term.erase_in_line(param(0, 0)),

            // Insert/Delete
            ('@', []) => self.term.insert_blank(param(0, 1) as usize),
            ('P', []) => self.term.delete_chars(param(0, 1) as usize),
            ('L', []) => self.term.insert_lines(param(0, 1) as usize),
            ('M', []) => self.term.delete_lines(param(0, 1) as usize),
            ('X', []) => {
                // Erase characters
                let n = param(0, 1) as usize;
                if let Some(line) = self.term.grid.line_mut(self.term.cursor.row) {
                    for col in self.term.cursor.col..(self.term.cursor.col + n).min(self.term.cols) {
                        line[col].reset();
                    }
                }
                self.term.dirty = true;
            }

            // Scroll
            ('S', []) => self.term.grid.scroll_up(param(0, 1) as usize, self.term.scroll_region.0, self.term.scroll_region.1),
            ('T', []) => self.term.grid.scroll_down(param(0, 1) as usize, self.term.scroll_region.0, self.term.scroll_region.1),

            // SGR (Select Graphic Rendition)
            ('m', []) => self.term.apply_sgr(params),

            // Modes
            ('h', [b'?']) => {
                for param in params.iter() {
                    if let Some(&mode) = param.first() {
                        self.term.modes.set_dec_mode(mode, true);
                        if mode == 1049 || mode == 47 || mode == 1047 {
                            self.term.switch_to_alt_screen();
                        }
                    }
                }
            }
            ('l', [b'?']) => {
                for param in params.iter() {
                    if let Some(&mode) = param.first() {
                        self.term.modes.set_dec_mode(mode, false);
                        if mode == 1049 || mode == 47 || mode == 1047 {
                            self.term.switch_to_primary_screen();
                        }
                    }
                }
            }
            ('h', []) => {
                for param in params.iter() {
                    if let Some(&mode) = param.first() {
                        self.term.modes.set_ansi_mode(mode, true);
                    }
                }
            }
            ('l', []) => {
                for param in params.iter() {
                    if let Some(&mode) = param.first() {
                        self.term.modes.set_ansi_mode(mode, false);
                    }
                }
            }

            // Scroll region
            ('r', []) => {
                let top = param(0, 1);
                let bottom = param(1, self.term.rows as u16);
                self.term.set_scroll_region(top as usize, bottom as usize);
            }

            // Tab stops
            ('g', []) => {
                match param(0, 0) {
                    0 => self.term.tabs[self.term.cursor.col] = false,
                    3 => self.term.tabs.fill(false),
                    _ => {}
                }
            }

            // Save/restore cursor
            ('s', []) => self.term.cursor.save(self.term.attrs, self.term.fg, self.term.bg, self.term.modes.origin, self.term.modes.autowrap),
            ('u', []) => {
                if let Some((attrs, fg, bg, origin, autowrap)) = self.term.cursor.restore() {
                    self.term.attrs = attrs;
                    self.term.fg = fg;
                    self.term.bg = bg;
                    self.term.modes.origin = origin;
                    self.term.modes.autowrap = autowrap;
                }
            }

            // Device status report
            ('n', []) => {
                // TODO: respond with cursor position, etc.
            }

            // Cursor style (DECSCUSR)
            ('q', [b' ']) => {
                self.term.cursor.style = match param(0, 0) {
                    0 | 1 => CursorStyle::Block,
                    2 => CursorStyle::Block,
                    3 | 4 => CursorStyle::Underline,
                    5 | 6 => CursorStyle::Bar,
                    _ => CursorStyle::Block,
                };
            }

            _ => {
                trace!("unhandled CSI: {:?} {:?} {:?}", action, intermediates, params);
            }
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        trace!("esc: {:?} {} 0x{:02x}", intermediates, ignore, byte);

        if ignore {
            return;
        }

        match (byte, intermediates) {
            // DECSC - Save cursor
            (b'7', []) => self.term.cursor.save(self.term.attrs, self.term.fg, self.term.bg, self.term.modes.origin, self.term.modes.autowrap),
            // DECRC - Restore cursor
            (b'8', []) => {
                if let Some((attrs, fg, bg, origin, autowrap)) = self.term.cursor.restore() {
                    self.term.attrs = attrs;
                    self.term.fg = fg;
                    self.term.bg = bg;
                    self.term.modes.origin = origin;
                    self.term.modes.autowrap = autowrap;
                }
            }
            // RI - Reverse Index
            (b'M', []) => self.term.reverse_index(),
            // IND - Index (same as LF)
            (b'D', []) => self.term.linefeed(),
            // NEL - Next Line
            (b'E', []) => {
                self.term.carriage_return();
                self.term.linefeed();
            }
            // HTS - Horizontal Tab Set
            (b'H', []) => self.term.tabs[self.term.cursor.col] = true,
            // RIS - Reset to Initial State
            (b'c', []) => {
                let (cols, rows) = (self.term.cols, self.term.rows);
                *self.term = Terminal::new(cols, rows);
            }
            // DECALN - Screen alignment test (fill with 'E')
            (b'8', [b'#']) => {
                for row in 0..self.term.rows {
                    if let Some(line) = self.term.grid.line_mut(row) {
                        for cell in line.iter_mut() {
                            cell.c = 'E';
                        }
                    }
                }
                self.term.dirty = true;
            }
            _ => {
                trace!("unhandled ESC: {:?} 0x{:02x}", intermediates, byte);
            }
        }
    }
}
