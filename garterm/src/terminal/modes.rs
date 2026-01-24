/// Terminal modes (DEC private modes and ANSI modes)
#[derive(Debug, Clone, Default)]
pub struct TerminalModes {
    // DEC Private Modes
    /// DECCKM - Application cursor keys
    pub application_cursor: bool,
    /// DECNKM - Application keypad
    pub application_keypad: bool,
    /// DECAWM - Auto-wrap mode
    pub autowrap: bool,
    /// DECOM - Origin mode (cursor relative to scroll region)
    pub origin: bool,
    /// DECTCEM - Text cursor enable (visibility)
    pub cursor_visible: bool,
    /// DECSCNM - Screen mode (reverse video)
    pub reverse_video: bool,
    /// DECBKM - Backarrow key mode
    pub backarrow_is_backspace: bool,

    // ANSI Modes
    /// IRM - Insert/Replace mode
    pub insert: bool,
    /// LNM - Linefeed/Newline mode
    pub linefeed: bool,

    // xterm Extensions
    /// Bracketed paste mode
    pub bracketed_paste: bool,
    /// Focus event reporting
    pub focus_events: bool,
    /// Alternate screen buffer active
    pub alt_screen: bool,

    // Mouse modes
    pub mouse_mode: MouseMode,
    pub mouse_encoding: MouseEncoding,
}

impl TerminalModes {
    pub fn new() -> Self {
        Self {
            autowrap: true,
            cursor_visible: true,
            ..Default::default()
        }
    }

    /// Reset to default modes
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Set a DEC private mode
    pub fn set_dec_mode(&mut self, mode: u16, value: bool) {
        match mode {
            1 => self.application_cursor = value,
            3 => {} // DECCOLM - 132 column mode (ignored, we handle resize differently)
            5 => self.reverse_video = value,
            6 => self.origin = value,
            7 => self.autowrap = value,
            12 => {} // Cursor blink - handled elsewhere
            25 => self.cursor_visible = value,
            47 => self.alt_screen = value,
            66 => self.application_keypad = value,
            67 => self.backarrow_is_backspace = value,
            1000 => self.mouse_mode = if value { MouseMode::X10 } else { MouseMode::None },
            1002 => self.mouse_mode = if value { MouseMode::ButtonEvent } else { MouseMode::None },
            1003 => self.mouse_mode = if value { MouseMode::AnyEvent } else { MouseMode::None },
            1004 => self.focus_events = value,
            1005 => self.mouse_encoding = if value { MouseEncoding::Utf8 } else { MouseEncoding::X10 },
            1006 => self.mouse_encoding = if value { MouseEncoding::Sgr } else { MouseEncoding::X10 },
            1015 => self.mouse_encoding = if value { MouseEncoding::Urxvt } else { MouseEncoding::X10 },
            1047 => self.alt_screen = value,
            1048 => {} // Save/restore cursor - handled by terminal
            1049 => self.alt_screen = value, // Combined alt screen + save cursor
            2004 => self.bracketed_paste = value,
            _ => {} // Unknown mode, ignore
        }
    }

    /// Set an ANSI mode
    pub fn set_ansi_mode(&mut self, mode: u16, value: bool) {
        match mode {
            4 => self.insert = value,
            20 => self.linefeed = value,
            _ => {} // Unknown mode, ignore
        }
    }
}

/// Mouse tracking mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MouseMode {
    #[default]
    None,
    /// X10 compatibility mode - report button press only
    X10,
    /// VT200 mode - report button press and release
    Vt200,
    /// Button-event tracking - report motion while button pressed
    ButtonEvent,
    /// Any-event tracking - report all motion
    AnyEvent,
}

/// Mouse coordinate encoding
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MouseEncoding {
    /// X10 encoding - limited to 223 columns/rows
    #[default]
    X10,
    /// UTF-8 encoding - extended range
    Utf8,
    /// SGR encoding - modern, preferred
    Sgr,
    /// urxvt encoding
    Urxvt,
}
