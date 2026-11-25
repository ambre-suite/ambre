// ambre-chrome/src/tip.rs
// ============================================================================
// The tooltip of an icon (`ambre-spec-shell.md` section 2, "An icon names
// itself").
//
//   CommandTip:      what the tooltip of a command shows, read from the
//                    description of the command alone (`CHROME-186`).
//   Press:           the long press of a touch, which shows the tooltip in
//                    place of the command (`CHROME-188`). A pure state
//                    machine, so the host tier drives every transition.
//   untipped_icons:  the probe a test runs over a mounted surface. Each
//                    control of an icon alone names itself in a tooltip, as
//                    its command describes it (`CHROME-185`).
//
// The icon button of `controls` draws the tooltip; this module holds the
// decisions it takes.
// ============================================================================

use std::cell::Cell;
use std::time::Duration;

use ambre_commands::{Command, CommandDescription, CommandTitle, KeyBinding, Locale, Reach};
use ambre_core::{Seq, SeqKind, Validated};
use leptos::wasm_bindgen::JsCast;

/// How long a touch holds an icon before its tooltip shows in place of its
/// command. The long press of the platforms sits near half a second.
pub(crate) const LONG_PRESS: Duration = Duration::from_millis(500);

/// How long a tooltip a long press showed stays after the finger lifts, so
/// the person reads it with the finger away.
pub(crate) const TIP_DWELL: Duration = Duration::from_millis(1500);

/// What the tooltip of a command's icon shows: its name, its chord and its
/// line of description (`CHROME-186`).
///
/// The fields are private and [`CommandTip::of`] is the one constructor, so
/// a tooltip built from a `CommandTip` carries no word of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandTip {
    title: Validated<CommandTitle>,
    chord: Option<KeyBinding>,
    description: Option<Validated<CommandDescription>>,
    reach: Reach,
}

impl CommandTip {
    /// The tooltip `command` describes. One language ships today, so the
    /// tooltip reads the English words, as every label of the chrome does.
    #[must_use]
    pub fn of(command: &Command) -> Self {
        let locale = Locale::En;
        Self {
            title: (command.title)(&locale),
            chord: command.chord(),
            description: command.description.map(|describe| describe(&locale)),
            reach: Reach::Command(command.id),
        }
    }

    /// The name of the command: the first line of the tooltip and the
    /// accessible name of the control.
    #[must_use]
    pub fn title(&self) -> &Validated<CommandTitle> {
        &self.title
    }

    /// The chord the tooltip shows as the shortcut chip.
    #[must_use]
    pub fn chord(&self) -> Option<KeyBinding> {
        self.chord
    }

    /// The line of description under the name, where the name is not enough.
    #[must_use]
    pub fn description(&self) -> Option<&Validated<CommandDescription>> {
        self.description.as_ref()
    }

    /// The command the control reaches (`CHROME-207`).
    #[must_use]
    pub fn reach(&self) -> Reach {
        self.reach
    }
}

/// The number of the hidden text a control's `aria-describedby` names, so
/// two tooltips of one page never share an id.
pub(crate) enum TipId {}

impl SeqKind for TipId {
    type IssueRight = ();
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("TipId");
}

thread_local! {
    /// The last number handed out. The UI thread alone mounts a control.
    static LAST_TIP: Cell<Seq<TipId>> = const { Cell::new(Seq::ZERO) };
}

/// A number no other tooltip of the page holds.
pub(crate) fn next_tip() -> Seq<TipId> {
    LAST_TIP.with(|last| {
        let next = last.get().next();
        last.set(next);
        next
    })
}

/// The kind of pointer a press comes from. A touch alone has a long press;
/// a mouse or a pen shows the tooltip on hover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointerKind {
    /// A finger on a touch screen.
    Touch,
    /// A mouse or a pen.
    Hover,
}

impl PointerKind {
    /// The kind of the pointer that fired `ev`.
    pub(crate) fn of(ev: &web_sys::PointerEvent) -> Self {
        if ev.pointer_type() == "touch" {
            Self::Touch
        } else {
            Self::Hover
        }
    }
}

/// Where a press on an icon stands (`CHROME-188`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Press {
    /// No touch holds the icon.
    #[default]
    Idle,
    /// A touch holds the icon, for less than [`LONG_PRESS`] so far.
    Pending,
    /// A touch held the icon for [`LONG_PRESS`]: the tooltip shows, and the
    /// click that ends the press runs nothing.
    Held,
}

/// What the click that ends a press does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PressEnd {
    /// The click runs the command.
    Run,
    /// The click ends a long press: the tooltip stays, and nothing runs.
    KeepTip,
}

impl Press {
    /// A pointer goes down on the icon. A touch starts the wait.
    pub(crate) fn down(kind: PointerKind) -> Self {
        match kind {
            PointerKind::Touch => Self::Pending,
            PointerKind::Hover => Self::Idle,
        }
    }

    /// The wait of [`LONG_PRESS`] ends. A touch still down becomes a long
    /// press.
    pub(crate) fn waited(self) -> Self {
        match self {
            Self::Pending | Self::Held => Self::Held,
            Self::Idle => Self::Idle,
        }
    }

    /// The pointer lifts or leaves before the click. A short touch is a tap;
    /// a long press keeps its state for the click that follows.
    pub(crate) fn lifted(self) -> Self {
        match self {
            Self::Pending | Self::Idle => Self::Idle,
            Self::Held => Self::Held,
        }
    }

