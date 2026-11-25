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

use ambre_core::{Categories, Commands, Count, Icons, NarrowCount, Properties, StaticKey};

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

/// The label of a command in one locale, which a person reads in a menu or
/// the palette. `Validated<CommandTitle>`: no rule bounds it.
pub enum CommandTitle {}

impl ambre_core::Free for CommandTitle {}

impl ambre_core::Displayed for CommandTitle {}

/// One line that says what a command does where its name is not enough,
/// which a person reads in the tooltip of its icon (`CHROME-185`).
/// `Validated<CommandDescription>`: no rule bounds it.
pub enum CommandDescription {}

impl ambre_core::Free for CommandDescription {}

impl ambre_core::Displayed for CommandDescription {}

/// The name of a home region, which a person reads beside a command in the
/// palette (`CHROME-157`). `Validated<RegionName>`: no rule bounds it.
pub enum RegionName {}

impl ambre_core::Free for RegionName {}

impl ambre_core::Displayed for RegionName {}

/// The one region a command lives in (`ambre-spec-shell.md` section 2,
/// `CHROME-148`). The six homes are the only variants. The context bar and
/// the palette reach commands but home none, so neither is a variant.
///
/// [`Command::home`] is a required field, so a description that names no
/// region does not compile:
///
/// ```compile_fail,E0063
/// use ambre_commands::{Command, CommandTitle, Locale};
/// use ambre_core::{Categories, Commands, StaticKey, Validated};
/// fn title(_: &Locale) -> Validated<CommandTitle> {
///     Validated::new("Bold")
/// }
/// const BOLD: Command = Command {
///     id: StaticKey::<Commands>::new("t.format.bold"),
///     title,
///     icon: None,
///     keybindings: &[],
///     category: StaticKey::<Categories>::new("format"),
///     description: None,
/// };
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Home {
    /// Shows the state of the document and never writes it (`CHROME-149`).
    StatusBar,
    /// The tools that act on the selection.
    ContextPanel,
    /// The tools that add a new element.
    CreationBar,
    /// The verbs on the thing under the pointer.
    ContextMenu,
    /// The whole document: the file, the history, the palette.
    AmbreMenu,
    /// The person: preferences and identity.
    AvatarMenu,
}

impl Home {
    /// The name a person reads for the region, in one locale.
    pub fn name(self, locale: &Locale) -> ambre_core::Validated<RegionName> {
        ambre_core::Validated::new(match (self, locale) {
            (Self::StatusBar, Locale::Fr) => "Barre d'\u{e9}tat",
            (Self::StatusBar, Locale::En) => "Status bar",
            (Self::ContextPanel, Locale::Fr) => "Panneau contextuel",
            (Self::ContextPanel, Locale::En) => "Context panel",
            (Self::CreationBar, Locale::Fr) => "Barre de cr\u{e9}ation",
            (Self::CreationBar, Locale::En) => "Creation bar",
            (Self::ContextMenu, Locale::Fr) => "Menu contextuel",
            (Self::ContextMenu, Locale::En) => "Context menu",
            (Self::AmbreMenu, Locale::Fr) => "Menu Ambre",
            (Self::AmbreMenu, Locale::En) => "Ambre menu",
            (Self::AvatarMenu, Locale::Fr) => "Menu de l'avatar",
            (Self::AvatarMenu, Locale::En) => "Avatar menu",
        })
    }
}

/// One verb on the call of the page (the Murmure calls spec, section 3, the call controls).
/// Each icon of the call controls names one, and its command names the same one. A verb that
/// sets a state names the state it sets, so a palette entry on a state already set does nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallVerb {
    /// Start a call in this channel, or join the one that runs there.
    Start(ambre_core::id::Id<ambre_core::id::Channel>),
    /// Mute the microphone.
    Mute,
    /// Unmute the microphone.
    Unmute,
    /// Turn the camera on.
    CameraOn,
    /// Turn the camera off.
    CameraOff,
    /// Share the screen.
    ShareScreen,
    /// Stop the share of the screen.
    StopSharing,
    /// Widen the call to the whole channel region.
    Fill,
    /// Shrink the call back to a band.
    Shrink,
    /// Open or close the list of the participants.
    Participants,
    /// Open or close the panel of the devices.
    Devices,
    /// Show the page of the grid before.
    PreviousPage,
    /// Show the page of the grid after.
    NextPage,
    /// Show the channel of the call again.
    BackToCall,
    /// Leave the call.
    HangUp,
    /// Accept the call that rings.
    Accept,
    /// Decline the call that rings.
    Decline,
}

