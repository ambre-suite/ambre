// ambre-toile/src/chrome/palette.rs
// ============================================================================
// The board's creation bar (`toile-tools`, `CHROME-152`), wasm-only. The board
// lays out the shared `ambre_chrome::CreationBar` over its own table: the
// tools of the pointer, then the two embeds. Exactly one tool reads pressed,
// the one the shared `RwSignal<Tool>` holds; a press on a tool arms it, and a
// press on an embed runs its command.
//
// The Sticky button is a split affordance. Clicking it opens a small colour popover
// of pastel swatches, and picking a swatch arms `Tool::Sticky(colour)`. The next
// click then places notes in that colour (folded into `CreateElement.fill`,
// `toile-elements`). The bar draws it in place of the plain button of its
// command.
//
// The active tool is UI-only, canvas-local state (`toile-tools`): it is NOT a
// `ToileCommand`. The bar writes a shared `RwSignal<Tool>` that the canvas
// `ToileView` applies each frame; the keyboard chords (`ui::palette::bind_tool_keys`)
// write the very same signal. The pure command surface (ids, key chords, the id ->
// Tool map, the swatch set) lives in the always-compiled, host-tested `ui::palette`.
// ============================================================================

use ambre_core::{Commands, StaticKey};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::ui::Tool;
use crate::ui::commands as registry;
use crate::ui::palette::{STICKY_SWATCHES, TOOL_STICKY, command_for_tool, tool_for_command};

use super::icons;

/// The Sticky button + its colour popover (`toile-tools`). Clicking the button
/// opens the popover; picking a pastel swatch arms `Tool::Sticky(colour)` (which
/// remembers the colour for the next note) and closes it. `aria-pressed` reads
/// active for any `Sticky(_)` regardless of colour. The popover dismisses on any
/// pointer-down outside the control, such as a click on the canvas or on another
/// tool.
fn sticky_button(active: RwSignal<Tool>) -> impl IntoView {
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    // The picked colour tints the button's strip so the palette shows what the
    // next note will be. Seeded from the current tool colour when Sticky is
    // armed, else the default (the first swatch).
    let strip_color = move || match active.get() {
        Tool::Sticky(c) => c,
        _ => STICKY_SWATCHES.first().copied().unwrap_or_default(),
    };
    let is_active = move || matches!(active.get(), Tool::Sticky(_));
    // The words of the button are the ones the command describes (`CHROME-186`).
    let tip =
        ambre_commands::find(registry::COMMANDS, TOOL_STICKY).map(ambre_chrome::CommandTip::of);

    // Close on any pointer-down outside the control (mirrors `ambre_chrome::
    // ColorPopover`): the wrapper node lets us tell inside from outside.
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

    view! {
        <span class="ambre-color" node_ref=root_ref>
            {tip.map(|tip| view! {
                <ambre_chrome::IconButton
                    icon=icondata::MdiNoteOutline
                    label=tip.title().as_str().to_owned()
                    chord=tip.chord()
                    description=tip.description().cloned()
                    reach=Some(tip.reach())
                    pressed=Some(Signal::derive(move || ambre_chrome::Pressed::from_bool(is_active())))
                    expanded=Some(open.into())
                    size=ambre_chrome::ControlSize::Large
                    look=ambre_chrome::IconLook::Ghost
                    on_press=Callback::new(move |()| open.update(|o| *o = o.toggled()))
                >
                    <span
                        class="swatch-strip"
                        style:background-color=move || format!("#{:06x}", strip_color().rgb().bits())
                    ></span>
                </ambre_chrome::IconButton>
            })}
            <Show when=move || open.get() == ambre_chrome::Openness::Open fallback=|| ()>
                <div class="ambre-color-popover">
                    <div class="swatches">
                        {STICKY_SWATCHES
                            .iter()
                            .map(|&card| {
                                view! {
                                    <button
                                        aria-label=card.rgb().to_string()
                                        style:background-color=card.rgb().to_string()
                                        on:mousedown=move |ev| ev.prevent_default()
                                        on:click=move |_| {
                                            active.set(Tool::Sticky(card));
                                            open.set(ambre_chrome::Openness::Closed);
                                        }
                                    ></button>
                                }
                            })
                            .collect_view()}
                    </div>
                </div>
            </Show>
        </span>
    }
}

/// The board's creation bar. `active` is the shared active-tool signal, the
/// same signal the keyboard chords write and the canvas view reads. A tool
/// writes it on a press, and reads it for its pressed state. An embed runs
/// `on_command`. The bar publishes the box it covers into `cover`, which a fit
/// reads (`CHROME-170`, `ZOOM-011`).
#[component]
pub fn ToolPalette(
    active: RwSignal<Tool>,
    cover: RwSignal<ambre_client_core::cover::Cover>,
    #[prop(optional)] on_command: Option<Callback<StaticKey<Commands>>>,
) -> impl IntoView {
    let run = Callback::new(move |id: StaticKey<Commands>| match tool_for_command(id) {
        Some(tool) => active.set(tool),
        None => {
            if let Some(on_command) = on_command {
                on_command.run(id);
            }
        }
    });
    let armed = Signal::derive(move || command_for_tool(active.get()));
    let sticky = ambre_chrome::CreationSlot {
        command: TOOL_STICKY,
        view: ViewFn::from(move || sticky_button(active)),
    };
    view! {
        <ambre_chrome::CreationBar
            table=registry::COMMANDS
            resolve=|command| command.icon.and_then(icons::resolve)
            on_command=run
            cover=cover
            armed=armed
            slots=vec![sticky]
        />
    }
}
