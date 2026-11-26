// ambre-toile/src/chrome/presenter.rs
// ============================================================================ The
// presenter overlay panel (toile-presentation): a floating Leptos chrome panel
// shown while presentation mode is active. It surfaces the active slide's speaker
// notes (editable, char-CRDT via `EditNotes`) plus a `k / n` counter and Prev /
// Next / Exit navigation. It is chrome, not a WebGPU overlay
// (`toile-presentation`). The same split as the Find panel: a pure model in
// `ui::presenter` (host-tested), and this wasm-only `#[component]`.
//
// The launcher feeds it four signals (open / index / count / notes), mirrored from
// the enriched `SlideChanged` + `PresentModeChanged` events. Plus one `on_intent`
// callback the launcher maps to the matching `ToileCommand`.
// ============================================================================

use crate::message::{Broadcast, PresentMode, SlidePosition};
use ambre_core::Utf16;
use ambre_core::id::{Id, User};
use ambre_core::index::Pos;
use leptos::html;
use leptos::prelude::*;

use crate::ui::presenter::{PresenterIntent, caret_through, notes_delta, slide_counter_label};

/// The presenter overlay panel. `open` is driven by `PresentModeChanged`;
/// `slide` drives the counter; `notes` is re-seeded on every slide
/// change. `broadcasting` reflects the local broadcast toggle; `following` the
/// active follow target; `presenter_peer` the latest peer seen broadcasting (it
/// drives whether the Follow affordance is shown). `on_intent` emits navigation,
/// note edits, and the broadcast / follow toggles ("Follow presenter").
#[component]
pub fn PresenterPanel(
    open: RwSignal<PresentMode>,
    slide: RwSignal<SlidePosition>,
    notes: RwSignal<String>,
    broadcasting: RwSignal<Broadcast>,
    following: RwSignal<Option<Id<User>>>,
    presenter_peer: RwSignal<Option<Id<User>>>,
    on_intent: Callback<PresenterIntent>,
) -> impl IntoView {
    let notes_ref = NodeRef::<html::Textarea>::new();

    // The last value we pushed into the textarea. A slide change updates `notes`
    // and this effect writes it into the DOM node. Local keystrokes update the
    // same signal, so without a guard the effect would fight the caret. We only
    // rewrite the node when the incoming value differs from what it holds: on
    // a fresh slide, or on a peer's write. Never on the user's own edit. A peer's write keeps the caret where the user left
    // it (`TOILE-UI-316`).
    Effect::new(move |_| {
        let next = notes.get();
        if let Some(area) = notes_ref.get() {
            let shown = area.value();
            if shown != next {
                let start = area.selection_start().ok().flatten();
                let end = area.selection_end().ok().flatten();
                area.set_value(&next);
                if let (Some(start), Some(end)) = (start, end) {
                    let start = caret_through(&shown, Pos::<Utf16>::new(start), &next);
                    let end = caret_through(&shown, Pos::<Utf16>::new(end), &next);
                    let _ = area.set_selection_range(start.get(), end.get());
                }
            }
        }
    });

    move || {
        open.get().is_presenting().then(|| {
            view! {
                <div
                    class="ambre-presenter-panel"
                    style:position="fixed"
                    style:bottom="16px"
                    style:right="16px"
                    style:z-index="70"
                >
                    <div class="ambre-presenter-nav">
                        <button
                            class="ambre-presenter-prev"
                            aria-label=ambre_commands::tr!("Previous slide", "Diapositive pr\u{e9}c\u{e9}dente")
                            on:click=move |_| on_intent.run(PresenterIntent::Prev)
                        >
                            {ambre_commands::tr!("Prev", "Pr\u{e9}c.")}
                        </button>
                        <span class="ambre-presenter-count">
                            {move || slide_counter_label(slide.get())}
                        </span>
                        <button
                            class="ambre-presenter-next"
                            aria-label=ambre_commands::tr!("Next slide", "Diapositive suivante")
                            on:click=move |_| on_intent.run(PresenterIntent::Next)
                        >
                            {ambre_commands::tr!("Next", "Suivant")}
                        </button>
                        <button
                            class="ambre-presenter-exit"
                            aria-label=ambre_commands::tr!("Exit presentation", "Quitter la pr\u{e9}sentation")
                            on:click=move |_| on_intent.run(PresenterIntent::Exit)
                        >
                            {ambre_commands::tr!("Exit", "Quitter")}
                        </button>
                        // Broadcast my playback position to peers. Active
                        // styling reflects the current toggle.
                        <button
                            class="ambre-presenter-broadcast"
                            class:active=move || broadcasting.get().is_on()
                            aria-label=ambre_commands::tr!("Broadcast position", "Diffuser la position")
                            on:click=move |_| {
                                let next = if broadcasting.get().is_on() {
                                    Broadcast::Private
                                } else {
                                    Broadcast::Broadcasting
                                };
                                on_intent.run(PresenterIntent::ToggleBroadcast(next))
                            }
                        >
                            {ambre_commands::tr!("Broadcast", "Diffuser")}
                        </button>
                        // Follow the current broadcaster. Shown only when a
                        // peer is actively broadcasting; toggles follow on / off.
                        {move || {
                            presenter_peer.get().map(|peer| {
                                view! {
                                    <button
                                        class="ambre-presenter-follow"
                                        class:active=move || following.get().is_some()
                                        aria-label=ambre_commands::tr!("Follow presenter", "Suivre le pr\u{e9}sentateur")
                                        on:click=move |_| {
                                            let next = if following.get().is_some() {
                                                None
                                            } else {
                                                Some(peer)
                                            };
                                            on_intent.run(PresenterIntent::Follow(next));
                                        }
                                    >
                                        {ambre_commands::tr!("Follow", "Suivre")}
                                    </button>
                                }
                            })
                        }}
                    </div>
                    <textarea
                        class="ambre-presenter-notes"
                        aria-label=ambre_commands::tr!("Speaker notes", "Notes de l'orateur")
                        placeholder=ambre_commands::tr!("Speaker notes", "Notes de l'orateur")
                        node_ref=notes_ref
                        on:input=move |ev| {
                            let new = event_target_value(&ev);
                            // Diff against the last known value, untracked so the
                            // read does not re-run this handler. Then commit the
                            // splice with the value it was measured against, and
                            // adopt the new text as the baseline.
                            let base = notes.get_untracked();
                            if let Some(delta) = notes_delta(&base, &new) {
                                notes.set(new);
                                on_intent.run(PresenterIntent::EditNotes { base, delta });
                            }
                        }
                    ></textarea>
                </div>
            }
        })
    }
}
