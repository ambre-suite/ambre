// ambre-trame/src/chrome.rs
// ============================================================================
// Trame-SPECIFIC chrome (ambre-spec-architecture.md, 1: per-app chrome lives in
// the app crate, NOT the launcher). Built on the app-agnostic `ambre-chrome`
// toolkit, this module owns everything the launcher used to hard-code about
// Trame:
//
//   - the format `CommandId`s (Bold/Italic) and their keybindings;
//   - the CommandId -> `AppCommand` mapping (UI affordance -> IPC payload);
//   - the inbound `UiEvent` -> command-store mirror (authoritative toggle state);
//   - the toolbar / floating-selection-toolbar markup and the status label.
//
// The launcher (`ambre-app`) stays app-agnostic: it owns the generic AppShell,
// the `CommandStates` store, and the per-frame relay, and reaches into this
// module for the Trame pieces (`COMMANDS`, `bind_keys`, `command_to_app`,
// `mirror_event`, the `<FormatBar>` / `<SelectionBar>` components, ...).
//
// Wasm-only, like the rest of the chrome stack (leptos CSR + DOM).
// ============================================================================

use ambre_bus::{
    AppCommand, AppEvent, BorderEdit, BorderPreset, BorderStyle, Borders, CellFormat, CellStyle,
    DocType, FilterKind, FilterOp, HAlign, StylePatch, TrameCommand, TrameEvent, UiEvent, VAlign,
};
use ambre_chrome::commands::{CommandId, KeyBinding, Keymap};
use ambre_chrome::{CommandStates, MenuEntry, Toasts};
use leptos::html;
use leptos::prelude::*;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;

use crate::render::{CommitDirection, FormulaAction};

/// The Trame "Bold" command. A UI affordance id (ambre-commands); the launcher
/// intercepts it into a `SetCellStyle` patch toggled against the tracked style.
pub const BOLD: CommandId = CommandId("trame.format.bold");
/// The Trame "Italic" command (see [`BOLD`]).
pub const ITALIC: CommandId = CommandId("trame.format.italic");
/// The Trame "Undo" command (Loro `UndoManager`, document-session scope).
pub const UNDO: CommandId = CommandId("trame.history.undo");
/// The Trame "Redo" command (see [`UNDO`]).
pub const REDO: CommandId = CommandId("trame.history.redo");

// Structural / merge commands (grid context menu, `-ui.md` sec.5.C). Each is a
// parameterless intent: the Logic thread resolves its geometry against the
// active cell / selection it tracks (via `SetActiveCell`).
/// Insert a row above the active cell.
pub const INSERT_ROW_ABOVE: CommandId = CommandId("trame.row.insert_above");
/// Insert a row below the active cell.
pub const INSERT_ROW_BELOW: CommandId = CommandId("trame.row.insert_below");
/// Insert a column left of the active cell.
pub const INSERT_COL_LEFT: CommandId = CommandId("trame.col.insert_left");
/// Insert a column right of the active cell.
pub const INSERT_COL_RIGHT: CommandId = CommandId("trame.col.insert_right");
/// Delete the rows spanned by the selection.
pub const DELETE_ROWS: CommandId = CommandId("trame.row.delete");
/// Delete the columns spanned by the selection.
pub const DELETE_COLS: CommandId = CommandId("trame.col.delete");
/// Merge the selection into a single cell.
pub const MERGE: CommandId = CommandId("trame.merge");
/// Unmerge the merge containing the active cell.
pub const UNMERGE: CommandId = CommandId("trame.unmerge");
/// Sort the selection ascending on its first column (single-column shared sort).
pub const SORT_RANGE_ASC: CommandId = CommandId("trame.sort.range_asc");
/// Sort the selection descending on its first column.
pub const SORT_RANGE_DESC: CommandId = CommandId("trame.sort.range_desc");
/// Sort the whole sheet ascending by the active cell's column.
pub const SORT_SHEET_ASC: CommandId = CommandId("trame.sort.sheet_asc");
/// Sort the whole sheet descending by the active cell's column.
pub const SORT_SHEET_DESC: CommandId = CommandId("trame.sort.sheet_desc");
/// Open the shared filter popover for the active cell's column (sec.5.D). A
/// UI-only intent: it has no `AppCommand` mapping; the launcher intercepts it to
/// open the popover and request the column's distinct values.
pub const FILTER_COLUMN: CommandId = CommandId("trame.filter.column");
/// Clear the active sheet's shared filter (sec.5.D).
pub const CLEAR_FILTER: CommandId = CommandId("trame.filter.clear");

/// Every command Trame contributes to the registry. The launcher builds the
/// `CommandStates` store and the keymap from this list, naming no command itself.
pub const COMMANDS: &[CommandId] = &[
    BOLD,
    ITALIC,
    UNDO,
    REDO,
    INSERT_ROW_ABOVE,
    INSERT_ROW_BELOW,
    INSERT_COL_LEFT,
    INSERT_COL_RIGHT,
    DELETE_ROWS,
    DELETE_COLS,
    MERGE,
    UNMERGE,
    SORT_RANGE_ASC,
    SORT_RANGE_DESC,
    SORT_SHEET_ASC,
    SORT_SHEET_DESC,
    FILTER_COLUMN,
    CLEAR_FILTER,
];

/// Bind Trame's command chords into the launcher's keymap.
pub fn bind_keys(keymap: &mut Keymap) {
    keymap.bind(KeyBinding::ctrl('b'), BOLD);
    keymap.bind(KeyBinding::ctrl('i'), ITALIC);
    keymap.bind(KeyBinding::ctrl('z'), UNDO);
    // Ctrl+Shift+Z and Ctrl+Y both redo (Excel/Sheets convention).
    keymap.bind(KeyBinding::ctrl_shift('z'), REDO);
    keymap.bind(KeyBinding::ctrl('y'), REDO);
}