/// Static description of a command. Apps build `const` tables of these and
/// expose them through [`CommandSource`].
pub struct Command {
    pub id: StaticKey<Commands>,
    /// Localized label. A function (not a string) so labels stay i18n-driven
    /// without the registry owning a translation table.
    pub title: fn(&Locale) -> ambre_core::Validated<CommandTitle>,
    pub icon: Option<StaticKey<Icons>>,
    /// Every chord the command answers, the announced one first. A slice and
    /// not one chord, because a command may answer two. Redo takes
    /// Ctrl+Shift+Z and Ctrl+Y, and each clipboard intent takes a modern chord
    /// and a legacy one. Empty for a command no chord triggers.
    pub keybindings: &'static [KeyBinding],
    pub category: StaticKey<Categories>,
    /// The one region the command lives in (`CHROME-148`). A required field,
    /// so a command without a home does not compile.
    pub home: Home,
    /// One line the tooltip of the icon shows under the name, where the name
    /// is not enough (`CHROME-185`). A function of the locale, as the title.
    /// `None` for a command its name describes.
    pub description: Option<fn(&Locale) -> ambre_core::Validated<CommandDescription>>,
}

impl Command {
    /// The chord a surface announces beside the command: the first it
    /// declares. `None` for a command no chord triggers.
    pub fn chord(&self) -> Option<KeyBinding> {
        self.keybindings.first().copied()
    }
}

/// Static description of a property (`ambre-spec-shell.md` section 2, the
/// property model). A property is a value of the selection that a control of
/// the context panel reads and writes. A colour, a font or a size is a
/// property; bold is a command.
///
/// A property has no `home` field: the context panel is its one home by
/// construction, so no description can file it elsewhere.
pub struct Property {
    pub id: StaticKey<Properties>,
    /// Localized label, as a command's.
    pub title: fn(&Locale) -> ambre_core::Validated<CommandTitle>,
    pub icon: Option<StaticKey<Icons>>,
}

/// What one entry of a contextual surface reaches: a command, or a property
/// (`CHROME-207`). The context bar and the context menu are lists of these,
/// and each control of the context panel carries one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Reach {
    /// A command, run with no argument.
    Command(StaticKey<Commands>),
    /// A property, written with a value.
    Property(StaticKey<Properties>),
}

impl Reach {
    /// The prefix of [`Reach::marker`] for a command.
    const COMMAND: &'static str = "command:";
    /// The prefix of [`Reach::marker`] for a property.
    const PROPERTY: &'static str = "property:";

    /// The word a control writes on its `data-reach` attribute, which a test
    /// reads back with [`Reach::matches_marker`].
    #[must_use]
    pub fn marker(self) -> String {
        match self {
            Self::Command(id) => format!("{}{}", Self::COMMAND, id.as_str()),
            Self::Property(id) => format!("{}{}", Self::PROPERTY, id.as_str()),
        }
    }

    /// Whether `marker`, read off a `data-reach` attribute, names this entry.
    #[must_use]
    pub fn matches_marker(self, marker: &str) -> bool {
        let (prefix, id) = match self {
            Self::Command(id) => (Self::COMMAND, id.as_str()),
            Self::Property(id) => (Self::PROPERTY, id.as_str()),
        };
        marker.strip_prefix(prefix) == Some(id)
    }
}

/// The value a property reads off the selection (`CHROME-210`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Reading<T> {
    /// Every selected element holds this value.
    Uniform(T),
    /// The selected elements disagree.
    Mixed,
}

impl<T> Reading<T> {
    /// The one value, or `None` when the selection is mixed.
    pub fn uniform(self) -> Option<T> {
        match self {
            Self::Uniform(value) => Some(value),
            Self::Mixed => None,
        }
    }

    /// Whether the selected elements disagree.
    pub fn is_mixed(&self) -> bool {
        matches!(self, Self::Mixed)
    }
}

/// Look up a property by id in a table.
pub fn find_property(properties: &[Property], id: StaticKey<Properties>) -> Option<&Property> {
    properties.iter().find(|p| p.id == id)
}

