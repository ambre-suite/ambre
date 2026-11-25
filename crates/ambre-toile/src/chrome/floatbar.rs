// ambre-toile/src/chrome/floatbar.rs
// ============================================================================ The
// content-anchored floating selection toolbar (`toile-styles`), wasm-only. A compact
// `position: fixed` action bar that follows the selection as the board pans / zooms. The
// canvas view recomputes its client anchor from the `ViewportTransform` every frame and
// publishes it (`ui::floatbar::FloatBar`, `None` == hidden). This component only paints
// the resolved bar at that anchor.
//
// It is the board's context bar (`ambre-spec-shell.md` section 2): the fill, the text
// colour, duplicate, delete, and lock or unlock. Each entry is a control the context panel
// shows for a shape, and `ui::panel::PanelKind::bar` declares the list (`CHROME-208`).
// The command buttons are `ui::menu` `StaticKey<Commands>`s routed through the shared
// `on_command` path. The two colours reuse the shared picker. They emit a single-field
// `StyleEdit` the view folds into a whole-register `RestyleElements` over the live
// selection.
//
// The style controls seed from the published inspector `StyleModel` (the same
// common-style model the side inspector reads), so the bar mirrors the selection. Each
// control emits ONE changed field (`StyleEdit`); the view re-derives the base live,
// avoiding the inspector's accumulating-working-copy staleness.
//
// Every control does `on:mousedown prevent_default` to keep DOM focus on the canvas text
// pipe, the palette / inspector hazard. A click then never clears the selection the bar
// acts on. ============================================================================

use ambre_chrome::commands::Reach;
use ambre_core::{Commands, Rgba, StaticKey};

use super::icons;
use crate::ui::commands as registry;
use ambre_chrome::ColorPopover;
use leptos::prelude::*;

use crate::ui::floatbar::{FloatBar, StyleEdit};
use crate::ui::menu;
use crate::ui::panel;
use crate::ui::style::StyleModel;

/// The name a colour trigger shows in its tooltip: the label the property
/// registry declares, as the context panel shows it (`CHROME-186`).
fn property_title(id: StaticKey<ambre_core::Properties>) -> String {
    panel::property(id)
        .title
        .pick(&ambre_chrome::language())
        .into_string()
}

/// The floating selection toolbar. `bar` is the per-frame published placement +
/// flags (`None` hides it). `model` is the selection's common style: it seeds
/// the text colour, and is the same model the context panel reads.
/// `on_command` is the shared command sink (duplicate / delete / lock,
/// resolved by the view against its selection). `on_style` receives a
/// single-field `StyleEdit` the view turns into a live-based `RestyleElements`.
#[component]
pub fn SelectionToolbar(
    bar: RwSignal<Option<FloatBar>>,
    model: RwSignal<Option<StyleModel>>,
    on_command: Callback<StaticKey<Commands>>,
    on_style: Callback<StyleEdit>,
    /// Where the bar, the board's context bar, publishes how far it reaches
    /// above the selection (`CHROME-170`, `ZOOM-011`).
    cover: RwSignal<ambre_client_core::cover::Cover>,
) -> impl IntoView {
    use ambre_client_core::cover::Surface;
    let node = NodeRef::<leptos::html::Div>::new();
    ambre_chrome::cover::watch_above(node, move || bar.get_untracked().map(|b| b.at.y), cover);
    Effect::new(move |_| {
        if bar.with(Option::is_none) {
            ambre_chrome::cover::clear(cover, Surface::ContextBar);
        }
    });
    let fill = RwSignal::new(None::<Rgba>);
    let text_color = RwSignal::new(None::<Rgba>);
    Effect::new(move |_| {
        if let Some(b) = bar.get() {
            fill.set(b.fill);
        }
    });
    Effect::new(move |_| {
        if let Some(m) = model.get() {
            text_color.set(m.base.text_color);
        }
    });
    let on_fill = Callback::new(move |c: Option<Rgba>| {
        fill.set(c);
        on_style.run(StyleEdit::Fill(c));
    });
    let on_text_color = Callback::new(move |c: Option<Rgba>| {
        text_color.set(c);
        on_style.run(StyleEdit::TextColor(c));
    });

    // A command button routing through the shared sink, as the context menu
    // does. The tooltip, its chord and its description and the glyph are the
    // ones the command declares (`UI-THREAD-019`, `CHROME-186`).
    let cmd_button = move |command: StaticKey<Commands>| {
        let icon = registry::icon(command).and_then(icons::resolve);
        ambre_commands::find(registry::COMMANDS, command).map(|described| {
            view! {
                <ambre_chrome::CommandButton
                    command=described
                    icon=icon
                    size=ambre_chrome::ControlSize::Large
                    look=ambre_chrome::IconLook::Ghost
                    on_command=on_command
                />
            }
        })
    };

    // A "Mixed" hint beside a colour that differs across the selection
    // (`TOILE-UI-164`).
    let mixed_hint = move |differs: bool| {
        differs.then(|| view! { <span class="ambre-inspector-mixed">"Mixed"</span> })
    };

    view! {
        {move || {
            bar.get()
                .map(|b| {
                    // Anchor: fixed at the selection's top-edge midpoint; the CSS
                    // lifts the bar above it with a gap (`.ambre-selbar` transform).
                    let (left, top) = (format!("{}px", b.at.x.get()), format!("{}px", b.at.y.get()));
                    view! {
                        <div
                            class="ambre-selbar ambre-commandbar"
                            role="toolbar"
                            aria-label="Selection actions"
                            node_ref=node
                            style:left=left
                            style:top=top
                        >
                            <span data-reach=Reach::Property(panel::FILL).marker()>
                                <ColorPopover
                                    title=property_title(panel::FILL)
                                    icon=icondata::MdiFormatColorFill
                                    current=fill
                                    default=Signal::derive(|| Rgba::from_bits(0xffff_ffff))
                                    reset_label="No fill"
                                    on_pick=on_fill
                                />
                            </span>
                            {mixed_hint(b.mixed.fill.is_mixed())}
                            <span data-reach=Reach::Property(panel::TEXT_COLOUR).marker()>
                                <ColorPopover
                                    title=property_title(panel::TEXT_COLOUR)
                                    icon=icondata::MdiFormatColorText
                                    current=text_color
                                    default=Signal::derive(|| Rgba::from_bits(0x1a1a_1aff))
                                    reset_label="Default"
                                    on_pick=on_text_color
                                />
                            </span>
                            {mixed_hint(b.mixed.text_color.is_mixed())}
                            {cmd_button(menu::DUPLICATE)}
                            {cmd_button(menu::DELETE)}
                            {b.locks.any_unlocked().then(|| cmd_button(menu::LOCK))}
                            {b.locks.any_locked().then(|| cmd_button(menu::UNLOCK))}
                        </div>
                    }
                })
        }}
    }
}
