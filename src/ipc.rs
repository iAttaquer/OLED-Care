//! Inter-process communication between the daemon and the UI process.
//!
//! Protocol: every message is prefixed with a 4-byte little-endian `u32`
//! giving the length of the JSON body that follows.  Both sides use
//! [`write_msg`] / [`read_msg`] for all communication.

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::monitor::MonitorInfo;

/// TCP port the daemon listens on.  Must not conflict with other local
/// services; chosen to be well outside the ephemeral-port range.
pub const DAEMON_PORT: u16 = 17432;

// ── Shared state snapshot ────────────────────────────────────────────────────

/// Full application state as reported by the daemon to the UI.
///
/// Sent on every command response so the UI always has an up-to-date view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    /// All monitors discovered at daemon start-up.
    pub monitors: Vec<MonitorInfo>,
    /// Per-monitor selection flags (same order as `monitors`).
    pub selected: Vec<bool>,
    /// Current overlay opacity (0 = transparent, 255 = opaque).
    pub opacity: u8,
    /// Whether overlay protection is currently active.
    pub overlays_active: bool,
    /// Whether each overlay window has been fully created (HWND registered).
    pub overlay_alive: Vec<bool>,
}

// ── Messages: UI → Daemon ────────────────────────────────────────────────────

/// Commands sent from the UI process to the daemon.
#[derive(Debug, Serialize, Deserialize)]
pub enum UiMsg {
    /// Request the full current state (first message after connecting).
    GetState,
    /// Set the overlay opacity.
    SetOpacity(u8),
    /// Toggle a monitor's selection by index.
    ToggleMonitor(usize),
    /// Enable (`true`) or disable (`false`) overlay protection.
    SetActive(bool),
    /// Sent by a second instance of the executable to ask the daemon to open
    /// a new UI window.  The sending process exits after this.
    ShowUi,
    /// Shut down the entire application (daemon + all overlays).
    Quit,
}

// ── Messages: Daemon → UI ────────────────────────────────────────────────────

/// Replies from the daemon to the UI.
///
/// The daemon always replies with the full current state after every command,
/// so the UI never needs a separate "poll" mechanism.
#[derive(Debug, Serialize, Deserialize)]
pub enum DaemonMsg {
    State(DaemonState),
}

// ── Wire helpers ─────────────────────────────────────────────────────────────

/// Write a message to `w` using the 4-byte-length-prefix + JSON format.
pub fn write_msg<W: Write, T: Serialize>(w: &mut BufWriter<W>, msg: &T) -> io::Result<()> {
    let body =
        serde_json::to_vec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = (body.len() as u32).to_le_bytes();
    w.write_all(&len)?;
    w.write_all(&body)?;
    w.flush()
}

/// Read a message from `r` using the 4-byte-length-prefix + JSON format.
pub fn read_msg<R: Read, T: DeserializeOwned>(r: &mut BufReader<R>) -> io::Result<T> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    // Guard against malformed/huge messages.
    if len > 4 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC message too large",
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Try to connect to the running daemon, retrying until `timeout_ms` elapses.
///
/// Returns the connected [`TcpStream`] on success, or the last I/O error on
/// timeout.
pub fn connect_to_daemon(timeout_ms: u64) -> io::Result<TcpStream> {
    let addr = format!("127.0.0.1:{}", DAEMON_PORT);
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match TcpStream::connect(&addr) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => {
                if std::time::Instant::now() >= deadline {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}
