#![allow(dead_code, unused_imports)]

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod app;
mod config;
mod input;
mod pty;
mod render;
mod terminal;

pub use config::Config;

#[derive(Parser)]
#[command(name = "garterm")]
#[command(about = "GPU-accelerated terminal emulator for gardesk")]
#[command(version)]
struct Cli {
    /// Command to execute instead of shell
    #[arg(short = 'e', long)]
    command: Option<String>,

    /// Working directory
    #[arg(long)]
    working_directory: Option<String>,

    /// Window title
    #[arg(long)]
    title: Option<String>,

    /// Use VSync-based rendering (may not work on Asahi Linux)
    #[arg(long)]
    vsync: bool,
}

fn main() -> Result<()> {
    // Set up file logging
    let log_file = std::fs::File::create("/tmp/garterm.log")
        .expect("Failed to create log file");
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false)
        .init();

    let cli = Cli::parse();

    info!("garterm starting");

    // Build configuration from CLI args
    let shell = cli
        .command
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/sh".to_string());

    let cwd = cli
        .working_directory
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok());

    let config = Config::new()
        .with_shell(shell)
        .with_working_directory(cwd)
        .with_vsync(cli.vsync);

    if config.vsync {
        info!("VSync mode enabled (dirty-flag rendering)");
    } else {
        info!("Continuous rendering mode (60fps timer-based)");
    }

    // Create and run the application
    let mut app = pollster::block_on(app::App::new(config))?;
    app.run()
}
