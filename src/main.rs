use gpui::prelude::*;
use gpui::{Application, Bounds, FontWeight, MouseButton, Pixels, WindowOptions, div, px, rgb};
use std::cell::Cell;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, EnumDisplayMonitors, FillRect, GetMonitorInfoW, HBRUSH,
    HDC, HMONITOR, MONITORINFOEXW, PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
    HWND_TOPMOST, LWA_ALPHA, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOW,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
    TranslateMessage, WINDOW_EX_STYLE, WM_CLOSE, WM_USER, WNDCLASSW, WS_DISABLED, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::PCWSTR;

// ─── Monitor info ───────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct MonitorInfo {
    name: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    _hmonitor: isize,
}

fn enumerate_monitors() -> Vec<MonitorInfo> {
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
                    _hmonitor: hmonitor.0 as isize,
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

    let result = monitors.lock().unwrap().clone();
    result
}

// ─── Switch component ───────────────────────────────────────────────────────

fn switch(
    checked: bool,
    on_click: impl Fn(&bool, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .w(px(44.0))
        .h(px(24.0))
        .rounded(px(12.0))
        .bg(if checked {
            rgb(0x4CAF50)
        } else {
            rgb(0x333333)
        })
        .border_1()
        .border_color(if checked {
            rgb(0x4CAF50)
        } else {
            rgb(0x555555)
        })
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            on_click(&checked, window, cx);
        })
        .child(
            div()
                .w(px(16.0))
                .h(px(16.0))
                .rounded_full()
                .bg(rgb(0xffffff))
                .ml(if checked { px(20.0) } else { px(4.0) }),
        )
}

// ─── Checkbox component ─────────────────────────────────────────────────────

fn checkbox(
    checked: bool,
    on_click: impl Fn(&bool, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(22.0))
        .h(px(22.0))
        .rounded(px(4.0))
        .bg(if checked {
            rgb(0x4CAF50)
        } else {
            rgb(0x2a2a2a)
        })
        .border_1()
        .border_color(if checked {
            rgb(0x4CAF50)
        } else {
            rgb(0x666666)
        })
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            on_click(&checked, window, cx);
        })
        .child(
            div()
                .text_color(rgb(0xffffff))
                .child(if checked { "✓" } else { "" }),
        )
}

// ─── Overlay Win32 ──────────────────────────────────────────────────────────

#[derive(Clone)]
struct OverlayConfig {
    opacity: u8,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

static mut WINDOW_CLASS_ATOM: u16 = 0;

const WM_UPDATE_OPACITY: u32 = WM_USER + 1;

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            0x000F => {
                // WM_PAINT
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
            WM_UPDATE_OPACITY => {
                let new_opacity = wparam.0 as u8;
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), new_opacity, LWA_ALPHA);
                LRESULT(0)
            }
            0x0002 => {
                // WM_DESTROY
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn register_overlay_class() -> Result<(), Box<dyn std::error::Error>> {
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
        return Err("Failed to register window class".into());
    }

    unsafe {
        WINDOW_CLASS_ATOM = atom;
    }
    Ok(())
}

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
            return Err("Failed to create window".into());
        }

        hwnd_tx.send(hwnd.0 as usize).unwrap();

        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), config.opacity, LWA_ALPHA);
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

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        Ok(())
    }
}

fn spawn_overlay(cfg: OverlayConfig, hwnd_tx: mpsc::Sender<usize>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || match create_win32_overlay(cfg, hwnd_tx) {
        Ok(_) => {}
        Err(e) => eprintln!("Overlay thread error: {:?}", e),
    })
}

// ─── Per-monitor overlay state ──────────────────────────────────────────────

#[derive(Clone)]
struct OverlayState {
    hwnd: Option<HWND>,
    _handle: Option<Arc<std::thread::JoinHandle<()>>>,
}

// ─── Controller ─────────────────────────────────────────────────────────────

