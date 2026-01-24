use crate::pty::{Pty, PtySize, ReceivedSignal, SignalHandler};
use crate::render::Renderer;
use crate::terminal::Terminal;
use anyhow::Result;
use gartk_x11::{Connection, Window, WindowConfig};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use std::os::fd::{AsRawFd, BorrowedFd};
use tracing::info;
use x11rb::protocol::xproto;

/// Terminal application
pub struct App {
    window: Window,
    renderer: Renderer,
    terminal: Terminal,
    pty: Pty,
    signals: SignalHandler,
    running: bool,
    wm_delete_window: xproto::Atom,
}

impl App {
    /// Create a new terminal application
    pub async fn new(shell: &str, cwd: Option<&std::path::Path>) -> Result<Self> {
        // Connect to X11
        let conn = Connection::connect(None)?;

        // Intern WM_DELETE_WINDOW atom
        let wm_delete_window = conn.intern_atom("WM_DELETE_WINDOW", false)?;

        // Calculate initial window size
        let font_size = 14.0;
        let cols = 80;
        let rows = 24;

        // Estimate cell size (will be refined after font loading)
        let cell_w = (font_size * 0.6) as u32;
        let cell_h = (font_size * 1.2) as u32;
        let width = cols * cell_w;
        let height = rows * cell_h;

        // Create window
        let window = Window::create(
            conn.clone(),
            WindowConfig::new()
                .title("garterm")
                .class("garterm")
                .size(width, height)
                .background(0xFF1a1b26), // Dark background
        )?;

        info!("Created window {}x{}", width, height);

        // Create renderer
        let renderer = Renderer::new(
            window.id(),
            conn.screen_num() as i32,
            width,
            height,
            font_size,
        ).await?;

        // Calculate actual cell size from loaded fonts
        let (cell_w, cell_h) = renderer.cell_size();
        let cols = (width as f32 / cell_w) as usize;
        let rows = (height as f32 / cell_h) as usize;

        info!("Terminal size: {}x{} (cell: {}x{})", cols, rows, cell_w, cell_h);

        // Create terminal
        let terminal = Terminal::new(cols, rows);

        // Create PTY
        let pty_size = PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: width as u16,
            pixel_height: height as u16,
        };
        let pty = Pty::spawn(shell, pty_size, cwd)?;

        // Set up signal handler
        let signals = SignalHandler::new()?;

