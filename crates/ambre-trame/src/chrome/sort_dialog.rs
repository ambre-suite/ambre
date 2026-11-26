// ambre-trame/src/chrome/sort_dialog.rs
// ============================================================================
// The multi-column Sort dialog (`trame-sort`, "Sort range..."). A small DOM
// dialog listing ordered sort levels (primary, secondary, ...), each a column
// picker + ascending/descending direction. It also carries a "data has header
// row" toggle and add/remove level. Applying emits a `SortApply` the launcher
// forwards as [`crate::message::TrameCommand::SortRangeMulti`], over the snapshotted
// selection rectangle. The Logic thread sorts the rows lexicographically by the
// keys (`trame-sort`) and replies `SortComplete`.
//
// The dialog is opened over a snapshot of the current selection bounds,
// published by the canvas view. The columns it offers are exactly the
// selection's columns. Each `SortKey::col` it emits is range-relative
// (`0` = the selection's left column), matching the `SortRangeMulti` contract.
// ============================================================================

use crate::message::{HeaderRow, SortKey, SortOrder};
use ambre_bus::{Col, Row};
use ambre_core::index::{Len, Pos};
use leptos::prelude::*;
use leptos_icons::Icon;

use crate::render::col_label;
use ambre_core::Flag as _;

/// Open state of the sort dialog: a snapshot of the selection rectangle the sort
/// applies to (`(r1, c1, r2, c2)`, inclusive, corners ordered). `None` means
/// closed. The ordered sort levels live in the component's own signal (reset
/// each time the dialog reopens).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortDialogState {
    pub r1: Pos<Row>,
    pub c1: Pos<Col>,
    pub r2: Pos<Row>,
    pub c2: Pos<Col>,
}

/// A multi-column sort request. The launcher forwards it as the parameterless
/// [`crate::message::TrameCommand::SortSelectionMulti`] (the Logic thread resolves the
/// range from the tracked selection). `sort_keys` is in decreasing priority
/// (`[primary, secondary, ...]`); each `col` is range-relative (`0` = the
/// selection's left column), matching the snapshot the dialog was opened over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortApply {
    pub sort_keys: Vec<SortKey>,
    pub header: HeaderRow,
}

