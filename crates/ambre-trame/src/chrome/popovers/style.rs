// ambre-trame/src/chrome/popovers/style.rs
// Formatting popovers (borders, font, number format, alignment, colour).

use super::*;
use ambre_core::Rgb;

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
/// weight). Each is paired with its label and the CSS modifier class that
/// renders a visual preview of the stroke (see `.line-preview.*` in
/// `styles.css`).
const BORDER_STYLES: &[(BorderStyle, &str, &str)] = &[
    (BorderStyle::Solid, "Solid", "solid"),
    (BorderStyle::Medium, "Medium", "medium"),
    (BorderStyle::Thick, "Thick", "thick"),
    (BorderStyle::Dashed, "Dashed", "dashed"),
    (BorderStyle::Dotted, "Dotted", "dotted"),
    (BorderStyle::Double, "Double", "double"),
];

/// The composed borders control (`trame-cell-styles`) is a single toolbar button
/// that toggles a popover. The popover holds the "pen" (stroke style + colour)
/// and a grid of preset icon buttons. A preset click applies the current pen to
/// its edges via [`StylePatch::border`] (a [`BorderEdit`]). The per-edge model
/// lets a cell carry a different style/colour on each side, accumulated across
/// clicks.
#[component]
pub(crate) fn BordersPopover(style: StyleState, on_style: Callback<StylePatch>) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    // The nested line-style picker's open state (a sub-popover inside this one).
    let line_open = RwSignal::new(ambre_chrome::Openness::Closed);
    // The wrapper node, so a global pointer-down can tell clicks inside the
    // popover apart from clicks elsewhere (another cell, the canvas, a sibling).
    let root_ref = NodeRef::<html::Span>::new();
    // Close on any pointer-down outside the control: selecting a cell hits the
    // canvas, which is outside this node, so the popover dismisses.
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if open.get_untracked() == ambre_chrome::Openness::Closed {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(ambre_chrome::Openness::Closed);
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
                aria-expanded=move || open.get().aria()
                // Keep the grid's focus: a button steals it on mousedown, which
                // would clear the active cell before on:click can apply the colour.
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = o.toggled())
            >
                <Icon icon=icondata::BsBorderAll />
            </button>
            <Show when=move || open.get() == ambre_chrome::Openness::Open fallback=|| ()>
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
                                aria-expanded=move || line_open.get().aria()
                                on:click=move |_| line_open.update(|o| *o = o.toggled())
                            >
                                <Icon icon=icondata::BsBorderStyle />
                            </button>
                            <Show when=move || line_open.get() == ambre_chrome::Openness::Open fallback=|| ()>
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
                                                        line_open.set(ambre_chrome::Openness::Closed);
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
                            default=Signal::derive(|| Rgb::from_bits(0x60_6060))
                            reset_label="Automatic"
                            on_pick=Callback::new(move |c| style.border_color.set(c))
                        />
                    </div>
                </div>
            </Show>
        </span>
    }
}

