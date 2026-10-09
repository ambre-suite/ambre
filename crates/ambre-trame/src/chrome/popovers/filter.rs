// ambre-trame/src/chrome/popovers/filter.rs
// The shared autofilter popover and its state.

use super::*;

/// Open state of the shared filter popover: which grid column it targets and the
/// viewport (client-space) anchor point under which it is drawn. `None` means
/// closed. The launcher derives the anchor from the active cell's published
/// screen position (the active cell is in the filtered column).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterPopoverState {
    /// Grid column (0-based) the filter targets.
    pub col: u32,
    /// Client-space anchor (CSS px): the popover's top-left.
    pub x: f64,
    pub y: f64,
}

/// The condition operators offered by the popover, paired with their labels.
/// The `<select>` index maps back to the [`FilterOp`] on apply.
const FILTER_OPS: &[(FilterOp, &str)] = &[
    (FilterOp::Eq, "equals"),
    (FilterOp::Ne, "not equals"),
    (FilterOp::Gt, "greater than"),
    (FilterOp::Lt, "less than"),
    (FilterOp::Gte, "greater or equal"),
    (FilterOp::Lte, "less or equal"),
    (FilterOp::Contains, "contains"),
    (FilterOp::NotContains, "does not contain"),
    (FilterOp::StartsWith, "starts with"),
    (FilterOp::IsEmpty, "is empty"),
    (FilterOp::IsNotEmpty, "is not empty"),
];

/// A1-style column label for a 0-based grid column index (0 -> "A", 26 -> "AA").
fn col_label(col: u32) -> String {
    crate::columns::col_to_alpha(col)
}

/// The shared filter popover (`trame-shared-filter`), anchored under the active
/// column header. It offers a "by values" mode (a checkbox list of the column's
/// distinct visible values) and a minimal "by condition" mode (operator + value).
/// Applying fires `on_apply` with the chosen [`FilterKind`]; "Clear filter"
/// routes through `on_command`, the shared command path. `values` is fed by the
/// launcher from [`ambre_bus::TrameEvent::ColumnValues`].
#[component]
pub fn FilterPopover(
    state: RwSignal<Option<FilterPopoverState>>,
    values: RwSignal<Vec<String>>,
    on_apply: Callback<FilterKind>,
    on_command: Callback<CommandId>,
) -> impl IntoView {
    // true = "by values", false = "by condition".
    let by_values = RwSignal::new(true);
    // The set of currently-checked (visible) values in the "by values" mode.
    let checked = RwSignal::new(std::collections::BTreeSet::<String>::new());
    // Whenever a fresh value list arrives (popover (re)opened on a column), check
    // every value by default -- the unfiltered state.
    Effect::new(move |_| {
        let vs = values.get();
        checked.set(vs.into_iter().collect());
    });
    // Condition-mode operator + operand.
    let op_idx = RwSignal::new(0usize);
    let cond_value = RwSignal::new(String::new());

    let close = move || state.set(None);

    let apply = move |_| {
        let kind = if by_values.get_untracked() {
            FilterKind::Values(checked.get_untracked().into_iter().collect())
        } else {
            let Some(&(op, _)) = FILTER_OPS.get(op_idx.get_untracked()).or(FILTER_OPS.last())
            else {
                return;
            };
            FilterKind::Condition {
                op,
                value: cond_value.get_untracked(),
            }
        };
        on_apply.run(kind);
        state.set(None);
    };

    move || {
        state.get().map(|st| {
            let style = format!(
                "position: fixed; left: {}px; top: {}px; z-index: 50;",
                st.x, st.y
            );
            view! {
                <div class="ambre-filter-popover" style=style>
                    <div class="ambre-filter-head">
                        {format!("Filter column {}", col_label(st.col))}
                    </div>
                    <div class="ambre-filter-modes">
                        <button
                            class="mode"
                            aria-pressed=move || by_values.get().then_some("true")
                            on:click=move |_| by_values.set(true)
                        >
                            "By values"
                        </button>
                        <button
                            class="mode"
                            aria-pressed=move || (!by_values.get()).then_some("true")
                            on:click=move |_| by_values.set(false)
                        >
                            "By condition"
                        </button>
                    </div>

                    <Show
                        when=move || by_values.get()
                        fallback=move || view! {
                            <div class="ambre-filter-cond">
                                <select on:change=move |ev| {
                                    op_idx.set(event_target_value(&ev).parse().unwrap_or(0));
                                }>
                                    {FILTER_OPS
                                        .iter()
                                        .enumerate()
                                        .map(|(i, (_, label))| {
                                            view! { <option value=i.to_string()>{label.to_string()}</option> }
                                        })
                                        .collect_view()}
                                </select>
                                <input
                                    type="text"
                                    placeholder="value"
                                    on:input=move |ev| cond_value.set(event_target_value(&ev))
                                />
                            </div>
                        }
                    >
                        <div class="ambre-filter-values">
                            <For
                                each=move || values.get()
                                key=|v| v.clone()
                                children=move |v: String| {
                                    let v_for_check = v.clone();
                                    let v_for_toggle = v.clone();
                                    let is_on = move || checked.get().contains(&v_for_check);
                                    view! {
                                        <label class="ambre-filter-value">
                                            <input
                                                type="checkbox"
                                                prop:checked=is_on
                                                on:change=move |ev| {
                                                    let on = event_target_checked(&ev);
                                                    checked.update(|set| {
                                                        if on {
                                                            set.insert(v_for_toggle.clone());
                                                        } else {
                                                            set.remove(&v_for_toggle);
                                                        }
                                                    });
                                                }
                                            />
                                            <span>{v}</span>
                                        </label>
                                    }
                                }
                            />
                        </div>
                    </Show>

                    <div class="ambre-filter-actions">
                        <button class="primary" on:click=apply>"Apply"</button>
                        <button on:click=move |_| {
                            on_command.run(CLEAR_FILTER);
                            close();
                        }>"Clear filter"</button>
                        <button on:click=move |_| close()>"Cancel"</button>
                    </div>
                </div>
            }
        })
    }
}
