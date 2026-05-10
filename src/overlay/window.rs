use std::sync::mpsc;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, FillRect, HBRUSH, PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA,
    GetCursorPos, GetMessageW, GetWindowLongPtrW, HWND_TOPMOST, KillTimer, LWA_ALPHA, MSG,
    PostQuitMessage, RegisterClassW, SW_SHOW, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, WINDOW_EX_STYLE, WM_DESTROY, WM_PAINT, WM_TIMER, WM_USER, WNDCLASSW,
    WS_DISABLED, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::PCWSTR;

use super::config::OverlayConfig;

// ─── Constants ───────────────────────────────────────────────────────────────

pub const WM_UPDATE_OPACITY: u32 = WM_USER + 1;

const TIMER_ID: usize = 1;

const FADE_STEP: u8 = 12;

static mut WINDOW_CLASS_ATOM: u16 = 0;

// ─── Per-window fade state ───────────────────────────────────────────────────

/// Heap-allocated state attached to each overlay window via `GWLP_USERDATA`.
///
/// Freed inside `WM_DESTROY`.
struct FadeState {
    base_opacity: u8,
    /// Opacity currently applied to the Win32 layered window.
    current_opacity: u8,
    /// Bounding rectangle of the monitor this overlay covers (used to hit-test
    /// the cursor position without any Win32 region API).
    mon_x: i32,
    mon_y: i32,
    mon_w: i32,
    mon_h: i32,
}

impl FadeState {
    /// Returns `true` when `pt` falls inside this overlay's monitor.
    #[inline]
    fn cursor_on_monitor(&self, pt: POINT) -> bool {
        pt.x >= self.mon_x
            && pt.x < self.mon_x + self.mon_w
            && pt.y >= self.mon_y
            && pt.y < self.mon_y + self.mon_h
    }
}

// ─── Window procedure ────────────────────────────────────────────────────────

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            // ── Repaint: fill the window with opaque black ─────────────
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if !hdc.is_invalid() {
                    let brush = CreateSolidBrush(COLORREF(0x00000000));
                    if !brush.is_invalid() {
                        let _ = FillRect(hdc, &ps.rcPaint, brush);
                    }
                    let _ = EndPaint(hwnd, &ps);
                }
                LRESULT(0)
            }

            // ── User request: change the base (target-when-away) opacity ──
            WM_UPDATE_OPACITY => {
                let new_base = wparam.0 as u8;
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut FadeState;

                if !ptr.is_null() {
                    (*ptr).base_opacity = new_base;

                    // Only apply immediately when the cursor is NOT on the
                    // monitor; otherwise the fade timer will pick it up once
                    // the cursor leaves.
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    if !(*ptr).cursor_on_monitor(pt) {
                        (*ptr).current_opacity = new_base;
                        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), new_base, LWA_ALPHA);
                    }
                } else {
                    // Fallback: no fade state yet, apply directly.
                    let _ =
                        SetLayeredWindowAttributes(hwnd, COLORREF(0), wparam.0 as u8, LWA_ALPHA);
                }
                LRESULT(0)
            }

            // ── Cursor-tracking / fade animation tick ─────────────────
            WM_TIMER if wparam.0 == TIMER_ID => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut FadeState;
                if ptr.is_null() {
                    return LRESULT(0);
                }
                let state = &mut *ptr;

                // Where is the cursor right now?
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);

                // Cursor on our monitor → fade to 0; cursor away → fade back.
                let target: u8 = if state.cursor_on_monitor(pt) {
                    0
                } else {
                    state.base_opacity
                };

                // Step current_opacity one FADE_STEP closer to target.
                if state.current_opacity != target {
                    let new_opacity = if state.current_opacity > target {
                        state.current_opacity.saturating_sub(FADE_STEP).max(target)
                    } else {
                        state.current_opacity.saturating_add(FADE_STEP).min(target)
                    };
                    state.current_opacity = new_opacity;
                    let _ = SetLayeredWindowAttributes(
                        hwnd,
                        COLORREF(0),
                        state.current_opacity,
                        LWA_ALPHA,
                    );
                }

                LRESULT(0)
            }

            // ── Window destroyed: clean up timer and fade state ────────
            WM_DESTROY => {
                // Stop the timer before freeing state so no stray ticks fire.
                let _ = KillTimer(Some(hwnd), TIMER_ID);

                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut FadeState;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }

                PostQuitMessage(0);
                LRESULT(0)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

// ─── Class registration ──────────────────────────────────────────────────────

/// Register the `OLEDCareOverlayClass` window class.
///
/// Idempotent — the class is only registered on the first call.
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

// ─── Window creation ─────────────────────────────────────────────────────────

/// Create a Win32 overlay window and run its message loop **on the current thread**.
///
/// Properties:
/// * Layered (`WS_EX_LAYERED`) — alpha transparency via `SetLayeredWindowAttributes`.
/// * Click-through (`WS_EX_TRANSPARENT`, `WS_DISABLED`).
/// * Always on top (`WS_EX_TOPMOST`).
/// * Hidden from the taskbar (`WS_EX_TOOLWINDOW`).
/// * Never steals focus (`WS_EX_NOACTIVATE`).
///
/// A 16 ms `WM_TIMER` drives a fade animation: the overlay fades to opacity 0
/// while the cursor is on its monitor and fades back to `config.opacity` when
/// the cursor moves away.
///
/// Once the window is ready its `HWND` (as `usize`) is sent through `hwnd_tx`.
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

        // ── Determine initial opacity based on current cursor position ──
        // If the cursor is already on this monitor when protection is
        // activated, start transparent so there is no jarring flash.
        let mut cursor_pt = POINT::default();
        let _ = GetCursorPos(&mut cursor_pt);
        let cursor_on = cursor_pt.x >= config.x
            && cursor_pt.x < config.x + config.width
            && cursor_pt.y >= config.y
            && cursor_pt.y < config.y + config.height;
        let initial_opacity: u8 = if cursor_on { 0 } else { config.opacity };

        // ── Attach per-window fade state ────────────────────────────────
        let fade_state = Box::new(FadeState {
            base_opacity: config.opacity,
            current_opacity: initial_opacity,
            mon_x: config.x,
            mon_y: config.y,
            mon_w: config.width,
            mon_h: config.height,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(fade_state) as isize);

        // ── Apply initial opacity and show the window ───────────────────
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), initial_opacity, LWA_ALPHA);
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

        // ── Start the cursor-tracking / fade timer (~60 fps) ───────────
        SetTimer(Some(hwnd), TIMER_ID, 16, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        Ok(())
    }
}

// ─── Thread helper ───────────────────────────────────────────────────────────

pub fn spawn_overlay(
    config: OverlayConfig,
    hwnd_tx: mpsc::Sender<usize>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || match create_win32_overlay(config, hwnd_tx) {
        Ok(()) => {}
        Err(e) => eprintln!("Overlay thread error: {:?}", e),
    })
}
