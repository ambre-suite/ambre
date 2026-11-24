// ambre-app/src/shell/header.rs
// ============================================================================
// The header every page shares (`ambre-spec-shell.md` section 2). It holds
// the Ambre menu behind the logo and the name of the open document. Then come
// the sync badge, the share, the about button and the avatar menu.
//
// The two menus are the only menus of the header, and each has a subject. The
// Ambre menu holds what acts on the document as a file, the way home first
// (`CHROME-154`, `CHROME-155`). The avatar menu holds what acts on the person
// (`CHROME-156`). Each reads its rows from the command tables, so no row
// carries a word of its own (`UI-THREAD-036`). The shell's own commands are
// `ambre_commands::shell`; the booted application's are its table.
// ============================================================================

use super::super::*;
use super::sync_badge::SyncBadge;
use ambre_chrome::commands::{Command, Home, Locale, shell};
use ambre_chrome::{ContextMenu, DockSide, MenuEntry};
use ambre_core::Flag as _;
use ambre_core::{Commands, StaticKey};

/// The rows of a header menu: each command of `table` that `region` homes,
/// in the order of the table, with its chord and its availability.
fn homed_entries(
    table: &[Command],
    region: Home,
    disabled: &[StaticKey<Commands>],
) -> Vec<MenuEntry> {
    table
        .iter()
        .filter(|c| c.home == region)
        .map(|c| {
            let entry = MenuEntry::command(c, &Locale::En);
            let entry = match c.chord() {
                Some(chord) => entry.shortcut(chord),
                None => entry,
            };
            if disabled.contains(&c.id) {
                entry.disabled()
            } else {
                entry
            }
        })
        .collect()
}

/// The rows of the Ambre menu (`CHROME-155`, `CHROME-169`). The way home comes
/// first. The share of the open document follows, then every command the page
/// homes in the Ambre menu, the command palette among them. `current` is the
/// open document: a page with none offers no share.
pub(crate) fn ambre_entries(
    table: &[Command],
    current: Option<Id<Document>>,
    disabled: &[StaticKey<Commands>],
) -> Vec<MenuEntry> {
    let shell_rows: Vec<MenuEntry> = shell::COMMANDS
        .iter()
        .filter(|c| c.id != shell::HOME && (c.id != shell::SHARE || current.is_some()))
        .filter(|c| c.home == Home::AmbreMenu)
        .map(|c| MenuEntry::command(c, &Locale::En))
        .collect();
    let app_rows = homed_entries(table, Home::AmbreMenu, disabled);
    let home = ambre_chrome::commands::find(shell::COMMANDS, shell::HOME)
        .map(|c| MenuEntry::command(c, &Locale::En));
    home.into_iter()
        .chain([MenuEntry::separator()])
        .chain(shell_rows)
        .chain(app_rows)
        .collect()
}

/// The rows of the avatar menu (`CHROME-156`). The theme comes first, then
/// the side of the panel on a page that lays one out. Every command the page
/// homes in the avatar menu follows.
pub(crate) fn avatar_entries(table: &[Command], panel: Option<DockSide>) -> Vec<MenuEntry> {
    shell::COMMANDS
        .iter()
        .filter(|c| c.home == Home::AvatarMenu)
        .filter(|c| c.id != shell::PANEL_SIDE || panel.is_some())
        .map(|c| MenuEntry::command(c, &Locale::En))
        .chain(homed_entries(table, Home::AvatarMenu, &[]))
        .collect()
}

/// The point under the bottom-left corner of the button `ev` fired on, where
/// its menu hangs.
fn under(ev: &web_sys::MouseEvent) -> ambre_core::Point<ambre_core::Px<ambre_core::Css>> {
    let rect = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .map(|el| el.get_bounding_client_rect());
    let (x, y) = rect.map_or((0.0, 0.0), |r| (r.left(), r.bottom()));
    ambre_core::Point::new(
        ambre_core::Px::<ambre_core::Css>::new(x as f32),
        ambre_core::Px::<ambre_core::Css>::new(y as f32),
    )
}

