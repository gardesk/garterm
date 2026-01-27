use crate::config::{Action, Config, KeybindSet, LuaRuntime, Modifiers as ConfigModifiers, TerminalCommand, expand_tilde};
use crate::input::{Clipboard, KeyboardHandler, MouseButton, MouseEvent, MouseHandler, SearchState, Selection, SelectionMode};
use crate::ipc::IpcServer;
use crate::pty::{PtySize, ReceivedSignal, SignalHandler};
use crate::render::{PaneRenderInfo, Renderer, SearchOverlay, SelectionBounds};
use crate::ui::{Direction, PaneId, TabManager};
use anyhow::Result;
use garterm_ipc::{Command, Response};
use gartk_x11::{Connection, Window, WindowConfig};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use std::os::fd::{AsRawFd, BorrowedFd};
use tracing::info;
use x11rb::protocol::xproto;

/// Terminal application
pub struct App {
    window: Window,
    renderer: Renderer,
    tabs: TabManager,
    signals: SignalHandler,
    ipc: Option<IpcServer>,
    running: bool,
    wm_delete_window: xproto::Atom,
    clipboard: Clipboard,
    selection: Selection,
    /// Keybindings from config
    keybinds: KeybindSet,
    /// Last click time for double/triple click detection
    last_click: std::time::Instant,
    /// Click count for double/triple click
    click_count: u32,
    /// Use VSync-based rendering (dirty flag only)
    vsync: bool,
    /// Current window dimensions
    width: u32,
    height: u32,
    /// Shell command for new panes
    shell: String,
    /// Working directory for new panes
    cwd: Option<std::path::PathBuf>,
    /// Lua runtime for scripting (callbacks, sessions)
    lua_runtime: Option<LuaRuntime>,
    /// EWMH atoms for fullscreen
    net_wm_state: xproto::Atom,
    net_wm_state_fullscreen: xproto::Atom,
    /// Fullscreen state
    fullscreen: bool,
    /// Original font size (for reset)
    original_font_size: f32,
    /// Search state for the focused pane
    search: SearchState,
}

