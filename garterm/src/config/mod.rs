//! Terminal configuration system
//!
//! Supports TOML (primary) and Lua (gar integration) configuration with sensible defaults.
//!
//! # Configuration Locations
//!
//! 1. CLI flags (highest priority)
//! 2. `~/.config/garterm/config.toml` (primary config file)
//! 3. `gar.terminal` table in `~/.config/gar/init.lua` (gar suite integration)
//! 4. Built-in defaults
//!
//! # Example TOML
//!
//! ```toml
//! [general]
//! shell = "/bin/zsh"
//! vsync = false
//!
//! [font]
//! family = "JetBrains Mono"
//! size = 12.0
//!
//! [colors]
//! preset = "tokyo-night"
//!
//! [keybinds]
//! "ctrl+shift+t" = "new_tab"
//! ```

pub mod colors;
pub mod keybinds;

pub use colors::{Color, ColorPalette};
pub use keybinds::{Action, Keybind, KeybindSet, Modifiers};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main terminal configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub font: FontConfig,
    pub window: WindowConfig,
    pub terminal: TerminalConfig,
    pub colors: ColorsConfig,
    pub mouse: MouseConfig,
    pub bell: BellConfig,

    #[serde(default)]
    pub keybinds: keybinds::KeybindConfig,
}

/// General settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Shell to execute (default: $SHELL or /bin/sh)
    pub shell: String,

    /// Arguments to pass to shell
    #[serde(default)]
    pub shell_args: Vec<String>,

    /// Working directory (default: inherit from parent)
    pub working_directory: Option<PathBuf>,

    /// Use VSync-based rendering (may not work on Asahi Linux)
    pub vsync: bool,

    /// Log level: "error", "warn", "info", "debug", "trace"
    pub log_level: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
            shell_args: vec![],
            working_directory: None,
            vsync: false,
            log_level: "info".into(),
        }
    }
}

/// Font configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    /// Primary font family
    pub family: String,

    /// Font size in points
    pub size: f32,

    /// Bold font family (default: same as family)
    pub bold_family: Option<String>,

    /// Italic font family (default: same as family)
    pub italic_family: Option<String>,

    /// Use bold font for bright colors
    pub bold_is_bright: bool,

    /// Enable font ligatures (requires swash, future feature)
    pub ligatures: bool,

    /// Extra spacing between characters (pixels)
    pub letter_spacing: f32,

    /// Extra spacing between lines (pixels)
    pub line_spacing: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "monospace".into(),
            size: 14.0,
            bold_family: None,
            italic_family: None,
            bold_is_bright: false,
            ligatures: false,
            letter_spacing: 0.0,
            line_spacing: 0.0,
        }
    }
}

/// Window configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    /// Window title (supports OSC 0/2 escape sequences to override)
    pub title: String,

    /// Window class for WM rules
    pub class: String,

    /// Padding inside window (horizontal, vertical) in pixels
    pub padding: (u32, u32),

    /// Initial window size in columns x rows (0 = auto)
    pub columns: u32,
    pub rows: u32,

    /// Window opacity (0.0 - 1.0, requires compositor)
    pub opacity: f32,

    /// Start in fullscreen mode
    pub fullscreen: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "garterm".into(),
            class: "garterm".into(),
            padding: (0, 0),
            columns: 80,
            rows: 24,
            opacity: 1.0,
            fullscreen: false,
        }
    }
}

/// Terminal behavior configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// Maximum scrollback lines
    pub scrollback_lines: usize,

    /// Lines to scroll per wheel tick
    pub scroll_lines: usize,

    /// Enable bracketed paste mode
    pub bracketed_paste: bool,

    /// Allow terminal to modify clipboard (OSC 52)
    pub clipboard_write: bool,

    /// Allow terminal to read clipboard (OSC 52) - security consideration
    pub clipboard_read: bool,

    /// Close pane when shell exits
    pub close_on_exit: bool,

    /// Hold pane open on exit (show exit code)
    pub hold_on_exit: bool,

    /// TERM environment variable
    pub term: String,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: 10000,
            scroll_lines: 3,
            bracketed_paste: true,
            clipboard_write: true,
            clipboard_read: false,  // Security: disabled by default
            close_on_exit: true,
            hold_on_exit: false,
            term: "xterm-256color".into(),
        }
    }
}

/// Color configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorsConfig {
    /// Use a preset theme: "tokyo-night", "catppuccin", "gruvbox", "dracula", "nord", etc.
    pub preset: Option<String>,

    /// Custom color overrides (applied on top of preset)
    #[serde(flatten)]
    pub palette: ColorPalette,
}

impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            preset: Some("tokyo-night".into()),
            palette: ColorPalette::default(),
        }
    }
}

impl ColorsConfig {
    /// Resolve to final ColorPalette
    pub fn resolve(&self) -> ColorPalette {
        if let Some(preset_name) = &self.preset {
            if let Some(preset) = ColorPalette::from_preset(preset_name) {
                // TODO: merge self.palette overrides on top of preset
                return preset;
            }
            tracing::warn!("Unknown color preset '{}', using default", preset_name);
        }
        self.palette.clone()
    }
}