/// A button of the header that opens a menu under itself. A second press
/// closes it. The menu gives the focus back to the button when it closes
/// (`CHROME-027`).
struct MenuOpener {
    /// Where the menu hangs, `None` while it is closed.
    pos: RwSignal<Option<ambre_core::Point<ambre_core::Px<ambre_core::Css>>>>,
    /// Whether the menu is open, for `aria-expanded`. It trails `pos` by one
    /// effect, so the press that closes the menu still reads it open.
    open: RwSignal<ambre_chrome::Openness>,
}

impl MenuOpener {
    fn new() -> Self {
        let pos = RwSignal::new(None);
        let open = RwSignal::new(ambre_chrome::Openness::Closed);
        Effect::new(move |_| {
            open.set(ambre_chrome::Openness::from_bool(pos.get().is_some()));
        });
        Self { pos, open }
    }

    /// The press on the opener: it opens the menu, or closes an open one.
    fn press(&self, ev: &web_sys::MouseEvent) {
        if self.open.get_untracked() == ambre_chrome::Openness::Open {
            self.pos.set(None);
        } else {
            self.pos.set(Some(under(ev)));
        }
    }
}

/// The Ambre logo, the menu it opens, and the name of the open document, at
/// the start of the header (`CHROME-155`, `CHROME-203`). The way home is the
/// first row of the menu. It takes the navigation the launcher opens a
/// document with, so the address bar and the frame loop follow
/// (`CHROME-204`). Any other row runs through `on_command`. The name is the
/// one the document list holds, composed at the point of use (`CHROME-080`).
#[component]
pub(crate) fn AmbreMenu(
    docs: RwSignal<Vec<crate::landing::DocItem>>,
    current_doc: RwSignal<Option<Id<Document>>>,
    /// The table of the booted application.
    table: &'static [Command],
    /// Runs a row other than the way home.
    on_command: Callback<StaticKey<Commands>>,
    /// The rows the page holds unavailable, such as an undo with nothing to undo.
    #[prop(optional, into)]
    disabled: Option<Signal<Vec<StaticKey<Commands>>>>,
) -> impl IntoView {
    let navigate = leptos_router::hooks::use_navigate();
    let opener = MenuOpener::new();
    let (pos, open) = (opener.pos, opener.open);
    let entries = Signal::derive(move || {
        let disabled = disabled.map(|d| d.get()).unwrap_or_default();
        ambre_entries(table, current_doc.get(), &disabled)
    });
    let run = Callback::new(move |id: StaticKey<Commands>| {
        if id == shell::HOME {
            navigate("/", Default::default());
        } else {
            on_command.run(id);
        }
    });
    view! {
        <button
            class="title ambre-home"
            aria-label="Ambre menu"
            aria-haspopup="menu"
            aria-expanded=move || open.get().aria()
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |ev| opener.press(&ev)
        >
            "Ambre"
        </button>
        <ContextMenu pos=pos entries=entries on_command=run />
        {move || {
            current_doc.get().map(|document_id| {
                let name = docs.with(|list| {
                    list.iter()
                        .find(|d| d.document_id == document_id)
                        .and_then(|d| d.name.clone())
                });
                view! {
                    <span class="ambre-doc-name">
                        {ambre_chrome::ComposedName::or_placeholder(name.as_ref()).into_view()}
                    </span>
                }
            })
        }}
    }
}

