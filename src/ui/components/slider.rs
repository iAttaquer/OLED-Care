use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{Bounds, FontWeight, MouseButton, Pixels, div, px, rgb};

use crate::ui::controller::Controller;

/// Width of the slider track in pixels.
const SLIDER_WIDTH: f32 = 400.0;

/// Build the complete opacity slider section.
///
/// Includes a label with the current percentage, a draggable track with a knob,
/// min/max labels, and a row of preset buttons for quick selection.
pub fn opacity_slider(
    opacity: u8,
    slider_bounds: &Rc<Cell<Option<Bounds<Pixels>>>>,
    overlays_active: bool,
    cx: &mut gpui::Context<Controller>,
) -> impl IntoElement + use<> {
    let opacity_pct = ((opacity as f32 / 255.0) * 100.0).round() as u8;
    let knob_position = (opacity as f32 / 255.0) * SLIDER_WIDTH;

    div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .max_w(px(500.0))
        // Header: label + percentage badge
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
                        .child("Overlay intensity:"),
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
        // Track row: 0% — [slider] — 100%
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().text_sm().text_color(rgb(0x666666)).child("0%"))
                .child(slider_track(
                    knob_position,
                    slider_bounds,
                    overlays_active,
                    cx,
                ))
                .child(div().text_sm().text_color(rgb(0x666666)).child("100%")),
        )
        // Preset buttons
        .child(
            div()
                .flex()
                .gap_2()
                .mt_1()
                .child(preset_btn(10, opacity, overlays_active, cx))
                .child(preset_btn(20, opacity, overlays_active, cx))
                .child(preset_btn(30, opacity, overlays_active, cx))
                .child(preset_btn(50, opacity, overlays_active, cx))
                .child(preset_btn(70, opacity, overlays_active, cx)),
        )
}

/// The interactive slider track with background, fill, and draggable knob.
///
/// Uses `on_children_prepainted` on a wrapper div to capture the child bounds
/// so that mouse positions (which are in window coordinates) can be converted
/// to a relative fraction along the track.
fn slider_track(
    knob_position: f32,
    slider_bounds: &Rc<Cell<Option<Bounds<Pixels>>>>,
    _overlays_active: bool,
    cx: &mut gpui::Context<Controller>,
) -> impl IntoElement + use<> {
    div()
        // Wrapper: captures child bounds via prepaint callback
        .on_children_prepainted({
            let bounds_cell = slider_bounds.clone();
            move |bounds, _window, _cx| {
                if let Some(b) = bounds.first() {
                    bounds_cell.set(Some(*b));
                }
            }
        })
        .child(
            div()
                .relative()
                .w(px(SLIDER_WIDTH))
                .h(px(28.0))
                .flex()
                .items_center()
                .cursor_pointer()
                // Click to set value
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &gpui::MouseDownEvent, _window, cx| {
                        if let Some(new_opacity) =
                            opacity_from_mouse(ev.position.x, &this.slider_bounds)
                        {
                            if this.opacity != new_opacity {
                                this.opacity = new_opacity;
                                if this.overlays_active {
                                    this.overlay_manager.update_opacity(this.opacity);
                                }
                                cx.notify();
                            }
                        }
                    }),
                )
                // Drag to adjust value
                .on_mouse_move(
                    cx.listener(move |this, ev: &gpui::MouseMoveEvent, _window, cx| {
                        if ev.pressed_button == Some(MouseButton::Left) {
                            if let Some(new_opacity) =
                                opacity_from_mouse(ev.position.x, &this.slider_bounds)
                            {
                                if this.opacity != new_opacity {
                                    this.opacity = new_opacity;
                                    if this.overlays_active {
                                        this.overlay_manager.update_opacity(this.opacity);
                                    }
                                    cx.notify();
                                }
                            }
                        }
                    }),
                )
                // Background track
                .child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .top(px(10.0))
                        .w(px(SLIDER_WIDTH))
                        .h(px(8.0))
                        .rounded(px(4.0))
                        .bg(rgb(0x333333)),
                )
                // Filled portion
                .child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .top(px(10.0))
                        .w(px(knob_position))
                        .h(px(8.0))
                        .rounded(px(4.0))
                        .bg(rgb(0x4CAF50)),
                )
                // Knob
                .child(
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
        )
}

/// Convert a mouse X position (in window coordinates) to an opacity value
/// using the previously-captured slider bounds.
///
/// Returns `None` if bounds have not been captured yet.
fn opacity_from_mouse(
    mouse_x: Pixels,
    slider_bounds: &Rc<Cell<Option<Bounds<Pixels>>>>,
) -> Option<u8> {
    let bounds = slider_bounds.get()?;
    let origin_x: f32 = bounds.origin.x.into();
    let width: f32 = bounds.size.width.into();
    let mx: f32 = mouse_x.into();
    let relative_x = mx - origin_x;
    let fraction = (relative_x / width).clamp(0.0, 1.0);
    Some((fraction * 255.0).round() as u8)
}

/// A small button that sets the opacity to a predefined percentage.
fn preset_btn(
    percent: u8,
    current_opacity: u8,
    _overlays_active: bool,
    cx: &mut gpui::Context<Controller>,
) -> impl IntoElement + use<> {
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
                    this.overlay_manager.update_opacity(this.opacity);
                }
                cx.notify();
            }),
        )
        .child(format!("{}%", percent))
}
