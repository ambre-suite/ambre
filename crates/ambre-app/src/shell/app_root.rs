// ambre-app/src/shell/app_root.rs
// The single root view (AppRoot): persistent chrome + canvas, landing overlay
// on `/`. Reads its wiring from context; the document binding is driven outside
// this tree by the per-frame route poll in `main`.

use super::super::*;
use super::call_bar::{CallBar, RingAlert};
use super::prose_status::ProseFigures;
use super::share::{InviteLink, MurmureInviteLink, ShareInviteLink, ShareMembers};
use super::sync_badge::SyncBadge;
use ambre_bus::DocType;
use ambre_bus::{Authored, Instant};
use ambre_chrome::commands::{ClipboardWrite, PasteForm};
use ambre_core::{Commands, StaticKey};
use ambre_murmure::MurmurePane;
use ambre_murmure::chrome::{
    ChannelHeader, ChannelList, ChannelMentionConfirm, DeleteConfirm, EmojiPicker, InviteControl,
    KeyList, MarkBarPanel, MemberList, MentionPanel, MessageSearch, MurmureRelay, SpaceList,
};

/// The table the command palette searches: the booted application's whole
/// declaration, never a menu's subset (`CHROME-008`). No application
/// borrows another's table.
pub(crate) fn palette_table(kind: DocType) -> &'static [ambre_chrome::commands::Command] {
    match kind {
        DocType::Toile => ambre_toile::ui::commands::COMMANDS,
        DocType::Prose => ambre_prose::commands::COMMANDS,
        DocType::Trame => trame_chrome::COMMANDS,
        DocType::Murmure => ambre_murmure::ui::commands::COMMANDS,
    }
}

/// The shell of a Murmure page: its channel list, its channel and its thread
/// (the Murmure layout spec). The pane names the region in focus, and the
/// command bar holds the channel header. No formula bar, tabs or status bar:
/// each row is absent.
#[component]
pub(crate) fn MurmureShell(
    /// The pane, the sidebar, and the channels this page opened.
    relay: MurmureRelay,
    /// Carries what the channel list asks of the Logic thread.
    send: Callback<ambre_murmure::message::LogicCommand>,
    /// The last invite into a space, whose link the toolbar shows
    /// (`SYNC-245`).
    invite_link: RwSignal<Option<InviteLink>>,
    /// The shared header.
    children: ChildrenFn,
) -> impl IntoView {
    let pane = relay.pane;
    // The page holds the call, and the header reads its phase (the Murmure calls spec).
    let call_phase = Signal::from(crate::call::phase());
    view! {
        <AppShell
            focus=Signal::derive(move || ambre_murmure::chrome::focus_of(pane.get()))
            panel_open=Signal::derive(move || ambre_murmure::chrome::panel_open(pane.get()))
        >
            <Header slot>{children()}</Header>
            <Toolbar slot>
                <ChannelHeader pane=pane name=relay.channel_name() has_members=relay.has_members()>
                    <CallBar pane=pane phase=call_phase relay=relay />
                </ChannelHeader>
                <RingAlert />
                <InviteControl relay=relay send=send />
                <MurmureInviteLink link=invite_link />
                <MessageSearch relay=relay send=send />
            </Toolbar>
            <Sidebar slot label="Channels">
                // The view stamps a creation: the Logic thread reads no clock.
                <SpaceList
                    relay=relay
                    send=send
                    now=Callback::new(|()| Instant::<Authored>::new(js_sys::Date::now() as u64))
                />
                <ChannelList relay=relay send=send />
            </Sidebar>
            <Panel slot label=Signal::derive(move || ambre_murmure::chrome::panel_label(pane.get()).to_owned())>
                <Show
                    when=move || matches!(pane.get(), MurmurePane::Members { .. })
                    // The thread is a canvas of its own, in the panel's cell
                    // (the Murmure message list spec). The host binds it by id.
                    fallback=|| view! { <CanvasCell id=crate::host::THREAD_CANVAS label="Thread" /> }
                >
                    <MemberList relay=relay send=send />
                </Show>
            </Panel>
        </AppShell>
        // The view stamps the deletion: the Logic thread reads no clock.
        <DeleteConfirm
            relay=relay
            send=send
            now=Callback::new(|()| Instant::<Authored>::new(js_sys::Date::now() as u64))
        />
        <ChannelMentionConfirm
            relay=relay
            send=send
            now=Callback::new(|()| Instant::<Authored>::new(js_sys::Date::now() as u64))
        />
        // The view stamps a reaction, as it stamps a deletion.
        <EmojiPicker
            relay=relay
            send=send
            now=Callback::new(|()| Instant::<Authored>::new(js_sys::Date::now() as u64))
        />
        <KeyList relay=relay />
        <MentionPanel relay=relay />
        <MarkBarPanel relay=relay />
    }
}

/// The header every application shares: the title, the sync badge, the theme
/// switch, the share and about buttons and the avatar.
#[component]
fn ShellHeader(
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
    dock_side: Option<RwSignal<ambre_chrome::DockSide>>,
) -> impl IntoView {
    view! {
        <HeaderTitle docs=docs current_doc=current_doc />
        <span class="spacer"></span>
        // The side of the context panel is a setting of the person. It sits
        // beside the theme switch, the one setting the header holds: no
        // avatar menu exists yet (`ambre-spec-shell.md` section 2).
        {dock_side.map(|side| view! { <super::panel::DockSideSwitch side=side /> })}
        <SyncBadge status=chrome_status.into() />
        <button
            class="icon"
            title="Toggle theme"
            aria-label="Toggle colour theme"
            on:click=move |_| {
                let next = switch_theme(&theme_ctl);
                theme_sig.set(next);
                let label = if next == ambre_canvas::Theme::Dark {
                    "Dark theme"
                } else {
                    "Light theme"
                };
                toasts.info(label);
            }
        >
            {move || {
                let icon = if theme_sig.get() == ambre_canvas::Theme::Dark {
                    icondata::AiMoonFilled
                } else {
                    icondata::AiSunFilled
                };
                view! { <Icon icon=icon /> }
            }}
        </button>
        <Show when=move || current_doc.get().is_some()>
            <button
                class="icon"
                title="Share"
                aria-label="Share document"
                aria-haspopup="dialog"
                aria-expanded=move || share_open.get().aria()
                on:click=move |_| share_open.set(ambre_chrome::Openness::Open)
            >
                <Icon icon=icondata::AiShareAltOutlined />
            </button>
        </Show>
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
        // The user's initials on their identity colour, the accent until the
        // session names them (`PROFILE-002`).
        <span
            class="ambre-avatar"
            title=move || username.get().map(ambre_core::Validated::into_string).unwrap_or_else(|| "Signed out".to_owned())
            style:background=move || me.get().map(|user| ambre_draw::peer_rgb(&user).to_string())
        >
            {move || ambre_chrome::avatar_initials(username.get().as_ref().map_or("", ambre_core::Validated::as_str))}
        </span>
    }
}

