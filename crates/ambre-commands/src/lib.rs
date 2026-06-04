// ambre-commands/src/lib.rs
// ============================================================================
// Shared command registry for the Ambre UI thread.
//
// Menus, toolbar buttons, keyboard shortcuts, context menus and the command
// palette are all *views* of one registry of `Command`s. This keeps the chrome
// tractable across 5 apps x ~200 commands (ambre-spec-ui.md 2.C).
//
// Layering: a `Command`/`CommandId` here is a UI *affordance*. It is distinct
// from the `AppCommand`/`LogicCommand` IPC payloads in `ambre-bus`: invoking a
// `Command` *produces* a `LogicCommand`. This crate never knows about IPC, the
// renderer, or Leptos -- it is the leaf of the UI-thread crate graph.
// ============================================================================

mod keybinding;

pub use keybinding::{Key, KeyBinding, Modifiers};

use std::collections::HashMap;

/// Stable, namespaced identifier, e.g. `CommandId("prose.format.bold")`.
///
/// The `&'static str` is the identity: two `CommandId`s are equal iff their
/// strings are equal, which makes them cheap map keys and trivially `Copy`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CommandId(pub &'static str);

/// Locale selector for command labels. Minimal today (the two shipping UI
/// languages); extend as i18n grows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Locale {
    #[default]
    Fr,
    En,
}

/// Opaque icon handle. The concrete glyph/asset is resolved by the chrome's
/// icon set; the registry only carries the reference.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IconId(pub &'static str);

/// Grouping key for the command palette and for ribbon tabs (one category =
/// one tab), e.g. `CommandCategory("format")`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CommandCategory(pub &'static str);

/// Static description of a command. Apps build `const` tables of these and
/// expose them through [`CommandSource`].
pub struct Command {
    pub id: CommandId,
    /// Localized label. A function (not a string) so labels stay i18n-driven
    /// without the registry owning a translation table.
    pub title: fn(&Locale) -> String,
    pub icon: Option<IconId>,
    pub default_keybinding: Option<KeyBinding>,
    pub category: CommandCategory,
}

/// Runtime state of a command, derived from app state on each interrogation
/// (never stored on the `Command`). Drives enable/visibility/toggle in every
/// view of the registry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CommandState {
    pub enabled: bool,
    pub visible: bool,
    pub toggled: bool,
}

impl Default for CommandState {
    /// Enabled and visible, not toggled -- the neutral default for a command
    /// with no contextual constraints.
    fn default() -> Self {
        Self { enabled: true, visible: true, toggled: false }
    }
}

/// An owner of commands. Each app implements this; the chrome consumes it
/// without knowing which app it is (the same dependency inversion as `DocView`
/// on the render side).
///
/// `Context` is the app's own state handle. Keeping it an associated type
/// rather than a concrete `AppContext` preserves this crate as a true leaf:
/// the app, not `ambre-commands`, decides what state derivation needs.
pub trait CommandSource {
    type Context;

    /// The full, static command table.
    fn commands(&self) -> &[Command];

    /// Derive a command's runtime state from current app context.
    fn state(&self, id: CommandId, ctx: &Self::Context) -> CommandState;

    /// Invoke a command against app context (typically: emit a `LogicCommand`).
    fn invoke(&self, id: CommandId, ctx: &mut Self::Context);
}

/// Look up a command by id in a table. Linear scan: tables are small and this
/// runs off the hot path (palette open, menu build), so a map would only add
/// allocation.
pub fn find(commands: &[Command], id: CommandId) -> Option<&Command> {
    commands.iter().find(|c| c.id == id)
}

/// Resolves keyboard chords to commands: a `HashMap<KeyBinding, CommandId>`.
///
/// Seeded from each command's `default_keybinding`; the chrome may later
/// rebind entries from user preferences via [`Keymap::bind`].
#[derive(Default)]
pub struct Keymap {
    map: HashMap<KeyBinding, CommandId>,
}

impl Keymap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from one or more command tables, registering every command that
    /// declares a `default_keybinding`. Later tables override earlier ones on
    /// a chord collision; same-table collisions keep the last seen.
    pub fn from_commands<'a>(tables: impl IntoIterator<Item = &'a [Command]>) -> Self {
        let mut km = Self::new();
        for table in tables {
            for cmd in table {
                if let Some(kb) = cmd.default_keybinding {
                    km.map.insert(kb, cmd.id);
                }
            }
        }
        km
    }

    /// Bind (or rebind) a chord to a command. Returns the command it displaced,
    /// if any -- useful to surface shortcut conflicts to the user.
    pub fn bind(&mut self, kb: KeyBinding, id: CommandId) -> Option<CommandId> {
        self.map.insert(kb, id)
    }

    /// Remove a binding, returning the command it pointed at.
    pub fn unbind(&mut self, kb: KeyBinding) -> Option<CommandId> {
        self.map.remove(&kb)
    }

    /// Resolve a chord to the command it triggers, if any.
    pub fn resolve(&self, kb: KeyBinding) -> Option<CommandId> {
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

    const FORMAT: CommandCategory = CommandCategory("format");

    const COMMANDS: &[Command] = &[
        Command {
            id: CommandId("prose.format.bold"),
            title: bold_title,
            icon: Some(IconId("bold")),
            default_keybinding: Some(KeyBinding::ctrl('b')),
            category: FORMAT,
        },
        Command {
            id: CommandId("prose.format.italic"),
            title: italic_title,
            icon: Some(IconId("italic")),
            default_keybinding: Some(KeyBinding::ctrl('i')),
            category: FORMAT,
        },
    ];

    #[test]
    fn find_returns_matching_command() {
        let cmd = find(COMMANDS, CommandId("prose.format.italic")).unwrap();
        assert_eq!((cmd.title)(&Locale::Fr), "Italique");
        assert_eq!((cmd.title)(&Locale::En), "Italic");
        assert!(find(COMMANDS, CommandId("prose.format.unknown")).is_none());
    }

    #[test]
    fn keymap_resolves_default_bindings() {
        let km = Keymap::from_commands([COMMANDS]);
        assert_eq!(km.len(), 2);
        assert_eq!(km.resolve(KeyBinding::ctrl('b')), Some(CommandId("prose.format.bold")));
        // Shift state matters: Ctrl+Shift+B is not bound.
        assert_eq!(km.resolve(KeyBinding::ctrl_shift('b')), None);
        assert_eq!(km.resolve(KeyBinding::ctrl('z')), None);
    }

    #[test]
    fn rebinding_reports_the_displaced_command() {
        let mut km = Keymap::from_commands([COMMANDS]);
        let displaced = km.bind(KeyBinding::ctrl('b'), CommandId("prose.format.italic"));
        assert_eq!(displaced, Some(CommandId("prose.format.bold")));
        assert_eq!(km.resolve(KeyBinding::ctrl('b')), Some(CommandId("prose.format.italic")));
        assert_eq!(km.unbind(KeyBinding::ctrl('b')), Some(CommandId("prose.format.italic")));
        assert!(km.resolve(KeyBinding::ctrl('b')).is_none());
    }

    #[test]
    fn command_state_default_is_enabled_visible_untoggled() {
        let s = CommandState::default();
        assert!(s.enabled && s.visible && !s.toggled);
    }
}