impl App {
    /// Create a new terminal application
    pub async fn new(config: Config) -> Result<Self> {
        // Connect to X11
        let conn = Connection::connect(None)?;

        // Intern WM_DELETE_WINDOW atom
        let wm_delete_window = conn.intern_atom("WM_DELETE_WINDOW", false)?;

        // Intern EWMH atoms for fullscreen
        let net_wm_state = conn.intern_atom("_NET_WM_STATE", false)?;
        let net_wm_state_fullscreen = conn.intern_atom("_NET_WM_STATE_FULLSCREEN", false)?;

        // Calculate initial window size
        let font_size = config.font.size;
        let cols = config.window.columns;
        let rows = config.window.rows;

        // Estimate cell size (will be refined after font loading)
        let cell_w = (font_size * 0.6) as u32;
        let cell_h = (font_size * 1.2) as u32;
        let width = cols * cell_w;
        let height = rows * cell_h;

        // Get background color from config for window
        let bg_color = config.color_palette().background.to_u32();

        // Create window (don't auto-map so we can wait for WM to configure it)
        let window = Window::create(
            conn.clone(),
            WindowConfig::new()
                .title(&config.window.title)
                .class(&config.window.class)
                .size(width, height)
                .background(bg_color)
                .map_on_create(false),
        )?;

        info!("Created window {}x{}", width, height);

        // Map window and wait for WM to assign final size
        window.map()?;
        conn.flush()?;

        // Wait for ConfigureNotify to get actual window size from WM
        use x11rb::protocol::Event;
        let (actual_width, actual_height) = loop {
            let event = conn.wait_event()?;
            if let Event::ConfigureNotify(e) = event {
                break (e.width as u32, e.height as u32);
            }
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

        // Create tab manager with initial tab/pane
        let mut tabs = TabManager::new(
            &config.general.shell,
            cols,
            rows,
            actual_width,
            actual_height,
            cell_w,
            cell_h,
            config.general.working_directory.as_deref(),
        )?;

        // Apply tab bar config
        tabs.set_tab_bar_config(&config.tab_bar);

        // Set up signal handler
        let signals = SignalHandler::new()?;

        // Set up clipboard
        let clipboard = Clipboard::new(&conn, window.id())?;

        // Load keybindings from config
        let mut keybinds = config.keybindings();
        info!("Loaded {} keybindings from config", keybinds.iter().count());

        // Start IPC server
        let ipc = match IpcServer::new() {
            Ok(server) => Some(server),
            Err(e) => {
                tracing::warn!("Failed to start IPC server: {}", e);
                None
            }
        };

        // Initialize Lua runtime for scripting
        let lua_runtime = match LuaRuntime::new() {
            Ok(runtime) => {
                if let Err(e) = runtime.load() {
                    tracing::warn!("Lua config error: {}", e);
                }
                // Merge Lua keybinds (function callbacks, sessions) into keybind set
                runtime.merge_keybinds(&mut keybinds);
                Some(runtime)
            }
            Err(e) => {
                tracing::warn!("Failed to create Lua runtime: {}", e);
                None
            }
        };

        Ok(Self {
            window,
            renderer,
            tabs,
            signals,
            ipc,
            running: true,
            wm_delete_window,
            clipboard,
            selection: Selection::new(),
            keybinds,
            last_click: std::time::Instant::now(),
            click_count: 0,
            vsync: config.general.vsync,
            width: actual_width,
            height: actual_height,
            shell: config.general.shell.clone(),
            cwd: config.general.working_directory.clone(),
            lua_runtime,
            net_wm_state,
            net_wm_state_fullscreen,
            fullscreen: false,
            original_font_size: config.font.size,
            search: SearchState::new(),
        })
    }

    /// Run the application event loop
    pub fn run(&mut self) -> Result<()> {
        let mut buf = [0u8; 4096];

        let signal_fd = self.signals.as_raw_fd();
        let x11_fd = self.window.connection().inner().stream().as_raw_fd();

        while self.running {
            // Poll signal and X11 file descriptors
            let signal_borrow = unsafe { BorrowedFd::borrow_raw(signal_fd) };
            let x11_borrow = unsafe { BorrowedFd::borrow_raw(x11_fd) };

            let mut fds = [
                PollFd::new(signal_borrow, PollFlags::POLLIN),
                PollFd::new(x11_borrow, PollFlags::POLLIN),
            ];

            // Use a short timeout for rendering (~60fps)
            poll(&mut fds, PollTimeout::from(16u16))?;

            let signal_ready = fds[0].revents().is_some_and(|r| r.contains(PollFlags::POLLIN));
            let x11_ready = fds[1].revents().is_some_and(|r| r.contains(PollFlags::POLLIN));

            let _ = fds;

            // Handle signals
            if signal_ready {
                self.handle_signals()?;
            }

            // Handle X11 events
            if x11_ready {
                self.handle_x11_events()?;
            }

            // Handle IPC commands
            self.handle_ipc()?;

            // Read from ALL panes in ALL tabs (non-blocking)
            // This ensures background tabs still receive PTY data and can run startup commands
            for tab in self.tabs.all_tabs_mut() {
                for pane in tab.panes.values_mut() {
                    loop {
                        match pane.read_pty(&mut buf) {
                            Ok(0) => break, // EOF
                            Ok(_n) => continue, // Try to read more
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(_) => break,
                        }
                    }

                    // Flush terminal responses back to PTY
                    let responses: Vec<_> = pane.terminal.take_responses().collect();
                    for response in responses {
                        let _ = pane.write_pty(&response);
                    }

                    // Handle bell
                    if pane.terminal.take_bell() {
                        tracing::debug!("Bell!");
                    }

                    // Handle startup command: check for OSC 133 prompt ready
                    if pane.terminal.take_prompt_ready() {
                        pane.on_prompt_ready();
                    }

                    // Check startup command deadline (fallback for shells without OSC 133)
                    if pane.has_pending_startup_cmd() {
                        pane.check_startup_deadline();
                    }
                }
            }

            // Check for exited panes
            let panes_closed = self.tabs.handle_exits();
            if !self.tabs.has_tabs() {
                self.running = false;
            }

            // Relayout and redraw if panes were closed
            if panes_closed && self.running {
                self.tabs.relayout(self.width, self.height)?;
                self.tabs.mark_all_dirty();
            }

            // Render all panes in the active tab
            let any_dirty = self.tabs.active_tab()
                .map(|tab| tab.panes.values().any(|p| p.terminal.is_dirty()))
                .unwrap_or(false);

            if !self.vsync || any_dirty {
                // Clear dirty flags first
                if let Some(tab) = self.tabs.active_tab_mut() {
                    for pane in tab.panes.values_mut() {
                        pane.take_dirty();
                    }
                }

                // Update tab titles from terminal OSC sequences
                self.tabs.update_titles();

                // Get tab bar render data
                let tab_bar_data = self.tabs.render_tab_bar(self.width, self.height);

                // Collect pane render info and render
                // Add content_offset to y positions (to account for tab bar)
                let content_offset = self.tabs.content_offset();
                let selection_bounds = self.get_selection_bounds();
                let pane_infos: Vec<PaneRenderInfo> = self.tabs.active_tab()
                    .map(|tab| {
                        tab.panes.values().map(|pane| PaneRenderInfo {
                            terminal: &pane.terminal,
                            x: pane.x,
                            y: pane.y + content_offset,
                            width: pane.width,
                            height: pane.height,
                            focused: pane.focused,
                            // Only show selection on focused pane
                            selection: if pane.focused { selection_bounds } else { None },
                            search: None, // Search rendering handled separately
                        }).collect()
                    })
                    .unwrap_or_default();

                if !pane_infos.is_empty() {
                    // Build search overlay if search is active or has matches
                    let match_count = self.search.match_count_text();
                    let search_overlay = if self.search.active || !self.search.matches.is_empty() {
                        Some(SearchOverlay {
                            query: &self.search.query,
                            match_count: &match_count,
                            active: self.search.active,
                            case_insensitive: self.search.case_insensitive,
                        })
                    } else {
                        None
                    };
                    self.renderer.render_scene_with_search(&tab_bar_data, &pane_infos, search_overlay.as_ref())?;
                }
                self.window.connection().flush()?;
            }
        }

        info!("garterm exiting");
        Ok(())
    }

    /// Execute a keybind action
    fn execute_action(&mut self, action: &Action) -> Result<bool> {
        match action {
            // Clipboard
            Action::Copy => {
                if !self.selection.is_empty() {
                    if let Some(pane) = self.tabs.focused_pane() {
                        let text = self.selection.get_text(pane.terminal.grid(), pane.terminal.cols());
                        if !text.is_empty() {
                            self.clipboard.copy_clipboard(self.window.connection(), text)?;
                        }
                    }
                }
                Ok(true)
            }
            Action::Paste => {
                self.clipboard.paste_clipboard(self.window.connection())?;
                Ok(true)
            }
            Action::PastePrimary => {
                self.clipboard.paste_primary(self.window.connection())?;
                Ok(true)
            }

            // Tabs
            Action::NewTab => {
                info!("Creating new tab");
                self.tabs.new_tab(self.width, self.height, self.cwd.as_deref())?;
                self.tabs.relayout(self.width, self.height)?;
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::CloseTab => {
                self.tabs.close_tab();
                self.tabs.relayout(self.width, self.height)?;
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::NextTab => {
                self.tabs.next_tab();
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::PrevTab => {
                self.tabs.prev_tab();
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::Tab(n) => {
                self.tabs.switch_to_tab(*n);
                self.tabs.mark_all_dirty();
                Ok(true)
            }

            // Panes
            Action::SplitHorizontal => {
                info!("Horizontal split");
                self.tabs.split_horizontal(self.cwd.as_deref())?;
                self.tabs.relayout(self.width, self.height)?;
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::SplitVertical => {
                info!("Vertical split");
                self.tabs.split_vertical(self.cwd.as_deref())?;
                self.tabs.relayout(self.width, self.height)?;
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::ClosePane => {
                info!("Closing pane");
                if self.tabs.close_pane() {
                    self.tabs.relayout(self.width, self.height)?;
                    self.tabs.mark_all_dirty();
                }
                Ok(true)
            }
            Action::FocusUp => {
                self.tabs.focus_direction(Direction::Up, self.width, self.height);
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::FocusDown => {
                self.tabs.focus_direction(Direction::Down, self.width, self.height);
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::FocusLeft => {
                self.tabs.focus_direction(Direction::Left, self.width, self.height);
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::FocusRight => {
                self.tabs.focus_direction(Direction::Right, self.width, self.height);
                self.tabs.mark_all_dirty();
                Ok(true)
            }

            // Scrollback navigation
            Action::ScrollUp(lines) => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.terminal.scroll_up(*lines);
                    pane.mark_dirty();
                }
                Ok(true)
            }
            Action::ScrollDown(lines) => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.terminal.scroll_down(*lines);
                    pane.mark_dirty();
                }
                Ok(true)
            }
            Action::ScrollPageUp => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    let page = pane.terminal.rows().saturating_sub(1).max(1);
                    pane.terminal.scroll_up(page);
                    pane.mark_dirty();
                }
                Ok(true)
            }
            Action::ScrollPageDown => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    let page = pane.terminal.rows().saturating_sub(1).max(1);
                    pane.terminal.scroll_down(page);
                    pane.mark_dirty();
                }
                Ok(true)
            }
            Action::ScrollToTop => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    let total = pane.terminal.grid().scrollback_len();
                    pane.terminal.scroll_up(total);
                    pane.mark_dirty();
                }
                Ok(true)
            }
            Action::ScrollToBottom => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.terminal.reset_viewport();
                    pane.mark_dirty();
                }
                Ok(true)
            }