/// The optional rows of the shell each application lays out (`CHROME-140`).
/// The formula bar is Trame's alone. No application lays out the tab strip:
/// Trame's sheet tabs sit in the status bar (`CHROME-162`). A Toile board docks
/// its menus in the command bar. A Prose page formats from the bar over its
/// selection, so it lays out no command bar. Murmure builds its own shell.
pub(crate) fn shell_rows(kind: DocType) -> ambre_chrome::ShellRows {
    use ambre_chrome::RowShown::{Absent, Shown};
    match kind {
        DocType::Trame => ambre_chrome::ShellRows {
            toolbar: Shown,
            formula_bar: Shown,
            tabs: Absent,
        },
        DocType::Toile => ambre_chrome::ShellRows {
            toolbar: Shown,
            formula_bar: Absent,
            tabs: Absent,
        },
        DocType::Prose => ambre_chrome::ShellRows {
            toolbar: Absent,
            formula_bar: Absent,
            tabs: Absent,
        },
        DocType::Murmure => ambre_chrome::ShellRows {
            toolbar: Shown,
            formula_bar: Absent,
            tabs: Absent,
        },
    }
}

/// The way home and the name of the open document, at the start of the
/// header (`CHROME-144`, `CHROME-145`). A plain click goes home in place. It
/// takes the navigation the launcher opens a document with, so the address bar
/// and the frame loop follow. The router's own anchor handling moves its state and
/// leaves the address unchanged, as no `Routes` completes the navigation. A
/// modified click keeps the browser's own behaviour, a new tab for example.
/// The name is the one the document list holds, composed at the point of use
/// (`CHROME-080`).
#[component]
pub(crate) fn HeaderTitle(
    docs: RwSignal<Vec<crate::landing::DocItem>>,
    current_doc: RwSignal<Option<Id<Document>>>,
) -> impl IntoView {
    let navigate = leptos_router::hooks::use_navigate();
    let go_home = move |ev: web_sys::MouseEvent| {
        let modified =
            ev.button() != 0 || ev.ctrl_key() || ev.meta_key() || ev.shift_key() || ev.alt_key();
        if !modified {
            ev.prevent_default();
            navigate("/", Default::default());
        }
    };
    view! {
        <a
            class="title ambre-home"
            href="/"
            title="All documents"
            aria-label="Ambre, all documents"
            on:click=go_home
        >
            "Ambre"
        </a>
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

/// Perform the half of a followed link that leaves the board (`TOILE-065`,
/// `TOILE-114`). An address opens in a fresh tab with `noopener,noreferrer`, so
/// the page can neither reach back into this window nor learn where it came
/// from. The board itself never fetches it (`TOILE-110`). A document opens the
/// same way the launcher opens one, reloading only when the target's render kind
/// differs from the booted one.
fn follow_shell_nav(
    nav: toile_chrome::ShellNav,
    navigate: &impl Fn(&str, leptos_router::NavigateOptions),
    booted: ambre_bus::DocType,
) {
    match nav {
        toile_chrome::ShellNav::Url(url) => crate::host::open_address(&url),
        toile_chrome::ShellNav::Document {
            document_id,
            doc_type,
        } => {
            let path = crate::route::doc_path(doc_type, document_id);
            if crate::route::needs_reload(doc_type, booted) {
                if let Some(window) = web_sys::window() {
                    let _ = window.location().set_href(&path);
                }
            } else {
                navigate(&path, Default::default());
            }
        }
    }
}

/// The single root view, rendered inside `<Router>` so it can read the client
/// route. Renders the persistent chrome + canvas and overlays the landing on the
/// `/` route. The document binding (open/close, canvas swap, rAF gate) is driven
/// OUTSIDE this tree by the per-frame route poll (see `pre_frame` in `main`).
/// The canvas handle is `!Send`, and the `<Router>` subtree must be Send.
#[component]
#[expect(
    clippy::expect_used,
    reason = "`mount_shell` provides the wiring before it mounts the root"
)]
pub(crate) fn AppRoot() -> impl IntoView {
    let Wiring {
        to_logic,
        theme_ctl,
        toggles,
        toile_toggles,
        style_state,
        toasts,
        username,
        docs,
        members,
        dev_mode,
        current_doc,
        filter_popover,
        filter_values,
        find_state,
        find_result,
        find_state_toile,
        find_result_toile,
        booted,
        prose_figures,
        prose_zoom,
        prose_signals,
        formula_ref,
        formula_text,
        formula_action,
        tofu_prompt,
        tofu_confirm,
        opened_file,
        keys,
        invite_role,
        invite_link,
        me,
        gate,
        sel_stats,
        trame_locale,
        sel_stats_toile,
        present_toile,
        presenter_slide,
        presenter_notes,
        broadcasting_toile,
        following_toile,
        presenter_peer_toile,
        perf_snap,
        zoom,
        board_zoom,
        ctx_scope,
        filter_active,
        toile_ctx_scope,
        toile_active_tool,
        toile_cursor,
        toile_style_model,
        toile_restyle,
        toile_float_bar,
        toile_quick_style,
        toile_cover,
        toile_comments,
        toile_deck,
        toile_dropped_file,
        toile_placed_embed,
        toile_set_link,
        toile_followed_link,
        sort_dialog,
        chrome_status,
        sheets,
        active_sheet,
        commands,
        toile_dialogs,
        trame_relay,
        toile_relay,
        murmure_relay,
    } = use_context::<Wiring>().expect("Wiring provided by mount_shell");

    // The reactive client route, used only to show/hide the landing overlay.
    let location = use_location();
    let is_landing = move || {
        matches!(
            route::parse(&location.pathname.get()),
            route::Route::Landing
        )
    };

    // Theme toggle drives only the button glyph; `theme_ctl` (canvas) and the
    // document `data-theme` (CSS custom properties) are the real switches.
    let theme_sig = RwSignal::new(theme_ctl.get());
    // The kind of the view this page booted, fixed per page load.
    let boot_is_toile = booted == DocType::Toile;
    let boot_is_prose = booted == DocType::Prose;
    let boot_is_murmure = booted == DocType::Murmure;
    let boot_is_trame = booted == DocType::Trame;

    // About modal. Every `Dialog` suspends the canvas rAF through the
    // `ModalHold` the shell provides, so no modal here wires the loop.
    let about_open = RwSignal::new(ambre_chrome::Openness::Closed);

    // Share modal. Opening it requests the current member list so the dialog
    // renders it; the relay fills `members` on the `DocumentMembers` response.
    let share_open = RwSignal::new(ambre_chrome::Openness::Closed);
    let share_on_open = Callback::new({
        let to_logic = to_logic.clone();
        move |()| {
            if let Some(document_id) = current_doc.get_untracked() {
                to_logic.send(LogicCommand::Rest {
                    id: ambre_bus::next_request(),
                    cmd: RestCommand::ListDocumentMembers { document_id },
                });
            }
        }
    });

    // Follow the outbound half of a link the board handed back (`toile-links`).
    // The board decided; this only performs, which is why nothing here reads a
    // target or writes one.
    {
        let navigate = leptos_router::hooks::use_navigate();
        Effect::new(move |_| {
            let Some(nav) = toile_followed_link.get() else {
                return;
            };
            toile_followed_link.set(None);
            follow_shell_nav(nav, &navigate, booted);
        });
    }
    // A Prose link followed with the modifier leaves through the same shell
    // path (`PROSE-UI-071`): the page hands the target over and never opens it.
    if boot_is_prose {
        let navigate = leptos_router::hooks::use_navigate();
        Effect::new(move |_| {
            let Some(url) = prose_signals.followed.get() else {
                return;
            };
            prose_signals.followed.set(None);
            follow_shell_nav(toile_chrome::ShellNav::Url(url), &navigate, booted);
        });
        // The link cursor over a link's text, the text cursor elsewhere
        // (`PROSE-UI-153`).
        Effect::new(move |_| {
            let cursor = match prose_signals.over_link.get() {
                ambre_prose::chrome::LinkHover::OverLink => "pointer",
                ambre_prose::chrome::LinkHover::Elsewhere => "text",
            };
            if let Some(doc) = web_sys::window().and_then(|w| w.document())
                && let Some(el) = doc.get_element_by_id(
                    ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT.as_str(),
                )
                && let Some(html) = el.dyn_ref::<web_sys::HtmlElement>()
            {
                let _ = html.style().set_property("cursor", cursor);
            }
        });
    }

    // Invite form state: the typed invitee (email, or a bare username in dev
    // mode). The role to grant lives in `invite_role` (shared via `Wiring`, since
    // the two-phase TOFU invite dispatches phase 2 outside this component).
    let invitee_input = RwSignal::new(String::new());

    // Canvas right-click -> context menu (the launcher owns the listener since
    // the canvas is created by the AppShell).
    let ctx_menu = RwSignal::new(None::<ambre_core::Point<ambre_core::Px<ambre_core::Css>>>);
    // A Murmure right-click off any message opens nothing. Its menu holds
    // the actions of the message under the pointer (the Murmure message list
    // spec). The view names it before the browser's menu event.
    install_canvas_context_menu(
        ctx_menu,
        Signal::derive(move || {
            (boot_is_toile && present_toile.get().is_presenting())
                || (boot_is_murmure && murmure_relay.target.with(Option::is_none))
        }),
        // A long press or a key names the point of the menu itself.
        Signal::derive(move || {
            (boot_is_murmure
                && murmure_relay
                    .target
                    .with(|t| t.as_ref().is_some_and(|t| t.pressed.is_some())))
                || (boot_is_toile && toile_ctx_scope.with(|s| s.keyed.is_some()))
                || (boot_is_prose && prose_signals.menu.with(Option::is_some))
                || (boot_is_trame && ctx_scope.with(|s| s.keyed.is_some()))
        }),
    );
    if boot_is_prose {
        // A key opens the page menu at the caret, which the view names in its
        // canvas (`prose-clipboard`). The point lives as long as its menu.
        Effect::new(move |_| {
            if let Some(at) = prose_signals.menu.get() {
                ctx_menu.set(Some(
                    at.on_page(prose_signals.canvas_origin.get_untracked()),
                ));
            }
        });
        Effect::new(move |_| {
            if ctx_menu.with(Option::is_none) && prose_signals.menu.with_untracked(Option::is_some)
            {
                prose_signals.menu.set(None);
            }
        });
    }
    if boot_is_toile {
        // A key opens the board's menu at the point the view named in its
        // canvas (`toile-menus`), and the point lives as long as its menu.
        Effect::new(move |_| {
            let Some(at) = toile_ctx_scope.with(|s| s.keyed) else {
                return;
            };
            // The view names the point in its canvas, and the menu needs the
            // client's.
            if let Some(canvas) = leptos::prelude::document()
                .get_element_by_id(ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT.as_str())
            {
                let rect = canvas.get_bounding_client_rect();
                let origin = ambre_core::Point::new(
                    ambre_core::Px::<ambre_core::Css>::new(rect.left() as f32),
                    ambre_core::Px::<ambre_core::Css>::new(rect.top() as f32),
                );
                ctx_menu.set(Some(at.on_page(origin)));
            }
        });
        Effect::new(move |_| {
            let mut scope = toile_ctx_scope.get_untracked();
            if ctx_menu.with(Option::is_none) && scope.keyed.is_some() {
                scope.keyed = None;
                toile_ctx_scope.set(scope);
            }
        });
    }
    if boot_is_trame {
        // A key opens the grid menu at the point the view named in its canvas
        // (`trame-context-menu`), and the point lives as long as its menu.
        Effect::new(move |_| {
            let Some(at) = ctx_scope.with(|s| s.keyed) else {
                return;
            };
            if let Some(canvas) = leptos::prelude::document()
                .get_element_by_id(ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT.as_str())
            {
                let rect = canvas.get_bounding_client_rect();
                let origin = ambre_core::Point::new(
                    ambre_core::Px::<ambre_core::Css>::new(rect.left() as f32),
                    ambre_core::Px::<ambre_core::Css>::new(rect.top() as f32),
                );
                ctx_menu.set(Some(at.on_page(origin)));
            }
        });
        Effect::new(move |_| {
            let mut scope = ctx_scope.get_untracked();
            if ctx_menu.with(Option::is_none) && scope.keyed.is_some() {
                scope.keyed = None;
                ctx_scope.set(scope);
            }
        });
    }
    if boot_is_murmure {
        // A long press opens the same menu where the finger held, and a key
        // below the focused row. The view names that point in its canvas, and
        // the menu needs the client's.
        Effect::new(move |_| {
            let Some((target, pressed)) = murmure_relay
                .target
                .with(|t| t.as_ref().and_then(|t| Some((t.canvas, t.pressed?))))
            else {
                return;
            };
            if let Some(canvas) = leptos::prelude::document().get_element_by_id(target.as_str()) {
                let rect = canvas.get_bounding_client_rect();
                let origin = ambre_core::Point::new(
                    ambre_core::Px::<ambre_core::Css>::new(rect.left() as f32),
                    ambre_core::Px::<ambre_core::Css>::new(rect.top() as f32),
                );
                ctx_menu.set(Some(pressed.on_page(origin)));
            }
        });
        // The target lives as long as its menu: once it closes, a palette
        // run of an action finds no message to act on.
        Effect::new(move |_| {
            if ctx_menu.with(Option::is_none) {
                murmure_relay.target.set(None);
            }
        });
    }
    // A file dropped on a board reaches the importer at the point it landed on
    // (`toile-import-export`). Same reason the listener lives here: the canvas
    // belongs to the AppShell, not to the view that paints into it.
    if boot_is_toile {
        install_board_file_drop(toile_dropped_file, toasts);
    }
    // A file dropped on a channel uploads for its draft
    // (`ambre-spec-murmure-attachments.md`).
    if boot_is_murmure {
        crate::platform::install_murmure_file_drop(murmure_relay);
    }

    // Toile cursor feedback (toile-tools): reflect the active tool on the
    // canvas element as a CSS cursor. That is crosshair while a creation
    // tool is armed, grab for Hand, arrow for Select. A pure function of the
    // shared active-tool signal, applied here in the chrome -- it never
    // touches the canvas render pass. Mounted only for a Toile board.
    if boot_is_toile {
        Effect::new(move |_| {
            // A position-dependent override wins: a directional resize or grab
            // cursor over a selection edge or corner, or during a transform.
            // Otherwise the cursor is the pure function of the active tool.
            let cursor = toile_cursor
                .get()
                .unwrap_or_else(|| ambre_toile::ui::palette::cursor_css(toile_active_tool.get()));
            if let Some(doc) = web_sys::window().and_then(|w| w.document())
                && let Some(el) = doc.get_element_by_id(
                    ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT.as_str(),
                )
                && let Some(html) = el.dyn_ref::<web_sys::HtmlElement>()
            {
                let _ = html.style().set_property("cursor", cursor.as_str());
            }
        });
    }

    // Every trigger shares one command path: toolbar, selection bar, context
    // menu, palette. The booted application's handler says what an id means,
    // and the shell performs the answer (`ARCH-018`). `Callback` is
    // `Copy`, so it is handed to each app-chrome component below.
    let palette_open = commands.palette_open;
    let seal_open = commands.seal_open;
    let on_command = Callback::new({
        let commands = commands.clone();
        move |id: StaticKey<Commands>| commands.run(id)
    });
    // The entries the booted application adds to the palette: the channels
    // of a Murmure page, none for the others.
    let murmure = commands.handler.murmure();
    let palette_entries = Signal::derive(move || {
        murmure
            .map(|h| h.relay.palette_entries().get())
            .unwrap_or_default()
    });
    let on_palette_entry = Callback::new(move |key: ambre_chrome::PaletteKey| {
        if let Some(h) = murmure {
            h.choose(key);
        }
    });

    // A panel's intent becomes its application's command in that application's
    // crate (`ARCH-018`). The shell only sends it to the routed document.
    let trame = {
        let commands = commands.clone();
        move |intent: trame_chrome::TrameIntent| commands.send(trame_relay.command(intent))
    };
    let toile = {
        let commands = commands.clone();
        move |intent: toile_chrome::ToileIntent| {
            if let Some(command) = toile_relay.command(intent) {
                commands.send(command);
            }
        }
    };
    use toile_chrome::ToileIntent;
    use trame_chrome::TrameIntent;
    let on_style = Callback::new({
        let trame = trame.clone();
        move |patch| trame(TrameIntent::Style(patch))
    });
    let on_filter_apply = Callback::new({
        let trame = trame.clone();
        move |kind| trame(TrameIntent::Filter(kind))
    });
    let on_sort_apply = Callback::new({
        let trame = trame.clone();
        move |req| trame(TrameIntent::Sort(req))
    });
    let on_find = Callback::new({
        let trame = trame.clone();
        move |q| trame(TrameIntent::Find(q))
    });
    let on_find_toile = Callback::new({
        let toile = toile.clone();
        move |q| toile(ToileIntent::Find(q))
    });
    let on_present_intent = Callback::new({
        let toile = toile.clone();
        move |intent| toile(ToileIntent::Present(intent))
    });
    let on_replace = Callback::new({
        let trame = trame.clone();
        move |r| trame(TrameIntent::Replace(r))
    });
    let on_replace_all = Callback::new({
        let trame = trame.clone();
        move |r| trame(TrameIntent::ReplaceAll(r))
    });
    let on_sheet_activate = Callback::new({
        let trame = trame.clone();
        move |id| trame(TrameIntent::ActivateSheet(id))
    });
    let on_sheet_create = Callback::new({
        let trame = trame.clone();
        move |()| trame(TrameIntent::CreateSheet)
    });
    let on_sheet_rename = Callback::new({
        let trame = trame.clone();
        move |(id, name)| trame(TrameIntent::RenameSheet(id, name))
    });
    let on_sheet_delete = Callback::new({
        let trame = trame.clone();
        move |id| trame(TrameIntent::DeleteSheet(id))
    });
    let on_sheet_reorder = Callback::new(move |req| trame(TrameIntent::ReorderSheet(req)));

    // The menu layout follows the right-click scope the view published
    // (`trame-context-menu`). A cell right-click gets the full menu, a header the
    // header-scoped one (no clipboard / merge). A Toile board builds its menu
    // from the selection scope the canvas view publishes (`toile-menus`); a Trame
    // sheet from the cell/header scope (`trame-context-menu`). Both return
    // `ambre_chrome::MenuEntry` rows, so one `<ContextMenu>` renders either.
    let ctx_entries = Memo::new(move |_| {
        if boot_is_toile {
            toile_chrome::context_entries_for(&toile_ctx_scope.get())
        } else if boot_is_prose {
            ambre_prose::chrome::context_entries(prose_signals.history.get())
        } else if boot_is_murmure {
            murmure_relay.menu_entries().get()
        } else {
            trame_chrome::context_entries_for(ctx_scope.get().scope, filter_active.get())
        }
    });

    // Per-slot sender clones. Each slot's children is a `Fn`, so it captures its
    // own clone and re-clones inside the handler rather than moving the sender
    // out. Moving it out would make the children `FnOnce`.
    let tx_invite = to_logic.clone();
    // Every command a member row issues, stamped with a request id on its way
    // to the worker. A `Callback` is `Copy`, so each row of the `<For>` holds it.
    let member_send = Callback::new({
        let tx = to_logic.clone();
        move |cmd| {
            tx.send(LogicCommand::Rest {
                id: ambre_bus::next_request(),
                cmd,
            });
        }
    });
    let landing_signals = LandingSignals {
        docs,
        username,
        dev_mode,
        booted,
        toasts,
        opened_file,
        keys,
        gate,
    };
    let landing_tx = to_logic.clone();

    // The context panel of a document page: one frame for Prose, Trame and
    // Toile, its view state kept on this device (`ambre-spec-shell.md`
    // section 2). Murmure lays out no context panel.
    let panel_prefs = (!boot_is_murmure).then(|| super::panel::panel_prefs(booted));
    let panel_content = ambre_chrome::PanelContent::empty();

    let header = {
        let theme_ctl = theme_ctl.clone();
        move || {
            view! {
                <ShellHeader
                    theme_ctl=theme_ctl.clone()
                    theme_sig=theme_sig
                    toasts=toasts
                    chrome_status=chrome_status
                    current_doc=current_doc
                    share_open=share_open
                    about_open=about_open
                    username=username
                    me=me
                    docs=docs
                    dock_side=panel_prefs.map(|p| p.side)
                />
            }
        }
    };
    // A Murmure page lays out its channel list, its channel and its thread
    // (the Murmure layout spec). It has no formula bar, tabs or status bar.
    let shell = if boot_is_murmure {
        let send = super::commands::murmure_send(to_logic.clone());
        view! {
            <MurmureShell relay=murmure_relay send=send invite_link=invite_link>
                {header.clone()}
            </MurmureShell>
        }
        .into_any()
    } else {
        view! {
        <AppShell rows=shell_rows(booted)>
            <Header slot>{header.clone()}</Header>
            <ContextRegion slot>
                {panel_prefs.map(|prefs| view! {
                    <ambre_chrome::ContextPanel
                        prefs=prefs
                        title=panel_content.title
                        sections=panel_content.sections
                    />
                })}
            </ContextRegion>
            // Each application lays out its own rows (`CHROME-140`). The formula
            // bar is Trame's alone, and its sheet tabs sit in the status bar. A Prose page has no
            // command bar. Gated on the booted app kind, fixed per page load.
            <Toolbar slot>
                {boot_is_trame.then(|| view! {
                    <trame_chrome::FormatBar
                        toggles=toggles
                        style=style_state
                        theme=theme_sig
                        on_command=on_command
                        on_style=on_style
                    />
                })}
                // Toile docks its menus and its zoom in the command bar, under
                // the header, so neither covers the header (`CHROME-141`).
                {boot_is_toile.then(|| view! {
                    <toile_chrome::EditingSurfaces present=present_toile>
                        <toile_chrome::TopMenuBar on_command=on_command toggles=toile_toggles />
                    </toile_chrome::EditingSurfaces>
                })}
            </Toolbar>
            <FormulaBar slot>
                {boot_is_trame.then(|| view! {
                    <trame_chrome::FormulaBar
                        cell_ref=formula_ref
                        content=formula_text
                        on_action=Callback::new(move |action| formula_action.set(Some(action)))
                        locale=trame_locale
                    />
                })}
            </FormulaBar>
            <StatusBar slot>
                {if boot_is_prose {
                    // The five figures of the Prose document
                    // (`PROSE-FEAT-040`), in the shared status band.
                    view! { <ProseFigures figures=prose_figures zoom=prose_zoom /> }.into_any()
                } else if boot_is_toile {
                    // Toile selection status bar: count/bounds + shared
                    // zoom & fps chips, styled identically to Trame's band.
                    view! {
                        <toile_chrome::StatusBar
                            stats=sel_stats_toile
                            perf=perf_snap
                            zoom=board_zoom
                        />
                    }
                    .into_any()
                } else {
                    view! {
                        // The sheet tabs sit in the band, before the
                        // zoom (`CHROME-162`).
                        <trame_chrome::StatusBar
                            stats=sel_stats
                            perf=perf_snap
                            zoom=zoom
                            locale=trame_locale
                            tabs=move || view! {
                                <trame_chrome::SheetTabs
                                    sheets=sheets
                                    active=active_sheet
                                    on_activate=on_sheet_activate
                                    on_create=on_sheet_create
                                    on_rename=on_sheet_rename
                                    on_delete=on_sheet_delete
                                    on_reorder=on_sheet_reorder
                                />
                            }
                        />
                    }
                    .into_any()
                }}
            </StatusBar>
        </AppShell>
        }
        .into_any()
    };

    view! {
        {shell}

        <ambre_chrome::PassphraseDialog
            open=seal_open
            intent=ambre_chrome::PassphraseIntent::Seal
            on_submit=Callback::new({
                // The application wraps the passphrase in its sealed-save
                // command, and nothing else holds it (`ARCH-004`).
                let commands = commands.clone();
                move |passphrase: ambre_core::Confidential<ambre_core::Passphrase>| commands.seal(passphrase)
            })
        />

        <Dialog open=about_open class={StaticKey::<ambre_core::CssClasses>::new("ambre-about")}>
            <div class="ambre-about-panel">
                <h2>"Ambre"</h2>
                <p>"Collaborative office suite, 100% Rust, E2EE encrypted."</p>
                <div class="ambre-dialog-actions">
                    <button on:click=move |_| about_open.set(ambre_chrome::Openness::Closed)>"Close"</button>
                </div>
            </div>
        </Dialog>

        <Dialog
            open=share_open
            on_open=share_on_open
            class={StaticKey::<ambre_core::CssClasses>::new("ambre-share")}
        >
            <div class="ambre-share-panel">
                <h2>"Share document"</h2>

                // Current members.
                <ShareMembers
                    members=members
                    me=me
                    current_doc=current_doc
                    send=member_send
                />

                // The link of the last invite, to send to the invitee.
                <ShareInviteLink link=invite_link current_doc=current_doc />

                // Invite a new collaborator.
                <form
                    class="ambre-share-invite"
                    on:submit={
                        let tx = tx_invite.clone();
                        move |ev| {
                            ev.prevent_default();
                            let Some(document_id) = current_doc.get_untracked() else {
                                return;
                            };
                            let raw = invitee_input.get_untracked();
                            let raw = raw.trim();
                            if raw.is_empty() {
                                return;
                            }
                            // Dev convention: a bare username maps to the
                            // deterministic `<username>@dev.local` email the dev
                            // login assigns (server resolves invitees by email).
                            let invitee_email = if dev_mode
                                .get_untracked()
                                .is_some_and(landing::SignInMode::is_dev)
                                && !raw.contains('@')
                            {
                                ambre_bus::Email::new(format!("{raw}@dev.local"))
                            } else {
                                ambre_bus::Email::from(raw)
                            };
                            // TOFU phase 1 (`ambre-spec-key-verification.md`): fetch the
                            // invitee's key + fingerprint without wrapping. The role
                            // is kept in `invite_role`; the relay dispatches phase 2
                            // (the actual share) once the trust decision is made.
                            let sent = tx.send(LogicCommand::Rest {
                                id: ambre_bus::next_request(),
                                cmd: RestCommand::FetchInviteeKey {
                                    document_id,
                                    invitee_email,
                                },
                            });
                            // A refused invite keeps the address typed, so the
                            // user can send it again (`BUS-018`).
                            if sent.accepted() {
                                invitee_input.set(String::new());
                            }
                        }
                    }
                >
                    <input
                        r#type="text"
                        class="ambre-share-input"
                        aria-label="Invitee"
                        placeholder=move || {
                            if dev_mode.get().is_some_and(landing::SignInMode::is_dev) {
                                "username or email"
                            } else {
                                "user@example.com"
                            }
                        }
                        prop:value=move || invitee_input.get()
                        on:input=move |ev| invitee_input.set(event_target_value(&ev))
                    />
                    <select
                        aria-label="Role"
                        on:change=move |ev| {
                            if let Ok(role) = event_target_value(&ev).parse::<Role>() {
                                invite_role.set(role);
                            }
                        }
                    >
                        <option value="editor" selected=true>"Editor"</option>
                        <option value="viewer">"Viewer"</option>
                        <option value="owner">"Owner"</option>
                    </select>
                    <button r#type="submit">"Invite"</button>
                </form>

                <div class="ambre-dialog-actions">
                    <button on:click=move |_| share_open.set(ambre_chrome::Openness::Closed)>"Close"</button>
                </div>
            </div>
        </Dialog>

        // Toile's choosers: a web embed, a document embed and a link. The
        // launcher lends the document list it alone holds (`ARCH-018`).
        <toile_chrome::ToileChoosers
            dialogs=toile_dialogs
            docs=Signal::derive(move || {
                docs.get()
                    .into_iter()
                    .map(|d| toile_chrome::DocChoice {
                        document_id: d.document_id,
                        doc_type: d.doc_type,
                        name: d.name,
                    })
                    .collect::<Vec<_>>()
            })
            here=current_doc
            deck=toile_deck
            placed=toile_placed_embed
            set_link=toile_set_link
        />

        // Key verification (`ambre-spec-key-verification.md`): a modal for a
        // new or a changed key. The relay pins and dispatches what it confirms.
        <crate::tofu::TofuDialog prompt=tofu_prompt confirmed=tofu_confirm />

        <ContextMenu pos=ctx_menu entries=ctx_entries on_command=on_command />

        // Shared filter popover (`trame-shared-filter`), under the active column.
        <trame_chrome::FilterPopover
            state=filter_popover
            values=filter_values
            on_apply=on_filter_apply
            on_command=on_command
        />

        // The command palette (`CHROME-008`): every command the booted
        // application declares, searchable, whether or not a menu shows it. One
        // overlay for both, fed by the app's own table. Murmure lists its
        // channels before its commands (the Murmure keyboard spec).
        <ambre_chrome::CommandPalette
            open=palette_open
            table=palette_table(booted)
            on_command=on_command
            entries=palette_entries
            on_entry=on_palette_entry
        />
        // Find panel, pinned top-right. Trame ships `trame-find-replace`;
        // Toile ships a find-only panel (`toile-find`). The booted app kind
        // is fixed per page load, so exactly one is mounted.
        {(!boot_is_toile).then(|| view! {
            <trame_chrome::FindReplace
                state=find_state
                result=find_result
                on_find=on_find
                on_replace=on_replace
                on_replace_all=on_replace_all
            />
        })}
        {boot_is_toile.then(|| view! {
            <toile_chrome::FindPanel
                state=find_state_toile
                result=find_result_toile
                on_find=on_find_toile
            />
        })}
        // Toile presenter overlay panel (toile-presentation): speaker notes +
        // navigation, shown while presentation mode is active. Visibility is
        // driven by the `present_toile` signal (mirrored from PresentModeChanged);
        // the slide payload feeds the counter + notes editor.
        {boot_is_toile.then(|| view! {
            <toile_chrome::PresenterPanel
                open=present_toile
                slide=presenter_slide
                notes=presenter_notes
                broadcasting=broadcasting_toile
                following=following_toile
                presenter_peer=presenter_peer_toile
                on_intent=on_present_intent
            />
        })}
        // Toile tool palette / primary toolbar (toile-tools): a floating
        // vertical bar, mounted only for a Toile board. It writes the shared active-
        // tool signal the canvas view applies; the keyboard chords write the same one.
        // Every editing surface below leaves the DOM while the board presents.
        {boot_is_toile.then(|| view! {
            <toile_chrome::EditingSurfaces present=present_toile>
                <toile_chrome::ToolPalette active=toile_active_tool cover=toile_cover />
            </toile_chrome::EditingSurfaces>
        })}
        // Toile style / property inspector (toile-styles): a side panel shown only for
        // a non-empty selection. It reads the common style the canvas view publishes.
        // It reuses `toile_ctx_scope` for the lock buttons and the shared `on_command`
        // path for z-order / lock. It writes debounced whole-style edits into
        // `toile_restyle`, which the view turns into `RestyleElements`.
        {boot_is_toile.then(|| view! {
            <toile_chrome::EditingSurfaces present=present_toile>
                <toile_chrome::StyleInspector
                    model=toile_style_model
                    scope=toile_ctx_scope
                    on_command=on_command
                    restyle=toile_restyle
                />
            </toile_chrome::EditingSurfaces>
        })}
        // Toile content-anchored floating selection toolbar (toile-styles): follows
        // the selection as the board pans/zooms (the canvas view republishes its
        // client anchor into `toile_float_bar` each frame). Its command buttons
        // reuse the shared `on_command` path, a strict subset of the context menu.
        // The style controls (fill, text colour, font, size, bold, italic) seed from
        // the inspector `toile_style_model`. They write single-field `StyleEdit`
        // edits the view turns into a live-based `RestyleElements` over the
        // selection.
        {boot_is_toile.then(|| view! {
            <toile_chrome::EditingSurfaces present=present_toile>
                <toile_chrome::SelectionToolbar
                    bar=toile_float_bar
                    cover=toile_cover
                    model=toile_style_model
                    on_command=on_command
                    on_style=Callback::new(move |s: toile_chrome::StyleEdit| {
                        toile_quick_style.set(Some(s))
                    })
                />
            </toile_chrome::EditingSurfaces>
        })}

        // Toile comments (`toile-comments`): the board-wide thread list with the
        // one thread it shows whole, and the composer the comment tool opens
        // where the user clicked. Both hold nothing of their own
        // (`TOILE-UI-023`). They render what the canvas view read back from the
        // board, and hand every action to it as one intent.
        {boot_is_toile.then(|| view! {
            <toile_chrome::EditingSurfaces present=present_toile>
                <toile_chrome::CommentsPanel
                    state=toile_comments.panel
                    list=toile_comments.list
                    open=toile_comments.open
                    on_intent=Callback::new(move |i: toile_chrome::CommentIntent| {
                        toile_comments.intent.set(Some(i))
                    })
                />
                <toile_chrome::CommentComposer
                    draft=toile_comments.draft
                    on_intent=Callback::new(move |i: toile_chrome::CommentIntent| {
                        toile_comments.intent.set(Some(i))
                    })
                />
            </toile_chrome::EditingSurfaces>
        })}

        // Prose annotations: the comment panel and the suggestion popover,
        // both DOM chrome (`PROSE-UI-050`, `PROSE-UI-052`).
        {boot_is_prose.then(|| view! {
            <ambre_prose::chrome::ProseCommentsPanel signals=prose_signals />
            <ambre_prose::chrome::ProseSuggestionPopover signals=prose_signals />
            <ambre_prose::chrome::ProseSpellingPopover signals=prose_signals />
            <ambre_prose::chrome::ProseFormatBar signals=prose_signals />
            <ambre_prose::chrome::ProseSlashMenu signals=prose_signals />
            <ambre_prose::chrome::ProseRuler signals=prose_signals />
        })}

        // The deck panel (`toile-presentation`): the surface that adds a frame
        // to the presentation, reorders it and removes it (`TOILE-FEAT-026`).
        // It holds no order of its own. It renders the frames the canvas view
        // read back from the board, and hands each edit to it as one intent.
        {boot_is_toile.then(|| view! {
            <toile_chrome::EditingSurfaces present=present_toile>
                <toile_chrome::DeckPanel
                    state=toile_deck.panel
                    view=toile_deck.view
                    on_intent=Callback::new(move |i: toile_chrome::DeckIntent| {
                        toile_deck.intent.set(Some(i))
                    })
                />
            </toile_chrome::EditingSurfaces>
        })}

        // Multi-column sort dialog (`trame-sort`), from "Sort range...".
        <trame_chrome::SortDialog state=sort_dialog on_apply=on_sort_apply />

        // Live-region toast host (announces transient messages politely).
        <ToastHost toasts=toasts />

        // Document-loading overlay (`-chrome.md` 2.G): covers the canvas while a
        // freshly opened document hydrates (snapshot fetch + decrypt), dismissed
        // on `UiEvent::DocumentReady`. Reuses the boot loader's spinner visual.
        // Never shown on the landing route -- `hydrating` is cleared there.
        <Show when=move || chrome_status.get().hydrating>
            <div class="ambre-doc-loading">
                <div class="spinner"></div>
                <p>"Loading..."</p>
            </div>
        </Show>

        // Landing overlay, shown on `/` over the (frozen, hidden) canvas.
        <Show when=is_landing>
            {
                let landing_tx = landing_tx.clone();
                move || landing::landing_view(landing_signals, landing_tx.clone())
            }
        </Show>
    }
}