/// Why an entry of a contextual surface breaks the rules of its region
/// (`CHROME-150`, `CHROME-151`, `CHROME-182`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReachDefect {
    /// The entry names a command no table describes.
    Undeclared(Reach),
    /// The command lives in another region than the context panel.
    NotPanelHomed(StaticKey<Commands>, Home),
    /// The context bar holds a clipboard command.
    ClipboardInBar(StaticKey<Commands>),
    /// The context menu does not open on cut, copy and paste, in that order.
    ClipboardNotFirst,
}

/// The order of the three clipboard entries a context menu opens on
/// (`CHROME-179`).
pub const MENU_CLIPBOARD: [Reach; 3] = [
    Reach::Command(shared::CUT),
    Reach::Command(shared::COPY),
    Reach::Command(shared::PASTE),
];

/// Every defect of a context bar. Each entry is a declared property, or a
/// command whose home is the context panel. No entry is a clipboard command.
pub fn bar_defects(
    bar: &[Reach],
    commands: &[Command],
    properties: &[Property],
) -> Vec<ReachDefect> {
    bar.iter()
        .filter_map(|&entry| match entry {
            Reach::Command(id) if shared::BROWSER_SERVICED.contains(&id) => {
                Some(ReachDefect::ClipboardInBar(id))
            }
            _ => panel_defect(entry, commands, properties),
        })
        .collect()
}

/// Every defect of a context menu (`CHROME-179`, `CHROME-182`). Its clipboard
/// entries come first, and cut, copy and paste keep that order among those
/// the menu shows. Every other entry is a declared property, or a command
/// whose home is the context panel.
pub fn menu_defects(
    menu: &[Reach],
    commands: &[Command],
    properties: &[Property],
) -> Vec<ReachDefect> {
    let clipboard = |entry: &Reach| matches!(entry, Reach::Command(id) if shared::BROWSER_SERVICED.contains(id));
    let lead = menu.iter().take_while(|e| clipboard(e)).count();
    let order: Vec<usize> = menu
        .iter()
        .filter_map(|e| MENU_CLIPBOARD.iter().position(|c| c == e))
        .collect();
    let mut defects = Vec::new();
    if menu.iter().skip(lead).any(clipboard)
        || order.windows(2).any(|w| matches!(w, [a, b] if a >= b))
    {
        defects.push(ReachDefect::ClipboardNotFirst);
    }
    defects.extend(
        menu.iter()
            .skip(lead)
            .filter(|e| !clipboard(e))
            .filter_map(|&entry| panel_defect(entry, commands, properties)),
    );
    defects
}

/// The defect of one entry that must reach the context panel, if any.
fn panel_defect(
    entry: Reach,
    commands: &[Command],
    properties: &[Property],
) -> Option<ReachDefect> {
    match entry {
        Reach::Command(id) => match find(commands, id) {
            None => Some(ReachDefect::Undeclared(entry)),
            Some(c) if c.home != Home::ContextPanel => Some(ReachDefect::NotPanelHomed(id, c.home)),
            Some(_) => None,
        },
        Reach::Property(id) => find_property(properties, id)
            .is_none()
            .then_some(ReachDefect::Undeclared(entry)),
    }
}

/// The subtitle of a context panel over an empty selection, the same words
/// in Prose, Trame and Toile (`CHROME-241`).
pub const NOTHING_SELECTED: &str = "nothing selected";

/// What every application shares, decided once: the clipboard identities and
/// the chords the universal operations answer (`ambre-spec-commands.md`,
/// `UI-THREAD-011`).
pub mod shared {
    use super::{Key, KeyBinding, KeyChar, ModifierKeys};
    use ambre_core::{Categories, Commands, StaticKey};

    /// The group of every command that acts on the document as a file: save,
    /// import, export, share. Its home is the Ambre menu (`CHROME-154`).
    pub const FILE: StaticKey<Categories> = StaticKey::<Categories>::new("file");
    /// The group of every command that acts on the account or a preference
    /// of the person. Its home is the avatar menu (`CHROME-156`).
    pub const ACCOUNT: StaticKey<Categories> = StaticKey::<Categories>::new("account");

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

    /// The command palette answers one chord in every application. The
    /// header finds the palette command of a table by it, so the shell names
    /// no command id of an application (`ARCH-018`).
    pub const PALETTE_CHORD: KeyBinding = KeyBinding::primary(KeyChar::K);

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

