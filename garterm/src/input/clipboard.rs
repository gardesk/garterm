//! X11 clipboard handling (PRIMARY and CLIPBOARD)

use anyhow::Result;
use gartk_x11::Connection;
use x11rb::protocol::xproto::{self, Atom, ConnectionExt, Window};
use std::time::{Duration, Instant};

/// X11 clipboard handler
pub struct Clipboard {
    /// Atoms for clipboard operations
    atoms: ClipboardAtoms,
    /// Current PRIMARY selection content (if we own it)
    primary_content: Option<String>,
    /// Current CLIPBOARD content (if we own it)
    clipboard_content: Option<String>,
    /// Our window (used for selection ownership)
    window: Window,
}

/// Clipboard atoms
struct ClipboardAtoms {
    primary: Atom,
    clipboard: Atom,
    utf8_string: Atom,
    targets: Atom,
    atom: Atom,
    /// Property for paste requests
    garterm_sel: Atom,
}

impl Clipboard {
    /// Create a new clipboard handler
    pub fn new(conn: &Connection, window: Window) -> Result<Self> {
        let atoms = ClipboardAtoms {
            primary: conn.intern_atom("PRIMARY", false)?,
            clipboard: conn.intern_atom("CLIPBOARD", false)?,
            utf8_string: conn.intern_atom("UTF8_STRING", false)?,
            targets: conn.intern_atom("TARGETS", false)?,
            atom: conn.intern_atom("ATOM", false)?,
            garterm_sel: conn.intern_atom("_GARTERM_SELECTION", false)?,
        };

        Ok(Self {
            atoms,
            primary_content: None,
            clipboard_content: None,
            window,
        })
    }

    /// Copy text to PRIMARY selection (for mouse selection)
    pub fn copy_primary(&mut self, conn: &Connection, text: String) -> Result<()> {
        self.primary_content = Some(text);
        conn.inner()
            .set_selection_owner(self.window, self.atoms.primary, x11rb::CURRENT_TIME)?;
        conn.flush()?;
        Ok(())
    }

    /// Copy text to CLIPBOARD (for Ctrl+Shift+C)
    pub fn copy_clipboard(&mut self, conn: &Connection, text: String) -> Result<()> {
        self.clipboard_content = Some(text);
        conn.inner()
            .set_selection_owner(self.window, self.atoms.clipboard, x11rb::CURRENT_TIME)?;
        conn.flush()?;
        Ok(())
    }

    /// Request paste from PRIMARY (middle-click paste)
    pub fn paste_primary(&self, conn: &Connection) -> Result<()> {
        conn.inner().convert_selection(
            self.window,
            self.atoms.primary,
            self.atoms.utf8_string,
            self.atoms.garterm_sel,
            x11rb::CURRENT_TIME,
        )?;
        conn.flush()?;
        Ok(())
    }

    /// Request paste from CLIPBOARD (Ctrl+Shift+V)
    pub fn paste_clipboard(&self, conn: &Connection) -> Result<()> {
        conn.inner().convert_selection(
            self.window,
            self.atoms.clipboard,
            self.atoms.utf8_string,
            self.atoms.garterm_sel,
            x11rb::CURRENT_TIME,
        )?;
        conn.flush()?;
        Ok(())
    }

    /// Handle SelectionRequest event (another client wants our selection)
    pub fn handle_selection_request(
        &self,
        conn: &Connection,
        event: &xproto::SelectionRequestEvent,
    ) -> Result<()> {
        // Determine which selection was requested
        let content = if event.selection == self.atoms.primary {
            self.primary_content.as_deref()
        } else if event.selection == self.atoms.clipboard {
            self.clipboard_content.as_deref()
        } else {
            None
        };

        let property = if event.target == self.atoms.targets {
            // Client is asking what formats we support
            let targets: Vec<Atom> = vec![self.atoms.targets, self.atoms.utf8_string];
            conn.inner().change_property(
                xproto::PropMode::REPLACE,
                event.requestor,
                event.property,
                self.atoms.atom,
                32,
                targets.len() as u32,
                bytemuck::cast_slice(&targets),
            )?;
            event.property
        } else if event.target == self.atoms.utf8_string {
            // Client wants UTF-8 text
            if let Some(text) = content {
                conn.inner().change_property(
                    xproto::PropMode::REPLACE,
                    event.requestor,
                    event.property,
                    self.atoms.utf8_string,
                    8,
                    text.len() as u32,
                    text.as_bytes(),
                )?;
                event.property
            } else {
                0 // No data
            }
        } else {
            0 // Unsupported target
        };

        // Send SelectionNotify
        let notify = xproto::SelectionNotifyEvent {
            response_type: xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: event.time,
            requestor: event.requestor,
            selection: event.selection,
            target: event.target,
            property,
        };
        conn.inner().send_event(
            false,
            event.requestor,
            xproto::EventMask::NO_EVENT,
            notify,
        )?;
        conn.flush()?;

        Ok(())
    }

    /// Handle SelectionNotify event (response to our paste request)
    pub fn handle_selection_notify(
        &self,
        conn: &Connection,
        event: &xproto::SelectionNotifyEvent,
    ) -> Result<Option<String>> {
        if event.property == 0 {
            // Selection request failed
            return Ok(None);
        }

        // Read the property
        let reply = conn.inner().get_property(
            true, // delete after reading
            self.window,
            self.atoms.garterm_sel,
            self.atoms.utf8_string,
            0,
            1024 * 1024, // 1MB max
        )?.reply()?;

        if reply.type_ == self.atoms.utf8_string {
            let text = String::from_utf8_lossy(&reply.value).into_owned();
            Ok(Some(text))
        } else {
            Ok(None)
        }
    }

    /// Handle SelectionClear event (we lost ownership)
    pub fn handle_selection_clear(&mut self, event: &xproto::SelectionClearEvent) {
        if event.selection == self.atoms.primary {
            self.primary_content = None;
        } else if event.selection == self.atoms.clipboard {
            self.clipboard_content = None;
        }
    }

    /// Check if we own a selection
    pub fn owns_primary(&self) -> bool {
        self.primary_content.is_some()
    }

    pub fn owns_clipboard(&self) -> bool {
        self.clipboard_content.is_some()
    }

    /// Get the garterm selection atom (for matching events)
    pub fn selection_property(&self) -> Atom {
        self.atoms.garterm_sel
    }

    /// Get PRIMARY atom
    pub fn primary_atom(&self) -> Atom {
        self.atoms.primary
    }

    /// Get CLIPBOARD atom
    pub fn clipboard_atom(&self) -> Atom {
        self.atoms.clipboard
    }
}
