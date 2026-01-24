//! Keybinding configuration for terminal actions
//!
//! Power users can customize every aspect of keyboard interaction.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

/// Keyboard modifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
}

impl Modifiers {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn ctrl() -> Self {
        Self { ctrl: true, ..Default::default() }
    }

    pub fn alt() -> Self {
        Self { alt: true, ..Default::default() }
    }

    pub fn shift() -> Self {
        Self { shift: true, ..Default::default() }
    }

    pub fn ctrl_shift() -> Self {
        Self { ctrl: true, shift: true, ..Default::default() }
    }

    pub fn ctrl_alt() -> Self {
        Self { ctrl: true, alt: true, ..Default::default() }
    }
}

impl std::fmt::Display for Modifiers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl { parts.push("ctrl"); }
        if self.alt { parts.push("alt"); }
        if self.shift { parts.push("shift"); }
        if self.super_key { parts.push("super"); }
        write!(f, "{}", parts.join("+"))
    }
}

/// Terminal actions that can be bound to keys
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    // Clipboard
    Copy,
    Paste,
    PastePrimary,

    // Tabs (future)
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    Tab(usize),  // Switch to specific tab (1-indexed)

    // Panes (future)
    SplitHorizontal,
    SplitVertical,
    ClosePane,
    FocusUp,
    FocusDown,
    FocusLeft,
    FocusRight,
    ResizeUp(u32),
    ResizeDown(u32),
    ResizeLeft(u32),
    ResizeRight(u32),

    // Scrollback
    ScrollUp(usize),
    ScrollDown(usize),
    ScrollPageUp,
    ScrollPageDown,
    ScrollToTop,
    ScrollToBottom,

    // Font
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,

    // Search (future)
    SearchForward,
    SearchBackward,

    // Misc
    ReloadConfig,
    ToggleFullscreen,
    ResetTerminal,
    ClearScrollback,

    // Send raw bytes to terminal
    SendBytes(Vec<u8>),
    SendText(String),

    // No action (for documenting disabled defaults)
    None,
}

impl Action {
    /// Parse action from string (for TOML config)
    pub fn from_str_loose(s: &str) -> Option<Self> {
        // Handle parameterized actions
        if let Some(n) = s.strip_prefix("tab_") {
            return n.parse().ok().map(Action::Tab);
        }
        if let Some(n) = s.strip_prefix("scroll_up_") {
            return n.parse().ok().map(Action::ScrollUp);
        }
        if let Some(n) = s.strip_prefix("scroll_down_") {
            return n.parse().ok().map(Action::ScrollDown);
        }

        match s.to_lowercase().replace(['-', '_'], "").as_str() {
            "copy" => Some(Action::Copy),
            "paste" => Some(Action::Paste),
            "pasteprimary" => Some(Action::PastePrimary),
            "newtab" => Some(Action::NewTab),
            "closetab" => Some(Action::CloseTab),
            "nexttab" => Some(Action::NextTab),
            "prevtab" => Some(Action::PrevTab),
            "splithorizontal" | "hsplit" => Some(Action::SplitHorizontal),
            "splitvertical" | "vsplit" => Some(Action::SplitVertical),
            "closepane" => Some(Action::ClosePane),
            "focusup" => Some(Action::FocusUp),
            "focusdown" => Some(Action::FocusDown),
            "focusleft" => Some(Action::FocusLeft),
            "focusright" => Some(Action::FocusRight),
            "scrollup" => Some(Action::ScrollUp(3)),
            "scrolldown" => Some(Action::ScrollDown(3)),
            "scrollpageup" | "pageup" => Some(Action::ScrollPageUp),
            "scrollpagedown" | "pagedown" => Some(Action::ScrollPageDown),
            "scrolltotop" | "top" => Some(Action::ScrollToTop),
            "scrolltobottom" | "bottom" => Some(Action::ScrollToBottom),
            "increasefontsize" | "zoomin" | "fontup" => Some(Action::IncreaseFontSize),
            "decreasefontsize" | "zoomout" | "fontdown" => Some(Action::DecreaseFontSize),
            "resetfontsize" | "zoomreset" | "fontreset" => Some(Action::ResetFontSize),
            "searchforward" | "search" => Some(Action::SearchForward),
            "searchbackward" => Some(Action::SearchBackward),
            "reloadconfig" | "reload" => Some(Action::ReloadConfig),
            "togglefullscreen" | "fullscreen" => Some(Action::ToggleFullscreen),
            "resetterminal" | "reset" => Some(Action::ResetTerminal),
            "clearscrollback" | "clear" => Some(Action::ClearScrollback),
            "none" | "noop" | "disabled" => Some(Action::None),
            _ => None,
        }
    }
}

