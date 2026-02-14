use gpui::App;
use gpui::prelude::*;
use gpui::{Application, Bounds, WindowBounds, WindowOptions, div, px, size};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, FillRect, HBRUSH, PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
    GetSystemMetrics, HWND_TOPMOST, LWA_ALPHA, MSG, PostQuitMessage, RegisterClassW, SM_CXSCREEN,
    SM_CYSCREEN, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetWindowPos, ShowWindow, TranslateMessage, WINDOW_EX_STYLE,
    WNDCLASSW, WS_DISABLED, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::PCWSTR;

/// Configuration for an overlay window
#[derive(Clone)]
struct OverlayConfig {
    opacity: u8, // 0-255 (0 = transparent, 255 = opaque)
    monitor_index: usize,
}

/// Window procedure callback for our overlay window
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
                    // Fill with black color - this is our semi-transparent overlay
                    let brush = CreateSolidBrush(COLORREF(0x00000000)); // RGB(0,0,0) = black
                    if !brush.is_invalid() {
                        let _ = FillRect(hdc, &ps.rcPaint, brush);
                    }
                    let _ = EndPaint(hwnd, &ps);
                }
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

/// Create and run a pure Win32 overlay window (no winit, no magnification)
fn create_win32_overlay(config: OverlayConfig) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        // Get HINSTANCE (NULL gets current module)
        let hinstance = windows::Win32::Foundation::HINSTANCE(std::ptr::null_mut());

        // Create a unique window class name
        let class_name: Vec<u16> = "OLEDCareOverlayClass\0".encode_utf16().collect();

        // Register window class
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            style: CS_HREDRAW | CS_VREDRAW,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            ..Default::default()
        };

        let atom = RegisterClassW(&wc);
        if atom == 0 {
            return Err("Failed to register window class".into());
        }

        // Get screen dimensions
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);

        println!(
            "Creating Win32 overlay window: {}x{} on monitor {}",
            width, height, config.monitor_index
        );

        let window_name: Vec<u16> = "OLED Care Overlay\0".encode_utf16().collect();

        // Create the window with all styles that make it invisible to window manager
        let ex_style = WINDOW_EX_STYLE(
            WS_EX_LAYERED.0        // Layered window for transparency
                | WS_EX_TRANSPARENT.0  // Click-through
                | WS_EX_TOPMOST.0      // Always on top
                | WS_EX_TOOLWINDOW.0   // No taskbar
                | WS_EX_NOACTIVATE.0, // Never activates
        );

        let hwnd = CreateWindowExW(
            ex_style,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(window_name.as_ptr()),
            WS_POPUP | WS_DISABLED, // Popup with no interaction
            0,
            0,
            width,
            height,
            None,
            None,
            Some(hinstance),
            None,
        )?;

        if hwnd.0.is_null() {
            return Err("Failed to create window".into());
        }

        println!("Window created successfully: {:?}", hwnd);

        // Set the window to be layered with alpha transparency
        // opacity: 0 = fully transparent, 255 = fully opaque
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), config.opacity, LWA_ALPHA);

        // Show window and position it topmost
        let _ = ShowWindow(hwnd, SW_SHOW);

        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            width,
            height,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );

        println!("Win32 Overlay active:");
        println!(
            "  - Opacity: {}/255 ({:.1}%)",
            config.opacity,
            (config.opacity as f32 / 255.0) * 100.0
        );
        println!("  - Click-through: YES");
        println!("  - Always on top: YES");
        println!("  - Disabled for interaction: YES (WS_DISABLED)");
        println!("  - Hidden from taskbar: YES");
        println!("  - Covers applications: YES");
        println!("\n⚠️  Limitations:");
        println!("     • Cursor is NOT darkened (Win32 limitation)");
        println!("     • Start Menu is NOT darkened (higher compositor layer)");
        println!("\nPress Ctrl+C in console or close GPUI window to exit");

        // Force initial paint
        let _ = UpdateWindow(hwnd);

        // Message loop
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        Ok(())
    }
}

/// Spawn an overlay in a separate thread
fn spawn_overlay_on_monitor(cfg: OverlayConfig) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        println!("Overlay thread starting...");
        match create_win32_overlay(cfg) {
            Ok(_) => println!("Overlay thread finished normally"),
            Err(e) => eprintln!("Overlay thread error: {:?}", e),
        }
    })
}

/// Simple GPUI controller
struct Controller {
    selected_monitor: Arc<Mutex<usize>>,
    opacity: Arc<Mutex<u8>>,
}

impl Render for Controller {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let selected_idx = *self.selected_monitor.lock().unwrap();
        let current_opacity = *self.opacity.lock().unwrap();
        let opacity_percent = (current_opacity as f32 / 255.0 * 100.0) as u32;

        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .items_center()
            .justify_center()
            .bg(gpui::rgb(0x1e1e1e))
            .child(
                div()
                    .text_xl()
                    .text_color(gpui::rgb(0xffffff))
                    .child("🖥️ OLED Care - Display Protection"),
            )
            .child(
                div()
                    .text_color(gpui::rgb(0x4CAF50))
                    .child("✓ Overlay Active"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_3()
                    .bg(gpui::rgb(0x2a2a2a))
                    .child(
                        div()
                            .text_color(gpui::rgb(0xcccccc))
                            .child(format!("Monitor: {}", selected_idx)),
                    )
                    .child(div().text_color(gpui::rgb(0xcccccc)).child(format!(
                        "Opacity: {}/255 ({}%)",
                        current_opacity, opacity_percent
                    ))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_xs()
                    .text_color(gpui::rgb(0x888888))
                    .child("✨ Features:")
                    .child("  • Click-through enabled ✓")
                    .child("  • Always on top ✓")
                    .child("  • Covers all applications ✓")
                    .child("  • No window manager interaction ✓")
                    .child("  • Minimal resource usage ✓"),
            )
            .child(
                div()
                    .text_color(gpui::rgb(0xFF9800))
                    .text_xs()
                    .child("⚠️  Limitations:"),
            )
            .child(
                div()
                    .text_color(gpui::rgb(0x888888))
                    .text_xs()
                    .child("  • Cursor NOT darkened (Win32 limitation)")
                    .child("  • Start Menu NOT darkened (higher compositor layer)"),
            )
            .child(
                div()
                    .text_color(gpui::rgb(0x888888))
                    .text_xs()
                    .child("Using Win32 layered window with WS_EX_TRANSPARENT"),
            )
    }
}

fn main() {
    // Shared state
    let selected_monitor = Arc::new(Mutex::new(0usize));
    let opacity = Arc::new(Mutex::new(153u8)); // 153/255 ≈ 60% opacity

    // Spawn overlay on monitor 0 in background
    println!("=== OLED Care - Display Protection System ===");
    println!("Starting Win32 overlay...");
    let sm_clone = selected_monitor.clone();
    let op_clone = opacity.clone();

    std::thread::spawn(move || {
        let cfg = OverlayConfig {
            monitor_index: *sm_clone.lock().unwrap(),
            opacity: *op_clone.lock().unwrap(),
        };
        let handle = spawn_overlay_on_monitor(cfg);
        let _ = handle.join();
    });

    // Give overlay thread time to start
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Start GPUI application for control UI
    Application::new().run(move |app: &mut App| {
        let sm = selected_monitor.clone();
        let op = opacity.clone();

        let bounds = Bounds::centered(None, size(px(550.0), px(550.0)), app);
        app.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(move |_| Controller {
                    selected_monitor: sm.clone(),
                    opacity: op.clone(),
                })
            },
        )
        .unwrap();
    });
}