struct Controller {
    monitors: Vec<MonitorInfo>,
    selected: Vec<bool>,
    overlays_active: bool,
    overlay_states: Vec<OverlayState>,
    opacity: u8,
    hwnd_tx: mpsc::Sender<(usize, usize)>, // (monitor_index, hwnd_ptr)
    hwnd_rx: mpsc::Receiver<(usize, usize)>,
    slider_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl Controller {
    fn new(monitors: Vec<MonitorInfo>) -> Self {
        let n = monitors.len();
        let (tx, rx) = mpsc::channel();
        Controller {
            monitors,
            selected: vec![false; n],
            overlays_active: false,
            overlay_states: vec![
                OverlayState {
                    hwnd: None,
                    _handle: None,
                };
                n
            ],
            opacity: 50, // ~20% overlay darkness
            hwnd_tx: tx,
            hwnd_rx: rx,
            slider_bounds: Rc::new(Cell::new(None)),
        }
    }

    fn activate_overlays(&mut self) {
        for i in 0..self.monitors.len() {
            if self.selected[i] && self.overlay_states[i].hwnd.is_none() {
                let mon = &self.monitors[i];
                let cfg = OverlayConfig {
                    opacity: self.opacity,
                    x: mon.x,
                    y: mon.y,
                    width: mon.width,
                    height: mon.height,
                };
                let idx = i;
                let tx = self.hwnd_tx.clone();
                let (inner_tx, inner_rx) = mpsc::channel::<usize>();
                let handle = spawn_overlay(cfg, inner_tx);

                // Try to receive HWND quickly
                if let Ok(ptr) = inner_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                    self.overlay_states[i].hwnd = Some(HWND(ptr as *mut c_void));
                    // Also send to main channel for deferred processing
                    let _ = tx.send((idx, ptr));
                }
                self.overlay_states[i]._handle = Some(Arc::new(handle));
            }
        }
    }

    fn deactivate_overlays(&mut self) {
        for i in 0..self.overlay_states.len() {
            if let Some(hwnd) = self.overlay_states[i].hwnd {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            self.overlay_states[i].hwnd = None;
            self.overlay_states[i]._handle = None;
        }
    }

    fn update_all_opacity(&self) {
        for state in &self.overlay_states {
            if let Some(hwnd) = state.hwnd {
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        WM_UPDATE_OPACITY,
                        WPARAM(self.opacity as usize),
                        LPARAM(0),
                    );
                }
            }
        }
    }

    fn opacity_percent(&self) -> u8 {
        ((self.opacity as f32 / 255.0) * 100.0).round() as u8
    }
}

impl Render for Controller {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        // Drain any pending HWND notifications
        while let Ok((idx, ptr)) = self.hwnd_rx.try_recv() {
            if idx < self.overlay_states.len() {
                self.overlay_states[idx].hwnd = Some(HWND(ptr as *mut c_void));
            }
        }

        let is_active = self.overlays_active;
        let opacity_val = self.opacity;
        let opacity_pct = self.opacity_percent();
        let any_selected = self.selected.iter().any(|&s| s);

        // Build monitor list
        let mut monitor_list = div().flex().flex_col().gap_2().w_full().max_w(px(500.0));

