use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};

use super::types::MonitorInfo;

/// Enumerate all monitors currently connected to the system.
///
/// Uses the Win32 `EnumDisplayMonitors` API to walk every active display and
/// collects geometry + device-name information into a [`Vec<MonitorInfo>`].
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    let monitors: Arc<Mutex<Vec<MonitorInfo>>> = Arc::new(Mutex::new(Vec::new()));
    let monitors_clone = monitors.clone();

    unsafe extern "system" fn enum_proc(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> windows::core::BOOL {
        unsafe {
            let monitors_ptr = lparam.0 as *mut Mutex<Vec<MonitorInfo>>;
            let monitors = &*monitors_ptr;

            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;

            if GetMonitorInfoW(hmonitor, &mut info as *mut _ as *mut _).as_bool() {
                let rc = info.monitorInfo.rcMonitor;
                let device_name_slice = &info.szDevice;
                let name_len = device_name_slice
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(device_name_slice.len());
                let device_name = String::from_utf16_lossy(&device_name_slice[..name_len]);

                monitors.lock().unwrap().push(MonitorInfo {
                    name: device_name,
                    x: rc.left,
                    y: rc.top,
                    width: rc.right - rc.left,
                    height: rc.bottom - rc.top,
                    hmonitor: hmonitor.0 as isize,
                });
            }

            windows::core::BOOL(1) // continue enumeration
        }
    }

    unsafe {
        let ptr = Arc::into_raw(monitors_clone);
        let _ = EnumDisplayMonitors(None, None, Some(enum_proc), LPARAM(ptr as isize));
        // Reconstruct Arc so it drops properly
        let _ = Arc::from_raw(ptr);
    }

    monitors.lock().unwrap().clone()
}