    /// Collapse an open context panel to its rail, or open a collapsed one
    /// (`CHROME-175`, `CHROME-176`). The panel is the shell's in every
    /// application, so the chord is shared.
    pub const PANEL_TOGGLE_CHORDS: &[KeyBinding] = &[KeyBinding::primary(KeyChar::new('.'))];

    /// The zoom step a chord asks for, in every application (`ZOOM-004`,
    /// `ZOOM-005`). `Ctrl` with `=` or `+` zooms in, and with `-` zooms out.
    /// `Ctrl` with `0` returns to one. Shift is free, as `+` is a shifted `=` on most
    /// layouts. Any other chord is `None`, so it stays the caller's.
    #[must_use]
    pub fn zoom_step(
        key: Key,
        mods: ambre_core::Flags<ModifierKeys>,
    ) -> Option<ambre_core::ZoomStep> {
        use ambre_core::ZoomStep;
        let held = mods.without(ModifierKeys::SHIFT);
        if held != ModifierKeys::PRIMARY {
            return None;
        }
        match key {
            Key::Char(c) => match c.get() {
                '=' | '+' => Some(ZoomStep::In),
                '-' => Some(ZoomStep::Out),
                '0' => Some(ZoomStep::Reset),
                _ => None,
            },
            _ => None,
        }
    }
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
    /// The command declares a description with no words in a locale, so its
    /// tooltip shows an empty line (`CHROME-185`).
    BlankDescription(StaticKey<Commands>, Locale),
    /// Two entries declare one default chord. Seeding keeps the last one, so
    /// the first silently leaves the keyboard.
    SharedChord(KeyBinding, StaticKey<Commands>, StaticKey<Commands>),
    /// The command acts on the file or on the account, and lives in another
    /// region than the one its subject names (`CHROME-154`, `CHROME-156`).
    HomedAway(StaticKey<Commands>, Home),
}

/// The one region the subject of `category` names, if it names one. A
/// command on the document as a file lives in the Ambre menu. A command on
/// the account of the person lives in the avatar menu
/// (`ambre-spec-shell.md` section 2).
#[must_use]
pub fn subject_home(category: StaticKey<Categories>) -> Option<Home> {
    if category == shared::FILE {
        Some(Home::AmbreMenu)
    } else if category == shared::ACCOUNT {
        Some(Home::AvatarMenu)
    } else {
        None
    }
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
            if (cmd.title)(&locale).as_str().trim().is_empty() {
                defects.push(TableDefect::Unlabelled(cmd.id, locale));
            }
        }
        if cmd.category.as_str().trim().is_empty() {
            defects.push(TableDefect::Ungrouped(cmd.id));
        }
        if cmd.icon.is_some_and(|icon| icon.as_str().trim().is_empty()) {
            defects.push(TableDefect::BlankIcon(cmd.id));
        }
        if let Some(description) = cmd.description {
            for locale in [Locale::Fr, Locale::En] {
                if description(&locale).as_str().trim().is_empty() {
                    defects.push(TableDefect::BlankDescription(cmd.id, locale));
                }
            }
        }
        for &kb in cmd.keybindings {
            if let Some(first) = chords.insert(kb, cmd.id) {
                defects.push(TableDefect::SharedChord(kb, first, cmd.id));
            }
        }
        if subject_home(cmd.category).is_some_and(|home| home != cmd.home) {
            defects.push(TableDefect::HomedAway(cmd.id, cmd.home));
        }
    }
    defects
}

/// The chords a [`Keymap`] binds, one per binding.
pub enum Bindings {}

impl NarrowCount for Bindings {
    const NAME: StaticKey<ambre_core::DebugNames> = StaticKey::new("Bindings");
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