/// The multi-column sort dialog. `state` carries the selection snapshot the sort
/// targets (set by the launcher when "Sort range..." is invoked); `on_apply`
/// emits the chosen levels.
#[component]
pub fn SortDialog(
    state: RwSignal<Option<SortDialogState>>,
    on_apply: Callback<SortApply>,
) -> impl IntoView {
    // Ordered sort levels (decreasing priority). Reset to a single primary level
    // (the selection's first column, ascending) each time the dialog reopens.
    let levels = RwSignal::new(vec![SortKey {
        col: Len::<Col>::ZERO,
        order: SortOrder::Ascending,
    }]);
    let has_header = RwSignal::new(false);

    Effect::new(move |_| {
        if state.get().is_some() {
            levels.set(vec![SortKey {
                col: Len::<Col>::ZERO,
                order: SortOrder::Ascending,
            }]);
            has_header.set(false);
        }
    });

    let close = move || state.set(None);

    let apply = move || {
        if state.get_untracked().is_none() {
            return;
        }
        // Keep only distinct columns (later duplicates are ignored), preserving
        // priority order: a repeated column never decides past its first level.
        let mut seen = Vec::new();
        let keys: Vec<SortKey> = levels
            .get_untracked()
            .into_iter()
            .filter(|k| {
                if seen.contains(&k.col) {
                    false
                } else {
                    seen.push(k.col);
                    true
                }
            })
            .collect();
        if keys.is_empty() {
            close();
            return;
        }
        on_apply.run(SortApply {
            sort_keys: keys,
            header: if has_header.get_untracked() {
                HeaderRow::Present
            } else {
                HeaderRow::Absent
            },
        });
        close();
    };

    // The shared modal traps the focus and leaves the sheet inert
    // (`CHROME-028`, `CHROME-068`). Its signal follows the snapshot, and a
    // native close, Escape or the backdrop, clears the snapshot.
    let open = RwSignal::new(ambre_chrome::Openness::Closed);
    Effect::new(move |_| open.set(ambre_chrome::Openness::from_bool(state.get().is_some())));

    let body = move || {
        state.get().map(|st| {
            // The selection's columns, offered as range-relative options
            // (value = index within the range, label = absolute A1 column).
            let cols: Vec<(Len<Col>, String)> = st
                .c1
                .through(st.c2)
                .into_iter()
                .map(|abs| (abs.len_from(st.c1), col_label(abs)))
                .collect();
            let cols = StoredValue::new(cols);

            view! {
                <div class="ambre-sort-dialog">
                    <div class="ambre-sort-title">
                        <span class="ambre-sort-heading">"Sort range"</span>
                        <button class="ambre-sort-close" aria-label="Close" on:click=move |_| close()>
                            <Icon icon=icondata::AiCloseOutlined />
                        </button>
                    </div>

                    <label class="ambre-sort-header-toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || has_header.get()
                            on:change=move |ev| has_header.set(event_target_checked(&ev))
                        />
                        <span>{ambre_commands::tr!("Data has header row", "Les donn\u{e9}es ont une ligne d'en-t\u{ea}te")}</span>
                    </label>

                    <div class="ambre-sort-levels">
                        {move || levels.get().into_iter().enumerate().map(|(i, key)| {
                            view! {
                                <div class="ambre-sort-level">
                                    <span class="ambre-sort-rank">
                                        {if i == 0 { "Sort by" } else { "then by" }}
                                    </span>
                                    <select
                                        class="ambre-sort-col"
                                        on:change=move |ev| {
                                            let v = Len::<Col>::new(event_target_value(&ev).parse::<u32>().unwrap_or(0));
                                            levels.update(|ls| {
                                                if let Some(l) = ls.get_mut(i) { l.col = v; }
                                            });
                                        }
                                    >
                                        {cols.get_value().into_iter().map(|(rel, label)| view! {
                                            <option value=rel.get().to_string() selected=rel == key.col>
                                                {format!("Column {label}")}
                                            </option>
                                        }).collect::<Vec<_>>()}
                                    </select>
                                    <select
                                        class="ambre-sort-dir"
                                        on:change=move |ev| {
                                            let order = if event_target_value(&ev) == "asc" {
                                                SortOrder::Ascending
                                            } else {
                                                SortOrder::Descending
                                            };
                                            levels.update(|ls| {
                                                if let Some(l) = ls.get_mut(i) { l.order = order; }
                                            });
                                        }
                                    >
                                        <option value="asc" selected=key.order == SortOrder::Ascending>"Ascending"</option>
                                        <option value="desc" selected=key.order == SortOrder::Descending>"Descending"</option>
                                    </select>
                                    <button
                                        class="ambre-sort-remove"
                                        aria-label=ambre_commands::tr!("Remove sort level", "Retirer le niveau de tri")
                                        // Keep at least one level.
                                        prop:disabled=move || levels.with(|ls| ls.len() <= 1)
                                        on:click=move |_| levels.update(|ls| {
                                            if ls.len() > 1 { ls.remove(i); }
                                        })
                                    >
                                        <Icon icon=icondata::AiDeleteOutlined />
                                    </button>
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>

                    <div class="ambre-sort-actions">
                        <button
                            class="ambre-sort-add"
                            // Cannot sort by more keys than the range has columns.
                            prop:disabled=move || levels.with(|ls| ls.len() >= cols.get_value().len())
                            on:click=move |_| levels.update(|ls| {
                                // New level defaults to the first not-yet-used column.
                                let used: Vec<Len<Col>> = ls.iter().map(|k| k.col).collect();
                                let next = cols.get_value().iter().map(|(rel, _)| *rel)
                                    .find(|rel| !used.contains(rel)).unwrap_or(Len::<Col>::ZERO);
                                ls.push(SortKey { col: next, order: SortOrder::Ascending });
                            })
                        >
                            <Icon icon=icondata::AiPlusOutlined />
                            <span>{ambre_commands::tr!("Add sort level", "Ajouter un niveau de tri")}</span>
                        </button>
                        <span class="ambre-sort-spacer"></span>
                        <button class="ambre-sort-cancel" on:click=move |_| close()>"Cancel"</button>
                        <button class="ambre-sort-apply" on:click=move |_| apply()>"Sort"</button>
                    </div>
                </div>
            }
        })
    };

    view! {
        <ambre_chrome::Dialog
            open=open
            class={ambre_core::StaticKey::<ambre_core::CssClasses>::new("ambre-sort-modal")}
            label=ambre_commands::tr!("Sort range", "Trier la plage")
            on_close=Callback::new(move |()| close())
        >
            {body}
        </ambre_chrome::Dialog>
    }
}
