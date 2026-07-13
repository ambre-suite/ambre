// ambre-toile/src/ui/edit.rs
// ============================================================================
// Inline text editor for sticky / text elements (toile-ui §6). Wraps a
// `parley::PlainEditor` -- the multi-line, layout-aware cousin of Trame's
// single-line `CellEditor`. PlainEditor owns its own `Layout` and re-shapes on
// every edit, so it needs a live `FontContext` for *any* operation (move,
// select, hit, render). The Toile render thread only holds that `FontContext`
// during paint (`ambre-app` frame host), so this editor cannot be driven at
// input time.
//
// The seam that resolves this is the same one Trame uses for its deferred hit
// resolution: `handle_input` records font-free [`EditIntent`]s in a queue; the
// paint pass drains and applies them through the PlainEditor driver, then
// renders caret / selection / glyphs. Reading the committed buffer needs no
// font context, so the view can emit `SetText` on close from the queue-flush.
//
// Coordinate spaces: PlainEditor lays out in its own local frame with the
// origin at the element's top-left screen corner and the zoom baked into the
// font size + wrap width (matching `render::paint_text`). The view converts
// screen pointers to this local frame before queueing a pointer intent, so this
// module stays font-free at input time and purely geometric at paint time.
// ============================================================================

use ambre_bus::app::toile::{DisplayElement, ElementId};
use ambre_canvas::Palette;
use ambre_draw::{
    Affine, Alignment, BoundingBox, Brush, Color, DrawSurface, FontContext, FontStyle, FontWeight,
    LayoutContext, PlainEditor, Point, PositionedLayoutItem, Rect, StyleProperty,
};

/// Caret bar width in screen pixels.
const CARET_WIDTH: f32 = 1.5;
/// Inner padding (screen px at zoom 1) between the element border and the text,
/// matching the sticky / text body inset.
const TEXT_PAD: f64 = 4.0;

/// A font-free editing action recorded at input time and applied at paint time,
/// when the `FontContext` is available. Pointer coordinates are already mapped
/// into the editor's local (element-top-left-relative) screen frame.
#[derive(Clone, Debug)]
pub enum EditIntent {
    /// Insert committed text at the caret (latin typing / paste / IME commit).
    Insert(String),
    /// Insert a hard line break (Enter in a multi-line body).
    Newline,
    Backspace,
    Delete,
    /// Horizontal caret motion; `select` extends the selection (Shift), `word`
    /// jumps by word (Ctrl/Alt).
    Left {
        select: bool,
        word: bool,
    },
    Right {
        select: bool,
        word: bool,
    },
    /// Vertical caret motion across wrapped lines; `select` extends (Shift).
    Up {
        select: bool,
    },
    Down {
        select: bool,
    },
    /// Caret to the start / end of the current visual line; `select` extends.
    LineStart {
        select: bool,
    },
    LineEnd {
        select: bool,
    },
    SelectAll,
    /// Place the caret at a local-space point (single click / drag start).
    PointerDown {
        x: f32,
        y: f32,
    },
    /// Extend the selection to a local-space point (drag / shift-click).
    PointerDrag {
        x: f32,
        y: f32,
    },
    /// Select the word under a local-space point (double click).
    WordAt {
        x: f32,
        y: f32,
    },
    /// Live IME composition preview for this pass (replaces the previous).
    Compose(String),
    /// IME composition committed: finalize the preedit as real text.
    ComposeEnd(String),
}

/// An in-progress inline edit of one sticky / text element.
pub struct InlineEditor {
    /// The element being edited (its body is authoritative here until close).
    id: ElementId,
    editor: PlainEditor<Brush>,
    /// The editor's own layout context (PlainEditor re-shapes through it). Kept
    /// here so the editor is self-contained and never borrows the shared
    /// `TextCache` layout context.
    layout_cx: LayoutContext<Brush>,
    /// Font-free actions awaiting a paint-time flush (`FontContext` needed).
    intents: Vec<EditIntent>,
    /// The body at open time, to suppress a no-op `SetText` on close.
    initial: String,
    /// Whether the caret has been seeded to the text end (done on first flush).
    seeded: bool,

    // Style snapshot captured at open, applied to the editor's default style so
    // the edit looks like the committed result.
    weight: u16,
    italic: bool,
    /// Font size in world units (zoom is applied per paint).
    base_size: f32,
    color: Color,
}

impl InlineEditor {
    /// Open an editor on `elem`, seeded with its current body `text`.
    pub fn open(elem: &DisplayElement, text: &str, palette: &Palette) -> Self {
        let font = &elem.style.font;
        let color = elem.style.fill.map(argb_text).unwrap_or(palette.cell_text);
        let mut editor = PlainEditor::<Brush>::new(font.size.max(1.0));
        editor.set_text(text);
        editor.set_alignment(Alignment::Start);
        Self {
            id: elem.id.clone(),
            editor,
            layout_cx: LayoutContext::new(),
            intents: Vec::new(),
            initial: text.to_owned(),
            seeded: false,
            weight: font.weight,
            italic: font.italic,
            base_size: font.size.max(1.0),
            color,
        }
    }

    /// The element currently under edit.
    pub fn id(&self) -> &ElementId {
        &self.id
    }

    /// Inner padding in screen pixels at the given zoom.
    pub fn pad(zoom: f64) -> f64 {
        TEXT_PAD * zoom
    }

    /// Record an action to apply at the next paint-time flush.
    pub fn push(&mut self, intent: EditIntent) {
        self.intents.push(intent);
    }

