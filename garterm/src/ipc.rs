//! IPC server for garterm
//!
//! Listens on a Unix socket for commands from gartermctl.

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
}

impl IpcServer {
    /// Create a new IPC server
    pub fn new() -> Result<Self> {
        let socket_path = Self::socket_path();

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

        info!("IPC server listening on {}", socket_path.display());

        Ok(Self {
            listener,
            socket_path,
        })
    }

    /// Get the socket path
    pub fn socket_path() -> PathBuf {
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            PathBuf::from(runtime_dir).join("garterm.sock")
        } else {
            PathBuf::from("/tmp").join(format!("garterm-{}.sock", std::process::id()))
        }
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