/// Translate a Trame `CommandId` into the `AppCommand` IPC payload the Logic
/// thread expects. `None` for a command Trame does not own (the launcher logs it).
pub fn command_to_app(id: CommandId) -> Option<AppCommand> {
    let cmd = if id == UNDO {
        TrameCommand::Undo
    } else if id == REDO {
        TrameCommand::Redo
    } else if id == INSERT_ROW_ABOVE {
        TrameCommand::InsertRowAbove
    } else if id == INSERT_ROW_BELOW {
        TrameCommand::InsertRowBelow
    } else if id == INSERT_COL_LEFT {
        TrameCommand::InsertColLeft
    } else if id == INSERT_COL_RIGHT {
        TrameCommand::InsertColRight
    } else if id == DELETE_ROWS {
        TrameCommand::DeleteSelectedRows
    } else if id == DELETE_COLS {
        TrameCommand::DeleteSelectedCols
    } else if id == MERGE {
        TrameCommand::MergeSelection
    } else if id == UNMERGE {
        TrameCommand::UnmergeActive
    } else if id == SORT_RANGE_ASC {
        TrameCommand::SortSelectionAsc
    } else if id == SORT_RANGE_DESC {
        TrameCommand::SortSelectionDesc
    } else if id == SORT_SHEET_ASC {
        TrameCommand::SortSheetAsc
    } else if id == SORT_SHEET_DESC {
        TrameCommand::SortSheetDesc
    } else if id == CLEAR_FILTER {
        TrameCommand::ClearActiveFilter
    } else {
        // BOLD / ITALIC (style toggles) and FILTER_COLUMN are UI-only intents the
        // launcher intercepts: BOLD/ITALIC become a `SetCellStyle` patch toggled
        // against the tracked style state, FILTER_COLUMN opens the popover. Neither
        // has a direct AppCommand here.
        return None;
    };
    Some(AppCommand::Trame(cmd))
}

/// Mirror authoritative toggle state from an inbound `UiEvent` into the command
/// store, so the chrome's `aria-pressed` buttons reflect the Logic thread. A
/// no-op for any event that is not a Trame format-state notification; the
/// launcher calls this for every event it relays, naming no Trame variant.
pub fn mirror_event(store: &CommandStates, ev: &UiEvent) {
    if let UiEvent::Viewport { event: AppEvent::Trame(te), .. } = ev {
        match te {
            // Loro `UndoManager` stack availability -> the Undo/Redo affordances'
            // enabled state. The Logic thread is the single source of truth.
            TrameEvent::UndoState { can_undo, can_redo } => {
                store.set_enabled(UNDO, *can_undo);
                store.set_enabled(REDO, *can_redo);
            }
            _ => {}
        }
    }
}

/// Seed the command store for a freshly mounted Trame document: Undo and Redo
/// start disabled (an untouched document has an empty history) until the first
/// [`TrameEvent::UndoState`] arrives. Every other command stays enabled by
/// default. Called by the launcher right after it builds the store.
pub fn init_command_states(store: &CommandStates) {
    store.set_enabled(UNDO, false);
    store.set_enabled(REDO, false);
}

/// Surface a Trame `UiEvent` to the user as a transient toast, when it carries a
/// message the user should see: a rejected merge/edit, a sheet-lifecycle error,
/// or the completion of a shared sort/filter. A no-op for every other event; the
/// launcher calls this for each event it relays, naming no Trame variant itself.
pub fn notify_event(toasts: &Toasts, ev: &UiEvent) {
    let UiEvent::Viewport { event: AppEvent::Trame(te), .. } = ev else { return };
    match te {
        // Rejections / validation failures: no CRDT mutation occurred, the user
        // needs to know why their action did nothing.
        TrameEvent::MergeRejected { reason } => {
            toasts.warning(format!("Merge rejected: {reason}"));
        }
        TrameEvent::ValidationError { message, .. } => {
            toasts.warning(format!("Invalid value: {message}"));
        }
        TrameEvent::Error { message } => toasts.warning(message.clone()),
        // A converged concurrent edit masked a covered cell on an active merge.
        // Purely informational: the value reappears on unmerge.
        TrameEvent::MergeConflictDetected { covered_row, covered_col, .. } => {
            toasts.info(format!(
                "A merge now covers a non-empty cell ({covered_row}, {covered_col}); its value is hidden until unmerge"
            ));
        }
        // Shared sort/filter completion: confirm the action took effect (the data
        // refresh rides on the accompanying `DataReset`).
        TrameEvent::FilterApplied { hidden_row_count, .. } => {
            toasts.info(format!("Filter applied ({hidden_row_count} rows hidden)"));
        }
        TrameEvent::FilterCleared => toasts.info("Filter cleared"),
        TrameEvent::SortComplete => toasts.info("Range sorted"),
        _ => {}
    }
}

