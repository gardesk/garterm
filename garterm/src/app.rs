use crate::config::Config;
use crate::input::{Clipboard, KeyboardHandler, MouseButton, MouseEvent, MouseHandler, Selection, SelectionMode};
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
    clipboard: Clipboard,
    selection: Selection,
    /// Last click time for double/triple click detection
    last_click: std::time::Instant,
    /// Click count for double/triple click
    click_count: u32,
    /// Use VSync-based rendering (dirty flag only)
    vsync: bool,
}

impl App {
    /// Create a new terminal application
    pub async fn new(config: Config) -> Result<Self> {
        // Connect to X11
        let conn = Connection::connect(None)?;

        // Intern WM_DELETE_WINDOW atom
        let wm_delete_window = conn.intern_atom("WM_DELETE_WINDOW", false)?;

        // Calculate initial window size
        let font_size = config.font.size;
        let cols = config.window.columns;
        let rows = config.window.rows;

        // Estimate cell size (will be refined after font loading)
        let cell_w = (font_size * 0.6) as u32;
        let cell_h = (font_size * 1.2) as u32;
        let width = cols * cell_w;
        let height = rows * cell_h;

        // Create window (don't auto-map so we can wait for WM to configure it)
        let window = Window::create(
            conn.clone(),
            WindowConfig::new()
                .title("garterm")
                .class("garterm")
                .size(width, height)
                .background(0x1a1b26) // Dark background (matching terminal)
                .map_on_create(false),
        )?;

        info!("Created window {}x{}", width, height);

        // Map window and wait for WM to assign final size
        // This avoids the "quarter shading" issue where wgpu surface is created
        // at requested size but WM immediately resizes to tiled size
        window.map()?;
        conn.flush()?;

        // Wait for ConfigureNotify to get actual window size from WM
        use x11rb::protocol::Event;
        let (actual_width, actual_height) = loop {
            let event = conn.wait_event()?;
            if let Event::ConfigureNotify(e) = event {
                break (e.width as u32, e.height as u32);
            }
            // Continue waiting for ConfigureNotify, ignore other events
        };

        info!("Window configured: {}x{}", actual_width, actual_height);

        // Resolve color palette from config
        let colors = config.color_palette();

        // Create renderer with actual window size
        let renderer = Renderer::new(
            window.id(),
            actual_width,
            actual_height,
            font_size,
            colors,
        ).await?;

        // Calculate actual cell size from loaded fonts
        let (cell_w, cell_h) = renderer.cell_size();
        let cols = (actual_width as f32 / cell_w) as usize;
        let rows = (actual_height as f32 / cell_h) as usize;

        info!("Terminal size: {}x{} (cell: {}x{})", cols, rows, cell_w, cell_h);

        // Create terminal
        let terminal = Terminal::new(cols, rows);

        // Create PTY
        let pty_size = PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: actual_width as u16,
            pixel_height: actual_height as u16,
        };
        let pty = Pty::spawn(
            &config.general.shell,
            pty_size,
            config.general.working_directory.as_deref(),
        )?;

        // Set up signal handler
        let signals = SignalHandler::new()?;

        // Set up clipboard
        let clipboard = Clipboard::new(&conn, window.id())?;

        Ok(Self {
            window,
            renderer,
            terminal,
            pty,
            signals,
            running: true,
            wm_delete_window,
            clipboard,
            selection: Selection::new(),
            last_click: std::time::Instant::now(),
            click_count: 0,
            vsync: config.general.vsync,
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
                        tracing::debug!("PTY read {} bytes", n);
                        self.terminal.input(&buf[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
                }
            }

            // Flush terminal responses (DA, DSR, etc.) back to PTY
            for response in self.terminal.take_responses() {
                self.pty.write_all(&response)?;
            }

            // Handle bell
            if self.terminal.take_bell() {
                // TODO: visual bell or audio bell based on config
                // For now, just log it
                tracing::debug!("Bell!");
            }

            // Check for hangup
            if pty_hup && !self.pty.is_alive() {
                self.running = false;
            }

            // Rendering strategy depends on vsync setting:
            //
            // vsync=false (default): Render every frame at ~60fps.
            //   Required on Asahi Linux where VBlank interrupts don't work and
            //   wgpu can't report X11 damage regions. Compositor needs continuous
            //   frame submission to display content.
            //
            // vsync=true: Only render when terminal content changes.
            //   More efficient but requires proper VSync/damage support.
            //   May cause display issues on Asahi Linux.
            let dirty = self.terminal.take_dirty();
            if !self.vsync || dirty {
                self.renderer.render(&self.terminal)?;
                self.window.connection().flush()?;
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
                ReceivedSignal::ReloadConfig => {
                    info!("Reloading configuration (SIGHUP)");
                    self.reload_config()?;
                }
            }
        }
        Ok(())
    }

