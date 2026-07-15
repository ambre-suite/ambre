// ambre-toile/src/chrome/presenter.rs
// ============================================================================
// The presenter overlay panel (toile-features §1.D): a floating Leptos chrome
// panel shown while presentation mode is active. It surfaces the active slide's
// speaker notes (editable, char-CRDT via `EditNotes`) plus a `k / n` counter and
// Prev / Next / Exit navigation. It is chrome, not a WebGPU overlay
// (`ambre-spec-toile-ui.md` lines 72-74) -- the same split as the Find panel:
// a pure model in `ui::presenter` (host-tested) and this wasm-only `#[component]`.
//
// The launcher feeds it four signals (open / index / count / notes) mirrored
// from the enriched `SlideChanged` + `PresentModeChanged` events, and one
// `on_intent` callback the launcher maps to the matching `ToileCommand`.
// ============================================================================

use ambre_bus::UserId;
use leptos::html;
use leptos::prelude::*;

use crate::logic::command::TextDelta;
use crate::ui::presenter::{notes_delta, slide_counter_label};

/// An intent emitted by the presenter panel. The launcher maps each variant to a
/// `ToileCommand` sent to Logic: `Prev`/`Next` -> `PrevSlide`/`NextSlide`,
/// `Exit` -> `ExitPresent`, `EditNotes` -> `EditNotes { frame, delta }` (the
/// launcher supplies the active `frame` from the last slide change),
/// `ToggleBroadcast` -> `SetPresentBroadcast`, `Follow` -> `FollowPresenter`
/// (§1.B "Follow presenter").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresenterIntent {
    Prev,
    Next,
    Exit,
    EditNotes(TextDelta),
    /// Toggle whether this viewer broadcasts its playback position to peers.
    ToggleBroadcast(bool),
    /// Follow the given presenter (`Some`) or stop following (`None`).
    Follow(Option<UserId>),
}

/// The presenter overlay panel. `open` is driven by `PresentModeChanged`;
/// `index` / `count` drive the counter; `notes` is re-seeded on every slide
/// change. `broadcasting` reflects the local broadcast toggle; `following` the
/// active follow target; `presenter_peer` the latest peer seen broadcasting (it
/// drives whether the Follow affordance is shown). `on_intent` emits navigation,
/// note edits, and the broadcast / follow toggles (§1.B "Follow presenter").
#[component]
pub fn PresenterPanel(
    open: RwSignal<bool>,
    index: RwSignal<usize>,
    count: RwSignal<usize>,
    notes: RwSignal<String>,
    broadcasting: RwSignal<bool>,
    following: RwSignal<Option<UserId>>,
    presenter_peer: RwSignal<Option<UserId>>,
    on_intent: Callback<PresenterIntent>,
) -> impl IntoView {
    let notes_ref = NodeRef::<html::Textarea>::new();

    // The last value we pushed into the textarea. A slide change updates `notes`
    // and this effect writes it into the DOM node; local keystrokes update the
    // same signal, so without a guard the effect would fight the caret. We only
    // rewrite the node when the incoming value actually differs from what the
    // element currently holds (a fresh slide), never on the user's own edit.
    Effect::new(move |_| {
        let next = notes.get();
        if let Some(area) = notes_ref.get()
            && area.value() != next
        {
            area.set_value(&next);
        }
    });

    move || {
        open.get().then(|| {
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
                            {move || slide_counter_label(index.get(), count.get())}
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
                        // Broadcast my playback position to peers (§1.B). Active
                        // styling reflects the current toggle.
                        <button
                            class="ambre-presenter-broadcast"
                            class:active=move || broadcasting.get()
                            aria-label="Broadcast position"
                            on:click=move |_| {
                                on_intent.run(PresenterIntent::ToggleBroadcast(!broadcasting.get()))
                            }
                        >
                            "Broadcast"
                        </button>
                        // Follow the current broadcaster (§1.B). Shown only when a
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
                            // Diff against the last known value (untracked so the
                            // read does not re-run this handler), then commit the
                            // splice and adopt the new text as the baseline.
                            let old = notes.get_untracked();
                            if let Some(delta) = notes_delta(&old, &new) {
                                notes.set(new);
                                on_intent.run(PresenterIntent::EditNotes(delta));
                            }
                        }
                    ></textarea>
                </div>
            }
        })
    }
}
