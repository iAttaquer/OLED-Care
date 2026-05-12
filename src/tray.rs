//! System tray icon and context menu for OLED Care.
//!
//! This module runs a dedicated background thread that:
//! 1. Creates a hidden message-only Win32 window to receive tray notifications.
//! 2. Registers a system tray icon via `Shell_NotifyIconW`.
//! 3. On right-click shows a dark-themed popup menu with "Enable/Disable
//!    Protection", "Open", and "Close" items.
//! 4. Communicates back to the main thread via [`TrayEvent`] through an `mpsc`
//!    channel.
//!
//! ### Dark mode
//! Dark menus are achieved by two complementary mechanisms:
//! * `SetPreferredAppMode(ForceDark)` (uxtheme ordinal 135) — tells Windows
//!   that this process prefers dark UI.
//! * `SetWindowTheme(popup_hwnd, "DarkMode_Explorer", None)` applied inside
//!   the `WM_INITMENUPOPUP` handler, which fires before the menu is painted.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::SetWindowTheme;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    FindWindowExW, GetCursorPos, GetMessageW, GetSystemMetrics, IDI_APPLICATION, LoadIconW,
    MF_SEPARATOR, MF_STRING, MSG, PostQuitMessage, RegisterClassW, SM_CYSMICON,
    SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TrackPopupMenu,
    TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_DESTROY, WM_INITMENUPOPUP, WM_LBUTTONDBLCLK,
    WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::PCSTR;
use windows::core::PCWSTR;

// ── Message constants ────────────────────────────────────────────────────────

/// Shell callback message posted to our tray window on user interaction.
const WM_TRAY_CALLBACK: u32 = WM_APP + 1;

/// Menu item IDs.
const IDM_TOGGLE: usize = 1000;
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
    /// User clicked the enable/disable toggle item.
    Toggle,
}

/// Spawn the tray background thread.
///
/// * `event_tx`   — sender used to push [`TrayEvent`]s toward the main thread.
/// * `active`     — shared flag that reflects whether overlay protection is
///                  currently enabled. The tray reads this to label the menu
///                  item correctly ("Enable" vs "Disable").
pub fn spawn_tray(event_tx: mpsc::Sender<TrayEvent>, active: Arc<AtomicBool>) {
    thread::spawn(move || run_tray_thread(event_tx, active));
}

// ── Thread-local state ───────────────────────────────────────────────────────

thread_local! {
    static TX: std::cell::RefCell<Option<mpsc::Sender<TrayEvent>>> =
        std::cell::RefCell::new(None);

    /// Shared flag reflecting whether overlays are currently active.
    static ACTIVE: std::cell::RefCell<Option<Arc<AtomicBool>>> =
        std::cell::RefCell::new(None);
}

// ── Internal implementation ──────────────────────────────────────────────────

fn run_tray_thread(event_tx: mpsc::Sender<TrayEvent>, active: Arc<AtomicBool>) {
    unsafe {
        // ── Apply dark mode to this process's menus ───────────────────────
        try_enable_dark_mode();

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

        // ── Store state in thread-locals so wnd_proc can reach it ────────
        TX.with(|cell| *cell.borrow_mut() = Some(event_tx));
        ACTIVE.with(|cell| *cell.borrow_mut() = Some(active));

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
            let event = (lparam.0 & 0xFFFF) as u32;
            match event {
                e if e == WM_RBUTTONUP => show_context_menu(hwnd),
                e if e == WM_LBUTTONDBLCLK => send_event(TrayEvent::Open),
                _ => {}
            }
            return LRESULT(0);
        }

        // Fires just before the popup is painted — apply dark theme here.
        if msg == WM_INITMENUPOPUP {
            apply_dark_theme_to_popup();
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

fn send_event(event: TrayEvent) {
    TX.with(|cell| {
        if let Some(tx) = cell.borrow().as_ref() {
            let _ = tx.send(event);
        }
    });
}

fn is_active() -> bool {
    ACTIVE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|a| a.load(Ordering::Relaxed))
            .unwrap_or(false)
    })
}

// ── Dark mode helpers ─────────────────────────────────────────────────────────

/// Call undocumented uxtheme ordinals to make Windows render dark menus for
/// this process.  Silently does nothing on older Windows versions where these
/// ordinals don't exist.
unsafe fn try_enable_dark_mode() {
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(windows::core::w!("uxtheme.dll")) else {
            return;
        };

        // Ordinal 135 — SetPreferredAppMode(PreferredAppMode) -> PreferredAppMode
        //   0 = Default, 1 = AllowDark, 2 = ForceDark, 3 = ForceLight
        type FnSetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
        if let Some(f) = GetProcAddress(uxtheme, PCSTR(135usize as *const u8)) {
            let set_mode: FnSetPreferredAppMode = std::mem::transmute(f);
            set_mode(2); // ForceDark
        }

        // Ordinal 136 — FlushMenuThemes() — refreshes cached theme data.
        type FnFlushMenuThemes = unsafe extern "system" fn();
        if let Some(f) = GetProcAddress(uxtheme, PCSTR(136usize as *const u8)) {
            let flush: FnFlushMenuThemes = std::mem::transmute(f);
            flush();
        }
    }
}

/// Find the currently-initialising popup menu window (class `#32768`) and
/// apply the `DarkMode_Explorer` theme so Windows paints it with dark colors.
unsafe fn apply_dark_theme_to_popup() {
    unsafe {
        let class_name: Vec<u16> = "#32768\0".encode_utf16().collect();
        if let Ok(menu_hwnd) =
            FindWindowExW(None, None, PCWSTR(class_name.as_ptr()), PCWSTR::null())
        {
            let _ = SetWindowTheme(menu_hwnd, windows::core::w!("DarkMode_Explorer"), None);
        }
    }
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

        // ── Toggle item ───────────────────────────────────────────────────
        let active = is_active();
        let toggle_label: Vec<u16> = if active {
            "Disable Protection\0"
        } else {
            "Enable Protection\0"
        }
        .encode_utf16()
        .collect();
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_TOGGLE, PCWSTR(toggle_label.as_ptr()));

        // ── Separator ─────────────────────────────────────────────────────
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());

        // ── Open ──────────────────────────────────────────────────────────
        let open_str: Vec<u16> = "Open\0".encode_utf16().collect();
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_OPEN, PCWSTR(open_str.as_ptr()));

        // ── Close ─────────────────────────────────────────────────────────
        let quit_str: Vec<u16> = "Close\0".encode_utf16().collect();
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_QUIT, PCWSTR(quit_str.as_ptr()));

        // Required so the menu dismisses on click-away.
        let _ = SetForegroundWindow(hwnd);

        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut pt);
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
            IDM_TOGGLE => send_event(TrayEvent::Toggle),
            IDM_OPEN => send_event(TrayEvent::Open),
            IDM_QUIT => send_event(TrayEvent::Quit),
            _ => {}
        }
    }
}
