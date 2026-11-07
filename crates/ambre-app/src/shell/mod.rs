// ambre-app/src/shell.rs
// ============================================================================
// Leptos shell. `mount_shell` creates all reactive state inside the mount owner,
// pulling `!Send` handles back out via slots, and mounts the `<Router>`.
// `AppRoot` is the single root view: persistent chrome, canvas and landing
// overlay. The document binding is driven outside this tree by the per-frame
// route poll in `main`.
// ============================================================================

use super::*;
use ambre_core::id::Id;

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

/// The slots the mount fills with the `!Send` handles it creates, for the
/// launcher to read back once the mount returns.
pub(crate) struct ShellSlots {
    pub(crate) store: Slot<CommandStates>,
    pub(crate) toile_store: Slot<CommandStates>,
    pub(crate) anchor: Slot<RwSignal<Option<ambre_core::Point<ambre_core::Px<ambre_core::View>>>>>,
    pub(crate) toasts: Slot<Toasts>,
    pub(crate) user: Slot<RwSignal<Option<ambre_core::Validated<ambre_core::DisplayName>>>>,
    pub(crate) docs: Slot<RwSignal<Vec<DocItem>>>,
    pub(crate) members: Slot<RwSignal<Vec<MemberInfo>>>,
    pub(crate) devmode: Slot<RwSignal<Option<landing::SignInMode>>>,
    pub(crate) current_doc: Slot<RwSignal<Option<Id<Document>>>>,
    pub(crate) relays: Slot<Relays>,
    pub(crate) style: Slot<trame_chrome::StyleState>,
    pub(crate) chrome_status: Slot<RwSignal<ChromeStatus>>,
    pub(crate) commands: Slot<CommandPath>,
}

/// The wiring of the shell: each channel, slot and signal the launcher
/// builds and the mount hands to the widget that reads it. The signals the
/// shell shares with a view travel as that view's own set.
pub(crate) struct ShellWiring {
    pub(crate) to_logic: crate::outbox::ShellOutbox,
    pub(crate) clipboard_paste: ambre_bus::conduit::Sender<ClipboardPaste>,
    pub(crate) raf: ambre_canvas::RafControl,
    pub(crate) theme_ctl: ambre_canvas::ThemeControl,
    pub(crate) boot_doc: Option<Id<Document>>,
    pub(crate) booted: ambre_bus::DocType,
    pub(crate) slots: ShellSlots,
    pub(crate) prose_figures: RwSignal<Option<ambre_prose::ui::StatusFigures>>,
    pub(crate) prose_signals: crate::host::ProseSignals,
    pub(crate) tofu_prompt: RwSignal<Option<TofuPrompt>>,
    pub(crate) tofu_confirm: RwSignal<Option<TofuPrompt>>,
    pub(crate) opened_file: RwSignal<Option<platform::OpenedFile>>,
    pub(crate) invite_role: RwSignal<Role>,
    pub(crate) me: RwSignal<Option<Id<User>>>,
    pub(crate) presenter_slide: RwSignal<SlidePosition>,
    pub(crate) presenter_notes: RwSignal<String>,
    pub(crate) presenter_frame: RwSignal<Option<Id<Frame>>>,
    pub(crate) broadcasting_toile: RwSignal<Broadcast>,
    pub(crate) following_toile: RwSignal<Option<Id<User>>>,
    pub(crate) presenter_peer_toile: RwSignal<Option<Id<User>>>,
    pub(crate) perf_snap: RwSignal<ambre_canvas::PerfSnapshot>,
    pub(crate) sheets: RwSignal<Vec<SheetTab>>,
    /// The signals the shell shares with a sheet view.
    pub(crate) trame: ambre_trame::ui::TrameSignals,
    /// The signals the shell shares with a board view.
    pub(crate) toile: ambre_toile::ui::ToileSignals,
}

