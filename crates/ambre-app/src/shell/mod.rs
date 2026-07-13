// ambre-app/src/shell.rs
// ============================================================================
// Leptos shell: `mount_shell` creates all reactive state inside the mount owner
// (pulling `!Send` handles back out via slots) and mounts the `<Router>`, and
// `AppRoot` is the single root view (persistent chrome + canvas + landing
// overlay). The document binding is driven outside this tree by the per-frame
// route poll in `main`.
// ============================================================================

use super::*;

// AppRoot lives in a sibling submodule; it reaches the launcher imports and
// `Wiring` via `use super::super::*`.
mod app_root;
use app_root::AppRoot;

/// Mount the Leptos shell onto the page body: the persistent AppShell chrome
/// (which owns the `<canvas>`) plus the landing overlay shown on `/`. All
/// reactive state is created here (the mount owner) and published via the slots
/// so the imperative engine wiring in `main` can read it back.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mount_shell(
    to_logic: flume::Sender<LogicCommand>,
    raf: ambre_canvas::RafControl,
    theme_ctl: ambre_canvas::ThemeControl,
    boot_doc: Option<DocumentId>,
    store_slot: Rc<RefCell<Option<CommandStates>>>,
    anchor_slot: Rc<RefCell<Option<RwSignal<Option<(f64, f64)>>>>>,
    toasts_slot: Rc<RefCell<Option<Toasts>>>,
    user_slot: Rc<RefCell<Option<RwSignal<Option<String>>>>>,
    docs_slot: Rc<RefCell<Option<RwSignal<Vec<DocItem>>>>>,
    members_slot: Rc<RefCell<Option<RwSignal<Vec<MemberInfo>>>>>,
    devmode_slot: Rc<RefCell<Option<RwSignal<Option<bool>>>>>,
    current_doc_slot: Rc<RefCell<Option<RwSignal<Option<DocumentId>>>>>,
    filter_popover_slot: Rc<RefCell<Option<RwSignal<Option<trame_chrome::FilterPopoverState>>>>>,
    filter_values_slot: Rc<RefCell<Option<RwSignal<Vec<String>>>>>,
    find_state_slot: Rc<RefCell<Option<RwSignal<Option<trame_chrome::FindReplaceState>>>>>,
    find_result_slot: Rc<RefCell<Option<RwSignal<Option<(usize, usize)>>>>>,
    find_state_toile_slot: Rc<RefCell<Option<RwSignal<bool>>>>,
    find_result_toile_slot: Rc<
        RefCell<Option<RwSignal<Option<ambre_bus::app::toile::FindResults>>>>,
    >,
    boot_is_toile: bool,
    style_slot: Rc<RefCell<Option<trame_chrome::StyleState>>>,
    chrome_status_slot: Rc<RefCell<Option<RwSignal<ChromeStatus>>>>,
    formula_ref: RwSignal<String>,
    formula_text: RwSignal<String>,
    formula_action: RwSignal<Option<FormulaAction>>,
    sel_stats: RwSignal<Option<trame_chrome::SelStats>>,
    perf_snap: RwSignal<ambre_canvas::PerfSnapshot>,
    zoom: RwSignal<f64>,
    ctx_scope: RwSignal<trame_chrome::CtxScope>,
    filter_active: RwSignal<bool>,
    sel_bounds: RwSignal<Option<(u32, u32, u32, u32)>>,
    toile_ctx_scope: RwSignal<ambre_toile::ui::menu::CtxScope>,
    toile_menu_cmd: RwSignal<Option<ambre_chrome::commands::CommandId>>,
    sheets: RwSignal<Vec<(String, String)>>,
    active_sheet: RwSignal<String>,
) {
    leptos::mount::mount_to_body(move || {
        // The store stays here (in the mount owner, NOT in the view): it is `Rc`
        // and the per-frame relay drives it. The shell sees only its toggle
        // signals (Send), so the `<Router>` subtree stays Send.
        let store = CommandStates::new(trame_chrome::COMMANDS);
        // Seed app-specific initial command states (Undo/Redo start disabled).
        trame_chrome::init_command_states(&store);
        let toggles = trame_chrome::Toggles::from_store(&store);
        *store_slot.borrow_mut() = Some(store);
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
        let dev_mode = RwSignal::new(None::<bool>);
        *devmode_slot.borrow_mut() = Some(dev_mode);
        let current_doc = RwSignal::new(boot_doc);
        *current_doc_slot.borrow_mut() = Some(current_doc);
        let filter_popover = RwSignal::new(None::<trame_chrome::FilterPopoverState>);
        *filter_popover_slot.borrow_mut() = Some(filter_popover);
        let filter_values = RwSignal::new(Vec::<String>::new());
        *filter_values_slot.borrow_mut() = Some(filter_values);
        let find_state = RwSignal::new(None::<trame_chrome::FindReplaceState>);
        *find_state_slot.borrow_mut() = Some(find_state);
        let find_result = RwSignal::new(None::<(usize, usize)>);
        *find_result_slot.borrow_mut() = Some(find_result);
        let find_state_toile = RwSignal::new(false);
        *find_state_toile_slot.borrow_mut() = Some(find_state_toile);
        let find_result_toile = RwSignal::new(None::<ambre_bus::app::toile::FindResults>);
        *find_result_toile_slot.borrow_mut() = Some(find_result_toile);
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
        // Multi-column sort dialog state (`-ui.md` sec.5.C): opened by `on_command`
        // and rendered in `AppRoot`, both inside this owner, so it needs no slot.
        let sort_dialog = RwSignal::new(None::<trame_chrome::SortDialogState>);

        // Provide the wiring via context rather than as a prop: `<Router>`'s
        // children must be `Fn` (re-runnable), so the children closure cannot
        // consume a moved `wiring`. `AppRoot` clones it back out of context.
        provide_context(Wiring {
            to_logic,
            raf,
            theme_ctl,
            toggles,
            style_state,
            anchor_pos,
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
            formula_ref,
            formula_text,
            formula_action,
            sel_stats,
            perf_snap,
            zoom,
            ctx_scope,
            filter_active,
            sel_bounds,
            toile_ctx_scope,
            toile_menu_cmd,
            sort_dialog,
            chrome_status,
            sheets,
            active_sheet,
        });

        view! {
            <Router>
                <AppRoot />
            </Router>
        }
    });
}
