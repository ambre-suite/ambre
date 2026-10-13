// ambre-chrome/src/color.rs
// ============================================================================
// Shared, app-agnostic colour control: a single toolbar button toggling a
// popover. The popover combines a reset entry ("Automatic" / "No fill"), a
// curated swatch palette, and an embedded native `<input type="color">` for
// arbitrary colours. Originally Trame's cell fg/bg picker (`trame-cell-styles`);
// promoted here so Toile's style inspector (`toile-styles`) shares the very same
// control.
//
// The value is a packed `u32`. With `alpha = false` (default) it is a plain
// `0xRRGGBB` (Trame's cells): the native picker covers the whole colour. With
// `alpha = true` it is a packed `0xAARRGGBB` (Toile's `ElementStyle` fill /
// stroke). The native `<input type="color">` still drives the RGB channel, since
// the widget has no alpha. An extra `<input type="range">` drives the top alpha
// byte, and the two combine into the ARGB the caller stores.
//
// Both widgets fire on every step of a drag, so each reports through its own
// `Debouncer`: a drag leaves as one pick, the last (`CHROME-015`). A swatch is
// a click and reports at once.
// ============================================================================

use crate::debounce::{DRAG_DEBOUNCE, Debouncer};

use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

/// Curated swatch palette for the colour picker popover (packed `0xRRGGBB`).
/// A small office-style set: greys, then a saturated row and a muted row.
const PALETTE: &[u32] = &[
    0x000000, 0x404040, 0x808080, 0xb0b0b0, 0xd9d9d9, 0xffffff, 0xc00000, 0xff0000, 0xffc000,
    0xffff00, 0x92d050, 0x00b050, 0x00b0f0, 0x0070c0, 0x002060, 0x7030a0,
];

/// Parse an `<input type="color">` value (`"#rrggbb"`) or an `"#aarrggbb"` string
/// into a packed `u32`. Both 6- and 8-hex-digit forms parse; a shorter/garbled
/// string yields `None`.
pub fn parse_hex_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    u32::from_str_radix(hex, 16).ok()
}

/// Format the RGB channel of a packed colour into a 6-digit `<input type="color">`
/// value, using `default` when unset (`<input type=color>` cannot be empty). The
/// top alpha byte is masked off -- the native widget is RGB-only; alpha is carried
/// by the separate slider. `default` is the caller's themed on-screen colour, so
/// an "Automatic" / "Transparent" value shows its real colour, tracking the theme.
pub fn color_hex(c: Option<u32>, default: u32) -> String {
    format!("#{:06x}", c.unwrap_or(default) & 0x00ff_ffff)
}

/// The alpha byte of a packed ARGB colour, defaulting to fully opaque when unset.
fn alpha_of(c: Option<u32>) -> u8 {
    c.map_or(0xff, |v| (v >> 24) as u8)
}

/// Combine an RGB channel (`0xRRGGBB`) and an alpha byte into the stored value:
/// packed ARGB when `alpha`, or the bare RGB otherwise.
fn pack(rgb: u32, a: u8, alpha: bool) -> u32 {
    if alpha {
        ((a as u32) << 24) | (rgb & 0x00ff_ffff)
    } else {
        rgb & 0x00ff_ffff
    }
}

/// A CSS colour string for the trigger's tint strip: `rgba(...)` when alpha is in
/// play (so transparency reads through), else a plain `#rrggbb`.
fn strip_css(c: Option<u32>, default: u32, alpha: bool) -> String {
    let v = c.unwrap_or(default);
    if alpha {
        let a = (v >> 24) & 0xff;
        format!(
            "rgba({},{},{},{:.3})",
            (v >> 16) & 0xff,
            (v >> 8) & 0xff,
            v & 0xff,
            a as f32 / 255.0
        )
    } else {
        format!("#{:06x}", v & 0x00ff_ffff)
    }
}