        for (i, mon) in self.monitors.iter().enumerate() {
            let is_selected = self.selected[i];
            let has_overlay = self.overlay_states[i].hwnd.is_some();

            let display_name = if mon.name.is_empty() {
                format!("Monitor {}", i + 1)
            } else {
                // Clean the device name: \\.\DISPLAY1 -> DISPLAY1
                let clean = mon.name.replace("\\\\.\\", "");
                format!("{} ({})", clean, i + 1)
            };

            let resolution = format!("{}x{}", mon.width, mon.height);
            let position = format!("pos: ({}, {})", mon.x, mon.y);

            let status_text = if has_overlay && is_active {
                "● aktywny"
            } else {
                ""
            };

            let idx = i;
            let row = div()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
                .w_full()
                .rounded(px(8.0))
                .bg(if is_selected {
                    rgb(0x1e3a1e)
                } else {
                    rgb(0x1e1e1e)
                })
                .border_1()
                .border_color(if is_selected {
                    rgb(0x4CAF50)
                } else {
                    rgb(0x333333)
                })
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _window, cx| {
                        if !this.overlays_active {
                            this.selected[idx] = !this.selected[idx];
                            cx.notify();
                        }
                    }),
                )
                .child(checkbox(
                    is_selected,
                    cx.listener(move |this, _, _window, cx| {
                        if !this.overlays_active {
                            this.selected[idx] = !this.selected[idx];
                            cx.notify();
                        }
                    }),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .flex_grow()
                        .child(
                            div().flex().items_center().gap_2().child(
                                div()
                                    .text_color(rgb(0xffffff))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("🖥️ {}", display_name)),
                            ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(div().text_sm().text_color(rgb(0x888888)).child(resolution))
                                .child(div().text_sm().text_color(rgb(0x666666)).child(position)),
                        ),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x4CAF50))
                        .child(status_text.to_string()),
                );

            monitor_list = monitor_list.child(row);
        }

        // Slider track
        let slider_width: f32 = 400.0;
        let knob_position = (opacity_val as f32 / 255.0) * slider_width;

        let slider = div()
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .max_w(px(500.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0xcccccc))
                            .child("Intensywność overlaya:"),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .bg(rgb(0x2a2a2a))
                            .rounded(px(6.0))
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0x4CAF50))
                            .child(format!("{}%", opacity_pct)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_sm().text_color(rgb(0x666666)).child("0%"))
                    .child(
                        // Outer wrapper to capture slider bounds via on_children_prepainted
                        div()
                            .on_children_prepainted({
                                let bounds_cell = self.slider_bounds.clone();
                                move |bounds, _window, _cx| {
                                    if let Some(b) = bounds.first() {
                                        bounds_cell.set(Some(*b));
                                    }
                                }
                            })
                            .child(
                                // Slider track container
                                div()
                                    .relative()
                                    .w(px(slider_width))
                                    .h(px(28.0))
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, ev: &gpui::MouseDownEvent, _window, cx| {
                                                if let Some(bounds) = this.slider_bounds.get() {
                                                    let origin_x: f32 = bounds.origin.x.into();
                                                    let width: f32 = bounds.size.width.into();
                                                    let mouse_x: f32 = ev.position.x.into();
                                                    let relative_x = mouse_x - origin_x;
                                                    let fraction =
                                                        (relative_x / width).clamp(0.0, 1.0);
                                                    let new_opacity =
                                                        (fraction * 255.0).round() as u8;
                                                    if this.opacity != new_opacity {
                                                        this.opacity = new_opacity;
                                                        if this.overlays_active {
                                                            this.update_all_opacity();
                                                        }
                                                        cx.notify();
                                                    }
                                                }
                                            },
                                        ),
                                    )
                                    .on_mouse_move(cx.listener(
                                        move |this, ev: &gpui::MouseMoveEvent, _window, cx| {
                                            if ev.pressed_button == Some(MouseButton::Left) {
                                                if let Some(bounds) = this.slider_bounds.get() {
                                                    let origin_x: f32 = bounds.origin.x.into();
                                                    let width: f32 = bounds.size.width.into();
                                                    let mouse_x: f32 = ev.position.x.into();
                                                    let relative_x = mouse_x - origin_x;
                                                    let fraction =
                                                        (relative_x / width).clamp(0.0, 1.0);
                                                    let new_opacity =
                                                        (fraction * 255.0).round() as u8;
                                                    if this.opacity != new_opacity {
                                                        this.opacity = new_opacity;
                                                        if this.overlays_active {
                                                            this.update_all_opacity();
                                                        }
                                                        cx.notify();
                                                    }
                                                }
                                            }
                                        },
                                    ))
                                    .child(
                                        // Background track
                                        div()
                                            .absolute()
                                            .left(px(0.0))
                                            .top(px(10.0))
                                            .w(px(slider_width))
                                            .h(px(8.0))
                                            .rounded(px(4.0))
                                            .bg(rgb(0x333333)),
                                    )
                                    .child(
                                        // Filled portion
                                        div()
                                            .absolute()
                                            .left(px(0.0))
                                            .top(px(10.0))
                                            .w(px(knob_position))
                                            .h(px(8.0))
                                            .rounded(px(4.0))
                                            .bg(rgb(0x4CAF50)),
                                    )
                                    .child(
                                        // Knob
                                        div()
                                            .absolute()
                                            .left(px(knob_position - 8.0))
                                            .top(px(6.0))
                                            .w(px(16.0))
                                            .h(px(16.0))
                                            .rounded_full()
                                            .bg(rgb(0xffffff))
                                            .border_2()
                                            .border_color(rgb(0x4CAF50)),
                                    ),
                            ),
                    )
                    .child(div().text_sm().text_color(rgb(0x666666)).child("100%")),
            )
            .child(
                // Quick preset buttons
                div()
                    .flex()
                    .gap_2()
                    .mt_1()
                    .child(opacity_preset_btn(10, opacity_val, cx))
                    .child(opacity_preset_btn(20, opacity_val, cx))
                    .child(opacity_preset_btn(30, opacity_val, cx))
                    .child(opacity_preset_btn(50, opacity_val, cx))
                    .child(opacity_preset_btn(70, opacity_val, cx)),
            );

        // Main layout
        div()
            .flex()
            .flex_col()
            .gap_5()
            .size_full()
            .p_6()
            .items_center()
            .bg(rgb(0x0e0e0e))
            // Title
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xffffff))
                            .child("🛡️ OLED Care"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x888888))
                            .child("Ochrona wyświetlaczy OLED przed wypaleniem"),
                    ),
            )
            // Separator
            .child(div().w_full().max_w(px(500.0)).h(px(1.0)).bg(rgb(0x333333)))
            // Monitor list header
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .max_w(px(500.0))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0xcccccc))
                            .child(format!("Monitory ({})", self.monitors.len())),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x666666))
                            .child(if is_active {
                                "🔒 Zaznaczanie zablokowane"
                            } else {
                                "Zaznacz monitory do ochrony"
                            }),
                    ),
            )
            // Monitor list
            .child(monitor_list)
            // Separator
            .child(div().w_full().max_w(px(500.0)).h(px(1.0)).bg(rgb(0x333333)))
            // Opacity slider
            .child(slider)
            // Separator
            .child(div().w_full().max_w(px(500.0)).h(px(1.0)).bg(rgb(0x333333)))
            // Activate switch row
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .max_w(px(500.0))
                    .px_4()
                    .py_4()
                    .rounded(px(12.0))
                    .bg(if is_active {
                        rgb(0x1e3a1e)
                    } else {
                        rgb(0x1e1e1e)
                    })
                    .border_1()
                    .border_color(if is_active {
                        rgb(0x4CAF50)
                    } else {
                        rgb(0x333333)
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0xffffff))
                                    .child(if is_active {
                                        "✅ Ochrona aktywna"
                                    } else {
                                        "Włącz ochronę"
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(if is_active {
                                        rgb(0x81C784)
                                    } else {
                                        rgb(0x888888)
                                    })
                                    .child(if is_active {
                                        let active_count = self
                                            .overlay_states
                                            .iter()
                                            .filter(|s| s.hwnd.is_some())
                                            .count();
                                        format!("Aktywne overlaye: {}", active_count)
                                    } else if !any_selected {
                                        "Zaznacz monitory, aby włączyć".to_string()
                                    } else {
                                        let sel_count =
                                            self.selected.iter().filter(|&&s| s).count();
                                        format!("Zaznaczono {} monitor(ów)", sel_count)
                                    }),
                            ),
                    )
                    .child(switch(
                        is_active,
                        cx.listener(move |this, _, _window, cx| {
                            if this.overlays_active {
                                // Deactivate
                                this.deactivate_overlays();
                                this.overlays_active = false;
                            } else {
                                // Only activate if something is selected
                                if this.selected.iter().any(|&s| s) {
                                    this.overlays_active = true;
                                    this.activate_overlays();
                                }
                            }
                            cx.notify();
                        }),
                    )),
            )
            // Info footer
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .items_center()
                    .mt_2()
                    .child(
                        div().text_xs().text_color(rgb(0x555555)).child(
                            "⚠️ Kursor i menu Start nie są przyciemniane (ograniczenie Win32)",
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x444444))
                            .child("Overlay jest click-through — nie blokuje interakcji"),
                    ),
            )
    }
}

