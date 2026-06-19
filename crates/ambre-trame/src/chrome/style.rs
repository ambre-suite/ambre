// ambre-trame/src/chrome/style.rs
// ============================================================================
// The reactive chrome state the toolbars subscribe to: `Toggles` (history
// enabled state, resolved from the command store) and `StyleState` (the active
// cell's resolved rich style), plus the inbound `StyleApplied` mirror. Both are
// `Copy` signal bundles that may live inside the `<Router>` subtree.
// ============================================================================

use ambre_bus::{AppEvent, CellFormat, BorderStyle, HAlign, TrameEvent, UiEvent, VAlign};
use ambre_chrome::CommandStates;
use leptos::prelude::*;

use super::commands::{REDO, UNDO};

/// The Send toggle signals the Trame chrome components subscribe to, resolved
/// once from the launcher's `CommandStates` store. Plain `RwSignal`s, so this is
/// `Copy` and `Send` and may live inside the `<Router>` subtree (unlike the
/// `!Send` store itself, which the launcher keeps outside the reactive tree).
#[derive(Clone, Copy)]
pub struct Toggles {
    /// Undo/Redo availability (Loro `UndoManager` stacks), driving the matching
    /// buttons' `disabled` state. Fed by [`TrameEvent::UndoState`] via
    /// [`super::mirror_event`].
    pub undo_enabled: RwSignal<bool>,
    pub redo_enabled: RwSignal<bool>,
}

impl Toggles {
    /// Resolve the history-enabled signals from a store built over
    /// [`super::COMMANDS`]. Bold/italic now live in [`StyleState`] (the single
    /// style channel), not the command store.
    pub fn from_store(store: &CommandStates) -> Self {
        Self {
            undo_enabled: store.enabled_signal(UNDO).expect("undo registered"),
            redo_enabled: store.enabled_signal(REDO).expect("redo registered"),
        }
    }
}

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
        // The borders pen (line style + colour) is user-owned and sticky: it is
        // NOT reseeded from the selected cell. Seeding it here would clobber an
        // explicit pick every time the selection emits `StyleApplied`, making the
        // line-style picker revert to the cell's current edge (`-ui.md` sec.5bis).
        s.fg.set(style.fg);
        s.bg.set(style.bg);
        s.font.set(style.font.clone());
        s.size.set(style.size);
    }
}
