//! IPC server for garterm
//!
//! Each garterm instance creates its own socket: garterm-{pid}.sock
//! A focus file tracks the last focused window for default targeting.

use anyhow::Result;
use garterm_ipc::{Command, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use tracing::{debug, error, info, warn};

/// IPC server listening for commands
pub struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
    pid: u32,
}

impl IpcServer {
    /// Create a new IPC server with PID-based socket
    pub fn new() -> Result<Self> {
        let pid = std::process::id();
        let socket_path = Self::socket_path_for_pid(pid);

        // Remove old socket if it exists
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }

        // Create parent directory if needed
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Bind to socket
        let listener = UnixListener::bind(&socket_path)?;
        listener.set_nonblocking(true)?;

        info!("IPC server listening on {} (PID {})", socket_path.display(), pid);

        Ok(Self {
            listener,
            socket_path,
            pid,
        })
    }

    /// Get the runtime directory for garterm sockets
    pub fn runtime_dir() -> PathBuf {
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            PathBuf::from(runtime_dir).join("garterm")
        } else {
            PathBuf::from("/tmp").join("garterm")
        }
    }

    /// Get socket path for a specific PID
    pub fn socket_path_for_pid(pid: u32) -> PathBuf {
        Self::runtime_dir().join(format!("garterm-{}.sock", pid))
    }

    /// Get the path to the focus tracking file
    pub fn focus_file_path() -> PathBuf {
        Self::runtime_dir().join("focused")
    }

    /// Mark this window as focused (called on FocusIn events)
    pub fn mark_focused(&self) -> Result<()> {
        let focus_path = Self::focus_file_path();

        // Create parent directory if needed
        if let Some(parent) = focus_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write our PID to the focus file
        std::fs::write(&focus_path, self.pid.to_string())?;
        debug!("Marked as focused window (PID {})", self.pid);
        Ok(())
    }

    /// Get the PID of the currently focused garterm window
    pub fn get_focused_pid() -> Option<u32> {
        let focus_path = Self::focus_file_path();
        std::fs::read_to_string(&focus_path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    /// List all running garterm instances (by checking socket files)
    pub fn list_instances() -> Vec<u32> {
        let runtime_dir = Self::runtime_dir();
        let mut pids = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&runtime_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("garterm-") && name.ends_with(".sock") {
                    if let Some(pid_str) = name.strip_prefix("garterm-").and_then(|s| s.strip_suffix(".sock")) {
                        if let Ok(pid) = pid_str.parse::<u32>() {
                            // Verify the process still exists
                            if Self::process_exists(pid) {
                                pids.push(pid);
                            } else {
                                // Clean up stale socket
                                let _ = std::fs::remove_file(entry.path());
                            }
                        }
                    }
                }
            }
        }

        pids
    }

    /// Check if a process exists
    fn process_exists(pid: u32) -> bool {
        // Use kill(0) to check if process exists
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    /// Get this server's PID
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Check for incoming connections and return any received commands
    pub fn poll(&self) -> Vec<(UnixStream, Command)> {
        let mut commands = Vec::new();

        loop {
            match self.listener.accept() {
                Ok((stream, _addr)) => {
                    debug!("IPC client connected");
                    if let Some(cmd) = self.read_command(&stream) {
                        commands.push((stream, cmd));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(e) => {
                    warn!("IPC accept error: {}", e);
                    break;
                }
            }
        }

        commands
    }

    /// Read a command from a client
    fn read_command(&self, stream: &UnixStream) -> Option<Command> {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();

        match reader.read_line(&mut line) {
            Ok(0) => None, // EOF
            Ok(_) => {
                match serde_json::from_str(&line) {
                    Ok(cmd) => Some(cmd),
                    Err(e) => {
                        error!("Failed to parse IPC command: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                error!("Failed to read IPC command: {}", e);
                None
            }
        }
    }

    /// Send a response to a client
    pub fn send_response(mut stream: UnixStream, response: Response) {
        let json = match serde_json::to_string(&response) {
            Ok(j) => j,
            Err(e) => {
                error!("Failed to serialize IPC response: {}", e);
                return;
            }
        };

        if let Err(e) = writeln!(stream, "{}", json) {
            error!("Failed to send IPC response: {}", e);
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        // Clean up socket file
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}
