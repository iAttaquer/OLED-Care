//! System tray icon and context menu for OLED Care.
//!
//! This module runs a dedicated background thread that:
//! 1. Creates a hidden message-only Win32 window to receive tray notifications.
//! 2. Registers a system tray icon via `Shell_NotifyIconW`.
//! 3. On right-click shows a popup menu with "Open" and "Close" items.
//! 4. Communicates back to the main thread via [`TrayEvent`] through an `mpsc` channel.
//! 5. Receives the GPUI window HWND and posts `WM_TRAY_WAKE` to nudge the message pump
//!    whenever an event is pushed, so the GPUI render loop picks it up promptly.

use std::sync::mpsc;
use std::thread;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMessageW, GetSystemMetrics, IDI_APPLICATION, LoadIconW, MF_STRING, MSG,
    PostQuitMessage, RegisterClassW, SM_CYSMICON, SetForegroundWindow, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RETURNCMD, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WM_APP,
    WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::PCWSTR;

// ── Message constants ────────────────────────────────────────────────────────

/// The window message posted by the shell when the user interacts with the tray icon.
const WM_TRAY_CALLBACK: u32 = WM_APP + 1;

/// Menu item IDs.
const IDM_OPEN: usize = 1001;
const IDM_QUIT: usize = 1002;

// ── Public API ───────────────────────────────────────────────────────────────

/// Events sent from the tray thread to the main thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayEvent {
    /// User clicked "Open" or double-clicked the icon — show the main window.
    Open,
    /// User clicked "Close" — the application should exit.
    Quit,
}

/// Spawn the tray background thread.
///
/// * `event_tx` — sender used to push [`TrayEvent`]s toward the main thread.
/// * `gpui_hwnd_rx` — receiver that will eventually yield the raw GPUI window
///   HWND (as `usize`) so the tray thread can post `WM_TRAY_WAKE` to nudge
///   the Win32 message pump whenever a new event is enqueued.
pub fn spawn_tray(event_tx: mpsc::Sender<TrayEvent>) {
    thread::spawn(move || run_tray_thread(event_tx));
}

// ── Thread-local state ───────────────────────────────────────────────────────

thread_local! {
    /// Sender for tray events — stored here so the static `wnd_proc` can reach it.
    static TX: std::cell::RefCell<Option<mpsc::Sender<TrayEvent>>> =
        std::cell::RefCell::new(None);


}

// ── Internal implementation ──────────────────────────────────────────────────

fn run_tray_thread(event_tx: mpsc::Sender<TrayEvent>) {
    unsafe {
        // ── Register a message-only window class ─────────────────────────
        let hinstance: HINSTANCE = match GetModuleHandleW(None) {
            Ok(h) => h.into(),
            Err(e) => {
                eprintln!("[tray] GetModuleHandleW failed: {:?}", e);
                return;
            }
        };

        let class_name_buf: Vec<u16> = "OLEDCareTrayClass\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name_buf.as_ptr()),
            ..Default::default()
        };
        // Ignore re-registration errors (harmless on restart).
        let _ = RegisterClassW(&wc);

        // ── Create a message-only (hidden) window ────────────────────────
        let hwnd_message = HWND(windows::Win32::UI::WindowsAndMessaging::HWND_MESSAGE.0 as *mut _);
        let window_name_buf: Vec<u16> = "OLEDCareTray\0".encode_utf16().collect();

        let tray_hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name_buf.as_ptr()),
            PCWSTR(window_name_buf.as_ptr()),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            Some(hwnd_message),
            None,
            Some(hinstance),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[tray] CreateWindowExW failed: {:?}", e);
                return;
            }
        };

        // ── Store sender in thread-local so wnd_proc can reach it ────────
        TX.with(|cell| *cell.borrow_mut() = Some(event_tx));

        // ── Register the tray icon ────────────────────────────────────────
        let icon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();

        let mut tip = [0u16; 128];
        let tip_str: Vec<u16> = "OLED Care\0".encode_utf16().collect();
        let copy_len = tip_str.len().min(tip.len());
        tip[..copy_len].copy_from_slice(&tip_str[..copy_len]);

        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: tray_hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY_CALLBACK,
            hIcon: icon,
            szTip: tip,
            ..Default::default()
        };

        if !Shell_NotifyIconW(NIM_ADD, &mut nid).as_bool() {
            eprintln!("[tray] Shell_NotifyIconW(NIM_ADD) failed");
        }

        // ── Message loop ─────────────────────────────────────────────────
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // ── Cleanup ───────────────────────────────────────────────────────
        let _ = Shell_NotifyIconW(NIM_DELETE, &mut nid);
    }
}

// ── Window procedure ──────────────────────────────────────────────────────────

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if msg == WM_TRAY_CALLBACK {
            // Low word of lparam carries the mouse/keyboard notification code.
            let event = (lparam.0 & 0xFFFF) as u32;

            match event {
                // Right-click → show context menu.
                e if e == WM_RBUTTONUP => {
                    show_context_menu(hwnd);
                }
                // Double left-click → open the main window.
                e if e == WM_LBUTTONDBLCLK => {
                    send_event(TrayEvent::Open);
                }
                _ => {}
            }

            return LRESULT(0);
        }

        if msg == WM_DESTROY {
            PostQuitMessage(0);
            return LRESULT(0);
        }

        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Send a [`TrayEvent`] through the thread-local sender.
fn send_event(event: TrayEvent) {
    TX.with(|cell| {
        if let Some(tx) = cell.borrow().as_ref() {
            let _ = tx.send(event);
        }
    });
}

// ── Context menu ─────────────────────────────────────────────────────────────

unsafe fn show_context_menu(hwnd: HWND) {
    unsafe {
        let hmenu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[tray] CreatePopupMenu failed: {:?}", e);
                return;
            }
        };

        let open_str: Vec<u16> = "Open\0".encode_utf16().collect();
        let quit_str: Vec<u16> = "Close\0".encode_utf16().collect();

        let _ = AppendMenuW(hmenu, MF_STRING, IDM_OPEN, PCWSTR(open_str.as_ptr()));
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_QUIT, PCWSTR(quit_str.as_ptr()));

        // Required so the menu dismisses when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);

        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut pt);

        // Place menu just above the tray icon area.
        let icon_h = GetSystemMetrics(SM_CYSMICON);

        let cmd = TrackPopupMenu(
            hmenu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD,
            pt.x,
            pt.y - icon_h,
            Some(0),
            hwnd,
            None,
        );

        let _ = DestroyMenu(hmenu);

        match cmd.0 as usize {
            IDM_OPEN => send_event(TrayEvent::Open),
            IDM_QUIT => send_event(TrayEvent::Quit),
            _ => {}
        }
    }
}
