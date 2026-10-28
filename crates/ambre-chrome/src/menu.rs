// ambre-chrome/src/menu.rs
// What a screen reader and a keyboard need from every menu and toggle
// (`ambre-spec-accessibility.md`). A two-state attribute always carries its
// value, false included. And one key map drives every menu in the suite, so
// no menu is operable by mouse alone.

use ambre_commands::{Key, Modifiers};
use wasm_bindgen::JsCast;

ambre_core::define_flag! {
    /// Whether a control's menu or panel is open, as `aria-expanded` reads it.
    /// The attribute always carries its value: an absent `aria-expanded` says
    /// "this opens nothing" (`CHROME-066`).
    pub enum Openness {
        /// The menu or panel is closed.
        Closed,
        /// The menu or panel is open.
        Open,
    }

    /// Whether an option is the one the keyboard is on, as `aria-selected`
    /// reads it. The attribute always carries its value.
    pub enum Selected {
        /// Another option is selected.
        Off,
        /// This option is selected.
        On,
    }

    /// Whether a radio item of a menu is the chosen one, as `aria-checked`
    /// reads it. The attribute always carries its value.
    pub enum Checked {
        /// Another item is chosen.
        Off,
        /// This item is chosen.
        On,
    }
}

impl Openness {
    /// The `aria-expanded` value: `"true"` or `"false"`, never absent.
    pub const fn aria(self) -> &'static str {
        match self {
            Self::Closed => "false",
            Self::Open => "true",
        }
    }
}

impl Checked {
    /// The `aria-checked` value: `"true"` or `"false"`, never absent.
    pub const fn aria(self) -> &'static str {
        match self {
            Self::Off => "false",
            Self::On => "true",
        }
    }
}

impl Selected {
    /// The `aria-selected` value: `"true"` or `"false"`, never absent.
    pub const fn aria(self) -> &'static str {
        match self {
            Self::Off => "false",
            Self::On => "true",
        }
    }
}

/// What a key does in an open menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKey {
    /// Move the focus to the item at this index.
    Focus(usize),
    /// Choose the item at this index.
    Choose(usize),
    /// Close the menu.
    Dismiss,
    /// Not a menu key: leave the event alone.
    Pass,
}

/// Map a key to its effect in a menu whose focus is on item `at`. `initials`
/// holds each item's first letter, lowercased, so its length is the item count.
///
/// The arrows and Tab move and wrap, Home and End jump to an end, and a letter
/// jumps to the next item it starts. Enter and Space choose, and Escape
/// dismisses (`CHROME-026`).
pub fn menu_key(key: Key, mods: Modifiers, at: usize, initials: &[char]) -> MenuKey {
    let len = initials.len();
    if len == 0 {
        return if key == Key::Escape {
            MenuKey::Dismiss
        } else {
            MenuKey::Pass
        };
    }
    let at = at.min(len - 1);
    let next = (at + 1) % len;
    let prev = (at + len - 1) % len;
    match key {
        Key::ArrowDown => MenuKey::Focus(next),
        Key::ArrowUp => MenuKey::Focus(prev),
        // A menu is a transient popup: Tab cycles inside it rather than
        // letting focus escape to the page behind.
        Key::Tab => MenuKey::Focus(if mods.shift() { prev } else { next }),
        Key::Home => MenuKey::Focus(0),
        Key::End => MenuKey::Focus(len - 1),
        Key::Escape => MenuKey::Dismiss,
        Key::Enter | Key::Space => MenuKey::Choose(at),
        // The key holds its letter lowercased already.
        Key::Char(ch) => (1..=len)
            .map(|off| (at + off) % len)
            .find(|&i| initials.get(i) == Some(&ch.get()))
            .map_or(MenuKey::Pass, MenuKey::Focus),
        _ => MenuKey::Pass,
    }
}

/// The effect of the keyboard event `ev` in a menu whose focus is on item
/// `at`. A key [`Key`] does not model passes.
pub(crate) fn event_menu_key(ev: &web_sys::KeyboardEvent, at: usize, initials: &[char]) -> MenuKey {
    let mods = crate::command::modifiers_of(ev);
    Key::from_dom(&ev.key()).map_or(MenuKey::Pass, |key| menu_key(key, mods, at, initials))
}

