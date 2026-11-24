// ambre-chrome/src/color.rs
// ============================================================================
// Shared, app-agnostic colour control: a single toolbar button toggling a
// popover. The popover combines a reset entry ("Automatic" / "No fill"), a
// curated swatch palette, and an embedded native `<input type="color">` for
// arbitrary colours. Originally Trame's cell fg/bg picker (`trame-cell-styles`);
// promoted here so Toile's style inspector (`toile-styles`) shares the very same
// control.
//
// The value is the caller's own packing, behind [`PickedColour`]: an `Rgb`
// (Trame's cells) or an `Rgba` (Toile's `ElementStyle` fill and stroke). The
// native `<input type="color">` drives the RGB channels, since the widget has no
// alpha. A packing that carries an alpha also gets an `<input type="range">`,
// which drives the alpha channel alone.
//
// Both widgets fire on every step of a drag, so each reports through its own
// `Debouncer`: a drag leaves as one pick, the last (`CHROME-015`). A swatch is
// a click and reports at once.
// ============================================================================

use crate::debounce::{DRAG_DEBOUNCE, Debouncer};

use ambre_core::{Alpha, Rgb, Rgba};
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

/// Curated swatch palette for the colour picker popover.
/// A small office-style set: greys, then a saturated row and a muted row.
const PALETTE: &[Rgb] = &[
    Rgb::from_bits(0x00_0000),
    Rgb::from_bits(0x40_4040),
    Rgb::from_bits(0x80_8080),
    Rgb::from_bits(0xb0_b0b0),
    Rgb::from_bits(0xd9_d9d9),
    Rgb::from_bits(0xff_ffff),
    Rgb::from_bits(0xc0_0000),
    Rgb::from_bits(0xff_0000),
    Rgb::from_bits(0xff_c000),
    Rgb::from_bits(0xff_ff00),
    Rgb::from_bits(0x92_d050),
    Rgb::from_bits(0x00_b050),
    Rgb::from_bits(0x00_b0f0),
    Rgb::from_bits(0x00_70c0),
    Rgb::from_bits(0x00_2060),
    Rgb::from_bits(0x70_30a0),
];

/// The colours a field lays out inline, before its custom-colour button
/// (`CHROME-252`). They are black, a light grey and the saturated hues of the
/// palette, in its order.
/// Each one carries its name and its CSS text as constants, so a swatch
/// formats nothing at run time.
pub(crate) const INLINE: [(&str, &str, Rgb); 7] = [
    ("Black", "#000000", Rgb::from_bits(0x00_0000)),
    ("Light grey", "#d9d9d9", Rgb::from_bits(0xd9_d9d9)),
    ("Red", "#ff0000", Rgb::from_bits(0xff_0000)),
    ("Amber", "#ffc000", Rgb::from_bits(0xff_c000)),
    ("Green", "#00b050", Rgb::from_bits(0x00_b050)),
    ("Blue", "#0070c0", Rgb::from_bits(0x00_70c0)),
    ("Purple", "#7030a0", Rgb::from_bits(0x70_30a0)),
];

/// What an unset colour reads as when a pick keeps its alpha: opaque black.
const UNSET: Rgba = Rgba::from_bits(0x0000_00ff);

/// Whether the picker edits the alpha channel of a packing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaEdit {
    /// The packing has no alpha, so the popover shows no slider.
    Absent,
    /// The popover shows an opacity slider for the alpha channel.
    Slider,
}

/// A colour packing the picker edits. The picker reads and writes the four
/// channels of an [`Rgba`], and the caller keeps its own packing.
pub trait PickedColour: Copy + Send + Sync + 'static {
    /// Whether this packing carries an alpha the picker edits.
    const ALPHA: AlphaEdit;

    /// The four channels. A packing with no alpha reads as opaque.
    fn as_rgba(self) -> Rgba;

    /// The packing of four channels. A packing with no alpha drops it.
    fn from_rgba(rgba: Rgba) -> Self;
}

impl PickedColour for Rgb {
    const ALPHA: AlphaEdit = AlphaEdit::Absent;

    fn as_rgba(self) -> Rgba {
        self.to_rgba(Alpha::OPAQUE)
    }

    fn from_rgba(rgba: Rgba) -> Self {
        rgba.rgb()
    }
}

impl PickedColour for Rgba {
    const ALPHA: AlphaEdit = AlphaEdit::Slider;

    fn as_rgba(self) -> Rgba {
        self
    }

    fn from_rgba(rgba: Rgba) -> Self {
        rgba
    }
}

/// A colour picked by its channels: the RGB, and the alpha when the pick
/// names one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pick {
    rgb: Rgb,
    /// The alpha an eight-digit spelling carries. A pick without one keeps
    /// the alpha of the colour it replaces.
    alpha: Option<Alpha>,
}

