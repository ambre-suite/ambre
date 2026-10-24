// ambre-trame/src/chrome/sheet_tabs.rs
// ============================================================================
// The sheet-tab strip (`trame-sheets`), as DOM Leptos chrome.
//
// Migrated off the canvas. Tabs, the trailing "+" add button, in-place rename,
// the per-tab `{ Rename, Delete }` context menu and drag-reorder are now plain
// DOM. They therefore share the chrome's hover tokens and accessibility for free.
// The canvas era hand-painted all of this and hand-routed every pointer event.
//
// The component is purely presentational. It emits intents through callbacks, and
// the launcher maps them onto `TrameCommand`s (ActivateSheet / CreateSheet /
// RenameSheet / DeleteSheet / MoveSheet). The sheet list + active id are fed as
// signals, mirrored from `TrameEvent::SheetList`.
// ============================================================================

use ambre_bus::{SheetId, SheetName, SheetTab, SheetTabPos};
use leptos::prelude::*;
use leptos::web_sys;

/// Reorder intent emitted on drop: move sheet `id` so it lands at `to_index`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReorderSheet {
    pub id: SheetId,
    pub to_index: SheetTabPos,
}

/// The sheet-tab strip. `sheets` is in display order, `active` the active
/// sheet id; both mirror `TrameEvent::SheetList`. Each user action is
/// surfaced through its callback; the launcher owns the `TrameCommand` mapping.
#[component]
pub fn SheetTabs(
    sheets: RwSignal<Vec<SheetTab>>,
    active: RwSignal<SheetId>,
    on_activate: Callback<SheetId>,
    on_create: Callback<()>,
    on_rename: Callback<(SheetId, SheetName)>,
    on_delete: Callback<SheetId>,
    on_reorder: Callback<ReorderSheet>,
) -> impl IntoView {
    // Id of the tab being renamed in place, or `None`.
    let renaming = RwSignal::new(None::<SheetId>);
    // Open per-tab context menu: `(sheet_id, client_x, client_y)`, or `None`.
    let menu = RwSignal::new(None::<(SheetId, f64, f64)>);
    // Id of the tab currently being dragged, or `None`.
    let dragging = RwSignal::new(None::<SheetId>);
    // Index a drop would land on (the tab currently hovered during a drag).
    let drop_idx = RwSignal::new(None::<SheetTabPos>);

    // Dismiss the context menu on any outside pointer-down.
    let menu_handle = window_event_listener(leptos::ev::pointerdown, move |_| {
        if menu.get_untracked().is_some() {
            menu.set(None);
        }
    });
    on_cleanup(move || menu_handle.remove());

    // Begin an in-place rename of `id`, closing any open menu first.
    let start_rename = move |id: SheetId| {
        menu.set(None);
        renaming.set(Some(id));
    };

    // The tab list, rebuilt when `sheets` changes. Per-tab `active`/`dragging`/
    // `drop` reactivity is handled by inner closures, so an active-sheet switch
    // or a drag does not rebuild the list.
    let tabs = move || {
        sheets
            .get()
            .into_iter()
            .enumerate()
            .map(|(index, SheetTab { id, name })| {
                let index = SheetTabPos::from_usize(index);
                let is_active = {
                    let id = id.clone();
                    move || active.with(|a| *a == id)
                };
                let is_renaming = {
                    let id = id.clone();
                    move || renaming.with(|r| r.as_ref() == Some(&id))
                };
                let is_dragging = {
                    let id = id.clone();
                    move || dragging.with(|d| d.as_ref() == Some(&id))
                };
                let is_drop = move || drop_idx.get() == Some(index);

                let click_id = id.clone();
                let dbl_id = id.clone();
                let ctx_id = id.clone();
                let drag_id = id.clone();
                let rename_id = id.clone();
                let blur_id = id.clone();
                let init_name = name.clone();

                view! {
                    <Show
                        when=is_renaming.clone()
                        fallback={
                            let name = name.clone();
                            let is_active = is_active.clone();
                            let is_dragging = is_dragging.clone();
                            let click_id = click_id.clone();
                            let dbl_id = dbl_id.clone();
                            let ctx_id = ctx_id.clone();
                            let drag_id = drag_id.clone();
                            move || {
                                let click_id = click_id.clone();
                                let dbl_id = dbl_id.clone();
                                let ctx_id = ctx_id.clone();
                                let drag_id = drag_id.clone();
                                view! {
                                    <button
                                        class="tab"
                                        class:active=is_active.clone()
                                        class:dragging=is_dragging.clone()
                                        class:drop-before=is_drop
                                        draggable="true"
                                        on:click=move |_| on_activate.run(click_id.clone())
                                        on:dblclick=move |_| start_rename(dbl_id.clone())
                                        on:contextmenu=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            menu.set(Some((
                                                ctx_id.clone(),
                                                ev.client_x() as f64,
                                                ev.client_y() as f64,
                                            )));
                                        }
                                        on:dragstart=move |_| dragging.set(Some(drag_id.clone()))
                                        on:dragover=move |ev: web_sys::DragEvent| {
                                            if dragging.get_untracked().is_some() {
                                                ev.prevent_default();
                                                drop_idx.set(Some(index));
                                            }
                                        }
                                        on:drop=move |ev: web_sys::DragEvent| {
                                            ev.prevent_default();
                                            if let Some(src) = dragging.get_untracked() {
                                                on_reorder.run(ReorderSheet {
                                                    id: src,
                                                    to_index: index,
                                                });
                                            }
                                            dragging.set(None);
                                            drop_idx.set(None);
                                        }
                                        on:dragend=move |_| {
                                            dragging.set(None);
                                            drop_idx.set(None);
                                        }
                                    >
                                        {name.to_string()}
                                    </button>
                                }
                            }
                        }
                    >
                        {
                            let rename_id = rename_id.clone();
                            let blur_id = blur_id.clone();
                            let init_name = init_name.to_string();
                            view! {
                                <input
                                    class="tab-rename"
                                    type="text"
                                    prop:value=init_name.clone()
                                    node_ref={
                                        let r = NodeRef::<leptos::html::Input>::new();
                                        r.on_load(|el: web_sys::HtmlInputElement| {
                                            let _ = el.focus();
                                            el.select();
                                        });
                                        r
                                    }
                                    on:keydown={
                                        let rename_id = rename_id.clone();
                                        move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
                                            // A name the rule refuses keeps the editor open,
                                            // its reason on the field (`TRAME-234`).
                                            "Enter" => match SheetName::new(&event_target_value(&ev)) {
                                                Ok(name) => {
                                                    on_rename.run((rename_id.clone(), name));
                                                    renaming.set(None);
                                                }
                                                Err(refusal) => {
                                                    let input = event_target::<web_sys::HtmlInputElement>(&ev);
                                                    input.set_title(&refusal.to_string());
                                                    let _ = input.set_attribute("aria-invalid", "true");
                                                }
                                            },
                                            "Escape" => renaming.set(None),
                                            _ => {}
                                        }
                                    }
                                    // Leaving the field with a refused name drops the edit.
                                    on:blur=move |ev| {
                                        if let Ok(name) = SheetName::new(&event_target_value(&ev)) {
                                            on_rename.run((blur_id.clone(), name));
                                        }
                                        renaming.set(None);
                                    }
                                />
                            }
                        }
                    </Show>
                }
            })
            .collect_view()
    };

    // The per-tab context menu, anchored at the click point. It takes the focus
    // when it opens and answers the shared menu keys (`CHROME-026`).
    let menu_ref = NodeRef::<leptos::html::Ul>::new();
    Effect::new(move |_| {
        if let Some(list) = menu_ref.get() {
            ambre_chrome::focus_first_item(&list);
        }
    });
    let context_menu = move || {
        menu.get().map(|(id, x, y)| {
            let style = format!("position: fixed; left: {x}px; top: {y}px; z-index: 60;");
            let id_rename = id.clone();
            let id_delete = id.clone();
            view! {
                <ul
                    class="ambre-tab-menu"
                    role="menu"
                    style=style
                    node_ref=menu_ref
                    on:keydown=move |ev| {
                        if let Some(list) = menu_ref.get_untracked() {
                            ambre_chrome::on_menu_keydown(&ev, &list, || menu.set(None));
                        }
                    }
                >
                    <li role="none">
                        <button
                            role="menuitem"
                            tabindex="-1"
                            on:click=move |_| start_rename(id_rename.clone())
                        >
                            "Rename"
                        </button>
                    </li>
                    <li role="none">
                        <button
                            role="menuitem"
                            tabindex="-1"
                            on:click=move |_| {
                                menu.set(None);
                                on_delete.run(id_delete.clone());
                            }
                        >
                            "Delete"
                        </button>
                    </li>
                </ul>
            }
        })
    };

    view! {
        {tabs}
        <button
            class="tab-add"
            title="Add sheet"
            aria-label="Add sheet"
            on:click=move |_| on_create.run(())
        >
            "+"
        </button>
        {context_menu}
    }
}
