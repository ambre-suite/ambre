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
/// weight). Each is paired with its label. `BorderStyle::css_word` names the
/// CSS modifier class that renders a visual preview of the stroke (see
/// `.line-preview.*` in `styles.css`).
const BORDER_STYLES: &[(BorderStyle, &str)] = &[
    (BorderStyle::Solid, "Solid"),
    (BorderStyle::Medium, "Medium"),
    (BorderStyle::Thick, "Thick"),
    (BorderStyle::Dashed, "Dashed"),
    (BorderStyle::Dotted, "Dotted"),
    (BorderStyle::Double, "Double"),
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
            // The icon button names the trigger in a tooltip, and keeps the
            // grid's focus on mousedown, so the active cell survives the press.
            <ambre_chrome::IconButton
                icon=icondata::BsBorderAll
                label="Borders"
                expanded=Some(Signal::from(open))
                on_press=Callback::new(move |()| open.update(|o| *o = o.toggled()))
            />
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
                                        .map(|&(bs, label)| {
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
                                                    <span class=format!(
                                                        "line-preview {}",
                                                        bs.css_word(),
                                                    )></span>
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
pub(crate) const NUMBER_FORMATS: &[(CellFormat, &str)] = &[
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

/// The horizontal alignments offered by the popover, each with its icon and
/// tooltip. The trigger mirrors the active entry's icon.
pub(crate) const H_ALIGNS: &[(HAlign, icondata::Icon, &str)] = &[
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
pub(crate) const V_ALIGNS: &[(VAlign, icondata::Icon, &str)] = &[
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

// The colour popover itself now lives in `ambre-chrome` (`ambre_chrome::ColorPopover`),
// re-exported through `super` so the call sites here and in `bars.rs` are unchanged.
// Trame uses it in RGB mode (no `alpha` prop); Toile's inspector uses the same
// component with `alpha=true` (`toile-styles`).
