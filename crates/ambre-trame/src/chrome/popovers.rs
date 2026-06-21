// ambre-trame/src/chrome/popovers.rs
// ============================================================================
// The composed popover controls of the formatting chrome: the borders popover
// (pen + preset grid), the colour popover (palette + reset + custom), and the
// shared filter popover (by-values / by-condition). The borders and colour
// popovers are `pub(crate)` so the `FormatBar` toolbar can mount them.
// ============================================================================

use ambre_bus::{
    BorderEdit, BorderPreset, BorderStyle, CellFormat, FilterKind, FilterOp, HAlign, StylePatch,
    VAlign,
};
use ambre_chrome::commands::CommandId;
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

use super::color::{color_hex, parse_hex_color};
use super::commands::CLEAR_FILTER;
use super::style::StyleState;

/// The border presets offered by the popover, each with its Bootstrap icon and
/// tooltip. Clicking one applies the popover's current pen (line style + colour)
/// to the named edges; `None` clears every edge.
const BORDER_BUTTONS: &[(BorderPreset, icondata::Icon, &str)] = &[
    (BorderPreset::All, icondata::BsBorderAll, "All borders"),
    (
        BorderPreset::Inner,
        icondata::BsBorderInner,
        "Inner borders",
    ),
    (
        BorderPreset::InnerHorizontal,
        icondata::BsBorderMiddle,
        "Inner horizontal",
    ),
    (
        BorderPreset::InnerVertical,
        icondata::BsBorderCenter,
        "Inner vertical",
    ),
    (BorderPreset::Outline, icondata::BsBorderOuter, "Outline"),
    (BorderPreset::Top, icondata::BsBorderTop, "Top border"),
    (
        BorderPreset::Bottom,
        icondata::BsBorderBottom,
        "Bottom border",
    ),
    (BorderPreset::Left, icondata::BsBorderLeft, "Left border"),
    (BorderPreset::Right, icondata::BsBorderRight, "Right border"),
    (BorderPreset::None, icondata::BsBorder, "Clear borders"),
];

/// The border stroke styles offered by the line-style picker (line pattern +
/// weight), each paired with its label and the CSS modifier class that renders a
/// visual preview of the stroke (see `.line-preview.*` in `styles.css`).
const BORDER_STYLES: &[(BorderStyle, &str, &str)] = &[
    (BorderStyle::Solid, "Solid", "solid"),
    (BorderStyle::Medium, "Medium", "medium"),
    (BorderStyle::Thick, "Thick", "thick"),
    (BorderStyle::Dashed, "Dashed", "dashed"),
    (BorderStyle::Dotted, "Dotted", "dotted"),
    (BorderStyle::Double, "Double", "double"),
];

