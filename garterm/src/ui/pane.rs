//! Pane - a terminal instance with its own PTY
//!
//! Each pane is an independent terminal that can be split, focused, and closed.

use crate::pty::{Pty, PtySize};
use crate::terminal::Terminal;
use anyhow::Result;

/// Unique identifier for a pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(pub u32);

impl PaneId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }
}

/// A terminal pane with its own PTY
pub struct Pane {
    /// Unique identifier
    pub id: PaneId,
    /// Terminal state
    pub terminal: Terminal,
    /// PTY connection
    pub pty: Pty,
    /// Pane dimensions in pixels
    pub width: u32,
    pub height: u32,
    /// Position within parent (set by layout)
    pub x: u32,
    pub y: u32,
    /// Whether this pane is focused
    pub focused: bool,
}

impl Pane {
    /// Create a new pane with the given shell
    pub fn new(
        id: PaneId,
        shell: &str,
        cols: usize,
        rows: usize,
        width: u32,
        height: u32,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self> {
        let terminal = Terminal::new(cols, rows);

        let pty_size = PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: width as u16,
            pixel_height: height as u16,
        };
        let pty = Pty::spawn(shell, pty_size, cwd)?;

        Ok(Self {
            id,
            terminal,
            pty,
            width,
            height,
            x: 0,
            y: 0,
            focused: false,
        })
    }

    /// Resize the pane
    pub fn resize(&mut self, cols: usize, rows: usize, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;

        if cols != self.terminal.cols() || rows != self.terminal.rows() {
            self.terminal.resize(cols, rows);
            self.pty.resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: width as u16,
                pixel_height: height as u16,
            })?;
        }

        Ok(())
    }

    /// Set pane position (called by layout)
    pub fn set_position(&mut self, x: u32, y: u32) {
        self.x = x;
        self.y = y;
    }

    /// Check if pane's shell is still alive
    pub fn is_alive(&self) -> bool {
        self.pty.is_alive()
    }

    /// Get the PTY's master fd for polling
    pub fn pty_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;
        self.pty.master_fd().as_raw_fd()
    }

    /// Process input from PTY
    pub fn read_pty(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.pty.read(buf)?;
        if n > 0 {
            self.terminal.input(&buf[..n]);
        }
        Ok(n)
    }

    /// Write to PTY
    pub fn write_pty(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.pty.write_all(data)
    }

    /// Check and clear dirty flag
    pub fn take_dirty(&mut self) -> bool {
        self.terminal.take_dirty()
    }

    /// Mark terminal as needing redraw
    pub fn mark_dirty(&mut self) {
        self.terminal.mark_dirty();
    }
}
