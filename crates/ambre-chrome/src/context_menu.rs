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

/// One entry: a command to invoke and its display label.
#[derive(Clone)]
pub struct MenuEntry {
    pub command: CommandId,
    pub label: String,
}

impl MenuEntry {
    pub fn new(command: CommandId, label: impl Into<String>) -> Self {
        Self { command, label: label.into() }
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

    // On open: reset highlight, then place at the click point and flip if the
    // menu would overflow the viewport (measured after it renders).
    Effect::new(move |_| {
        let Some((x, y)) = pos.get() else { return };
        focused.set(0);
        let (mut lx, mut ty) = (x, y);
        if let Some(menu) = menu_ref.get() {
            let rect = menu.get_bounding_client_rect();
            let win = web_sys::window().unwrap();
            let vw = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(rect.width());
            let vh = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(rect.height());
            if lx + rect.width() > vw {
                lx = (lx - rect.width()).max(0.0);
            }
            if ty + rect.height() > vh {
                ty = (ty - rect.height()).max(0.0);
            }
        }
        menu_x.set(lx);
        menu_y.set(ty);
    });

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
        if pos.get().is_none() {
            return None;
        }

        let len = commands.len();
        let cmds = commands.clone();
        let on_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
            "ArrowDown" => {
                ev.prevent_default();
                focused.update(|i| *i = (*i + 1) % len.max(1));
            }
            "ArrowUp" => {
                ev.prevent_default();
                focused.update(|i| *i = (*i + len.saturating_sub(1)) % len.max(1));
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
            _ => {}
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
            // Full-viewport layer: left-click closes, right-click repositions.
            <div
                class="ambre-ctx-backdrop"
                on:click=move |_| pos.set(None)
                on:contextmenu=move |e: web_sys::MouseEvent| {
                    e.prevent_default();
                    pos.set(Some((e.client_x() as f64, e.client_y() as f64)));
                }
            ></div>
            <ul
                node_ref=menu_ref
                class="ambre-ctx-menu"
                role="menu"
                tabindex="-1"
                style=move || format!("left:{}px; top:{}px;", menu_x.get(), menu_y.get())
                on:keydown=on_keydown
            >
                {items}
            </ul>
        })
    }
}