/// The lowercased first letter of a label, or a space for an empty one.
pub fn initial(label: &str) -> char {
    label
        .trim()
        .chars()
        .next()
        .unwrap_or(' ')
        .to_ascii_lowercase()
}

/// The items of a rendered menu, in order: every element whose role starts
/// with `menuitem`.
fn items(menu: &web_sys::Element) -> Vec<web_sys::HtmlElement> {
    let Ok(list) = menu.query_selector_all("[role^=\"menuitem\"]") else {
        return Vec::new();
    };
    (0..list.length())
        .filter_map(|i| list.item(i)?.dyn_into::<web_sys::HtmlElement>().ok())
        .collect()
}

thread_local! {
    /// The menu that took the focus last, and what held it before. One slot
    /// is enough: the focus sits in one menu at a time.
    static OPENER: std::cell::RefCell<Option<(web_sys::Element, web_sys::HtmlElement)>> =
        const { std::cell::RefCell::new(None) };
}

/// Focus the first item of a menu that has just opened. What held the focus
/// gets it back when a key closes the menu (`CHROME-027`).
pub fn focus_first_item(menu: &web_sys::Element) {
    let active = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .filter(|el| !menu.contains(Some(el)))
        .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
    if let Some(active) = active {
        OPENER.with(|slot| *slot.borrow_mut() = Some((menu.clone(), active)));
    }
    if let Some(first) = items(menu).first() {
        let _ = first.focus();
    }
}

/// Give the focus back to what held it before `menu` opened, if it is still
/// on the page.
fn return_focus(menu: &web_sys::Element) {
    let opener = OPENER.with(|slot| {
        let mut slot = slot.borrow_mut();
        match slot.take() {
            Some((opened, opener)) if opened == *menu => Some(opener),
            other => {
                *slot = other;
                None
            }
        }
    });
    if let Some(opener) = opener
        && opener.is_connected()
    {
        let _ = opener.focus();
    }
}

