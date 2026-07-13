// ambre-toile/src/chrome/find.rs
// ============================================================================
// The Find panel (toile-ui §5quater): a single Leptos component pinned to a
// board corner, opened by Ctrl+F. Find-only -- Replace is out of MVP on a
// free-form canvas (§12). The panel owns the query and the two options
// (match_case / whole_word) as its own signals and emits one intent, `on_find`
// (-> ToileCommand::Find), whenever the query changes or the user steps through
// matches with Enter / Shift+Enter.
//
// Because a match can sit off-screen (the UI holds only the culled viewport),
// the search runs in the Logic thread: `on_find` carries a `from` element so
// Logic's forward-wrapping scan lands on the next / previous hit. The panel
// tracks the current `FindResults` (fed back by the launcher relay) purely to
// render the "k/n" counter and to compute the `from` for the next step -- the
// canvas view handles the highlight + camera recenter off the same event.
// ============================================================================

use ambre_bus::app::toile::{ElementId, FindOptions, FindResults};
use leptos::html;
use leptos::prelude::*;

use crate::ui::find::active_index;

/// A `Find` request emitted by the panel: the query, the options, the hit to
/// resume from (`None` = a fresh search focusing the first hit), and the step
/// direction. The launcher forwards it as
/// [`ambre_bus::app::toile::ToileCommand::Find`]; Logic steps `focus` forward or
/// backward from `from` itself (toile-logic §8 quater).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindQuery {
    pub query: String,
    pub options: FindOptions,
    pub from: Option<ElementId>,
    pub backward: bool,
}

/// The Find panel. `state` is the open flag (Ctrl+F / the launcher sets it);
/// `result` carries the last [`FindResults`] from Logic (drives the counter and
/// the step origin). `on_find` emits every search / step request.
#[component]
pub fn FindPanel(
    state: RwSignal<bool>,
    result: RwSignal<Option<FindResults>>,
    on_find: Callback<FindQuery>,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let match_case = RwSignal::new(false);
    let whole_word = RwSignal::new(false);

    let find_ref = NodeRef::<html::Input>::new();

    // Focus the field whenever the panel opens.
    Effect::new(move |_| {
        if state.get()
            && let Some(input) = find_ref.get()
        {
            let _ = input.focus();
        }
    });

    // Emit a `Find` with the current query / options, resume point and direction.
    let emit = move |from: Option<ElementId>, backward: bool| {
        on_find.run(FindQuery {
            query: query.get_untracked(),
            options: FindOptions {
                match_case: match_case.get_untracked(),
                whole_word: whole_word.get_untracked(),
            },
            from,
            backward,
        });
    };

    // A fresh search from the query box: no resume point, Logic focuses the first
    // hit. An empty query is a valid "clear" signal (Logic replies with no hits,
    // wiping the canvas highlight and the counter).
    let run_fresh = move || emit(None, false);

    // Step to the next (or previous) match, resuming from the current focus.
    // Logic advances or rewinds from `from` and wraps (toile-logic §8 quater).
    let step = move |backward: bool| {
        let from = result.with_untracked(|r| r.as_ref().and_then(|fr| fr.focus.clone()));
        emit(from, backward);
    };

    let close = move || {
        // Clear the query and emit so the canvas highlight is wiped, then hide.
        query.set(String::new());
        emit(None, false);
        state.set(false);
        result.set(None);
    };

    // Re-emit the current query when an option toggles, so the highlight and
    // counter reflect the new matching rules immediately.
    let on_toggle = move || {
        result.set(None);
        run_fresh();
    };

    move || {
        state.get().then(|| {
            view! {
                <div class="ambre-find-panel" style="position: fixed; top: 8px; right: 8px; z-index: 60;">
                    <div class="ambre-find-title">
                        <span class="ambre-find-heading">"Find on board"</span>
                        <button class="ambre-find-close" aria-label="Close" on:click=move |_| close()>
                            "x"
                        </button>
                    </div>
                    <div class="ambre-find-row">
                        <input
                            type="text"
                            class="ambre-find-input"
                            placeholder="Find"
                            node_ref=find_ref
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                query.set(v);
                                // A changed query invalidates the current counter;
                                // the fresh search also clears the highlight when
                                // emptied (Logic returns no hits).
                                result.set(None);
                                run_fresh();
                            }
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                match ev.key().as_str() {
                                    "Enter" => {
                                        ev.prevent_default();
                                        step(ev.shift_key());
                                    }
                                    "Escape" => { ev.prevent_default(); close(); }
                                    _ => {}
                                }
                            }
                        />
                        <span class="ambre-find-count">
                            {move || match result.get() {
                                Some(fr) if !fr.hits.is_empty() => {
                                    let active = active_index(&fr.hits, fr.focus.as_ref())
                                        .map(|i| i + 1)
                                        .unwrap_or(0);
                                    format!("{active}/{}", fr.hits.len())
                                }
                                Some(_) => "0/0".to_string(),
                                None => String::new(),
                            }}
                        </span>
                        <button
                            class="ambre-find-prev"
                            aria-label="Previous match"
                            on:click=move |_| step(true)
                        >
                            "Prev"
                        </button>
                        <button
                            class="ambre-find-next"
                            aria-label="Next match"
                            on:click=move |_| step(false)
                        >
                            "Next"
                        </button>
                    </div>

                    <div class="ambre-find-opts">
                        <label>
                            <input
                                type="checkbox"
                                prop:checked=move || match_case.get()
                                on:change=move |ev| {
                                    match_case.set(event_target_checked(&ev));
                                    on_toggle();
                                }
                            />
                            <span>"Match case"</span>
                        </label>
                        <label>
                            <input
                                type="checkbox"
                                prop:checked=move || whole_word.get()
                                on:change=move |ev| {
                                    whole_word.set(event_target_checked(&ev));
                                    on_toggle();
                                }
                            />
                            <span>"Whole word"</span>
                        </label>
                    </div>
                </div>
            }
        })
    }
}
