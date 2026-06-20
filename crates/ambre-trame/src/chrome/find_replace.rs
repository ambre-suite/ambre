// ambre-trame/src/chrome/find_replace.rs
// ============================================================================
// The Find & Replace DOM panel (`-ui.md` sec.5.G): a single Leptos component
// with two modes (find / replace), pinned top-right. Ctrl+F opens find mode,
// Ctrl+H opens/expands replace mode (the launcher sets `state` accordingly);
// Escape closes. The panel holds the query, replacement and the search options
// (match_case / whole_cell / in_formulas / all_sheets) as its own state, and
// emits two intents: `on_find` (-> TrameCommand::FindNext) and `on_replace_all`
// (-> TrameCommand::ReplaceAll). The launcher resolves the search origin from
// the tracked active cell, so the panel never carries the cursor.
// ============================================================================

use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;

/// Which row set the panel shows. Find mode hides the replacement row and the
/// replace-only toggles; replace mode shows everything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindMode {
    Find,
    Replace,
}

/// Open state of the find/replace panel: the mode it was opened in. `None` means
/// closed. The query, replacement and options live in the component's own
/// signals (reset when the panel is reopened).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindReplaceState {
    pub mode: FindMode,
}

/// A `FindNext` request: the query plus the two matching options that also apply
/// to find. The launcher forwards it as [`ambre_bus::TrameCommand::FindNext`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindQuery {
    pub query: String,
    pub match_case: bool,
    pub whole_cell: bool,
}

/// A `ReplaceAll` request. The launcher forwards it as
/// [`ambre_bus::TrameCommand::ReplaceAll`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplaceRequest {
    pub query: String,
    pub replacement: String,
    pub match_case: bool,
    pub whole_cell: bool,
    pub in_formulas: bool,
    pub all_sheets: bool,
}

/// The find/replace panel. `result` carries `(active_1based, total)` of the last
/// `FindResult` for the "k/n" counter (`None` = no search yet / no match).
#[component]
pub fn FindReplace(
    state: RwSignal<Option<FindReplaceState>>,
    result: RwSignal<Option<(usize, usize)>>,
    on_find: Callback<FindQuery>,
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

    // Focus the find field whenever the panel (re)opens.
    Effect::new(move |_| {
        if state.get().is_some()
            && let Some(input) = find_ref.get()
        {
            let _ = input.focus();
        }
    });

    // Emit a find request with the current query and options. An empty query is a
    // valid "clear" signal: the Logic thread replies with an empty `FindResult`,
    // which wipes the canvas highlight and the counter.
    let emit_find = move || {
        on_find.run(FindQuery {
            query: query.get_untracked(),
            match_case: match_case.get_untracked(),
            whole_cell: whole_cell.get_untracked(),
        });
    };

    let close = move || {
        // Clear the canvas highlight before tearing the panel down.
        query.set(String::new());
        emit_find();
        state.set(None);
        result.set(None);
    };

    let run_find = move || emit_find();

    let replace_request = move || ReplaceRequest {
        query: query.get_untracked(),
        replacement: replacement.get_untracked(),
        match_case: match_case.get_untracked(),
        whole_cell: whole_cell.get_untracked(),
        in_formulas: in_formulas.get_untracked(),
        all_sheets: all_sheets.get_untracked(),
    };
    let run_replace = move || {
        if query.get_untracked().is_empty() {
            return;
        }
        on_replace.run(replace_request());
    };
    let run_replace_all = move || {
        if query.get_untracked().is_empty() {
            return;
        }
        on_replace_all.run(replace_request());
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
            let replace_mode = st.mode == FindMode::Replace;
            view! {
                <div class="ambre-find-panel" style="position: fixed; top: 8px; right: 8px; z-index: 60;">
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
                            aria-label=move || if replace_mode { "Hide replace" } else { "Show replace" }
                            aria-expanded=move || replace_mode.then_some("true")
                            on:click=move |_| toggle_mode()
                        >
                            <Icon icon=move || if replace_mode {
                                icondata::AiDownOutlined
                            } else {
                                icondata::AiRightOutlined
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
                            {move || match result.get() {
                                Some((active, total)) if total > 0 => format!("{active}/{total}"),
                                Some(_) => "0/0".to_string(),
                                None => String::new(),
                            }}
                        </span>
                        <button class="ambre-find-next" aria-label="Find next" on:click=move |_| run_find()>
                            <Icon icon=icondata::AiArrowDownOutlined />
                        </button>
                    </div>

                    <Show when=move || replace_mode fallback=|| ()>
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
                        <Show when=move || replace_mode fallback=|| ()>
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
