// ambre-chrome/src/creation.rs
// ============================================================================
// The creation bar: one bar of the commands that add something to the
// document. It is the same in Prose, Trame and Toile (`ambre-spec-shell.md`
// section 2, `CHROME-152`, `CHROME-153`). The bar shows every command its
// table homes in the creation bar, in the order of the table, and no other.
// It floats centred at the bottom of the content cell, and publishes the box
// it covers (`CHROME-171`).
//
// One component serves the three applications. It takes the table and a
// resolver of icons, never a type of an application, so it compiles once.
// ============================================================================

use ambre_client_core::cover::{Cover, Surface};
use ambre_commands::{Command, Home};
use ambre_core::{Commands, StaticKey};
use leptos::html;
use leptos::prelude::*;

/// The commands `table` homes in the creation bar, in the order of the table.
#[must_use]
pub fn creation_entries(table: &'static [Command]) -> Vec<&'static Command> {
    table
        .iter()
        .filter(|c| c.home == Home::CreationBar)
        .collect()
}

/// A view an application draws in place of the plain button of one command.
/// The board draws the colour picker of its sticky note this way. The
/// view carries the reach of that command, so the bar still names it.
#[derive(Clone)]
pub struct CreationSlot {
    /// The command the view stands for.
    pub command: StaticKey<Commands>,
    /// The view the bar draws in its place.
    pub view: ViewFn,
}

/// The creation bar of a page. `table` is the whole table of the booted
/// application, and the bar keeps its creation entries alone. `resolve` gives
/// the glyph of a command. `armed` names the tool the next press takes, which
/// reads pressed. A rule separates two groups of the table. A table that homes
/// nothing in the bar lays out no bar, and covers nothing.
#[component]
pub fn CreationBar(
    table: &'static [Command],
    resolve: fn(&Command) -> Option<icondata::Icon>,
    on_command: Callback<StaticKey<Commands>>,
    cover: RwSignal<Cover>,
    #[prop(optional, into)] armed: Option<Signal<Option<StaticKey<Commands>>>>,
    #[prop(optional)] slots: Vec<CreationSlot>,
) -> impl IntoView {
    let entries = creation_entries(table);
    let node = NodeRef::<html::Div>::new();
    crate::cover::watch_fixed(node, Surface::CreationBar, cover);
    let first = entries.first().map(|c| c.category);
    let buttons = entries
        .iter()
        .scan(first, |last, &command| {
            let rule =
                (*last != Some(command.category)).then(|| view! { <span class="sep"></span> });
            *last = Some(command.category);
            let id = command.id;
            let slot = slots
                .iter()
                .find(|s| s.command == id)
                .map(|s| s.view.clone());
            let button = match slot {
                Some(view) => view.run(),
                None => match armed {
                    Some(armed) => {
                        let pressed = Signal::derive(move || {
                            crate::Pressed::from_bool(armed.get() == Some(id))
                        });
                        view! {
                            <crate::CommandButton
                                command=command
                                icon=resolve(command)
                                on_command=on_command
                                pressed=pressed
                                size=crate::ControlSize::Large
                                look=crate::IconLook::Ghost
                            />
                        }
                        .into_any()
                    }
                    None => view! {
                        <crate::CommandButton
                            command=command
                            icon=resolve(command)
                            on_command=on_command
                            size=crate::ControlSize::Large
                            look=crate::IconLook::Ghost
                        />
                    }
                    .into_any(),
                },
            };
            Some(view! { {rule} {button} })
        })
        .collect_view();
    (!entries.is_empty()).then(|| {
        view! {
            <div class="ambre-creation-bar" role="toolbar" aria-label="Creation bar" node_ref=node>
                {buttons}
            </div>
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_commands::Locale;
    use ambre_core::Categories;

    fn title(_: &Locale) -> ambre_core::Validated<ambre_commands::CommandTitle> {
        ambre_core::Validated::new("Entry")
    }

    const fn entry(id: &'static str, home: Home) -> Command {
        Command {
            id: StaticKey::<Commands>::new(id),
            title,
            icon: None,
            keybindings: &[],
            category: StaticKey::<Categories>::new("t"),
            home,
            description: None,
        }
    }

    static TABLE: [Command; 4] = [
        entry("t.picture", Home::CreationBar),
        entry("t.bold", Home::ContextPanel),
        entry("t.save", Home::AmbreMenu),
        entry("t.table", Home::CreationBar),
    ];

    /// `CHROME-152`: the bar keeps every command its table homes there, in
    /// the order of the table, and no other.
    #[test]
    fn the_bar_keeps_the_creation_entries_of_its_table_alone() {
        let ids: Vec<&str> = creation_entries(&TABLE)
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(ids, ["t.picture", "t.table"]);
    }
}
