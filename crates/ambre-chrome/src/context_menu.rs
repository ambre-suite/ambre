// ambre-chrome/src/context_menu.rs
// ============================================================================
// DOM context menu (ambre-spec-chrome.md, CHROME-003).
//
// Opened from the canvas right-click: the launcher's wgpu-side handler reads
// the click point and sets `pos` (the Canvas -> DOM signal). This
// component renders a `role="menu"` popover at those viewport coordinates,
// flipped if it would overflow, with arrow/Home/End/Enter/Escape keyboard
// navigation and close on Escape / click-outside.
//
// Items are registry commands. Selecting one invokes a `StaticKey<Commands>` through
// `on_command`, the same path as the toolbar and shortcuts. A right-click
// "Bold" therefore runs the exact same command, decoupled from the trigger.
// ============================================================================

use ambre_commands::{Command, KeyBinding, Locale};
use ambre_core::{Commands, StaticKey};
use ambre_core::{Css, Point, Px, Vector};
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;

use crate::menu::MenuKey;
use crate::shortcut::Kbd;
use ambre_core::index::Pos;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

/// One row of the context menu: either a selectable command item (optionally
/// iconed) or a non-interactive group separator (`trame-context-menu`, "grouped,
/// iconed entries with separators"). A separator is skipped by keyboard
/// navigation and carries no command.
#[derive(Clone, PartialEq)]
pub enum MenuEntry {
    /// A selectable command. See [`MenuItem`].
    Item(MenuItem),
    /// A horizontal divider between two groups of items.
    Separator,
}

/// A selectable command row, with an optional leading icon and an optional
/// trailing keybinding chip (shown right-aligned, for example a menu-bar
/// accelerator). A disabled row renders inert: it announces `aria-disabled`
/// and fires no command (`UNDO-006`).
///
/// The fields are private and the one constructor takes the command's
/// description, so a menu cannot show a label of its own (`UI-THREAD-036`).
#[derive(Clone, PartialEq)]
pub struct MenuItem {
    command: StaticKey<Commands>,
    label: String,
    icon: Option<icondata::Icon>,
    shortcut: Option<KeyBinding>,
    disabled: bool,
}

impl MenuItem {
    /// The command the row invokes.
    #[must_use]
    pub const fn command(&self) -> StaticKey<Commands> {
        self.command
    }

    /// The label the row shows, read from the command's description.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

impl MenuEntry {
    /// The entry for `command`, labelled in `locale` from its description.
    /// No icon and no chip until a builder below adds one.
    #[must_use]
    pub fn command(command: &Command, locale: &Locale) -> Self {
        Self::Item(MenuItem {
            command: command.id,
            label: (command.title)(locale).into_string(),
            icon: None,
            shortcut: None,
            disabled: false,
        })
    }

    /// Attach a leading icon to an `Item` (no-op on a separator). The icon
    /// is the one the application resolves from the command's icon handle.
    #[must_use]
    pub fn icon(mut self, icon: icondata::Icon) -> Self {
        if let Self::Item(item) = &mut self {
            item.icon = Some(icon);
        }
        self
    }

    /// Attach a trailing keybinding chip to an `Item` (no-op on a separator).
    /// A menu-bar row shows its accelerator (`toile-menus`), and a
    /// context-menu row stays chip-less.
    #[must_use]
    pub fn shortcut(mut self, binding: KeyBinding) -> Self {
        if let Self::Item(item) = &mut self {
            item.shortcut = Some(binding);
        }
        self
    }

    /// Mark an `Item` inert: it renders with `aria-disabled`, fires no command
    /// and answers no keyboard selection (`UNDO-006`). No-op on a separator.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        if let Self::Item(item) = &mut self {
            item.disabled = true;
        }
        self
    }

    /// A group separator.
    pub const fn separator() -> Self {
        Self::Separator
    }
}

