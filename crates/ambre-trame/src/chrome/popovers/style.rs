// ambre-trame/src/chrome/popovers/style.rs
// Formatting popovers (borders, font, number format, alignment, colour).

use super::*;
use ambre_core::Rgb;

/// The border presets offered by the popover, each with its Bootstrap icon and
/// tooltip. Clicking one applies the popover's current pen (line style + colour)
/// to the named edges; `None` clears every edge.
const BORDER_BUTTONS: &[(BorderPreset, icondata::Icon, ambre_commands::TitleWords)] = &[
    (
        BorderPreset::All,
        icondata::BsBorderAll,
        ambre_commands::words!("All borders", "Toutes les bordures"),
    ),
    (
        BorderPreset::Inner,
        icondata::BsBorderInner,
        ambre_commands::words!("Inner borders", "Bordures int\u{e9}rieures"),
    ),
    (
        BorderPreset::InnerHorizontal,
        icondata::BsBorderMiddle,
        ambre_commands::words!("Inner horizontal", "Int\u{e9}rieures horizontales"),
    ),
    (
        BorderPreset::InnerVertical,
        icondata::BsBorderCenter,
        ambre_commands::words!("Inner vertical", "Int\u{e9}rieures verticales"),
    ),
    (
        BorderPreset::Outline,
        icondata::BsBorderOuter,
        ambre_commands::words!("Outline", "Contour"),
    ),
    (
        BorderPreset::Top,
        icondata::BsBorderTop,
        ambre_commands::words!("Top border", "Bordure sup\u{e9}rieure"),
    ),
    (
        BorderPreset::Bottom,
        icondata::BsBorderBottom,
        ambre_commands::words!("Bottom border", "Bordure inf\u{e9}rieure"),
    ),
    (
        BorderPreset::Left,
        icondata::BsBorderLeft,
        ambre_commands::words!("Left border", "Bordure gauche"),
    ),
    (
        BorderPreset::Right,
        icondata::BsBorderRight,
        ambre_commands::words!("Right border", "Bordure droite"),
    ),
    (
        BorderPreset::None,
        icondata::BsBorder,
        ambre_commands::words!("Clear borders", "Effacer les bordures"),
    ),
];

/// The border stroke styles offered by the line-style picker (line pattern +
/// weight). Each is paired with its label. `BorderStyle::css_word` names the
/// CSS modifier class that renders a visual preview of the stroke (see
/// `.line-preview.*` in `styles.css`).
const BORDER_STYLES: &[(BorderStyle, ambre_commands::TitleWords)] = &[
    (BorderStyle::Solid, ambre_commands::words!("Solid", "Plein")),
    (
        BorderStyle::Medium,
        ambre_commands::words!("Medium", "Moyen"),
    ),
    (
        BorderStyle::Thick,
        ambre_commands::words!("Thick", "\u{c9}pais"),
    ),
    (
        BorderStyle::Dashed,
        ambre_commands::words!("Dashed", "Tirets"),
    ),
    (
        BorderStyle::Dotted,
        ambre_commands::words!("Dotted", "Pointill\u{e9}s"),
    ),
    (
        BorderStyle::Double,
        ambre_commands::words!("Double", "Double"),
    ),
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
                label=ambre_commands::tr!("Borders", "Bordures")
                expanded=Some(Signal::from(open))
                on_press=Callback::new(move |()| open.update(|o| *o = o.toggled()))
            />
            <Show when=move || open.get() == ambre_chrome::Openness::Open fallback=|| ()>
                <div class="ambre-borders-popover">
                    // Preset grid: each applies the pen to its edges.
                    <div class="grid">
                        {BORDER_BUTTONS
                            .iter()
                            .map(|&(preset, icon, words)| {
                                let label = ambre_commands::shown(&words);
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
                                aria-label=ambre_commands::tr!("Border style", "Style de bordure")
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
                                        .map(|&(bs, words)| {
                                            let label = ambre_commands::shown(&words);
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
                            title=ambre_commands::tr!("Border colour", "Couleur de bordure")
                            icon=icondata::MdiBorderColor
                            current=style.border_color
                            default=Signal::derive(|| Rgb::from_bits(0x60_6060))
                            reset_label=ambre_commands::tr!("Automatic", "Automatique")
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
pub(crate) const NUMBER_FORMATS: &[(CellFormat, ambre_commands::TitleWords)] = &[
    (
        CellFormat::General,
        ambre_commands::words!("General", "Standard"),
    ),
    (
        CellFormat::Number,
        ambre_commands::words!("Number", "Nombre"),
    ),
    (
        CellFormat::Currency,
        ambre_commands::words!("Currency", "Mon\u{e9}taire"),
    ),
    (
        CellFormat::Percent,
        ambre_commands::words!("Percent", "Pourcentage"),
    ),
    (
        CellFormat::Scientific,
        ambre_commands::words!("Scientific", "Scientifique"),
    ),
    (
        CellFormat::Fraction,
        ambre_commands::words!("Fraction", "Fraction"),
    ),
    (CellFormat::Date, ambre_commands::words!("Date", "Date")),
    (CellFormat::Time, ambre_commands::words!("Time", "Heure")),
    (CellFormat::Text, ambre_commands::words!("Text", "Texte")),
];

/// The horizontal alignments offered by the popover, each with its icon and
/// tooltip. The trigger mirrors the active entry's icon.
pub(crate) const H_ALIGNS: &[(HAlign, icondata::Icon, ambre_commands::TitleWords)] = &[
    (
        HAlign::Left,
        icondata::BiAlignLeftRegular,
        ambre_commands::words!("Align left", "Aligner \u{e0} gauche"),
    ),
    (
        HAlign::Center,
        icondata::BiAlignMiddleRegular,
        ambre_commands::words!("Align center", "Centrer"),
    ),
    (
        HAlign::Right,
        icondata::BiAlignRightRegular,
        ambre_commands::words!("Align right", "Aligner \u{e0} droite"),
    ),
];

/// The vertical alignments offered by the popover, each with its icon and
/// tooltip. The trigger mirrors the active entry's icon.
pub(crate) const V_ALIGNS: &[(VAlign, icondata::Icon, ambre_commands::TitleWords)] = &[
    (
        VAlign::Top,
        icondata::AiVerticalAlignTopOutlined,
        ambre_commands::words!("Align top", "Aligner en haut"),
    ),
    (
        VAlign::Middle,
        icondata::AiVerticalAlignMiddleOutlined,
        ambre_commands::words!("Align middle", "Centrer verticalement"),
    ),
    (
        VAlign::Bottom,
        icondata::AiVerticalAlignBottomOutlined,
        ambre_commands::words!("Align bottom", "Aligner en bas"),
    ),
];

// The colour popover itself now lives in `ambre-chrome` (`ambre_chrome::ColorPopover`),
// re-exported through `super` so the call sites here and in `bars.rs` are unchanged.
// Trame uses it in RGB mode (no `alpha` prop); Toile's inspector uses the same
// component with `alpha=true` (`toile-styles`).
