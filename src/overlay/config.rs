use std::sync::Arc;

use windows::Win32::Foundation::HWND;

/// Parameters needed to spawn a single overlay window on a specific monitor.
#[derive(Clone, Debug)]
pub struct OverlayConfig {
    /// Opacity of the overlay (0 = fully transparent, 255 = fully opaque).
    pub opacity: u8,
    /// X coordinate of the target monitor's top-left corner.
    pub x: i32,
    /// Y coordinate of the target monitor's top-left corner.
    pub y: i32,
    /// Width of the target monitor in pixels.
    pub width: i32,
    /// Height of the target monitor in pixels.
    pub height: i32,
}

/// Tracks the runtime state of an overlay that has been spawned on a monitor.
#[derive(Clone)]
pub struct OverlayState {
    /// Handle to the Win32 overlay window, if it is currently alive.
    pub hwnd: Option<HWND>,
    /// Join handle for the background thread running the overlay message loop.
    pub handle: Option<Arc<std::thread::JoinHandle<()>>>,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            hwnd: None,
            handle: None,
        }
    }
}