/// Mouse configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MouseConfig {
    /// Copy to PRIMARY selection automatically when selecting text
    pub copy_on_select: bool,

    /// Paste from PRIMARY on middle click
    pub middle_click_paste: bool,

    /// Right click action: "paste", "context_menu", "none"
    pub right_click: String,

    /// Double-click word characters (in addition to alphanumeric)
    pub word_chars: String,

    /// Enable URL detection and click-to-open
    pub url_detection: bool,

    /// Modifier to require for URL clicks (e.g., "ctrl")
    pub url_modifier: Option<String>,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            copy_on_select: true,
            middle_click_paste: true,
            right_click: "paste".into(),
            word_chars: "-_".into(),
            url_detection: true,
            url_modifier: Some("ctrl".into()),
        }
    }
}

/// Bell configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BellConfig {
    /// Enable visual bell (flash screen)
    pub visual: bool,

    /// Visual bell duration in milliseconds
    pub visual_duration_ms: u32,

    /// Enable audio bell
    pub audio: bool,

    /// Audio bell command (e.g., "paplay /usr/share/sounds/bell.ogg")
    pub audio_command: Option<String>,
}

impl Default for BellConfig {
    fn default() -> Self {
        Self {
            visual: true,
            visual_duration_ms: 100,
            audio: false,
            audio_command: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            font: FontConfig::default(),
            window: WindowConfig::default(),
            terminal: TerminalConfig::default(),
            colors: ColorsConfig::default(),
            mouse: MouseConfig::default(),
            bell: BellConfig::default(),
            keybinds: keybinds::KeybindConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration with fallback chain
    pub fn load() -> Self {
        ConfigLoader::new().load()
    }

    /// Load from a specific TOML file
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
        toml::from_str(&content)
            .map_err(|e| ConfigError::Parse(path.to_path_buf(), e.to_string()))
    }

    /// Get resolved color palette
    pub fn color_palette(&self) -> ColorPalette {
        self.colors.resolve()
    }

    /// Get resolved keybindings
    pub fn keybindings(&self) -> KeybindSet {
        let (set, warnings) = self.keybinds.to_keybind_set();
        for warning in warnings {
            tracing::warn!("Keybind config: {}", warning);
        }
        set
    }

    // Builder methods for CLI overrides
    pub fn with_shell(mut self, shell: String) -> Self {
        self.general.shell = shell;
        self
    }

    pub fn with_working_directory(mut self, path: Option<PathBuf>) -> Self {
        self.general.working_directory = path;
        self
    }

    pub fn with_vsync(mut self, vsync: bool) -> Self {
        self.general.vsync = vsync;
        self
    }

    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font.size = size;
        self
    }
}

/// Configuration loading errors
#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(path, e) => write!(f, "Failed to read {}: {}", path.display(), e),
            ConfigError::Parse(path, e) => write!(f, "Failed to parse {}: {}", path.display(), e),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Configuration loader with fallback chain
pub struct ConfigLoader {
    toml_path: PathBuf,
    lua_path: PathBuf,
}

impl ConfigLoader {
    pub fn new() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"));

        Self {
            toml_path: config_dir.join("garterm/config.toml"),
            lua_path: config_dir.join("gar/init.lua"),
        }
    }

    pub fn with_toml_path(mut self, path: PathBuf) -> Self {
        self.toml_path = path;
        self
    }

    /// Load configuration with fallback chain
    pub fn load(&self) -> Config {
        // Try TOML first
        if self.toml_path.exists() {
            match Config::load_from_file(&self.toml_path) {
                Ok(config) => {
                    tracing::info!("Loaded config from {}", self.toml_path.display());
                    return config;
                }
                Err(e) => {
                    tracing::error!("Config error: {}", e);
                    // Fall through to defaults
                }
            }
        }

        // Try Lua (gar.terminal table) - TODO: implement in lua.rs
        if self.lua_path.exists() {
            if let Some(config) = self.try_load_lua() {
                tracing::info!("Loaded config from gar.terminal in {}", self.lua_path.display());
                return config;
            }
        }

        // Use defaults
        tracing::info!("Using default configuration");
        Config::default()
    }

    fn try_load_lua(&self) -> Option<Config> {
        // TODO: Implement Lua loading in lua.rs
        // For now, return None to fall through to defaults
        None
    }
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.font.size, 14.0);
        assert_eq!(config.terminal.scrollback_lines, 10000);
        assert!(!config.general.vsync);
    }

    #[test]
    fn test_parse_toml() {
        let toml = r#"
            [general]
            shell = "/bin/zsh"
            vsync = true

            [font]
            family = "Fira Code"
            size = 12.0

            [colors]
            preset = "dracula"
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.general.shell, "/bin/zsh");
        assert!(config.general.vsync);
        assert_eq!(config.font.family, "Fira Code");
        assert_eq!(config.font.size, 12.0);
        assert_eq!(config.colors.preset, Some("dracula".into()));
    }

    #[test]
    fn test_color_preset_resolve() {
        let colors = ColorsConfig {
            preset: Some("gruvbox".into()),
            ..Default::default()
        };
        let palette = colors.resolve();
        // Gruvbox background is #282828
        assert_eq!(palette.background.to_u32(), 0x282828);
    }
}
