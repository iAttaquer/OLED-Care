use std::ffi::c_void;
use std::sync::{Arc, mpsc};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

use crate::monitor::MonitorInfo;
use crate::overlay::config::{OverlayConfig, OverlayState};
use crate::overlay::window::{WM_UPDATE_OPACITY, spawn_overlay};

/// High-level manager that controls the lifecycle of per-monitor overlay windows.
///
/// It bridges the gap between the UI layer (which knows *which* monitors are
/// selected and at *what* opacity) and the low-level Win32 overlay windows that
/// live on dedicated background threads.
pub struct OverlayManager {
    /// One [`OverlayState`] entry per monitor (mirrors the monitor list order).
    pub states: Vec<OverlayState>,
}

impl OverlayManager {
    /// Create a manager sized to match the given monitor list.
    pub fn new(monitor_count: usize) -> Self {
        Self {
            states: vec![OverlayState::default(); monitor_count],
        }
    }

    /// Spawn overlay windows on every monitor that is marked as *selected* but
    /// does not already have an active overlay.
    ///
    /// `hwnd_tx` is used to notify the main channel about each new `HWND` for
    /// deferred bookkeeping inside the render loop.
    pub fn activate(
        &mut self,
        monitors: &[MonitorInfo],
        selected: &[bool],
        opacity: u8,
        hwnd_tx: &mpsc::Sender<(usize, usize)>,
    ) {
        for i in 0..monitors.len() {
            if selected[i] && self.states[i].hwnd.is_none() {
                let mon = &monitors[i];
                let cfg = OverlayConfig {
                    opacity,
                    x: mon.x,
                    y: mon.y,
                    width: mon.width,
                    height: mon.height,
                };

                let idx = i;
                let tx = hwnd_tx.clone();
                let (inner_tx, inner_rx) = mpsc::channel::<usize>();
                let handle = spawn_overlay(cfg, inner_tx);

                // Wait briefly for the HWND so we can reference it immediately.
                if let Ok(ptr) = inner_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                    self.states[i].hwnd = Some(HWND(ptr as *mut c_void));
                    let _ = tx.send((idx, ptr));
                }

                self.states[i].handle = Some(Arc::new(handle));
            }
        }
    }

    /// Close every active overlay window and clear all tracked state.
    pub fn deactivate(&mut self) {
        for state in &mut self.states {
            if let Some(hwnd) = state.hwnd {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            state.hwnd = None;
            state.handle = None;
        }
    }

    /// Send an opacity update to every currently-active overlay window.
    ///
    /// This is non-blocking — it posts a custom `WM_UPDATE_OPACITY` message to
    /// each overlay's message loop which applies the change asynchronously.
    pub fn update_opacity(&self, opacity: u8) {
        for state in &self.states {
            if let Some(hwnd) = state.hwnd {
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_UPDATE_OPACITY,
                        WPARAM(opacity as usize),
                        LPARAM(0),
                    );
                }
            }
        }
    }

    /// Returns the number of overlays that are currently alive.
    pub fn active_count(&self) -> usize {
        self.states.iter().filter(|s| s.hwnd.is_some()).count()
    }

    /// Record a newly-received `HWND` for the given monitor index.
    pub fn register_hwnd(&mut self, index: usize, ptr: usize) {
        if index < self.states.len() {
            self.states[index].hwnd = Some(HWND(ptr as *mut c_void));
        }
    }
}
