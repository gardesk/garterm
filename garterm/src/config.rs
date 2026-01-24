/// Terminal configuration
///
/// This is a minimal config struct for now. It will be extended with
/// Lua/TOML loading in a future sprint.

#[derive(Debug, Clone)]
pub struct Config {
    /// Use VSync-based rendering (dirty flag only).
    ///
    /// When false (default), renders continuously at ~60fps using timer-based
    /// frame pacing. This is required on Asahi Linux where VBlank interrupts
    /// don't work, and is safe to use on all platforms.
    ///
    /// When true, only renders when terminal content changes. This is more
    /// efficient but may cause display issues on systems without proper
    /// VSync/damage reporting support (notably Asahi Linux).
    pub vsync: bool,

    /// Shell command to execute
    pub shell: String,

    /// Working directory
    pub working_directory: Option<std::path::PathBuf>,

    /// Font size in points
    pub font_size: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Default to continuous rendering - works everywhere
            vsync: false,
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
            working_directory: None,
            font_size: 14.0,
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_vsync(mut self, vsync: bool) -> Self {
        self.vsync = vsync;
        self
    }

    pub fn with_shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = shell.into();
        self
    }

    pub fn with_working_directory(mut self, path: Option<std::path::PathBuf>) -> Self {
        self.working_directory = path;
        self
    }

    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }
}