    /// Reload configuration from disk
    fn reload_config(&mut self) -> Result<()> {
        let config = Config::load();
        info!("Config reloaded");

        // Update vsync setting
        self.vsync = config.general.vsync;

        // Reload colors
        self.renderer.set_colors(config.color_palette());

        // TODO: Reload keybinds

        // Force redraw
        self.terminal.mark_dirty();

        Ok(())
    }

    fn handle_x11_events(&mut self) -> Result<()> {
        use x11rb::protocol::Event;

        // Collect events first to avoid borrow issues
        let mut events = Vec::new();
        {
            let conn = self.window.connection();
            while let Some(event) = conn.poll_event()? {
                events.push(event);
            }
        }

        for event in events {
            match event {
                Event::Expose(_) => {
                    self.terminal.mark_dirty();
                }

                Event::ConfigureNotify(e) => {
                    let width = e.width as u32;
                    let height = e.height as u32;

                    // Only process if size actually changed
                    if self.renderer.size() != (width, height) {
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

                        // Force immediate re-render after resize to clear stale content
                        self.terminal.mark_dirty();
                        self.renderer.render(&self.terminal)?;
                    }
                }

                Event::KeyPress(e) => {
                    self.handle_key_press(e)?;
                }

                Event::ButtonPress(e) => {
                    self.handle_button_press(e)?;
                }

                Event::ButtonRelease(e) => {
                    self.handle_button_release(e)?;
                }

                Event::MotionNotify(e) => {
                    self.handle_motion(e)?;
                }

                Event::SelectionRequest(e) => {
                    let conn = self.window.connection();
                    self.clipboard.handle_selection_request(conn, &e)?;
                }

                Event::SelectionNotify(e) => {
                    let conn = self.window.connection();
                    if let Some(text) = self.clipboard.handle_selection_notify(conn, &e)? {
                        // Paste the text
                        self.pty.write_all(text.as_bytes())?;
                    }
                }

                Event::SelectionClear(e) => {
                    self.clipboard.handle_selection_clear(&e);
                }

                Event::ClientMessage(e) => {
                    // Check for WM_DELETE_WINDOW
                    if e.data.as_data32()[0] == self.wm_delete_window {
                        self.running = false;
                    }
                }

                Event::FocusIn(_) => {
                    tracing::debug!("FocusIn event");
                    self.terminal.mark_dirty();
                }

                Event::FocusOut(_) => {
                    tracing::debug!("FocusOut event");
                }

                Event::EnterNotify(e) => {
                    tracing::debug!("EnterNotify at ({}, {})", e.event_x, e.event_y);
                }

                Event::LeaveNotify(_) => {
                    tracing::debug!("LeaveNotify");
                }

                _ => {
                    tracing::trace!("Unhandled event: {:?}", event);
                }
            }
        }

        Ok(())
    }

