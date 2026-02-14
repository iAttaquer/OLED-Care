use gpui::prelude::*;
use gpui::{FontWeight, MouseButton, div, px, rgb};

use crate::monitor::MonitorInfo;
use crate::ui::components::checkbox;
use crate::ui::controller::Controller;

/// Build the monitor list section: a vertical stack of selectable monitor rows.
///
/// Each row displays the monitor's device name, resolution, position, and
/// an activity indicator. Clicking a row toggles its selection — but only
/// when overlays are **not** currently active (to prevent mid-flight changes).
pub fn monitor_list(
    monitors: &[MonitorInfo],
    selected: &[bool],
    overlay_hwnds: &[bool],
    overlays_active: bool,
    cx: &mut gpui::Context<Controller>,
) -> impl IntoElement + use<> {
    let mut list = div().flex().flex_col().gap_2().w_full().max_w(px(500.0));

    for (i, mon) in monitors.iter().enumerate() {
        let is_selected = selected.get(i).copied().unwrap_or(false);
        let has_overlay = overlay_hwnds.get(i).copied().unwrap_or(false);

        let display_name = if mon.name.is_empty() {
            format!("Monitor {}", i + 1)
        } else {
            let clean = mon.name.replace("\\\\.\\", "");
            format!("{} ({})", clean, i + 1)
        };

        let resolution = format!("{}x{}", mon.width, mon.height);
        let position = format!("pos: ({}, {})", mon.x, mon.y);

        let status_text = if has_overlay && overlays_active {
            "● active"
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

        list = list.child(row);
    }

    list
}
