//! Mouse input handling for terminal

use crate::terminal::{MouseEncoding, MouseMode};

/// Mouse button identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    /// Scroll up
    WheelUp,
    /// Scroll down
    WheelDown,
    /// No button (motion only)
    None,
}

impl MouseButton {
    /// Get the button code for mouse reporting
    fn code(&self) -> u8 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
            MouseButton::None => 3, // Release or motion
        }
    }
}

/// Mouse event type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEvent {
    Press(MouseButton),
    Release(MouseButton),
    Motion,
    Drag(MouseButton),
}

/// Mouse handler for generating terminal escape sequences
pub struct MouseHandler;

impl MouseHandler {
    /// Generate mouse escape sequence
    ///
    /// Returns None if mouse reporting is disabled for this event type
    pub fn encode(
        event: MouseEvent,
        col: usize,
        row: usize,
        shift: bool,
        alt: bool,
        ctrl: bool,
        mode: MouseMode,
        encoding: MouseEncoding,
    ) -> Option<Vec<u8>> {
        // Check if this event should be reported
        if !Self::should_report(event, mode) {
            return None;
        }

        // Calculate button code with modifiers
        let (button, is_release) = match event {
            MouseEvent::Press(btn) => (btn, false),
            MouseEvent::Release(btn) => (btn, true),
            MouseEvent::Motion => (MouseButton::None, false),
            MouseEvent::Drag(btn) => (btn, false),
        };

        let mut code = button.code();

        // Add modifier flags
        if shift {
            code |= 4;
        }
        if alt {
            code |= 8;
        }
        if ctrl {
            code |= 16;
        }

        // Motion flag
        if matches!(event, MouseEvent::Motion | MouseEvent::Drag(_)) {
            code |= 32;
        }

        // Convert to 1-indexed coordinates
        let col = col.saturating_add(1);
        let row = row.saturating_add(1);

        match encoding {
            MouseEncoding::X10 => Self::encode_x10(code, col, row, is_release),
            MouseEncoding::Utf8 => Self::encode_utf8(code, col, row, is_release),
            MouseEncoding::Sgr => Self::encode_sgr(code, col, row, is_release),
            MouseEncoding::Urxvt => Self::encode_urxvt(code, col, row, is_release),
        }
    }

    /// Check if event should be reported based on mouse mode
    fn should_report(event: MouseEvent, mode: MouseMode) -> bool {
        match mode {
            MouseMode::None => false,
            MouseMode::X10 => matches!(event, MouseEvent::Press(_)),
            MouseMode::Vt200 => matches!(event, MouseEvent::Press(_) | MouseEvent::Release(_)),
            MouseMode::ButtonEvent => {
                matches!(event, MouseEvent::Press(_) | MouseEvent::Release(_) | MouseEvent::Drag(_))
            }
            MouseMode::AnyEvent => true,
        }
    }

    /// X10 encoding: ESC [ M Cb Cx Cy
    fn encode_x10(code: u8, col: usize, row: usize, is_release: bool) -> Option<Vec<u8>> {
        // X10 mode doesn't report release
        if is_release {
            return None;
        }

        // X10 encoding limited to 223 (+ 32 = 255)
        if col > 223 || row > 223 {
            return None;
        }

        Some(vec![
            0x1b,
            b'[',
            b'M',
            code + 32,
            (col as u8) + 32,
            (row as u8) + 32,
        ])
    }

    /// UTF-8 encoding: ESC [ M Cb Cx Cy (with UTF-8 for large values)
    fn encode_utf8(code: u8, col: usize, row: usize, is_release: bool) -> Option<Vec<u8>> {
        let mut result = vec![0x1b, b'[', b'M'];

        // Code byte
        result.push(code + 32);

        // Column (UTF-8 encoded if > 127)
        Self::push_utf8_coord(&mut result, col);

        // Row (UTF-8 encoded if > 127)
        Self::push_utf8_coord(&mut result, row);

        // For release in UTF-8 mode, code 3 is used
        if is_release {
            result[3] = 3 + 32;
        }

        Some(result)
    }

    fn push_utf8_coord(result: &mut Vec<u8>, coord: usize) {
        let val = (coord as u32) + 32;
        if val < 128 {
            result.push(val as u8);
        } else {
            // UTF-8 encode
            let mut buf = [0u8; 4];
            let s = char::from_u32(val).unwrap_or(' ').encode_utf8(&mut buf);
            result.extend_from_slice(s.as_bytes());
        }
    }

    /// SGR encoding: ESC [ < Pb ; Px ; Py M/m
    fn encode_sgr(code: u8, col: usize, row: usize, is_release: bool) -> Option<Vec<u8>> {
        let terminator = if is_release { b'm' } else { b'M' };
        Some(format!("\x1b[<{};{};{}{}", code, col, row, terminator as char).into_bytes())
    }

    /// urxvt encoding: ESC [ Pb ; Px ; Py M
    fn encode_urxvt(code: u8, col: usize, row: usize, is_release: bool) -> Option<Vec<u8>> {
        // urxvt uses code 3 for release
        let code = if is_release { 3 } else { code };
        Some(format!("\x1b[{};{};{}M", code + 32, col, row).into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sgr_press() {
        let result = MouseHandler::encode(
            MouseEvent::Press(MouseButton::Left),
            10,
            20,
            false,
            false,
            false,
            MouseMode::ButtonEvent,
            MouseEncoding::Sgr,
        );
        assert_eq!(result, Some(b"\x1b[<0;11;21M".to_vec()));
    }

    #[test]
    fn test_sgr_release() {
        let result = MouseHandler::encode(
            MouseEvent::Release(MouseButton::Left),
            10,
            20,
            false,
            false,
            false,
            MouseMode::ButtonEvent,
            MouseEncoding::Sgr,
        );
        assert_eq!(result, Some(b"\x1b[<0;11;21m".to_vec()));
    }

    #[test]
    fn test_x10_no_release() {
        let result = MouseHandler::encode(
            MouseEvent::Release(MouseButton::Left),
            10,
            20,
            false,
            false,
            false,
            MouseMode::X10,
            MouseEncoding::X10,
        );
        assert_eq!(result, None);
    }

    #[test]
    fn test_x10_press() {
        let result = MouseHandler::encode(
            MouseEvent::Press(MouseButton::Left),
            0,
            0,
            false,
            false,
            false,
            MouseMode::X10,
            MouseEncoding::X10,
        );
        assert_eq!(result, Some(vec![0x1b, b'[', b'M', 32, 33, 33]));
    }
}
