use gpui::prelude::*;
use gpui::{MouseButton, div, px, rgb};

/// A checkbox component with a checkmark indicator.
///
/// Renders as a small rounded square that shows a "✓" when checked.
/// Stops mouse event propagation so the parent row does not also fire its handler.
pub fn checkbox(
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
        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
            cx.stop_propagation();
            on_click(&checked, window, cx);
        })
        .child(
            div()
                .text_color(rgb(0xffffff))
                .child(if checked { "✓" } else { "" }),
        )
}