/// The right-click context-menu entries Trame contributes.
pub fn context_entries() -> Vec<MenuEntry> {
    vec![
        MenuEntry::new(BOLD, "Bold"),
        MenuEntry::new(ITALIC, "Italic"),
        MenuEntry::new(UNDO, "Undo"),
        MenuEntry::new(REDO, "Redo"),
        // Structural / merge ops (sec.5.C). Always listed; the Logic thread
        // guards them (Merge on a single cell is a silent no-op, Merge over a
        // non-empty covered cell replies MergeRejected -> toast, Unmerge outside
        // a merge is a no-op).
        MenuEntry::new(INSERT_ROW_ABOVE, "Insert row above"),
        MenuEntry::new(INSERT_ROW_BELOW, "Insert row below"),
        MenuEntry::new(INSERT_COL_LEFT, "Insert column left"),
        MenuEntry::new(INSERT_COL_RIGHT, "Insert column right"),
        MenuEntry::new(DELETE_ROWS, "Delete rows"),
        MenuEntry::new(DELETE_COLS, "Delete columns"),
        MenuEntry::new(MERGE, "Merge cells"),
        MenuEntry::new(UNMERGE, "Unmerge cells"),
        // Single-column shared sort over the selection (sec.5.C). Reference
        // column = the selection's first column; the Logic thread no-ops an
        // empty/single-row range.
        MenuEntry::new(SORT_RANGE_ASC, "Sort range ascending"),
        MenuEntry::new(SORT_RANGE_DESC, "Sort range descending"),
        // Whole-sheet sort by the active column (whole-column selection, sec.5.C):
        // sorts the full used range so rows stay aligned across all columns.
        MenuEntry::new(SORT_SHEET_ASC, "Sort sheet by column (asc)"),
        MenuEntry::new(SORT_SHEET_DESC, "Sort sheet by column (desc)"),
        // Shared filter on the active cell's column (sec.5.D). "Filter by column"
        // opens the popover (values / condition); "Clear filter" drops the active
        // sheet's filter. The Logic thread no-ops a clear when nothing is filtered.
        MenuEntry::new(FILTER_COLUMN, "Filter by column"),
        MenuEntry::new(CLEAR_FILTER, "Clear filter"),
    ]
}

/// The status-bar label for a Trame document.
pub fn status_label() -> &'static str {
    "Trame"
}

/// The document type this app creates from the landing "new document" button.
pub const DOC_TYPE: DocType = DocType::Trame;

/// The label of the landing "new document" button for this app.
pub fn new_doc_label() -> &'static str {
    "New Trame"
}

/// The Send toggle signals the Trame chrome components subscribe to, resolved
/// once from the launcher's `CommandStates` store. Plain `RwSignal`s, so this is
/// `Copy` and `Send` and may live inside the `<Router>` subtree (unlike the
/// `!Send` store itself, which the launcher keeps outside the reactive tree).
#[derive(Clone, Copy)]
pub struct Toggles {
    /// Undo/Redo availability (Loro `UndoManager` stacks), driving the matching
    /// buttons' `disabled` state. Fed by [`TrameEvent::UndoState`] via
    /// [`mirror_event`].
    pub undo_enabled: RwSignal<bool>,
    pub redo_enabled: RwSignal<bool>,
}

impl Toggles {
    /// Resolve the history-enabled signals from a store built over [`COMMANDS`].
    /// Bold/italic now live in [`StyleState`] (the single style channel), not the
    /// command store.
    pub fn from_store(store: &CommandStates) -> Self {
        Self {
            undo_enabled: store.enabled_signal(UNDO).expect("undo registered"),
            redo_enabled: store.enabled_signal(REDO).expect("redo registered"),
        }
    }
}

// -----------------------------------------------------------------------------
// Rich formatting state (the formatting toolbar, `-ui.md` sec.5bis)
// -----------------------------------------------------------------------------

/// The active cell's resolved rich style, mirrored into `Copy` signals so the
/// formatting toolbar's selects/swatches/toggles reflect the selection. Fed by
/// [`TrameEvent::StyleApplied`] via [`apply_style_event`] -- this is the single
/// style channel, so bold/italic live here too. Lives in the `<Router>` subtree.
#[derive(Clone, Copy)]
pub struct StyleState {
    pub bold: RwSignal<bool>,
    pub italic: RwSignal<bool>,
    pub underline: RwSignal<bool>,
    pub strikethrough: RwSignal<bool>,
    pub wrap: RwSignal<bool>,
    pub h_align: RwSignal<HAlign>,
    pub v_align: RwSignal<VAlign>,
    pub format: RwSignal<CellFormat>,
    /// Borders popover picker state: the stroke style and colour the next preset
    /// click applies. Seeded (best-effort) from the active cell's first present
    /// edge so reopening the popover shows the cell's current pen.
    pub border_line: RwSignal<BorderStyle>,
    pub border_color: RwSignal<Option<u32>>,
    pub fg: RwSignal<Option<u32>>,
    pub bg: RwSignal<Option<u32>>,
    pub font: RwSignal<Option<String>>,
    pub size: RwSignal<Option<u16>>,
    /// Whether the active cell is part of a merge, driving the toolbar's
    /// merge/unmerge toggle pressed state.
    pub merged: RwSignal<bool>,
}

impl Default for StyleState {
    fn default() -> Self {
        Self::new()
    }
}

impl StyleState {
    /// Build a fresh state with every control at its default value.
    pub fn new() -> Self {
        Self {
            bold: RwSignal::new(false),
            italic: RwSignal::new(false),
            underline: RwSignal::new(false),
            strikethrough: RwSignal::new(false),
            wrap: RwSignal::new(false),
            h_align: RwSignal::new(HAlign::Auto),
            v_align: RwSignal::new(VAlign::Middle),
            format: RwSignal::new(CellFormat::General),
            border_line: RwSignal::new(BorderStyle::Solid),
            border_color: RwSignal::new(None),
            fg: RwSignal::new(None),
            bg: RwSignal::new(None),
            font: RwSignal::new(None),
            size: RwSignal::new(None),
            merged: RwSignal::new(false),
        }
    }
}

