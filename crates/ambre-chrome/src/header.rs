// ambre-chrome/src/header.rs
// ============================================================================
// The pieces of the shared header (`ambre-spec-shell.md` section 2). They
// are the field that opens the command palette, the button of a menu, and
// the primary button of the header.
//
// The header holds no command of its own. The search field runs the palette
// command of the booted application, so its chord agrees with the palette.
// A menu lists what the shell hands it.
// ============================================================================

use ambre_commands::{Command, Reach};
use ambre_core::{Commands, StaticKey};
use leptos::prelude::*;

/// The words the search field shows, and its accessible name.
pub const SEARCH_LABEL: &str = "Search for a command";

/// The field in the centre of the header that opens the command palette
/// (`CHROME-237`). It is no search engine of its own. A press runs
/// `command`, the palette command of the booted application. It runs it
/// through `on_command`, the path every trigger shares. Below the tablet
/// width the stylesheet keeps its icon alone.
#[component]
pub fn CommandSearch(
    /// The palette command of the booted application.
    command: &'static Command,
    /// The one command path of the shell.
    on_command: Callback<StaticKey<Commands>>,
) -> impl IntoView {
    let id = command.id;
    view! {
        <button
            class="ambre-command-search"
            aria-label=SEARCH_LABEL
            aria-haspopup="dialog"
            aria-keyshortcuts=command.chord().map(|c| c.to_string())
            data-reach=Reach::Command(id).marker()
            on:click=move |_| on_command.run(id)
        >
            <leptos_icons::Icon icon=icondata::LuSearch />
            <span class="ambre-command-search-label">{SEARCH_LABEL}</span>
            {command.chord().map(|binding| view! { <crate::Kbd binding=binding /> })}
        </button>
    }
}

/// The button of a header menu, and the menu it opens under it
/// (`ambre-spec-shell.md` section 2). The Ambre menu opens on the mark, the
/// avatar menu on the avatar. `trigger` draws the button, and `children` the
/// items, each a `role="menuitem"` element. A press on an item closes the menu, and so do
/// Escape and a press outside it. The keys of every menu drive it
/// (`CHROME-026`).
#[component]
pub fn HeaderMenu(
    /// The accessible name of the button.
    #[prop(into)]
    label: String,
    /// The class the button and the menu are styled by.
    class: StaticKey<ambre_core::CssClasses>,
    /// The content of the button.
    #[prop(into)]
    trigger: ViewFn,
    /// The items of the menu.
    children: ChildrenFn,
) -> impl IntoView {
    let open = RwSignal::new(crate::Openness::Closed);
    let menu_ref = NodeRef::<leptos::html::Div>::new();
    // An opened menu takes the focus, so the keys reach it (`CHROME-026`).
    Effect::new(move |_| {
        if let Some(menu) = menu_ref.get() {
            crate::menu::focus_first_item(&menu);
        }
    });
    let close = move || open.set(crate::Openness::Closed);
    view! {
        <div class=format!("ambre-header-menu {}", class.as_str())>
            <button
                class="ambre-header-menu-button"
                aria-label=label
                aria-haspopup="menu"
                aria-expanded=move || open.get().aria()
                on:click=move |_| open.update(|o| *o = o.toggled())
            >
                {trigger.run()}
            </button>
            <Show when=move || open.get() == crate::Openness::Open>
                <div class="ambre-header-menu-backdrop" on:pointerdown=move |_| close()></div>
                <div
                    class="ambre-header-menu-list"
                    role="menu"
                    node_ref=menu_ref
                    on:click=move |_| close()
                    on:keydown=move |ev| {
                        if let Some(menu) = menu_ref.get_untracked() {
                            crate::menu::on_menu_keydown(&ev, &menu, close);
                        }
                    }
                >
                    {children()}
                </div>
            </Show>
        </div>
    }
}

/// The primary button of the header: an accent fill, an icon and a label.
/// Share is the one (`CHROME-238`).
#[component]
pub fn PrimaryButton(
    /// The icon before the label.
    icon: icondata::Icon,
    /// The words of the button.
    label: &'static str,
    /// Whether the dialog the button opens is open.
    #[prop(into)]
    expanded: Signal<crate::Openness>,
    /// Runs on a press.
    on_press: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            class="ambre-primary-button"
            aria-haspopup="dialog"
            aria-expanded=move || expanded.get().aria()
            on:click=move |_| on_press.run(())
        >
            <leptos_icons::Icon icon=icon />
            <span>{label}</span>
        </button>
    }
}