/// The avatar of the person and the menu it opens, at the end of the header
/// (`CHROME-156`). The initials sit on the identity colour of the person, the
/// accent until the session names them (`PROFILE-002`). The button names the
/// menu and the person, so a screen reader hears whose account it opens.
#[component]
pub(crate) fn AvatarMenu(
    username: RwSignal<Option<ambre_core::Validated<ambre_core::DisplayName>>>,
    me: RwSignal<Option<Id<User>>>,
    /// The table of the booted application.
    table: &'static [Command],
    /// The side of the context panel, on a page that lays one out.
    dock_side: Option<RwSignal<DockSide>>,
    /// Runs a row.
    on_command: Callback<StaticKey<Commands>>,
) -> impl IntoView {
    let opener = MenuOpener::new();
    let (pos, open) = (opener.pos, opener.open);
    let entries = Signal::derive(move || avatar_entries(table, dock_side.map(|s| s.get())));
    let name = move || {
        username.get().map_or_else(
            || "Signed out".to_owned(),
            ambre_core::Validated::into_string,
        )
    };
    view! {
        <button
            class="ambre-avatar"
            aria-label=move || format!("Account menu, {}", name())
            aria-haspopup="menu"
            aria-expanded=move || open.get().aria()
            title=name
            style:background=move || me.get().map(|user| ambre_draw::peer_rgb(&user).to_string())
            on:mousedown=move |ev| ev.prevent_default()
            on:click=move |ev| opener.press(&ev)
        >
            {move || ambre_chrome::avatar_initials(username.get().as_ref().map_or("", ambre_core::Validated::as_str))}
        </button>
        <ContextMenu pos=pos entries=entries on_command=on_command />
    }
}

