// ambre-chrome/src/palette.rs
// ============================================================================
// The command palette (ambre-spec-chrome.md, CHROME-008).
//
// One overlay listing every command the active application DECLARES -- not the
// subset a menu happens to show. That distinction is the whole rule: a menu is
// a layout, and a command outside every layout is reachable only here.
//
// The palette is the first view that can borrow no label from a menu, so it
// reads the description a command carries (`ambre-spec-commands.md`,
// UI-THREAD-019) and nothing else. It is app-agnostic: it takes a command
// table and a state store, and names no application.
//
// The filter is a pure function, tested on the host; the component around it is
// the DOM half, tested under wasm.
// ============================================================================

use ambre_commands::{Command, CommandId, Locale};
use leptos::html;
use leptos::prelude::*;

use crate::shortcut::Kbd;

/// Whether `label` answers `query`: a case-insensitive substring match.
///
/// A substring and not a subsequence: "sr" must not offer "Sort range" beside
/// nine other rows. A user who types three letters expects the rows that
/// contain them, and that expectation is falsifiable in one assertion.
/// An empty query answers everything, so an opened palette lists the whole
/// table (`CHROME-008`).
pub fn matches(label: &str, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    label.to_lowercase().contains(&query.to_lowercase())
}

/// The commands of `table` whose label answers `query`, in declaration order.
///
/// Declaration order, never a relevance rank: the table is written in the order
/// an application groups its commands, and a rank that reorders rows under the
/// user's fingers as they type costs more than it buys.
pub fn filter<'a>(table: &'a [Command], query: &str, locale: &Locale) -> Vec<&'a Command> {
    table
        .iter()
        .filter(|c| matches(&(c.title)(locale), query))
        .collect()
}

/// The command palette overlay.
///
/// `table` is the active application's whole command table. Selecting a row
/// invokes it through `on_command`, the same sink every other trigger uses, and
/// closes the overlay.
///
/// Every row reads enabled: the command store is `Rc` and stays in the mount
/// owner, out of the `Send` router subtree, so this view cannot subscribe to it.
/// The palette therefore offers Undo on an empty history
/// (`ambre-spec-chrome.md` section 5).
#[component]
pub fn CommandPalette(
    /// Visibility, driven by the launcher when the palette command is invoked.
    open: RwSignal<bool>,
    /// Every command the active application declares (`CHROME-008`).
    table: &'static [Command],
    on_command: Callback<CommandId>,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let focused = RwSignal::new(0usize);
    let input_ref = NodeRef::<html::Input>::new();

    // Each open starts from a blank query on the first row, and takes the
    // caret: a palette that reopens on the last search is a palette that runs
    // the wrong command on the next Enter.
    Effect::new(move |_| {
        if open.get() {
            query.set(String::new());
            focused.set(0);
            if let Some(el) = input_ref.get() {
                let _ = el.focus();
            }
        }
    });

    let rows = Memo::new(move |_| {
        filter(table, &query.get(), &Locale::En)
            .into_iter()
            .map(|c| (c.id, (c.title)(&Locale::En), c.chord()))
            .collect::<Vec<_>>()
    });

    let run = move |id: CommandId| {
        open.set(false);
        on_command.run(id);
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        let len = rows.get_untracked().len();
        match ev.key().as_str() {
            "ArrowDown" => {
                ev.prevent_default();
                focused.update(|i| *i = (*i + 1) % len.max(1));
            }
            "ArrowUp" => {
                ev.prevent_default();
                focused.update(|i| *i = (*i + len.saturating_sub(1)) % len.max(1));
            }
            "Enter" => {
                ev.prevent_default();
                if let Some((id, _, _)) = rows.get_untracked().get(focused.get_untracked()) {
                    run(*id);
                }
            }
            "Escape" => open.set(false),
            _ => {}
        }
    };

    view! {
        <Show when=move || open.get() fallback=|| ()>
            <div
                class="ambre-palette-backdrop"
                on:click=move |_| open.set(false)
            ></div>
            <div class="ambre-palette" role="dialog" aria-label="Command palette">
                <input
                    node_ref=input_ref
                    class="ambre-palette-query"
                    type="text"
                    role="combobox"
                    aria-label="Search commands"
                    aria-expanded="true"
                    autocomplete="off"
                    prop:value=move || query.get()
                    on:input=move |ev| {
                        query.set(event_target_value(&ev));
                        focused.set(0);
                    }
                    on:keydown=on_keydown
                />
                <ul class="ambre-palette-list" role="listbox">
                    {move || {
                        rows.get()
                            .into_iter()
                            .enumerate()
                            .map(|(i, (id, label, chord))| {
                                view! {
                                    <li
                                        class="ambre-palette-row"
                                        class:focused=move || focused.get() == i
                                        role="option"
                                        aria-selected=move || {
                                            (focused.get() == i).then_some("true")
                                        }
                                        on:mouseenter=move |_| focused.set(i)
                                        on:click=move |_| run(id)
                                    >
                                        <span class="ambre-palette-label">{label}</span>
                                        {chord
                                            .map(|kb| {
                                                view! {
                                                    <span class="ambre-palette-kbd">
                                                        <Kbd binding=kb />
                                                    </span>
                                                }
                                            })}
                                    </li>
                                }
                            })
                            .collect_view()
                    }}
                </ul>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_commands::{CommandCategory, IconId, KeyBinding};

    fn bold(_: &Locale) -> String {
        "Bold".into()
    }
    fn sort_asc(_: &Locale) -> String {
        "Sort range ascending".into()
    }
    fn export(_: &Locale) -> String {
        "Export CSV".into()
    }

    const TABLE: &[Command] = &[
        Command {
            id: CommandId("t.format.bold"),
            title: bold,
            icon: Some(IconId("format-bold")),
            keybindings: &[KeyBinding::primary('b')],
            category: CommandCategory("format"),
        },
        Command {
            id: CommandId("t.sort.asc"),
            title: sort_asc,
            icon: None,
            keybindings: &[],
            category: CommandCategory("sort"),
        },
        Command {
            id: CommandId("t.interop.export"),
            title: export,
            icon: None,
            keybindings: &[],
            category: CommandCategory("interop"),
        },
    ];

    /// An opened palette lists the whole table: every command the application
    /// declares, and not a subset (`CHROME-008`).
    #[test]
    fn an_empty_query_lists_every_declared_command() {
        assert_eq!(filter(TABLE, "", &Locale::En).len(), TABLE.len());
        // Whitespace is not a query either.
        assert_eq!(filter(TABLE, "   ", &Locale::En).len(), TABLE.len());
    }

    #[test]
    fn a_query_matches_any_case_and_any_position() {
        let hits = filter(TABLE, "RANGE", &Locale::En);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, CommandId("t.sort.asc"));
        assert!(matches("Export CSV", "csv"));
        assert!(matches("Export CSV", "Export"));
    }

    #[test]
    fn a_query_that_answers_nothing_lists_nothing() {
        assert!(filter(TABLE, "zzz", &Locale::En).is_empty());
    }

    /// Rows keep the table's order, so they do not move under the user as the
    /// query narrows.
    #[test]
    fn rows_keep_the_declaration_order() {
        let ids: Vec<CommandId> = filter(TABLE, "o", &Locale::En)
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                CommandId("t.format.bold"),
                CommandId("t.sort.asc"),
                CommandId("t.interop.export"),
            ]
        );
    }
}