/// Mount the Leptos shell onto the page body: the persistent AppShell chrome
/// (which owns the `<canvas>`) plus the landing overlay shown on `/`. All
/// reactive state is created here (the mount owner) and published via the slots
/// so the imperative engine wiring in `main` can read it back.
pub(crate) fn mount_shell(wiring: ShellWiring) {
    let ShellWiring {
        to_logic,
        clipboard_paste,
        raf,
        theme_ctl,
        boot_doc,
        booted,
        slots,
        prose_figures,
        prose_signals,
        tofu_prompt,
        tofu_confirm,
        opened_file,
        invite_role,
        me,
        presenter_slide,
        presenter_notes,
        presenter_frame,
        broadcasting_toile,
        following_toile,
        presenter_peer_toile,
        perf_snap,
        sheets,
        trame,
        toile,
    } = wiring;
    let ShellSlots {
        store: store_slot,
        toile_store: toile_store_slot,
        anchor: anchor_slot,
        toasts: toasts_slot,
        user: user_slot,
        docs: docs_slot,
        members: members_slot,
        devmode: devmode_slot,
        current_doc: current_doc_slot,
        relays: relays_slot,
        style: style_slot,
        chrome_status: chrome_status_slot,
        commands: commands_slot,
    } = slots;
    let ambre_trame::ui::TrameSignals {
        formula_ref,
        formula_text,
        formula_action,
        sel_stats,
        ctx_scope,
        zoom,
        active_sheet,
        filter_active,
        find_result,
    } = trame;
    let ambre_toile::ui::ToileSignals {
        ctx_scope: toile_ctx_scope,
        menu_cmd: toile_menu_cmd,
        active_tool: toile_active_tool,
        cursor: toile_cursor,
        zoom: board_zoom,
        sel_stats: sel_stats_toile,
        style_model: toile_style_model,
        restyle: toile_restyle,
        float_bar: toile_float_bar,
        quick_style: toile_quick_style,
        comments: toile_comments,
        deck: toile_deck,
        dropped_file: toile_dropped_file,
        placed_embed: toile_placed_embed,
        set_link: toile_set_link,
        followed_link: toile_followed_link,
        present_mode: present_toile,
        find_result: find_result_toile,
    } = toile;
    leptos::mount::mount_to_body(move || {
        // The store stays here (in the mount owner, NOT in the view): it is `Rc`
        // and the switchboard drives it. The shell sees only its toggle
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
        let anchor_pos = RwSignal::new(None::<ambre_core::Point<ambre_core::Px<ambre_core::View>>>);
        *anchor_slot.borrow_mut() = Some(anchor_pos);
        let toasts = Toasts::new();
        *toasts_slot.borrow_mut() = Some(toasts);
        let username = RwSignal::new(None::<ambre_core::Validated<ambre_core::DisplayName>>);
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
        // their intents into its own commands (`ARCH-018`). The switchboard
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
            connected: ambre_core::LinkState::Disconnected,
        });
        *chrome_status_slot.borrow_mut() = Some(chrome_status);

        // The command path (`ARCH-018`): the booted application's handler, and
        // what the shell needs to perform its answers. The keyboard listener
        // reads it back through the slot.
        let toile_dialogs = toile_chrome::ToileDialogs::new();
        let handler = if booted == ambre_bus::DocType::Toile {
            AppHandler::Toile(toile_chrome::ToileHandler {
                find: find_state_toile,
                tool: toile_active_tool,
                menu_cmd: toile_menu_cmd,
                dropped: toile_dropped_file,
                toasts,
                dialogs: toile_dialogs,
            })
        } else if booted == ambre_bus::DocType::Prose {
            AppHandler::Prose(ambre_prose::chrome::ProseHandler {
                format: prose_signals.format,
            })
        } else if booted == ambre_bus::DocType::Murmure {
            AppHandler::Murmure(ambre_murmure::chrome::MurmureHandler {
                relay: murmure_relay,
                send: commands::murmure_send(to_logic.clone()),
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
            palette_open: RwSignal::new(ambre_chrome::Openness::Closed),
            seal_open: RwSignal::new(ambre_chrome::Openness::Closed),
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
            booted,
            prose_figures,
            prose_signals,
            formula_ref,
            formula_text,
            formula_action,
            tofu_prompt,
            tofu_confirm,
            opened_file,
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
