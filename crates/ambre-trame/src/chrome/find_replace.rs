// ambre-trame/src/chrome/find_replace.rs
// ============================================================================
// The Find & Replace DOM panel (`trame-find-replace`): a single Leptos component
// with two modes (find / replace), pinned top-right. Ctrl+F opens find mode,
// Ctrl+H opens/expands replace mode (the launcher sets `state` accordingly);
// Escape closes. The panel holds the query, replacement and the search options
// (case, extent, formula scope and sheet scope) as its own state. It emits
// two intents: `on_find` (-> TrameCommand::FindNext) and `on_replace_all`
// (-> TrameCommand::ReplaceAll). The launcher resolves the search origin from
// the tracked active cell, so the panel never carries the cursor.
// ============================================================================

use ambre_core::index::{Len, Pos};
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;

use ambre_bus::app::find::{FindQuery, Match};

use crate::{CaseMatch, FindOptions, MatchExtent, SearchScope, SheetScope};

/// Which row set the panel shows. Find mode hides the replacement row and the
/// replace-only toggles; replace mode shows everything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindMode {
    Find,
    Replace,
}

impl FindMode {
    /// Whether the replacement row shows: the disclosure the chevron toggles.
    pub const fn replace_row(self) -> ambre_chrome::Openness {
        match self {
            Self::Find => ambre_chrome::Openness::Closed,
            Self::Replace => ambre_chrome::Openness::Open,
        }
    }
}

/// Open state of the find/replace panel: the mode it was opened in. `None` means
/// closed. The query, replacement and options live in the component's own
/// signals (reset when the panel is reopened).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindReplaceState {
    pub mode: FindMode,
}

/// A `ReplaceOne` or `ReplaceAll` request. The panel sends none for an empty
/// query. The launcher forwards it as [`crate::message::TrameCommand::ReplaceAll`].
#[derive(Clone, PartialEq, Eq)]
pub struct ReplaceRequest {
    pub query: FindQuery<FindOptions>,
    pub replacement: String,
}

/// The replacement's length alone: the text is document content. The query
/// prints as its own type prints it.
impl std::fmt::Debug for ReplaceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplaceRequest")
            .field("query", &self.query)
            .field("replacement_chars", &self.replacement.chars().count())
            .finish()
    }
}

/// Where the last search left the counter: the focused match, counted from one,
/// out of the matches found. The focus never passes the total, and reads 0 when
/// no match is focused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindPosition {
    focus: Option<Pos<Match>>,
    total: Len<Match>,
}

impl FindPosition {
    /// The position a `FindResult` reports: `active` indexes its `total`
    /// matches from zero. An index past the end focuses nothing.
    pub fn new(active: Option<Pos<Match>>, total: Len<Match>) -> Self {
        let focus = active.filter(|i| *i < total.as_end());
        Self { focus, total }
    }

    /// The "k/n" counter.
    pub fn label(self) -> String {
        let focus = self.focus.map_or(Len::ZERO, Pos::<Match>::number);
        format!("{focus}/{}", self.total.get())
    }
}

