// ambre-chrome/src/palette.rs
// ============================================================================
// The command palette (ambre-spec-chrome.md, CHROME-008).
//
// One overlay listing every command the active application DECLARES -- not the
// subset a menu happens to show. That distinction is the whole rule: a menu is
// a layout, and a command outside every layout is reachable only here.
//
// The palette is the first view that can borrow no label from a menu. It reads
// the description a command carries (`ambre-spec-commands.md`, UI-THREAD-019),
// and nothing else. It is app-agnostic: it takes a command table and a state
// store, and names no application.
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
/// Declaration order, never a relevance rank. The table is written in the order
/// an application groups its commands. A rank that reorders rows under the
/// user's fingers as they type costs more than it buys.
pub fn filter<'a>(table: &'a [Command], query: &str, locale: &Locale) -> Vec<&'a Command> {
    table
        .iter()
        .filter(|c| matches(&(c.title)(locale), query))
        .collect()
}

/// What a palette entry names: a channel to open, or a member to write to.
/// The palette hands it back when the entry is chosen. A list that changed
/// meanwhile picks no other, and a member is never read as a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaletteKey {
    /// A channel, or a direct message.
    Channel(ambre_core::ChannelId),
    /// A member of a space.
    Member(ambre_core::UserId),
}

/// One entry an application supplies to the palette, such as a channel to
/// open. The entries come before the commands (`CHROME-093`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    /// What the entry names, handed back when it is chosen.
    pub key: PaletteKey,
    /// What the row reads, and what the query matches.
    pub label: String,
    /// A word beside the label, such as the section of the entry.
    pub hint: Option<&'static str>,
}

/// One row of the palette: an entry of the application, or a command.
#[derive(Clone, Copy)]
pub enum PaletteRow<'a> {
    Entry(&'a PaletteEntry),
    Command(&'a Command),
}

/// The rows `query` lists: the entries whose label answers it, then the
/// commands (`CHROME-093`). A query that starts with `>` lists the commands
/// alone, matched against what follows it (`CHROME-094`).
pub fn rows<'a>(
    entries: &'a [PaletteEntry],
    table: &'a [Command],
    query: &str,
    locale: &Locale,
) -> Vec<PaletteRow<'a>> {
    let (entries, query) = match query.trim_start().strip_prefix('>') {
        Some(rest) => (&[][..], rest),
        None => (entries, query),
    };
    entries
        .iter()
        .filter(|e| matches(&e.label, query))
        .map(PaletteRow::Entry)
        .chain(
            filter(table, query, locale)
                .into_iter()
                .map(PaletteRow::Command),
        )
        .collect()
}

/// What a drawn row runs once chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chosen {
    Entry(PaletteKey),
    Command(CommandId),
}

/// The command palette overlay.
///
/// `table` is the active application's whole command table. Selecting a row
/// invokes it through `on_command`, the same sink every other trigger uses, and
/// closes the overlay. An application may supply `entries` too, listed first,
/// and `on_entry` receives the key of the one chosen.
///
/// Every row reads enabled. The command store is `Rc` and stays in the mount
/// owner, out of the `Send` router subtree, so this view cannot subscribe to it.
/// The palette therefore offers Undo on an empty history (`ambre-spec-chrome.md`
/// section 5).
#[component]
pub fn CommandPalette(
    /// Visibility, driven by the launcher when the palette command is invoked.
    open: RwSignal<bool>,
    /// Every command the active application declares (`CHROME-008`).
    table: &'static [Command],
    on_command: Callback<CommandId>,
    /// The entries of the application, listed before the commands.
    #[prop(optional, into)]
    entries: Option<Signal<Vec<PaletteEntry>>>,
    /// Receives the key of the entry chosen.
    #[prop(optional)]
    on_entry: Option<Callback<PaletteKey>>,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let focused = RwSignal::new(0usize);
    let input_ref = NodeRef::<html::Input>::new();

    // Each open starts from a blank query on the first row, and takes the
    // caret. A palette that reopens on the last search is a palette that runs
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
        let entries = entries.map(|e| e.get()).unwrap_or_default();
        self::rows(&entries, table, &query.get(), &Locale::En)
            .into_iter()
            .map(|row| match row {
                PaletteRow::Entry(e) => (Chosen::Entry(e.key), e.label.clone(), e.hint, None),
                PaletteRow::Command(c) => (
                    Chosen::Command(c.id),
                    (c.title)(&Locale::En),
                    None,
                    c.chord(),
                ),
            })
            .collect::<Vec<_>>()
    });

    let run = move |chosen: Chosen| {
        open.set(false);
        match chosen {
            Chosen::Command(id) => on_command.run(id),
            Chosen::Entry(key) => {
                if let Some(on_entry) = on_entry {
                    on_entry.run(key);
                }
            }
        }
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
                if let Some((chosen, ..)) = rows.get_untracked().get(focused.get_untracked()) {
                    run(*chosen);
                }
            }
            "Escape" => open.set(false),
            _ => {}
        }
    };

    view! {
        // The shared modal traps the focus and leaves the page inert
        // (`CHROME-028`, `CHROME-068`); its backdrop and Escape close it.
        <crate::dialog::Dialog open=open class="ambre-palette-modal" label="Command palette">
            <div class="ambre-palette">
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
                            .map(|(i, (chosen, label, hint, chord))| {
                                view! {
                                    <li
                                        class="ambre-palette-row"
                                        class:focused=move || focused.get() == i
                                        role="option"
                                        aria-selected=move || {
                                            crate::menu::aria_bool(focused.get() == i)
                                        }
                                        on:mouseenter=move |_| focused.set(i)
                                        on:click=move |_| run(chosen)
                                    >
                                        <span class="ambre-palette-label">{label}</span>
                                        {hint
                                            .map(|hint| {
                                                view! {
                                                    <span class="ambre-palette-hint">{hint}</span>
                                                }
                                            })}
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
        </crate::dialog::Dialog>
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

    fn entry(key: u8, label: &str) -> PaletteEntry {
        PaletteEntry {
            key: PaletteKey::Channel(ambre_core::ChannelId::from_bytes([key; 16])),
            label: label.into(),
            hint: None,
        }
    }

    /// Each row as the label of its entry or the id of its command.
    fn shown(entries: &[PaletteEntry], query: &str) -> Vec<String> {
        rows(entries, TABLE, query, &Locale::En)
            .into_iter()
            .map(|row| match row {
                PaletteRow::Entry(e) => e.label.clone(),
                PaletteRow::Command(c) => c.id.0.to_owned(),
            })
            .collect()
    }

    /// The entries an application supplies come before its commands, each
    /// side filtered by the one query.
    #[test]
    fn the_entries_of_the_application_come_before_the_commands() {
        let entries = [entry(1, "budget"), entry(2, "general"), entry(3, "Bob")];
        assert_eq!(shown(&entries, "b"), ["budget", "Bob", "t.format.bold"]);
        // An application that supplies none sees the commands alone.
        assert_eq!(shown(&[], "").len(), TABLE.len());
    }

    /// A query that starts with `>` drops the entries, and what follows it
    /// filters the commands.
    #[test]
    fn a_query_that_starts_with_a_chevron_lists_the_commands_alone() {
        let entries = [entry(1, "export-notes"), entry(2, "Bold ideas")];
        assert_eq!(
            shown(&entries, ">"),
            ["t.format.bold", "t.sort.asc", "t.interop.export"]
        );
        assert_eq!(shown(&entries, " > export"), ["t.interop.export"]);
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
