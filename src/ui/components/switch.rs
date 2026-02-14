use gpui::prelude::*;
use gpui::{MouseButton, div, px, rgb};

/// A toggle switch component styled similarly to modern UI toolkits.
///
/// Renders as a pill-shaped track with a circular knob that slides between
/// the *off* (left) and *on* (right) positions.
pub fn switch(
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