/// Mirror a [`TrameEvent::StyleApplied`] into the toolbar's [`StyleState`]. A
/// no-op for any other event; the launcher calls it for every event it relays.
pub fn apply_style_event(s: &StyleState, ev: &UiEvent) {
    if let UiEvent::Viewport {
        event: AppEvent::Trame(TrameEvent::StyleApplied { style, bold, italic, merged }),
        ..
    } = ev
    {
        s.bold.set(*bold);
        s.italic.set(*italic);
        s.merged.set(*merged);
        s.underline.set(style.underline);
        s.strikethrough.set(style.strikethrough);
        s.wrap.set(style.wrap);
        s.h_align.set(style.h_align);
        s.v_align.set(style.v_align);
        s.format.set(style.format.clone());
        // Seed the popover pen from the first present edge (best effort: the
        // per-edge model has no single style/colour, but reopening the popover
        // on a bordered cell should show a sensible current pen).
        let b = &style.borders;
        if let Some(edge) = b.top.or(b.right).or(b.bottom).or(b.left) {
            s.border_line.set(edge.style);
            s.border_color.set(edge.color);
        }
        s.fg.set(style.fg);
        s.bg.set(style.bg);
        s.font.set(style.font.clone());
        s.size.set(style.size);
    }
}

/// The number-format families offered by the toolbar `<select>`, paired with
/// their labels. The option index maps back to the [`CellFormat`] on change.
const NUMBER_FORMATS: &[(CellFormat, &str)] = &[
    (CellFormat::General, "General"),
    (CellFormat::Number, "Number"),
    (CellFormat::Currency, "Currency"),
    (CellFormat::Percent, "Percent"),
    (CellFormat::Scientific, "Scientific"),
    (CellFormat::Fraction, "Fraction"),
    (CellFormat::Date, "Date"),
    (CellFormat::Time, "Time"),
    (CellFormat::Text, "Text"),
];

/// The border presets offered by the popover, each with its Bootstrap icon and
/// tooltip. Clicking one applies the popover's current pen (line style + colour)
/// to the named edges; `None` clears every edge.
const BORDER_BUTTONS: &[(BorderPreset, icondata::Icon, &str)] = &[
    (BorderPreset::All, icondata::BsBorderAll, "All borders"),
    (BorderPreset::Inner, icondata::BsBorderInner, "Inner borders"),
    (BorderPreset::InnerHorizontal, icondata::BsBorderMiddle, "Inner horizontal"),
    (BorderPreset::InnerVertical, icondata::BsBorderCenter, "Inner vertical"),
    (BorderPreset::Outline, icondata::BsBorderOuter, "Outline"),
    (BorderPreset::Top, icondata::BsBorderTop, "Top border"),
    (BorderPreset::Bottom, icondata::BsBorderBottom, "Bottom border"),
    (BorderPreset::Left, icondata::BsBorderLeft, "Left border"),
    (BorderPreset::Right, icondata::BsBorderRight, "Right border"),
    (BorderPreset::None, icondata::BsBorder, "Clear borders"),
];

/// The border stroke styles offered by the toolbar `<select>` (line pattern +
/// weight), paired with their labels. The option index maps back on change.
const BORDER_STYLES: &[(BorderStyle, &str)] = &[
    (BorderStyle::Solid, "Solid"),
    (BorderStyle::Dashed, "Dashed"),
    (BorderStyle::Dotted, "Dotted"),
    (BorderStyle::Medium, "Medium"),
    (BorderStyle::Thick, "Thick"),
    (BorderStyle::Double, "Double"),
];

/// The font families offered by the toolbar `<select>` (the renderer falls back
/// to its default for any family it does not have loaded).
const FONTS: &[&str] = &["Default", "Sans", "Serif", "Monospace"];

/// The font sizes offered by the toolbar `<select>` (points).
const SIZES: &[u16] = &[8, 9, 10, 11, 12, 14, 16, 18, 24, 36];

/// The grid's default body size (matches `render::BODY_SIZE`), shown in the size
/// picker when a cell has no explicit size.
const DEFAULT_SIZE: u16 = 12;

/// Parse an `<input type="color">` value (`"#rrggbb"`) into a packed `0xRRGGBB`.
fn parse_hex_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    u32::from_str_radix(hex, 16).ok()
}

/// Format a packed `0xRRGGBB` colour into an `<input type="color">` value, using
/// `default` when the cell has no explicit colour (`<input type=color>` cannot be
/// empty). The default is the themed text / fill colour, so an "Automatic" /
/// "Transparent" cell shows its real on-screen colour rather than a misleading
/// black/white -- the picker tracks the active light/dark theme.
fn color_hex(c: Option<u32>, default: u32) -> String {
    format!("#{:06x}", c.unwrap_or(default))
}

/// The themed default text colour (matches the canvas `cell_text` token), so the
/// fg picker, when on "Automatic", shows the colour the glyphs actually render in.
fn theme_fg_default(theme: ambre_canvas::Theme) -> u32 {
    match theme {
        ambre_canvas::Theme::Dark => 0xe8e8e8,
        ambre_canvas::Theme::Light => 0x202020,
    }
}

/// The themed default fill colour (matches the canvas `content_bg` token), so the
/// bg picker, when on "Transparent", shows the sheet background.
fn theme_bg_default(theme: ambre_canvas::Theme) -> u32 {
    match theme {
        ambre_canvas::Theme::Dark => 0x1c1c1f,
        ambre_canvas::Theme::Light => 0xffffff,
    }
}