/// A composed colour control. `on_pick` receives `Some(packed)` for a chosen
/// colour or `None` for the reset entry. `alpha = true` switches the stored value
/// to packed ARGB and reveals an opacity slider (see the module header).
#[component]
pub fn ColorPopover(
    title: &'static str,
    icon: icondata::Icon,
    current: RwSignal<Option<u32>>,
    default: Signal<u32>,
    reset_label: &'static str,
    on_pick: Callback<Option<u32>>,
    /// Edit a packed ARGB `u32` with an alpha slider (Toile), instead of a bare
    /// `0xRRGGBB` (Trame). Defaults to `false`.
    #[prop(optional)]
    alpha: bool,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let (rgb_drag, alpha_drag) = (Debouncer::new(), Debouncer::new());
    // The wrapper node, so a global pointerdown can tell clicks inside the popover
    // apart from clicks elsewhere (another cell, the canvas, a sibling control).
    let root_ref = NodeRef::<html::Span>::new();
    // Close on any pointer-down outside the control: selecting another cell hits
    // the canvas, which is outside this node, so the popover dismisses.
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if !open.get_untracked() {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || handle.remove());
    view! {
        <span class="ambre-color" node_ref=root_ref>
            <button
                aria-label=title
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                // Keep the canvas' focus: a button steals it on mousedown, which
                // would clear the active selection before on:click can apply.
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=icon />
                // Thin strip tinted to the current colour (themed default when unset).
                <span
                    class="swatch-strip"
                    style:background-color=move || strip_css(current.get(), default.get(), alpha)
                ></span>
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-color-popover">
                    // Reset entry: "Automatic" (text) / "No fill (transparent)" (fill).
                    <button
                        class="reset"
                        on:mousedown=move |ev| ev.prevent_default()
                        on:click=move |_| {
                            on_pick.run(None);
                            open.set(false);
                        }
                    >
                        {reset_label}
                    </button>
                    // Swatch palette. Each swatch is RGB; in alpha mode it keeps the
                    // current opacity (fully opaque when the value is unset).
                    <div class="swatches">
                        {PALETTE
                            .iter()
                            .map(|&rgb| {
                                view! {
                                    <button
                                        aria-label=format!("#{:06x}", rgb)
                                        style:background-color=format!("#{:06x}", rgb)
                                        on:mousedown=move |ev| ev.prevent_default()
                                        on:click=move |_| {
                                            let a = alpha_of(current.get_untracked());
                                            on_pick.run(Some(pack(rgb, a, alpha)));
                                            open.set(false);
                                        }
                                    ></button>
                                }
                            })
                            .collect_view()}
                    </div>
                    // Custom colour: the standard native widget, kept for arbitrary picks.
                    <label class="custom">
                        <input
                            type="color"
                            prop:value=move || color_hex(current.get(), default.get())
                            on:input=move |ev| {
                                if let Some(rgb) = parse_hex_color(&event_target_value(&ev)) {
                                    let a = alpha_of(current.get_untracked());
                                    let picked = pack(rgb, a, alpha);
                                    rgb_drag.call(DRAG_DEBOUNCE, move || on_pick.run(Some(picked)));
                                }
                            }
                        />
                        "Custom..."
                    </label>
                    // Opacity slider, ARGB mode only: drives the top alpha byte while
                    // preserving the current RGB channel.
                    {alpha
                        .then(|| {
                            view! {
                                <label class="opacity">
                                    <input
                                        type="range"
                                        min="0"
                                        max="255"
                                        prop:value=move || alpha_of(current.get()).to_string()
                                        on:input=move |ev| {
                                            let a = event_target_value(&ev)
                                                .parse::<u8>()
                                                .unwrap_or(0xff);
                                            let rgb = current.get_untracked().unwrap_or(default.get_untracked());
                                            let picked = pack(rgb, a, true);
                                            alpha_drag.call(DRAG_DEBOUNCE, move || on_pick.run(Some(picked)));
                                        }
                                    />
                                    "Opacity"
                                </label>
                            }
                        })}
                </div>
            </Show>
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_round_trips_six_digits() {
        assert_eq!(parse_hex_color("#ff0000"), Some(0xff0000));
        assert_eq!(parse_hex_color("#000000"), Some(0x000000));
    }

    #[test]
    fn parse_hex_round_trips_eight_digits() {
        assert_eq!(parse_hex_color("#80ff0000"), Some(0x80ff_0000));
        assert_eq!(parse_hex_color("#ffffffff"), Some(0xffff_ffff));
    }

    #[test]
    fn parse_hex_rejects_garbage() {
        assert_eq!(parse_hex_color("ff0000"), None); // no leading '#'
        assert_eq!(parse_hex_color("#zzz"), None);
    }

    #[test]
    fn color_hex_masks_alpha_and_falls_back_to_default() {
        // The native input is RGB-only: the alpha byte must be stripped.
        assert_eq!(color_hex(Some(0x80ab_cdef), 0), "#abcdef");
        assert_eq!(color_hex(None, 0x123456), "#123456");
    }

    #[test]
    fn pack_builds_argb_only_in_alpha_mode() {
        assert_eq!(pack(0x00_ff00, 0x80, true), 0x8000_ff00);
        assert_eq!(pack(0x00_ff00, 0x80, false), 0x0000_ff00);
        // A leaking alpha in the rgb argument is masked off.
        assert_eq!(pack(0xffab_cdef, 0x11, true), 0x11ab_cdef);
    }

    #[test]
    fn alpha_of_defaults_opaque_when_unset() {
        assert_eq!(alpha_of(None), 0xff);
        assert_eq!(alpha_of(Some(0x80ff_0000)), 0x80);
    }
}
