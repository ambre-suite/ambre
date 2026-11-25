// ambre-app/src/shell/header.rs
// ============================================================================
// The rows the two header menus take from the command tables
// (`ambre-spec-shell.md` section 2). The Ambre menu holds what acts on the
// document as a file (`CHROME-154`). The avatar menu holds what acts on the
// person (`CHROME-156`). Each row reads its words and its chord from the
// description of its command, so no row carries a word of its own
// (`UI-THREAD-036`).
// ============================================================================

use super::super::*;
use ambre_chrome::commands::{Command, Home, Locale};
use ambre_core::{Commands, StaticKey};

/// The commands of `table` that `region` homes, in the order of the table.
/// `skip` names a command the menu already shows in a row of its own, such
/// as the palette of the Ambre menu.
pub(crate) fn homed_rows(
    table: &'static [Command],
    region: Home,
    skip: Option<StaticKey<Commands>>,
) -> Vec<&'static Command> {
    table
        .iter()
        .filter(|c| c.home == region && Some(c.id) != skip)
        .collect()
}

/// The rows of a header menu for each command `region` homes. A row is a
/// `menuitem` that names its command, shows its chord, and runs it through
/// the one command path. A row the page holds unavailable is disabled, such
/// as an undo with nothing to undo (`UNDO-006`).
#[component]
pub(crate) fn HomedRows(
    /// The table of the booted application.
    table: &'static [Command],
    /// The region whose commands the rows show.
    region: Home,
    /// A command the menu shows in a row of its own.
    #[prop(optional_no_strip)]
    skip: Option<StaticKey<Commands>>,
    /// The one command path of the shell.
    on_command: Callback<StaticKey<Commands>>,
    /// The commands the page holds unavailable.
    #[prop(optional, into)]
    disabled: Option<Signal<Vec<StaticKey<Commands>>>>,
) -> impl IntoView {
    homed_rows(table, region, skip)
        .into_iter()
        .map(|command| {
            let id = command.id;
            let label = (command.title)(&Locale::En).into_string();
            let off = move || disabled.is_some_and(|d| d.with(|ids| ids.contains(&id)));
            view! {
                <button
                    role="menuitem"
                    class="ambre-menu-item ambre-menu-command"
                    data-reach=ambre_chrome::commands::Reach::Command(id).marker()
                    aria-keyshortcuts=command.chord().map(|c| c.to_string())
                    disabled=off
                    on:click=move |_| on_command.run(id)
                >
                    <span>{label}</span>
                    {command.chord().map(|c| view! { <ambre_chrome::Kbd binding=c /> })}
                </button>
            }
        })
        .collect_view()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// The table of each document page that lays out the shared header.
    fn tables() -> [&'static [Command]; 3] {
        [
            ambre_prose::commands::COMMANDS,
            trame_chrome::COMMANDS,
            ambre_toile::ui::commands::COMMANDS,
        ]
    }

    /// In each application, the Ambre menu shows every command its page
    /// homes there, every save and export among them. The palette keeps its
    /// own row, so no row repeats it (`CHROME-154`).
    #[wasm_bindgen_test]
    fn the_ambre_menu_shows_every_command_its_page_homes_there() {
        let on_file = |c: &&Command| c.category == ambre_chrome::commands::shared::FILE;
        for table in tables() {
            let palette = table
                .iter()
                .find(|c| {
                    c.keybindings
                        .contains(&ambre_chrome::commands::shared::PALETTE_CHORD)
                })
                .map(|c| c.id);
            assert!(palette.is_some(), "each table declares a palette");
            let rows = homed_rows(table, Home::AmbreMenu, palette);
            assert!(rows.iter().all(|c| c.home == Home::AmbreMenu));
            assert!(rows.iter().all(|c| Some(c.id) != palette));
            let file = table.iter().filter(|c| on_file(c)).count();
            assert!(file > 0, "each table files a save or an export");
            assert_eq!(
                rows.iter().filter(|c| on_file(c)).count(),
                file,
                "every command on the file is a row of the Ambre menu"
            );
        }
    }

    /// The avatar menu of a board shows its pointer modes, and no command
    /// another region homes (`CHROME-156`).
    #[wasm_bindgen_test]
    fn the_avatar_menu_shows_the_settings_of_the_person() {
        let rows = homed_rows(ambre_toile::ui::commands::COMMANDS, Home::AvatarMenu, None);
        let ids: Vec<_> = rows.iter().map(|c| c.id).collect();
        assert!(ids.contains(&ambre_toile::ui::menu_bar::POINTER_MOUSE));
        assert!(rows.iter().all(|c| c.home == Home::AvatarMenu));
    }
}
