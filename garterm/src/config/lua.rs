//! Lua configuration loader for gar suite integration
//!
//! Loads `gar.terminal` table from `~/.config/gar/init.lua`
//! This allows power users to configure garterm alongside gar WM.

use super::{
    BellConfig, ColorsConfig, Config, FontConfig, GeneralConfig, MouseConfig,
    TerminalConfig, WindowConfig,
};
use std::path::Path;

/// Load config from Lua file (gar.terminal table)
///
/// Creates stub functions for gar.* to allow executing init.lua
/// without errors from WM-specific functions.
pub fn load_from_lua(path: &Path) -> Option<Config> {
    let lua = mlua::Lua::new();

    // Create gar table with stub functions
    if let Err(e) = create_gar_stubs(&lua) {
        tracing::warn!("Failed to create gar stubs: {}", e);
        return None;
    }

    // Load and execute the Lua file
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Could not read {}: {}", path.display(), e);
            return None;
        }
    };

    if let Err(e) = lua.load(&content).exec() {
        tracing::debug!("Error executing {}: {}", path.display(), e);
        return None;
    }

    // Check if gar.terminal exists
    let globals = lua.globals();
    let gar: mlua::Table = match globals.get("gar") {
        Ok(t) => t,
        Err(_) => return None,
    };

    let terminal: mlua::Table = match gar.get("terminal") {
        Ok(t) => t,
        Err(_) => {
            tracing::debug!("No gar.terminal table found");
            return None;
        }
    };

    // Parse the terminal table into Config
    Some(parse_terminal_table(&terminal))
}

