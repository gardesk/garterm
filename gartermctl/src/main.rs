//! gartermctl - Control garterm terminal emulator

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use garterm_ipc::{Command, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "gartermctl")]
#[command(about = "Control garterm terminal emulator")]
#[command(version)]
struct Cli {
    /// Socket path (default: $XDG_RUNTIME_DIR/garterm.sock)
    #[arg(long, short)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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

fn socket_path(cli_path: Option<PathBuf>) -> PathBuf {
    if let Some(path) = cli_path {
        return path;
    }

    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("garterm.sock")
    } else {
        // Try to find any garterm socket in /tmp
        PathBuf::from("/tmp/garterm.sock")
    }
}

fn send_command(socket: &PathBuf, cmd: Command) -> Result<Response> {
    let mut stream = UnixStream::connect(socket)
        .with_context(|| format!("Failed to connect to {}", socket.display()))?;

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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = socket_path(cli.socket);

    let cmd = match cli.command {
        Commands::NewTab { cwd } => Command::NewTab { cwd },
        Commands::CloseTab => Command::CloseTab,
        Commands::NextTab => Command::NextTab,
        Commands::PrevTab => Command::PrevTab,
        Commands::Tab { index } => Command::SwitchTab { index },
        Commands::Split { horizontal } => Command::Split {
            direction: if horizontal { "horizontal".into() } else { "vertical".into() },
        },
        Commands::ClosePane => Command::ClosePane,
        Commands::Focus { direction } => Command::FocusPaneDirection { direction },
        Commands::Send { text } => Command::SendText { text },
        Commands::Info => Command::GetInfo,
        Commands::Reload => Command::Reload,
        Commands::Quit => Command::Quit,
    };

    let response = send_command(&socket, cmd)?;

    if response.success {
        if let Some(data) = response.data {
            println!("{}", serde_json::to_string_pretty(&data)?);
        } else if let Some(msg) = response.message {
            println!("{}", msg);
        }
    } else {
        eprintln!("Error: {}", response.message.unwrap_or_else(|| "Unknown error".into()));
        std::process::exit(1);
    }

    Ok(())
}
