use std::time::Duration;

use gpui::prelude::*;
use gpui::{Animation, AnimationExt, AnyElement, MouseButton, div, ease_in_out, px, rgb};

// ─── Colour helpers ───────────────────────────────────────────────────────────

/// Linearly interpolate between two packed 0xRRGGBB colours.
fn lerp_color(from: u32, to: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let fr = ((from >> 16) & 0xFF) as f32;
    let fg = ((from >> 8) & 0xFF) as f32;
    let fb = (from & 0xFF) as f32;
    let tr = ((to >> 16) & 0xFF) as f32;
    let tg = ((to >> 8) & 0xFF) as f32;
    let tb = (to & 0xFF) as f32;
    let r = (fr + (tr - fr) * t).round() as u32;
    let g = (fg + (tg - fg) * t).round() as u32;
    let b = (fb + (tb - fb) * t).round() as u32;
    (r << 16) | (g << 8) | b
}

// ─── Constants ────────────────────────────────────────────────────────────────

const TRACK_OFF: u32 = 0x333333;
const TRACK_ON: u32 = 0x4CAF50;
const BORDER_OFF: u32 = 0x555555;
const ANIM_MS: u64 = 200;

// ─── Component ────────────────────────────────────────────────────────────────

/// A toggle switch with a smooth animated transition driven by GPUI's built-in
/// animation system.
///
/// * `checked`     — current logical state (used by the click handler).
/// * `click_count` — incremented by the caller on every *effective* toggle.
///                   The animation restarts on each new value because the
///                   `ElementId` changes.  When `0` the element renders
///                   statically so there is no spurious slide on first paint.
/// * `on_click`    — callback invoked on mouse-down.
pub fn switch(
    checked: bool,
    click_count: u64,
    on_click: impl Fn(&bool, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    if click_count == 0 {
        // ── Initial render: static, no animation ─────────────────────────────
        // Animating from delta=0 on the very first frame would show the knob
        // at the wrong position for ~16 ms.  Avoid this by rendering the
        // correct final state directly.
        let track_clr = if checked { TRACK_ON } else { TRACK_OFF };
        let border_clr = if checked { TRACK_ON } else { BORDER_OFF };
        let knob_ml = if checked { px(20.0) } else { px(4.0) };

        div()
            .flex()
            .items_center()
            .w(px(44.0))
            .h(px(24.0))
            .rounded(px(12.0))
            .bg(rgb(track_clr))
            .border_1()
            .border_color(rgb(border_clr))
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
                    .ml(knob_ml),
            )
            .into_any_element()
    } else {
        // ── Animated transition ───────────────────────────────────────────────
        // `click_count` is baked into the ElementId so every new click value
        // creates a fresh AnimationState and the transition plays from the
        // beginning again, regardless of GPUI's render-frame caching.
        //
        // delta = 0  →  visual state BEFORE the click (previous position)
        // delta = 1  →  visual state AFTER  the click (current position)
        let (from_ml, to_ml): (f32, f32) = if checked { (4.0, 20.0) } else { (20.0, 4.0) };
        let (from_clr, to_clr): (u32, u32) = if checked {
            (TRACK_OFF, TRACK_ON)
        } else {
            (TRACK_ON, TRACK_OFF)
        };
        let (from_border, to_border): (u32, u32) = if checked {
            (BORDER_OFF, TRACK_ON)
        } else {
            (TRACK_ON, BORDER_OFF)
        };

        let anim = Animation::new(Duration::from_millis(ANIM_MS)).with_easing(ease_in_out);

        // Knob slides left ↔ right.
        let knob = div()
            .w(px(16.0))
            .h(px(16.0))
            .rounded_full()
            .bg(rgb(0xffffff))
            .with_animation(
                ("switch_knob", click_count),
                anim.clone(),
                move |el, delta| el.ml(px(from_ml + (to_ml - from_ml) * delta)),
            );

        // Track background and border colour fade simultaneously.
        div()
            .flex()
            .items_center()
            .w(px(44.0))
            .h(px(24.0))
            .rounded(px(12.0))
            .border_1()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                on_click(&checked, window, cx);
            })
            .child(knob)
            .with_animation(("switch_track", click_count), anim, move |el, delta| {
                el.bg(rgb(lerp_color(from_clr, to_clr, delta)))
                    .border_color(rgb(lerp_color(from_border, to_border, delta)))
            })
            .into_any_element()
    }
}
