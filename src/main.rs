mod daemon;
mod ipc;
mod monitor;
mod overlay;
mod tray;
mod ui;

use std::io::{BufReader, BufWriter};
use std::net::TcpStream;
use std::sync::mpsc;
use std::time::Duration;

use gpui::{AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use raw_window_handle::RawWindowHandle;
use windows::Win32::Foundation::HWND;

use crate::ipc::{DaemonMsg, DaemonState, UiMsg, connect_to_daemon};
use crate::ui::Controller;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--ui") {
        // ── UI mode: connect to running daemon and show the window ───────
        run_ui();
    } else {
        // ── Launcher / daemon mode ───────────────────────────────────────
        // Try to connect to an already-running daemon.  If one exists, ask
        // it to open a UI window and exit immediately.  Otherwise become
        // the daemon ourselves.
        match TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", ipc::DAEMON_PORT).parse().unwrap(),
            Duration::from_millis(300),
        ) {
            Ok(stream) => {
                // Daemon is running — ask it to open a UI, then exit.
                let mut writer = BufWriter::new(stream);
                let _ = ipc::write_msg(&mut writer, &UiMsg::ShowUi);
                // The daemon will spawn oled-care.exe --ui on its own.
            }
            Err(_) => {
                // No daemon yet — become it.
                daemon::run_daemon();
            }
        }
    }
}

/// UI mode: connect to the daemon, fetch initial state, run GPUI.
///
/// This process holds all GPUI / GPU resources.  When the user closes the
/// window the process exits, freeing everything immediately.  The daemon
/// keeps running with its overlays intact.
fn run_ui() {
    // Connect to the daemon (retry for up to 3 s to handle the race where
    // we were just spawned by a daemon that hasn't bound its port yet).
    let stream = match connect_to_daemon(3000) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[ui] Cannot connect to daemon: {:?}", e);
            std::process::exit(1);
        }
    };

    // Fetch the initial application state before starting GPUI.
    let initial_state: DaemonState = {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = BufWriter::new(stream.try_clone().unwrap());
        if let Err(e) = ipc::write_msg(&mut writer, &UiMsg::GetState) {
            eprintln!("[ui] Failed to send GetState: {:?}", e);
            std::process::exit(1);
        }
        match ipc::read_msg::<_, DaemonMsg>(&mut reader) {
            Ok(DaemonMsg::State(s)) => s,
            Err(e) => {
                eprintln!("[ui] Failed to read initial state: {:?}", e);
                std::process::exit(1);
            }
        }
    };

    // ── IPC background thread ─────────────────────────────────────────────
    // cmd_tx  : Controller → this thread → daemon (commands)
    // state_rx: daemon → this thread → Controller (state updates)
    let (cmd_tx, cmd_rx) = mpsc::sync_channel::<UiMsg>(32);
    let (state_tx, state_rx) = mpsc::channel::<DaemonState>();

    {
        let reader = BufReader::new(stream.try_clone().unwrap());
        let writer = BufWriter::new(stream);
        std::thread::spawn(move || ipc_thread(reader, writer, cmd_rx, state_tx));
    }

    // ── GPUI ─────────────────────────────────────────────────────────────
    Application::new().run(move |app: &mut gpui::App| {
        let _window = app
            .open_window(
                WindowOptions {
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("OLED Care".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(530.0), px(720.0)),
                        app,
                    ))),
                    is_resizable: false,
                    ..Default::default()
                },
                |window, cx| {
                    // In UI mode, closing the window exits this process.
                    // The daemon + overlays keep running independently.
                    window.on_window_should_close(cx, |_window, cx| {
                        cx.quit();
                        true
                    });

                    let cmd_tx = cmd_tx.clone();
                    cx.new(move |cx| Controller::new(initial_state, cmd_tx, state_rx, cx))
                },
            )
            .unwrap();
    });
}

/// Background thread: relays commands from the Controller to the daemon and
/// forwards daemon state-update replies back to the Controller.
fn ipc_thread(
    mut reader: BufReader<TcpStream>,
    mut writer: BufWriter<TcpStream>,
    cmd_rx: mpsc::Receiver<UiMsg>,
    state_tx: mpsc::Sender<DaemonState>,
) {
    while let Ok(cmd) = cmd_rx.recv() {
        if ipc::write_msg(&mut writer, &cmd).is_err() {
            break;
        }
        match ipc::read_msg::<_, DaemonMsg>(&mut reader) {
            Ok(DaemonMsg::State(s)) => {
                let _ = state_tx.send(s);
            }
            Err(_) => break,
        }
    }
}

// ── Win32 helpers (kept for potential future use) ─────────────────────────────

pub fn gpui_hwnd(window: &gpui::Window) -> Option<HWND> {
    let handle =
        <gpui::Window as raw_window_handle::HasWindowHandle>::window_handle(window).ok()?;
    if let RawWindowHandle::Win32(win32) = handle.as_raw() {
        Some(HWND(win32.hwnd.get() as *mut std::ffi::c_void))
    } else {
        None
    }
}