/// The composed borders control (`-ui.md` sec.5bis): a single toolbar button
/// that toggles a popover holding the "pen" (stroke style + colour) and a grid
/// of preset icon buttons. A preset click applies the current pen to its edges
/// via [`StylePatch::border`] (a [`BorderEdit`]); the per-edge model lets a cell
/// carry a different style/colour on each side, accumulated across clicks.
#[component]
fn BordersPopover(style: StyleState, on_style: Callback<StylePatch>) -> impl IntoView {
    let open = RwSignal::new(false);
    // The wrapper node, so a global pointer-down can tell clicks inside the
    // popover apart from clicks elsewhere (another cell, the canvas, a sibling).
    let root_ref = NodeRef::<html::Span>::new();
    // Close on any pointer-down outside the control: selecting a cell hits the
    // canvas, which is outside this node, so the popover dismisses.
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if !open.get_untracked() {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || handle.remove());
    // Emit a border edit for `preset` using the popover's current pen.
    let apply = move |preset: BorderPreset| {
        on_style.run(StylePatch {
            border: Some(BorderEdit {
                preset,
                style: style.border_line.get_untracked(),
                color: style.border_color.get_untracked(),
            }),
            ..Default::default()
        });
    };
    view! {
        <span class="ambre-borders" node_ref=root_ref>
            <button
                title="Borders"
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=icondata::BsBorderAll />
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-borders-popover">
                    // Pen: stroke style + colour applied by the preset buttons.
                    <div class="row">
                        <select
                            title="Line style"
                            prop:value=move || {
                                BORDER_STYLES
                                    .iter()
                                    .position(|(bs, _)| *bs == style.border_line.get())
                                    .unwrap_or(0)
                                    .to_string()
                            }
                            on:change=move |ev| {
                                let i = event_target_value(&ev).parse::<usize>().unwrap_or(0);
                                style.border_line.set(BORDER_STYLES[i.min(BORDER_STYLES.len() - 1)].0);
                            }
                        >
                            {BORDER_STYLES
                                .iter()
                                .enumerate()
                                .map(|(i, (_, label))| {
                                    view! { <option value=i.to_string()>{label.to_string()}</option> }
                                })
                                .collect_view()}
                        </select>
                        <input
                            type="color"
                            title="Line colour"
                            prop:value=move || color_hex(style.border_color.get(), 0x606060)
                            on:input=move |ev| {
                                style.border_color.set(parse_hex_color(&event_target_value(&ev)));
                            }
                        />
                    </div>
                    // Preset grid: each applies the pen to its edges.
                    <div class="grid">
                        {BORDER_BUTTONS
                            .iter()
                            .map(|&(preset, icon, label)| {
                                view! {
                                    <button
                                        title=label
                                        on:click=move |_| apply(preset)
                                    >
                                        <Icon icon=icon />
                                    </button>
                                }
                            })
                            .collect_view()}
                    </div>
                </div>
            </Show>
        </span>
    }
}

/// Curated swatch palette for the colour picker popover (packed `0xRRGGBB`).
/// A small office-style set: greys, then a saturated row and a muted row.
const PALETTE: &[u32] = &[
    0x000000, 0x404040, 0x808080, 0xb0b0b0, 0xd9d9d9, 0xffffff, 0xc00000, 0xff0000,
    0xffc000, 0xffff00, 0x92d050, 0x00b050, 0x00b0f0, 0x0070c0, 0x002060, 0x7030a0,
];

/// A composed colour control (text or fill): a single toolbar button toggling a
/// popover that combines a reset entry ("Automatic" / "No fill"), a swatch
/// palette, and an embedded native `<input type="color">` for arbitrary colours.
/// `on_pick` receives `Some(rgb)` for a chosen colour or `None` for the reset,
/// which the caller maps onto `StylePatch { fg/bg: Some(_) }` (sec.5bis).
#[component]
fn ColorPopover(
    title: &'static str,
    icon: icondata::Icon,
    current: RwSignal<Option<u32>>,
    default: Signal<u32>,
    reset_label: &'static str,
    on_pick: Callback<Option<u32>>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    // The wrapper node, so a global mousedown can tell clicks inside the popover
    // apart from clicks elsewhere (another cell, the canvas, a sibling control).
    let root_ref = NodeRef::<html::Span>::new();
    // Close on any pointer-down outside the control: selecting another cell hits
    // the canvas, which is outside this node, so the popover dismisses.
    let handle = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if !open.get_untracked() {
            return;
        }
        let inside = root_ref
            .get_untracked()
            .zip(ev.target())
            .and_then(|(root, target)| target.dyn_into::<web_sys::Node>().ok().map(|n| (root, n)))
            .is_some_and(|(root, node)| root.contains(Some(&node)));
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || handle.remove());
    view! {
        <span class="ambre-color" node_ref=root_ref>
            <button
                title=title
                aria-haspopup="true"
                aria-expanded=move || open.get().then_some("true")
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon icon=icon />
                // Thin strip tinted to the current colour (themed default when unset).
                <span
                    class="swatch-strip"
                    style:background-color=move || color_hex(current.get(), default.get())
                ></span>
            </button>
            <Show when=move || open.get() fallback=|| ()>
                <div class="ambre-color-popover">
                    // Reset entry: "Automatic" (text) / "No fill (transparent)" (fill).
                    <button
                        class="reset"
                        on:click=move |_| {
                            on_pick.run(None);
                            open.set(false);
                        }
                    >
                        {reset_label}
                    </button>
                    // Swatch palette.
                    <div class="swatches">
                        {PALETTE
                            .iter()
                            .map(|&rgb| {
                                view! {
                                    <button
                                        title=format!("#{:06x}", rgb)
                                        style:background-color=format!("#{:06x}", rgb)
                                        on:click=move |_| {
                                            on_pick.run(Some(rgb));
                                            open.set(false);
                                        }
                                    ></button>
                                }
                            })
                            .collect_view()}
                    </div>
                    // Custom colour: the standard native widget, kept for arbitrary picks.
                    <label class="custom">
                        <input
                            type="color"
                            prop:value=move || color_hex(current.get(), default.get())
                            on:input=move |ev| {
                                on_pick.run(parse_hex_color(&event_target_value(&ev)));
                            }
                        />
                        "Custom..."
                    </label>
                </div>
            </Show>
        </span>
    }
}

