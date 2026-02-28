use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc;

use gpui::prelude::*;
use gpui::{Bounds, FontWeight, Pixels, div, px, rgb};

use crate::monitor::MonitorInfo;
use crate::overlay::OverlayManager;
use crate::ui::components::{opacity_slider, switch};
use crate::ui::monitor_list::monitor_list;

/// Central application controller.
///
/// Owns all shared state: the list of monitors, which ones are selected,
/// the current opacity value, and the [`OverlayManager`] that drives the
/// Win32 overlay windows.
pub struct Controller {
    /// Information about every connected monitor.
    pub monitors: Vec<MonitorInfo>,
    /// Per-monitor selection flags (same length as `monitors`).
    pub selected: Vec<bool>,
    /// Whether overlay protection is currently enabled.
    pub overlays_active: bool,
    /// Manages the lifecycle of per-monitor overlay windows.
    pub overlay_manager: OverlayManager,
    /// Current overlay opacity (0–255).
    pub opacity: u8,
    /// Sender for `(monitor_index, hwnd_ptr)` notifications from overlay threads.
    pub hwnd_tx: mpsc::Sender<(usize, usize)>,
    /// Receiver for `(monitor_index, hwnd_ptr)` notifications from overlay threads.
    hwnd_rx: mpsc::Receiver<(usize, usize)>,
    /// Cached bounds of the slider track element (updated every frame via
    /// `on_children_prepainted`). Stored in an `Rc<Cell>` so the prepaint
    /// closure can write to it without requiring `&mut self`.
    pub slider_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl Controller {
    /// Create a new controller for the given set of monitors.
    pub fn new(monitors: Vec<MonitorInfo>) -> Self {
        let n = monitors.len();
        let (tx, rx) = mpsc::channel();
        Self {
            monitors,
            selected: vec![false; n],
            overlays_active: false,
            overlay_manager: OverlayManager::new(n),
            opacity: 50, // ~20 % darkness
            hwnd_tx: tx,
            hwnd_rx: rx,
            slider_bounds: Rc::new(Cell::new(None)),
        }
    }
}

impl Render for Controller {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        // ── Drain pending HWND notifications from overlay threads ────────
        while let Ok((idx, ptr)) = self.hwnd_rx.try_recv() {
            self.overlay_manager.register_hwnd(idx, ptr);
        }

        // ── Snapshot values for the closures / builders below ────────────
        let is_active = self.overlays_active;
        let opacity_val = self.opacity;
        let any_selected = self.selected.iter().any(|&s| s);

        // Pre-compute which monitors currently have a live overlay.
        let overlay_alive: Vec<bool> = self
            .overlay_manager
            .states
            .iter()
            .map(|s| s.hwnd.is_some())
            .collect();

        // ── Monitor list ─────────────────────────────────────────────────
        let mon_list = monitor_list(
            &self.monitors,
            &self.selected,
            &overlay_alive,
            is_active,
            cx,
        );

        // ── Opacity slider ───────────────────────────────────────────────
        let slider = opacity_slider(opacity_val, &self.slider_bounds, is_active, cx);

        // ── Activation panel ─────────────────────────────────────────────
        let active_count = self.overlay_manager.active_count();

        let activation_panel = div()
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
                                "✅ Protection active"
                            } else {
                                "Enable protection"
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
                                format!("Active overlays: {}", active_count)
                            } else if !any_selected {
                                "Select monitors to enable".to_string()
                            } else {
                                let sel = self.selected.iter().filter(|&&s| s).count();
                                format!("{} monitor(s) selected", sel)
                            }),
                    ),
            )
            .child(switch(
                is_active,
                cx.listener(move |this, _, _window, cx| {
                    if this.overlays_active {
                        this.overlay_manager.deactivate();
                        this.overlays_active = false;
                    } else if this.selected.iter().any(|&s| s) {
                        this.overlays_active = true;
                        this.overlay_manager.activate(
                            &this.monitors,
                            &this.selected,
                            this.opacity,
                            &this.hwnd_tx,
                        );
                    }
                    cx.notify();
                }),
            ));

        // ── Separator helper ─────────────────────────────────────────────
        let sep = || div().w_full().max_w(px(500.0)).h(px(1.0)).bg(rgb(0x333333));

        // ── Assemble the full layout ─────────────────────────────────────
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
                            .child("Protect your OLED display from burn-in"),
                    ),
            )
            .child(sep())
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
                            .child(format!("Monitors ({})", self.monitors.len())),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x666666))
                            .child(if is_active {
                                "🔒 Selection locked"
                            } else {
                                "Select monitors to protect"
                            }),
                    ),
            )
            // Monitor list
            .child(mon_list)
            .child(sep())
            // Opacity slider
            .child(slider)
            .child(sep())
            // Activation panel
            .child(activation_panel)
    }
}
