//! Connection lifecycle events for the operator: connecting, connected, why a
//! session ended and how long until the next attempt. They go to stderr and,
//! with `--syslog`, to the system log as well, where `logread` shows them.
//! Neither can stall or kill the client: write errors on stderr are ignored,
//! and the syslog socket never blocks.

use std::io::{IsTerminal, Write};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Where the system log daemon receives messages.
pub const SYSLOG_SOCKET: &str = "/dev/log";
const LOG_DAEMON: u8 = 3 << 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
}

impl Severity {
    const fn priority(self) -> u8 {
        LOG_DAEMON
            + match self {
                Self::Warning => 4,
                Self::Info => 6,
            }
    }
}

/// Sends to the syslog socket without ever blocking. The socket is not
/// connected, so messages keep arriving after syslogd restarts.
pub struct Syslog {
    socket: UnixDatagram,
    path: PathBuf,
}

impl Syslog {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let socket = UnixDatagram::unbound()?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            path: path.to_path_buf(),
        })
    }

    /// Drops the message if syslogd is down or its queue is full.
    fn send(&self, severity: Severity, message: &str) {
        let line = syslog_line(severity, std::process::id(), message);
        let _ = self.socket.send_to(line.as_bytes(), &self.path);
    }
}

fn syslog_line(severity: Severity, pid: u32, message: &str) -> String {
    format!("<{}>usque-rs[{pid}]: {message}", severity.priority())
}

static SYSLOG: OnceLock<Syslog> = OnceLock::new();

/// Copy every later event to the system log.
pub fn enable_syslog(syslog: Syslog) {
    let _ = SYSLOG.set(syslog);
}

/// Whether the live status line has anywhere useful to go.
pub fn stderr_is_terminal() -> bool {
    std::io::stderr().is_terminal()
}

/// Report a lifecycle event.
pub fn event(severity: Severity, message: &str) {
    let mut stderr = std::io::stderr().lock();
    // On a terminal, first erase the status line the event replaces.
    let erase = if stderr.is_terminal() {
        "\r\x1b[2K"
    } else {
        ""
    };
    let _ = writeln!(stderr, "{erase}{message}");
    drop(stderr);
    syslog(severity, message);
}

/// Send to the system log only, if enabled.
pub fn syslog(severity: Severity, message: &str) {
    if let Some(syslog) = SYSLOG.get() {
        syslog.send(severity, message);
    }
}

/// Redraw the live status line (terminals only).
pub fn status(line: &str) {
    let _ = write!(std::io::stderr().lock(), "\r\x1b[2K{line}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn syslog_lines_carry_the_daemon_priority_tag_and_pid() {
        assert_eq!(
            syslog_line(Severity::Info, 42, "[connected] MASQUE tunnel established"),
            "<30>usque-rs[42]: [connected] MASQUE tunnel established"
        );
        assert_eq!(
            syslog_line(
                Severity::Warning,
                7,
                "[error] no packets from the server for 90s"
            ),
            "<28>usque-rs[7]: [error] no packets from the server for 90s"
        );
    }

    #[test]
    fn events_reach_the_syslog_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        let receiver = UnixDatagram::bind(&path).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Syslog::new(&path)
            .unwrap()
            .send(Severity::Warning, "[error] lost");
        let mut buf = [0u8; 256];
        let len = receiver.recv(&mut buf).unwrap();
        let expected = format!("<28>usque-rs[{}]: [error] lost", std::process::id());
        assert_eq!(std::str::from_utf8(&buf[..len]).unwrap(), expected);
    }

    #[test]
    fn a_missing_or_full_syslog_never_blocks() {
        let dir = tempfile::tempdir().unwrap();
        Syslog::new(&dir.path().join("missing"))
            .unwrap()
            .send(Severity::Info, "dropped");
        // A receiver that never reads fills up; further sends must be dropped.
        let path = dir.path().join("log");
        let _receiver = UnixDatagram::bind(&path).unwrap();
        let syslog = Syslog::new(&path).unwrap();
        let started = Instant::now();
        for _ in 0..10_000 {
            syslog.send(Severity::Info, "[retry] Waiting 30s before reconnecting");
        }
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
