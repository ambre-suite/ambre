// ambre-commands/src/lib.rs
// ============================================================================
// Shared command registry for the Ambre UI thread.
//
// Menus, toolbar buttons, keyboard shortcuts, context menus and the command
// palette are all *views* of one registry of `Command`s. This keeps the chrome
// tractable across 5 apps x ~200 commands (ambre-spec-commands.md).
//
// Layering: a `Command` here, named by an `ambre_core::StaticKey<Commands>`,
// is a UI *affordance*. It is distinct
// from the `AppCommand`/`LogicCommand` IPC payloads in `ambre-bus`: invoking a
// `Command` *produces* a `LogicCommand`. This crate never knows about IPC, the
// renderer, or Leptos -- it is the leaf of the UI-thread crate graph.
// ============================================================================

mod keybinding;
mod mode;

pub use keybinding::{FunctionKey, Key, KeyBinding, KeyChar, KeyState, ModifierFold, ModifierKeys};
pub use mode::{ClipboardWrite, PasteForm, SelectionMode};

use ambre_core::{Categories, Commands, Icons, StaticKey};

use std::collections::HashMap;
use std::hash::Hash;

/// Locale selector for command labels. Minimal today (the two shipping UI
/// languages); extend as i18n grows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Locale {
    #[default]
    Fr,
    En,
}

/// Static description of a command. Apps build `const` tables of these and
/// expose them through [`CommandSource`].
pub struct Command {
    pub id: StaticKey<Commands>,
    /// Localized label. A function (not a string) so labels stay i18n-driven
    /// without the registry owning a translation table.
    pub title: fn(&Locale) -> String,
    pub icon: Option<StaticKey<Icons>>,
    /// Every chord the command answers, the announced one first. A slice and
    /// not one chord, because a command may answer two. Redo takes
    /// Ctrl+Shift+Z and Ctrl+Y, and each clipboard intent takes a modern chord
    /// and a legacy one. Empty for a command no chord triggers.
    pub keybindings: &'static [KeyBinding],
    pub category: StaticKey<Categories>,
}

impl Command {
    /// The chord a surface announces beside the command: the first it
    /// declares. `None` for a command no chord triggers.
    pub fn chord(&self) -> Option<KeyBinding> {
        self.keybindings.first().copied()
    }
}

/// What every application shares, decided once: the clipboard identities and
/// the chords the universal operations answer (`ambre-spec-commands.md`,
/// `UI-THREAD-011`).
pub mod shared {
    use super::{Key, KeyBinding, KeyChar, ModifierKeys};
    use ambre_core::{Commands, StaticKey};

    /// The clipboard identities. One intent, one identity, in every
    /// application: a menu entry, a toolbar button and the browser's own event
    /// name the same command. A paste is one operation whichever raised it.
    /// They carry no application prefix because they belong to no application.
    pub const CUT: StaticKey<Commands> = StaticKey::<Commands>::new("clipboard.cut");
    pub const COPY: StaticKey<Commands> = StaticKey::<Commands>::new("clipboard.copy");
    pub const PASTE: StaticKey<Commands> = StaticKey::<Commands>::new("clipboard.paste");
    /// Paste the plain text alone, whatever richer half the clipboard holds.
    pub const PASTE_PLAIN: StaticKey<Commands> =
        StaticKey::<Commands>::new("clipboard.paste_plain");

    /// The identities the browser services itself. It raises `cut`, `copy` and
    /// `paste` with the clipboard attached, on the modern and the legacy chords
    /// alike. A keymap therefore declares those chords and decodes none of them
    /// (`ambre-spec-text-input.md`, `CHROME-021`).
    /// The plain paste rides the same `paste` event. The host notes its chord
    /// on the keydown, then strips the richer halves from the event.
    pub const BROWSER_SERVICED: [StaticKey<Commands>; 4] = [CUT, COPY, PASTE, PASTE_PLAIN];

    /// Undo answers one chord.
    pub const UNDO_CHORDS: &[KeyBinding] = &[KeyBinding::primary(KeyChar::Z)];