    fn handle_key_press(&mut self, event: xproto::KeyPressEvent) -> Result<()> {
        use gartk_core::Key;
        use gartk_x11::{key_from_keycode, modifiers_from_x11};

        let modifiers = modifiers_from_x11(event.state);
        let key = key_from_keycode(event.detail, &modifiers);

        // Handle Ctrl+Shift+C (copy) and Ctrl+Shift+V (paste)
        if modifiers.ctrl && modifiers.shift {
            match key {
                Key::Char('c') | Key::Char('C') => {
                    // Copy selection to clipboard
                    if !self.selection.is_empty() {
                        let text = self.selection.get_text(self.terminal.grid(), self.terminal.cols());
                        if !text.is_empty() {
                            self.clipboard.copy_clipboard(self.window.connection(), text)?;
                        }
                    }
                    return Ok(());
                }
                Key::Char('v') | Key::Char('V') => {
                    // Paste from clipboard
                    self.clipboard.paste_clipboard(self.window.connection())?;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Use KeyboardHandler for normal key translation
        if let Some(bytes) = KeyboardHandler::translate(key, &modifiers, self.terminal.modes()) {
            self.pty.write_all(&bytes)?;
        }

        Ok(())
    }

    fn handle_button_press(&mut self, event: xproto::ButtonPressEvent) -> Result<()> {
        let (cell_w, cell_h) = self.renderer.cell_size();
        let col = (event.event_x as f32 / cell_w) as usize;
        let row = (event.event_y as f32 / cell_h) as usize;

        let button = match event.detail {
            1 => MouseButton::Left,
            2 => MouseButton::Middle,
            3 => MouseButton::Right,
            4 => MouseButton::WheelUp,
            5 => MouseButton::WheelDown,
            _ => return Ok(()),
        };

        // Check mouse mode for reporting to application
        let modes = self.terminal.modes();
        let state: u16 = event.state.into();
        if modes.mouse_mode != crate::terminal::MouseMode::None {
            let shift = state & 0x01 != 0;
            let alt = state & 0x08 != 0;
            let ctrl = state & 0x04 != 0;

            if let Some(bytes) = MouseHandler::encode(
                MouseEvent::Press(button),
                col,
                row,
                shift,
                alt,
                ctrl,
                modes.mouse_mode,
                modes.mouse_encoding,
            ) {
                self.pty.write_all(&bytes)?;
                return Ok(());
            }
        }

        // Handle local mouse events (selection, paste)
        match button {
            MouseButton::Left => {
                // Click detection for double/triple click
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(self.last_click);

                if elapsed.as_millis() < 400 {
                    self.click_count += 1;
                } else {
                    self.click_count = 1;
                }
                self.last_click = now;

                match self.click_count {
                    1 => {
                        // Single click - start selection
                        let mode = if state & 0x04 != 0 {
                            // Ctrl held - block selection
                            SelectionMode::Block
                        } else {
                            SelectionMode::Normal
                        };
                        self.selection.start(row, col, mode);
                    }
                    2 => {
                        // Double click - select word
                        self.selection.select_word(row, col, self.terminal.grid(), self.terminal.cols());
                    }
                    _ => {
                        // Triple+ click - select line
                        self.selection.select_line(row, self.terminal.cols());
                    }
                }
                self.terminal.mark_dirty();
            }
            MouseButton::Middle => {
                // Middle click - paste from PRIMARY
                self.clipboard.paste_primary(self.window.connection())?;
            }
            MouseButton::Right => {
                // Right click could paste or show context menu
                // For now, paste from clipboard
                self.clipboard.paste_clipboard(self.window.connection())?;
            }
            MouseButton::WheelUp => {
                // Scroll up into history (3 lines per tick)
                self.terminal.scroll_up(3);
            }
            MouseButton::WheelDown => {
                // Scroll down towards current (3 lines per tick)
                self.terminal.scroll_down(3);
            }
            MouseButton::None => {}
        }

        Ok(())
    }

    fn handle_button_release(&mut self, event: xproto::ButtonReleaseEvent) -> Result<()> {
        let (cell_w, cell_h) = self.renderer.cell_size();
        let col = (event.event_x as f32 / cell_w) as usize;
        let row = (event.event_y as f32 / cell_h) as usize;

        let button = match event.detail {
            1 => MouseButton::Left,
            2 => MouseButton::Middle,
            3 => MouseButton::Right,
            _ => return Ok(()),
        };

        // Check mouse mode for reporting
        let modes = self.terminal.modes();
        let state: u16 = event.state.into();
        if modes.mouse_mode != crate::terminal::MouseMode::None {
            let shift = state & 0x01 != 0;
            let alt = state & 0x08 != 0;
            let ctrl = state & 0x04 != 0;

            if let Some(bytes) = MouseHandler::encode(
                MouseEvent::Release(button),
                col,
                row,
                shift,
                alt,
                ctrl,
                modes.mouse_mode,
                modes.mouse_encoding,
            ) {
                self.pty.write_all(&bytes)?;
            }
        }

        // Finish selection and copy to PRIMARY
        if button == MouseButton::Left && self.selection.is_active() {
            self.selection.finish();

            // Copy selection to PRIMARY (X11 convention)
            if !self.selection.is_empty() {
                let text = self.selection.get_text(self.terminal.grid(), self.terminal.cols());
                if !text.is_empty() {
                    self.clipboard.copy_primary(self.window.connection(), text)?;
                }
            }
        }

        Ok(())
    }

    fn handle_motion(&mut self, event: xproto::MotionNotifyEvent) -> Result<()> {
        let (cell_w, cell_h) = self.renderer.cell_size();
        let col = (event.event_x as f32 / cell_w) as usize;
        let row = (event.event_y as f32 / cell_h) as usize;

        // Check mouse mode for motion reporting
        let modes = self.terminal.modes();
        let state: u16 = event.state.into();
        if modes.mouse_mode != crate::terminal::MouseMode::None {
            let shift = state & 0x01 != 0;
            let alt = state & 0x08 != 0;
            let ctrl = state & 0x04 != 0;

            // Determine if button is held (drag) or just motion
            let mouse_event = if state & 0x100 != 0 {
                MouseEvent::Drag(MouseButton::Left)
            } else if state & 0x200 != 0 {
                MouseEvent::Drag(MouseButton::Middle)
            } else if state & 0x400 != 0 {
                MouseEvent::Drag(MouseButton::Right)
            } else {
                MouseEvent::Motion
            };

            if let Some(bytes) = MouseHandler::encode(
                mouse_event,
                col,
                row,
                shift,
                alt,
                ctrl,
                modes.mouse_mode,
                modes.mouse_encoding,
            ) {
                self.pty.write_all(&bytes)?;
            }
        }

        // Update selection during drag
        if self.selection.is_active() && state & 0x100 != 0 {
            self.selection.update(row, col);
            self.terminal.mark_dirty();
        }

        Ok(())
    }

    /// Get selection for rendering
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
}