/// Context menu opened at a viewport point. `pos` is `Some(point)` to open,
/// `None` to close; the component flips it back to `None` on Escape, selection
/// and click-outside.
#[component]
pub fn ContextMenu(
    pos: RwSignal<Option<Point<Px<Css>>>>,
    /// The menu rows, reactive so the launcher can swap a cell-scoped layout for
    /// a header-scoped one between right-clicks (`trame-context-menu`).
    #[prop(into)]
    entries: Signal<Vec<MenuEntry>>,
    on_command: Callback<StaticKey<Commands>>,
) -> impl IntoView {
    let menu_ref = NodeRef::<html::Ul>::new();
    let focused = RwSignal::new(0usize);
    let menu_at = RwSignal::new(Point::default());

    // The menu takes the focus, so it gives it back to what held it before it
    // opened (`CHROME-027`). Created before the effect that moves the focus in,
    // so the opener is read first. A focus moved elsewhere while the menu
    // was open is the user's, and stays.
    let opener = std::rc::Rc::new(std::cell::RefCell::new(None::<web_sys::HtmlElement>));
    Effect::new(move |was_open: Option<bool>| {
        let open = pos.with(Option::is_some);
        let doc = web_sys::window().and_then(|w| w.document());
        let active = doc.as_ref().and_then(web_sys::Document::active_element);
        let in_menu = |el: &web_sys::Element| {
            menu_ref
                .get_untracked()
                .is_some_and(|menu| menu.contains(Some(el)))
        };
        match (was_open.unwrap_or(false), open) {
            (false, true) => {
                *opener.borrow_mut() = active
                    .filter(|el| !in_menu(el))
                    .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
            }
            (true, false) => {
                let left_on_page = active.is_none_or(|el| {
                    in_menu(&el)
                        || doc
                            .as_ref()
                            .and_then(|d| d.body())
                            .is_some_and(|b| *b == el)
                });
                if let Some(el) = opener.borrow_mut().take()
                    && left_on_page
                    && el.is_connected()
                {
                    let _ = el.focus();
                }
            }
            _ => {}
        }
        open
    });

    // On open: reset highlight, then place at the click point and flip if the
    // menu would overflow the viewport (measured after it renders).
    Effect::new(move |_| {
        let Some(at) = pos.get() else { return };
        focused.set(0);
        let mut placed = at;
        if let Some(menu) = menu_ref.get()
            && let Some(win) = web_sys::window()
        {
            let rect = menu.get_bounding_client_rect();
            let vw = win
                .inner_width()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(rect.width());
            let vh = win
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(rect.height());
            // The DOM measures in f64 CSS px; a menu is placed in f32 ones.
            let css = |v: f64| Px::<Css>::new(v as f32);
            let size = Vector::new(css(rect.width()), css(rect.height()));
            placed = place_menu(at, size, Point::new(css(vw), css(vh)));
        }
        menu_at.set(placed);
    });

    // Close on any pointerdown outside the menu. A document-level capture
    // listener (rather than a full-viewport backdrop) lets the click reach
    // whatever is underneath. A right-click on the canvas reaches the canvas, so
    // it re-selects the cell (pointerdown) and reopens the menu at the new point
    // (contextmenu). A backdrop would have swallowed both -- the menu moved but
    // the selection stayed on the old cell. Registered once per mount; it no-ops
    // while closed or unmounted, and ignores clicks inside the menu.
    // Without a document, there is no outside to click: the menu still closes
    // on Escape and on a command.
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        let cb =
            Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
                // The listener outlives the menu: a menu bar unmounts its
                // menu on each presentation, and `pos` is disposed then.
                if pos.try_get_untracked().flatten().is_none() {
                    return;
                }
                // A click inside the menu is the item handlers' business; ignore it.
                if let (Some(menu), Some(target)) = (menu_ref.get_untracked(), e.target())
                    && let Ok(node) = target.dyn_into::<web_sys::Node>()
                    && menu.contains(Some(&node))
                {
                    return;
                }
                pos.set(None);
            });
        let _ = doc.add_event_listener_with_callback_and_bool(
            "pointerdown",
            cb.as_ref().unchecked_ref(),
            true,
        );
        cb.forget();
    }

    // Move DOM focus to the highlighted item (roving tabindex).
    Effect::new(move |_| {
        if pos.get().is_none() {
            return;
        }
        let idx = focused.get();
        if let Some(menu) = menu_ref.get()
            && let Ok(Some(el)) = menu.query_selector(&format!("[data-idx=\"{idx}\"]"))
            && let Ok(h) = el.dyn_into::<web_sys::HtmlElement>()
        {
            let _ = h.focus();
        }
    });

    move || {
        pos.get()?;

        // Only `Item` rows are focusable. Build the parallel command + typeahead
        // vectors over them, skipping separators, so keyboard nav and the roving
        // `data-idx` index the items, not the raw rows.
        let rows = entries.get();
        let cmds: Vec<StaticKey<Commands>> = rows
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item(item) => Some(item.command),
                MenuEntry::Separator => None,
            })
            .collect();
        // The inert flag for each item, parallel to `cmds`, so Enter/Space can
        // refuse a disabled row the same way a click does (`UNDO-006`).
        let disabled: Vec<bool> = rows
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item(item) => Some(item.disabled),
                MenuEntry::Separator => None,
            })
            .collect();
        // Lowercased first chars for first-letter typeahead (WAI-ARIA menu pattern).
        let initials: Vec<char> = rows
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item(item) => Some(crate::menu::initial(&item.label)),
                MenuEntry::Separator => None,
            })
            .collect();

        // The shared key map decides; this menu keeps its own focus index
        // because its items are commands, not clickable DOM handlers.
        let on_keydown = move |ev: web_sys::KeyboardEvent| {
            let at = focused.get_untracked();
            match crate::menu::event_menu_key(&ev, Pos::from_usize(at), &initials) {
                MenuKey::Focus(i) => {
                    ev.prevent_default();
                    focused.set(i.to_usize());
                }
                MenuKey::Dismiss => pos.set(None),
                MenuKey::Choose(i) => {
                    let i = i.to_usize();
                    ev.prevent_default();
                    if disabled.get(i).copied().unwrap_or(false) {
                        return;
                    }
                    if let Some(&cmd) = cmds.get(i) {
                        on_command.run(cmd);
                    }
                    pos.set(None);
                }
                MenuKey::Pass => {}
            }
        };

        // Render rows in order. A separator is a non-interactive divider; an item
        // carries the next focusable index (`data-idx`), used by the roving
        // tabindex and typeahead above.
        let mut focus_idx = 0usize;
        let items = rows
            .into_iter()
            .map(|e| match e {
                MenuEntry::Separator => view! {
                    <li class="ambre-ctx-sep" role="separator"></li>
                }
                .into_any(),
                MenuEntry::Item(MenuItem {
                    command,
                    label,
                    icon,
                    shortcut,
                    disabled: is_disabled,
                }) => {
                    let i = focus_idx;
                    focus_idx += 1;
                    view! {
                        <li
                            class="ambre-ctx-item"
                            class:ambre-ctx-disabled=is_disabled
                            role="menuitem"
                            aria-disabled=is_disabled.then_some("true")
                            tabindex="-1"
                            data-idx=i.to_string()
                            on:click=move |_| {
                                if is_disabled {
                                    return;
                                }
                                on_command.run(command);
                                pos.set(None);
                            }
                            // `mousemove`, not `mouseenter`: Firefox fires the
                            // latter when a menu opens under a pointer at rest.
                            // That took the focus off the first entry of a menu
                            // opened from the keyboard.
                            on:mousemove=move |_| {
                                if focused.get_untracked() != i {
                                    focused.set(i);
                                }
                            }
                        >
                            {icon.map(|ic| view! { <span class="ambre-ctx-icon"><Icon icon=ic /></span> })}
                            <span class="ambre-ctx-label">{label}</span>
                            {shortcut.map(|kb| view! { <span class="ambre-ctx-kbd"><Kbd binding=kb /></span> })}
                        </li>
                    }
                    .into_any()
                }
            })
            .collect_view();

        Some(view! {
            <ul
                node_ref=menu_ref
                class="ambre-ctx-menu"
                role="menu"
                aria-orientation="vertical"
                aria-label="Context menu"
                tabindex="-1"
                style:left=move || format!("{}px", menu_at.get().x.get())
                style:top=move || format!("{}px", menu_at.get().y.get())
                on:keydown=on_keydown
            >
                {items}
            </ul>
        })
    }
}

