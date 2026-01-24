/// A single cell in the terminal grid
#[derive(Debug, Clone)]
pub struct Cell {
    /// Primary character
    pub c: char,
    /// Extra characters for combining chars / grapheme clusters
    pub extra: Option<Box<Vec<char>>>,
    /// Cell rendering attributes
    pub attrs: CellAttrs,
    /// Foreground color
    pub fg: CellColor,
    /// Background color
    pub bg: CellColor,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            extra: None,
            attrs: CellAttrs::default(),
            fg: CellColor::Default,
            bg: CellColor::Default,
        }
    }
}

impl Cell {
    /// Create a new cell with the given character
    pub fn new(c: char) -> Self {
        Self {
            c,
            ..Default::default()
        }
    }

    /// Reset cell to default (space with default colors)
    pub fn reset(&mut self) {
        self.c = ' ';
        self.extra = None;
        self.attrs = CellAttrs::default();
        self.fg = CellColor::Default;
        self.bg = CellColor::Default;
    }

    /// Check if this cell is empty (space with default attrs)
    pub fn is_empty(&self) -> bool {
        self.c == ' ' && self.extra.is_none() && self.fg == CellColor::Default && self.bg == CellColor::Default
    }

    /// Get the width of this cell (1 for normal, 2 for wide chars)
    pub fn width(&self) -> usize {
        use unicode_width::UnicodeWidthChar;
        self.c.width().unwrap_or(1)
    }
}

/// Cell color representation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellColor {
    /// Default foreground/background
    Default,
    /// Indexed color (0-255)
    /// 0-7: Standard colors
    /// 8-15: Bright colors
    /// 16-231: 6x6x6 color cube
    /// 232-255: Grayscale
    Indexed(u8),
    /// True color RGB
    Rgb(u8, u8, u8),
}

/// Cell rendering attributes
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellAttrs {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: UnderlineStyle,
    pub blink: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub strikethrough: bool,
    /// Hyperlink ID for OSC 8 (0 = no hyperlink)
    pub hyperlink_id: u16,
}

impl CellAttrs {
    /// Reset all attributes to default
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Underline rendering style
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cell_default() {
        let cell = Cell::default();
        assert_eq!(cell.c, ' ');
        assert!(cell.is_empty());
    }

    #[test]
    fn test_cell_reset() {
        let mut cell = Cell::new('A');
        cell.fg = CellColor::Indexed(1);
        cell.attrs.bold = true;
        cell.reset();
        assert!(cell.is_empty());
    }
}