    /// The committed body, or `None` when it is unchanged from open (no-op
    /// commit). IME preedit is excluded. Requires no font context.
    pub fn committed_text(&self) -> Option<String> {
        let text: String = self.editor.text().chars().collect();
        (text != self.initial).then_some(text)
    }

    /// Push the pinned style + geometry onto the editor for this paint, then
    /// apply every queued intent through the driver. Must run before rendering
    /// so the layout reflects the latest edits. `wrap_w` is the wrap width in
    /// screen pixels; `zoom` bakes into the font size.
    pub fn flush(&mut self, font_cx: &mut FontContext, wrap_w: f32, zoom: f64) {
        // Re-apply style + geometry each frame: zoom (hence font size + wrap
        // width) can change mid-edit, and PlainEditor holds no camera.
        let styles = self.editor.edit_styles();
        styles.insert(StyleProperty::FontSize(self.base_size * zoom as f32));
        styles.insert(StyleProperty::FontWeight(FontWeight::new(
            self.weight as f32,
        )));
        styles.insert(StyleProperty::FontStyle(if self.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }));
        styles.insert(StyleProperty::Brush(Brush::Solid(self.color)));
        self.editor.set_width(Some(wrap_w.max(1.0)));

        let mut drv = self.editor.driver(font_cx, &mut self.layout_cx);
        if !self.seeded {
            // Open with the caret at the end of the seeded body (Trame parity).
            drv.move_to_text_end();
            self.seeded = true;
        }
        for intent in self.intents.drain(..) {
            match intent {
                EditIntent::Insert(s) => drv.insert_or_replace_selection(&s),
                EditIntent::Newline => drv.insert_or_replace_selection("\n"),
                EditIntent::Backspace => drv.backdelete(),
                EditIntent::Delete => drv.delete(),
                EditIntent::Left { select, word } => match (select, word) {
                    (false, false) => drv.move_left(),
                    (true, false) => drv.select_left(),
                    (false, true) => drv.move_word_left(),
                    (true, true) => drv.select_word_left(),
                },
                EditIntent::Right { select, word } => match (select, word) {
                    (false, false) => drv.move_right(),
                    (true, false) => drv.select_right(),
                    (false, true) => drv.move_word_right(),
                    (true, true) => drv.select_word_right(),
                },
                EditIntent::Up { select } => {
                    if select {
                        drv.select_up()
                    } else {
                        drv.move_up()
                    }
                }
                EditIntent::Down { select } => {
                    if select {
                        drv.select_down()
                    } else {
                        drv.move_down()
                    }
                }
                EditIntent::LineStart { select } => {
                    if select {
                        drv.select_to_line_start()
                    } else {
                        drv.move_to_line_start()
                    }
                }
                EditIntent::LineEnd { select } => {
                    if select {
                        drv.select_to_line_end()
                    } else {
                        drv.move_to_line_end()
                    }
                }
                EditIntent::SelectAll => drv.select_all(),
                EditIntent::PointerDown { x, y } => drv.move_to_point(x, y),
                EditIntent::PointerDrag { x, y } => drv.extend_selection_to_point(x, y),
                EditIntent::WordAt { x, y } => drv.select_word_at_point(x, y),
                EditIntent::Compose(s) => {
                    if s.is_empty() {
                        drv.clear_compose();
                    } else {
                        drv.set_compose(&s, None);
                    }
                }
                EditIntent::ComposeEnd(s) => {
                    // Finalize: drop the preedit, then insert the committed text.
                    drv.clear_compose();
                    if !s.is_empty() {
                        drv.insert_or_replace_selection(&s);
                    }
                }
            }
        }
    }

    /// The caret rectangle in local editor coordinates, for the transparent
    /// textarea pipe / IME anchor. Call after [`flush`](Self::flush).
    pub fn caret_local(&mut self) -> Option<BoundingBox> {
        self.editor.cursor_geometry(CARET_WIDTH)
    }

    /// Draw the selection band, glyphs and caret over the element (screen space).
    /// `origin` is the element's top-left screen corner + padding; the editor's
    /// local coordinates are offset by it. Call after [`flush`](Self::flush).
    pub fn render<S: DrawSurface>(&mut self, surface: &mut S, origin: Point, palette: &Palette) {
        let dx = origin.x;
        let dy = origin.y;

        // Selection band behind the text.
        let sel = palette.selection.with_alpha(0.30);
        for (bb, _line) in self.editor.selection_geometry() {
            surface.fill_rect(offset_box(bb, dx, dy), sel);
        }

        // Glyph runs.
        let brush = Brush::Solid(self.color);
        let layout = self.editor.try_layout();
        if let Some(layout) = layout {
            for line in layout.lines() {
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else {
                        continue;
                    };
                    let tx = dx + run.offset() as f64;
                    let ty = dy + run.baseline() as f64;
                    surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
                }
            }
        }

        // Caret bar.
        if let Some(bb) = self.editor.cursor_geometry(CARET_WIDTH) {
            surface.fill_rect(offset_box(bb, dx, dy), palette.caret);
        }
    }
}

/// Translate a parley `BoundingBox` (local editor space) into a screen `Rect`.
fn offset_box(bb: BoundingBox, dx: f64, dy: f64) -> Rect {
    Rect::new(bb.x0 + dx, bb.y0 + dy, bb.x1 + dx, bb.y1 + dy)
}

/// ARGB `u32` text color to an opaque `Color` (text ignores element opacity).
fn argb_text(v: u32) -> Color {
    let r = ((v >> 16) & 0xff) as u8;
    let g = ((v >> 8) & 0xff) as u8;
    let b = (v & 0xff) as u8;
    Color::from_rgba8(r, g, b, 255)
}
