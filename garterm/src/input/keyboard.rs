//! Keyboard input handling for terminal

use crate::terminal::TerminalModes;
use gartk_core::{Key, Modifiers};

/// Keyboard handler for translating X11 key events to terminal escape sequences
pub struct KeyboardHandler;

impl KeyboardHandler {
    /// Translate a key event to bytes for the PTY
    pub fn translate(key: Key, mods: &Modifiers, modes: &TerminalModes) -> Option<Vec<u8>> {
        let ctrl = mods.ctrl;
        let alt = mods.alt;
        let shift = mods.shift;

        let bytes = match key {
            // Basic control characters
            Key::Return => vec![b'\r'],
            Key::Tab => {
                if shift {
                    // Shift+Tab = reverse tab (CSI Z)
                    vec![0x1b, b'[', b'Z']
                } else {
                    vec![b'\t']
                }
            }
            Key::Escape => vec![0x1b],
            Key::Backspace => {
                if alt {
                    vec![0x1b, 0x7f]
                } else {
                    vec![0x7f]
                }
            }

            // Editing keys
            Key::Delete => Self::modified_key(b"3~", ctrl, alt, shift),
            Key::Insert => Self::modified_key(b"2~", ctrl, alt, shift),
            Key::Home => {
                if ctrl || alt || shift {
                    Self::modified_key(b"H", ctrl, alt, shift)
                } else if modes.application_cursor {
                    vec![0x1b, b'O', b'H']
                } else {
                    vec![0x1b, b'[', b'H']
                }
            }
            Key::End => {
                if ctrl || alt || shift {
                    Self::modified_key(b"F", ctrl, alt, shift)
                } else if modes.application_cursor {
                    vec![0x1b, b'O', b'F']
                } else {
                    vec![0x1b, b'[', b'F']
                }
            }
            Key::PageUp => Self::modified_key(b"5~", ctrl, alt, shift),
            Key::PageDown => Self::modified_key(b"6~", ctrl, alt, shift),

            // Arrow keys
            Key::Up => Self::arrow_key(b'A', ctrl, alt, shift, modes),
            Key::Down => Self::arrow_key(b'B', ctrl, alt, shift, modes),
            Key::Right => Self::arrow_key(b'C', ctrl, alt, shift, modes),
            Key::Left => Self::arrow_key(b'D', ctrl, alt, shift, modes),

            // Function keys
            Key::F1 => Self::function_key(1, ctrl, alt, shift),
            Key::F2 => Self::function_key(2, ctrl, alt, shift),
            Key::F3 => Self::function_key(3, ctrl, alt, shift),
            Key::F4 => Self::function_key(4, ctrl, alt, shift),
            Key::F5 => Self::function_key(5, ctrl, alt, shift),
            Key::F6 => Self::function_key(6, ctrl, alt, shift),
            Key::F7 => Self::function_key(7, ctrl, alt, shift),
            Key::F8 => Self::function_key(8, ctrl, alt, shift),
            Key::F9 => Self::function_key(9, ctrl, alt, shift),
            Key::F10 => Self::function_key(10, ctrl, alt, shift),
            Key::F11 => Self::function_key(11, ctrl, alt, shift),
            Key::F12 => Self::function_key(12, ctrl, alt, shift),

            // Space
            Key::Space => {
                if ctrl {
                    vec![0] // Ctrl+Space = NUL
                } else if alt {
                    vec![0x1b, b' ']
                } else {
                    vec![b' ']
                }
            }

            // Regular characters
            Key::Char(c) => Self::char_key(c, ctrl, alt),

            _ => return None,
        };

        Some(bytes)
    }

    /// Generate escape sequence for arrow keys with modifiers
    fn arrow_key(dir: u8, ctrl: bool, alt: bool, shift: bool, modes: &TerminalModes) -> Vec<u8> {
        let modifier = Self::modifier_code(ctrl, alt, shift);

        if modifier > 1 {
            // Modified arrow: CSI 1 ; modifier dir
            format!("\x1b[1;{}{}", modifier, dir as char).into_bytes()
        } else if modes.application_cursor {
            vec![0x1b, b'O', dir]
        } else {
            vec![0x1b, b'[', dir]
        }
    }