fn opacity_preset_btn(
    percent: u8,
    current_opacity: u8,
    cx: &mut gpui::Context<Controller>,
) -> impl IntoElement {
    let target_val = ((percent as f32 / 100.0) * 255.0).round() as u8;
    let is_current = (current_opacity as i16 - target_val as i16).unsigned_abs() < 4;

    div()
        .px_3()
        .py_1()
        .rounded(px(6.0))
        .bg(if is_current {
            rgb(0x4CAF50)
        } else {
            rgb(0x2a2a2a)
        })
        .text_sm()
        .text_color(if is_current {
            rgb(0xffffff)
        } else {
            rgb(0x888888)
        })
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _window, cx| {
                this.opacity = target_val;
                if this.overlays_active {
                    this.update_all_opacity();
                }
                cx.notify();
            }),
        )
        .child(format!("{}%", percent))
}

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() {
    println!("=== OLED Care - Display Protection System ===");

    // Register overlay window class
    unsafe {
        if let Err(e) = register_overlay_class() {
            eprintln!("Failed to register window class: {:?}", e);
            return;
        }
    }

    // Enumerate monitors
    let monitors = enumerate_monitors();
    println!("Found {} monitor(s):", monitors.len());
    for (i, mon) in monitors.iter().enumerate() {
        println!(
            "  [{}] {} - {}x{} at ({}, {})",
            i, mon.name, mon.width, mon.height, mon.x, mon.y
        );
    }

    if monitors.is_empty() {
        eprintln!("No monitors found!");
        return;
    }

    Application::new().run(move |app: &mut gpui::App| {
        let monitors_clone = monitors.clone();

        app.open_window(
            WindowOptions {
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("OLED Care".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(move |_| Controller::new(monitors_clone)),
        )
        .unwrap();
    });
}