impl Pick {
    /// A pick of the RGB channels alone, as a swatch makes.
    const fn keeping_alpha(rgb: Rgb) -> Self {
        Self { rgb, alpha: None }
    }
}

/// Parse a hex colour, `"#rrggbb"` as the native widget writes it or
/// `"#rrggbbaa"` with its alpha. Any other spelling yields `None`.
fn parse_hex_color(s: &str) -> Option<Pick> {
    let hex = s
        .strip_prefix('#')
        .filter(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let bits = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(Pick::keeping_alpha(Rgb::from_bits(bits))),
        8 => {
            let [r, g, b, alpha] = bits.to_be_bytes();
            Some(Pick {
                rgb: Rgb::from_channels(r, g, b),
                alpha: Some(Alpha::new(alpha)),
            })
        }
        _ => None,
    }
}

/// The four channels of `picked` over `current`. The alpha is the one the
/// pick names, else the alpha of `current`, else opaque.
fn repaint_rgba(current: Option<Rgba>, picked: Pick) -> Rgba {
    let [.., kept] = current.unwrap_or(UNSET).channels();
    picked.rgb.to_rgba(picked.alpha.unwrap_or(Alpha::new(kept)))
}

/// The colour `picked` in the packing of `current`. The alpha is the one the
/// pick names, else the alpha of `current`, else opaque. A packing with no
/// alpha drops it.
#[cfg(test)]
fn repaint<C: PickedColour>(current: Option<C>, picked: Pick) -> C {
    C::from_rgba(repaint_rgba(current.map(C::as_rgba), picked))
}

/// A composed colour control. `on_pick` receives `Some(colour)` for a chosen
/// colour or `None` for the reset entry. A packing with an alpha also gets an
/// opacity slider (see the module header).
///
/// The packing stops at this wrapper. The popover below reads and writes
/// four channels, so the main instance carries one copy of its view.
#[component]
pub fn ColorPopover<C: PickedColour>(
    #[prop(into)] title: String,
    icon: icondata::Icon,
    current: RwSignal<Option<C>>,
    default: Signal<C>,
    reset_label: &'static str,
    on_pick: Callback<Option<C>>,
) -> impl IntoView {
    color_popover(
        title,
        icon,
        Signal::derive(move || current.get().map(C::as_rgba)),
        Signal::derive(move || default.get().as_rgba()),
        C::ALPHA,
        reset_label,
        Callback::new(move |picked: Option<Rgba>| on_pick.run(picked.map(C::from_rgba))),
    )
}

