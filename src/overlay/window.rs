use std::sync::mpsc;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, FillRect, HBRUSH, PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
    HWND_TOPMOST, LWA_ALPHA, MSG, PostQuitMessage, RegisterClassW, SW_SHOW, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow, TranslateMessage,
    WINDOW_EX_STYLE, WM_USER, WNDCLASSW, WS_DISABLED, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::PCWSTR;

use super::config::OverlayConfig;

/// Custom window message used to update the overlay opacity at runtime.
pub const WM_UPDATE_OPACITY: u32 = WM_USER + 1;

/// Global window class atom — registered once, reused by every overlay window.
static mut WINDOW_CLASS_ATOM: u16 = 0;

// ─── Window procedure ───────────────────────────────────────────────────────

/// Window procedure callback for overlay windows.
///
/// Handles three messages:
/// * `WM_PAINT`           — fills the window with solid black.
/// * `WM_UPDATE_OPACITY`  — applies a new alpha value received via `WPARAM`.
/// * `WM_DESTROY`         — posts a quit message to end the thread's message loop.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            // WM_PAINT
            0x000F => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if !hdc.is_invalid() {
                    let brush = CreateSolidBrush(COLORREF(0x00000000)); // solid black
                    if !brush.is_invalid() {
                        let _ = FillRect(hdc, &ps.rcPaint, brush);
                    }
                    let _ = EndPaint(hwnd, &ps);
                }
                LRESULT(0)
            }
            // Custom: live opacity update
            WM_UPDATE_OPACITY => {
                let new_opacity = wparam.0 as u8;
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), new_opacity, LWA_ALPHA);
                LRESULT(0)
            }
            // WM_DESTROY
            0x0002 => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

// ─── Class registration ─────────────────────────────────────────────────────

/// Register the `OLEDCareOverlayClass` window class.
///
/// This is idempotent — the class is only registered on the first call.
pub unsafe fn register_overlay_class() -> Result<(), Box<dyn std::error::Error>> {
    if unsafe { WINDOW_CLASS_ATOM } != 0 {
        return Ok(());
    }

    let hinstance = windows::Win32::Foundation::HINSTANCE(std::ptr::null_mut());
    let class_name: Vec<u16> = "OLEDCareOverlayClass\0".encode_utf16().collect();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        style: CS_HREDRAW | CS_VREDRAW,
        hbrBackground: HBRUSH(std::ptr::null_mut()),
        ..Default::default()
    };

    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        return Err("Failed to register overlay window class".into());
    }

    unsafe {
        WINDOW_CLASS_ATOM = atom;
    }
    Ok(())
}

// ─── Window creation ────────────────────────────────────────────────────────

/// Create a Win32 overlay window and run its message loop **on the current thread**.
///
/// The window is:
/// * Layered (`WS_EX_LAYERED`) with alpha-based transparency.
/// * Click-through (`WS_EX_TRANSPARENT`, `WS_DISABLED`).
/// * Always on top (`WS_EX_TOPMOST`).
/// * Hidden from the taskbar (`WS_EX_TOOLWINDOW`).
/// * Never steals focus (`WS_EX_NOACTIVATE`).
///
/// Once the window is created its `HWND` (as a `usize`) is sent through
/// `hwnd_tx` so that the UI thread can reference it later.
fn create_win32_overlay(
    config: OverlayConfig,
    hwnd_tx: mpsc::Sender<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let hinstance = windows::Win32::Foundation::HINSTANCE(std::ptr::null_mut());
        let class_name: Vec<u16> = "OLEDCareOverlayClass\0".encode_utf16().collect();
        let window_name: Vec<u16> = "OLED Care Overlay\0".encode_utf16().collect();

        let ex_style = WINDOW_EX_STYLE(
            WS_EX_LAYERED.0
                | WS_EX_TRANSPARENT.0
                | WS_EX_TOPMOST.0
                | WS_EX_TOOLWINDOW.0
                | WS_EX_NOACTIVATE.0,
        );

        let hwnd = CreateWindowExW(
            ex_style,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(window_name.as_ptr()),
            WS_POPUP | WS_DISABLED,
            config.x,
            config.y,
            config.width,
            config.height,
            None,
            None,
            Some(hinstance),
            None,
        )?;

        if hwnd.0.is_null() {
            return Err("Failed to create overlay window".into());
        }

        // Notify the caller about the new window handle.
        hwnd_tx.send(hwnd.0 as usize).unwrap();

        // Apply initial opacity.
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), config.opacity, LWA_ALPHA);

        // Show and position the window.
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            config.x,
            config.y,
            config.width,
            config.height,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
        let _ = UpdateWindow(hwnd);

        // Run the message loop until WM_DESTROY / WM_CLOSE.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        Ok(())
    }
}

// ─── Thread helper ──────────────────────────────────────────────────────────

/// Spawn a new overlay on a dedicated background thread.
///
/// Returns the [`JoinHandle`] for the thread so the caller can track its
/// lifetime. The thread exits when the overlay window is closed.
pub fn spawn_overlay(
    config: OverlayConfig,
    hwnd_tx: mpsc::Sender<usize>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || match create_win32_overlay(config, hwnd_tx) {
        Ok(()) => {}
        Err(e) => eprintln!("Overlay thread error: {:?}", e),
    })
}