/// The Trame format toolbar (the launcher mounts it inside the chrome `Toolbar`
/// slot). Bold/Italic/Undo/Redo invoke their `CommandId` through `on_command`
/// (the shared command path); the rich-formatting controls (underline, font,
/// size, colours, alignment, number format, borders, wrap) emit a [`StylePatch`]
/// through `on_style`, which the launcher routes as `SetCellStyle` (sec.5bis).
#[component]
pub fn FormatBar(
    toggles: Toggles,
    style: StyleState,
    /// The active light/dark theme, so the colour pickers' "Automatic" /
    /// "Transparent" defaults match what the canvas actually renders.
    theme: RwSignal<ambre_canvas::Theme>,
    on_command: Callback<CommandId>,
    on_style: Callback<StylePatch>,
) -> impl IntoView {
    view! {
        <button
            title="Bold"
            aria-pressed=move || style.bold.get().then_some("true")
            on:click=move |_| on_command.run(BOLD)
        >
            <Icon icon=icondata::BsTypeBold />
        </button>
        <button
            title="Italic"
            aria-pressed=move || style.italic.get().then_some("true")
            on:click=move |_| on_command.run(ITALIC)
        >
            <Icon icon=icondata::BsTypeItalic />
        </button>
        <button
            title="Underline"
            aria-pressed=move || style.underline.get().then_some("true")
            on:click=move |_| on_style.run(StylePatch {
                underline: Some(!style.underline.get_untracked()),
                ..Default::default()
            })
        >
            <Icon icon=icondata::BsTypeUnderline />
        </button>
        <button
            title="Strikethrough"
            aria-pressed=move || style.strikethrough.get().then_some("true")
            on:click=move |_| on_style.run(StylePatch {
                strikethrough: Some(!style.strikethrough.get_untracked()),
                ..Default::default()
            })
        >
            <Icon icon=icondata::BsTypeStrikethrough />
        </button>
        <span class="sep"></span>

        // Font family + size.
        <select
            title="Font"
            prop:value=move || style.font.get().unwrap_or_else(|| "Default".to_owned())
            on:change=move |ev| {
                let v = event_target_value(&ev);
                let font = (v != "Default").then_some(v);
                on_style.run(StylePatch { font: Some(font), ..Default::default() });
            }
        >
            {FONTS
                .iter()
                .map(|f| view! { <option value=*f>{f.to_string()}</option> })
                .collect_view()}
        </select>
        <select
            title="Font size"
            prop:value=move || {
                style.size.get().map(|s| s.to_string()).unwrap_or_else(|| DEFAULT_SIZE.to_string())
            }
            on:change=move |ev| {
                let sz = event_target_value(&ev).parse::<u16>().ok();
                on_style.run(StylePatch { size: Some(sz), ..Default::default() });
            }
        >
            {SIZES
                .iter()
                .map(|&s| view! { <option value=s.to_string()>{s.to_string()}</option> })
                .collect_view()}
        </select>
        <span class="sep"></span>

        // Text colour: composed picker (palette + "Automatic" reset + custom).
        <ColorPopover
            title="Text colour"
            icon=icondata::BiFontColorRegular
            current=style.fg
            default=Signal::derive(move || theme_fg_default(theme.get()))
            reset_label="Automatic"
            on_pick=Callback::new(move |c| {
                on_style.run(StylePatch { fg: Some(c), ..Default::default() })
            })
        />
        // Fill colour: composed picker (palette + "No fill" reset + custom).
        <ColorPopover
            title="Fill colour"
            icon=icondata::BiColorFillSolid
            current=style.bg
            default=Signal::derive(move || theme_bg_default(theme.get()))
            reset_label="No fill (transparent)"
            on_pick=Callback::new(move |c| {
                on_style.run(StylePatch { bg: Some(c), ..Default::default() })
            })
        />
        <span class="sep"></span>

        // Horizontal alignment.
        <button
            title="Align left"
            aria-pressed=move || (style.h_align.get() == HAlign::Left).then_some("true")
            on:click=move |_| on_style.run(StylePatch { h_align: Some(HAlign::Left), ..Default::default() })
        >
            <Icon icon=icondata::BiAlignLeftRegular />
        </button>
        <button
            title="Align center"
            aria-pressed=move || (style.h_align.get() == HAlign::Center).then_some("true")
            on:click=move |_| on_style.run(StylePatch { h_align: Some(HAlign::Center), ..Default::default() })
        >
            <Icon icon=icondata::BiAlignMiddleRegular />
        </button>
        <button
            title="Align right"
            aria-pressed=move || (style.h_align.get() == HAlign::Right).then_some("true")
            on:click=move |_| on_style.run(StylePatch { h_align: Some(HAlign::Right), ..Default::default() })
        >
            <Icon icon=icondata::BiAlignRightRegular />
        </button>
        <span class="sep"></span>

        // Vertical alignment.
        <button
            title="Align top"
            aria-pressed=move || (style.v_align.get() == VAlign::Top).then_some("true")
            on:click=move |_| on_style.run(StylePatch { v_align: Some(VAlign::Top), ..Default::default() })
        >
            <Icon icon=icondata::AiVerticalAlignTopOutlined />
        </button>
        <button
            title="Align middle"
            aria-pressed=move || (style.v_align.get() == VAlign::Middle).then_some("true")
            on:click=move |_| on_style.run(StylePatch { v_align: Some(VAlign::Middle), ..Default::default() })
        >
            <Icon icon=icondata::AiVerticalAlignMiddleOutlined />
        </button>
        <button
            title="Align bottom"
            aria-pressed=move || (style.v_align.get() == VAlign::Bottom).then_some("true")
            on:click=move |_| on_style.run(StylePatch { v_align: Some(VAlign::Bottom), ..Default::default() })
        >
            <Icon icon=icondata::AiVerticalAlignBottomOutlined />
        </button>
        <span class="sep"></span>

        // Number format + decimals.
        <select
            title="Number format"
            prop:value=move || {
                NUMBER_FORMATS
                    .iter()
                    .position(|(f, _)| *f == style.format.get())
                    .unwrap_or(0)
                    .to_string()
            }
            on:change=move |ev| {
                let i = event_target_value(&ev).parse::<usize>().unwrap_or(0);
                let fmt = NUMBER_FORMATS[i.min(NUMBER_FORMATS.len() - 1)].0.clone();
                on_style.run(StylePatch { format: Some(fmt), ..Default::default() });
            }
        >
            {NUMBER_FORMATS
                .iter()
                .enumerate()
                .map(|(i, (_, label))| {
                    view! { <option value=i.to_string()>{label.to_string()}</option> }
                })
                .collect_view()}
        </select>
        <button
            title="Decrease decimals"
            on:click=move |_| on_style.run(StylePatch { decimals: Some(Some(0)), ..Default::default() })
        >
            <Icon icon=icondata::LuDecimalsArrowLeft />
        </button>
        <button
            title="Increase decimals"
            on:click=move |_| on_style.run(StylePatch { decimals: Some(Some(2)), ..Default::default() })
        >
            <Icon icon=icondata::LuDecimalsArrowRight />
        </button>
        <span class="sep"></span>

        // Borders: a single button opening a composed popover (pen + presets).
        <BordersPopover style=style on_style=on_style />
        <span class="sep"></span>

        // Wrap text.
        <button
            title="Wrap text"
            aria-pressed=move || style.wrap.get().then_some("true")
            on:click=move |_| on_style.run(StylePatch {
                wrap: Some(!style.wrap.get_untracked()),
                ..Default::default()
            })
        >
            <Icon icon=icondata::BsTextWrap />
        </button>

        // Merge / unmerge cells: a stateful toggle. Pressed when the active cell
        // is part of a merge, in which case clicking splits it (UNMERGE); on a
        // plain selection it merges the range (MERGE). The Logic thread guards
        // both (single-cell merge is a no-op, non-empty covered cell rejects).
        <button
            title="Merge cells"
            aria-pressed=move || style.merged.get().then_some("true")
            on:click=move |_| {
                on_command.run(if style.merged.get_untracked() { UNMERGE } else { MERGE })
            }
        >
            <Icon icon=icondata::AiMergeCellsOutlined />
        </button>
        <span class="sep"></span>

        <button
            title="Undo"
            disabled=move || !toggles.undo_enabled.get()
            on:click=move |_| on_command.run(UNDO)
        >
            <Icon icon=icondata::BiUndoRegular />
        </button>
        <button
            title="Redo"
            disabled=move || !toggles.redo_enabled.get()
            on:click=move |_| on_command.run(REDO)
        >
            <Icon icon=icondata::BiRedoRegular />
        </button>
    }
}

