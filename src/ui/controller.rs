use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, AnyElement, Bounds, FontWeight, MouseButton, Pixels, div, px, rgb,
};
use std::f32::consts::PI;

use crate::ipc::{DaemonState, UiMsg};
use crate::monitor::MonitorInfo;
use crate::ui::components::{opacity_from_mouse, opacity_slider, switch};
use crate::ui::monitor_list::monitor_list;

/// Central application controller.
///
/// Holds a local optimistic cache of daemon state, plus the IPC channels used
/// to send commands to the daemon and receive state-update replies.
pub struct Controller {
    // ── State synced from daemon (local optimistic cache) ─────────────────
    /// Information about every connected monitor.
    pub monitors: Vec<MonitorInfo>,
    /// Per-monitor selection flags (same length as `monitors`).
    pub selected: Vec<bool>,
    /// Whether overlay protection is currently enabled.
    pub overlays_active: bool,
    /// Current overlay opacity (0–255).
    pub opacity: u8,
    /// Per-monitor flag: true if the daemon reports an active overlay window.
    overlay_alive: Vec<bool>,

    // ── IPC channels ──────────────────────────────────────────────────────
    /// Send commands to the background IPC thread (→ daemon).
    pub cmd_tx: mpsc::SyncSender<UiMsg>,
    /// Receive state snapshots pushed by the background IPC thread (← daemon).
    state_rx: mpsc::Receiver<DaemonState>,

    // ── UI-only state (unchanged from before) ─────────────────────────────
    /// Monotonically-incrementing counter, advanced by one on every *effective*
    /// toggle (i.e. only when `overlays_active` actually changes).
    ///
    /// Passed to `switch()` as part of the animation `ElementId`; changing it
    /// causes GPUI to create a fresh `AnimationState` so the transition
    /// replays from the beginning on each click.
    switch_click_count: u64,
    /// Incremented each time the user tries to enable protection without any
    /// monitors selected.  Baked into the animation `ElementId` to restart
    /// the shake transition from the beginning on each failed attempt.
    shake_count: u64,
    /// Cached bounds of the slider track element (updated every frame via
    /// `on_children_prepainted`). Stored in an `Rc<Cell>` so the prepaint
    /// closure can write to it without requiring `&mut self`.
    pub slider_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Whether the user is currently dragging the opacity slider.
    /// When true, a full-screen transparent capture overlay is rendered
    /// so the drag continues even when the cursor leaves the slider bounds.
    pub is_dragging: bool,
    /// Timestamp of the last combined UI + overlay flush during a slider drag.
    /// Throttles `cx.notify()` and IPC sends to ~60 fps so that fast mouse
    /// movement (1000 Hz polling) does not flood the daemon or GPUI pipeline.
    pub last_drag_flush: Option<Instant>,
}

impl Controller {
    pub fn new(
        initial: DaemonState,
        cmd_tx: mpsc::SyncSender<UiMsg>,
        state_rx: mpsc::Receiver<DaemonState>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        // Spawn a background task that wakes up every 100 ms to pull the
        // latest daemon state.  Sending GetState means the IPC thread will
        // push a fresh DaemonState into state_rx, which render() then drains.
        // This also catches state changes made outside the UI (e.g. the
        // system-tray toggle) without requiring a daemon→UI push protocol.
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if weak
                    .update(cx, |this, cx| {
                        // try_send won't block; if the channel is full we
                        // simply skip this tick and catch up on the next one.
                        let _ = this.cmd_tx.try_send(UiMsg::GetState);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        Self {
            monitors: initial.monitors,
            selected: initial.selected,
            overlays_active: initial.overlays_active,
            opacity: initial.opacity,
            overlay_alive: initial.overlay_alive,
            cmd_tx,
            state_rx,
            switch_click_count: 0,
            shake_count: 0,
            slider_bounds: Rc::new(Cell::new(None)),
            is_dragging: false,
            last_drag_flush: None,
        }
    }
}

impl Render for Controller {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        // ── Drain pending state updates from daemon ───────────────────────
        while let Ok(state) = self.state_rx.try_recv() {
            self.monitors = state.monitors;
            self.selected = state.selected;
            self.opacity = state.opacity;
            self.overlays_active = state.overlays_active;
            self.overlay_alive = state.overlay_alive;
        }

        // ── Snapshot values for the closures / builders below ────────────
        let is_active = self.overlays_active;
        let opacity_val = self.opacity;
        let is_dragging = self.is_dragging;
        let any_selected = self.selected.iter().any(|&s| s);
        let switch_click_count = self.switch_click_count;
        let shake_count = self.shake_count;

        // Pre-compute which monitors currently have a live overlay.
        let overlay_alive = self.overlay_alive.clone();

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
        let active_count = self.overlay_alive.iter().filter(|&&a| a).count();

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
                switch_click_count,
                cx.listener(move |this, _, _window, cx| {
                    if this.overlays_active {
                        // Optimistic update + send command
                        this.overlays_active = false;
                        this.switch_click_count += 1;
                        this.shake_count = 0;
                        let _ = this.cmd_tx.try_send(UiMsg::SetActive(false));
                    } else if this.selected.iter().any(|&s| s) {
                        this.overlays_active = true;
                        this.switch_click_count += 1;
                        this.shake_count = 0;
                        let _ = this.cmd_tx.try_send(UiMsg::SetActive(true));
                    } else {
                        // No monitors selected — shake the hint label.
                        this.shake_count += 1;
                    }
                    cx.notify();
                }),
            ));