        Ok(Self {
            window,
            renderer,
            terminal,
            pty,
            signals,
            running: true,
            wm_delete_window,
        })
    }

    /// Run the application event loop
    pub fn run(&mut self) -> Result<()> {
        let mut buf = [0u8; 4096];

        // Get raw fds for polling
        let pty_fd = self.pty.master_fd().as_raw_fd();
        let signal_fd = self.signals.as_raw_fd();
        let x11_fd = self.window.connection().inner().stream().as_raw_fd();

        while self.running {
            // Poll all file descriptors
            let pty_borrow = unsafe { BorrowedFd::borrow_raw(pty_fd) };
            let signal_borrow = unsafe { BorrowedFd::borrow_raw(signal_fd) };
            let x11_borrow = unsafe { BorrowedFd::borrow_raw(x11_fd) };

            let mut fds = [
                PollFd::new(pty_borrow, PollFlags::POLLIN),
                PollFd::new(signal_borrow, PollFlags::POLLIN),
                PollFd::new(x11_borrow, PollFlags::POLLIN),
            ];

            // Use a short timeout for rendering
            poll(&mut fds, PollTimeout::from(16u16))?; // ~60fps

            let pty_ready = fds[0].revents().is_some_and(|r| r.contains(PollFlags::POLLIN));
            let signal_ready = fds[1].revents().is_some_and(|r| r.contains(PollFlags::POLLIN));
            let x11_ready = fds[2].revents().is_some_and(|r| r.contains(PollFlags::POLLIN));
            let pty_hup = fds[0].revents().is_some_and(|r| r.contains(PollFlags::POLLHUP));

            let _ = fds;

            // Handle signals
            if signal_ready {
                self.handle_signals()?;
            }

            // Handle X11 events
            if x11_ready {
                self.handle_x11_events()?;
            }

            // Read from PTY
            if pty_ready {
                match self.pty.read(&mut buf) {
                    Ok(0) => {
                        self.running = false;
                    }
                    Ok(n) => {
                        self.terminal.input(&buf[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
                }
            }

            // Check for hangup
            if pty_hup && !self.pty.is_alive() {
                self.running = false;
            }

            // Render if dirty
            if self.terminal.take_dirty() {
                self.renderer.render(&self.terminal)?;
            }
        }

        info!("garterm exiting");
        Ok(())
    }

    fn handle_signals(&mut self) -> Result<()> {
        for sig in self.signals.read_signals()? {
            match sig {
                ReceivedSignal::ChildExited { pid, status } => {
                    info!("child {} exited with status {}", pid, status);
                    self.running = false;
                }
                ReceivedSignal::WindowResized => {
                    // X11 window resize is handled via ConfigureNotify
                }
            }
        }
        Ok(())
    }

    fn handle_x11_events(&mut self) -> Result<()> {
        let conn = self.window.connection();

        while let Some(event) = conn.poll_event()? {
            use x11rb::protocol::Event;

            match event {
                Event::Expose(_) => {
                    self.terminal.mark_dirty();
                }

                Event::ConfigureNotify(e) => {
                    let width = e.width as u32;
                    let height = e.height as u32;

                    self.renderer.resize(width, height);

                    let (cell_w, cell_h) = self.renderer.cell_size();
                    let cols = (width as f32 / cell_w) as usize;
                    let rows = (height as f32 / cell_h) as usize;

                    if cols != self.terminal.cols() || rows != self.terminal.rows() {
                        self.terminal.resize(cols, rows);
                        self.pty.resize(PtySize {
                            rows: rows as u16,
                            cols: cols as u16,
                            pixel_width: width as u16,
                            pixel_height: height as u16,
                        })?;
                        info!("Resized to {}x{}", cols, rows);
                    }

                    self.terminal.mark_dirty();
                }

                Event::KeyPress(e) => {
                    if let Some(bytes) = self.translate_key(e) {
                        self.pty.write_all(&bytes)?;
                    }
                }

                Event::ClientMessage(e) => {
                    // Check for WM_DELETE_WINDOW
                    if e.data.as_data32()[0] == self.wm_delete_window {
                        self.running = false;
                    }
                }

                Event::FocusIn(_) => {
                    // Could send focus event to terminal if mode enabled
                }

                Event::FocusOut(_) => {
                    // Could send focus event to terminal if mode enabled
                }

                _ => {}
            }
        }

        Ok(())
    }

    fn translate_key(&self, event: xproto::KeyPressEvent) -> Option<Vec<u8>> {
        use gartk_x11::{key_from_keycode, modifiers_from_x11};
        use gartk_core::Key;

        let modifiers = modifiers_from_x11(event.state);
        let key = key_from_keycode(event.detail, &modifiers);
        let ctrl = modifiers.ctrl;
        let alt = modifiers.alt;

        // Handle special keys
        let bytes: Vec<u8> = match key {
            Key::Return => vec![b'\r'],
            Key::Tab => vec![b'\t'],
            Key::Escape => vec![0x1b],
            Key::Backspace => vec![0x7f],
            Key::Delete => vec![0x1b, b'[', b'3', b'~'],
            Key::Home => vec![0x1b, b'[', b'H'],
            Key::End => vec![0x1b, b'[', b'F'],
            Key::PageUp => vec![0x1b, b'[', b'5', b'~'],
            Key::PageDown => vec![0x1b, b'[', b'6', b'~'],
            Key::Insert => vec![0x1b, b'[', b'2', b'~'],

            Key::Up => {
                if self.terminal.modes().application_cursor {
                    vec![0x1b, b'O', b'A']
                } else {
                    vec![0x1b, b'[', b'A']
                }
            }
            Key::Down => {
                if self.terminal.modes().application_cursor {
                    vec![0x1b, b'O', b'B']
                } else {
                    vec![0x1b, b'[', b'B']
                }
            }
            Key::Right => {
                if self.terminal.modes().application_cursor {
                    vec![0x1b, b'O', b'C']
                } else {
                    vec![0x1b, b'[', b'C']
                }
            }
            Key::Left => {
                if self.terminal.modes().application_cursor {
                    vec![0x1b, b'O', b'D']
                } else {
                    vec![0x1b, b'[', b'D']
                }
            }

            Key::F1 => vec![0x1b, b'O', b'P'],
            Key::F2 => vec![0x1b, b'O', b'Q'],
            Key::F3 => vec![0x1b, b'O', b'R'],
            Key::F4 => vec![0x1b, b'O', b'S'],
            Key::F5 => vec![0x1b, b'[', b'1', b'5', b'~'],
            Key::F6 => vec![0x1b, b'[', b'1', b'7', b'~'],
            Key::F7 => vec![0x1b, b'[', b'1', b'8', b'~'],
            Key::F8 => vec![0x1b, b'[', b'1', b'9', b'~'],
            Key::F9 => vec![0x1b, b'[', b'2', b'0', b'~'],
            Key::F10 => vec![0x1b, b'[', b'2', b'1', b'~'],
            Key::F11 => vec![0x1b, b'[', b'2', b'3', b'~'],
            Key::F12 => vec![0x1b, b'[', b'2', b'4', b'~'],

            Key::Char(c) => {
                if ctrl {
                    // Ctrl+A = 0x01, etc.
                    let code = c.to_ascii_lowercase() as u8;
                    if code >= b'a' && code <= b'z' {
                        vec![code - b'a' + 1]
                    } else {
                        return None;
                    }
                } else if alt {
                    // Alt sends escape prefix
                    let mut bytes = vec![0x1b];
                    let mut buf = [0u8; 4];
                    bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    bytes
                } else {
                    let mut buf = [0u8; 4];
                    c.encode_utf8(&mut buf).as_bytes().to_vec()
                }
            }

            Key::Space => {
                if ctrl {
                    vec![0] // Ctrl+Space = NUL
                } else {
                    vec![b' ']
                }
            }

            _ => return None,
        };

        Some(bytes)
    }
}
