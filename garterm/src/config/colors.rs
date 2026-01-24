//! Terminal color palette configuration
//!
//! Provides 16 ANSI colors plus foreground, background, cursor, and selection colors.
//! Includes popular preset themes for power users.

use serde::{Deserialize, Serialize};

/// Parse a hex color string like "#1a1b26" or "1a1b26" into RGB components
pub fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Convert RGB to u32 for X11/rendering
pub fn rgb_to_u32(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// A color that can be specified as hex string or RGB tuple
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Color {
    Hex(String),
    Rgb { r: u8, g: u8, b: u8 },
}

impl Color {
    pub fn hex(s: impl Into<String>) -> Self {
        Self::Hex(s.into())
    }

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::Rgb { r, g, b }
    }

    /// Convert to RGB tuple
    pub fn to_rgb(&self) -> (u8, u8, u8) {
        match self {
            Color::Hex(s) => parse_hex_color(s).unwrap_or((0, 0, 0)),
            Color::Rgb { r, g, b } => (*r, *g, *b),
        }
    }

    /// Convert to u32 for X11/rendering
    pub fn to_u32(&self) -> u32 {
        let (r, g, b) = self.to_rgb();
        rgb_to_u32(r, g, b)
    }

    /// Convert to linear RGB for wgpu (sRGB to linear conversion)
    pub fn to_linear(&self) -> (f64, f64, f64) {
        let (r, g, b) = self.to_rgb();
        let to_linear = |c: u8| {
            let c = c as f64 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        (to_linear(r), to_linear(g), to_linear(b))
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::Hex("#ffffff".into())
    }
}

/// Terminal color palette with 16 ANSI colors and UI colors
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorPalette {
    // UI colors
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
    pub cursor_text: Color,
    pub selection: Color,
    pub selection_text: Color,

    // Standard ANSI colors (0-7)
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub white: Color,

    // Bright ANSI colors (8-15)
    pub bright_black: Color,
    pub bright_red: Color,
    pub bright_green: Color,
    pub bright_yellow: Color,
    pub bright_blue: Color,
    pub bright_magenta: Color,
    pub bright_cyan: Color,
    pub bright_white: Color,
}

impl ColorPalette {
    /// Get ANSI color by index (0-15)
    pub fn ansi(&self, index: u8) -> &Color {
        match index {
            0 => &self.black,
            1 => &self.red,
            2 => &self.green,
            3 => &self.yellow,
            4 => &self.blue,
            5 => &self.magenta,
            6 => &self.cyan,
            7 => &self.white,
            8 => &self.bright_black,
            9 => &self.bright_red,
            10 => &self.bright_green,
            11 => &self.bright_yellow,
            12 => &self.bright_blue,
            13 => &self.bright_magenta,
            14 => &self.bright_cyan,
            15 => &self.bright_white,
            _ => &self.foreground,
        }
    }

    /// Tokyo Night theme (default)
    pub fn tokyo_night() -> Self {
        Self {
            foreground: Color::hex("#c0caf5"),
            background: Color::hex("#1a1b26"),
            cursor: Color::hex("#c0caf5"),
            cursor_text: Color::hex("#1a1b26"),
            selection: Color::hex("#33467c"),
            selection_text: Color::hex("#c0caf5"),

            black: Color::hex("#15161e"),
            red: Color::hex("#f7768e"),
            green: Color::hex("#9ece6a"),
            yellow: Color::hex("#e0af68"),
            blue: Color::hex("#7aa2f7"),
            magenta: Color::hex("#bb9af7"),
            cyan: Color::hex("#7dcfff"),
            white: Color::hex("#a9b1d6"),

            bright_black: Color::hex("#414868"),
            bright_red: Color::hex("#f7768e"),
            bright_green: Color::hex("#9ece6a"),
            bright_yellow: Color::hex("#e0af68"),
            bright_blue: Color::hex("#7aa2f7"),
            bright_magenta: Color::hex("#bb9af7"),
            bright_cyan: Color::hex("#7dcfff"),
            bright_white: Color::hex("#c0caf5"),
        }
    }

    /// Catppuccin Mocha theme
    pub fn catppuccin_mocha() -> Self {
        Self {
            foreground: Color::hex("#cdd6f4"),
            background: Color::hex("#1e1e2e"),
            cursor: Color::hex("#f5e0dc"),
            cursor_text: Color::hex("#1e1e2e"),
            selection: Color::hex("#45475a"),
            selection_text: Color::hex("#cdd6f4"),

            black: Color::hex("#45475a"),
            red: Color::hex("#f38ba8"),
            green: Color::hex("#a6e3a1"),
            yellow: Color::hex("#f9e2af"),
            blue: Color::hex("#89b4fa"),
            magenta: Color::hex("#f5c2e7"),
            cyan: Color::hex("#94e2d5"),
            white: Color::hex("#bac2de"),

            bright_black: Color::hex("#585b70"),
            bright_red: Color::hex("#f38ba8"),
            bright_green: Color::hex("#a6e3a1"),
            bright_yellow: Color::hex("#f9e2af"),
            bright_blue: Color::hex("#89b4fa"),
            bright_magenta: Color::hex("#f5c2e7"),
            bright_cyan: Color::hex("#94e2d5"),
            bright_white: Color::hex("#a6adc8"),
        }
    }

    /// Gruvbox Dark theme
    pub fn gruvbox_dark() -> Self {
        Self {
            foreground: Color::hex("#ebdbb2"),
            background: Color::hex("#282828"),
            cursor: Color::hex("#ebdbb2"),
            cursor_text: Color::hex("#282828"),
            selection: Color::hex("#504945"),
            selection_text: Color::hex("#ebdbb2"),

            black: Color::hex("#282828"),
            red: Color::hex("#cc241d"),
            green: Color::hex("#98971a"),
            yellow: Color::hex("#d79921"),
            blue: Color::hex("#458588"),
            magenta: Color::hex("#b16286"),
            cyan: Color::hex("#689d6a"),
            white: Color::hex("#a89984"),

            bright_black: Color::hex("#928374"),
            bright_red: Color::hex("#fb4934"),
            bright_green: Color::hex("#b8bb26"),
            bright_yellow: Color::hex("#fabd2f"),
            bright_blue: Color::hex("#83a598"),
            bright_magenta: Color::hex("#d3869b"),
            bright_cyan: Color::hex("#8ec07c"),
            bright_white: Color::hex("#ebdbb2"),
        }
    }

    /// Dracula theme
    pub fn dracula() -> Self {
        Self {
            foreground: Color::hex("#f8f8f2"),
            background: Color::hex("#282a36"),
            cursor: Color::hex("#f8f8f2"),
            cursor_text: Color::hex("#282a36"),
            selection: Color::hex("#44475a"),
            selection_text: Color::hex("#f8f8f2"),

            black: Color::hex("#21222c"),
            red: Color::hex("#ff5555"),
            green: Color::hex("#50fa7b"),
            yellow: Color::hex("#f1fa8c"),
            blue: Color::hex("#bd93f9"),
            magenta: Color::hex("#ff79c6"),
            cyan: Color::hex("#8be9fd"),
            white: Color::hex("#f8f8f2"),

            bright_black: Color::hex("#6272a4"),
            bright_red: Color::hex("#ff6e6e"),
            bright_green: Color::hex("#69ff94"),
            bright_yellow: Color::hex("#ffffa5"),
            bright_blue: Color::hex("#d6acff"),
            bright_magenta: Color::hex("#ff92df"),
            bright_cyan: Color::hex("#a4ffff"),
            bright_white: Color::hex("#ffffff"),
        }
    }

    /// Nord theme
    pub fn nord() -> Self {
        Self {
            foreground: Color::hex("#d8dee9"),
            background: Color::hex("#2e3440"),
            cursor: Color::hex("#d8dee9"),
            cursor_text: Color::hex("#2e3440"),
            selection: Color::hex("#434c5e"),
            selection_text: Color::hex("#d8dee9"),

            black: Color::hex("#3b4252"),
            red: Color::hex("#bf616a"),
            green: Color::hex("#a3be8c"),
            yellow: Color::hex("#ebcb8b"),
            blue: Color::hex("#81a1c1"),
            magenta: Color::hex("#b48ead"),
            cyan: Color::hex("#88c0d0"),
            white: Color::hex("#e5e9f0"),

            bright_black: Color::hex("#4c566a"),
            bright_red: Color::hex("#bf616a"),
            bright_green: Color::hex("#a3be8c"),
            bright_yellow: Color::hex("#ebcb8b"),
            bright_blue: Color::hex("#81a1c1"),
            bright_magenta: Color::hex("#b48ead"),
            bright_cyan: Color::hex("#8fbcbb"),
            bright_white: Color::hex("#eceff4"),
        }
    }

    /// Solarized Dark theme
    pub fn solarized_dark() -> Self {
        Self {
            foreground: Color::hex("#839496"),
            background: Color::hex("#002b36"),
            cursor: Color::hex("#839496"),
            cursor_text: Color::hex("#002b36"),
            selection: Color::hex("#073642"),
            selection_text: Color::hex("#93a1a1"),

            black: Color::hex("#073642"),
            red: Color::hex("#dc322f"),
            green: Color::hex("#859900"),
            yellow: Color::hex("#b58900"),
            blue: Color::hex("#268bd2"),
            magenta: Color::hex("#d33682"),
            cyan: Color::hex("#2aa198"),
            white: Color::hex("#eee8d5"),

            bright_black: Color::hex("#002b36"),
            bright_red: Color::hex("#cb4b16"),
            bright_green: Color::hex("#586e75"),
            bright_yellow: Color::hex("#657b83"),
            bright_blue: Color::hex("#839496"),
            bright_magenta: Color::hex("#6c71c4"),
            bright_cyan: Color::hex("#93a1a1"),
            bright_white: Color::hex("#fdf6e3"),
        }
    }

    /// One Dark theme (Atom)
    pub fn one_dark() -> Self {
        Self {
            foreground: Color::hex("#abb2bf"),
            background: Color::hex("#282c34"),
            cursor: Color::hex("#528bff"),
            cursor_text: Color::hex("#282c34"),
            selection: Color::hex("#3e4451"),
            selection_text: Color::hex("#abb2bf"),

            black: Color::hex("#282c34"),
            red: Color::hex("#e06c75"),
            green: Color::hex("#98c379"),
            yellow: Color::hex("#e5c07b"),
            blue: Color::hex("#61afef"),
            magenta: Color::hex("#c678dd"),
            cyan: Color::hex("#56b6c2"),
            white: Color::hex("#abb2bf"),

            bright_black: Color::hex("#5c6370"),
            bright_red: Color::hex("#e06c75"),
            bright_green: Color::hex("#98c379"),
            bright_yellow: Color::hex("#e5c07b"),
            bright_blue: Color::hex("#61afef"),
            bright_magenta: Color::hex("#c678dd"),
            bright_cyan: Color::hex("#56b6c2"),
            bright_white: Color::hex("#ffffff"),
        }
    }

    /// Get a preset by name
    pub fn from_preset(name: &str) -> Option<Self> {
        match name.to_lowercase().replace(['-', '_'], "").as_str() {
            "tokyonight" => Some(Self::tokyo_night()),
            "catppuccinmocha" | "catppuccin" => Some(Self::catppuccin_mocha()),
            "gruvboxdark" | "gruvbox" => Some(Self::gruvbox_dark()),
            "dracula" => Some(Self::dracula()),
            "nord" => Some(Self::nord()),
            "solarizeddark" | "solarized" => Some(Self::solarized_dark()),
            "onedark" | "atom" => Some(Self::one_dark()),
            _ => None,
        }
    }

    /// List available preset names
    pub fn preset_names() -> &'static [&'static str] {
        &[
            "tokyo-night",
            "catppuccin-mocha",
            "gruvbox-dark",
            "dracula",
            "nord",
            "solarized-dark",
            "one-dark",
        ]
    }
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self::tokyo_night()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#1a1b26"), Some((0x1a, 0x1b, 0x26)));
        assert_eq!(parse_hex_color("1a1b26"), Some((0x1a, 0x1b, 0x26)));
        assert_eq!(parse_hex_color("#ffffff"), Some((255, 255, 255)));
        assert_eq!(parse_hex_color("invalid"), None);
    }

    #[test]
    fn test_color_to_linear() {
        let white = Color::hex("#ffffff");
        let (r, g, b) = white.to_linear();
        assert!((r - 1.0).abs() < 0.001);
        assert!((g - 1.0).abs() < 0.001);
        assert!((b - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_preset_lookup() {
        assert!(ColorPalette::from_preset("tokyo-night").is_some());
        assert!(ColorPalette::from_preset("TokyoNight").is_some());
        assert!(ColorPalette::from_preset("catppuccin").is_some());
        assert!(ColorPalette::from_preset("nonexistent").is_none());
    }
}