    /// Redo answers two: Ctrl+Shift+Z, and Ctrl+Y after Excel and Sheets. Both
    /// reach the same identity, in every application.
    pub const REDO_CHORDS: &[KeyBinding] = &[
        KeyBinding::primary_shift(KeyChar::Z),
        KeyBinding::primary(KeyChar::Y),
    ];

    /// Cut: the modern chord, then the legacy one.
    pub const CUT_CHORDS: &[KeyBinding] = &[
        KeyBinding::primary(KeyChar::X),
        KeyBinding::shift(Key::Delete),
    ];

    /// Copy: the modern chord, then the legacy one.
    pub const COPY_CHORDS: &[KeyBinding] = &[
        KeyBinding::primary(KeyChar::C),
        KeyBinding::new(ModifierKeys::PRIMARY, Key::Insert),
    ];

    /// Paste: the modern chord, then the legacy one.
    pub const PASTE_CHORDS: &[KeyBinding] = &[
        KeyBinding::primary(KeyChar::V),
        KeyBinding::shift(Key::Insert),
    ];

    /// Paste as plain text: the chord Chrome, Edge, Firefox, Word and Google
    /// Docs share.
    pub const PASTE_PLAIN_CHORDS: &[KeyBinding] = &[KeyBinding::primary_shift(KeyChar::V)];
}

/// Runtime state of a command, derived from app state on each interrogation
/// (never stored on the `Command`). Drives enable/visibility/toggle in every
/// view of the registry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CommandState {
    pub enabled: ambre_core::Availability,
    pub visible: Listing,
    pub toggled: ambre_core::Toggle,
}

ambre_core::define_flag! {
    /// Whether the views of the registry list a command now.
    pub enum Listing {
        /// No view lists the command.
        Hidden,
        /// The menus, the palette and the toolbars list the command.
        Listed,
    }
}

impl Default for CommandState {
    /// Enabled and listed, not toggled -- the neutral default for a command
    /// with no contextual constraints.
    fn default() -> Self {
        Self {
            enabled: ambre_core::Availability::Enabled,
            visible: Listing::Listed,
            toggled: ambre_core::Toggle::Off,
        }
    }
}

/// An owner of commands. Each app implements this. The chrome consumes it
/// without knowing which app it is (the same dependency inversion as the
/// `AppView` enum on the render side).
///
/// `Context` is the app's own state handle. Keeping it an associated type
/// rather than a concrete `AppContext` preserves this crate as a true leaf.
/// The app, not `ambre-commands`, decides what state derivation needs.
pub trait CommandSource {
    type Context;

    /// The full, static command table.
    fn commands(&self) -> &[Command];

    /// Derive a command's runtime state from current app context.
    fn state(&self, id: StaticKey<Commands>, ctx: &Self::Context) -> CommandState;

    /// Invoke a command against app context (typically: emit a `LogicCommand`).
    fn invoke(&self, id: StaticKey<Commands>, ctx: &mut Self::Context);
}

/// Look up a command by id in a table. Linear scan: tables are small and this
/// runs off the hot path (palette open, menu build), so a map would only add
/// allocation.
pub fn find(commands: &[Command], id: StaticKey<Commands>) -> Option<&Command> {
    commands.iter().find(|c| c.id == id)
}

/// What makes a command table unfit to describe its commands
/// (`ambre-spec-commands.md`, `UI-THREAD-019`, `UI-THREAD-037`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TableDefect {
    /// The table describes the identity twice, so `find` reaches one of them.
    DescribedTwice(StaticKey<Commands>),
    /// The command has no label in a locale, so a view has nothing to show.
    Unlabelled(StaticKey<Commands>, Locale),
    /// The command belongs to no group, so the palette cannot place it.
    Ungrouped(StaticKey<Commands>),
    /// The command names an icon with no name, which no icon set resolves.
    BlankIcon(StaticKey<Commands>),
    /// Two entries declare one default chord. Seeding keeps the last one, so
    /// the first silently leaves the keyboard.
    SharedChord(KeyBinding, StaticKey<Commands>, StaticKey<Commands>),
}

