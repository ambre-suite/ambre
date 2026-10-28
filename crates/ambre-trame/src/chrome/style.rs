// ambre-trame/src/chrome/style.rs
// ============================================================================
// The reactive chrome state the toolbars subscribe to. `Toggles` is the history
// enabled state, resolved from the command store, and `StyleState` is the active
// cell's resolved rich style. The inbound `StyleApplied` mirror rides along. Both
// are `Copy` signal bundles that may live inside the `<Router>` subtree.
// ============================================================================

use ambre_bus::{AppEvent, BorderStyle, CellFormat, HAlign, TrameEvent, UiEvent, VAlign};
use ambre_chrome::CommandStates;
use ambre_core::Rgb;
use leptos::prelude::*;

use super::commands::{REDO, UNDO};

/// The Send toggle signals the Trame chrome components subscribe to, resolved
/// once from the launcher's `CommandStates` store. Plain `RwSignal`s, so this is
/// `Copy` and `Send` and may live inside the `<Router>` subtree. The `!Send`
/// store itself, by contrast, is kept outside the reactive tree by the launcher.
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
    /// style event), not the command store. A command missing from the store
    /// reads as disabled.
    pub fn from_store(store: &CommandStates) -> Self {
        let enabled = |id| {
            store
                .enabled_signal(id)
                .unwrap_or_else(|| RwSignal::new(false))
        };
        Self {
            undo_enabled: enabled(UNDO),
            redo_enabled: enabled(REDO),
        }
    }
}

/// The active cell's resolved rich style, mirrored into `Copy` signals so the
/// formatting toolbar's selects/swatches/toggles reflect the selection. Fed by
/// [`TrameEvent::StyleApplied`] via [`apply_style_event`] -- this is the single
/// style event, so bold/italic live here too. Lives in the `<Router>` subtree.
#[derive(Clone, Copy)]
pub struct StyleState {
    pub bold: RwSignal<bool>,
    pub italic: RwSignal<bool>,
    pub underline: RwSignal<bool>,
    pub strikethrough: RwSignal<bool>,
    pub wrap: RwSignal<bool>,
    pub h_align: RwSignal<HAlign>,
    pub v_align: RwSignal<VAlign>,
    /// Whether the active cell's value is numeric. Lets the alignment control
    /// resolve [`HAlign::Auto`] the way the renderer does (numbers right, text
    /// left) so an `Auto` cell shows its de-facto alignment.
    pub numeric: RwSignal<bool>,
    pub format: RwSignal<CellFormat>,
    /// Borders popover picker state: the stroke style and colour the next preset
    /// click applies. Seeded (best-effort) from the active cell's first present
    /// edge so reopening the popover shows the cell's current pen.
    pub border_line: RwSignal<BorderStyle>,
    pub border_color: RwSignal<Option<Rgb>>,
    pub fg: RwSignal<Option<Rgb>>,
    pub bg: RwSignal<Option<Rgb>>,
    pub font: RwSignal<Option<ambre_core::FontFamily>>,
    pub size: RwSignal<Option<ambre_core::FontSize>>,
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
            numeric: RwSignal::new(false),
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
        event:
            AppEvent::Trame(TrameEvent::StyleApplied {
                style,
                bold,
                italic,
                merged,
                numeric,
            }),
        ..
    } = ev
    {
        s.bold.set(bold.as_bool());
        s.italic.set(italic.as_bool());
        s.merged.set(*merged);
        s.underline.set(style.underline.as_bool());
        s.strikethrough.set(style.strikethrough.as_bool());
        s.wrap.set(style.wrap.as_bool());
        s.h_align.set(style.h_align);
        s.v_align.set(style.v_align);
        s.numeric.set(*numeric);
        s.format.set(style.format.clone());
        // The borders pen (line style + colour) is user-owned and sticky: it is
        // NOT reseeded from the selected cell. Seeding it here would clobber an
        // explicit pick every time the selection emits `StyleApplied`, making the
        // line-style picker revert to the cell's current edge
        // (`trame-cell-styles`).
        s.fg.set(style.fg);
        s.bg.set(style.bg);
        s.font.set(style.font.clone());
        s.size.set(style.size);
    }
}