            // Font size changes
            Action::IncreaseFontSize => {
                let current = self.renderer.font_size();
                let new_size = (current + 1.0).min(72.0);
                self.change_font_size(new_size)?;
                Ok(true)
            }
            Action::DecreaseFontSize => {
                let current = self.renderer.font_size();
                let new_size = (current - 1.0).max(6.0);
                self.change_font_size(new_size)?;
                Ok(true)
            }
            Action::ResetFontSize => {
                self.change_font_size(self.original_font_size)?;
                Ok(true)
            }

            // Search
            Action::SearchForward => {
                info!("Starting forward search");
                self.search.start();
                self.tabs.mark_all_dirty();
                Ok(true)
            }
            Action::SearchBackward => {
                info!("Starting backward search");
                self.search.start();
                // For backward search, we could track direction
                // but for now just start search mode
                self.tabs.mark_all_dirty();
                Ok(true)
            }

            // Misc
            Action::ReloadConfig => {
                info!("Reloading configuration");

                // Load fresh config
                let config = Config::load();

                // Update keybindings
                self.keybinds = config.keybindings();
                info!("Keybindings reloaded");

                // Update color palette
                self.renderer.set_colors(config.color_palette());
                info!("Color palette reloaded");

                // Update tab bar config
                self.tabs.set_tab_bar_config(&config.tab_bar);
                info!("Tab bar config reloaded");

                // Mark everything dirty to repaint
                self.tabs.mark_all_dirty();

                info!("Configuration reload complete");
                Ok(true)
            }
            Action::ToggleFullscreen => {
                info!("Toggling fullscreen");
                self.fullscreen = !self.fullscreen;

                // Send EWMH client message to toggle fullscreen
                use x11rb::protocol::xproto::{ClientMessageEvent, CLIENT_MESSAGE_EVENT, EventMask, ConnectionExt};

                let data = [
                    if self.fullscreen { 1 } else { 0 }, // _NET_WM_STATE_ADD or _REMOVE
                    self.net_wm_state_fullscreen,
                    0,
                    1, // Source indication: normal application
                    0,
                ];

                let event = ClientMessageEvent {
                    response_type: CLIENT_MESSAGE_EVENT,
                    format: 32,
                    sequence: 0,
                    window: self.window.id(),
                    type_: self.net_wm_state,
                    data: data.into(),
                };

                let conn = self.window.connection();
                conn.inner().send_event(
                    false,
                    conn.root(),
                    EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
                    event,
                )?;
                conn.flush()?;
                Ok(true)
            }
            Action::ResetTerminal => {
                info!("Resetting terminal");
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    // Full terminal reset (like ESC c)
                    let cols = pane.terminal.cols();
                    let rows = pane.terminal.rows();
                    pane.terminal = crate::terminal::Terminal::new(cols, rows);
                    pane.mark_dirty();
                }
                Ok(true)
            }
            Action::ClearScrollback => {
                info!("Clearing scrollback");
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.terminal.grid_mut().clear_scrollback();
                    pane.terminal.reset_viewport();
                    pane.mark_dirty();
                }
                Ok(true)
            }

            // Send raw data
            Action::SendBytes(bytes) => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.write_pty(bytes)?;
                }
                Ok(true)
            }
            Action::SendText(text) => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.write_pty(text.as_bytes())?;
                }
                Ok(true)
            }

            // Pane resize - adjust split ratios
            Action::ResizeUp(amount) | Action::ResizeLeft(amount) => {
                // Shrink focused pane (make it smaller)
                let delta = -(*amount as f32 / 100.0).max(0.01);
                if self.tabs.resize_focused_pane(delta) {
                    self.tabs.relayout(self.width, self.height)?;
                    self.tabs.mark_all_dirty();
                }
                Ok(true)
            }
            Action::ResizeDown(amount) | Action::ResizeRight(amount) => {
                // Grow focused pane (make it larger)
                let delta = (*amount as f32 / 100.0).max(0.01);
                if self.tabs.resize_focused_pane(delta) {
                    self.tabs.relayout(self.width, self.height)?;
                    self.tabs.mark_all_dirty();
                }
                Ok(true)
            }

            // Lua scripting
            Action::LuaCallback(index) => {
                if let Some(ref runtime) = self.lua_runtime {
                    if let Err(e) = runtime.execute_callback(*index) {
                        tracing::error!("Lua callback error: {}", e);
                    }
                    // Process any pending terminal commands from the callback
                    self.process_lua_commands()?;
                }
                Ok(true)
            }
            Action::LoadSession(name) => {
                self.load_session(name)?;
                Ok(true)
            }

            Action::None => Ok(false),
        }
    }

    /// Process pending Lua commands from callbacks
    fn process_lua_commands(&mut self) -> Result<()> {
        let Some(ref runtime) = self.lua_runtime else { return Ok(()) };

        let commands = runtime.take_pending_commands();
        for cmd in commands {
            match cmd {
                TerminalCommand::NewTab { cwd, cmd, title } => {
                    let cwd_path = cwd.map(|s| expand_tilde(&s));
                    let tab_id = self.tabs.new_tab_with_command(
                        self.width,
                        self.height,
                        cwd_path.as_deref(),
                        cmd.as_deref(),
                    )?;
                    if let Some(title) = title {
                        self.tabs.set_tab_title(tab_id, title);
                    }
                    self.tabs.relayout(self.width, self.height)?;
                    self.tabs.mark_all_dirty();
                }
                TerminalCommand::Split { direction, cwd, cmd } => {
                    let cwd_path = cwd.map(|s| expand_tilde(&s));
                    match direction.to_lowercase().as_str() {
                        "horizontal" | "h" => {
                            self.tabs.split_horizontal_with_command(cwd_path.as_deref(), cmd.as_deref())?
                        }
                        _ => {
                            self.tabs.split_vertical_with_command(cwd_path.as_deref(), cmd.as_deref())?
                        }
                    };
                    self.tabs.relayout(self.width, self.height)?;
                    self.tabs.mark_all_dirty();
                }
                TerminalCommand::SendText { pane_id, text } => {
                    let pane = if let Some(id) = pane_id {
                        self.tabs.get_pane_mut(PaneId(id))
                    } else {
                        self.tabs.focused_pane_mut()
                    };
                    if let Some(pane) = pane {
                        pane.write_pty(text.as_bytes())?;
                    }
                }
                TerminalCommand::CloseTab { tab_id: _ } => {
                    self.tabs.close_tab();
                    self.tabs.relayout(self.width, self.height)?;
                    self.tabs.mark_all_dirty();
                }
                TerminalCommand::ClosePane { pane_id: _ } => {
                    if self.tabs.close_pane() {
                        self.tabs.relayout(self.width, self.height)?;
                        self.tabs.mark_all_dirty();
                    }
                }
                TerminalCommand::FocusTab { index } => {
                    self.tabs.switch_to_tab(index);
                    self.tabs.mark_all_dirty();
                }
                TerminalCommand::FocusPane { pane_id } => {
                    if self.tabs.focus_pane(PaneId(pane_id)) {
                        self.tabs.mark_all_dirty();
                    }
                }
                TerminalCommand::FocusDirection { direction } => {
                    let dir = match direction.to_lowercase().as_str() {
                        "up" => crate::ui::Direction::Up,
                        "down" => crate::ui::Direction::Down,
                        "left" => crate::ui::Direction::Left,
                        _ => crate::ui::Direction::Right,
                    };
                    self.tabs.focus_direction(dir, self.width, self.height);
                    self.tabs.mark_all_dirty();
                }
                TerminalCommand::NextTab => {
                    self.tabs.next_tab();
                    self.tabs.mark_all_dirty();
                }
                TerminalCommand::PrevTab => {
                    self.tabs.prev_tab();
                    self.tabs.mark_all_dirty();
                }
                TerminalCommand::LoadSession { name } => {
                    self.load_session(&name)?;
                }
            }
        }
        Ok(())
    }

    /// Change font size and relayout
    fn change_font_size(&mut self, new_size: f32) -> Result<()> {
        info!("Changing font size to {}", new_size);

        // Update renderer font
        self.renderer.set_font_size(new_size);

        // Get new cell dimensions
        let (cell_w, cell_h) = self.renderer.cell_size();
        self.tabs.set_cell_size(cell_w, cell_h);

        // Relayout all panes with new cell size
        self.tabs.relayout(self.width, self.height)?;

        // Mark all dirty for repaint
        self.tabs.mark_all_dirty();

        Ok(())
    }

    /// Load a named session from Lua config
    fn load_session(&mut self, name: &str) -> Result<()> {
        let Some(ref runtime) = self.lua_runtime else {
            tracing::warn!("No Lua runtime, cannot load session");
            return Ok(());
        };

        let Some(session) = runtime.get_session(name) else {
            tracing::warn!("Session '{}' not found", name);
            return Ok(());
        };

        info!("Loading session '{}' with {} tabs", name, session.tabs.len());

        for tab_def in &session.tabs {
            // Create the tab with startup command (waits for OSC 133 prompt)
            let tab_id = self.tabs.new_tab_with_command(
                self.width,
                self.height,
                tab_def.cwd.as_deref(),
                tab_def.cmd.as_deref(),
            )?;

            // Set custom title if provided
            if let Some(ref title) = tab_def.title {
                self.tabs.set_tab_title(tab_id, title.clone());
            }

            // Create splits within the tab
            for split_def in &tab_def.splits {
                match split_def.direction.to_lowercase().as_str() {
                    "horizontal" | "h" => {
                        self.tabs.split_horizontal_with_command(
                            split_def.cwd.as_deref(),
                            split_def.cmd.as_deref(),
                        )?
                    }
                    _ => {
                        self.tabs.split_vertical_with_command(
                            split_def.cwd.as_deref(),
                            split_def.cmd.as_deref(),
                        )?
                    }
                };
            }
        }

        self.tabs.relayout(self.width, self.height)?;
        self.tabs.mark_all_dirty();

        // Focus first tab
        self.tabs.switch_to_tab(1);

        Ok(())
    }

    /// Get selection bounds for rendering (if selection exists and spans multiple cells)
    fn get_selection_bounds(&self) -> Option<SelectionBounds> {
        let bounds = self.selection.bounds()?;
        // Don't render single-cell selections (just a click, not a drag)
        if bounds.0.row == bounds.1.row && bounds.0.col == bounds.1.col {
            return None;
        }
        Some(SelectionBounds {
            start_row: bounds.0.row,
            start_col: bounds.0.col,
            end_row: bounds.1.row,
            end_col: bounds.1.col,
            is_block: matches!(self.selection.mode(), SelectionMode::Block),
        })
    }

    /// Convert gartk Modifiers to config Modifiers
    fn modifiers_to_config(mods: &gartk_core::Modifiers) -> ConfigModifiers {
        ConfigModifiers {
            ctrl: mods.ctrl,
            alt: mods.alt,
            shift: mods.shift,
            super_key: mods.super_key,
        }
    }

    /// Convert gartk Key to string for keybind lookup
    fn key_to_string(key: &gartk_core::Key) -> String {
        use gartk_core::Key;
        match key {
            Key::Char(c) => c.to_lowercase().to_string(),
            Key::Return => "return".into(),
            Key::Tab => "tab".into(),
            Key::Backspace => "backspace".into(),
            Key::Escape => "escape".into(),
            Key::Up => "up".into(),
            Key::Down => "down".into(),
            Key::Left => "left".into(),
            Key::Right => "right".into(),
            Key::Home => "home".into(),
            Key::End => "end".into(),
            Key::PageUp => "page_up".into(),
            Key::PageDown => "page_down".into(),
            Key::Insert => "insert".into(),
            Key::Delete => "delete".into(),
            Key::F1 => "f1".into(),
            Key::F2 => "f2".into(),
            Key::F3 => "f3".into(),
            Key::F4 => "f4".into(),
            Key::F5 => "f5".into(),
            Key::F6 => "f6".into(),
            Key::F7 => "f7".into(),
            Key::F8 => "f8".into(),
            Key::F9 => "f9".into(),
            Key::F10 => "f10".into(),
            Key::F11 => "f11".into(),
            Key::F12 => "f12".into(),
            Key::Space => "space".into(),
            Key::Unknown(_) => "unknown".into(),
        }
    }

    fn handle_signals(&mut self) -> Result<()> {
        for sig in self.signals.read_signals()? {
            match sig {
                ReceivedSignal::ChildExited { pid, status } => {
                    info!("child {} exited with status {}", pid, status);
                    // Don't exit immediately - handle_exits will clean up
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

        // Reload keybindings from TOML config
        self.keybinds = config.keybindings();
        info!("Reloaded {} keybindings from config", self.keybinds.iter().count());

        // Reload Lua runtime and merge keybinds
        match LuaRuntime::new() {
            Ok(runtime) => {
                if let Err(e) = runtime.load() {
                    tracing::warn!("Lua config error on reload: {}", e);
                }
                // Merge Lua keybinds into the keybind set
                runtime.merge_keybinds(&mut self.keybinds);
                self.lua_runtime = Some(runtime);
            }
            Err(e) => {
                tracing::warn!("Failed to create Lua runtime on reload: {}", e);
            }
        }

        // Update shell for new panes
        self.shell = config.general.shell.clone();

        // Force redraw
        self.tabs.mark_all_dirty();

        Ok(())
    }

    /// Handle IPC commands from gartermctl
    fn handle_ipc(&mut self) -> Result<()> {
        let Some(ref ipc) = self.ipc else { return Ok(()) };

        for (stream, cmd) in ipc.poll() {
            let response = self.execute_ipc_command(&cmd);
            IpcServer::send_response(stream, response);
        }

        Ok(())
    }

    /// Execute an IPC command and return the response
    fn execute_ipc_command(&mut self, cmd: &Command) -> Response {
        match cmd {
            Command::Reload => {
                match self.reload_config() {
                    Ok(()) => Response::ok(),
                    Err(e) => Response::error(format!("Failed to reload: {}", e)),
                }
            }
            Command::NewTab { cwd, startup_cmd, title: _ } => {
                let cwd_path = cwd.as_ref().map(|s| std::path::PathBuf::from(s));
                match self.tabs.new_tab_with_command(
                    self.width,
                    self.height,
                    cwd_path.as_deref(),
                    startup_cmd.as_deref(),
                ) {
                    Ok(_) => {
                        let _ = self.tabs.relayout(self.width, self.height);
                        self.tabs.mark_all_dirty();
                        Response::ok()
                    }
                    Err(e) => Response::error(format!("Failed to create tab: {}", e)),
                }
            }
            Command::CloseTab => {
                self.tabs.close_tab();
                let _ = self.tabs.relayout(self.width, self.height);
                self.tabs.mark_all_dirty();
                Response::ok()
            }
            Command::NextTab => {
                self.tabs.next_tab();
                self.tabs.mark_all_dirty();
                Response::ok()
            }
            Command::PrevTab => {
                self.tabs.prev_tab();
                self.tabs.mark_all_dirty();
                Response::ok()
            }
            Command::SwitchTab { index } => {
                self.tabs.switch_to_tab(*index);
                self.tabs.mark_all_dirty();
                Response::ok()
            }
            Command::Split { direction, cwd, startup_cmd } => {
                let cwd_path = cwd.as_ref()
                    .map(|s| std::path::PathBuf::from(s))
                    .or_else(|| self.cwd.clone());
                let result = match direction.to_lowercase().as_str() {
                    "horizontal" | "h" => {
                        self.tabs.split_horizontal_with_command(cwd_path.as_deref(), startup_cmd.as_deref())
                    }
                    "vertical" | "v" => {
                        self.tabs.split_vertical_with_command(cwd_path.as_deref(), startup_cmd.as_deref())
                    }
                    _ => return Response::error(format!("Invalid direction: {}", direction)),
                };
                match result {
                    Ok(_) => {
                        let _ = self.tabs.relayout(self.width, self.height);
                        self.tabs.mark_all_dirty();
                        Response::ok()
                    }
                    Err(e) => Response::error(format!("Failed to split: {}", e)),
                }
            }
            Command::LoadSession { name } => {
                match self.load_session(&name) {
                    Ok(()) => Response::ok(),
                    Err(e) => Response::error(format!("Failed to load session: {}", e)),
                }
            }
            Command::ClosePane => {
                if self.tabs.close_pane() {
                    let _ = self.tabs.relayout(self.width, self.height);
                    self.tabs.mark_all_dirty();
                }
                Response::ok()
            }
            Command::FocusPaneDirection { direction } => {
                let dir = match direction.to_lowercase().as_str() {
                    "up" => Direction::Up,
                    "down" => Direction::Down,
                    "left" => Direction::Left,
                    "right" => Direction::Right,
                    _ => return Response::error(format!("Invalid direction: {}", direction)),
                };
                self.tabs.focus_direction(dir, self.width, self.height);
                self.tabs.mark_all_dirty();
                Response::ok()
            }
            Command::SendText { text } => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    if let Err(e) = pane.write_pty(text.as_bytes()) {
                        return Response::error(format!("Failed to send text: {}", e));
                    }
                }
                Response::ok()
            }
            Command::GetInfo => {
                let info = serde_json::json!({
                    "tabs": self.tabs.tab_count(),
                    "width": self.width,
                    "height": self.height,
                });
                Response::ok_with_data(info)
            }
            Command::Quit => {
                self.running = false;
                Response::ok()
            }
            Command::Ping => {
                Response::ok()
            }
            Command::NewWindow { .. } | Command::ResizePane { .. } => {
                Response::error("Not implemented")
            }
        }
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
                    if let Some(pane) = self.tabs.focused_pane_mut() {
                        pane.mark_dirty();
                    }
                }

                Event::ConfigureNotify(e) => {
                    let width = e.width as u32;
                    let height = e.height as u32;

                    // Only process if size actually changed
                    if (width, height) != (self.width, self.height) {
                        self.width = width;
                        self.height = height;
                        self.renderer.resize(width, height);

                        // Relayout all panes
                        let (cell_w, cell_h) = self.renderer.cell_size();
                        self.tabs.set_cell_size(cell_w, cell_h);
                        self.tabs.relayout(width, height)?;

                        info!("Window resized to {}x{}", width, height);

                        // Force immediate re-render of all panes
                        if let Some(tab) = self.tabs.active_tab_mut() {
                            for pane in tab.panes.values_mut() {
                                pane.mark_dirty();
                            }
                        }

                        // Render tab bar + all panes
                        let tab_bar_data = self.tabs.render_tab_bar(width, height);
                        let content_offset = self.tabs.content_offset();
                        let selection_bounds = self.get_selection_bounds();
                        let pane_infos: Vec<PaneRenderInfo> = self.tabs.active_tab()
                            .map(|tab| {
                                tab.panes.values().map(|pane| PaneRenderInfo {
                                    terminal: &pane.terminal,
                                    x: pane.x,
                                    y: pane.y + content_offset,
                                    width: pane.width,
                                    height: pane.height,
                                    focused: pane.focused,
                                    selection: if pane.focused { selection_bounds } else { None },
                                    search: None, // Search rendering handled separately
                                }).collect()
                            })
                            .unwrap_or_default();

                        if !pane_infos.is_empty() {
                            // Build search overlay if search is active or has matches
                            let match_count = self.search.match_count_text();
                            let search_overlay = if self.search.active || !self.search.matches.is_empty() {
                                Some(SearchOverlay {
                                    query: &self.search.query,
                                    match_count: &match_count,
                                    active: self.search.active,
                                    case_insensitive: self.search.case_insensitive,
                                })
                            } else {
                                None
                            };
                            self.renderer.render_scene_with_search(&tab_bar_data, &pane_infos, search_overlay.as_ref())?;
                        }
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
                        // Paste the text to focused pane with bracketed paste if enabled
                        if let Some(pane) = self.tabs.focused_pane_mut() {
                            if pane.terminal.modes().bracketed_paste {
                                // Wrap with bracketed paste sequences
                                pane.write_pty(b"\x1b[200~")?;
                                pane.write_pty(text.as_bytes())?;
                                pane.write_pty(b"\x1b[201~")?;
                            } else {
                                pane.write_pty(text.as_bytes())?;
                            }
                        }
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
                    // Mark this window as focused for IPC targeting
                    if let Some(ref ipc) = self.ipc {
                        let _ = ipc.mark_focused();
                    }
                    if let Some(pane) = self.tabs.focused_pane_mut() {
                        pane.mark_dirty();
                    }
                }

                Event::FocusOut(_) => {
                    tracing::debug!("FocusOut event");
                }

                _ => {
                    tracing::trace!("Unhandled event: {:?}", event);
                }
            }
        }

        Ok(())
    }

    fn handle_key_press(&mut self, event: xproto::KeyPressEvent) -> Result<()> {
        use gartk_x11::{key_from_keycode, modifiers_from_x11};

        let modifiers = modifiers_from_x11(event.state);
        let key = key_from_keycode(event.detail, &modifiers);

        tracing::debug!("Key press: {:?}, modifiers: ctrl={}, shift={}, alt={}",
            key, modifiers.ctrl, modifiers.shift, modifiers.alt);

        // Handle search mode input
        if self.search.active {
            return self.handle_search_key(key, &modifiers);
        }

        // Convert to config modifiers and key string for lookup
        let config_mods = Self::modifiers_to_config(&modifiers);
        let key_str = Self::key_to_string(&key);

        // Check keybindings first
        if let Some(action) = self.keybinds.get(&config_mods, &key_str).cloned() {
            if self.execute_action(&action)? {
                return Ok(());
            }
        }

        // Handle Alt+1-9 for tab switching (special case - not in default keybinds)
        if modifiers.alt && !modifiers.ctrl && !modifiers.shift {
            if let gartk_core::Key::Char(c) = key {
                if let Some(n) = c.to_digit(10) {
                    if n >= 1 && n <= 9 {
                        self.tabs.switch_to_tab(n as usize);
                        self.tabs.mark_all_dirty();
                        return Ok(());
                    }
                }
                // Next/prev tab: Alt+]/[
                match c {
                    ']' => { self.tabs.next_tab(); self.tabs.mark_all_dirty(); return Ok(()); }
                    '[' => { self.tabs.prev_tab(); self.tabs.mark_all_dirty(); return Ok(()); }
                    _ => {}
                }
            }
        }

        // Get terminal modes for key translation
        let modes = self.tabs.focused_pane()
            .map(|p| *p.terminal.modes())
            .unwrap_or_default();

        // Normal key translation - send to focused pane
        if let Some(bytes) = KeyboardHandler::translate(key, &modifiers, &modes) {
            // Clear selection when sending input to terminal (like Alacritty)
            if !self.selection.is_empty() {
                self.selection.clear();
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.mark_dirty();
                }
            }
            if let Some(pane) = self.tabs.focused_pane_mut() {
                pane.write_pty(&bytes)?;
            }
        }

        Ok(())
    }

    /// Handle key events in search mode
    fn handle_search_key(&mut self, key: gartk_core::Key, modifiers: &gartk_core::Modifiers) -> Result<()> {
        use gartk_core::Key;

        match key {
            // Escape or Ctrl+C cancels search
            Key::Escape => {
                self.search.cancel();
                self.tabs.mark_all_dirty();
            }
            // Enter confirms search (exits input mode but keeps highlights)
            Key::Return => {
                self.search.confirm();
                self.tabs.mark_all_dirty();
            }
            // Backspace removes last character
            Key::Backspace => {
                self.search.pop_char();
                self.update_search_matches();
                self.tabs.mark_all_dirty();
            }
            // n/N for next/previous match (when not typing)
            Key::Char('n') if modifiers.ctrl => {
                self.search.next_match();
                self.scroll_to_current_match();
                self.tabs.mark_all_dirty();
            }
            Key::Char('n') if modifiers.shift => {
                self.search.prev_match();
                self.scroll_to_current_match();
                self.tabs.mark_all_dirty();
            }
            Key::Char('n') if !modifiers.ctrl && !modifiers.shift && !modifiers.alt => {
                if self.search.query.is_empty() {
                    self.search.push_char('n');
                    self.update_search_matches();
                } else {
                    self.search.next_match();
                    self.scroll_to_current_match();
                }
                self.tabs.mark_all_dirty();
            }
            Key::Char('N') => {
                self.search.prev_match();
                self.scroll_to_current_match();
                self.tabs.mark_all_dirty();
            }
            // Ctrl+I toggles case sensitivity
            Key::Char('i') if modifiers.ctrl => {
                self.search.toggle_case_sensitive();
                self.update_search_matches();
                self.tabs.mark_all_dirty();
            }
            // Regular characters add to query
            Key::Char(c) => {
                self.search.push_char(c);
                self.update_search_matches();
                self.tabs.mark_all_dirty();
            }
            _ => {}
        }

        Ok(())
    }

    /// Update search matches based on current query
    fn update_search_matches(&mut self) {
        if let Some(pane) = self.tabs.focused_pane() {
            let matches = pane.terminal.grid().search(
                &self.search.query,
                self.search.case_insensitive,
            );
            self.search.set_matches(matches);

            // Auto-scroll to first match
            if !self.search.matches.is_empty() {
                self.scroll_to_current_match();
            }
        }
    }

    /// Scroll viewport to show the current match
    fn scroll_to_current_match(&mut self) {
        if let Some(m) = self.search.current() {
            if let Some(pane) = self.tabs.focused_pane_mut() {
                let grid = pane.terminal.grid();
                let scrollback_len = grid.scrollback_len();

                // Calculate the scroll offset needed to show this match
                // Match row is absolute (0 = start of scrollback)
                if m.row < scrollback_len {
                    // Match is in scrollback
                    let offset = scrollback_len - m.row;
                    pane.terminal.grid_mut().set_scroll_offset(offset);
                } else {
                    // Match is in active display - scroll to bottom
                    pane.terminal.reset_viewport();
                }
                pane.mark_dirty();
            }
        }
    }

    fn handle_button_press(&mut self, event: xproto::ButtonPressEvent) -> Result<()> {
        let (cell_w, cell_h) = self.renderer.cell_size();

        // Check for tab bar click first (left click only)
        if event.detail == 1 && self.tabs.handle_click(event.event_x, event.event_y, self.width) {
            return Ok(());
        }

        // Adjust Y coordinate for tab bar offset
        let content_offset = self.tabs.content_offset() as i16;
        let adjusted_y = (event.event_y - content_offset).max(0) as f32;

        let col = (event.event_x as f32 / cell_w) as usize;
        let row = (adjusted_y / cell_h) as usize;

        let button = match event.detail {
            1 => MouseButton::Left,
            2 => MouseButton::Middle,
            3 => MouseButton::Right,
            4 => MouseButton::WheelUp,
            5 => MouseButton::WheelDown,
            _ => return Ok(()),
        };

        // Get terminal modes from focused pane
        let modes = self.tabs.focused_pane()
            .map(|p| *p.terminal.modes())
            .unwrap_or_default();

        let state: u16 = event.state.into();

        // Check mouse mode for reporting to application
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
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.write_pty(&bytes)?;
                }
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

                if let Some(pane) = self.tabs.focused_pane() {
                    match self.click_count {
                        1 => {
                            // Single click: clear any existing selection, prepare for potential drag
                            self.selection.clear();
                            let mode = if state & 0x04 != 0 {
                                SelectionMode::Block
                            } else {
                                SelectionMode::Normal
                            };
                            self.selection.start(row, col, mode);
                        }
                        2 => {
                            self.selection.select_word(row, col, pane.terminal.grid(), pane.terminal.cols());
                        }
                        _ => {
                            self.selection.select_line(row, pane.terminal.cols());
                        }
                    }
                }
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.mark_dirty();
                }
            }
            MouseButton::Middle => {
                self.clipboard.paste_primary(self.window.connection())?;
            }
            MouseButton::Right => {
                self.clipboard.paste_clipboard(self.window.connection())?;
            }
            MouseButton::WheelUp => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.terminal.scroll_up(3);
                }
            }
            MouseButton::WheelDown => {
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.terminal.scroll_down(3);
                }
            }
            MouseButton::None => {}
        }

        Ok(())
    }

    fn handle_button_release(&mut self, event: xproto::ButtonReleaseEvent) -> Result<()> {
        let (cell_w, cell_h) = self.renderer.cell_size();

        // Adjust Y coordinate for tab bar offset
        let content_offset = self.tabs.content_offset() as i16;
        let adjusted_y = (event.event_y - content_offset).max(0) as f32;

        let col = (event.event_x as f32 / cell_w) as usize;
        let row = (adjusted_y / cell_h) as usize;

        let button = match event.detail {
            1 => MouseButton::Left,
            2 => MouseButton::Middle,
            3 => MouseButton::Right,
            _ => return Ok(()),
        };

        // Get terminal modes from focused pane
        let modes = self.tabs.focused_pane()
            .map(|p| *p.terminal.modes())
            .unwrap_or_default();

        let state: u16 = event.state.into();

        // Check mouse mode for reporting
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
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.write_pty(&bytes)?;
                }
            }
        }

        // Finish selection and copy to PRIMARY
        if button == MouseButton::Left && self.selection.is_active() {
            self.selection.finish();

            // Only copy if there's an actual selection (not just a single click)
            if let Some((start, end)) = self.selection.bounds() {
                // Skip single-cell selections (just a click, not a drag)
                if start.row != end.row || start.col != end.col {
                    if let Some(pane) = self.tabs.focused_pane() {
                        let text = self.selection.get_text(pane.terminal.grid(), pane.terminal.cols());
                        if !text.is_empty() {
                            self.clipboard.copy_primary(self.window.connection(), text)?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_motion(&mut self, event: xproto::MotionNotifyEvent) -> Result<()> {
        let (cell_w, cell_h) = self.renderer.cell_size();

        // Adjust Y coordinate for tab bar offset
        let content_offset = self.tabs.content_offset() as i16;
        let adjusted_y = (event.event_y - content_offset).max(0) as f32;

        let col = (event.event_x as f32 / cell_w) as usize;
        let row = (adjusted_y / cell_h) as usize;

        // Get terminal modes from focused pane
        let modes = self.tabs.focused_pane()
            .map(|p| *p.terminal.modes())
            .unwrap_or_default();

        let state: u16 = event.state.into();

        // Check mouse mode for motion reporting
        if modes.mouse_mode != crate::terminal::MouseMode::None {
            let shift = state & 0x01 != 0;
            let alt = state & 0x08 != 0;
            let ctrl = state & 0x04 != 0;

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
                if let Some(pane) = self.tabs.focused_pane_mut() {
                    pane.write_pty(&bytes)?;
                }
            }
        }

        // Update selection during drag
        if self.selection.is_active() && state & 0x100 != 0 {
            self.selection.update(row, col);
            if let Some(pane) = self.tabs.focused_pane_mut() {
                pane.mark_dirty();
            }
        }

        Ok(())
    }

    /// Get selection for rendering
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
}