/// The composed borders control (`-ui.md` sec.5bis): a single toolbar button
/// that toggles a popover holding the "pen" (stroke style + colour) and a grid
/// of preset icon buttons. A preset click applies the current pen to its edges
/// via [`StylePatch::border`] (a [`BorderEdit`]); the per-edge model lets a cell
/// carry a different style/colour on each side, accumulated across clicks.
#[component]
pub(crate) fn BordersPopover(style: StyleState, on_style: Callback<StylePatch>) -> impl IntoView {
    let open = RwSignal::new(false);
    // The nested line-style picker's open state (a sub-popover inside this one).
    let line_open = RwSignal::new(false);
    // The wrapper node, so a global pointer-down can tell clicks inside the
    // popover apart from clicks elsewhere (another cell, the canvas, a sibling).
    let root_ref = NodeRef::<html::Span>::new();
    // Close on any pointer-down outside the control: selecting a cell hits the
    // canvas, which is outside this node, so the popover dismisses.
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
    // Emit a border edit for `preset` using the popover's current pen.
    let apply = move |preset: BorderPreset| {
        on_style.run(StylePatch {
            border: Some(BorderEdit {
                preset,
                style: style.border_line.get_untracked(),
                color: style.border_color.get_untracked(),
            }),
            ..Default::default()
        });
    };
    view! {
        <span class="ambre-borders" node_ref=root_ref>
            <button
                aria-label="Borders"
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=icondata::BsBorderAll />
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-borders-popover">
                    // Preset grid: each applies the pen to its edges.
                    <div class="grid">
                        {BORDER_BUTTONS
                            .iter()
                            .map(|&(preset, icon, label)| {
                                view! {
                                    <button
                                        aria-label=label
                                        on:click=move |_| apply(preset)
                                    >
                                        <Icon icon=icon />
                                    </button>
                                }
                            })
                            .collect_view()}
                    </div>
                    // Pen: stroke style + colour applied by the preset buttons,
                    // left-aligned beneath the preset grid.
                    <div class="row">
                        // Line style: an icon button toggling a sub-popover that
                        // renders each stroke with a check on the active one.
                        <span class="ambre-line-style">
                            <button
                                class="line-style-trigger"
                                aria-label="Border style"
                                aria-haspopup="true"
                                aria-expanded=move || line_open.get().then_some("true")
                                on:click=move |_| line_open.update(|o| *o = !*o)
                            >
                                <Icon icon=icondata::BsBorderStyle />
                            </button>
                            <Show when=move || line_open.get() fallback=|| ()>
                                <div class="ambre-line-style-popover">
                                    {BORDER_STYLES
                                        .iter()
                                        .map(|&(bs, label, css)| {
                                            view! {
                                                <button
                                                    class="line-style-item"
                                                    aria-label=label
                                                    on:click=move |_| {
                                                        style.border_line.set(bs);
                                                        line_open.set(false);
                                                    }
                                                >
                                                    <span class="check">
                                                        <Show
                                                            when=move || style.border_line.get() == bs
                                                            fallback=|| ()
                                                        >
                                                            <Icon icon=icondata::BsCheck />
                                                        </Show>
                                                    </span>
                                                    <span class=format!("line-preview {css}")></span>
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            </Show>
                        </span>
                        // Border colour: the same composed picker as Text colour
                        // (palette + "Automatic" reset + custom), driving the pen.
                        <ColorPopover
                            title="Border colour"
                            icon=icondata::MdiBorderColor
                            current=style.border_color
                            default=Signal::derive(|| 0x606060)
                            reset_label="Automatic"
                            on_pick=Callback::new(move |c| style.border_color.set(c))
                        />
                    </div>
                </div>
            </Show>
        </span>
    }
}

/// The font families offered by the font popover: the abstract family name shown
/// in the toolbar (the renderer falls back to its default for any it lacks),
/// paired with the CSS `font-family` used to render that entry in its own
/// typeface (a live preview). "Default" maps to no explicit font (`None`).
const FONT_FAMILIES: &[(&str, &str)] = &[
    ("Default", "inherit"),
    ("Sans", "'Roboto', system-ui, sans-serif"),
    ("Serif", "Georgia, 'Times New Roman', serif"),
    ("Monospace", "'Roboto Mono', ui-monospace, monospace"),
];

/// The composed font control: a single toolbar trigger (current family name +
/// caret) toggling a context-menu popover that lists the families, each rendered
/// in its own typeface so the user previews the font before picking it. A
/// `<select>` cannot reliably style its `<option>`s per-font across browsers,
/// hence this custom popover (mirrors [`NumberFormatPopover`]). Picking emits a
/// [`StylePatch`] through `on_style` ("Default" -> `Some(None)`, a reset).
#[component]
pub(crate) fn FontPopover(style: StyleState, on_style: Callback<StylePatch>) -> impl IntoView {
    let open = RwSignal::new(false);
    // The wrapper node, so a global pointer-down can tell clicks inside the menu
    // apart from clicks elsewhere (another cell, the canvas, a sibling control).
    let root_ref = NodeRef::<html::Span>::new();
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
    // The active family label ("Default" when the cell carries no explicit font).
    let current_label = move || style.font.get().unwrap_or_else(|| "Default".to_owned());
    // CSS font-family for the active family, so the trigger previews it too.
    let current_css = move || {
        let label = current_label();
        FONT_FAMILIES
            .iter()
            .find(|(name, _)| *name == label)
            .map_or("inherit", |(_, css)| css)
    };
    view! {
        <span class="ambre-font" node_ref=root_ref>
            <button
                class="font-trigger"
                aria-label="Font"
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                style:font-family=current_css
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <span class="label">{current_label}</span>
                <Icon icon=icondata::BiDownArrowSolid />
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-font-popover">
                    {FONT_FAMILIES
                        .iter()
                        .map(|&(name, css)| {
                            view! {
                                <button
                                    class="font-item"
                                    style:font-family=css
                                    on:click=move |_| {
                                        // "Default" clears the explicit font (None).
                                        let font = (name != "Default").then(|| name.to_owned());
                                        on_style.run(StylePatch {
                                            font: Some(font),
                                            ..Default::default()
                                        });
                                        open.set(false);
                                    }
                                >
                                    <span class="check">
                                        <Show when=move || current_label() == name fallback=|| ()>
                                            <Icon icon=icondata::BsCheck />
                                        </Show>
                                    </span>
                                    <span class="label">{name}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            </Show>
        </span>
    }
}

/// The number-format families offered by the format popover, paired with their
/// labels. Clicking an item applies its [`CellFormat`] through [`StylePatch`].
const NUMBER_FORMATS: &[(CellFormat, &str)] = &[
    (CellFormat::General, "General"),
    (CellFormat::Number, "Number"),
    (CellFormat::Currency, "Currency"),
    (CellFormat::Percent, "Percent"),
    (CellFormat::Scientific, "Scientific"),
    (CellFormat::Fraction, "Fraction"),
    (CellFormat::Date, "Date"),
    (CellFormat::Time, "Time"),
    (CellFormat::Text, "Text"),
];

/// The composed number-format control (`-ui.md` sec.5bis): a single toolbar
/// button (the "123" icon) toggling a context-menu-style popover that lists the
/// [`CellFormat`] families, with a check next to the active one. Picking a format
/// emits a [`StylePatch`] through `on_style` and closes the menu.
#[component]
pub(crate) fn NumberFormatPopover(
    style: StyleState,
    on_style: Callback<StylePatch>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    // The wrapper node, so a global pointer-down can tell clicks inside the menu
    // apart from clicks elsewhere (another cell, the canvas, a sibling control).
    let root_ref = NodeRef::<html::Span>::new();
    // Close on any pointer-down outside the control: selecting a cell hits the
    // canvas, which is outside this node, so the menu dismisses.
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
        <span class="ambre-number-format" node_ref=root_ref>
            <button
                aria-label="Number format"
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=icondata::Bs123 />
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-number-format-popover">
                    {NUMBER_FORMATS
                        .iter()
                        .map(|(fmt, label)| {
                            let fmt = fmt.clone();
                            let fmt_for_check = fmt.clone();
                            view! {
                                <button
                                    class="number-format-item"
                                    on:click=move |_| {
                                        on_style.run(StylePatch {
                                            format: Some(fmt.clone()),
                                            ..Default::default()
                                        });
                                        open.set(false);
                                    }
                                >
                                    <span class="check">
                                        <Show
                                            when=move || style.format.get() == fmt_for_check
                                            fallback=|| ()
                                        >
                                            <Icon icon=icondata::BsCheck />
                                        </Show>
                                    </span>
                                    <span class="label">{label.to_string()}</span>
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            </Show>
        </span>
    }
}

/// The horizontal alignments offered by the popover, each with its icon and
/// tooltip. The trigger mirrors the active entry's icon.
const H_ALIGNS: &[(HAlign, icondata::Icon, &str)] = &[
    (HAlign::Left, icondata::BiAlignLeftRegular, "Align left"),
    (
        HAlign::Center,
        icondata::BiAlignMiddleRegular,
        "Align center",
    ),
    (HAlign::Right, icondata::BiAlignRightRegular, "Align right"),
];

/// The vertical alignments offered by the popover, each with its icon and
/// tooltip. The trigger mirrors the active entry's icon.
const V_ALIGNS: &[(VAlign, icondata::Icon, &str)] = &[
    (
        VAlign::Top,
        icondata::AiVerticalAlignTopOutlined,
        "Align top",
    ),
    (
        VAlign::Middle,
        icondata::AiVerticalAlignMiddleOutlined,
        "Align middle",
    ),
    (
        VAlign::Bottom,
        icondata::AiVerticalAlignBottomOutlined,
        "Align bottom",
    ),
];

/// The composed horizontal-alignment control: a single toolbar button showing the
/// active alignment's icon, toggling a context-menu popover that lists the three
/// alignments with a check on the active one. Picking emits a [`StylePatch`].
#[component]
pub(crate) fn HAlignPopover(style: StyleState, on_style: Callback<StylePatch>) -> impl IntoView {
    let open = RwSignal::new(false);
    let root_ref = NodeRef::<html::Span>::new();
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
    // Resolve the effective horizontal alignment: `Auto` follows the renderer's
    // heuristic (numbers right, text left), every explicit value is itself. This
    // drives both the trigger icon and the menu's check so an `Auto` cell shows
    // its de-facto alignment.
    let effective = move || match style.h_align.get() {
        HAlign::Auto if style.numeric.get() => HAlign::Right,
        HAlign::Auto => HAlign::Left,
        other => other,
    };
    // The effective alignment's icon, mirrored on the trigger.
    let current_icon = move || {
        let cur = effective();
        H_ALIGNS
            .iter()
            .find(|(a, ..)| *a == cur)
            .map_or(icondata::BiAlignLeftRegular, |(_, icon, _)| *icon)
    };
    view! {
        <span class="ambre-align" node_ref=root_ref>
            <button
                aria-label="Horizontal alignment"
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=Signal::derive(current_icon) />
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-align-popover">
                    {H_ALIGNS
                        .iter()
                        .map(|&(align, icon, label)| {
                            view! {
                                <button
                                    class="align-item"
                                    aria-label=label
                                    on:click=move |_| {
                                        on_style.run(StylePatch {
                                            h_align: Some(align),
                                            ..Default::default()
                                        });
                                        open.set(false);
                                    }
                                >
                                    <span class="check">
                                        // Check the effective alignment: `Auto`
                                        // resolves to left (text) or right (number)
                                        // exactly as the cell renders.
                                        <Show
                                            when=move || effective() == align
                                            fallback=|| ()
                                        >
                                            <Icon icon=icondata::BsCheck />
                                        </Show>
                                    </span>
                                    <Icon icon=icon />
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            </Show>
        </span>
    }
}

/// The composed vertical-alignment control: a single toolbar button showing the
/// active alignment's icon, toggling a context-menu popover that lists the three
/// alignments with a check on the active one. Picking emits a [`StylePatch`].
#[component]
pub(crate) fn VAlignPopover(style: StyleState, on_style: Callback<StylePatch>) -> impl IntoView {
    let open = RwSignal::new(false);
    let root_ref = NodeRef::<html::Span>::new();
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
    // The active alignment's icon, mirrored on the trigger.
    let current_icon = move || {
        let cur = style.v_align.get();
        V_ALIGNS
            .iter()
            .find(|(a, ..)| *a == cur)
            .map_or(icondata::AiVerticalAlignBottomOutlined, |(_, icon, _)| {
                *icon
            })
    };
    view! {
        <span class="ambre-align" node_ref=root_ref>
            <button
                aria-label="Vertical alignment"
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=Signal::derive(current_icon) />
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-align-popover">
                    {V_ALIGNS
                        .iter()
                        .map(|&(align, icon, label)| {
                            view! {
                                <button
                                    class="align-item"
                                    aria-label=label
                                    on:click=move |_| {
                                        on_style.run(StylePatch {
                                            v_align: Some(align),
                                            ..Default::default()
                                        });
                                        open.set(false);
                                    }
                                >
                                    <span class="check">
                                        <Show
                                            when=move || style.v_align.get() == align
                                            fallback=|| ()
                                        >
                                            <Icon icon=icondata::BsCheck />
                                        </Show>
                                    </span>
                                    <Icon icon=icon />
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            </Show>
        </span>
    }
}

/// Curated swatch palette for the colour picker popover (packed `0xRRGGBB`).
/// A small office-style set: greys, then a saturated row and a muted row.
const PALETTE: &[u32] = &[
    0x000000, 0x404040, 0x808080, 0xb0b0b0, 0xd9d9d9, 0xffffff, 0xc00000, 0xff0000, 0xffc000,
    0xffff00, 0x92d050, 0x00b050, 0x00b0f0, 0x0070c0, 0x002060, 0x7030a0,
];

/// A composed colour control (text or fill): a single toolbar button toggling a
/// popover that combines a reset entry ("Automatic" / "No fill"), a swatch
/// palette, and an embedded native `<input type="color">` for arbitrary colours.
/// `on_pick` receives `Some(rgb)` for a chosen colour or `None` for the reset,
/// which the caller maps onto `StylePatch { fg/bg: Some(_) }` (sec.5bis).
#[component]
pub(crate) fn ColorPopover(
    title: &'static str,
    icon: icondata::Icon,
    current: RwSignal<Option<u32>>,
    default: Signal<u32>,
    reset_label: &'static str,
    on_pick: Callback<Option<u32>>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    // The wrapper node, so a global mousedown can tell clicks inside the popover
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
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=icon />
                // Thin strip tinted to the current colour (themed default when unset).
                <span
                    class="swatch-strip"
                    style:background-color=move || color_hex(current.get(), default.get())
                ></span>
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-color-popover">
                    // Reset entry: "Automatic" (text) / "No fill (transparent)" (fill).
                    <button
                        class="reset"
                        on:click=move |_| {
                            on_pick.run(None);
                            open.set(false);
                        }
                    >
                        {reset_label}
                    </button>
                    // Swatch palette.
                    <div class="swatches">
                        {PALETTE
                            .iter()
                            .map(|&rgb| {
                                view! {
                                    <button
                                        aria-label=format!("#{:06x}", rgb)
                                        style:background-color=format!("#{:06x}", rgb)
                                        on:click=move |_| {
                                            on_pick.run(Some(rgb));
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
                                on_pick.run(parse_hex_color(&event_target_value(&ev)));
                            }
                        />
                        "Custom..."
                    </label>
                </div>
            </Show>
        </span>
    }
}

/// Open state of the shared filter popover: which grid column it targets and the
/// viewport (client-space) anchor point under which it is drawn. `None` means
/// closed. The launcher derives the anchor from the active cell's published
/// screen position (the active cell is in the filtered column).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterPopoverState {
    /// Grid column (0-based) the filter targets.
    pub col: u32,
    /// Client-space anchor (CSS px): the popover's top-left.
    pub x: f64,
    pub y: f64,
}

/// The condition operators offered by the popover, paired with their labels.
/// The `<select>` index maps back to the [`FilterOp`] on apply.
const FILTER_OPS: &[(FilterOp, &str)] = &[
    (FilterOp::Eq, "equals"),
    (FilterOp::Ne, "not equals"),
    (FilterOp::Gt, "greater than"),
    (FilterOp::Lt, "less than"),
    (FilterOp::Gte, "greater or equal"),
    (FilterOp::Lte, "less or equal"),
    (FilterOp::Contains, "contains"),
    (FilterOp::NotContains, "does not contain"),
    (FilterOp::StartsWith, "starts with"),
    (FilterOp::IsEmpty, "is empty"),
    (FilterOp::IsNotEmpty, "is not empty"),
];

/// A1-style column label for a 0-based grid column index (0 -> "A", 26 -> "AA").
fn col_label(mut col: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// The shared filter popover: anchored under the active column header, it offers
/// the "by values" (checkbox list of the column's distinct visible values) and a
/// minimal "by condition" (operator + value) modes (`-ui.md` sec.5.D). Applying
/// fires `on_apply` with the chosen [`FilterKind`]; "Clear filter" routes through
/// `on_command` (the shared command path). `values` is fed by the launcher from
/// [`ambre_bus::TrameEvent::ColumnValues`].
#[component]
pub fn FilterPopover(
    state: RwSignal<Option<FilterPopoverState>>,
    values: RwSignal<Vec<String>>,
    on_apply: Callback<FilterKind>,
    on_command: Callback<CommandId>,
) -> impl IntoView {
    // true = "by values", false = "by condition".
    let by_values = RwSignal::new(true);
    // The set of currently-checked (visible) values in the "by values" mode.
    let checked = RwSignal::new(std::collections::BTreeSet::<String>::new());
    // Whenever a fresh value list arrives (popover (re)opened on a column), check
    // every value by default -- the unfiltered state.
    Effect::new(move |_| {
        let vs = values.get();
        checked.set(vs.into_iter().collect());
    });
    // Condition-mode operator + operand.
    let op_idx = RwSignal::new(0usize);
    let cond_value = RwSignal::new(String::new());

    let close = move || state.set(None);

    let apply = move |_| {
        let kind = if by_values.get_untracked() {
            FilterKind::Values(checked.get_untracked().into_iter().collect())
        } else {
            let (op, _) = FILTER_OPS[op_idx.get_untracked().min(FILTER_OPS.len() - 1)];
            FilterKind::Condition {
                op,
                value: cond_value.get_untracked(),
            }
        };
        on_apply.run(kind);
        state.set(None);
    };

    move || {
        state.get().map(|st| {
            let style = format!(
                "position: fixed; left: {}px; top: {}px; z-index: 50;",
                st.x, st.y
            );
            view! {
                <div class="ambre-filter-popover" style=style>
                    <div class="ambre-filter-head">
                        {format!("Filter column {}", col_label(st.col))}
                    </div>
                    <div class="ambre-filter-modes">
                        <button
                            class="mode"
                            aria-pressed=move || by_values.get().then_some("true")
                            on:click=move |_| by_values.set(true)
                        >
                            "By values"
                        </button>
                        <button
                            class="mode"
                            aria-pressed=move || (!by_values.get()).then_some("true")
                            on:click=move |_| by_values.set(false)
                        >
                            "By condition"
                        </button>
                    </div>

                    <Show
                        when=move || by_values.get()
                        fallback=move || view! {
                            <div class="ambre-filter-cond">
                                <select on:change=move |ev| {
                                    op_idx.set(event_target_value(&ev).parse().unwrap_or(0));
                                }>
                                    {FILTER_OPS
                                        .iter()
                                        .enumerate()
                                        .map(|(i, (_, label))| {
                                            view! { <option value=i.to_string()>{label.to_string()}</option> }
                                        })
                                        .collect_view()}
                                </select>
                                <input
                                    type="text"
                                    placeholder="value"
                                    on:input=move |ev| cond_value.set(event_target_value(&ev))
                                />
                            </div>
                        }
                    >
                        <div class="ambre-filter-values">
                            <For
                                each=move || values.get()
                                key=|v| v.clone()
                                children=move |v: String| {
                                    let v_for_check = v.clone();
                                    let v_for_toggle = v.clone();
                                    let is_on = move || checked.get().contains(&v_for_check);
                                    view! {
                                        <label class="ambre-filter-value">
                                            <input
                                                type="checkbox"
                                                prop:checked=is_on
                                                on:change=move |ev| {
                                                    let on = event_target_checked(&ev);
                                                    checked.update(|set| {
                                                        if on {
                                                            set.insert(v_for_toggle.clone());
                                                        } else {
                                                            set.remove(&v_for_toggle);
                                                        }
                                                    });
                                                }
                                            />
                                            <span>{v}</span>
                                        </label>
                                    }
                                }
                            />
                        </div>
                    </Show>

                    <div class="ambre-filter-actions">
                        <button class="primary" on:click=apply>"Apply"</button>
                        <button on:click=move |_| {
                            on_command.run(CLEAR_FILTER);
                            close();
                        }>"Clear filter"</button>
                        <button on:click=move |_| close()>"Cancel"</button>
                    </div>
                </div>
            }
        })
    }
}