/// Place a menu of extent `size` opened at click point `at`, in a viewport
/// whose far corner is `viewport`. Each axis that would overflow is flipped,
/// so the menu stays visible.
///
/// Pure geometry (no DOM). The `Effect` above reads the real rect/viewport from
/// `web-sys` and delegates the decision here, so it is host-testable. The
/// `cfg(wasm32)` boundary stays at the DOM read. On the flipped axis the menu is
/// anchored at `point - size`, clamped to `0`. A menu taller/wider than the
/// viewport pins to the top/left edge rather than going off-screen.
pub(crate) fn place_menu(
    at: Point<Px<Css>>,
    size: Vector<Px<Css>>,
    viewport: Point<Px<Css>>,
) -> Point<Px<Css>> {
    let far = at + size;
    let back = at - size;
    Point::new(
        if far.x > viewport.x {
            back.x.max(Px::<Css>::ZERO)
        } else {
            at.x
        },
        if far.y > viewport.y {
            back.y.max(Px::<Css>::ZERO)
        } else {
            at.y
        },
    )
}

#[cfg(test)]
mod tests {
    use super::place_menu;
    use ambre_core::{Css, Point, Px, Vector};

    /// The point at `(x, y)`, in CSS px.
    fn pt(x: f32, y: f32) -> Point<Px<Css>> {
        Point::new(Px::<Css>::new(x), Px::<Css>::new(y))
    }

