// ambre-chrome/src/context_menu.rs
// ============================================================================
// DOM context menu (ambre-spec-chrome.md, 2.B).
//
// Opened from the canvas right-click: the launcher's wgpu-side handler reads
// the click point and sets `pos` (the Canvas -> DOM signal channel). This
// component renders a `role="menu"` popover at those viewport coordinates,
// flipped if it would overflow, with arrow/Home/End/Enter/Escape keyboard
// navigation and close on Escape / click-outside.
//
// Items are registry commands: selecting one invokes a `CommandId` through
// `on_command`, the same path as the toolbar and shortcuts -- so a right-click
// "Bold" runs the exact same command, decoupled from the trigger.
// ============================================================================

use ambre_commands::CommandId;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

/// One entry: a command to invoke and its display label.
#[derive(Clone)]
pub struct MenuEntry {
    pub command: CommandId,
    pub label: String,
}

impl MenuEntry {
    pub fn new(command: CommandId, label: impl Into<String>) -> Self {
        Self {
            command,
            label: label.into(),
        }
    }
}

/// Context menu opened at viewport coordinates. `pos` is `Some((x, y))` to open,
/// `None` to close; the component flips it back to `None` on Escape, selection
/// and click-outside.
#[component]
pub fn ContextMenu(
    pos: RwSignal<Option<(f64, f64)>>,
    entries: Vec<MenuEntry>,
    on_command: Callback<CommandId>,
) -> impl IntoView {
    let menu_ref = NodeRef::<html::Ul>::new();
    let focused = RwSignal::new(0usize);
    let menu_x = RwSignal::new(0.0_f64);
    let menu_y = RwSignal::new(0.0_f64);
    let commands: Vec<CommandId> = entries.iter().map(|e| e.command).collect();
    // Lowercased first chars for first-letter typeahead (WAI-ARIA menu pattern).
    let initials: Vec<char> = entries
        .iter()
        .map(|e| e.label.chars().next().unwrap_or(' ').to_ascii_lowercase())
        .collect();

    // On open: reset highlight, then place at the click point and flip if the
    // menu would overflow the viewport (measured after it renders).
    Effect::new(move |_| {
        let Some((x, y)) = pos.get() else { return };
        focused.set(0);
        let (mut lx, mut ty) = (x, y);
        if let Some(menu) = menu_ref.get() {
            let rect = menu.get_bounding_client_rect();
            let win = web_sys::window().unwrap();
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
            (lx, ty) = place_menu(x, y, rect.width(), rect.height(), vw, vh);
        }
        menu_x.set(lx);
        menu_y.set(ty);
    });

    // Close on any pointerdown outside the menu. A document-level capture
    // listener (rather than a full-viewport backdrop) lets the click reach
    // whatever is underneath: a right-click on the canvas reaches the canvas, so
    // it re-selects the cell (pointerdown) and reopens the menu at the new point
    // (contextmenu). A backdrop would have swallowed both -- the menu moved but
    // the selection stayed on the old cell. Registered once (the menu is a shell
    // singleton); it no-ops while closed and ignores clicks inside the menu.
    {
        let doc = web_sys::window().unwrap().document().unwrap();
        let cb =
            Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
                if pos.get_untracked().is_none() {
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

        let len = commands.len();
        let cmds = commands.clone();
        let initials = initials.clone();
        let on_keydown = move |ev: web_sys::KeyboardEvent| {
            let key = ev.key();
            match key.as_str() {
                "ArrowDown" => {
                    ev.prevent_default();
                    focused.update(|i| *i = (*i + 1) % len.max(1));
                }
                "ArrowUp" => {
                    ev.prevent_default();
                    focused.update(|i| *i = (*i + len.saturating_sub(1)) % len.max(1));
                }
                // Tab is trapped inside the menu (it is a transient popup): cycle
                // like the arrows rather than letting focus escape to the page.
                "Tab" => {
                    ev.prevent_default();
                    if ev.shift_key() {
                        focused.update(|i| *i = (*i + len.saturating_sub(1)) % len.max(1));
                    } else {
                        focused.update(|i| *i = (*i + 1) % len.max(1));
                    }
                }
                "Home" => {
                    ev.prevent_default();
                    focused.set(0);
                }
                "End" => {
                    ev.prevent_default();
                    focused.set(len.saturating_sub(1));
                }
                "Escape" => pos.set(None),
                "Enter" | " " => {
                    ev.prevent_default();
                    if let Some(&cmd) = cmds.get(focused.get_untracked()) {
                        on_command.run(cmd);
                    }
                    pos.set(None);
                }
                // First-letter typeahead: jump to the next item whose label
                // starts with the typed character, wrapping around.
                _ if key.chars().count() == 1 => {
                    let ch = key.chars().next().unwrap().to_ascii_lowercase();
                    let start = focused.get_untracked();
                    if let Some(off) =
                        (1..=len).find(|&o| initials.get((start + o) % len.max(1)) == Some(&ch))
                    {
                        focused.set((start + off) % len.max(1));
                    }
                }
                _ => {}
            }
        };

        let items = entries
            .clone()
            .into_iter()
            .enumerate()
            .map(|(i, e)| {
                let cmd = e.command;
                view! {
                    <li
                        class="ambre-ctx-item"
                        role="menuitem"
                        tabindex="-1"
                        data-idx=i.to_string()
                        on:click=move |_| {
                            on_command.run(cmd);
                            pos.set(None);
                        }
                        on:mouseenter=move |_| focused.set(i)
                    >
                        {e.label}
                    </li>
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
                style=move || format!("left:{}px; top:{}px;", menu_x.get(), menu_y.get())
                on:keydown=on_keydown
            >
                {items}
            </ul>
        })
    }
}

/// Place a `w` x `h` menu opened at click point `(x, y)` inside a `vw` x `vh`
/// viewport, flipping each axis that would overflow so the menu stays visible.
///
/// Pure geometry (no DOM): the `Effect` above reads the real rect/viewport from
/// `web-sys` and delegates the decision here so it is host-testable (the
/// `cfg(wasm32)` boundary stays at the DOM read). On the flipped axis the menu
/// is anchored at `point - size`, clamped to `0` so a menu taller/wider than the
/// viewport pins to the top/left edge rather than going off-screen.
pub(crate) fn place_menu(x: f64, y: f64, w: f64, h: f64, vw: f64, vh: f64) -> (f64, f64) {
    let lx = if x + w > vw { (x - w).max(0.0) } else { x };
    let ty = if y + h > vh { (y - h).max(0.0) } else { y };
    (lx, ty)
}

#[cfg(test)]
mod tests {
    use super::place_menu;

    #[test]
    fn no_overflow_keeps_click_point() {
        assert_eq!(
            place_menu(10.0, 20.0, 100.0, 200.0, 1000.0, 800.0),
            (10.0, 20.0)
        );
    }

    #[test]
    fn right_overflow_flips_horizontally() {
        // x + w = 960 + 100 = 1060 > 1000 -> anchor at x - w = 860.
        assert_eq!(
            place_menu(960.0, 20.0, 100.0, 200.0, 1000.0, 800.0),
            (860.0, 20.0)
        );
    }

    #[test]
    fn bottom_overflow_flips_vertically() {
        // y + h = 700 + 200 = 900 > 800 -> anchor at y - h = 500.
        assert_eq!(
            place_menu(10.0, 700.0, 100.0, 200.0, 1000.0, 800.0),
            (10.0, 500.0)
        );
    }

    #[test]
    fn corner_overflow_flips_both_axes() {
        assert_eq!(
            place_menu(960.0, 700.0, 100.0, 200.0, 1000.0, 800.0),
            (860.0, 500.0)
        );
    }

    #[test]
    fn menu_larger_than_viewport_pins_to_origin() {
        // Flipping would push negative; clamp to 0 (pin to top/left edge).
        assert_eq!(
            place_menu(50.0, 50.0, 2000.0, 2000.0, 1000.0, 800.0),
            (0.0, 0.0)
        );
    }
}
