//! Daemon process logic for OLED Care.
//!
//! The daemon:
//! - Owns the [`OverlayManager`] (manages overlay windows)
//! - Runs the system tray icon
//! - Listens on TCP 127.0.0.1:17432 for UI connections
//! - Processes commands from the UI (SetOpacity, ToggleMonitor, SetActive, …)
//! - Spawns `oled-care.exe --ui` when the user clicks "Open" in the tray
//! - Never uses GPUI

use std::io::{BufReader, BufWriter};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use crate::ipc::{self, DAEMON_PORT, DaemonMsg, DaemonState, UiMsg};
use crate::monitor::{MonitorInfo, enumerate_monitors};
use crate::overlay::{OverlayManager, register_overlay_class};
use crate::tray::{TrayEvent, spawn_tray};

// ── Internal state ────────────────────────────────────────────────────────────

/// Internal daemon state (not serialized — used inside the daemon process only).
struct CoreState {
    monitors: Vec<MonitorInfo>,
    selected: Vec<bool>,
    opacity: u8,
    overlays_active: bool,
}

impl CoreState {
    /// Build the serializable snapshot to send to the UI.
    fn to_daemon_state(&self, mgr: &OverlayManager) -> DaemonState {
        DaemonState {
            monitors: self.monitors.clone(),
            selected: self.selected.clone(),
            opacity: self.opacity,
            overlays_active: self.overlays_active,
            overlay_alive: mgr.states.iter().map(|s| s.hwnd.is_some()).collect(),
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Entry point for daemon mode.
pub fn run_daemon() {
    // Register the Win32 overlay window class once, before any windows are created.
    unsafe {
        if let Err(e) = register_overlay_class() {
            eprintln!("[daemon] Failed to register overlay class: {:?}", e);
            return;
        }
    }

    let monitors = enumerate_monitors();
    println!("[daemon] {} monitor(s) found", monitors.len());
    let n = monitors.len();

    let state = Arc::new(Mutex::new(CoreState {
        monitors,
        selected: vec![false; n],
        opacity: 50,
        overlays_active: false,
    }));

    let overlay_mgr = Arc::new(Mutex::new(OverlayManager::new(n)));

    // Track whether a UI window is currently open (to avoid spawning duplicates).
    let ui_open = Arc::new(AtomicBool::new(false));

    // Shared flag: true when overlays are currently active.
    // Passed to the tray so the menu label stays in sync.
    let active_flag = Arc::new(AtomicBool::new(false));

    // Start the tray icon thread.
    let (tray_tx, tray_rx) = mpsc::channel::<TrayEvent>();
    spawn_tray(tray_tx, active_flag.clone());

    // Start the TCP IPC server on a background thread.
    {
        let state = state.clone();
        let mgr = overlay_mgr.clone();
        let ui_open = ui_open.clone();
        let active_flag = active_flag.clone();
        thread::spawn(move || run_tcp_server(state, mgr, ui_open, active_flag));
    }

    // Spawn the initial UI window so the user sees it on first launch.
    spawn_ui_process();

    // Main daemon loop: poll tray events every 100 ms.
    loop {
        thread::sleep(Duration::from_millis(100));

        while let Ok(ev) = tray_rx.try_recv() {
            match ev {
                TrayEvent::Open => {
                    // Only spawn a new UI if one isn't already open.
                    if !ui_open.load(Ordering::Relaxed) {
                        spawn_ui_process();
                    }
                }
                TrayEvent::Quit => {
                    overlay_mgr.lock().unwrap().deactivate();
                    std::process::exit(0);
                }
                TrayEvent::Toggle => {
                    let mut s = state.lock().unwrap();
                    if s.overlays_active {
                        // ── Disable ──────────────────────────────────────
                        s.overlays_active = false;
                        drop(s);
                        overlay_mgr.lock().unwrap().deactivate();
                        active_flag.store(false, Ordering::Relaxed);
                    } else {
                        // ── Enable (only if at least one monitor selected) ─
                        if s.selected.iter().any(|&sel| sel) {
                            let monitors = s.monitors.clone();
                            let selected = s.selected.clone();
                            let opacity = s.opacity;
                            drop(s);
                            let (dummy_tx, _dummy_rx) = mpsc::channel::<(usize, usize)>();
                            overlay_mgr
                                .lock()
                                .unwrap()
                                .activate(&monitors, &selected, opacity, &dummy_tx);
                            state.lock().unwrap().overlays_active = true;
                            active_flag.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Spawn a new UI process (`oled-care.exe --ui`).
fn spawn_ui_process() {
    if let Ok(exe) = std::env::current_exe() {
        match std::process::Command::new(&exe).arg("--ui").spawn() {
            Ok(_) => {}
            Err(e) => eprintln!("[daemon] Failed to spawn UI: {:?}", e),
        }
    }
}

// ── TCP server ────────────────────────────────────────────────────────────────

/// TCP server loop: accept one connection at a time from the UI.
fn run_tcp_server(
    state: Arc<Mutex<CoreState>>,
    mgr: Arc<Mutex<OverlayManager>>,
    ui_open: Arc<AtomicBool>,
    active_flag: Arc<AtomicBool>,
) {
    let addr = format!("127.0.0.1:{}", DAEMON_PORT);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[daemon] Cannot bind IPC port {}: {:?}", DAEMON_PORT, e);
            std::process::exit(1);
        }
    };
    println!("[daemon] Listening on {}", addr);

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let state = state.clone();
                let mgr = mgr.clone();
                let ui_open = ui_open.clone();
                let active_flag = active_flag.clone();
                thread::spawn(move || {
                    ui_open.store(true, Ordering::Relaxed);
                    handle_client(s, state, mgr, active_flag);
                    ui_open.store(false, Ordering::Relaxed);
                });
            }
            Err(e) => eprintln!("[daemon] Accept error: {:?}", e),
        }
    }
}

// ── Client handler ────────────────────────────────────────────────────────────

/// Handle one UI client connection in a loop until it disconnects.
fn handle_client(
    stream: TcpStream,
    state: Arc<Mutex<CoreState>>,
    mgr: Arc<Mutex<OverlayManager>>,
    active_flag: Arc<AtomicBool>,
) {
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = BufWriter::new(stream);

    loop {
        let msg: UiMsg = match ipc::read_msg(&mut reader) {
            Ok(m) => m,
            Err(_) => break, // UI disconnected
        };

        match msg {
            UiMsg::GetState => {
                // Fall through to the reply-with-state at the bottom of the loop.
            }

            UiMsg::ShowUi => {
                // A second instance sent this — reply with current state and
                // close the connection.  The main daemon loop will open a fresh
                // UI window if none is currently alive.
                let reply = {
                    let s = state.lock().unwrap();
                    let m = mgr.lock().unwrap();
                    DaemonMsg::State(s.to_daemon_state(&m))
                };
                let _ = ipc::write_msg(&mut writer, &reply);
                break;
            }

            UiMsg::SetOpacity(opacity) => {
                let mut s = state.lock().unwrap();
                s.opacity = opacity;
                if s.overlays_active {
                    mgr.lock().unwrap().update_opacity(opacity);
                }
            }

            UiMsg::ToggleMonitor(idx) => {
                let mut s = state.lock().unwrap();
                // Only allow toggling when overlays are not active.
                if !s.overlays_active {
                    if let Some(sel) = s.selected.get_mut(idx) {
                        *sel = !*sel;
                    }
                }
            }

            UiMsg::SetActive(active) => {
                if active {
                    // ── Enable ──────────────────────────────────────────
                    let s = state.lock().unwrap();
                    if !s.overlays_active && s.selected.iter().any(|&sel| sel) {
                        let monitors = s.monitors.clone();
                        let selected = s.selected.clone();
                        let opacity = s.opacity;
                        drop(s); // release before locking mgr

                        // Dummy hwnd channel — activate() blocks until all
                        // HWNDs are registered into mgr.states directly.
                        let (dummy_tx, _dummy_rx) = mpsc::channel::<(usize, usize)>();
                        mgr.lock()
                            .unwrap()
                            .activate(&monitors, &selected, opacity, &dummy_tx);

                        state.lock().unwrap().overlays_active = true;
                        active_flag.store(true, Ordering::Relaxed);
                    }
                    // if already active: lock guard drops here, nothing to do
                } else {
                    // ── Disable ─────────────────────────────────────────
                    // Take a fresh lock (no outer guard in scope, no deadlock).
                    let mut s = state.lock().unwrap();
                    if s.overlays_active {
                        s.overlays_active = false;
                        drop(s); // release before locking mgr
                        mgr.lock().unwrap().deactivate();
                        active_flag.store(false, Ordering::Relaxed);
                    }
                }
            }

            UiMsg::Quit => {
                mgr.lock().unwrap().deactivate();
                std::process::exit(0);
            }
        }

        // After every command, reply with the full current state.
        let reply = {
            let s = state.lock().unwrap();
            let m = mgr.lock().unwrap();
            DaemonMsg::State(s.to_daemon_state(&m))
        };
        if ipc::write_msg(&mut writer, &reply).is_err() {
            break; // UI disconnected while we were writing
        }
    }
}