    /// Place a `w` x `h` menu at `(x, y)` in a `vw` x `vh` viewport.
    fn place(x: f32, y: f32, w: f32, h: f32, vw: f32, vh: f32) -> Point<Px<Css>> {
        place_menu(
            pt(x, y),
            Vector::new(Px::<Css>::new(w), Px::<Css>::new(h)),
            pt(vw, vh),
        )
    }

    #[test]
    fn no_overflow_keeps_click_point() {
        assert_eq!(
            place(10.0, 20.0, 100.0, 200.0, 1000.0, 800.0),
            pt(10.0, 20.0)
        );
    }

    #[test]
    fn right_overflow_flips_horizontally() {
        // x + w = 960 + 100 = 1060 > 1000 -> anchor at x - w = 860.
        assert_eq!(
            place(960.0, 20.0, 100.0, 200.0, 1000.0, 800.0),
            pt(860.0, 20.0)
        );
    }

    #[test]
    fn bottom_overflow_flips_vertically() {
        // y + h = 700 + 200 = 900 > 800 -> anchor at y - h = 500.
        assert_eq!(
            place(10.0, 700.0, 100.0, 200.0, 1000.0, 800.0),
            pt(10.0, 500.0)
        );
    }

    #[test]
    fn corner_overflow_flips_both_axes() {
        assert_eq!(
            place(960.0, 700.0, 100.0, 200.0, 1000.0, 800.0),
            pt(860.0, 500.0)
        );
    }

    #[test]
    fn menu_larger_than_viewport_pins_to_origin() {
        // Flipping would push negative; clamp to 0 (pin to top/left edge).
        assert_eq!(
            place(50.0, 50.0, 2000.0, 2000.0, 1000.0, 800.0),
            pt(0.0, 0.0)
        );
    }
}
