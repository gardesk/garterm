use serde::{Deserialize, Serialize};

/// Commands sent to garterm daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Create new window
    NewWindow {
        cwd: Option<String>,
        exec: Option<String>,
    },
    /// Create new tab in focused window
    NewTab { cwd: Option<String> },
    /// Close current tab
    CloseTab,
    /// Switch to next tab
    NextTab,
    /// Switch to previous tab
    PrevTab,
    /// Switch to specific tab
    SwitchTab { index: usize },
    /// Split focused pane
    Split { direction: String },
    /// Close focused pane
    ClosePane,
    /// Focus pane in direction
    FocusPaneDirection { direction: String },
    /// Resize pane
    ResizePane { direction: String, amount: i32 },
    /// Send text to focused terminal
    SendText { text: String },
    /// Get terminal info
    GetInfo,
    /// Reload configuration
    Reload,
    /// Quit daemon
    Quit,
    /// Ping to check if instance is alive
    Ping,
}

/// Information about a garterm window instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub pid: u32,
    pub tabs: usize,
    pub focused: bool,
}

/// Response from garterm daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            success: true,
            message: None,
            data: None,
        }
    }

    pub fn ok_with_data(data: serde_json::Value) -> Self {
        Self {
            success: true,
            message: None,
            data: Some(data),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: Some(message.into()),
            data: None,
        }
    }
}

/// Events broadcast to subscribers
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Terminal title changed
    TitleChanged {
        window_id: u32,
        pane_id: u32,
        title: String,
    },
    /// Pane was created
    PaneCreated { window_id: u32, pane_id: u32 },
    /// Pane was closed
    PaneClosed {
        window_id: u32,
        pane_id: u32,
        exit_code: i32,
    },
    /// Bell received
    Bell { window_id: u32, pane_id: u32 },
}
