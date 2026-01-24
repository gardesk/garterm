#![allow(dead_code, unused_imports)]

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod app;
mod input;
mod pty;
mod render;
mod terminal;

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
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    info!("garterm starting");

    let shell = cli
        .command
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/sh".to_string());

    let cwd = cli
        .working_directory
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok());

    // Create and run the application
    let mut app = pollster::block_on(app::App::new(&shell, cwd.as_deref()))?;
    app.run()
}