/// The composed font control: a thin adapter over the shared
/// [`ambre_chrome::FontPicker`] (the control now lives in `ambre-chrome` so Trame
/// cells and Toile elements share it). It maps the picker's family pick onto a
/// [`StylePatch`] (`None` = the "Default" reset).
#[component]
pub(crate) fn FontPopover(style: StyleState, on_style: Callback<StylePatch>) -> impl IntoView {
    let on_pick = Callback::new(move |family: Option<ambre_core::FontFamily>| {
        on_style.run(StylePatch {
            font: Some(family),
            ..Default::default()
        });
    });
    view! { <ambre_chrome::FontPicker current=style.font.into() on_pick=on_pick /> }
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

/// The composed number-format control (`trame-number-formats`) is a single
/// toolbar button, the "123" icon. It toggles a context-menu-style popover that
/// lists the [`CellFormat`] families, with a check next to the active one.
/// Picking a format emits a [`StylePatch`] through `on_style` and closes the
/// menu.
#[component]
pub(crate) fn NumberFormatPopover(
    style: StyleState,
    on_style: Callback<StylePatch>,
) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    // The wrapper node, so a global pointer-down can tell clicks inside the menu
    // apart from clicks elsewhere (another cell, the canvas, a sibling control).
    let root_ref = NodeRef::<html::Span>::new();
    // Close on any pointer-down outside the control: selecting a cell hits the
    // canvas, which is outside this node, so the menu dismisses.
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if open.get_untracked() == ambre_chrome::Openness::Closed {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(ambre_chrome::Openness::Closed);
        }
    });
    on_cleanup(move || handle.remove());
    view! {
        <span class="ambre-number-format" node_ref=root_ref>
            <button
                aria-label="Number format"
                aria-haspopup="true"
                aria-expanded=move || open.get().aria()
                // Keep the grid's focus: a button steals it on mousedown, which
                // would clear the active cell before on:click can apply the colour.
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = o.toggled())
            >
                <Icon icon=icondata::Bs123 />
            </button>
            <Show when=move || open.get() == ambre_chrome::Openness::Open fallback=|| ()>
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
                                        open.set(ambre_chrome::Openness::Closed);
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

/// The composed horizontal-alignment control is a single toolbar button showing
/// the active alignment's icon. It toggles a context-menu popover that lists the
/// three alignments, with a check on the active one. Picking emits a
/// [`StylePatch`].
#[component]
pub(crate) fn HAlignPopover(style: StyleState, on_style: Callback<StylePatch>) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    let root_ref = NodeRef::<html::Span>::new();
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if open.get_untracked() == ambre_chrome::Openness::Closed {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(ambre_chrome::Openness::Closed);
        }
    });
    on_cleanup(move || handle.remove());
    // Resolve the effective horizontal alignment: `Auto` follows the renderer's
    // heuristic (numbers right, text left), every explicit value is itself. This
    // drives both the trigger icon and the menu's check so an `Auto` cell shows
    // its de-facto alignment.
    let effective = move || match style.h_align.get() {
        HAlign::Auto if style.numeric.get() == crate::message::CellValueKind::Number => {
            HAlign::Right
        }
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
                aria-expanded=move || open.get().aria()
                // Keep the grid's focus: a button steals it on mousedown, which
                // would clear the active cell before on:click can apply the colour.
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = o.toggled())
            >
                <Icon icon=Signal::derive(current_icon) />
            </button>
            <Show when=move || open.get() == ambre_chrome::Openness::Open fallback=|| ()>
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
                                        open.set(ambre_chrome::Openness::Closed);
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

/// The composed vertical-alignment control is a single toolbar button showing
/// the active alignment's icon. It toggles a context-menu popover that lists the
/// three alignments, with a check on the active one. Picking emits a
/// [`StylePatch`].
#[component]
pub(crate) fn VAlignPopover(style: StyleState, on_style: Callback<StylePatch>) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    let root_ref = NodeRef::<html::Span>::new();
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if open.get_untracked() == ambre_chrome::Openness::Closed {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(ambre_chrome::Openness::Closed);
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
                aria-expanded=move || open.get().aria()
                // Keep the grid's focus: a button steals it on mousedown, which
                // would clear the active cell before on:click can apply the colour.
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| open.update(|o| *o = o.toggled())
            >
                <Icon icon=Signal::derive(current_icon) />
            </button>
            <Show when=move || open.get() == ambre_chrome::Openness::Open fallback=|| ()>
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
                                        open.set(ambre_chrome::Openness::Closed);
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

// The colour popover itself now lives in `ambre-chrome` (`ambre_chrome::ColorPopover`),
// re-exported through `super` so the call sites here and in `bars.rs` are unchanged.
// Trame uses it in RGB mode (no `alpha` prop); Toile's inspector uses the same
// component with `alpha=true` (`toile-styles`).
