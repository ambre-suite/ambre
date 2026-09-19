// ambre-toile/src/chrome/floatbar.rs
// ============================================================================ The
// content-anchored floating selection toolbar (`toile-styles`), wasm-only. A compact
// `position: fixed` action bar that follows the selection as the board pans / zooms. The
// canvas view recomputes its client anchor from the `ViewportTransform` every frame and
// publishes it (`ui::floatbar::FloatBar`, `None` == hidden). This component only paints
// the resolved bar at that anchor.
//
// It offers the highest-frequency actions -- quick fill + text formatting (text colour,
// font, size, bold, italic), duplicate, delete, group / ungroup, lock / unlock. The
// command buttons (duplicate / delete / group / lock) are `ui::menu` `CommandId`s routed
// through the shared `on_command` path. So the bar is a strict subset of the context
// menu (`toile-menus`), and the two never diverge. The style controls reuse the shared
// `ambre_chrome` value controls (`ColorPopover` in ARGB mode, `FontPicker`). They emit a
// single-field `StyleEdit` the view folds into a whole-register `RestyleElements` over
// the live selection.
//
// The style controls seed from the published inspector `StyleModel` (the same
// common-style model the side inspector reads), so the bar mirrors the selection. Each
// control emits ONE changed field (`StyleEdit`); the view re-derives the base live,
// avoiding the inspector's accumulating-working-copy staleness.
//
// Every control does `on:mousedown prevent_default` to keep DOM focus on the canvas text
// pipe, the palette / inspector hazard. A click then never clears the selection the bar
// acts on. ============================================================================

use ambre_chrome::commands::{CommandId, Locale};

use super::icons;
use crate::ui::commands as registry;
use ambre_chrome::{ColorPopover, FontPicker, FontSizeSelect, FormatToggle};
use leptos::prelude::*;

use crate::ui::floatbar::{FloatBar, StyleEdit};
use crate::ui::menu;
use crate::ui::style::StyleModel;

/// Map a stored `FontSpec.family` string to the picker's family label (`None` =
/// Default): only the offered labels round-trip. Anything else reads as
/// Default, the raw "sans-serif" included. Mirrors the inspector's mapping.
fn family_label(family: &str) -> Option<String> {
    matches!(family, "Sans" | "Serif" | "Monospace").then(|| family.to_owned())
}

