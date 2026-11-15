// ambre-toile/src/chrome/deck.rs
// ============================================================================
// The deck panel (toile-presentation): the surface that builds a presentation.
// One Leptos component over the shared signals `ui::deck::DeckSignals` carries.
//
// Two lists. The deck comes first, in playback order, each row carrying its
// rank, the two reorder steps and the drop. Then the board's other frames, each
// addable at the end of the deck. Every word it renders was read back from the
// board, and every action leaves as a `DeckIntent` the canvas view drains. The
// DOM names no command queue, and the panel keeps no order of its own.
// ============================================================================

use crate::message::{DeckView, Slide};
use ambre_core::id::{Frame, Id};
use ambre_core::index::{Len, Pos};
use leptos::prelude::*;
use wasm_bindgen::JsCast as _;

use crate::ui::PanelState;
use crate::ui::deck::{DeckIntent, Move, deck_label};

/// The `change` handler of one row's title field: it emits the typed name for
/// `frame` (`TOILE-247`). `change` rather than `input`, so one edit is one
/// command and one undo item rather than one per keystroke (`TOILE-248`).
fn name_handler(
    on_intent: Callback<DeckIntent>,
    frame: Id<Frame>,
) -> impl Fn(web_sys::Event) + 'static {
    move |ev: web_sys::Event| {
        let Some(field) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        on_intent.run(DeckIntent::Rename(frame.clone(), field.value()));
    }
}

/// The deck panel. `state` says whether it shows, set by the Present menu.
/// `view` is the board's frames as the view last read them, and `on_intent`
/// carries every action out.
#[component]
pub fn DeckPanel(
    state: RwSignal<PanelState>,
    view: RwSignal<DeckView>,
    on_intent: Callback<DeckIntent>,
) -> impl IntoView {
    move || {
        state.get().is_open().then(|| {
            view! {
                <div class="ambre-deck-panel"
                    style:position="fixed" style:top="8px" style:right="8px" style:z-index="60">
                    <div class="ambre-deck-title">
                        <span class="ambre-deck-heading">"Deck"</span>
                        <button
                            class="ambre-deck-close"
                            aria-label="Close"
                            on:click=move |_| state.set(PanelState::Closed)
                        >
                            "x"
                        </button>
                    </div>

                    <ol class="ambre-deck-list">
                        {move || {
                            let slides = view.get().slides;
                            let last = Pos::<Slide>::MAX.within(Len::of(slides.len()));
                            slides
                                .into_iter()
                                .enumerate()
                                .map(|(rank, frame)| (Pos::<Slide>::from_usize(rank), frame))
                                .map(|(rank, frame)| view! {
                                    <li class="ambre-deck-row">
                                        <button
                                            class="ambre-deck-goto"
                                            aria-label="Go to slide"
                                            on:click=move |_| on_intent.run(DeckIntent::Goto(rank))
                                        >
                                            <span class="ambre-deck-rank">{rank.number().to_string()}</span>
                                        </button>
                                        <input
                                            class="ambre-deck-name"
                                            aria-label="Frame title"
                                            placeholder=deck_label(&frame, rank)
                                            prop:value=frame.name.clone()
                                            on:change=name_handler(on_intent, frame.id.clone())
                                        />
                                        <button
                                            class="ambre-deck-up"
                                            aria-label="Move up"
                                            disabled=rank == Pos::ZERO
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
                                            // The pool numbers its unnamed
                                            // frames the way the deck does.
                                            let rank = Pos::<Slide>::from_usize(rank);
                                            let id = frame.id.clone();
                                            view! {
                                                <li class="ambre-deck-pool-row">
                                                    <input
                                                        class="ambre-deck-name"
                                                        aria-label="Frame title"
                                                        placeholder=deck_label(&frame, rank)
                                                        prop:value=frame.name.clone()
                                                        on:change=name_handler(
                                                            on_intent,
                                                            frame.id.clone(),
                                                        )
                                                    />
                                                    <button
                                                        class="ambre-deck-add-btn"
                                                        aria-label="Add to deck"
                                                        on:click=move |_| {
                                                            on_intent.run(DeckIntent::Add(id.clone()))
                                                        }
                                                    >
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
