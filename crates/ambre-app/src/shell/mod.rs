// ambre-app/src/shell.rs
// ============================================================================
// Leptos shell. `mount_shell` creates all reactive state inside the mount owner,
// pulling `!Send` handles back out via slots, and mounts the `<Router>`.
// `AppRoot` is the single root view: persistent chrome, canvas and landing
// overlay. The document binding is driven outside this tree by the per-frame
// route poll in `main`.
// ============================================================================

use super::*;

// AppRoot lives in a sibling submodule; it reaches the launcher imports and
// `Wiring` via `use super::super::*`.
mod app_root;
mod commands;
#[cfg(test)]
mod murmure_regions;
#[cfg(test)]
mod narrow;
pub(crate) mod prose_status;
mod share;
mod sync_badge;
use app_root::AppRoot;
pub(crate) use commands::{AppHandler, CommandPath};

/// Each application's relay, read back by the per-frame loop (`ARCH-018`).
pub(crate) type Relays = (
    trame_chrome::TrameRelay,
    toile_chrome::ToileRelay,
    ambre_murmure::chrome::MurmureRelay,
);

/// Mount the Leptos shell onto the page body: the persistent AppShell chrome
/// (which owns the `<canvas>`) plus the landing overlay shown on `/`. All
/// reactive state is created here (the mount owner) and published via the slots
/// so the imperative engine wiring in `main` can read it back.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mount_shell(
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    clipboard_paste: ambre_bus::conduit::Sender<ClipboardPaste>,
    raf: ambre_canvas::RafControl,
    theme_ctl: ambre_canvas::ThemeControl,
    boot_doc: Option<DocumentId>,
    store_slot: Slot<CommandStates>,
    toile_store_slot: Slot<CommandStates>,
    anchor_slot: Slot<RwSignal<Option<(f64, f64)>>>,
    toasts_slot: Slot<Toasts>,
    user_slot: Slot<RwSignal<Option<String>>>,
    docs_slot: Slot<RwSignal<Vec<DocItem>>>,
    members_slot: Slot<RwSignal<Vec<MemberInfo>>>,
    devmode_slot: Slot<RwSignal<Option<landing::SignInMode>>>,
    current_doc_slot: Slot<RwSignal<Option<DocumentId>>>,
    relays_slot: Slot<Relays>,
    boot_is_toile: bool,
    boot_is_prose: bool,
    boot_is_murmure: bool,
    prose_figures: RwSignal<Option<ambre_prose::ui::StatusFigures>>,
    prose_signals: crate::host::ProseSignals,
    style_slot: Slot<trame_chrome::StyleState>,
    chrome_status_slot: Slot<RwSignal<ChromeStatus>>,
    formula_ref: RwSignal<String>,
    formula_text: RwSignal<String>,
    formula_action: RwSignal<Option<FormulaAction>>,
    tofu_prompt: RwSignal<Option<TofuPrompt>>,
    tofu_confirm: RwSignal<Option<TofuPrompt>>,
    invite_role: RwSignal<Role>,
    me: RwSignal<Option<UserId>>,
    sel_stats: RwSignal<Option<trame_chrome::SelStats>>,
    sel_stats_toile: RwSignal<Option<toile_chrome::SelStats>>,
    find_result: RwSignal<Option<trame_chrome::FindPosition>>,
    find_result_toile: RwSignal<Option<ambre_bus::app::toile::FindResults>>,
    present_toile: RwSignal<PresentMode>,
    presenter_slide: RwSignal<SlidePosition>,
    presenter_notes: RwSignal<String>,
    presenter_frame: RwSignal<Option<ambre_bus::app::toile::ElementId>>,
    broadcasting_toile: RwSignal<Broadcast>,
    following_toile: RwSignal<Option<ambre_bus::UserId>>,
    presenter_peer_toile: RwSignal<Option<ambre_bus::UserId>>,
    perf_snap: RwSignal<ambre_canvas::PerfSnapshot>,
    zoom: RwSignal<ZoomFactor>,
    ctx_scope: RwSignal<trame_chrome::CtxScope>,
    filter_active: RwSignal<trame_chrome::SheetFilter>,
    toile_ctx_scope: RwSignal<ambre_toile::ui::menu::CtxScope>,
    toile_menu_cmd: RwSignal<Option<ambre_chrome::commands::CommandId>>,
    toile_active_tool: RwSignal<ambre_toile::ui::Tool>,
    toile_cursor: RwSignal<Option<ambre_toile::ui::palette::CanvasCursor>>,
    toile_style_model: RwSignal<Option<toile_chrome::StyleModel>>,
    toile_restyle: RwSignal<Option<ambre_bus::app::toile::ElementStyle>>,
    toile_float_bar: RwSignal<Option<toile_chrome::FloatBar>>,
    toile_quick_style: RwSignal<Option<toile_chrome::StyleEdit>>,
    toile_comments: toile_chrome::CommentSignals,
    toile_deck: toile_chrome::DeckSignals,
    toile_dropped_file: RwSignal<Option<toile_chrome::DroppedFile>>,
    toile_placed_embed: RwSignal<Option<toile_chrome::PlacedEmbed>>,
    toile_set_link: RwSignal<Option<toile_chrome::LinkEdit>>,
    toile_followed_link: RwSignal<Option<toile_chrome::ShellNav>>,
    sheets: RwSignal<Vec<SheetTab>>,
    active_sheet: RwSignal<SheetId>,
    commands_slot: Slot<CommandPath>,
) {
    leptos::mount::mount_to_body(move || {
        // The store stays here (in the mount owner, NOT in the view): it is `Rc`
        // and the per-frame relay drives it. The shell sees only its toggle
        // signals (Send), so the `<Router>` subtree stays Send.
        let store = CommandStates::new(trame_chrome::COMMANDS.iter().map(|c| c.id));
        // Seed app-specific initial command states (Undo/Redo start disabled).
        trame_chrome::init_command_states(&store);
        let toggles = trame_chrome::Toggles::from_store(&store);
        *store_slot.borrow_mut() = Some(store);
        // Toile's own store: the two Undo/Redo rows read its enabled signals,
        // fed by `ToileEvent::UndoState` through the launcher's relay.
        let toile_store = CommandStates::new(toile_chrome::COMMANDS.iter().map(|c| c.id));
        toile_chrome::init_command_states(&toile_store);
        let toile_toggles = toile_chrome::Toggles::from_store(&toile_store);
        *toile_store_slot.borrow_mut() = Some(toile_store);
        let anchor_pos = RwSignal::new(None::<(f64, f64)>);
        *anchor_slot.borrow_mut() = Some(anchor_pos);
        let toasts = Toasts::new();
        *toasts_slot.borrow_mut() = Some(toasts);
        let username = RwSignal::new(None::<String>);
        *user_slot.borrow_mut() = Some(username);
        let docs = RwSignal::new(Vec::<DocItem>::new());
        *docs_slot.borrow_mut() = Some(docs);
        let members = RwSignal::new(Vec::<MemberInfo>::new());
        *members_slot.borrow_mut() = Some(members);
        let dev_mode = RwSignal::new(None::<landing::SignInMode>);
        *devmode_slot.borrow_mut() = Some(dev_mode);
        let current_doc = RwSignal::new(boot_doc);
        *current_doc_slot.borrow_mut() = Some(current_doc);
        let filter_popover = RwSignal::new(None::<trame_chrome::FilterPopoverState>);
        let filter_values = RwSignal::new(Vec::<String>::new());
        let find_state = RwSignal::new(None::<trame_chrome::FindReplaceState>);
        let find_state_toile = RwSignal::new(toile_chrome::FindPanelState::Closed);
        // The multi-column sort dialog (`trame-sort`), which Trame's relay
        // opens on the Logic thread's reply.
        let sort_dialog = RwSignal::new(None::<trame_chrome::SortDialogState>);
        // Each application fills its panels from its own events, and turns
        // their intents into its own commands (`ARCH-018`). The per-frame relay
        // reads both back through the slot.
        let trame_relay = trame_chrome::TrameRelay {
            sort_dialog,
            filter_popover,
            filter_values,
            sheets,
            sel_stats,
        };
        let toile_relay = toile_chrome::ToileRelay {
            sel_stats: sel_stats_toile,
            presenter_slide,
            presenter_notes,
            presenter_frame,
            broadcasting: broadcasting_toile,
            following: following_toile,
            presenter_peer: presenter_peer_toile,
        };
        let murmure_relay = ambre_murmure::chrome::MurmureRelay::new();
        *relays_slot.borrow_mut() = Some((trame_relay, toile_relay, murmure_relay));
        let style_state = trame_chrome::StyleState::new();
        *style_slot.borrow_mut() = Some(style_state);
        // Boot directly into a document => start hydrating; the landing route
        // clears it on first poll. `connected` starts false until the first
        // `ConnectionStatus` lands.
        let chrome_status = RwSignal::new(ChromeStatus {
            hydrating: boot_doc.is_some(),
            connected: false,
        });
        *chrome_status_slot.borrow_mut() = Some(chrome_status);

        // The command path (`ARCH-018`): the booted application's handler, and
        // what the shell needs to perform its answers. The keyboard listener
        // reads it back through the slot.
        let toile_dialogs = toile_chrome::ToileDialogs::new();
        let handler = if boot_is_toile {
            AppHandler::Toile(toile_chrome::ToileHandler {
                find: find_state_toile,
                tool: toile_active_tool,
                menu_cmd: toile_menu_cmd,
                dropped: toile_dropped_file,
                toasts,
                dialogs: toile_dialogs,
            })
        } else if boot_is_prose {
            AppHandler::Prose(ambre_prose::chrome::ProseHandler {
                format: prose_signals.format,
            })
        } else {
            AppHandler::Trame(trame_chrome::TrameHandler {
                find: find_state,
                filter_popover,
                filter_values,
                style: style_state,
                anchor: anchor_pos,
            })
        };
        let commands = CommandPath {
            handler,
            current_doc,
            to_logic: to_logic.clone(),
            clipboard_paste,
            toasts,
            palette_open: RwSignal::new(false),
            seal_open: RwSignal::new(false),
        };
        *commands_slot.borrow_mut() = Some(commands.clone());

        // Every modal suspends the frame loop while open (`CHROME-011`).
        // `Dialog` takes and releases this hold; no caller wires the loop.
        provide_context(ambre_chrome::ModalHold::new(move |held| {
            if held {
                raf.hold();
            } else {
                raf.release();
            }
        }));

        // Provide the wiring via context rather than as a prop: `<Router>`'s
        // children must be `Fn` (re-runnable), so the children closure cannot
        // consume a moved `wiring`. `AppRoot` clones it back out of context.
        provide_context(Wiring {
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
            boot_is_toile,
            boot_is_prose,
            boot_is_murmure,
            prose_figures,
            prose_signals,
            formula_ref,
            formula_text,
            formula_action,
            tofu_prompt,
            tofu_confirm,
            invite_role,
            me,
            sel_stats,
            sel_stats_toile,
            present_toile,
            presenter_slide,
            presenter_notes,
            broadcasting_toile,
            following_toile,
            presenter_peer_toile,
            perf_snap,
            zoom,
            ctx_scope,
            filter_active,
            toile_ctx_scope,
            toile_active_tool,
            toile_cursor,
            toile_style_model,
            toile_restyle,
            toile_float_bar,
            toile_quick_style,
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
        });

        view! {
            <Router>
                <AppRoot />
            </Router>
        }
    });
}
