use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gartermctl")]
#[command(about = "Control garterm terminal emulator")]
#[command(version)]
struct Cli {
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
    /// Split the focused pane
    Split {
        /// Split direction
        #[arg(long, short = 'H')]
        horizontal: bool,
    },
    /// Close the focused pane
    ClosePane,
    /// Get terminal info
    Info,
    /// Reload configuration
    Reload,
}

fn main() -> Result<()> {
    let _cli = Cli::parse();
    // TODO: Connect to socket and send commands
    eprintln!("gartermctl: not yet implemented");
    Ok(())
}
