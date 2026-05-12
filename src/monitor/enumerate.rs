use std::collections::HashMap;
use std::mem;

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};

use super::types::MonitorInfo;

/// Enumerate all monitors currently connected to the system.
///
/// Uses `EnumDisplayMonitors` for geometry and `QueryDisplayConfig` +
/// `DisplayConfigGetDeviceInfo` for the human-readable model name.
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    // Build GDI-name → friendly-name lookup first.
    let friendly_map = build_friendly_name_map();

    // Bundle the output Vec and the map so the callback can reach both.
    struct CallbackData {
        monitors: Vec<MonitorInfo>,
        friendly_map: HashMap<String, String>,
    }

    let mut cb_data = CallbackData {
        monitors: Vec::new(),
        friendly_map,
    };

    unsafe extern "system" fn enum_proc(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> windows::core::BOOL {
        unsafe {
            let data = &mut *(lparam.0 as *mut CallbackData);

            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;

            if GetMonitorInfoW(hmonitor, &mut info as *mut _ as *mut _).as_bool() {
                let rc = info.monitorInfo.rcMonitor;

                let raw = &info.szDevice;
                let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                let name = String::from_utf16_lossy(&raw[..end]);

                let friendly_name = data.friendly_map.get(&name).cloned().unwrap_or_default();

                data.monitors.push(MonitorInfo {
                    name,
                    friendly_name,
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
        let ptr = &mut cb_data as *mut CallbackData;
        let _ = EnumDisplayMonitors(None, None, Some(enum_proc), LPARAM(ptr as isize));
    }

    cb_data.monitors
}

// ── Friendly-name lookup ──────────────────────────────────────────────────────

/// Query the Display Configuration API to build a map from GDI device name
/// (e.g. `\\.\DISPLAY1`) to the monitor's friendly model name
/// (e.g. `"LG ULTRAGEAR 27GP850-B"`).
///
/// Returns an empty map on any failure; the caller falls back to the GDI name.
fn build_friendly_name_map() -> HashMap<String, String> {
    let mut map = HashMap::new();

    unsafe {
        // Step 1 — ask Windows how many paths and mode-info entries exist.
        let mut num_paths: u32 = 0;
        let mut num_modes: u32 = 0;

        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut num_paths, &mut num_modes).0 != 0
        {
            return map;
        }

        // Step 2 — fill the arrays with the active display configuration.
        let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![mem::zeroed(); num_paths as usize];
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![mem::zeroed(); num_modes as usize];

        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut num_paths,
            paths.as_mut_ptr(),
            &mut num_modes,
            modes.as_mut_ptr(),
            None,
        )
        .0 != 0
        {
            return map;
        }

        // Step 3 — for every active path, pair the GDI source name with the
        //           friendly target (monitor model) name.
        for path in paths.iter().take(num_paths as usize) {
            // ── Source: get the GDI device name (\\.\DISPLAYn) ───────────
            let mut src: DISPLAYCONFIG_SOURCE_DEVICE_NAME = mem::zeroed();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = path.sourceInfo.adapterId;
            src.header.id = path.sourceInfo.id;

            if DisplayConfigGetDeviceInfo(&mut src.header as *mut _) != 0 {
                continue;
            }

            let end = src
                .viewGdiDeviceName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(src.viewGdiDeviceName.len());
            let gdi_name = String::from_utf16_lossy(&src.viewGdiDeviceName[..end]);

            // ── Target: get the friendly monitor model name ───────────────
            let mut tgt: DISPLAYCONFIG_TARGET_DEVICE_NAME = mem::zeroed();
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt.header.size = mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = path.targetInfo.adapterId;
            tgt.header.id = path.targetInfo.id;

            if DisplayConfigGetDeviceInfo(&mut tgt.header as *mut _) != 0 {
                continue;
            }

            let end = tgt
                .monitorFriendlyDeviceName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(tgt.monitorFriendlyDeviceName.len());
            let friendly = String::from_utf16_lossy(&tgt.monitorFriendlyDeviceName[..end]);

            // Keep the first mapping for each GDI name; skip blank names
            // (happens for some embedded / virtual displays).
            if !friendly.is_empty() {
                map.entry(gdi_name).or_insert(friendly);
            }
        }
    }

    map
}