/// The floating selection toolbar's inner content, transform-synced onto the
/// active cell by the launcher's generic `.ambre-selbar-anchor` wrapper.
#[component]
pub fn SelectionBar(style: StyleState, on_command: Callback<CommandId>) -> impl IntoView {
    view! {
        <div class="ambre-selbar">
            <button
                title="Bold"
                aria-pressed=move || style.bold.get().then_some("true")
                on:click=move |_| on_command.run(BOLD)
            >
                "B"
            </button>
            <button
                title="Italic"
                aria-pressed=move || style.italic.get().then_some("true")
                on:click=move |_| on_command.run(ITALIC)
            >
                <em>"I"</em>
            </button>
        </div>
    }
}

// -----------------------------------------------------------------------------
// Formula bar (HTML-based, editable input)
// -----------------------------------------------------------------------------

/// The formula bar component: a name box showing the active cell reference
/// (e.g. `"B2"`) and an HTML `<input>` bound to the cell's raw value or the
/// live edit buffer. The input is always editable; focus or typed characters
/// open an edit session.
///
/// `on_action` fires on Enter (commit Down), Tab (commit Right), or Escape
/// (cancel). The caret, selection, IME, and clipboard are handled natively by
/// the HTML input element.
#[component]
pub fn FormulaBar(
    cell_ref: RwSignal<String>,
    content: RwSignal<String>,
    on_action: Callback<FormulaAction>,
) -> impl IntoView {
    let input_ref = NodeRef::<html::Input>::new();

    // When the signal changes from outside (e.g. a new cell was selected or an
    // edit was seeded), push the new value into the DOM input without resetting
    // the caret. The comparison avoids fighting the user's typing.
    create_effect(move |_| {
        let val = content.get();
        if let Some(input) = input_ref.get() {
            if val != input.value() {
                input.set_value(&val);
            }
        }
    });

    let on_input = move |ev: web_sys::Event| {
        content.set(event_target_value(&ev));
    };

    let on_keydown = {
        let content = content;
        move |ev: web_sys::KeyboardEvent| {
            match ev.key().as_str() {
                "Enter" => {
                    ev.prevent_default();
                    // Ctrl/Cmd+Enter fills the whole selection with the value.
                    if ev.ctrl_key() || ev.meta_key() {
                        on_action.run(FormulaAction::CommitFill);
                    } else {
                        on_action.run(FormulaAction::Commit(CommitDirection::Down));
                    }
                }
                "Tab" => {
                    ev.prevent_default();
                    on_action.run(FormulaAction::Commit(CommitDirection::Right));
                }
                "Escape" => {
                    ev.prevent_default();
                    on_action.run(FormulaAction::Cancel);
                }
                _ => {}
            }
        }
    };

    view! {
        <span class="name-box">{move || cell_ref.get()}</span>
        <input
            type="text"
            id="ambre-formula-input"
            class="formula-input"
            node_ref=input_ref
            on:input=on_input
            on:keydown=on_keydown
        />
    }
}