/// A keybinding mapping a key combo to an action
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybind {
    pub modifiers: Modifiers,
    pub key: String,  // Key name like "t", "Return", "Up"
    pub action: Action,
}

impl Keybind {
    pub fn new(modifiers: Modifiers, key: impl Into<String>, action: Action) -> Self {
        Self {
            modifiers,
            key: key.into(),
            action,
        }
    }

    /// Parse a keybind string like "ctrl+shift+t"
    pub fn parse_key_combo(s: &str) -> Option<(Modifiers, String)> {
        let parts: Vec<&str> = s.split('+').collect();
        if parts.is_empty() {
            return None;
        }

        let mut modifiers = Modifiers::default();
        let mut key = None;

        for part in parts {
            match part.to_lowercase().as_str() {
                "ctrl" | "control" => modifiers.ctrl = true,
                "alt" | "meta" | "mod1" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                "super" | "mod4" | "win" | "cmd" => modifiers.super_key = true,
                _ => {
                    // Last non-modifier part is the key
                    key = Some(part.to_string());
                }
            }
        }

        key.map(|k| (modifiers, k))
    }
}

/// Collection of keybindings with conflict detection
#[derive(Debug, Clone, Default)]
pub struct KeybindSet {
    binds: Vec<Keybind>,
    /// Map for fast lookup: (modifiers_bits, key) -> index
    lookup: HashMap<(u8, String), usize>,
}

impl KeybindSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a keybind, returning any conflicting keybind that was replaced
    pub fn add(&mut self, bind: Keybind) -> Option<Keybind> {
        let key = (self.modifiers_to_bits(&bind.modifiers), bind.key.to_lowercase());

        if let Some(&idx) = self.lookup.get(&key) {
            // Replace existing
            let old = std::mem::replace(&mut self.binds[idx], bind);
            Some(old)
        } else {
            // Add new
            let idx = self.binds.len();
            self.lookup.insert(key, idx);
            self.binds.push(bind);
            None
        }
    }

    /// Look up action for a key combo
    pub fn get(&self, modifiers: &Modifiers, key: &str) -> Option<&Action> {
        let lookup_key = (self.modifiers_to_bits(modifiers), key.to_lowercase());
        self.lookup.get(&lookup_key)
            .map(|&idx| &self.binds[idx].action)
    }

    /// Get all keybinds
    pub fn iter(&self) -> impl Iterator<Item = &Keybind> {
        self.binds.iter()
    }

    fn modifiers_to_bits(&self, m: &Modifiers) -> u8 {
        let mut bits = 0u8;
        if m.ctrl { bits |= 1; }
        if m.alt { bits |= 2; }
        if m.shift { bits |= 4; }
        if m.super_key { bits |= 8; }
        bits
    }

    /// Create default keybindings
    /// Uses Alt+key for tabs/panes to avoid conflicts with window managers
    pub fn defaults() -> Self {
        let mut set = Self::new();

        // Clipboard (Ctrl+Shift is standard terminal convention)
        set.add(Keybind::new(Modifiers::ctrl_shift(), "c", Action::Copy));
        set.add(Keybind::new(Modifiers::ctrl_shift(), "v", Action::Paste));

        // Tabs (Alt+key to avoid WM conflicts)
        set.add(Keybind::new(Modifiers::alt(), "t", Action::NewTab));
        set.add(Keybind::new(Modifiers::alt(), "w", Action::ClosePane));

        // Splits (Alt+key)
        set.add(Keybind::new(Modifiers::alt(), "h", Action::SplitHorizontal));
        set.add(Keybind::new(Modifiers::alt(), "v", Action::SplitVertical));

        // Focus navigation (Alt+Arrow)
        set.add(Keybind::new(Modifiers::alt(), "up", Action::FocusUp));
        set.add(Keybind::new(Modifiers::alt(), "down", Action::FocusDown));
        set.add(Keybind::new(Modifiers::alt(), "left", Action::FocusLeft));
        set.add(Keybind::new(Modifiers::alt(), "right", Action::FocusRight));

        // Scrollback
        set.add(Keybind::new(Modifiers::shift(), "page_up", Action::ScrollPageUp));
        set.add(Keybind::new(Modifiers::shift(), "page_down", Action::ScrollPageDown));
        set.add(Keybind::new(Modifiers::ctrl_shift(), "home", Action::ScrollToTop));
        set.add(Keybind::new(Modifiers::ctrl_shift(), "end", Action::ScrollToBottom));

        // Font size (Ctrl+Shift standard)
        set.add(Keybind::new(Modifiers::ctrl_shift(), "equal", Action::IncreaseFontSize));
        set.add(Keybind::new(Modifiers::ctrl_shift(), "plus", Action::IncreaseFontSize));
        set.add(Keybind::new(Modifiers::ctrl_shift(), "minus", Action::DecreaseFontSize));
        set.add(Keybind::new(Modifiers::ctrl_shift(), "0", Action::ResetFontSize));

        // Search
        set.add(Keybind::new(Modifiers::ctrl_shift(), "f", Action::SearchForward));

        // Misc
        set.add(Keybind::new(Modifiers::ctrl_shift(), "r", Action::ReloadConfig));

        set
    }
}

