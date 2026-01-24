use nix::sys::signal::{sigprocmask, SigSet, SigmaskHow, Signal};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use std::io;
use std::os::fd::AsRawFd;

/// Handles SIGCHLD and SIGWINCH via signalfd for poll integration
pub struct SignalHandler {
    fd: SignalFd,
    mask: SigSet,
}

impl SignalHandler {
    /// Create a new signal handler, blocking SIGCHLD and SIGWINCH
    pub fn new() -> io::Result<Self> {
        let mut mask = SigSet::empty();
        mask.add(Signal::SIGCHLD);
        mask.add(Signal::SIGWINCH);

        // Block these signals in the main thread
        sigprocmask(SigmaskHow::SIG_BLOCK, Some(&mask), None)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Create signalfd
        let fd = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Ok(Self { fd, mask })
    }

    /// Get the raw fd for polling
    pub fn as_raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    /// Read pending signals, returns list of received signals
    pub fn read_signals(&mut self) -> io::Result<Vec<ReceivedSignal>> {
        let mut signals = Vec::new();

        loop {
            match self.fd.read_signal() {
                Ok(Some(info)) => {
                    let sig = match info.ssi_signo as i32 {
                        nix::libc::SIGCHLD => ReceivedSignal::ChildExited {
                            pid: info.ssi_pid as i32,
                            status: info.ssi_status,
                        },
                        nix::libc::SIGWINCH => ReceivedSignal::WindowResized,
                        _ => continue,
                    };
                    signals.push(sig);
                }
                Ok(None) => break,
                Err(nix::errno::Errno::EAGAIN) => break,
                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
            }
        }

        Ok(signals)
    }
}

impl Drop for SignalHandler {
    fn drop(&mut self) {
        // Restore signal mask
        let _ = sigprocmask(SigmaskHow::SIG_UNBLOCK, Some(&self.mask), None);
    }
}

/// A signal that was received
#[derive(Debug, Clone)]
pub enum ReceivedSignal {
    /// SIGCHLD - a child process exited
    ChildExited { pid: i32, status: i32 },
    /// SIGWINCH - terminal window was resized
    WindowResized,
}