    /// Generate escape sequence for function keys with modifiers
    fn function_key(n: u8, ctrl: bool, alt: bool, shift: bool) -> Vec<u8> {
        let modifier = Self::modifier_code(ctrl, alt, shift);

        // F1-F4 use SS3 (ESC O), F5+ use CSI with numbers
        let (prefix, code) = match n {
            1 => (b'O', b'P'),
            2 => (b'O', b'Q'),
            3 => (b'O', b'R'),
            4 => (b'O', b'S'),
            5 => (b'[', b'1'), // 15~
            6 => (b'[', b'1'), // 17~
            7 => (b'[', b'1'), // 18~
            8 => (b'[', b'1'), // 19~
            9 => (b'[', b'2'), // 20~
            10 => (b'[', b'2'), // 21~
            11 => (b'[', b'2'), // 23~
            12 => (b'[', b'2'), // 24~
            _ => return vec![],
        };

        if n <= 4 {
            if modifier > 1 {
                format!("\x1b[1;{}{}", modifier, code as char).into_bytes()
            } else {
                vec![0x1b, prefix, code]
            }
        } else {
            // F5-F12 use CSI number ~
            let num = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return vec![],
            };
            if modifier > 1 {
                format!("\x1b[{};{}~", num, modifier).into_bytes()
            } else {
                format!("\x1b[{}~", num).into_bytes()
            }
        }
    }

    /// Generate escape sequence for editing keys with modifiers
    fn modified_key(base: &[u8], ctrl: bool, alt: bool, shift: bool) -> Vec<u8> {
        let modifier = Self::modifier_code(ctrl, alt, shift);

        if base.ends_with(b"~") {
            // CSI number ~ format
            let num = std::str::from_utf8(&base[..base.len() - 1]).unwrap_or("0");
            if modifier > 1 {
                format!("\x1b[{};{}~", num, modifier).into_bytes()
            } else {
                let mut result = vec![0x1b, b'['];
                result.extend_from_slice(base);
                result
            }
        } else {
            // CSI code format (H, F, etc.)
            if modifier > 1 {
                format!("\x1b[1;{}{}", modifier, base[0] as char).into_bytes()
            } else {
                let mut result = vec![0x1b, b'['];
                result.extend_from_slice(base);
                result
            }
        }
    }

    /// Generate bytes for character keys with modifiers
    fn char_key(c: char, ctrl: bool, alt: bool) -> Vec<u8> {
        if ctrl {
            // Ctrl+letter = control code
            let code = c.to_ascii_lowercase() as u8;
            let ctrl_code = match code {
                b'a'..=b'z' => code - b'a' + 1,
                b'[' => 0x1b, // Ctrl+[ = ESC
                b'\\' => 0x1c,
                b']' => 0x1d,
                b'^' => 0x1e,
                b'_' => 0x1f,
                b'?' => 0x7f, // Ctrl+? = DEL
                b'@' => 0x00, // Ctrl+@ = NUL
                _ => return vec![],
            };
            if alt {
                vec![0x1b, ctrl_code]
            } else {
                vec![ctrl_code]
            }
        } else if alt {
            // Alt sends escape prefix
            let mut bytes = vec![0x1b];
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            bytes
        } else {
            // Regular character
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf).as_bytes().to_vec()
        }
    }

    /// Calculate xterm-style modifier code
    /// 1 = none, 2 = shift, 3 = alt, 4 = shift+alt
    /// 5 = ctrl, 6 = shift+ctrl, 7 = alt+ctrl, 8 = shift+alt+ctrl
    fn modifier_code(ctrl: bool, alt: bool, shift: bool) -> u8 {
        let mut code = 1u8;
        if shift {
            code += 1;
        }
        if alt {
            code += 2;
        }
        if ctrl {
            code += 4;
        }
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_mods() -> Modifiers {
        Modifiers::default()
    }

    fn ctrl() -> Modifiers {
        Modifiers { ctrl: true, ..Default::default() }
    }

    fn alt() -> Modifiers {
        Modifiers { alt: true, ..Default::default() }
    }

    fn shift() -> Modifiers {
        Modifiers { shift: true, ..Default::default() }
    }

    fn modes() -> TerminalModes {
        TerminalModes::new()
    }

    #[test]
    fn test_ctrl_c() {
        let result = KeyboardHandler::translate(Key::Char('c'), &ctrl(), &modes());
        assert_eq!(result, Some(vec![0x03])); // ETX
    }

    #[test]
    fn test_alt_x() {
        let result = KeyboardHandler::translate(Key::Char('x'), &alt(), &modes());
        assert_eq!(result, Some(vec![0x1b, b'x']));
    }

    #[test]
    fn test_shift_tab() {
        let result = KeyboardHandler::translate(Key::Tab, &shift(), &modes());
        assert_eq!(result, Some(vec![0x1b, b'[', b'Z']));
    }

    #[test]
    fn test_ctrl_arrow() {
        let mods = Modifiers { ctrl: true, ..Default::default() };
        let result = KeyboardHandler::translate(Key::Right, &mods, &modes());
        assert_eq!(result, Some(b"\x1b[1;5C".to_vec()));
    }
}
