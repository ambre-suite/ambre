// ambre-toile/src/chrome/menu_bar.rs
// ============================================================================
// The top menu bar and zoom control (`toile-menus`), wasm-only. A
// horizontal `.ambre-commandbar` docked top-center, with one title per top-level
// menu (File / Edit / View / Object / Present). It also carries a zoom cluster:
// zoom out, a % readout, zoom in, Fit and Zoom-to-selection.
//
// Styling is SHARED with the tool palette and the Trame command bar. The two
// containers carry `.ambre-commandbar`, so buttons, hover, the active fill and
// tooltips all come from the launcher CSS. They add `.ambre-toile-menubar` /
// `.ambre-toile-zoom` modifiers that only re-dock them.
//
// The dropdowns REUSE `ambre_chrome::ContextMenu`: a title click opens it at the
// title's screen position with that menu's rows (`ui::menu_bar::menu_layout`
// mapped to `MenuEntry`s, keybinding chips included). Every selection, menu
// entry or zoom button, routes through the same `on_command` callback the
// context menu and shortcuts use. So a menu-bar "Group" and a right-click
// "Group" are one command (`toile-menus`). The zoom `%` readout mirrors the
// shared `zoom` signal the canvas view publishes each frame.
// ============================================================================

use ambre_chrome::commands::Locale;
use ambre_chrome::{Availability, ContextMenu, MenuEntry};
use ambre_core::{Board, Scale, View};
use ambre_core::{Commands, StaticKey};
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

use crate::ui::commands as registry;
use crate::ui::menu_bar::{
    MenuId, MenuRow, REDO, TOP_MENUS, UNDO, ZOOM_FIT, ZOOM_IN, ZOOM_OUT, ZOOM_RESET,
    ZOOM_TO_SELECTION, menu_layout,
};

use super::Toggles;
use super::icons;
use ambre_core::Flag as _;

/// Map the pure `MenuRow` layout to the chrome `MenuEntry` rows `ContextMenu`
/// renders. The label and the accelerator chip come from the registry
/// (`UI-THREAD-019`); the menu bar shows no leading icons, keeping the dropdowns
/// text-first. An Undo/Redo row the command store holds as disabled is rendered
/// inert (`UNDO-006`). A row the registry does not describe contributes nothing,
/// which no layout produces.
fn entries_for(id: MenuId, toggles: Toggles) -> Vec<MenuEntry> {
    menu_layout(id)
        .into_iter()
        .filter_map(|row| match row {
            MenuRow::Separator => Some(MenuEntry::separator()),
            MenuRow::Command(command) => {
                let cmd = ambre_chrome::commands::find(registry::COMMANDS, command)?;
                let entry = MenuEntry::command(cmd, &Locale::En);
                let entry = match cmd.chord() {
                    Some(kb) => entry.shortcut(kb),
                    None => entry,
                };
                // The Undo/Redo affordances are disabled from the Logic thread's
                // `UndoState` alone; the chrome counts nothing (`UNDO-006`).
                let disabled = (command == UNDO
                    && toggles.undo_enabled.get() == Availability::Disabled)
                    || (command == REDO && toggles.redo_enabled.get() == Availability::Disabled);
                Some(if disabled { entry.disabled() } else { entry })
            }
        })
        .collect()
}

/// A zoom-cluster button: a plain `.ambre-commandbar` icon button that emits
/// `command` through the shared path. Its tooltip and its glyph are the ones
/// the command declares (`UI-THREAD-019`). `mousedown` `prevent_default` keeps
/// DOM focus on the canvas text pipe (the palette / FormatBar hazard).
fn zoom_button(
    on_command: Callback<StaticKey<Commands>>,
    command: StaticKey<Commands>,
) -> impl IntoView {
    let label = registry::title(command, &Locale::En)
        .map(ambre_core::Validated::into_string)
        .unwrap_or_default();
    let icon = registry::icon(command).and_then(icons::resolve);
    view! {
        <button
            aria-label=label
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(command)
        >
            {icon.map(|ic| view! { <Icon icon=ic /> })}
        </button>
    }
}

/// The Toile top menu bar + zoom control. `zoom` is the shared live zoom factor
/// (`1.0` = 100%) the canvas view publishes. `on_command` is the shared command
/// sink, the same `Callback` the context menu uses. `toggles` carries the
/// authoritative Undo/Redo availability the menu rows read (`UNDO-006`).
#[component]
pub fn TopMenuBar(
    zoom: RwSignal<Scale<Board, View>>,
    on_command: Callback<StaticKey<Commands>>,
    toggles: Toggles,
) -> impl IntoView {
    // Which menu is open (drives the pressed title) and where its dropdown sits.
    // `ContextMenu` clears `pos` on select / Escape / click-outside; an effect
    // then clears `open_menu` so the title un-presses.
    let open_menu = RwSignal::new(None::<MenuId>);
    let pos = RwSignal::new(None::<ambre_core::Point<ambre_core::Px<ambre_core::Css>>>);
    Effect::new(move |_| {
        if pos.get().is_none() {
            open_menu.set(None);
        }
    });

    // The rows of the currently open menu (empty when closed -> nothing renders).
    // Reads the Undo/Redo signals so a disabled row updates reactively.
    let entries = Memo::new(move |_| {
        open_menu
            .get()
            .map(|id| entries_for(id, toggles))
            .unwrap_or_default()
    });

    // One title button per top-level menu. Clicking opens (or, when already open,
    // closes) its dropdown anchored at the title's bottom-left.
    let titles = TOP_MENUS
        .iter()
        .map(|(id, title)| {
            let id = *id;
            view! {
                <button
                    aria-label=*title
                    aria-haspopup="menu"
                    aria-expanded=move || ambre_chrome::Openness::from_bool(open_menu.get() == Some(id)).aria()
                    on:mousedown=move |ev| ev.prevent_default()
                    on:click=move |ev| {
                        if open_menu.get_untracked() == Some(id) {
                            pos.set(None);
                            return;
                        }
                        // Anchor the dropdown at the title's bottom-left corner.
                        let (mut x, mut y) = (0.0, 24.0);
                        if let Some(el) = ev
                            .current_target()
                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                        {
                            let r = el.get_bounding_client_rect();
                            x = r.left() as f32;
                            y = r.bottom() as f32;
                        }
                        open_menu.set(Some(id));
                        pos.set(Some(ambre_core::Point::new(
                            ambre_core::Px::<ambre_core::Css>::new(x),
                            ambre_core::Px::<ambre_core::Css>::new(y),
                        )));
                    }
                >
                    <span class="ambre-menu-title">{*title}</span>
                </button>
            }
        })
        .collect_view();

    view! {
        <div class="ambre-commandbar ambre-toile-menubar" role="menubar" aria-label="Menu">
            {titles}
        </div>
        <div class="ambre-commandbar ambre-toile-zoom" role="toolbar" aria-label="Zoom">
            {zoom_button(on_command, ZOOM_OUT)}
            <button
                class="ambre-zoom-readout"
                aria-label=registry::title(ZOOM_RESET, &Locale::En).map(ambre_core::Validated::into_string).unwrap_or_default()
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| on_command.run(ZOOM_RESET)
            >
                {move || format!("{:.0}%", (zoom.get().get() * 100.0).round())}
            </button>
            {zoom_button(on_command, ZOOM_IN)}
            <span class="sep"></span>
            {zoom_button(on_command, ZOOM_FIT)}
            {zoom_button(on_command, ZOOM_TO_SELECTION)}
        </div>
        <ContextMenu pos=pos entries=entries on_command=on_command />
    }
}
