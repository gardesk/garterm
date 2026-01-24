//! Persistent Lua runtime for garterm scripting
//!
//! Provides a long-lived Lua environment that supports:
//! - Function keybinds (callbacks executed on keypress)
//! - Terminal API (gar.terminal.* functions)
//! - Session definitions
//!
//! Unlike the static config loader, this runtime is kept alive
//! for the entire application lifetime to support callbacks.

use super::keybinds::{Action, Keybind, KeybindSet, Modifiers};
use super::{Config, ConfigLoader};
use mlua::{Function, Lua, RegistryKey, Result as LuaResult, Table, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

/// Terminal commands queued by Lua callbacks
#[derive(Debug, Clone)]
pub enum TerminalCommand {
    NewTab {
        cwd: Option<String>,
        cmd: Option<String>,
        title: Option<String>,
    },
    Split {
        direction: String,
        cwd: Option<String>,
        cmd: Option<String>,
    },
    SendText {
        pane_id: Option<u32>,
        text: String,
    },
    CloseTab {
        tab_id: Option<u32>,
    },
    ClosePane {
        pane_id: Option<u32>,
    },
    FocusTab {
        index: usize,
    },
    FocusPane {
        pane_id: u32,
    },
    FocusDirection {
        direction: String,
    },
    NextTab,
    PrevTab,
    LoadSession {
        name: String,
    },
}

/// Session definition parsed from Lua
#[derive(Debug, Clone)]
pub struct Session {
    pub name: String,
    pub tabs: Vec<SessionTab>,
}

/// Tab within a session
#[derive(Debug, Clone)]
pub struct SessionTab {
    pub title: Option<String>,
    pub cwd: Option<PathBuf>,
    pub cmd: Option<String>,
    pub splits: Vec<SessionSplit>,
}

/// Split within a session tab
#[derive(Debug, Clone)]
pub struct SessionSplit {
    pub direction: String,
    pub cwd: Option<PathBuf>,
    pub cmd: Option<String>,
}

/// Shared state between Rust and Lua
pub struct LuaState {
    /// Registered Lua function callbacks
    pub callbacks: Vec<RegistryKey>,
    /// Parsed session definitions
    pub sessions: HashMap<String, Session>,
    /// Pending terminal commands from Lua callbacks
    pub pending_commands: Vec<TerminalCommand>,
    /// Keybinds parsed from Lua (separate from TOML keybinds)
    pub lua_keybinds: Vec<(String, LuaKeybind)>,
    /// Last assigned IDs for return values
    pub last_tab_id: u32,
    pub last_pane_id: u32,
}

impl Default for LuaState {
    fn default() -> Self {
        Self {
            callbacks: Vec::new(),
            sessions: HashMap::new(),
            pending_commands: Vec::new(),
            lua_keybinds: Vec::new(),
            last_tab_id: 0,
            last_pane_id: 0,
        }
    }
}

/// Type of keybind action from Lua
#[derive(Debug, Clone)]
pub enum LuaKeybind {
    /// String action name: "new_tab", "copy", etc.
    Action(String),
    /// Index into callbacks vector
    Callback(usize),
    /// Load a named session
    SessionLoad(String),
}

/// Persistent Lua runtime
pub struct LuaRuntime {
    lua: Lua,
    state: Arc<Mutex<LuaState>>,
    lua_path: PathBuf,
}

impl LuaRuntime {
    /// Create a new Lua runtime
    pub fn new() -> LuaResult<Self> {
        let lua = Lua::new();
        let state = Arc::new(Mutex::new(LuaState::default()));

        let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
        let lua_path = config_dir.join("gar/init.lua");

        Ok(Self {
            lua,
            state,
            lua_path,
        })
    }

    /// Load and execute the Lua config file
    pub fn load(&self) -> LuaResult<Option<Config>> {
        // Set up gar.* stubs for WM compatibility
        self.setup_gar_stubs()?;

        // Register gar.terminal.* API
        self.register_terminal_api()?;

        // Load the config file if it exists
        if !self.lua_path.exists() {
            debug!("No Lua config at {}", self.lua_path.display());
            return Ok(None);
        }

        let content = match std::fs::read_to_string(&self.lua_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {}: {}", self.lua_path.display(), e);
                return Ok(None);
            }
        };

        // Execute the Lua file
        if let Err(e) = self.lua.load(&content).exec() {
            error!("Lua config error: {}", e);
            return Ok(None);
        }

        info!("Loaded Lua config from {}", self.lua_path.display());

        // Parse gar.terminal table into Config
        let config = self.parse_terminal_config()?;

        // Parse sessions
        self.parse_sessions()?;

        // Parse keybinds (including function callbacks)
        self.parse_keybinds()?;

        Ok(config)
    }

    /// Set up gar.* stub functions for WM compatibility
    fn setup_gar_stubs(&self) -> LuaResult<()> {
        let globals = self.lua.globals();
        let gar = self.lua.create_table()?;

        // No-op functions that accept any arguments
        let noop = self.lua.create_function(|_, _: mlua::MultiValue| Ok(()))?;

        gar.set("set", noop.clone())?;
        gar.set("bind", noop.clone())?;
        gar.set("exec", noop.clone())?;
        gar.set("exec_once", noop.clone())?;
        gar.set("rule", noop.clone())?;
        gar.set("picom_rule", noop.clone())?;

        // Action functions that return nil
        let nil_fn = self.lua.create_function(|_, _: mlua::MultiValue| Ok(Value::Nil))?;

        gar.set("focus", nil_fn.clone())?;
        gar.set("swap", nil_fn.clone())?;
        gar.set("resize", nil_fn.clone())?;
        gar.set("workspace", nil_fn.clone())?;
        gar.set("workspace_next", nil_fn.clone())?;
        gar.set("workspace_prev", nil_fn.clone())?;
        gar.set("move_to_workspace", nil_fn.clone())?;
        gar.set("focus_monitor", nil_fn.clone())?;
        gar.set("move_to_monitor", nil_fn.clone())?;
        gar.set("close_window", nil_fn.clone())?;
        gar.set("force_close_window", nil_fn.clone())?;
        gar.set("exit", nil_fn.clone())?;
        gar.set("reload", nil_fn.clone())?;
        gar.set("equalize", nil_fn.clone())?;
        gar.set("toggle_floating", nil_fn.clone())?;
        gar.set("toggle_fullscreen", nil_fn.clone())?;

        globals.set("gar", gar)?;
        Ok(())
    }

    /// Register gar.terminal.* API functions
    fn register_terminal_api(&self) -> LuaResult<()> {
        let globals = self.lua.globals();
        let gar: Table = globals.get("gar")?;
        let terminal = self.lua.create_table()?;

        // gar.terminal.new_tab({ cwd = "...", cmd = "...", title = "..." })
        let state = Arc::clone(&self.state);
        let new_tab = self.lua.create_function(move |_, opts: Option<Table>| {
            let (cwd, cmd, title) = if let Some(t) = opts {
                (
                    t.get::<Option<String>>("cwd").ok().flatten(),
                    t.get::<Option<String>>("cmd").ok().flatten(),
                    t.get::<Option<String>>("title").ok().flatten(),
                )
            } else {
                (None, None, None)
            };

            let mut state = state.lock().unwrap();
            state.last_tab_id += 1;
            let tab_id = state.last_tab_id;
            state.pending_commands.push(TerminalCommand::NewTab { cwd, cmd, title });
            Ok(tab_id)
        })?;
        terminal.set("new_tab", new_tab)?;

        // gar.terminal.split({ direction = "horizontal", cwd = "...", cmd = "..." })
        let state = Arc::clone(&self.state);
        let split = self.lua.create_function(move |_, opts: Option<Table>| {
            let (direction, cwd, cmd) = if let Some(t) = opts {
                (
                    t.get::<String>("direction").unwrap_or_else(|_| "vertical".into()),
                    t.get::<Option<String>>("cwd").ok().flatten(),
                    t.get::<Option<String>>("cmd").ok().flatten(),
                )
            } else {
                ("vertical".into(), None, None)
            };

            let mut state = state.lock().unwrap();
            state.last_pane_id += 1;
            let pane_id = state.last_pane_id;
            state.pending_commands.push(TerminalCommand::Split { direction, cwd, cmd });
            Ok(pane_id)
        })?;
        terminal.set("split", split)?;

        // gar.terminal.send_text(pane_id, text) or gar.terminal.send_text(text)
        let state = Arc::clone(&self.state);
        let send_text = self.lua.create_function(move |_, args: mlua::MultiValue| {
            let args: Vec<Value> = args.into_iter().collect();
            let (pane_id, text) = match args.len() {
                1 => {
                    // send_text("text") - send to focused pane
                    let text = match &args[0] {
                        Value::String(s) => s.to_str()?.to_string(),
                        _ => return Err(mlua::Error::runtime("expected string")),
                    };
                    (None, text)
                }
                2 => {
                    // send_text(pane_id, "text")
                    let pane_id = match &args[0] {
                        Value::Integer(n) => Some(*n as u32),
                        Value::Nil => None,
                        _ => return Err(mlua::Error::runtime("expected pane_id or nil")),
                    };
                    let text = match &args[1] {
                        Value::String(s) => s.to_str()?.to_string(),
                        _ => return Err(mlua::Error::runtime("expected string")),
                    };
                    (pane_id, text)
                }
                _ => return Err(mlua::Error::runtime("expected 1 or 2 arguments")),
            };

            let mut state = state.lock().unwrap();
            state.pending_commands.push(TerminalCommand::SendText { pane_id, text });
            Ok(())
        })?;
        terminal.set("send_text", send_text)?;

        // gar.terminal.close_tab(tab_id?)
        let state = Arc::clone(&self.state);
        let close_tab = self.lua.create_function(move |_, tab_id: Option<u32>| {
            let mut state = state.lock().unwrap();
            state.pending_commands.push(TerminalCommand::CloseTab { tab_id });
            Ok(())
        })?;
        terminal.set("close_tab", close_tab)?;

        // gar.terminal.close_pane(pane_id?)
        let state = Arc::clone(&self.state);
        let close_pane = self.lua.create_function(move |_, pane_id: Option<u32>| {
            let mut state = state.lock().unwrap();
            state.pending_commands.push(TerminalCommand::ClosePane { pane_id });
            Ok(())
        })?;
        terminal.set("close_pane", close_pane)?;

        // gar.terminal.focus_tab(n)
        let state = Arc::clone(&self.state);
        let focus_tab = self.lua.create_function(move |_, index: usize| {
            let mut state = state.lock().unwrap();
            state.pending_commands.push(TerminalCommand::FocusTab { index });
            Ok(())
        })?;
        terminal.set("focus_tab", focus_tab)?;

        // gar.terminal.focus_pane(pane_id)
        let state = Arc::clone(&self.state);
        let focus_pane = self.lua.create_function(move |_, pane_id: u32| {
            let mut state = state.lock().unwrap();
            state.pending_commands.push(TerminalCommand::FocusPane { pane_id });
            Ok(())
        })?;
        terminal.set("focus_pane", focus_pane)?;

        // gar.terminal.focus_direction(dir)
        let state = Arc::clone(&self.state);
        let focus_direction = self.lua.create_function(move |_, direction: String| {
            let mut state = state.lock().unwrap();
            state.pending_commands.push(TerminalCommand::FocusDirection { direction });
            Ok(())
        })?;
        terminal.set("focus_direction", focus_direction)?;

        // gar.terminal.next_tab()
        let state = Arc::clone(&self.state);
        let next_tab = self.lua.create_function(move |_, ()| {
            let mut state = state.lock().unwrap();
            state.pending_commands.push(TerminalCommand::NextTab);
            Ok(())
        })?;
        terminal.set("next_tab", next_tab)?;

        // gar.terminal.prev_tab()
        let state = Arc::clone(&self.state);
        let prev_tab = self.lua.create_function(move |_, ()| {
            let mut state = state.lock().unwrap();
            state.pending_commands.push(TerminalCommand::PrevTab);
            Ok(())
        })?;
        terminal.set("prev_tab", prev_tab)?;

        // gar.terminal.load_session(name)
        let state = Arc::clone(&self.state);
        let load_session = self.lua.create_function(move |_, name: String| {
            let mut state = state.lock().unwrap();
            state.pending_commands.push(TerminalCommand::LoadSession { name });
            Ok(())
        })?;
        terminal.set("load_session", load_session)?;

        gar.set("terminal", terminal)?;
        Ok(())
    }

    /// Parse gar.terminal table into Config (static settings only)
    fn parse_terminal_config(&self) -> LuaResult<Option<Config>> {
        let globals = self.lua.globals();
        let gar: Table = match globals.get("gar") {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };

        let terminal: Table = match gar.get("terminal") {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };

        // Delegate to existing lua.rs parsing logic
        // This reuses the static config parsing
        let config = super::lua::parse_terminal_table_internal(&terminal);
        Ok(Some(config))
    }

    /// Parse session definitions from gar.terminal.sessions
    fn parse_sessions(&self) -> LuaResult<()> {
        let globals = self.lua.globals();
        let gar: Table = globals.get("gar")?;
        let terminal: Table = match gar.get("terminal") {
            Ok(t) => t,
            Err(_) => return Ok(()),
        };

        let sessions: Table = match terminal.get("sessions") {
            Ok(t) => t,
            Err(_) => return Ok(()),
        };

        let mut state = self.state.lock().unwrap();

        for pair in sessions.pairs::<String, Table>() {
            let (name, session_table) = pair?;
            if let Ok(session) = self.parse_session(&name, &session_table) {
                debug!("Parsed session: {}", name);
                state.sessions.insert(name, session);
            }
        }

        info!("Loaded {} sessions", state.sessions.len());
        Ok(())
    }

    /// Parse a single session definition
    fn parse_session(&self, name: &str, table: &Table) -> LuaResult<Session> {
        let mut tabs = Vec::new();

        if let Ok(tabs_table) = table.get::<Table>("tabs") {
            for i in 1..=tabs_table.len()? {
                if let Ok(tab_table) = tabs_table.get::<Table>(i) {
                    tabs.push(self.parse_session_tab(&tab_table)?);
                }
            }
        }

        Ok(Session {
            name: name.to_string(),
            tabs,
        })
    }

    /// Parse a session tab definition
    fn parse_session_tab(&self, table: &Table) -> LuaResult<SessionTab> {
        let title = table.get::<Option<String>>("title").ok().flatten();
        let cwd = table.get::<Option<String>>("cwd").ok().flatten().map(PathBuf::from);
        let cmd = table.get::<Option<String>>("cmd").ok().flatten();

        let mut splits = Vec::new();
        if let Ok(splits_table) = table.get::<Table>("splits") {
            for i in 1..=splits_table.len()? {
                if let Ok(split_table) = splits_table.get::<Table>(i) {
                    splits.push(self.parse_session_split(&split_table)?);
                }
            }
        }

        Ok(SessionTab { title, cwd, cmd, splits })
    }

    /// Parse a session split definition
    fn parse_session_split(&self, table: &Table) -> LuaResult<SessionSplit> {
        let direction = table.get::<String>("direction").unwrap_or_else(|_| "vertical".into());
        let cwd = table.get::<Option<String>>("cwd").ok().flatten().map(PathBuf::from);
        let cmd = table.get::<Option<String>>("cmd").ok().flatten();

        Ok(SessionSplit { direction, cwd, cmd })
    }

    /// Parse keybinds from gar.terminal.keybinds (supports functions!)
    fn parse_keybinds(&self) -> LuaResult<()> {
        let globals = self.lua.globals();
        let gar: Table = globals.get("gar")?;
        let terminal: Table = match gar.get("terminal") {
            Ok(t) => t,
            Err(_) => return Ok(()),
        };

        let keybinds: Table = match terminal.get("keybinds") {
            Ok(t) => t,
            Err(_) => return Ok(()),
        };

        for pair in keybinds.pairs::<String, Value>() {
            let (key_combo, value) = pair?;

            let lua_keybind = match value {
                Value::String(s) => {
                    // String action: "new_tab", "copy", etc.
                    LuaKeybind::Action(s.to_str()?.to_string())
                }
                Value::Function(f) => {
                    // Lua function callback - store in registry
                    let key = self.lua.create_registry_value(f)?;
                    let mut state = self.state.lock().unwrap();
                    let index = state.callbacks.len();
                    state.callbacks.push(key);
                    drop(state);
                    debug!("Registered Lua callback {} for {}", index, key_combo);
                    LuaKeybind::Callback(index)
                }
                Value::Table(t) => {
                    // Action table: { action = "load_session", session = "webdev" }
                    if let Ok(session) = t.get::<String>("session") {
                        LuaKeybind::SessionLoad(session)
                    } else if let Ok(action) = t.get::<String>("action") {
                        LuaKeybind::Action(action)
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };

            let mut state = self.state.lock().unwrap();
            state.lua_keybinds.push((key_combo.clone(), lua_keybind));
        }

        let state = self.state.lock().unwrap();
        info!("Loaded {} Lua keybinds ({} callbacks)",
              state.lua_keybinds.len(),
              state.callbacks.len());
        Ok(())
    }

    /// Execute a registered callback by index
    pub fn execute_callback(&self, index: usize) -> LuaResult<()> {
        let state = self.state.lock().unwrap();
        let key = state.callbacks.get(index)
            .ok_or_else(|| mlua::Error::runtime(format!("callback {} not found", index)))?;

        let func: Function = self.lua.registry_value(key)?;
        drop(state); // Release lock before calling Lua

        func.call::<()>(())?;
        Ok(())
    }

    /// Take pending commands (drains the queue)
    pub fn take_pending_commands(&self) -> Vec<TerminalCommand> {
        let mut state = self.state.lock().unwrap();
        std::mem::take(&mut state.pending_commands)
    }

    /// Get a session by name
    pub fn get_session(&self, name: &str) -> Option<Session> {
        let state = self.state.lock().unwrap();
        state.sessions.get(name).cloned()
    }

    /// Get Lua keybinds to merge with config keybinds
    pub fn get_lua_keybinds(&self) -> Vec<(String, LuaKeybind)> {
        let state = self.state.lock().unwrap();
        state.lua_keybinds.clone()
    }

    /// Check if we have any Lua callbacks (for feature detection)
    pub fn has_callbacks(&self) -> bool {
        let state = self.state.lock().unwrap();
        !state.callbacks.is_empty()
    }

    /// Merge Lua keybinds into an existing KeybindSet
    ///
    /// This converts LuaKeybind variants to Action variants and adds them
    /// to the keybind set, overriding any existing bindings.
    pub fn merge_keybinds(&self, keybinds: &mut KeybindSet) {
        let state = self.state.lock().unwrap();

        for (key_combo, lua_bind) in &state.lua_keybinds {
            // Parse the key combo
            let Some((modifiers, key)) = Keybind::parse_key_combo(key_combo) else {
                warn!("Invalid key combo from Lua: {}", key_combo);
                continue;
            };

            // Convert LuaKeybind to Action
            let action = match lua_bind {
                LuaKeybind::Action(s) => {
                    // Parse string action
                    if let Some(a) = Action::from_str_loose(s) {
                        a
                    } else {
                        warn!("Unknown action '{}' for Lua keybind '{}'", s, key_combo);
                        continue;
                    }
                }
                LuaKeybind::Callback(index) => Action::LuaCallback(*index),
                LuaKeybind::SessionLoad(name) => Action::LoadSession(name.clone()),
            };

            // Add to keybind set (overrides existing)
            if let Some(old) = keybinds.add(Keybind::new(modifiers, key, action)) {
                if old.action != Action::None {
                    debug!("Lua keybind {} overrides {:?}", key_combo, old.action);
                }
            }
        }

        info!("Merged {} Lua keybinds", state.lua_keybinds.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_creation() {
        let runtime = LuaRuntime::new().unwrap();
        assert!(!runtime.has_callbacks());
    }

    #[test]
    fn test_terminal_api_registration() {
        let runtime = LuaRuntime::new().unwrap();
        runtime.setup_gar_stubs().unwrap();
        runtime.register_terminal_api().unwrap();

        // Execute Lua that calls the API
        runtime.lua.load(r#"
            local tab = gar.terminal.new_tab({ cwd = "/tmp", title = "Test" })
            gar.terminal.send_text(tab, "echo hello\n")
        "#).exec().unwrap();

        let commands = runtime.take_pending_commands();
        assert_eq!(commands.len(), 2);
    }

    #[test]
    fn test_session_parsing() {
        let runtime = LuaRuntime::new().unwrap();
        runtime.setup_gar_stubs().unwrap();
        runtime.register_terminal_api().unwrap();

        runtime.lua.load(r#"
            gar.terminal.sessions = {
                webdev = {
                    tabs = {
                        { title = "Frontend", cwd = "~/app", cmd = "npm run dev" },
                        { title = "Backend", cmd = "python manage.py runserver" },
                    }
                }
            }
        "#).exec().unwrap();

        runtime.parse_sessions().unwrap();

        let session = runtime.get_session("webdev").unwrap();
        assert_eq!(session.tabs.len(), 2);
        assert_eq!(session.tabs[0].title, Some("Frontend".into()));
        assert_eq!(session.tabs[0].cmd, Some("npm run dev".into()));
    }

    #[test]
    fn test_function_keybind() {
        let runtime = LuaRuntime::new().unwrap();
        runtime.setup_gar_stubs().unwrap();
        runtime.register_terminal_api().unwrap();

        runtime.lua.load(r#"
            gar.terminal.keybinds = {
                ["alt+t"] = function()
                    gar.terminal.new_tab({ title = "From callback" })
                end
            }
        "#).exec().unwrap();

        runtime.parse_keybinds().unwrap();

        assert!(runtime.has_callbacks());

        // Execute the callback
        runtime.execute_callback(0).unwrap();

        let commands = runtime.take_pending_commands();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            TerminalCommand::NewTab { title, .. } => {
                assert_eq!(title.as_deref(), Some("From callback"));
            }
            _ => panic!("expected NewTab command"),
        }
    }
}