        // ── Separator helper ─────────────────────────────────────────────
        let sep = || div().w_full().max_w(px(500.0)).h(px(1.0)).bg(rgb(0x333333));

        // ── Assemble the full layout ─────────────────────────────────────
        div()
            .relative()
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
                    .child({
                        // Build the hint label.  When the user clicks the
                        // switch with no monitors selected we play a
                        // horizontal-shake + orange-to-grey colour fade.
                        let hint: AnyElement = if shake_count > 0 && !is_active {
                            div()
                                .text_sm()
                                .child("Select monitors to protect")
                                .with_animation(
                                    ("monitor_hint_shake", shake_count),
                                    Animation::new(Duration::from_millis(500)),
                                    |el, delta| {
                                        // Decaying sinusoidal offset (3 oscillations).
                                        let offset = (delta * PI * 6.0).sin() * 8.0 * (1.0 - delta);
                                        // Colour: orange (0xee6b2f) → grey (0x666666).
                                        let t = delta.clamp(0.0, 1.0);
                                        let r = (0xeeu8 as f32
                                            + (0x66u8 as f32 - 0xeeu8 as f32) * t)
                                            .round()
                                            as u32;
                                        let g = (0x6bu8 as f32
                                            + (0x66u8 as f32 - 0x6bu8 as f32) * t)
                                            .round()
                                            as u32;
                                        let b = (0x2fu8 as f32
                                            + (0x66u8 as f32 - 0x2fu8 as f32) * t)
                                            .round()
                                            as u32;
                                        let color = (r << 16) | (g << 8) | b;
                                        // relative() + left() supports negative offsets
                                        // (ml() clamps to 0 in the flex engine).
                                        el.text_color(rgb(color)).relative().left(px(offset))
                                    },
                                )
                                .into_any_element()
                        } else if is_active {
                            div()
                                .text_sm()
                                .text_color(rgb(0x666666))
                                .child("🔒 Selection locked")
                                .into_any_element()
                        } else {
                            div()
                                .text_sm()
                                .text_color(rgb(0x666666))
                                .child("Select monitors to protect")
                                .into_any_element()
                        };
                        hint
                    }),
            )
            // Monitor list
            .child(mon_list)
            .child(sep())
            // Opacity slider
            .child(slider)
            .child(sep())
            // Activation panel
            .child(activation_panel)
            .when(is_dragging, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(0.0))
                        .left(px(0.0))
                        .w_full()
                        .h_full()
                        .on_mouse_move(cx.listener(
                            |this, ev: &gpui::MouseMoveEvent, _window, cx| {
                                if let Some(new_opacity) =
                                    opacity_from_mouse(ev.position.x, &this.slider_bounds)
                                {
                                    // Always update the raw state — writing a u8 is free.
                                    if this.opacity != new_opacity {
                                        this.opacity = new_opacity;
                                    }

                                    // Throttle the expensive operations (GPUI re-render +
                                    // IPC send) to ~60 fps so that a 1000 Hz mouse doesn't
                                    // flood the daemon or the GPUI render pipeline.
                                    let now = Instant::now();
                                    let elapsed = this
                                        .last_drag_flush
                                        .map_or(Duration::MAX, |t| now.duration_since(t));

                                    if elapsed >= Duration::from_millis(16) {
                                        this.last_drag_flush = Some(now);
                                        let _ =
                                            this.cmd_tx.try_send(UiMsg::SetOpacity(this.opacity));
                                        cx.notify();
                                    }
                                }
                            },
                        ))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _window, cx| {
                                this.is_dragging = false;
                                this.last_drag_flush = None;
                                let _ = this.cmd_tx.try_send(UiMsg::SetOpacity(this.opacity));
                                cx.notify();
                            }),
                        ),
                )
            })
    }
}
