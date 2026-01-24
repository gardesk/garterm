use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod pty;

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

    run(&shell, cwd.as_deref())
}

fn run(shell: &str, cwd: Option<&std::path::Path>) -> Result<()> {
    use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
    use std::io::{Read, Write};
    use std::os::fd::{AsFd, AsRawFd, BorrowedFd};

    let size = pty::PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    };

    let mut pty = pty::Pty::spawn(shell, size, cwd)?;
    info!("spawned shell: {}", shell);

    // Set stdin to raw mode for proper terminal behavior
    let _raw_guard = RawModeGuard::new()?;

    let stdin_fd = std::io::stdin().as_fd().try_clone_to_owned()?;
    let mut stdin = std::fs::File::from(stdin_fd);

    let mut buf = [0u8; 4096];
    let mut stdin_buf = [0u8; 256];

    // Store raw fds for polling
    let pty_raw_fd = pty.master_fd().as_raw_fd();
    let stdin_raw_fd = stdin.as_raw_fd();

    loop {
        // Create PollFds fresh each iteration using raw fd conversion
        // SAFETY: fds are valid for the duration of poll
        let pty_borrow = unsafe { BorrowedFd::borrow_raw(pty_raw_fd) };
        let stdin_borrow = unsafe { BorrowedFd::borrow_raw(stdin_raw_fd) };

        let mut fds = [
            PollFd::new(pty_borrow, PollFlags::POLLIN),
            PollFd::new(stdin_borrow, PollFlags::POLLIN),
        ];

        poll(&mut fds, PollTimeout::NONE)?;

        let pty_ready = fds[0].revents().is_some_and(|r| r.contains(PollFlags::POLLIN));
        let stdin_ready = fds[1].revents().is_some_and(|r| r.contains(PollFlags::POLLIN));
        let pty_hup = fds[0].revents().is_some_and(|r| r.contains(PollFlags::POLLHUP));

        // Let the fds slice go out of scope before mutable borrows
        let _ = fds;

        // Read from PTY
        if pty_ready {
            match pty.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    std::io::stdout().write_all(&buf[..n])?;
                    std::io::stdout().flush()?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
        }

        // Read from stdin
        if stdin_ready {
            match stdin.read(&mut stdin_buf) {
                Ok(0) => break,
                Ok(n) => {
                    pty.write_all(&stdin_buf[..n])?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
        }

        // Check if child exited
        if !pty.is_alive() {
            break;
        }

        // Check for hangup
        if pty_hup {
            break;
        }
    }

    info!("garterm exiting");
    Ok(())
}

/// RAII guard for raw terminal mode
struct RawModeGuard {
    original: nix::sys::termios::Termios,
}

impl RawModeGuard {
    fn new() -> Result<Self> {
        use nix::sys::termios::{self, LocalFlags, InputFlags, SetArg};
        use std::os::fd::AsFd;

        let stdin = std::io::stdin();
        let original = termios::tcgetattr(stdin.as_fd())?;

        let mut raw = original.clone();
        raw.local_flags.remove(LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::ISIG);
        raw.input_flags.remove(InputFlags::IXON | InputFlags::ICRNL);
        termios::tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &raw)?;

        Ok(Self { original })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        use nix::sys::termios::{self, SetArg};
        use std::os::fd::AsFd;

        let stdin = std::io::stdin();
        let _ = termios::tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &self.original);
    }
}
