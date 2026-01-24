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
    /// Path to config file (default: ~/.config/garterm/config.toml)
    #[arg(short, long)]
    config: Option<String>,

    /// Command to execute instead of shell
    #[arg(short = 'e', long)]
    command: Option<String>,

    /// Working directory
    #[arg(long)]
    working_directory: Option<String>,

    /// Window title
    #[arg(long)]
    title: Option<String>,

    /// Font size in points
    #[arg(long)]
    font_size: Option<f32>,

    /// Use VSync-based rendering (may not work on Asahi Linux)
    #[arg(long)]
    vsync: bool,

    /// Print loaded configuration and exit
    #[arg(long)]
    print_config: bool,
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

    // Load configuration from file or defaults
    let mut config = if let Some(config_path) = &cli.config {
        match Config::load_from_file(std::path::Path::new(config_path)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error loading config: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        Config::load()
    };

    // Apply CLI overrides
    if let Some(cmd) = cli.command {
        config = config.with_shell(cmd);
    }
    if let Some(cwd) = cli.working_directory {
        config = config.with_working_directory(Some(cwd.into()));
    }
    if let Some(size) = cli.font_size {
        config = config.with_font_size(size);
    }
    if cli.vsync {
        config = config.with_vsync(true);
    }

    // Print config and exit if requested
    if cli.print_config {
        println!("{}", toml::to_string_pretty(&config)?);
        return Ok(());
    }

    if config.general.vsync {
        info!("VSync mode enabled (dirty-flag rendering)");
    } else {
        info!("Continuous rendering mode (60fps timer-based)");
    }

    // Create and run the application
    let mut app = pollster::block_on(app::App::new(config))?;
    app.run()
}