/// Drive a rendered menu from a keydown on it, through [`menu_key`]. The items
/// are read off the page, so the menu keeps no index of its own. A chosen item
/// is clicked, so the choice runs the item's own handler. `dismiss` closes it.
pub fn on_menu_keydown(
    ev: &web_sys::KeyboardEvent,
    menu: &web_sys::Element,
    dismiss: impl FnOnce(),
) {
    let items = items(menu);
    let active = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element());
    let at = items
        .iter()
        .position(|item| active.as_ref() == Some(item.as_ref()))
        .unwrap_or(0);
    // An item's accessible name is its `aria-label` when it has one, so an
    // item opening on a decorative glyph still answers to its word.
    let initials: Vec<char> = items
        .iter()
        .map(|item| {
            let name = item
                .get_attribute("aria-label")
                .or_else(|| item.text_content())
                .unwrap_or_default();
            initial(&name)
        })
        .collect();
    match event_menu_key(ev, at, &initials) {
        MenuKey::Focus(i) => {
            ev.prevent_default();
            if let Some(item) = items.get(i) {
                let _ = item.focus();
            }
        }
        MenuKey::Choose(i) => {
            ev.prevent_default();
            if let Some(item) = items.get(i)
                && item.get_attribute("aria-disabled").as_deref() != Some("true")
            {
                item.click();
                return_focus(menu);
            }
        }
        MenuKey::Dismiss => {
            ev.prevent_default();
            dismiss();
            return_focus(menu);
        }
        MenuKey::Pass => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_commands::FunctionKey;

    const ITEMS: [char; 4] = ['c', 'p', 'c', 'd'];

    /// The arrows and Tab move and wrap at both ends.
    #[test]
    fn the_arrows_and_tab_move_and_wrap() {
        assert_eq!(
            menu_key(Key::ArrowDown, Modifiers::NONE, 0, &ITEMS),
            MenuKey::Focus(1)
        );
        assert_eq!(
            menu_key(Key::ArrowDown, Modifiers::NONE, 3, &ITEMS),
            MenuKey::Focus(0)
        );
        assert_eq!(
            menu_key(Key::ArrowUp, Modifiers::NONE, 0, &ITEMS),
            MenuKey::Focus(3)
        );
        assert_eq!(
            menu_key(Key::Tab, Modifiers::NONE, 1, &ITEMS),
            MenuKey::Focus(2)
        );
        assert_eq!(
            menu_key(Key::Tab, Modifiers::SHIFT, 0, &ITEMS),
            MenuKey::Focus(3)
        );
    }

    /// Home and End jump to an end.
    #[test]
    fn home_and_end_jump_to_an_end() {
        assert_eq!(
            menu_key(Key::Home, Modifiers::NONE, 2, &ITEMS),
            MenuKey::Focus(0)
        );
        assert_eq!(
            menu_key(Key::End, Modifiers::NONE, 0, &ITEMS),
            MenuKey::Focus(3)
        );
    }

    /// A letter jumps to the next item it starts, wrapping, and in either case.
    #[test]
    fn a_letter_jumps_to_the_next_item_it_starts() {
        assert_eq!(
            menu_key(Key::char('c'), Modifiers::NONE, 0, &ITEMS),
            MenuKey::Focus(2)
        );
        assert_eq!(
            menu_key(Key::char('C'), Modifiers::NONE, 2, &ITEMS),
            MenuKey::Focus(0)
        );
        assert_eq!(
            menu_key(Key::char('d'), Modifiers::NONE, 0, &ITEMS),
            MenuKey::Focus(3)
        );
        assert_eq!(
            menu_key(Key::char('z'), Modifiers::NONE, 0, &ITEMS),
            MenuKey::Pass
        );
    }

    /// Enter and Space choose the focused item, and Escape dismisses.
    #[test]
    fn enter_and_space_choose_and_escape_dismisses() {
        assert_eq!(
            menu_key(Key::Enter, Modifiers::NONE, 1, &ITEMS),
            MenuKey::Choose(1)
        );
        assert_eq!(
            menu_key(Key::Space, Modifiers::NONE, 2, &ITEMS),
            MenuKey::Choose(2)
        );
        assert_eq!(
            menu_key(Key::Escape, Modifiers::NONE, 1, &ITEMS),
            MenuKey::Dismiss
        );
        assert_eq!(
            menu_key(Key::F(FunctionKey::F1), Modifiers::NONE, 1, &ITEMS),
            MenuKey::Pass
        );
    }

    /// An empty menu can still be dismissed, and moves nowhere.
    #[test]
    fn an_empty_menu_can_only_be_dismissed() {
        assert_eq!(
            menu_key(Key::Escape, Modifiers::NONE, 0, &[]),
            MenuKey::Dismiss
        );
        assert_eq!(
            menu_key(Key::ArrowDown, Modifiers::NONE, 0, &[]),
            MenuKey::Pass
        );
    }

    /// A two-state attribute says false rather than vanishing.
    #[test]
    fn a_two_state_attribute_always_has_a_value() {
        assert_eq!(Openness::Open.aria(), "true");
        assert_eq!(Openness::Closed.aria(), "false");
        assert_eq!(Selected::On.aria(), "true");
        assert_eq!(Selected::Off.aria(), "false");
        assert_eq!(Checked::On.aria(), "true");
        assert_eq!(Checked::Off.aria(), "false");
        assert_eq!(crate::Pressed::On.aria(), "true");
        assert_eq!(crate::Pressed::Off.aria(), "false");
        // Each flag reads back as itself through its stored `bool`, and a
        // toggle's `bool` reads as the pressed state of the same name.
        for flag in [Openness::Open, Openness::Closed] {
            assert_eq!(Openness::from_bool(flag.as_bool()), flag);
        }
        for flag in [Selected::On, Selected::Off] {
            assert_eq!(Selected::from_bool(flag.as_bool()), flag);
        }
        for flag in [Checked::On, Checked::Off] {
            assert_eq!(Checked::from_bool(flag.as_bool()), flag);
        }
        assert_eq!(
            crate::Pressed::from_bool(Checked::On.as_bool()),
            crate::Pressed::On
        );
        assert_eq!(
            crate::Pressed::from_bool(Checked::Off.as_bool()),
            crate::Pressed::Off
        );
    }
}