// -----------------------------------------------------------------------------
// Shared filter popover (`-ui.md` sec.5.D)
// -----------------------------------------------------------------------------

/// Open state of the shared filter popover: which grid column it targets and the
/// viewport (client-space) anchor point under which it is drawn. `None` means
/// closed. The launcher derives the anchor from the active cell's published
/// screen position (the active cell is in the filtered column).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterPopoverState {
    /// Grid column (0-based) the filter targets.
    pub col: u32,
    /// Client-space anchor (CSS px): the popover's top-left.
    pub x: f64,
    pub y: f64,
}

/// The condition operators offered by the popover, paired with their labels.
/// The `<select>` index maps back to the [`FilterOp`] on apply.
const FILTER_OPS: &[(FilterOp, &str)] = &[
    (FilterOp::Eq, "equals"),
    (FilterOp::Ne, "not equals"),
    (FilterOp::Gt, "greater than"),
    (FilterOp::Lt, "less than"),
    (FilterOp::Gte, "greater or equal"),
    (FilterOp::Lte, "less or equal"),
    (FilterOp::Contains, "contains"),
    (FilterOp::NotContains, "does not contain"),
    (FilterOp::StartsWith, "starts with"),
    (FilterOp::IsEmpty, "is empty"),
    (FilterOp::IsNotEmpty, "is not empty"),
];

/// A1-style column label for a 0-based grid column index (0 -> "A", 26 -> "AA").
fn col_label(mut col: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// The shared filter popover: anchored under the active column header, it offers
/// the "by values" (checkbox list of the column's distinct visible values) and a
/// minimal "by condition" (operator + value) modes (`-ui.md` sec.5.D). Applying
/// fires `on_apply` with the chosen [`FilterKind`]; "Clear filter" routes through
/// `on_command` (the shared command path). `values` is fed by the launcher from
/// [`TrameEvent::ColumnValues`].
#[component]
pub fn FilterPopover(
    state: RwSignal<Option<FilterPopoverState>>,
    values: RwSignal<Vec<String>>,
    on_apply: Callback<FilterKind>,
    on_command: Callback<CommandId>,
) -> impl IntoView {
    // true = "by values", false = "by condition".
    let by_values = RwSignal::new(true);
    // The set of currently-checked (visible) values in the "by values" mode.
    let checked = RwSignal::new(std::collections::BTreeSet::<String>::new());
    // Whenever a fresh value list arrives (popover (re)opened on a column), check
    // every value by default -- the unfiltered state.
    Effect::new(move |_| {
        let vs = values.get();
        checked.set(vs.into_iter().collect());
    });
    // Condition-mode operator + operand.
    let op_idx = RwSignal::new(0usize);
    let cond_value = RwSignal::new(String::new());

    let close = move || state.set(None);

    let apply = move |_| {
        let kind = if by_values.get_untracked() {
            FilterKind::Values(checked.get_untracked().into_iter().collect())
        } else {
            let (op, _) = FILTER_OPS[op_idx.get_untracked().min(FILTER_OPS.len() - 1)];
            FilterKind::Condition { op, value: cond_value.get_untracked() }
        };
        on_apply.run(kind);
        state.set(None);
    };

    move || {
        state.get().map(|st| {
            let style = format!(
                "position: fixed; left: {}px; top: {}px; z-index: 50;",
                st.x, st.y
            );
            view! {
                <div class="ambre-filter-popover" style=style>
                    <div class="ambre-filter-head">
                        {format!("Filter column {}", col_label(st.col))}
                    </div>
                    <div class="ambre-filter-modes">
                        <button
                            class="mode"
                            aria-pressed=move || by_values.get().then_some("true")
                            on:click=move |_| by_values.set(true)
                        >
                            "By values"
                        </button>
                        <button
                            class="mode"
                            aria-pressed=move || (!by_values.get()).then_some("true")
                            on:click=move |_| by_values.set(false)
                        >
                            "By condition"
                        </button>
                    </div>

                    <Show
                        when=move || by_values.get()
                        fallback=move || view! {
                            <div class="ambre-filter-cond">
                                <select on:change=move |ev| {
                                    op_idx.set(event_target_value(&ev).parse().unwrap_or(0));
                                }>
                                    {FILTER_OPS
                                        .iter()
                                        .enumerate()
                                        .map(|(i, (_, label))| {
                                            view! { <option value=i.to_string()>{label.to_string()}</option> }
                                        })
                                        .collect_view()}
                                </select>
                                <input
                                    type="text"
                                    placeholder="value"
                                    on:input=move |ev| cond_value.set(event_target_value(&ev))
                                />
                            </div>
                        }
                    >
                        <div class="ambre-filter-values">
                            <For
                                each=move || values.get()
                                key=|v| v.clone()
                                children=move |v: String| {
                                    let v_for_check = v.clone();
                                    let v_for_toggle = v.clone();
                                    let is_on = move || checked.get().contains(&v_for_check);
                                    view! {
                                        <label class="ambre-filter-value">
                                            <input
                                                type="checkbox"
                                                prop:checked=is_on
                                                on:change=move |ev| {
                                                    let on = event_target_checked(&ev);
                                                    checked.update(|set| {
                                                        if on {
                                                            set.insert(v_for_toggle.clone());
                                                        } else {
                                                            set.remove(&v_for_toggle);
                                                        }
                                                    });
                                                }
                                            />
                                            <span>{v}</span>
                                        </label>
                                    }
                                }
                            />
                        </div>
                    </Show>

                    <div class="ambre-filter-actions">
                        <button class="primary" on:click=apply>"Apply"</button>
                        <button on:click=move |_| {
                            on_command.run(CLEAR_FILTER);
                            close();
                        }>"Clear filter"</button>
                        <button on:click=move |_| close()>"Cancel"</button>
                    </div>
                </div>
            }
        })
    }
}
