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

use ambre_bus::UserId;
use ambre_bus::app::toile::{Broadcast, PresentMode, SlidePosition};
use ambre_core::Utf16Offset;
use leptos::html;
use leptos::prelude::*;

use crate::logic::command::TextDelta;

use crate::ui::presenter::{caret_through, notes_delta, slide_counter_label};

/// An intent emitted by the presenter panel. The launcher maps each variant to a
/// `ToileCommand` sent to Logic. `Prev`/`Next` -> `PrevSlide`/`NextSlide`,
/// `Exit` -> `ExitPresent`, and `EditNotes` -> `EditNotes { frame, base, delta }`, the
/// launcher supplying the active `frame` from the last slide change. Then
/// `ToggleBroadcast` -> `SetPresentBroadcast`, and `Follow` -> `FollowPresenter`
/// ("Follow presenter").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresenterIntent {
    Prev,
    Next,
    Exit,
    /// A splice, and the note it was measured against (`TOILE-FEAT-088`).
    EditNotes {
        base: String,
        delta: TextDelta,
    },
    /// Toggle whether this viewer broadcasts its playback position to peers.
    ToggleBroadcast(Broadcast),
    /// Follow the given presenter (`Some`) or stop following (`None`).
    Follow(Option<UserId>),
}

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
    following: RwSignal<Option<UserId>>,
    presenter_peer: RwSignal<Option<UserId>>,
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
                    let start = caret_through(&shown, Utf16Offset::new(start), &next);
                    let end = caret_through(&shown, Utf16Offset::new(end), &next);
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
                    style="position: fixed; bottom: 16px; right: 16px; z-index: 70;"
                >
                    <div class="ambre-presenter-nav">
                        <button
                            class="ambre-presenter-prev"
                            aria-label="Previous slide"
                            on:click=move |_| on_intent.run(PresenterIntent::Prev)
                        >
                            "Prev"
                        </button>
                        <span class="ambre-presenter-count">
                            {move || slide_counter_label(slide.get())}
                        </span>
                        <button
                            class="ambre-presenter-next"
                            aria-label="Next slide"
                            on:click=move |_| on_intent.run(PresenterIntent::Next)
                        >
                            "Next"
                        </button>
                        <button
                            class="ambre-presenter-exit"
                            aria-label="Exit presentation"
                            on:click=move |_| on_intent.run(PresenterIntent::Exit)
                        >
                            "Exit"
                        </button>
                        // Broadcast my playback position to peers. Active
                        // styling reflects the current toggle.
                        <button
                            class="ambre-presenter-broadcast"
                            class:active=move || broadcasting.get().is_on()
                            aria-label="Broadcast position"
                            on:click=move |_| {
                                let next = if broadcasting.get().is_on() {
                                    Broadcast::Private
                                } else {
                                    Broadcast::Broadcasting
                                };
                                on_intent.run(PresenterIntent::ToggleBroadcast(next))
                            }
                        >
                            "Broadcast"
                        </button>
                        // Follow the current broadcaster. Shown only when a
                        // peer is actively broadcasting; toggles follow on / off.
                        {move || {
                            presenter_peer.get().map(|peer| {
                                view! {
                                    <button
                                        class="ambre-presenter-follow"
                                        class:active=move || following.get().is_some()
                                        aria-label="Follow presenter"
                                        on:click=move |_| {
                                            let next = if following.get().is_some() {
                                                None
                                            } else {
                                                Some(peer)
                                            };
                                            on_intent.run(PresenterIntent::Follow(next));
                                        }
                                    >
                                        "Follow"
                                    </button>
                                }
                            })
                        }}
                    </div>
                    <textarea
                        class="ambre-presenter-notes"
                        aria-label="Speaker notes"
                        placeholder="Speaker notes"
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
