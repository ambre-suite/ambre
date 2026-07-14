// ambre-toile/src/chrome/floatbar.rs
// ============================================================================
// The content-anchored floating selection toolbar (toile-ui SS5quinquies.D),
// wasm-only. A compact `position: fixed` action bar that follows the selection
// as the board pans / zooms: the canvas view recomputes its client anchor from
// the `ViewportTransform` every frame and publishes it (`ui::floatbar::FloatBar`,
// `None` == hidden); this component only paints the resolved bar at that anchor.
//
// It offers the highest-frequency actions -- quick colour, duplicate, delete,
// group / ungroup, lock / unlock. Every button but the colour swatch is a
// `ui::menu` `CommandId` routed through the shared `on_command` path, so the bar
// is a strict subset of the context menu (SS5ter) and the two never diverge. The
// quick colour reuses the shared `ambre_chrome::ColorPopover` in ARGB mode and
// emits a `QuickFill` the view folds into a whole-register `RestyleElements`.
//
// Every control does `on:mousedown prevent_default` to keep DOM focus on the
// canvas text pipe (the palette / inspector hazard) so a click never clears the
// selection the bar acts on.
// ============================================================================

use ambre_chrome::ColorPopover;
use ambre_chrome::commands::CommandId;
use leptos::prelude::*;

use crate::ui::floatbar::FloatBar;
use crate::ui::menu;

/// The floating selection toolbar. `bar` is the per-frame published placement +
/// flags (`None` hides it); `on_command` is the shared command sink (duplicate /
/// delete / group / lock, resolved by the view against its selection); `on_fill`
/// receives a quick-colour pick the view turns into `RestyleElements`.
#[component]
pub fn SelectionToolbar(
    bar: RwSignal<Option<FloatBar>>,
    on_command: Callback<CommandId>,
    on_fill: Callback<Option<u32>>,
) -> impl IntoView {
    // The quick-colour swatch's working value: seeded from the selection's common
    // fill each time the view republishes the bar, and updated on every pick so the
    // trigger strip tracks. Reading `bar` here never re-triggers the picker itself.
    let current = RwSignal::new(None::<u32>);
    Effect::new(move |_| {
        if let Some(b) = bar.get() {
            current.set(b.fill);
        }
    });
    let pick = Callback::new(move |c: Option<u32>| {
        current.set(c);
        on_fill.run(c);
    });

    // A command button routing through the shared sink (identical to the inspector
    // / context menu). `prevent_default` on mousedown keeps the canvas focused.
    let cmd_button = move |command: CommandId, label: &'static str, icon: icondata::Icon| {
        view! {
            <button
                aria-label=label
                title=label
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| on_command.run(command)
            >
                <leptos_icons::Icon icon=icon />
            </button>
        }
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
                            <ColorPopover
                                title="Fill"
                                icon=icondata::MdiFormatColorFill
                                current=current
                                default=Signal::derive(|| 0xffff_ffffu32)
                                reset_label="No fill"
                                on_pick=pick
                                alpha=true
                            />
                            {cmd_button(menu::DUPLICATE, "Duplicate", icondata::MdiContentDuplicate)}
                            {cmd_button(menu::DELETE, "Delete", icondata::MdiDelete)}
                            {b.multi
                                .then(|| cmd_button(menu::GROUP, "Group", icondata::MdiGroup))}
                            {b.has_group
                                .then(|| cmd_button(menu::UNGROUP, "Ungroup", icondata::MdiUngroup))}
                            {b.any_unlocked
                                .then(|| cmd_button(menu::LOCK, "Lock", icondata::MdiLock))}
                            {b.any_locked
                                .then(|| {
                                    cmd_button(menu::UNLOCK, "Unlock", icondata::MdiLockOpenVariant)
                                })}
                        </div>
                    }
                })
        }}
    }
}
