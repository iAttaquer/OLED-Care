mod monitor;
mod overlay;
mod ui;

use gpui::{AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};

use crate::monitor::enumerate_monitors;
use crate::overlay::register_overlay_class;
use crate::ui::Controller;

fn main() {
    println!("=== OLED Care - Display Protection System ===");

    // Register the Win32 overlay window class (once, before any windows are created).
    unsafe {
        if let Err(e) = register_overlay_class() {
            eprintln!("Failed to register overlay window class: {:?}", e);
            return;
        }
    }

    // Discover connected monitors.
    let monitors = enumerate_monitors();
    println!("Found {} monitor(s):", monitors.len());
    for (i, mon) in monitors.iter().enumerate() {
        println!(
            "  [{}] {} — {}x{} at ({}, {})",
            i, mon.name, mon.width, mon.height, mon.x, mon.y,
        );
    }

    if monitors.is_empty() {
        eprintln!("No monitors detected — nothing to protect.");
        return;
    }

    // Launch the GPUI control window.
    Application::new().run(move |app: &mut gpui::App| {
        let monitors_clone = monitors.clone();

        app.open_window(
            WindowOptions {
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("OLED Care".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(530.0), px(720.0)),
                    app,
                ))),
                is_resizable: false,
                ..Default::default()
            },
            |_, cx| cx.new(move |_| Controller::new(monitors_clone)),
        )
        .unwrap();
    });
}
