mod monitor;
mod overlay;
mod tray;
mod ui;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use gpui::{AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use raw_window_handle::RawWindowHandle;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, SetForegroundWindow, ShowWindow};

use crate::monitor::enumerate_monitors;
use crate::overlay::register_overlay_class;
use crate::tray::{TrayEvent, spawn_tray};
use crate::ui::Controller;

fn main() {
    println!("=== OLED Care - Display Protection System ===");

    // Register the Win32 overlay window class (once, before any windows are created).
    unsafe {
        if let Err(e) = register_overlay_class() {
            eprintln!("Failed to register overlay window class: {:?}", e);
            return;
        }
    }

    // Discover connected monitors.
    let monitors = enumerate_monitors();
    println!("Found {} monitor(s):", monitors.len());
    for (i, mon) in monitors.iter().enumerate() {
        println!(
            "  [{}] {} — {}x{} at ({}, {})",
            i, mon.name, mon.width, mon.height, mon.x, mon.y,
        );
    }

    if monitors.is_empty() {
        eprintln!("No monitors detected — nothing to protect.");
        return;
    }

    // Channel for tray → Controller communication.
    let (tray_tx, tray_rx) = mpsc::channel::<TrayEvent>();

    // Start the system tray icon on its own thread.
    spawn_tray(tray_tx);

    let shared_hwnd: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));

    Application::new().run(move |app: &mut gpui::App| {
        let monitors_clone = monitors.clone();
        let shared_hwnd_for_window = shared_hwnd.clone();
        let shared_hwnd_for_ctrl = shared_hwnd.clone();

        let _window_handle = app
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
                    if let Some(hwnd) = gpui_hwnd(window) {
                        shared_hwnd_for_window.store(hwnd.0 as usize, Ordering::Relaxed);
                    }

                    window.on_window_should_close(cx, |window: &mut gpui::Window, _cx| {
                        if let Some(hwnd) = gpui_hwnd(window) {
                            unsafe {
                                let _ = ShowWindow(hwnd, SW_HIDE);
                            }
                        }
                        false
                    });

                    cx.new(move |cx| {
                        Controller::new(monitors_clone, tray_rx, shared_hwnd_for_ctrl, cx)
                    })
                },
            )
            .unwrap();
    });
}

// ── Win32 helpers ─────────────────────────────────────────────────────────────

/// Extract the Win32 HWND from a GPUI Window using raw-window-handle.
pub fn gpui_hwnd(window: &gpui::Window) -> Option<HWND> {
    let handle =
        <gpui::Window as raw_window_handle::HasWindowHandle>::window_handle(window).ok()?;
    if let RawWindowHandle::Win32(win32) = handle.as_raw() {
        Some(HWND(win32.hwnd.get() as *mut std::ffi::c_void))
    } else {
        None
    }
}

/// Show the GPUI window and bring it to the foreground.
pub fn show_and_focus_window(window: &mut gpui::Window) {
    if let Some(hwnd) = gpui_hwnd(window) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }
    window.activate_window();
}