/// Every defect of `table`, in declaration order. An empty answer is a table
/// whose every command carries one whole description.
///
/// An icon and a chord are optional: a command may have neither. A command
/// that declares one must declare a usable one.
pub fn table_defects(table: &[Command]) -> Vec<TableDefect> {
    let mut defects = Vec::new();
    let mut chords: HashMap<KeyBinding, StaticKey<Commands>> = HashMap::new();
    for (i, cmd) in table.iter().enumerate() {
        if table.iter().take(i).any(|c| c.id == cmd.id) {
            defects.push(TableDefect::DescribedTwice(cmd.id));
        }
        for locale in [Locale::Fr, Locale::En] {
            if (cmd.title)(&locale).trim().is_empty() {
                defects.push(TableDefect::Unlabelled(cmd.id, locale));
            }
        }
        if cmd.category.as_str().trim().is_empty() {
            defects.push(TableDefect::Ungrouped(cmd.id));
        }
        if cmd.icon.is_some_and(|icon| icon.as_str().trim().is_empty()) {
            defects.push(TableDefect::BlankIcon(cmd.id));
        }
        for &kb in cmd.keybindings {
            if let Some(first) = chords.insert(kb, cmd.id) {
                defects.push(TableDefect::SharedChord(kb, first, cmd.id));
            }
        }
    }
    defects
}

/// Resolves keyboard chords to commands: a `HashMap<KeyBinding, StaticKey<Commands>>`.
///
/// Seeded from each command's declared chords; the chrome may later rebind
/// entries from user preferences via [`Keymap::bind`].
#[derive(Default)]
pub struct Keymap {
    map: HashMap<KeyBinding, StaticKey<Commands>>,
}