/// Serde helper for keybinds in TOML
/// Allows format like: `"ctrl+shift+t" = "new_tab"`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeybindConfig {
    #[serde(flatten)]
    pub bindings: HashMap<String, String>,
}

impl KeybindConfig {
    /// Convert to KeybindSet
    pub fn to_keybind_set(&self) -> (KeybindSet, Vec<String>) {
        let mut set = KeybindSet::defaults();
        let mut warnings = Vec::new();

        for (key_combo, action_str) in &self.bindings {
            // Parse key combo
            let Some((modifiers, key)) = Keybind::parse_key_combo(key_combo) else {
                warnings.push(format!("Invalid key combo: {}", key_combo));
                continue;
            };

            // Parse action
            let Some(action) = Action::from_str_loose(action_str) else {
                warnings.push(format!("Unknown action '{}' for key '{}'", action_str, key_combo));
                continue;
            };

            // Add/replace binding
            if let Some(old) = set.add(Keybind::new(modifiers, key, action)) {
                if old.action != Action::None {
                    tracing::debug!("Keybind {} overriding default {:?}", key_combo, old.action);
                }
            }
        }

        (set, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_combo() {
        let (mods, key) = Keybind::parse_key_combo("ctrl+shift+t").unwrap();
        assert!(mods.ctrl);
        assert!(mods.shift);
        assert!(!mods.alt);
        assert_eq!(key, "t");

        let (mods, key) = Keybind::parse_key_combo("alt+Return").unwrap();
        assert!(mods.alt);
        assert!(!mods.ctrl);
        assert_eq!(key, "Return");
    }

    #[test]
    fn test_action_parse() {
        assert_eq!(Action::from_str_loose("copy"), Some(Action::Copy));
        assert_eq!(Action::from_str_loose("new_tab"), Some(Action::NewTab));
        assert_eq!(Action::from_str_loose("NewTab"), Some(Action::NewTab));
        assert_eq!(Action::from_str_loose("zoom_in"), Some(Action::IncreaseFontSize));
    }

    #[test]
    fn test_keybind_set_lookup() {
        let set = KeybindSet::defaults();
        let action = set.get(&Modifiers::ctrl_shift(), "c");
        assert_eq!(action, Some(&Action::Copy));
    }
}