    /// The chords the keymap binds.
    pub fn len(&self) -> Count<Bindings> {
        Count::of(self.map.len())
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ZOOM-004`, `ZOOM-005`: the zoom chords are one decision, read by
    /// every application. A plain key and another modifier are no zoom.
    #[test]
    fn the_zoom_chords_step_the_zoom_in_every_application() {
        use ambre_core::ZoomStep;
        let ctrl = ModifierKeys::PRIMARY;
        for (c, step) in [
            ('=', ZoomStep::In),
            ('+', ZoomStep::In),
            ('-', ZoomStep::Out),
            ('0', ZoomStep::Reset),
        ] {
            let key = Key::Char(KeyChar::new(c));
            assert_eq!(shared::zoom_step(key, ctrl), Some(step), "{c}");
            assert_eq!(
                shared::zoom_step(key, ctrl.with(ModifierKeys::SHIFT)),
                Some(step)
            );
            assert_eq!(shared::zoom_step(key, ModifierKeys::NONE), None);
            assert_eq!(shared::zoom_step(key, ctrl.with(ModifierKeys::ALT)), None);
        }
        assert_eq!(shared::zoom_step(Key::Char(KeyChar::Z), ctrl), None);
    }

    fn bold_title(l: &Locale) -> ambre_core::Validated<CommandTitle> {
        match l {
            Locale::Fr => ambre_core::Validated::new("Gras"),
            Locale::En => ambre_core::Validated::new("Bold"),
        }
    }

    fn italic_title(l: &Locale) -> ambre_core::Validated<CommandTitle> {
        match l {
            Locale::Fr => ambre_core::Validated::new("Italique"),
            Locale::En => ambre_core::Validated::new("Italic"),
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
            home: Home::ContextPanel,
            description: None,
        },
        Command {
            id: StaticKey::<Commands>::new("prose.format.italic"),
            title: italic_title,
            icon: Some(StaticKey::<Icons>::new("italic")),
            keybindings: &[KeyBinding::primary(KeyChar::I)],
            category: FORMAT,
            home: Home::ContextPanel,
            description: None,
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
            home: Home::ContextPanel,
            description: None,
        },
        Command {
            id: shared::COPY,
            title: bold_title,
            icon: None,
            keybindings: shared::COPY_CHORDS,
            category: FORMAT,
            home: Home::ContextPanel,
            description: None,
        },
        Command {
            id: shared::PASTE,
            title: bold_title,
            icon: None,
            keybindings: shared::PASTE_CHORDS,
            category: FORMAT,
            home: Home::ContextPanel,
            description: None,
        },
    ];

    #[test]
    fn find_returns_matching_command() {
        let cmd = find(COMMANDS, StaticKey::<Commands>::new("prose.format.italic")).unwrap();
        assert_eq!((cmd.title)(&Locale::Fr).as_str(), "Italique");
        assert_eq!((cmd.title)(&Locale::En).as_str(), "Italic");
        assert!(find(COMMANDS, StaticKey::<Commands>::new("prose.format.unknown")).is_none());
    }

    #[test]
    fn keymap_resolves_default_bindings() {
        let km = Keymap::from_commands([COMMANDS]);
        assert_eq!(km.len().get(), 2);
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

    fn blank(_: &Locale) -> ambre_core::Validated<CommandTitle> {
        ambre_core::Validated::default()
    }

    fn blank_description(_: &Locale) -> ambre_core::Validated<CommandDescription> {
        ambre_core::Validated::new(" ")
    }

    fn english_only(l: &Locale) -> ambre_core::Validated<CommandTitle> {
        match l {
            Locale::Fr => ambre_core::Validated::new(" "),
            Locale::En => ambre_core::Validated::new("Bold"),
        }
    }

    /// A command on the file outside the Ambre menu is a defect of the table.
    /// So is a command on the account outside the avatar menu. A
    /// command of another group may live in any region (`CHROME-154`,
    /// `CHROME-156`).
    #[test]
    fn a_command_on_the_file_or_the_account_homed_away_is_a_defect() {
        let save = StaticKey::<Commands>::new("t.file.save");
        let theme = StaticKey::<Commands>::new("t.account.theme");
        let bold = StaticKey::<Commands>::new("t.format.bold");
        let entry = |id, category, home| Command {
            id,
            title: bold_title,
            icon: None,
            keybindings: &[],
            category,
            home,
            description: None,
        };
        let homed = [
            entry(save, shared::FILE, Home::AmbreMenu),
            entry(theme, shared::ACCOUNT, Home::AvatarMenu),
            entry(bold, FORMAT, Home::AvatarMenu),
        ];
        assert_eq!(table_defects(&homed), []);
        let away = [
            entry(save, shared::FILE, Home::ContextPanel),
            entry(theme, shared::ACCOUNT, Home::AmbreMenu),
        ];
        assert_eq!(
            table_defects(&away),
            [
                TableDefect::HomedAway(save, Home::ContextPanel),
                TableDefect::HomedAway(theme, Home::AmbreMenu),
            ]
        );
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
                home: Home::ContextPanel,
                description: Some(blank_description),
            },
            Command {
                id: other,
                title: english_only,
                icon: None,
                keybindings: CHORD,
                category: FORMAT,
                home: Home::ContextPanel,
                description: None,
            },
            Command {
                id: bold,
                title: bold_title,
                icon: None,
                keybindings: &[],
                category: FORMAT,
                home: Home::ContextPanel,
                description: None,
            },
        ];
        assert_eq!(
            table_defects(&broken),
            [
                TableDefect::Unlabelled(bold, Locale::Fr),
                TableDefect::Unlabelled(bold, Locale::En),
                TableDefect::Ungrouped(bold),
                TableDefect::BlankIcon(bold),
                TableDefect::BlankDescription(bold, Locale::Fr),
                TableDefect::BlankDescription(bold, Locale::En),
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
        assert_eq!(km.len(), Count::of(shared::REDO_CHORDS.len()));
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

    fn t(_: &Locale) -> ambre_core::Validated<CommandTitle> {
        ambre_core::Validated::new("x")
    }

    const BOLD: StaticKey<Commands> = StaticKey::<Commands>::new("t.bold");
    const SAVE: StaticKey<Commands> = StaticKey::<Commands>::new("t.save");
    const FILL: StaticKey<Properties> = StaticKey::<Properties>::new("t.fill");

    const TABLE: &[Command] = &[
        Command {
            id: BOLD,
            title: t,
            icon: None,
            keybindings: &[],
            category: StaticKey::<Categories>::new("format"),
            home: Home::ContextPanel,
            description: None,
        },
        Command {
            id: SAVE,
            title: t,
            icon: None,
            keybindings: &[],
            category: StaticKey::<Categories>::new("file"),
            home: Home::AmbreMenu,
            description: None,
        },
    ];

    const PROPS: &[Property] = &[Property {
        id: FILL,
        title: t,
        icon: None,
    }];

    #[test]
    fn a_bar_of_panel_commands_and_properties_has_no_defect() {
        let bar = [Reach::Command(BOLD), Reach::Property(FILL)];
        assert!(bar_defects(&bar, TABLE, PROPS).is_empty());
    }

    #[test]
    fn a_bar_refuses_the_clipboard_and_a_command_of_another_home() {
        let bar = [Reach::Command(shared::COPY), Reach::Command(SAVE)];
        assert_eq!(
            bar_defects(&bar, TABLE, PROPS),
            vec![
                ReachDefect::ClipboardInBar(shared::COPY),
                ReachDefect::NotPanelHomed(SAVE, Home::AmbreMenu),
            ]
        );
    }

    #[test]
    fn a_menu_opens_on_cut_copy_paste_then_reaches_the_panel_alone() {
        let mut menu = MENU_CLIPBOARD.to_vec();
        menu.push(Reach::Command(BOLD));
        assert!(menu_defects(&menu, TABLE, PROPS).is_empty());
        // A menu that shows copy and paste alone keeps them first.
        let locked = [
            Reach::Command(shared::COPY),
            Reach::Command(shared::PASTE),
            Reach::Command(BOLD),
        ];
        assert!(menu_defects(&locked, TABLE, PROPS).is_empty());
        let late = [Reach::Command(BOLD), Reach::Command(shared::PASTE)];
        assert_eq!(
            menu_defects(&late, TABLE, PROPS),
            vec![ReachDefect::ClipboardNotFirst]
        );
        let swapped = [
            Reach::Command(shared::COPY),
            Reach::Command(shared::CUT),
            Reach::Command(shared::PASTE),
            Reach::Command(SAVE),
        ];
        assert_eq!(
            menu_defects(&swapped, TABLE, PROPS),
            vec![
                ReachDefect::ClipboardNotFirst,
                ReachDefect::NotPanelHomed(SAVE, Home::AmbreMenu),
            ]
        );
    }

    #[test]
    fn a_reach_marker_reads_back_as_its_entry_alone() {
        let cmd = Reach::Command(BOLD);
        let prop = Reach::Property(FILL);
        assert!(cmd.matches_marker(&cmd.marker()));
        assert!(prop.matches_marker(&prop.marker()));
        assert!(!cmd.matches_marker(&prop.marker()));
        assert!(
            !Reach::Property(StaticKey::<Properties>::new("t.bold")).matches_marker(&cmd.marker())
        );
    }
}