/// The header every application shares. It holds the Ambre menu, the name
/// of the document, the sync badge, the share, the about button and the
/// avatar menu.
/// The theme and the side of the panel are settings of the person, so they
/// live in the avatar menu (`CHROME-156`). The share is an icon that names
/// itself in a tooltip read from its command (`CHROME-185`).
#[component]
pub(crate) fn ShellHeader(
    theme_ctl: ambre_canvas::ThemeControl,
    theme_sig: RwSignal<ambre_canvas::Theme>,
    toasts: Toasts,
    chrome_status: RwSignal<ChromeStatus>,
    current_doc: RwSignal<Option<Id<Document>>>,
    share_open: RwSignal<ambre_chrome::Openness>,
    about_open: RwSignal<ambre_chrome::Openness>,
    username: RwSignal<Option<ambre_core::Validated<ambre_core::DisplayName>>>,
    me: RwSignal<Option<Id<User>>>,
    docs: RwSignal<Vec<crate::landing::DocItem>>,
    /// The side of the context panel, on a page that lays one out.
    dock_side: Option<RwSignal<DockSide>>,
    /// The table of the booted application.
    table: &'static [Command],
    /// Runs a command of the booted application.
    on_command: Callback<StaticKey<Commands>>,
    /// The rows of the Ambre menu the page holds unavailable.
    #[prop(optional, into)]
    disabled: Option<Signal<Vec<StaticKey<Commands>>>>,
) -> impl IntoView {
    // A row of either menu is a command of the shell, which the header
    // performs, or a command of the application, which its handler answers.
    let run = Callback::new(move |id: StaticKey<Commands>| {
        if id == shell::SHARE {
            share_open.set(ambre_chrome::Openness::Open);
        } else if id == shell::THEME {
            let next = switch_theme(&theme_ctl);
            theme_sig.set(next);
            toasts.info(if next == ambre_canvas::Theme::Dark {
                "Dark theme"
            } else {
                "Light theme"
            });
        } else if id == shell::PANEL_SIDE {
            if let Some(side) = dock_side {
                side.update(|s| *s = s.flipped());
            }
        } else {
            on_command.run(id);
        }
    });
    let share = ambre_chrome::commands::find(shell::COMMANDS, shell::SHARE);
    let disabled = disabled.unwrap_or_else(|| Signal::stored(Vec::new()));
    view! {
        <AmbreMenu docs=docs current_doc=current_doc table=table on_command=run disabled=disabled />
        <span class="spacer"></span>
        <SyncBadge status=chrome_status.into() />
        {share.map(|command| view! {
            <Show when=move || current_doc.get().is_some()>
                <ambre_chrome::CommandButton
                    command=command
                    icon=Some(icondata::AiShareAltOutlined)
                    on_command=run
                    look=ambre_chrome::IconLook::Ghost
                />
            </Show>
        })}
        <button
            class="icon"
            title="About"
            aria-label="About Ambre"
            aria-haspopup="dialog"
            aria-expanded=move || about_open.get().aria()
            on:click=move |_| about_open.set(ambre_chrome::Openness::Open)
        >
            "?"
        </button>
        <AvatarMenu username=username me=me table=table dock_side=dock_side on_command=run />
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::DocType;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// The table of each application whose page lays out the shared header.
    fn tables() -> [&'static [Command]; 4] {
        [
            ambre_prose::commands::COMMANDS,
            trame_chrome::COMMANDS,
            ambre_toile::ui::commands::COMMANDS,
            ambre_murmure::ui::commands::COMMANDS,
        ]
    }

    /// The commands the rows of `entries` run, separators left out.
    fn commands_of(entries: &[MenuEntry]) -> Vec<StaticKey<Commands>> {
        entries
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item(item) => Some(item.command()),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    /// A fresh container in the page, which the test removes at its end.
    fn container() -> web_sys::Element {
        let page = web_sys::window()
            .and_then(|w| w.document())
            .expect("a document");
        let container = page.create_element("div").expect("a container");
        page.body()
            .expect("a body")
            .append_child(&container)
            .expect("appended");
        container
    }

    /// The labels of the rows of the open menu under `root`.
    fn open_rows(root: &web_sys::Element) -> Vec<web_sys::HtmlElement> {
        let rows = root
            .query_selector_all(".ambre-ctx-item")
            .expect("a selector");
        (0..rows.length())
            .filter_map(|i| rows.item(i)?.dyn_into::<web_sys::HtmlElement>().ok())
            .collect()
    }

    /// In every application, the first row of the Ambre menu is the way home,
    /// and the menu offers the command palette. The rows read no width, so a
    /// phone shows the same menu (`CHROME-155`, `CHROME-169`). Each other row
    /// is a command its page homes in the Ambre menu, the share first while a
    /// document is open (`CHROME-154`).
    #[wasm_bindgen_test]
    fn the_ambre_menu_starts_with_the_way_home_and_offers_the_palette() {
        let open = Some(Id::<Document>::mint());
        for table in tables() {
            let rows = commands_of(&ambre_entries(table, open, &[]));
            assert_eq!(rows.first(), Some(&shell::HOME), "the way home comes first");
            assert_eq!(rows.get(1), Some(&shell::SHARE), "then the share");
            assert!(
                rows.iter().any(
                    |id| ambre_chrome::commands::find(table, *id).is_some_and(|c| (c.title)(
                        &Locale::En
                    )
                    .as_str()
                        == "Command palette")
                ),
                "the menu offers the palette: {rows:?}"
            );
            for id in rows.iter().skip(2) {
                let home = ambre_chrome::commands::find(table, *id).map(|c| c.home);
                assert_eq!(home, Some(Home::AmbreMenu), "{id:?}");
            }
            let closed = commands_of(&ambre_entries(table, None, &[]));
            assert!(!closed.contains(&shell::SHARE), "no document, no share");
        }
    }

    /// The avatar menu holds the theme, and the side of the panel on a page
    /// that lays one out. It holds the commands each page homes there too,
    /// such as the pointer of the board (`CHROME-156`).
    #[wasm_bindgen_test]
    fn the_avatar_menu_holds_the_settings_of_the_person() {
        let rows = commands_of(&avatar_entries(
            ambre_toile::ui::commands::COMMANDS,
            Some(DockSide::Right),
        ));
        assert_eq!(rows.first(), Some(&shell::THEME));
        assert_eq!(rows.get(1), Some(&shell::PANEL_SIDE));
        assert!(rows.contains(&ambre_toile::ui::menu_bar::POINTER_MOUSE));
        let murmure = commands_of(&avatar_entries(ambre_murmure::ui::commands::COMMANDS, None));
        assert!(!murmure.contains(&shell::PANEL_SIDE), "no panel, no side");
        for table in tables() {
            for id in commands_of(&avatar_entries(table, Some(DockSide::Left))) {
                let home = ambre_chrome::commands::find(table, id)
                    .or_else(|| ambre_chrome::commands::find(shell::COMMANDS, id))
                    .map(|c| c.home);
                assert_eq!(home, Some(Home::AvatarMenu), "{id:?}");
            }
        }
    }

    /// The header names the open document (`CHROME-203`). A click on the
    /// logo opens the Ambre menu on the way home (`CHROME-155`). A click on
    /// that row moves the address bar home (`CHROME-204`) with no reload
    /// (`CHROME-205`). A page with no document shows no name.
    #[wasm_bindgen_test]
    async fn the_header_links_home_and_names_the_open_document() {
        let document_id = Id::<Document>::mint();
        let docs = RwSignal::new(vec![crate::landing::DocItem {
            document_id,
            doc_type: DocType::Prose,
            name: Some(ambre_core::Validated::<ambre_core::Name>::parse("Plan").expect("a name")),
            updated_at: None,
            member_count: ambre_core::Count::<ambre_core::Members>::ONE,
        }]);
        let current_doc = RwSignal::new(None);
        let ran = RwSignal::new(None::<StaticKey<Commands>>);
        let root = container();
        let target: web_sys::HtmlElement = root.clone().dyn_into().expect("an element");
        leptos::mount::mount_to(target, move || {
            view! {
                <leptos_router::components::Router>
                    <AmbreMenu
                        docs=docs
                        current_doc=current_doc
                        table=ambre_prose::commands::COMMANDS
                        on_command=Callback::new(move |id| ran.set(Some(id)))
                    />
                </leptos_router::components::Router>
            }
        })
        .forget();
        let query = |selector: &str| root.query_selector(selector).expect("a selector");
        assert!(query(".ambre-doc-name").is_none(), "no document, no name");

        current_doc.set(Some(document_id));
        leptos::task::tick().await;
        let name = query(".ambre-doc-name").expect("the name of the open document");
        assert_eq!(name.text_content().as_deref(), Some("Plan"));

        let logo: web_sys::HtmlElement = query("button.ambre-home")
            .expect("the logo")
            .dyn_into()
            .expect("an element");
        assert_eq!(logo.get_attribute("aria-haspopup").as_deref(), Some("menu"));
        logo.click();
        leptos::task::tick().await;
        assert_eq!(logo.get_attribute("aria-expanded").as_deref(), Some("true"));
        let rows = open_rows(&root);
        let first = rows.first().expect("a row");
        assert!(
            first
                .text_content()
                .is_some_and(|t| t.contains("All documents")),
            "the way home comes first"
        );

        // The share row of the open document runs through the sink.
        let share = rows
            .iter()
            .find(|r| {
                r.text_content()
                    .is_some_and(|t| t.contains("Share document"))
            })
            .expect("the share row");
        share.click();
        leptos::task::tick().await;
        assert_eq!(ran.get_untracked(), Some(shell::SHARE));

        // The way home moves the address bar home, which the frame loop reads.
        logo.click();
        leptos::task::tick().await;
        let window = web_sys::window().expect("a window");
        let history = window.history().expect("a history");
        let runner = window.location().href().expect("the runner address");
        let doc = crate::route::doc_path(DocType::Prose, document_id);
        history
            .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&doc))
            .expect("a document address");
        open_rows(&root).first().expect("the way home").click();
        leptos::task::tick().await;
        let path = window.location().pathname().expect("a path");
        history
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&runner))
            .expect("the runner address back");
        assert_eq!(path, "/", "the address bar goes home");
        assert_eq!(
            ran.get_untracked(),
            Some(shell::SHARE),
            "home runs no command"
        );
        assert!(
            logo.is_connected(),
            "the page did not reload (`CHROME-205`)"
        );
        root.remove();
    }

    /// The whole header of a document page. Only the logo and the avatar open
    /// a menu, and each has a subject: no other button opens a list of
    /// commands (`CHROME-161`). The theme and the side of the panel left the
    /// header for the avatar menu (`CHROME-156`). Each icon alone names
    /// itself in a tooltip its command describes, the share included
    /// (`CHROME-185`).
    #[wasm_bindgen_test]
    async fn only_the_logo_and_the_avatar_open_a_menu_and_every_icon_names_itself() {
        let root = container();
        let target: web_sys::HtmlElement = root.clone().dyn_into().expect("an element");
        let current_doc = RwSignal::new(Some(Id::<Document>::mint()));
        let side = RwSignal::new(DockSide::Right);
        let ran = RwSignal::new(None::<StaticKey<Commands>>);
        leptos::mount::mount_to(target, move || {
            view! {
                <leptos_router::components::Router>
                    <ShellHeader
                        theme_ctl=ambre_canvas::ThemeControl::new(ambre_canvas::Theme::Light)
                        theme_sig=RwSignal::new(ambre_canvas::Theme::Light)
                        toasts=Toasts::new()
                        chrome_status=RwSignal::new(ChromeStatus::default())
                        current_doc=current_doc
                        share_open=RwSignal::new(ambre_chrome::Openness::Closed)
                        about_open=RwSignal::new(ambre_chrome::Openness::Closed)
                        username=RwSignal::new(None)
                        me=RwSignal::new(None)
                        docs=RwSignal::new(Vec::new())
                        dock_side=Some(side)
                        table=ambre_toile::ui::commands::COMMANDS
                        on_command=Callback::new(move |id| ran.set(Some(id)))
                    />
                </leptos_router::components::Router>
            }
        })
        .forget();
        leptos::task::tick().await;
        let openers = root
            .query_selector_all("[aria-haspopup=\"menu\"]")
            .expect("a selector");
        let classes: Vec<String> = (0..openers.length())
            .filter_map(|i| openers.item(i)?.dyn_into::<web_sys::Element>().ok())
            .map(|e| e.class_name())
            .collect();
        assert_eq!(classes, ["title ambre-home", "ambre-avatar"]);
        assert!(
            root.query_selector(".ambre-dock-side")
                .expect("a selector")
                .is_none()
        );
        assert!(
            root.query_selector("[aria-label=\"Toggle colour theme\"]")
                .expect("a selector")
                .is_none(),
            "the theme left the header"
        );
        assert_eq!(
            ambre_chrome::untipped_icons(&root, shell::COMMANDS),
            Vec::<String>::new()
        );
        let share = root
            .query_selector(&format!(
                "[data-reach=\"{}\"]",
                ambre_chrome::commands::Reach::Command(shell::SHARE).marker()
            ))
            .expect("a selector");
        assert!(share.is_some(), "the share reaches its command");

        // The avatar menu flips the side of the panel, and runs the pointer of
        // the board through the sink of the application.
        let avatar: web_sys::HtmlElement = root
            .query_selector("button.ambre-avatar")
            .expect("a selector")
            .expect("the avatar")
            .dyn_into()
            .expect("an element");
        avatar.click();
        leptos::task::tick().await;
        open_rows(&root)
            .iter()
            .find(|r| {
                r.text_content()
                    .is_some_and(|t| t.contains("Dock the panel"))
            })
            .expect("the side row")
            .click();
        leptos::task::tick().await;
        assert_eq!(side.get_untracked(), DockSide::Left);
        avatar.click();
        leptos::task::tick().await;
        open_rows(&root)
            .iter()
            .find(|r| {
                r.text_content()
                    .is_some_and(|t| t.contains("Pointer: mouse"))
            })
            .expect("the pointer row")
            .click();
        leptos::task::tick().await;
        assert_eq!(
            ran.get_untracked(),
            Some(ambre_toile::ui::menu_bar::POINTER_MOUSE)
        );
        root.remove();
    }
}
