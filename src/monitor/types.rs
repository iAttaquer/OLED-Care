/// Information about a connected display monitor.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct MonitorInfo {
    /// Device name reported by Windows (e.g. `\\.\DISPLAY1`).
    pub name: String,
    /// X coordinate of the monitor's top-left corner in virtual-screen space.
    pub x: i32,
    /// Y coordinate of the monitor's top-left corner in virtual-screen space.
    pub y: i32,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Raw `HMONITOR` handle stored as an opaque integer.
    pub hmonitor: isize,
}