/// The find/replace panel. `result` carries the position of the last
/// `FindResult` for the "k/n" counter (`None` = no search yet).
#[component]
pub fn FindReplace(
    state: RwSignal<Option<FindReplaceState>>,
    result: RwSignal<Option<FindPosition>>,
    // A `FindNext` request: the find text plus the options, or `None` for an
    // emptied field. A find reads only the case and the extent. The launcher
    // forwards it as `TrameCommand::FindNext`.
    on_find: Callback<Option<FindQuery<FindOptions>>>,
    on_replace: Callback<ReplaceRequest>,
    on_replace_all: Callback<ReplaceRequest>,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let replacement = RwSignal::new(String::new());
    let match_case = RwSignal::new(false);
    let whole_cell = RwSignal::new(false);
    let in_formulas = RwSignal::new(false);
    let all_sheets = RwSignal::new(false);

    let find_ref = NodeRef::<html::Input>::new();

    // The checkbox states, read as the typed options of a request.
    let options = move || FindOptions {
        case: if match_case.get_untracked() {
            CaseMatch::Sensitive
        } else {
            CaseMatch::Insensitive
        },
        extent: if whole_cell.get_untracked() {
            MatchExtent::WholeCell
        } else {
            MatchExtent::Substring
        },
        scope: if in_formulas.get_untracked() {
            SearchScope::Formulas
        } else {
            SearchScope::Values
        },
        sheets: if all_sheets.get_untracked() {
            SheetScope::All
        } else {
            SheetScope::Active
        },
    };

    // Focus the find field whenever the panel (re)opens.
    Effect::new(move |_| {
        if state.get().is_some()
            && let Some(input) = find_ref.get()
        {
            let _ = input.focus();
        }
    });

    // Emit a find request with the current query and options. An empty field
    // sends no query, the "clear" signal: the Logic thread replies with an empty
    // `FindResult`, which wipes the canvas highlight and the counter.
    let typed_query = move || {
        ambre_bus::app::find::FindText::new(query.get_untracked())
            .map(|text| FindQuery::new(text, options()))
    };
    let emit_find = move || {
        on_find.run(typed_query());
    };

    let close = move || {
        // Clear the canvas highlight before tearing the panel down.
        query.set(String::new());
        emit_find();
        state.set(None);
        result.set(None);
    };

    let run_find = move || emit_find();

    // An empty query replaces nothing, so it sends no request.
    let replace_request = move || {
        typed_query().map(|query| ReplaceRequest {
            query,
            replacement: replacement.get_untracked(),
        })
    };
    let run_replace = move || {
        if let Some(request) = replace_request() {
            on_replace.run(request);
        }
    };
    let run_replace_all = move || {
        if let Some(request) = replace_request() {
            on_replace_all.run(request);
        }
    };
    // Flip find <-> replace mode from the mouse (the chevron button), so reaching
    // replace does not require the Ctrl+H chord.
    let toggle_mode = move || {
        state.update(|st| {
            if let Some(st) = st {
                st.mode = match st.mode {
                    FindMode::Find => FindMode::Replace,
                    FindMode::Replace => FindMode::Find,
                };
            }
        });
    };

    move || {
        state.get().map(|st| {
            let replace_row = st.mode.replace_row();
            view! {
                <div class="ambre-find-panel"
                    style:position="fixed" style:top="8px" style:right="8px" style:z-index="60">
                    <div class="ambre-find-title">
                        <span class="ambre-find-heading">"Find & Replace"</span>
                        <button class="ambre-find-close" aria-label="Close" on:click=move |_| close()>
                            <Icon icon=icondata::AiCloseOutlined />
                        </button>
                    </div>
                    <div class="ambre-find-row">
                        // Chevron (left gutter): toggles find <-> replace with the
                        // mouse, isolated so it reads as the mode switch.
                        <button
                            class="ambre-find-toggle"
                            aria-label=move || match replace_row {
                                ambre_chrome::Openness::Open => "Hide replace",
                                ambre_chrome::Openness::Closed => "Show replace",
                            }
                            aria-expanded=move || replace_row.aria()
                            on:click=move |_| toggle_mode()
                        >
                            <Icon icon=move || match replace_row {
                                ambre_chrome::Openness::Open => icondata::AiDownOutlined,
                                ambre_chrome::Openness::Closed => icondata::AiRightOutlined,
                            } />
                        </button>
                        <input
                            type="text"
                            class="ambre-find-input"
                            placeholder="Find"
                            node_ref=find_ref
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                query.set(v.clone());
                                // A changed query invalidates the current counter;
                                // an emptied field also clears the canvas highlight.
                                result.set(None);
                                if v.is_empty() {
                                    emit_find();
                                }
                            }
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                match ev.key().as_str() {
                                    "Enter" => { ev.prevent_default(); run_find(); }
                                    "Escape" => { ev.prevent_default(); close(); }
                                    _ => {}
                                }
                            }
                        />
                        <span class="ambre-find-count">
                            {move || result.get().map(FindPosition::label).unwrap_or_default()}
                        </span>
                        <button class="ambre-find-next" aria-label="Find next" on:click=move |_| run_find()>
                            <Icon icon=icondata::AiArrowDownOutlined />
                        </button>
                    </div>

                    <Show when=move || replace_row == ambre_chrome::Openness::Open fallback=|| ()>
                        <div class="ambre-find-row ambre-find-indent">
                            <input
                                type="text"
                                class="ambre-find-input"
                                placeholder="Replace with"
                                on:input=move |ev| replacement.set(event_target_value(&ev))
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Escape" { ev.prevent_default(); close(); }
                                }
                            />
                            <button class="ambre-find-replace" on:click=move |_| run_replace()>
                                "Replace"
                            </button>
                            <button class="ambre-find-replace-all" on:click=move |_| run_replace_all()>
                                "Replace all"
                            </button>
                        </div>
                    </Show>

                    <div class="ambre-find-opts ambre-find-indent">
                        <label>
                            <input
                                type="checkbox"
                                prop:checked=move || match_case.get()
                                on:change=move |ev| match_case.set(event_target_checked(&ev))
                            />
                            <span>"Match case"</span>
                        </label>
                        <label>
                            <input
                                type="checkbox"
                                prop:checked=move || whole_cell.get()
                                on:change=move |ev| whole_cell.set(event_target_checked(&ev))
                            />
                            <span>"Whole cell"</span>
                        </label>
                        <Show when=move || replace_row == ambre_chrome::Openness::Open fallback=|| ()>
                            <label>
                                <input
                                    type="checkbox"
                                    prop:checked=move || in_formulas.get()
                                    on:change=move |ev| in_formulas.set(event_target_checked(&ev))
                                />
                                <span>"In formulas"</span>
                            </label>
                            <label>
                                <input
                                    type="checkbox"
                                    prop:checked=move || all_sheets.get()
                                    on:change=move |ev| all_sheets.set(event_target_checked(&ev))
                                />
                                <span>"All sheets"</span>
                            </label>
                        </Show>
                    </div>
                </div>
            }
        })
    }
}