/// Create stub functions for gar.* API
fn create_gar_stubs(lua: &mlua::Lua) -> mlua::Result<()> {
    let globals = lua.globals();

    // Create gar table
    let gar = lua.create_table()?;

    // Stub functions that do nothing (no-ops)
    let noop = lua.create_function(|_, _: mlua::MultiValue| Ok(()))?;

    gar.set("set", noop.clone())?;
    gar.set("bind", noop.clone())?;
    gar.set("exec", noop.clone())?;
    gar.set("exec_once", noop.clone())?;
    gar.set("rule", noop.clone())?;
    gar.set("picom_rule", noop.clone())?;

    // Action stubs (return nil)
    let nil_fn = lua.create_function(|_, _: mlua::MultiValue| Ok(mlua::Value::Nil))?;

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

/// Parse gar.terminal Lua table into Config
/// This is also called by runtime.rs for the persistent Lua runtime
pub fn parse_terminal_table_internal(table: &mlua::Table) -> Config {
    parse_terminal_table(table)
}

fn parse_terminal_table(table: &mlua::Table) -> Config {
    let mut config = Config::default();

    // General settings
    if let Ok(shell) = table.get::<String>("shell") {
        config.general.shell = shell;
    }
    if let Ok(vsync) = table.get::<bool>("vsync") {
        config.general.vsync = vsync;
    }
    if let Ok(cwd) = table.get::<String>("working_directory") {
        config.general.working_directory = Some(cwd.into());
    }

    // Font settings
    if let Ok(font) = table.get::<mlua::Table>("font") {
        if let Ok(family) = font.get::<String>("family") {
            config.font.family = family;
        }
        if let Ok(size) = font.get::<f32>("size") {
            config.font.size = size;
        }
        if let Ok(bold_is_bright) = font.get::<bool>("bold_is_bright") {
            config.font.bold_is_bright = bold_is_bright;
        }
        if let Ok(ligatures) = font.get::<bool>("ligatures") {
            config.font.ligatures = ligatures;
        }
    }

    // Window settings
    if let Ok(padding) = table.get::<mlua::Table>("padding") {
        if let (Ok(x), Ok(y)) = (padding.get::<u32>("x"), padding.get::<u32>("y")) {
            config.window.padding = (x, y);
        } else if let (Ok(x), Ok(y)) = (padding.get::<u32>(1), padding.get::<u32>(2)) {
            config.window.padding = (x, y);
        }
    }
    if let Ok(title) = table.get::<String>("title") {
        config.window.title = title;
    }
    if let Ok(opacity) = table.get::<f32>("opacity") {
        config.window.opacity = opacity;
    }

    // Terminal settings
    if let Ok(scrollback) = table.get::<usize>("scrollback_lines") {
        config.terminal.scrollback_lines = scrollback;
    }

    // Colors
    if let Ok(colors) = table.get::<mlua::Table>("colors") {
        if let Ok(preset) = colors.get::<String>("preset") {
            config.colors.preset = Some(preset);
        }
        // Individual color overrides would be parsed here
        parse_color_table(&colors, &mut config.colors.palette);
    }

    // Cursor settings (often under colors in Lua configs)
    if let Ok(cursor) = table.get::<mlua::Table>("cursor") {
        if let Ok(style) = cursor.get::<String>("style") {
            // Store for later use (cursor style enum)
            tracing::debug!("Cursor style from Lua: {}", style);
        }
        if let Ok(blink) = cursor.get::<bool>("blink") {
            tracing::debug!("Cursor blink from Lua: {}", blink);
        }
    }

    // Mouse settings
    if let Ok(mouse) = table.get::<mlua::Table>("mouse") {
        if let Ok(copy_on_select) = mouse.get::<bool>("copy_on_select") {
            config.mouse.copy_on_select = copy_on_select;
        }
        if let Ok(right_click) = mouse.get::<String>("right_click") {
            config.mouse.right_click = right_click;
        }
    }

    // Bell settings
    if let Ok(bell) = table.get::<mlua::Table>("bell") {
        if let Ok(visual) = bell.get::<bool>("visual") {
            config.bell.visual = visual;
        }
        if let Ok(audio) = bell.get::<bool>("audio") {
            config.bell.audio = audio;
        }
    }

    // Keybinds
    if let Ok(keybinds) = table.get::<mlua::Table>("keybinds") {
        parse_keybind_table(&keybinds, &mut config.keybinds);
    }

    // Tab bar settings
    if let Ok(tab_bar) = table.get::<mlua::Table>("tab_bar") {
        if let Ok(height) = tab_bar.get::<u32>("height") {
            config.tab_bar.height = height;
        }
        if let Ok(position) = tab_bar.get::<String>("position") {
            config.tab_bar.position = position;
        }
        if let Ok(show_single) = tab_bar.get::<bool>("show_single_tab") {
            config.tab_bar.show_single_tab = show_single;
        }
        if let Ok(max_width) = tab_bar.get::<f32>("max_tab_width") {
            config.tab_bar.max_tab_width = max_width;
        }
        if let Ok(padding) = tab_bar.get::<f32>("tab_padding") {
            config.tab_bar.tab_padding = padding;
        }
        if let Ok(shorten) = tab_bar.get::<bool>("shorten_paths") {
            config.tab_bar.shorten_paths = shorten;
        }
        // Colors as arrays [r, g, b, a] or tables {r, g, b, a}
        if let Ok(bg) = parse_color_array(&tab_bar, "background") {
            config.tab_bar.background = bg;
        }
        if let Ok(active_bg) = parse_color_array(&tab_bar, "active_bg") {
            config.tab_bar.active_bg = active_bg;
        }
        if let Ok(inactive_bg) = parse_color_array(&tab_bar, "inactive_bg") {
            config.tab_bar.inactive_bg = inactive_bg;
        }
        if let Ok(active_fg) = parse_color_array(&tab_bar, "active_fg") {
            config.tab_bar.active_fg = active_fg;
        }
        if let Ok(inactive_fg) = parse_color_array(&tab_bar, "inactive_fg") {
            config.tab_bar.inactive_fg = inactive_fg;
        }
    }

    config
}

/// Parse an RGBA color array from Lua table
fn parse_color_array(table: &mlua::Table, key: &str) -> Result<[f32; 4], mlua::Error> {
    let color: mlua::Table = table.get(key)?;
    // Try array format: {0.1, 0.2, 0.3, 1.0}
    if let (Ok(r), Ok(g), Ok(b)) = (color.get::<f32>(1), color.get::<f32>(2), color.get::<f32>(3)) {
        let a = color.get::<f32>(4).unwrap_or(1.0);
        return Ok([r, g, b, a]);
    }
    // Try table format: {r = 0.1, g = 0.2, b = 0.3, a = 1.0}
    if let (Ok(r), Ok(g), Ok(b)) = (color.get::<f32>("r"), color.get::<f32>("g"), color.get::<f32>("b")) {
        let a = color.get::<f32>("a").unwrap_or(1.0);
        return Ok([r, g, b, a]);
    }
    Err(mlua::Error::external("Invalid color format"))
}

/// Parse color values from Lua table
fn parse_color_table(table: &mlua::Table, palette: &mut super::ColorPalette) {
    macro_rules! parse_color {
        ($name:ident) => {
            if let Ok(hex) = table.get::<String>(stringify!($name)) {
                palette.$name = super::Color::hex(hex);
            }
        };
    }

    parse_color!(foreground);
    parse_color!(background);
    parse_color!(cursor);
    parse_color!(cursor_text);
    parse_color!(selection);
    parse_color!(selection_text);
    parse_color!(black);
    parse_color!(red);
    parse_color!(green);
    parse_color!(yellow);
    parse_color!(blue);
    parse_color!(magenta);
    parse_color!(cyan);
    parse_color!(white);
    parse_color!(bright_black);
    parse_color!(bright_red);
    parse_color!(bright_green);
    parse_color!(bright_yellow);
    parse_color!(bright_blue);
    parse_color!(bright_magenta);
    parse_color!(bright_cyan);
    parse_color!(bright_white);
}

/// Parse keybinds from Lua table
fn parse_keybind_table(table: &mlua::Table, config: &mut super::keybinds::KeybindConfig) {
    for pair in table.pairs::<String, mlua::Value>() {
        if let Ok((key_combo, value)) = pair {
            let action_str = match value {
                mlua::Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
                mlua::Value::Table(t) => {
                    // Handle { action = "...", ... } format
                    t.get::<String>("action").ok()
                }
                _ => None,
            };

            if let Some(action) = action_str {
                config.bindings.insert(key_combo, action);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_stubs() {
        let lua = mlua::Lua::new();
        create_gar_stubs(&lua).unwrap();

        // Should be able to call stub functions without error
        lua.load(r#"
            gar.set("test", "value")
            gar.bind("mod+Return", function() end)
            gar.exec("echo test")
        "#).exec().unwrap();
    }

    #[test]
    fn test_parse_terminal_table() {
        let lua = mlua::Lua::new();
        create_gar_stubs(&lua).unwrap();

        lua.load(r#"
            gar.terminal = {
                shell = "/bin/zsh",
                vsync = true,
                font = {
                    family = "JetBrains Mono",
                    size = 12.0,
                },
                colors = {
                    preset = "dracula",
                },
                scrollback_lines = 5000,
            }
        "#).exec().unwrap();

        let globals = lua.globals();
        let gar: mlua::Table = globals.get("gar").unwrap();
        let terminal: mlua::Table = gar.get("terminal").unwrap();

        let config = parse_terminal_table(&terminal);
        assert_eq!(config.general.shell, "/bin/zsh");
        assert!(config.general.vsync);
        assert_eq!(config.font.family, "JetBrains Mono");
        assert_eq!(config.font.size, 12.0);
        assert_eq!(config.colors.preset, Some("dracula".into()));
        assert_eq!(config.terminal.scrollback_lines, 5000);
    }
}