/// The popover of [`ColorPopover`], over the four channels of an [`Rgba`].
/// A write leaves through `on_pick`, which puts it back in the caller's
/// packing.
fn color_popover(
    title: String,
    icon: icondata::Icon,
    current: Signal<Option<Rgba>>,
    default: Signal<Rgba>,
    alpha_edit: AlphaEdit,
    reset_label: &'static str,
    on_pick: Callback<Option<Rgba>>,
) -> AnyView {
    let open = RwSignal::new(crate::Openness::Closed);
    let (rgb_drag, alpha_drag) = (Debouncer::new(), Debouncer::new());
    // The wrapper node, so a global pointerdown can tell clicks inside the popover
    // apart from clicks elsewhere (another cell, the canvas, a sibling control).
    let root_ref = NodeRef::<html::Span>::new();
    // Close on any pointer-down outside the control: selecting another cell hits
    // the canvas, which is outside this node, so the popover dismisses.
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if open.get_untracked() == crate::Openness::Closed {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(crate::Openness::Closed);
        }
    });
    on_cleanup(move || handle.remove());
    view! {
        <span class="ambre-color" node_ref=root_ref>
            <button
                aria-label=title
                aria-haspopup="true"
                aria-expanded=move || open.get().aria()
                // Keep the canvas' focus: a button steals it on mousedown, which
                // would clear the active selection before on:click can apply.
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = o.toggled())
            >
                <Icon icon=icon />
                // Thin strip tinted to the current colour (themed default when unset).
                <span
                    class="swatch-strip"
                    style:background-color=move || {
                        current.get().unwrap_or(default.get()).to_string()
                    }
                ></span>
            </button>
            <Show when=move || open.get() == crate::Openness::Open fallback=|| ()>
                <div class="ambre-color-popover">
                    // Reset entry: "Automatic" (text) / "No fill (transparent)" (fill).
                    <button
                        class="reset"
                        on:mousedown=move |ev| ev.prevent_default()
                        on:click=move |_| {
                            on_pick.run(None);
                            open.set(crate::Openness::Closed);
                        }
                    >
                        {reset_label}
                    </button>
                    // Swatch palette. Each swatch is RGB, and keeps the current
                    // alpha (fully opaque when the value is unset).
                    <div class="swatches">
                        {PALETTE
                            .iter()
                            .map(|&rgb| {
                                view! {
                                    <button
                                        aria-label=rgb.to_string()
                                        style:background-color=rgb.to_string()
                                        on:mousedown=move |ev| ev.prevent_default()
                                        on:click=move |_| {
                                            on_pick.run(Some(repaint_rgba(current.get_untracked(), Pick::keeping_alpha(rgb))));
                                            open.set(crate::Openness::Closed);
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
                            prop:value=move || {
                                current.get().unwrap_or(default.get()).rgb().to_string()
                            }
                            on:input=move |ev| {
                                if let Some(pick) = parse_hex_color(&event_target_value(&ev)) {
                                    let picked = repaint_rgba(current.get_untracked(), pick);
                                    rgb_drag.call(DRAG_DEBOUNCE, move || on_pick.run(Some(picked)));
                                }
                            }
                        />
                        "Custom..."
                    </label>
                    // Opacity slider, for a packing with an alpha: drives the
                    // alpha channel while preserving the current RGB channels.
                    {(alpha_edit == AlphaEdit::Slider)
                        .then(|| {
                            view! {
                                <label class="opacity">
                                    <input
                                        type="range"
                                        min="0"
                                        max="255"
                                        prop:value=move || {
                                            let [.., alpha] = current.get().unwrap_or(UNSET).channels();
                                            alpha.to_string()
                                        }
                                        on:input=move |ev| {
                                            let alpha = event_target_value(&ev)
                                                .parse::<u8>()
                                                .unwrap_or(u8::MAX);
                                            let base = current
                                                .get_untracked()
                                                .unwrap_or(default.get_untracked());
                                            let [r, g, b, _] = base.channels();
                                            let picked = Rgba::from_channels(r, g, b, alpha);
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
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_reads_the_native_widget_value() {
        let red = Pick::keeping_alpha(Rgb::from_bits(0xff_0000));
        assert_eq!(parse_hex_color("#ff0000"), Some(red));
        let black = Pick::keeping_alpha(Rgb::from_bits(0x00_0000));
        assert_eq!(parse_hex_color("#000000"), Some(black));
    }

    #[test]
    fn parse_hex_reads_an_eight_digit_spelling_with_its_alpha() {
        let pick = Pick {
            rgb: Rgb::from_bits(0x80_ff00),
            alpha: Some(Alpha::new(0x40)),
        };
        assert_eq!(parse_hex_color("#80ff0040"), Some(pick));
    }

    #[test]
    fn parse_hex_rejects_garbage() {
        assert_eq!(parse_hex_color("ff0000"), None); // no leading '#'
        assert_eq!(parse_hex_color("#zzz"), None);
        assert_eq!(parse_hex_color("#80ff000"), None);
        assert_eq!(parse_hex_color("#80ff00000"), None);
        assert_eq!(parse_hex_color("#+fffff"), None);
        assert_eq!(parse_hex_color("#+fffffff"), None);
    }

    #[test]
    fn a_repaint_keeps_the_alpha_of_the_current_colour() {
        let rgb = Pick::keeping_alpha(Rgb::from_bits(0x00_ff00));
        let translucent = Rgba::from_bits(0xabcd_ef80);
        assert_eq!(
            repaint(Some(translucent), rgb),
            Rgba::from_bits(0x00ff_0080)
        );
        assert_eq!(repaint(None::<Rgba>, rgb), Rgba::from_bits(0x00ff_00ff));
        assert_eq!(
            repaint(Some(Rgb::from_bits(0x12_3456)), rgb),
            Rgb::from_bits(0x00_ff00)
        );
    }

    /// An eight-digit pick puts its alpha in the colour's alpha when the
    /// packing holds one, and an RGB packing drops it.
    #[test]
    fn a_picked_alpha_reaches_the_colour_that_holds_one() {
        let pick = parse_hex_color("#00ff0040").unwrap();
        let translucent = Rgba::from_bits(0xabcd_ef80);
        assert_eq!(
            repaint(Some(translucent), pick),
            Rgba::from_bits(0x00ff_0040)
        );
        assert_eq!(repaint(None::<Rgba>, pick), Rgba::from_bits(0x00ff_0040));
        assert_eq!(
            repaint(Some(Rgb::from_bits(0x12_3456)), pick),
            Rgb::from_bits(0x00_ff00)
        );
    }

    #[test]
    fn an_rgb_reads_as_opaque_and_drops_a_picked_alpha() {
        let rgb = Rgb::from_bits(0x12_3456);
        assert_eq!(rgb.as_rgba(), Rgba::from_bits(0x1234_56ff));
        assert_eq!(Rgb::from_rgba(Rgba::from_bits(0x1234_5680)), rgb);
    }
}
