use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::libc;
use nix::pty::{openpty, OpenptyResult};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{dup2, execvp, fork, setsid, ForkResult, Pid};
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("failed to open pty: {0}")]
    OpenPty(#[from] Errno),
    #[error("fork failed: {0}")]
    Fork(Errno),
    #[error("exec failed: {0}")]
    Exec(Errno),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// PTY window size
#[derive(Debug, Clone, Copy)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

impl From<PtySize> for libc::winsize {
    fn from(size: PtySize) -> Self {
        libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: size.pixel_width,
            ws_ypixel: size.pixel_height,
        }
    }
}

/// Pseudo-terminal handle
pub struct Pty {
    master: OwnedFd,
    child_pid: Pid,
}

impl Pty {
    /// Spawn a new PTY with the given shell command
    pub fn spawn(shell: &str, size: PtySize, cwd: Option<&Path>) -> Result<Self, PtyError> {
        // Open PTY pair
        let OpenptyResult { master, slave } = openpty(None, None)?;

        // Set initial window size
        let winsize: libc::winsize = size.into();
        unsafe {
            if libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &winsize) < 0 {
                return Err(PtyError::OpenPty(Errno::last()));
            }
        }

        // Fork
        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                // Child process
                drop(master);

                // Create new session and set controlling terminal
                setsid().ok();

                unsafe {
                    libc::ioctl(slave.as_raw_fd(), libc::TIOCSCTTY, 0);
                }

                // Dup slave to stdin/stdout/stderr
                dup2(slave.as_raw_fd(), libc::STDIN_FILENO).ok();
                dup2(slave.as_raw_fd(), libc::STDOUT_FILENO).ok();
                dup2(slave.as_raw_fd(), libc::STDERR_FILENO).ok();

                if slave.as_raw_fd() > libc::STDERR_FILENO {
                    drop(slave);
                }

                // Change directory if specified
                if let Some(dir) = cwd {
                    std::env::set_current_dir(dir).ok();
                }

                // Set up environment
                // SAFETY: We are in the child process after fork, single-threaded
                unsafe {
                    std::env::set_var("TERM", "xterm-256color");
                    std::env::set_var("COLORTERM", "truecolor");
                }

                // Exec shell
                let shell_cstr = CString::new(shell).unwrap();
                let shell_name = Path::new(shell)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| format!("-{}", n)) // Login shell prefix
                    .unwrap_or_else(|| shell.to_string());
                let argv0 = CString::new(shell_name).unwrap();

                // execvp only returns on error (Ok is Infallible)
                let e = execvp(&shell_cstr, &[argv0]).unwrap_err();
                let msg = format!("garterm: failed to exec '{}': {}\r\n", shell, e);
                let _ = unsafe {
                    libc::write(
                        libc::STDERR_FILENO,
                        msg.as_ptr() as *const _,
                        msg.len(),
                    )
                };

                // If exec fails, use _exit to avoid running atexit handlers
                // (which can crash in GPU drivers like NVIDIA after fork)
                unsafe { libc::_exit(127) };
            }
            Ok(ForkResult::Parent { child }) => {
                // Parent process
                drop(slave);

                // Set master to non-blocking
                let flags = fcntl(master.as_raw_fd(), FcntlArg::F_GETFL)?;
                let flags = OFlag::from_bits_truncate(flags);
                fcntl(
                    master.as_raw_fd(),
                    FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK),
                )?;

                Ok(Pty {
                    master,
                    child_pid: child,
                })
            }
            Err(e) => Err(PtyError::Fork(e)),
        }
    }

    /// Get the master file descriptor
    pub fn master_fd(&self) -> &OwnedFd {
        &self.master
    }

    /// Get the child process ID
    pub fn child_pid(&self) -> Pid {
        self.child_pid
    }

    /// Read from PTY
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let fd = self.master.as_raw_fd();
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }

    /// Write to PTY
    pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let fd = self.master.as_raw_fd();
        let n = unsafe { libc::write(fd, buf.as_ptr() as *const _, buf.len()) };
        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }

    /// Write all bytes to PTY
    pub fn write_all(&mut self, mut buf: &[u8]) -> io::Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "write zero")),
                Ok(n) => buf = &buf[n..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // Busy wait for non-blocking fd
                    std::thread::yield_now();
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Resize the PTY
    pub fn resize(&mut self, size: PtySize) -> io::Result<()> {
        let winsize: libc::winsize = size.into();
        let ret = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &winsize) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Check if child process is still running
    pub fn is_alive(&self) -> bool {
        match waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => true,
            Ok(_) => false,
            Err(_) => false,
        }
    }

    /// Wait for child process (non-blocking)
    pub fn try_wait(&self) -> Option<i32> {
        match waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(_, code)) => Some(code),
            Ok(WaitStatus::Signaled(_, sig, _)) => Some(128 + sig as i32),
            _ => None,
        }
    }

    /// Wait for child process (blocking)
    pub fn wait(&self) -> i32 {
        match waitpid(self.child_pid, None) {
            Ok(WaitStatus::Exited(_, code)) => code,
            Ok(WaitStatus::Signaled(_, sig, _)) => 128 + sig as i32,
            _ => 1,
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // The master fd is closed automatically by OwnedFd

        // Send SIGHUP to the entire process group (shell + all children)
        // The shell became a session leader via setsid(), so its PID is the PGID
        // Using negative PID targets the whole process group
        unsafe {
            libc::kill(-(self.child_pid.as_raw()), libc::SIGHUP);
        }

        // Give processes time to shut down gracefully (save state, etc.)
        // Complex apps like VSCode need more than a few ms
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            match waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => continue,
                _ => return, // Already exited
            }
        }

        // Still alive after 2 seconds, force kill
        unsafe {
            libc::kill(-(self.child_pid.as_raw()), libc::SIGKILL);
        }
        let _ = waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG));
    }
}
