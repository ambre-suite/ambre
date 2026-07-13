// ambre-toile/src/chrome/menu_bar.rs
// ============================================================================
// The top menu bar and zoom control (toile-ui SS5quinquies.E), wasm-only. A
// horizontal `.ambre-commandbar` docked top-center with one title per top-level
// menu (File / Edit / View / Object / Present) and a zoom cluster (zoom out / %
// readout / zoom in / Fit / Zoom-to-selection).
//
// Styling is SHARED with the tool palette and the Trame command bar: the two
// containers carry `.ambre-commandbar` (so buttons, hover, the active fill and
// tooltips all come from the launcher CSS) plus `.ambre-toile-menubar` /
// `.ambre-toile-zoom` modifiers that only re-dock them.
//
// The dropdowns REUSE `ambre_chrome::ContextMenu`: a title click opens it at the
// title's screen position with that menu's rows (`ui::menu_bar::menu_layout`
// mapped to `MenuEntry`s, keybinding chips included). Every selection -- menu
// entry or zoom button -- routes through the same `on_command` callback the
// context menu and shortcuts use, so a menu-bar "Group" and a right-click "Group"
// are one command (SS5quinquies). The zoom `%` readout mirrors the shared `zoom`
// signal the canvas view publishes each frame.
// ============================================================================

use ambre_chrome::commands::CommandId;
use ambre_chrome::{ContextMenu, MenuEntry};
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

use crate::ui::menu_bar::{
    MenuId, MenuRow, TOP_MENUS, ZOOM_FIT, ZOOM_IN, ZOOM_OUT, ZOOM_RESET, ZOOM_TO_SELECTION,
    menu_layout,
};

/// Map the pure `MenuRow` layout to the chrome `MenuEntry` rows `ContextMenu`
/// renders (labels + optional keybinding chip; the menu bar shows no leading
/// icons, keeping the dropdowns text-first).
fn entries_for(id: MenuId) -> Vec<MenuEntry> {
    menu_layout(id)
        .into_iter()
        .map(|row| match row {
            MenuRow::Separator => MenuEntry::separator(),
            MenuRow::Item {
                command,
                label,
                shortcut,
            } => {
                let entry = MenuEntry::new(command, label);
                match shortcut {
                    Some(kb) => entry.shortcut(kb),
                    None => entry,
                }
            }
        })
        .collect()
}

/// A zoom-cluster button: a plain `.ambre-commandbar` icon button that emits
/// `command` through the shared path. `mousedown` `prevent_default` keeps DOM
/// focus on the canvas text pipe (the palette / FormatBar hazard).
fn zoom_button(
    on_command: Callback<CommandId>,
    command: CommandId,
    label: &'static str,
    icon: icondata::Icon,
) -> impl IntoView {
    view! {
        <button
            aria-label=label
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |_| on_command.run(command)
        >
            <Icon icon=icon />
        </button>
    }
}

/// The Toile top menu bar + zoom control. `zoom` is the shared live zoom factor
/// (`1.0` = 100%) the canvas view publishes; `on_command` is the shared command
/// sink (the same `Callback` the context menu uses).
#[component]
pub fn TopMenuBar(zoom: RwSignal<f64>, on_command: Callback<CommandId>) -> impl IntoView {
    // Which menu is open (drives the pressed title) and where its dropdown sits.
    // `ContextMenu` clears `pos` on select / Escape / click-outside; an effect
    // then clears `open_menu` so the title un-presses.
    let open_menu = RwSignal::new(None::<MenuId>);
    let pos = RwSignal::new(None::<(f64, f64)>);
    Effect::new(move |_| {
        if pos.get().is_none() {
            open_menu.set(None);
        }
    });

    // The rows of the currently open menu (empty when closed -> nothing renders).
    let entries = Memo::new(move |_| open_menu.get().map(entries_for).unwrap_or_default());

    // One title button per top-level menu. Clicking opens (or, when already open,
    // closes) its dropdown anchored at the title's bottom-left.
    let titles = TOP_MENUS
        .iter()
        .map(|(id, title)| {
            let id = *id;
            view! {
                <button
                    aria-label=*title
                    aria-pressed=move || (open_menu.get() == Some(id)).then_some("true")
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
                            x = r.left();
                            y = r.bottom();
                        }
                        open_menu.set(Some(id));
                        pos.set(Some((x, y)));
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
            {zoom_button(on_command, ZOOM_OUT, "Zoom out", icondata::MdiMinus)}
            <button
                class="ambre-zoom-readout"
                aria-label="Zoom to 100%"
                on:mousedown=move |ev| ev.prevent_default()
                on:click=move |_| on_command.run(ZOOM_RESET)
            >
                {move || format!("{:.0}%", (zoom.get() * 100.0).round())}
            </button>
            {zoom_button(on_command, ZOOM_IN, "Zoom in", icondata::MdiPlus)}
            <span class="sep"></span>
            {zoom_button(on_command, ZOOM_FIT, "Fit to content", icondata::MdiFitToScreenOutline)}
            {zoom_button(
                on_command,
                ZOOM_TO_SELECTION,
                "Zoom to selection",
                icondata::MdiSelectionSearch,
            )}
        </div>
        <ContextMenu pos=pos entries=entries on_command=on_command />
    }
}
