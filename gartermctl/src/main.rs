//! gartermctl - Control garterm terminal emulator
//!
//! Supports multiple garterm windows via PID-based sockets.
//! By default, targets the last focused window.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use garterm_ipc::{Command, Response, WindowInfo};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "gartermctl")]
#[command(about = "Control garterm terminal emulator")]
#[command(version)]
struct Cli {
    /// Target a specific window by PID
    #[arg(long, short = 'w')]
    window: Option<u32>,

    /// Send command to all running garterm instances
    #[arg(long, short = 'a')]
    all: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all running garterm instances
    List,
    /// Create a new tab
    NewTab {
        /// Working directory
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Close the current tab
    CloseTab,
    /// Switch to next tab
    NextTab,
    /// Switch to previous tab
    PrevTab,
    /// Switch to a specific tab (1-indexed)
    Tab {
        /// Tab number (1-indexed)
        index: usize,
    },
    /// Split the focused pane
    Split {
        /// Split horizontally (side-by-side)
        #[arg(long, short = 'H')]
        horizontal: bool,
    },
    /// Close the focused pane
    ClosePane,
    /// Focus pane in direction
    Focus {
        /// Direction: up, down, left, right
        direction: String,
    },
    /// Send text to the focused pane
    Send {
        /// Text to send
        text: String,
    },
    /// Get terminal info
    Info,
    /// Reload configuration
    Reload,
    /// Quit garterm
    Quit,
}

/// Get the runtime directory for garterm sockets
fn runtime_dir() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("garterm")
    } else {
        PathBuf::from("/tmp").join("garterm")
    }
}

/// Get socket path for a specific PID
fn socket_path_for_pid(pid: u32) -> PathBuf {
    runtime_dir().join(format!("garterm-{}.sock", pid))
}

/// Get the path to the focus tracking file
fn focus_file_path() -> PathBuf {
    runtime_dir().join("focused")
}

/// Check if a process exists
fn process_exists(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// List all running garterm instances by checking socket files
fn list_instances() -> Vec<u32> {
    let runtime_dir = runtime_dir();
    let mut pids = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&runtime_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("garterm-") && name.ends_with(".sock") {
                if let Some(pid_str) = name.strip_prefix("garterm-").and_then(|s| s.strip_suffix(".sock")) {
                    if let Ok(pid) = pid_str.parse::<u32>() {
                        // Verify the process still exists
                        if process_exists(pid) {
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

/// Get the PID of the currently focused garterm window
fn get_focused_pid() -> Option<u32> {
    let focus_path = focus_file_path();
    std::fs::read_to_string(&focus_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&pid| process_exists(pid))
}

/// Send a command to a specific PID and return the response
fn send_command_to_pid(pid: u32, cmd: &Command) -> Result<Response> {
    let socket = socket_path_for_pid(pid);
    let mut stream = UnixStream::connect(&socket)
        .with_context(|| format!("Failed to connect to garterm {} at {}", pid, socket.display()))?;

    // Send command as JSON
    let json = serde_json::to_string(&cmd)?;
    writeln!(stream, "{}", json)?;

    // Read response
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let response: Response = serde_json::from_str(&line)
        .with_context(|| format!("Invalid response: {}", line.trim()))?;

    Ok(response)
}

/// Get info from a garterm instance (tabs, focused status)
fn get_window_info(pid: u32, focused_pid: Option<u32>) -> Option<WindowInfo> {
    let cmd = Command::Ping;
    if send_command_to_pid(pid, &cmd).is_ok() {
        // Get actual tab count via GetInfo
        let info_cmd = Command::GetInfo;
        let tabs = if let Ok(response) = send_command_to_pid(pid, &info_cmd) {
            response.data
                .and_then(|d| d.get("tabs").and_then(|v| v.as_u64()))
                .unwrap_or(1) as usize
        } else {
            1
        };

        Some(WindowInfo {
            pid,
            tabs,
            focused: focused_pid == Some(pid),
        })
    } else {
        None
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle list command specially - doesn't target any window
    if matches!(cli.command, Commands::List) {
        let instances = list_instances();
        let focused = get_focused_pid();

        if instances.is_empty() {
            println!("No garterm instances running");
        } else {
            println!("PID\tTABS\tFOCUSED");
            for pid in instances {
                if let Some(info) = get_window_info(pid, focused) {
                    println!("{}\t{}\t{}", info.pid, info.tabs, if info.focused { "*" } else { "" });
                }
            }
        }
        return Ok(());
    }

    // Convert command enum to IPC Command
    let cmd = match &cli.command {
        Commands::List => unreachable!(),
        Commands::NewTab { cwd } => Command::NewTab { cwd: cwd.clone() },
        Commands::CloseTab => Command::CloseTab,
        Commands::NextTab => Command::NextTab,
        Commands::PrevTab => Command::PrevTab,
        Commands::Tab { index } => Command::SwitchTab { index: *index },
        Commands::Split { horizontal } => Command::Split {
            direction: if *horizontal { "horizontal".into() } else { "vertical".into() },
        },
        Commands::ClosePane => Command::ClosePane,
        Commands::Focus { direction } => Command::FocusPaneDirection { direction: direction.clone() },
        Commands::Send { text } => Command::SendText { text: text.clone() },
        Commands::Info => Command::GetInfo,
        Commands::Reload => Command::Reload,
        Commands::Quit => Command::Quit,
    };

    // Determine target(s)
    let targets: Vec<u32> = if cli.all {
        list_instances()
    } else if let Some(pid) = cli.window {
        if !process_exists(pid) {
            eprintln!("Error: No garterm instance with PID {}", pid);
            std::process::exit(1);
        }
        vec![pid]
    } else {
        // Default: target last focused window
        match get_focused_pid() {
            Some(pid) => vec![pid],
            None => {
                // Fall back to first available instance
                let instances = list_instances();
                if instances.is_empty() {
                    eprintln!("Error: No garterm instances running");
                    std::process::exit(1);
                }
                vec![instances[0]]
            }
        }
    };

    if targets.is_empty() {
        eprintln!("Error: No garterm instances running");
        std::process::exit(1);
    }

    // Send command to all targets
    let mut any_success = false;
    let mut errors = Vec::new();

    for pid in &targets {
        match send_command_to_pid(*pid, &cmd) {
            Ok(response) => {
                if response.success {
                    any_success = true;
                    if targets.len() > 1 {
                        // Multi-target: prefix output with PID
                        if let Some(data) = response.data {
                            println!("[{}] {}", pid, serde_json::to_string_pretty(&data)?);
                        } else if let Some(msg) = response.message {
                            println!("[{}] {}", pid, msg);
                        }
                    } else {
                        // Single target: normal output
                        if let Some(data) = response.data {
                            println!("{}", serde_json::to_string_pretty(&data)?);
                        } else if let Some(msg) = response.message {
                            println!("{}", msg);
                        }
                    }
                } else {
                    let msg = response.message.unwrap_or_else(|| "Unknown error".into());
                    if targets.len() > 1 {
                        errors.push(format!("[{}] {}", pid, msg));
                    } else {
                        errors.push(msg);
                    }
                }
            }
            Err(e) => {
                if targets.len() > 1 {
                    errors.push(format!("[{}] {}", pid, e));
                } else {
                    errors.push(e.to_string());
                }
            }
        }
    }

    // Report errors
    for error in &errors {
        eprintln!("Error: {}", error);
    }

    if !any_success && !errors.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}