/// Copy or cut from a menu: focus the text pipe and ask the browser to fire
/// the `copy`/`cut` DOM event the host listeners service. They write every
/// format the active view offers.
pub(super) fn exec_clipboard(write: ClipboardWrite) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Ok(Some(ta)) = doc.query_selector(".ambre-input")
            && let Some(el) = ta.dyn_ref::<web_sys::HtmlElement>()
        {
            let _ = el.focus();
        }
        let cmd = match write {
            ClipboardWrite::Copy => "copy",
            ClipboardWrite::Cut => "cut",
        };
        if let Ok(html_doc) = doc.dyn_into::<web_sys::HtmlDocument>() {
            let _ = html_doc.exec_command(cmd);
        }
    }
}

/// Paste from a menu. `execCommand("paste")` is blocked by browsers, so read
/// the OS clipboard via the async Clipboard API, allowed in this click
/// gesture. The read is decoded and handed to the host, which routes it to the
/// active view through the same ladder as Ctrl+V (`UI-THREAD-024`,
/// `UI-THREAD-025`). A `Plain` form keeps the text alone (`PROSE-UI-227`).
pub(super) fn read_clipboard(
    paste_tx: ambre_bus::conduit::Sender<crate::host::ClipboardPaste>,
    toasts: Toasts,
    form: PasteForm,
) {
    let clipboard = web_sys::window().map(|w| w.navigator().clipboard());
    // `navigator.clipboard` is undefined outside a secure context, such as
    // http:// on a LAN IP. There the entry cannot read at all, so tell the
    // user instead of failing silently.
    let available = clipboard
        .as_ref()
        .map(|c| {
            let v: &wasm_bindgen::JsValue = c.as_ref();
            !v.is_undefined() && !v.is_null()
        })
        .unwrap_or(false);
    if let Some(clipboard) = clipboard.filter(|_| available) {
        // Call `read()` synchronously, still inside the click's user gesture:
        // Firefox shows its mandatory "paste" confirmation right away instead
        // of deferring it to the next click.
        let read_promise = clipboard.read();
        wasm_bindgen_futures::spawn_local(paste_from_clipboard(
            read_promise,
            clipboard,
            paste_tx,
            toasts,
            form,
        ));
    } else {
        toasts.warning("Clipboard unavailable here (needs HTTPS); use Ctrl+V to paste");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// The palette is fed each application's whole table, not a menu's subset
    /// (`CHROME-008`). A Toile menu lists fewer ids than the board declares.
    #[wasm_bindgen_test]
    fn the_palette_is_fed_the_whole_declared_table() {
        let ids = |table: &[ambre_chrome::commands::Command]| {
            table.iter().map(|c| c.id).collect::<Vec<_>>()
        };
        let fed = |kind| ids(palette_table(kind));
        assert_eq!(
            fed(DocType::Toile),
            ids(ambre_toile::ui::commands::COMMANDS)
        );
        assert_eq!(fed(DocType::Prose), ids(ambre_prose::commands::COMMANDS));
        assert_eq!(fed(DocType::Trame), ids(trame_chrome::COMMANDS));
        assert_eq!(
            fed(DocType::Murmure),
            ids(ambre_murmure::ui::commands::COMMANDS)
        );
        assert!(fed(DocType::Toile).len() > ambre_toile::ui::menu::COMMANDS.len());
    }

    /// The header names the open document (`CHROME-144`), and its way home
    /// moves the address bar home (`CHROME-145`) with no reload
    /// (`CHROME-146`). A page with no document
    /// shows the way home alone.
    #[wasm_bindgen_test]
    async fn the_header_links_home_and_names_the_open_document() {
        use wasm_bindgen::JsCast as _;
        let document_id = Id::<Document>::mint();
        let docs = RwSignal::new(vec![crate::landing::DocItem {
            document_id,
            doc_type: DocType::Prose,
            name: Some(ambre_core::Validated::<ambre_core::Name>::parse("Plan").expect("a name")),
            updated_at: None,
            member_count: ambre_core::Count::<ambre_core::Members>::ONE,
        }]);
        let current_doc = RwSignal::new(None);
        let page = web_sys::window()
            .and_then(|w| w.document())
            .expect("a document");
        let container = page.create_element("div").expect("a container");
        page.body()
            .expect("a body")
            .append_child(&container)
            .expect("appended");
        let target: web_sys::HtmlElement = container.clone().dyn_into().expect("an element");
        leptos::mount::mount_to(target, move || {
            view! {
                <leptos_router::components::Router>
                    <HeaderTitle docs=docs current_doc=current_doc />
                </leptos_router::components::Router>
            }
        })
        .forget();
        let query = |selector: &str| container.query_selector(selector).expect("a selector");
        let home = query("a.ambre-home").expect("the way home");
        assert_eq!(home.get_attribute("href").as_deref(), Some("/"));
        assert!(query(".ambre-doc-name").is_none(), "no document, no name");

        current_doc.set(Some(document_id));
        leptos::task::tick().await;
        let name = query(".ambre-doc-name").expect("the name of the open document");
        assert_eq!(name.text_content().as_deref(), Some("Plan"));

        // A plain click moves the address bar home, which the frame loop reads.
        let window = web_sys::window().expect("a window");
        let history = window.history().expect("a history");
        let runner = window.location().href().expect("the runner address");
        let doc = crate::route::doc_path(DocType::Prose, document_id);
        history
            .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&doc))
            .expect("a document address");
        home.dyn_ref::<web_sys::HtmlElement>()
            .expect("an element")
            .click();
        leptos::task::tick().await;
        let path = window.location().pathname().expect("a path");
        history
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&runner))
            .expect("the runner address back");
        assert_eq!(path, "/", "the address bar goes home");
        assert!(
            home.is_connected(),
            "the page did not reload (`CHROME-146`)"
        );
        container.remove();
    }

    /// Each application lays out its own rows (`CHROME-140`). The formula bar
    /// is Trame's alone. No application lays out the tab strip, since Trame's
    /// sheet tabs sit in the status bar (`CHROME-162`). A Prose page lays out
    /// no command bar.
    #[wasm_bindgen_test]
    fn the_formula_bar_is_trames_alone_and_prose_has_no_command_bar() {
        use ambre_chrome::RowShown::{Absent, Shown};
        let rows = |kind| {
            let r = shell_rows(kind);
            (r.toolbar, r.formula_bar, r.tabs)
        };
        assert_eq!(rows(DocType::Trame), (Shown, Shown, Absent));
        assert_eq!(rows(DocType::Toile), (Shown, Absent, Absent));
        assert_eq!(rows(DocType::Prose), (Absent, Absent, Absent));
    }

    /// `TOILE-065`: a followed web address opens in a fresh tab that can neither
    /// reach back into this window nor learn where it came from. The page's own
    /// `open` is swapped for a recorder, so the test opens no tab.
    #[wasm_bindgen_test]
    fn a_web_address_opens_in_a_fresh_tab_with_no_opener_and_no_referrer() {
        let window = web_sys::window().expect("a window");
        let key = wasm_bindgen::JsValue::from_str("open");
        let original = js_sys::Reflect::get(&window, &key).expect("window.open");
        let recorder = js_sys::Function::new_with_args(
            "url, target, features",
            "globalThis.__ambreOpened = [url, target, features]; return null;",
        );
        js_sys::Reflect::set(&window, &key, &recorder).expect("a writable open");
        let navigated = std::cell::Cell::new(false);
        follow_shell_nav(
            toile_chrome::ShellNav::Url("https://example.com/a".into()),
            &|_: &str, _| navigated.set(true),
            DocType::Toile,
        );
        js_sys::Reflect::set(&window, &key, &original).expect("open restored");
        let opened = js_sys::Reflect::get(&window, &"__ambreOpened".into()).expect("a record");
        let opened: Vec<String> = js_sys::Array::from(&opened)
            .iter()
            .filter_map(|v| v.as_string())
            .collect();
        assert_eq!(
            opened,
            ["https://example.com/a", "_blank", "noopener,noreferrer"]
        );
        assert!(!navigated.get(), "an address moved this tab");
    }

    /// Audit `P49`: a followed address that is no web address
    /// opens nothing, whatever reached the shell. The page's own `open` is
    /// swapped for a recorder, which stays empty.
    #[wasm_bindgen_test]
    fn an_address_that_is_no_web_address_opens_nothing() {
        let window = web_sys::window().expect("a window");
        let key = wasm_bindgen::JsValue::from_str("open");
        let original = js_sys::Reflect::get(&window, &key).expect("window.open");
        let recorder = js_sys::Function::new_with_args(
            "url",
            "globalThis.__ambreRefused = (globalThis.__ambreRefused || 0) + 1; return null;",
        );
        js_sys::Reflect::set(&window, &key, &recorder).expect("a writable open");
        for hostile in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ms-officecmd:open",
            "https://exa mple.com",
        ] {
            follow_shell_nav(
                toile_chrome::ShellNav::Url(hostile.into()),
                &|_: &str, _| {},
                DocType::Toile,
            );
        }
        js_sys::Reflect::set(&window, &key, &original).expect("open restored");
        let opened = js_sys::Reflect::get(&window, &"__ambreRefused".into()).expect("a record");
        assert!(opened.is_undefined(), "an address opened a tab");
    }

    /// `TOILE-114`: a followed link to another Ambre document is opened by the
    /// shell's own router, at the document's path, and opens no tab.
    #[wasm_bindgen_test]
    fn a_document_link_is_opened_by_the_shells_router() {
        let document_id = Id::<Document>::mint();
        let to = std::cell::RefCell::new(Vec::new());
        follow_shell_nav(
            toile_chrome::ShellNav::Document {
                document_id,
                doc_type: DocType::Toile,
            },
            &|path: &str, _| to.borrow_mut().push(path.to_owned()),
            DocType::Toile,
        );
        assert_eq!(
            *to.borrow(),
            [crate::route::doc_path(DocType::Toile, document_id)]
        );
    }
}