/// The floating selection toolbar. `bar` is the per-frame published placement +
/// flags (`None` hides it). `model` is the selection's common style: it seeds
/// the style controls, and is the same model the side inspector reads.
/// `on_command` is the shared command sink (duplicate / delete / group / lock,
/// resolved by the view against its selection). `on_style` receives a
/// single-field `StyleEdit` the view turns into a live-based `RestyleElements`.
#[component]
pub fn SelectionToolbar(
    bar: RwSignal<Option<FloatBar>>,
    model: RwSignal<Option<StyleModel>>,
    on_command: Callback<CommandId>,
    on_style: Callback<StyleEdit>,
) -> impl IntoView {
    // Local working values for the style controls. They are seeded from the
    // published bar / model each time the view republishes them, and updated on
    // every pick so the trigger strips track. Reading `bar` / `model` here never
    // re-triggers a picker.
    let fill = RwSignal::new(None::<u32>);
    let text_color = RwSignal::new(None::<u32>);
    let font_family = RwSignal::new(None::<String>);
    let font_size = RwSignal::new(0.0_f32);
    let bold = RwSignal::new(false);
    let italic = RwSignal::new(false);
    let underline = RwSignal::new(false);
    let strikethrough = RwSignal::new(false);
    // The fill swatch seeds from the bar's common fill (already folded there).
    Effect::new(move |_| {
        if let Some(b) = bar.get() {
            fill.set(b.fill);
        }
    });
    // The text controls seed from the inspector's common-style model.
    Effect::new(move |_| {
        if let Some(m) = model.get() {
            text_color.set(m.base.text_color);
            font_family.set(family_label(&m.base.font.family));
            font_size.set(m.base.font.size);
            bold.set(m.base.font.weight >= 700);
            italic.set(m.base.font.italic);
            underline.set(m.base.font.underline);
            strikethrough.set(m.base.font.strikethrough);
        }
    });

    // Fill / text-colour picks: track the local swatch and emit the field edit.
    let on_fill = Callback::new(move |c: Option<u32>| {
        fill.set(c);
        on_style.run(StyleEdit::Fill(c));
    });
    let on_text_color = Callback::new(move |c: Option<u32>| {
        text_color.set(c);
        on_style.run(StyleEdit::TextColor(c));
    });
    let on_font = Callback::new(move |f: Option<String>| {
        font_family.set(f.clone());
        on_style.run(StyleEdit::Font(f));
    });
    // Size: the shared dropdown speaks points (`Option<u16>`, `None` = auto). Toile
    // stores world px as `f32` with `0.0` = the auto sentinel, so map both ways.
    let size_current = Signal::derive(move || {
        let s = font_size.get();
        (s > 0.0).then_some(s.round() as u16)
    });
    let on_size = Callback::new(move |p: Option<u16>| {
        let v = p.map(f32::from).unwrap_or(0.0);
        font_size.set(v);
        on_style.run(StyleEdit::Size(v));
    });
    // The four format toggles emit their flipped state and track locally.
    let on_bold = Callback::new(move |()| {
        let next = !bold.get_untracked();
        bold.set(next);
        on_style.run(StyleEdit::Bold(next));
    });
    let on_italic = Callback::new(move |()| {
        let next = !italic.get_untracked();
        italic.set(next);
        on_style.run(StyleEdit::Italic(next));
    });
    let on_underline = Callback::new(move |()| {
        let next = !underline.get_untracked();
        underline.set(next);
        on_style.run(StyleEdit::Underline(next));
    });
    let on_strike = Callback::new(move |()| {
        let next = !strikethrough.get_untracked();
        strikethrough.set(next);
        on_style.run(StyleEdit::Strikethrough(next));
    });

    // A command button routing through the shared sink (identical to the inspector
    // / context menu). `prevent_default` on mousedown keeps the canvas focused.
    // The tooltip and the glyph are the ones the command declares
    // (`UI-THREAD-019`), so this bar and the right-click menu never disagree.
    let cmd_button = move |command: CommandId| {
        let label = registry::title(command, &Locale::En).unwrap_or_default();
        let icon = registry::icon(command).and_then(icons::resolve);
        view! {
            <button
                aria-label=label.clone()
                title=label
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| on_command.run(command)
            >
                {icon.map(|ic| view! { <leptos_icons::Icon icon=ic /> })}
            </button>
        }
    };

    // A "Mixed" hint shown beside a control whose value differs across the
    // selection (`TOILE-UI-164`), the same hint the inspector uses.
    let mixed_hint = move |differs: bool| {
        differs.then(|| view! { <span class="ambre-inspector-mixed">"Mixed"</span> })
    };

    view! {
        {move || {
            bar.get()
                .map(|b| {
                    // Anchor: fixed at the selection's top-edge midpoint; the CSS
                    // lifts the bar above it with a gap (`.ambre-selbar` transform).
                    let pos = format!("left: {}px; top: {}px;", b.x, b.y);
                    view! {
                        <div
                            class="ambre-selbar ambre-commandbar"
                            role="toolbar"
                            aria-label="Selection actions"
                            style=pos
                        >
                            // -- Text format: bold / italic / underline / strike, then
                            // font, size, text colour, fill -- same order as Trame's
                            // toolbar (shared `ambre_chrome` controls). --
                            <FormatToggle
                                label="Bold"
                                icon=icondata::BsTypeBold
                                pressed=bold.into()
                                on_toggle=on_bold
                            />
                            <FormatToggle
                                label="Italic"
                                icon=icondata::BsTypeItalic
                                pressed=italic.into()
                                on_toggle=on_italic
                            />
                            <FormatToggle
                                label="Underline"
                                icon=icondata::BsTypeUnderline
                                pressed=underline.into()
                                on_toggle=on_underline
                            />
                            <FormatToggle
                                label="Strikethrough"
                                icon=icondata::BsTypeStrikethrough
                                pressed=strikethrough.into()
                                on_toggle=on_strike
                            />
                            {mixed_hint(
                                b.mixed.bold || b.mixed.italic || b.mixed.underline
                                    || b.mixed.strikethrough,
                            )}
                            <FontPicker current=font_family.into() on_pick=on_font />
                            {mixed_hint(b.mixed.font_family)}
                            <FontSizeSelect current=size_current on_pick=on_size auto_label="Auto" />
                            {mixed_hint(b.mixed.font_size)}
                            <ColorPopover
                                title="Text colour"
                                icon=icondata::MdiFormatColorText
                                current=text_color
                                default=Signal::derive(|| 0xff1a_1a1au32)
                                reset_label="Default"
                                on_pick=on_text_color
                                alpha=true
                            />
                            {mixed_hint(b.mixed.text_color)}
                            <ColorPopover
                                title="Fill"
                                icon=icondata::MdiFormatColorFill
                                current=fill
                                default=Signal::derive(|| 0xffff_ffffu32)
                                reset_label="No fill"
                                on_pick=on_fill
                                alpha=true
                            />
                            {mixed_hint(b.mixed.fill)}
                            // -- Structural actions (strict subset of the context menu) --
                            {cmd_button(menu::DUPLICATE)}
                            {cmd_button(menu::DELETE)}
                            {b.multi
                                .then(|| cmd_button(menu::GROUP))}
                            {b.has_group
                                .then(|| cmd_button(menu::UNGROUP))}
                            {b.any_unlocked
                                .then(|| cmd_button(menu::LOCK))}
                            {b.any_locked
                                .then(|| {
                                    cmd_button(menu::UNLOCK)
                                })}
                        </div>
                    }
                })
        }}
    }
}
