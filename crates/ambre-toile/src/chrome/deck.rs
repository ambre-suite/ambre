// ambre-toile/src/chrome/deck.rs
// ============================================================================
// The deck panel (toile-presentation): the surface that builds a presentation.
// One Leptos component over the shared signals `ui::deck::DeckSignals` carries.
//
// Two lists. The deck comes first, in playback order, each row carrying its
// rank, the two reorder steps and the drop. Then the board's other frames, each
// addable at the end of the deck. Every word it renders was read back from the
// board, and every action leaves as a `DeckIntent` the canvas view drains. The
// DOM names no command channel, and the panel keeps no order of its own.
// ============================================================================

use ambre_bus::app::toile::DeckView;
use leptos::prelude::*;

use crate::ui::deck::{DeckIntent, Move, deck_label};

/// The deck panel. `state` is the open flag, set by the Present menu. `view`
/// is the board's frames as the view last read them, and `on_intent` carries
/// every action out.
#[component]
pub fn DeckPanel(
    state: RwSignal<bool>,
    view: RwSignal<DeckView>,
    on_intent: Callback<DeckIntent>,
) -> impl IntoView {
    move || {
        state.get().then(|| {
            view! {
                <div class="ambre-deck-panel" style="position: fixed; top: 8px; right: 8px; z-index: 60;">
                    <div class="ambre-deck-title">
                        <span class="ambre-deck-heading">"Deck"</span>
                        <button
                            class="ambre-deck-close"
                            aria-label="Close"
                            on:click=move |_| state.set(false)
                        >
                            "x"
                        </button>
                    </div>

                    <ol class="ambre-deck-list">
                        {move || {
                            let slides = view.get().slides;
                            let last = slides.len().saturating_sub(1);
                            slides
                                .into_iter()
                                .enumerate()
                                .map(|(rank, frame)| view! {
                                    <li class="ambre-deck-row">
                                        <button
                                            class="ambre-deck-goto"
                                            on:click=move |_| on_intent.run(DeckIntent::Goto(rank))
                                        >
                                            <span class="ambre-deck-rank">{(rank + 1).to_string()}</span>
                                            <span class="ambre-deck-name">{deck_label(&frame, rank)}</span>
                                        </button>
                                        <button
                                            class="ambre-deck-up"
                                            aria-label="Move up"
                                            disabled=rank == 0
                                            on:click=move |_| {
                                                on_intent.run(DeckIntent::Shift(rank, Move::Up))
                                            }
                                        >
                                            "^"
                                        </button>
                                        <button
                                            class="ambre-deck-down"
                                            aria-label="Move down"
                                            disabled=rank == last
                                            on:click=move |_| {
                                                on_intent.run(DeckIntent::Shift(rank, Move::Down))
                                            }
                                        >
                                            "v"
                                        </button>
                                        <button
                                            class="ambre-deck-remove"
                                            aria-label="Remove from deck"
                                            on:click=move |_| on_intent.run(DeckIntent::Remove(rank))
                                        >
                                            "x"
                                        </button>
                                    </li>
                                })
                                .collect_view()
                        }}
                    </ol>

                    {move || {
                        let off = view.get().off_deck;
                        (!off.is_empty()).then(|| view! {
                            <div class="ambre-deck-add">
                                <span class="ambre-deck-subhead">"Frames"</span>
                                <ul class="ambre-deck-pool">
                                    {off
                                        .into_iter()
                                        .enumerate()
                                        .map(|(rank, frame)| {
                                            let id = frame.id.clone();
                                            view! {
                                                <li class="ambre-deck-pool-row">
                                                    <button
                                                        class="ambre-deck-add-btn"
                                                        on:click=move |_| {
                                                            on_intent.run(DeckIntent::Add(id.clone()))
                                                        }
                                                    >
                                                        <span class="ambre-deck-name">
                                                            {deck_label(&frame, rank)}
                                                        </span>
                                                        <span class="ambre-deck-plus">"+"</span>
                                                    </button>
                                                </li>
                                            }
                                        })
                                        .collect_view()}
                                </ul>
                            </div>
                        })
                    }}

                    {move || {
                        view.get().slides.is_empty().then(|| view! {
                            <p class="ambre-deck-empty">
                                "No slides yet. Add a frame to start the deck."
                            </p>
                        })
                    }}
                </div>
            }
        })
    }
}