impl Keymap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from one or more command tables. Later tables override earlier
    /// ones on a chord collision; same-table collisions keep the last seen.
    pub fn from_commands<'a>(tables: impl IntoIterator<Item = &'a [Command]>) -> Self {
        let mut km = Self::new();
        for table in tables {
            km.bind_table(table);
        }
        km
    }

    /// Register every chord a table declares, except the chords of the
    /// identities the browser services itself. The table announces those and
    /// the keymap never resolves them. No path decodes a copy twice
    /// (`ambre-spec-commands.md`, `UI-THREAD-011`).
    ///
    /// The one seeding path an application calls, so the exception holds
    /// wherever a table is loaded.
    pub fn bind_table(&mut self, commands: &[Command]) {
        for cmd in commands
            .iter()
            .filter(|c| !shared::BROWSER_SERVICED.contains(&c.id))
        {
            for &kb in cmd.keybindings {
                self.map.insert(kb, cmd.id);
            }
        }
    }

    /// Bind (or rebind) a chord to a command. Returns the command it displaced,
    /// if any -- useful to surface shortcut conflicts to the user.
    pub fn bind(&mut self, kb: KeyBinding, id: StaticKey<Commands>) -> Option<StaticKey<Commands>> {
        self.map.insert(kb, id)
    }

    /// Remove a binding, returning the command it pointed at.
    pub fn unbind(&mut self, kb: KeyBinding) -> Option<StaticKey<Commands>> {
        self.map.remove(&kb)
    }

    /// Resolve a chord to the command it triggers, if any.
    pub fn resolve(&self, kb: KeyBinding) -> Option<StaticKey<Commands>> {
        self.map.get(&kb).copied()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold_title(l: &Locale) -> String {
        match l {
            Locale::Fr => "Gras".into(),
            Locale::En => "Bold".into(),
        }
    }

    fn italic_title(l: &Locale) -> String {
        match l {
            Locale::Fr => "Italique".into(),
            Locale::En => "Italic".into(),
        }
    }

    const FORMAT: StaticKey<Categories> = StaticKey::<Categories>::new("format");

    const COMMANDS: &[Command] = &[
        Command {
            id: StaticKey::<Commands>::new("prose.format.bold"),
            title: bold_title,
            icon: Some(StaticKey::<Icons>::new("bold")),
            keybindings: &[KeyBinding::primary(KeyChar::B)],
            category: FORMAT,
        },
        Command {
            id: StaticKey::<Commands>::new("prose.format.italic"),
            title: italic_title,
            icon: Some(StaticKey::<Icons>::new("italic")),
            keybindings: &[KeyBinding::primary(KeyChar::I)],
            category: FORMAT,
        },
    ];

    /// A table shaped like a real application's clipboard and history rows. It
    /// holds the shared identities and the shared chords, which is what every
    /// table declares once this crate owns them.
    const EDITING: &[Command] = &[
        Command {
            id: StaticKey::<Commands>::new("prose.history.redo"),
            title: bold_title,
            icon: None,
            keybindings: shared::REDO_CHORDS,
            category: FORMAT,
        },
        Command {
            id: shared::COPY,
            title: bold_title,
            icon: None,
            keybindings: shared::COPY_CHORDS,
            category: FORMAT,
        },
        Command {
            id: shared::PASTE,
            title: bold_title,
            icon: None,
            keybindings: shared::PASTE_CHORDS,
            category: FORMAT,
        },
    ];

    #[test]
    fn find_returns_matching_command() {
        let cmd = find(COMMANDS, StaticKey::<Commands>::new("prose.format.italic")).unwrap();
        assert_eq!((cmd.title)(&Locale::Fr), "Italique");
        assert_eq!((cmd.title)(&Locale::En), "Italic");
        assert!(find(COMMANDS, StaticKey::<Commands>::new("prose.format.unknown")).is_none());
    }

    #[test]
    fn keymap_resolves_default_bindings() {
        let km = Keymap::from_commands([COMMANDS]);
        assert_eq!(km.len(), 2);
        assert_eq!(
            km.resolve(KeyBinding::primary(KeyChar::B)),
            Some(StaticKey::<Commands>::new("prose.format.bold"))
        );
        // Shift state matters: Ctrl+Shift+B is not bound.
        assert_eq!(km.resolve(KeyBinding::primary_shift(KeyChar::B)), None);
        assert_eq!(km.resolve(KeyBinding::primary(KeyChar::Z)), None);
    }

    #[test]
    fn rebinding_reports_the_displaced_command() {
        let mut km = Keymap::from_commands([COMMANDS]);
        let displaced = km.bind(
            KeyBinding::primary(KeyChar::B),
            StaticKey::<Commands>::new("prose.format.italic"),
        );
        assert_eq!(
            displaced,
            Some(StaticKey::<Commands>::new("prose.format.bold"))
        );
        assert_eq!(
            km.resolve(KeyBinding::primary(KeyChar::B)),
            Some(StaticKey::<Commands>::new("prose.format.italic"))
        );
        assert_eq!(
            km.unbind(KeyBinding::primary(KeyChar::B)),
            Some(StaticKey::<Commands>::new("prose.format.italic"))
        );
        assert!(km.resolve(KeyBinding::primary(KeyChar::B)).is_none());
    }

    /// A rebinding reaches its own chord and no other: every chord it did not
    /// name resolves to the command it resolved to before (`UI-THREAD-042`).
    #[test]
    fn a_rebinding_leaves_every_other_chord_alone() {
        let mut km = Keymap::from_commands([COMMANDS, EDITING]);
        let chords: Vec<KeyBinding> = COMMANDS
            .iter()
            .chain(EDITING)
            .flat_map(|c| c.keybindings.iter().copied())
            .chain([
                KeyBinding::primary(KeyChar::Q),
                KeyBinding::primary_shift(KeyChar::B),
            ])
            .collect();
        let before: Vec<Option<StaticKey<Commands>>> =
            chords.iter().map(|&kb| km.resolve(kb)).collect();

        let rebound = KeyBinding::primary(KeyChar::B);
        km.bind(rebound, StaticKey::<Commands>::new("prose.format.italic"));
        km.bind(
            KeyBinding::primary(KeyChar::Q),
            StaticKey::<Commands>::new("prose.format.bold"),
        );

        for (kb, was) in chords.iter().zip(before) {
            if *kb == rebound || *kb == KeyBinding::primary(KeyChar::Q) {
                continue;
            }
            assert_eq!(km.resolve(*kb), was, "{kb} moved");
        }
    }

    fn blank(_: &Locale) -> String {
        String::new()
    }

    fn english_only(l: &Locale) -> String {
        match l {
            Locale::Fr => " ".into(),
            Locale::En => "Bold".into(),
        }
    }

    /// A whole table has no defect, and each part of a description that goes
    /// missing is named (`UI-THREAD-019`, `UI-THREAD-037`).
    #[test]
    fn a_table_names_each_missing_part_of_a_description() {
        assert_eq!(table_defects(COMMANDS), []);
        assert_eq!(table_defects(EDITING), []);

        let bold = StaticKey::<Commands>::new("prose.format.bold");
        let other = StaticKey::<Commands>::new("prose.format.other");
        const CHORD: &[KeyBinding] = &[KeyBinding::primary(KeyChar::B)];
        let broken = [
            Command {
                id: bold,
                title: blank,
                icon: Some(StaticKey::<Icons>::new(" ")),
                keybindings: CHORD,
                category: StaticKey::<Categories>::new(""),
            },
            Command {
                id: other,
                title: english_only,
                icon: None,
                keybindings: CHORD,
                category: FORMAT,
            },
            Command {
                id: bold,
                title: bold_title,
                icon: None,
                keybindings: &[],
                category: FORMAT,
            },
        ];
        assert_eq!(
            table_defects(&broken),
            [
                TableDefect::Unlabelled(bold, Locale::Fr),
                TableDefect::Unlabelled(bold, Locale::En),
                TableDefect::Ungrouped(bold),
                TableDefect::BlankIcon(bold),
                TableDefect::Unlabelled(other, Locale::Fr),
                TableDefect::SharedChord(KeyBinding::primary(KeyChar::B), bold, other),
                TableDefect::DescribedTwice(bold),
            ]
        );
    }

    #[test]
    fn both_redo_chords_reach_the_same_command() {
        let km = Keymap::from_commands([EDITING]);
        let redo = Some(StaticKey::<Commands>::new("prose.history.redo"));
        assert_eq!(km.resolve(KeyBinding::primary_shift(KeyChar::Z)), redo);
        assert_eq!(km.resolve(KeyBinding::primary(KeyChar::Y)), redo);
    }

    #[test]
    fn the_keymap_binds_no_browser_serviced_chord() {
        // The table declares the four clipboard chords, so a surface announces
        // them. The keymap resolves none, so the browser's own event stays the
        // one path (`UI-THREAD-011`, `CHROME-021`).
        let km = Keymap::from_commands([EDITING]);
        for chord in shared::COPY_CHORDS
            .iter()
            .chain(shared::PASTE_CHORDS)
            .chain(shared::PASTE_PLAIN_CHORDS)
        {
            assert_eq!(km.resolve(*chord), None, "{chord} must reach no command");
        }
        assert_eq!(km.len(), shared::REDO_CHORDS.len());
    }

    #[test]
    fn a_command_announces_the_first_chord_it_declares() {
        let redo = find(EDITING, StaticKey::<Commands>::new("prose.history.redo")).unwrap();
        assert_eq!(redo.chord(), Some(KeyBinding::primary_shift(KeyChar::Z)));
        let bold = find(COMMANDS, StaticKey::<Commands>::new("prose.format.bold")).unwrap();
        assert_eq!(bold.chord(), Some(KeyBinding::primary(KeyChar::B)));
    }

    #[test]
    fn command_state_default_is_enabled_visible_untoggled() {
        let s = CommandState::default();
        assert_eq!(s.enabled, ambre_core::Availability::Enabled);
        assert_eq!(s.visible, Listing::Listed);
        assert_eq!(s.toggled, ambre_core::Toggle::Off);
    }
}