    /// The click that ends the press: what it does.
    pub(crate) fn click(self) -> PressEnd {
        match self {
            Self::Held => PressEnd::KeepTip,
            Self::Idle | Self::Pending => PressEnd::Run,
        }
    }
}

/// The controls under `root` that show an icon and no text and break the
/// rule of the tooltip, by accessible name (`CHROME-185`, `CHROME-186`). A
/// test holds it empty.
///
/// A control breaks the rule when it has no tooltip, or when the tooltip does
/// not name the control as its accessible name does. A control that reaches a
/// command of `table` breaks it too on a name, a chord or a description that
/// the command does not describe. A control of a property reaches no
/// command, so its tooltip names the control alone.
pub fn untipped_icons(root: &web_sys::Element, table: &[Command]) -> Vec<String> {
    let Ok(list) = root.query_selector_all("button") else {
        return Vec::new();
    };
    // The trimmed text of the first element under `el` that a selector names.
    let text_of = |el: &web_sys::Element, selector| {
        el.query_selector(selector)
            .ok()
            .flatten()
            .and_then(|found| found.text_content())
            .map(|t| t.trim().to_owned())
    };
    // An icon, and no text a person reads outside the tooltip and the hidden
    // description.
    let shows_an_icon_alone = |button: &web_sys::Element| {
        let has_icon = button.query_selector("svg").ok().flatten().is_some();
        let children = button.child_nodes();
        let shown_text = (0..children.length())
            .filter_map(|i| children.item(i))
            .filter(|node| {
                node.dyn_ref::<web_sys::Element>().is_none_or(|el| {
                    !el.class_list().contains("ambre-tip") && !el.has_attribute("hidden")
                })
            })
            .filter_map(|node| node.text_content())
            .any(|text| !text.trim().is_empty());
        has_icon && !shown_text
    };
    // The tooltip names the control, and shows what its command describes.
    let tip_holds = |button: &web_sys::Element| {
        let Some(tip) = button
            .query_selector(".ambre-tip[role=\"tooltip\"]")
            .ok()
            .flatten()
        else {
            return false;
        };
        let name = text_of(&tip, ".ambre-tip-name");
        if name.is_none() || name != button.get_attribute("aria-label") {
            return false;
        }
        let marker = button.get_attribute("data-reach").unwrap_or_default();
        let Some(command) = table
            .iter()
            .find(|c| Reach::Command(c.id).matches_marker(&marker))
        else {
            return true;
        };
        let want = CommandTip::of(command);
        name.as_deref() == Some(want.title().as_str())
            && text_of(&tip, ".ambre-kbd") == want.chord().map(|c| c.to_string())
            && text_of(&tip, ".ambre-tip-description")
                == want.description().map(|d| d.as_str().to_owned())
    };
    (0..list.length())
        .filter_map(|i| list.item(i)?.dyn_into::<web_sys::Element>().ok())
        .filter(|e| shows_an_icon_alone(e) && !tip_holds(e))
        .map(|e| {
            e.get_attribute("aria-label")
                .unwrap_or_else(|| e.outer_html())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_commands::{Home, KeyChar};
    use ambre_core::{Categories, Commands, StaticKey};

    fn hand_title(_: &Locale) -> Validated<CommandTitle> {
        Validated::new("Hand")
    }

    fn hand_description(_: &Locale) -> Validated<CommandDescription> {
        Validated::new("Drag to move the view")
    }

    const HAND: Command = Command {
        id: StaticKey::<Commands>::new("t.tool.hand"),
        title: hand_title,
        icon: None,
        keybindings: &[KeyBinding::primary(KeyChar::H)],
        category: StaticKey::<Categories>::new("tool"),
        home: Home::CreationBar,
        description: Some(hand_description),
    };

    #[test]
    fn a_tip_reads_the_name_the_chord_and_the_description_of_its_command() {
        let tip = CommandTip::of(&HAND);
        assert_eq!(tip.title().as_str(), "Hand");
        assert_eq!(tip.chord(), HAND.chord());
        assert_eq!(
            tip.description().map(Validated::as_str),
            Some("Drag to move the view")
        );
        assert_eq!(tip.reach(), Reach::Command(HAND.id));
        let plain = Command {
            description: None,
            keybindings: &[],
            ..HAND
        };
        let tip = CommandTip::of(&plain);
        assert_eq!(tip.chord(), None);
        assert_eq!(tip.description(), None);
    }

    #[test]
    fn a_touch_held_past_the_wait_shows_the_tip_and_runs_nothing() {
        let press = Press::down(PointerKind::Touch).waited();
        assert_eq!(press, Press::Held);
        assert_eq!(press.lifted(), Press::Held);
        assert_eq!(press.lifted().click(), PressEnd::KeepTip);
    }

    #[test]
    fn a_short_tap_runs_the_command() {
        let press = Press::down(PointerKind::Touch);
        assert_eq!(press.click(), PressEnd::Run);
        assert_eq!(press.lifted(), Press::Idle);
        assert_eq!(press.lifted().waited(), Press::Idle);
        assert_eq!(press.lifted().click(), PressEnd::Run);
    }

    #[test]
    fn a_mouse_press_never_holds() {
        let press = Press::down(PointerKind::Hover).waited();
        assert_eq!(press, Press::Idle);
        assert_eq!(press.click(), PressEnd::Run);
    }

    #[test]
    fn two_tips_never_share_a_number() {
        assert_ne!(next_tip(), next_tip());
    }
}
