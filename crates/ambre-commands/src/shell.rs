// ambre-commands/src/shell.rs
// ============================================================================
// The commands of the shell itself (`ambre-spec-shell.md` section 2). The
// Ambre menu holds the way home and the share, and the avatar menu holds the
// preferences. No application declares them, since every page of the suite offers
// them. The header and its two menus read their words here, as a toolbar
// reads an application's table (`UI-THREAD-019`).
// ============================================================================

use ambre_core::{Categories, Commands, Icons, StaticKey, Validated};

use crate::{Command, CommandDescription, CommandTitle, Home, Locale, shared};

/// Leave the open document for the list of every document: the first entry
/// of the Ambre menu (`CHROME-155`).
pub const HOME: StaticKey<Commands> = StaticKey::<Commands>::new("shell.file.home");
/// Open the share dialog of the open document (`CHROME-154`).
pub const SHARE: StaticKey<Commands> = StaticKey::<Commands>::new("shell.file.share");
/// Switch between the light and the dark theme (`CHROME-156`).
pub const THEME: StaticKey<Commands> = StaticKey::<Commands>::new("shell.account.theme");
/// Dock the context panel on the other side (`CHROME-156`, `CHROME-194`).
pub const PANEL_SIDE: StaticKey<Commands> = StaticKey::<Commands>::new("shell.account.panel_side");

/// The group of the way home: it leaves the document, and acts on no file.
const NAVIGATE: StaticKey<Categories> = StaticKey::<Categories>::new("navigate");

// One language ships today and nothing supplies an active locale, so each
// label is the same for both variants (`ambre-spec-commands.md` section 5).

fn home_title(_: &Locale) -> Validated<CommandTitle> {
    Validated::new("All documents")
}

fn share_title(_: &Locale) -> Validated<CommandTitle> {
    Validated::new("Share document")
}

fn share_description(_: &Locale) -> Validated<CommandDescription> {
    Validated::new("Invite a member, or copy the link of an invite")
}

fn theme_title(_: &Locale) -> Validated<CommandTitle> {
    Validated::new("Switch the colour theme")
}

fn panel_side_title(_: &Locale) -> Validated<CommandTitle> {
    Validated::new("Dock the panel on the other side")
}

/// Every command of the shell, described once.
pub const COMMANDS: &[Command] = &[
    Command {
        id: HOME,
        title: home_title,
        icon: None,
        keybindings: &[],
        category: NAVIGATE,
        home: Home::AmbreMenu,
        description: None,
    },
    Command {
        id: SHARE,
        title: share_title,
        icon: Some(StaticKey::<Icons>::new("share-variant-outline")),
        keybindings: &[],
        category: shared::FILE,
        home: Home::AmbreMenu,
        description: Some(share_description),
    },
    Command {
        id: THEME,
        title: theme_title,
        icon: None,
        keybindings: &[],
        category: shared::ACCOUNT,
        home: Home::AvatarMenu,
        description: None,
    },
    Command {
        id: PANEL_SIDE,
        title: panel_side_title,
        icon: None,
        keybindings: &[],
        category: shared::ACCOUNT,
        home: Home::AvatarMenu,
        description: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The table of the shell is whole, and each command lives where its
    /// subject names. The share lives in the Ambre menu, and the two
    /// preferences in the avatar menu (`CHROME-154`, `CHROME-156`).
    #[test]
    fn the_shell_homes_the_file_in_the_ambre_menu_and_the_person_in_the_avatar_menu() {
        assert_eq!(crate::table_defects(COMMANDS), []);
        let home_of = |id| crate::find(COMMANDS, id).map(|c| c.home);
        assert_eq!(home_of(HOME), Some(Home::AmbreMenu));
        assert_eq!(home_of(SHARE), Some(Home::AmbreMenu));
        assert_eq!(home_of(THEME), Some(Home::AvatarMenu));
        assert_eq!(home_of(PANEL_SIDE), Some(Home::AvatarMenu));
    }
}
