// ambre-canvas/src/editor.rs
// ============================================================================
// Shared inline text-editor core, built on `parley::PlainEditor` (the multi-line,
// layout-aware editor). Both apps drive the SAME core: Toile's sticky / text
// element editor and Trame's cell editor wrap it (`ambre-toile/src/ui/edit.rs`,
// `ambre-trame/src/ui/grid`).
//
// PlainEditor owns its own `Layout` and re-shapes on every edit, so it needs a
// live `FontContext` for *any* operation (move, select, hit, render). The render
// thread only holds that `FontContext` during paint (`ambre-app` frame host), so
// the editor cannot be driven at input time. The seam that resolves this: input
// handling records font-free [`EditIntent`]s in a queue; the paint pass drains
// and applies them through the PlainEditor driver, then renders caret / selection
// / glyphs. Reading the committed buffer needs no font context, so a view can
// emit its commit from the queue-flush.
//
// Coordinate spaces: PlainEditor lays out in its own local frame with the origin
// at the text's top-left screen corner and the zoom baked into the font size +
// wrap width. The host converts screen pointers to this local frame before
// queueing a pointer intent, so this module stays font-free at input time and
// purely geometric at paint time.
// ============================================================================

use ambre_draw::{
    Affine, Alignment, BezPath, BoundingBox, Brush, Color, DrawSurface, FontContext, FontFamily,
    FontStack, FontStyle, FontWeight, LayoutContext, PlainEditor, Point, PositionedLayoutItem,
    Rect, StyleProperty,
};

use crate::registered_family_name;

/// Caret bar width in screen pixels.
pub const CARET_WIDTH: f32 = 1.5;

/// A font-free editing action recorded at input time and applied at paint time,
/// when the `FontContext` is available. Pointer coordinates are already mapped
/// into the editor's local (text-top-left-relative) screen frame.
#[derive(Clone, Debug)]
pub enum EditIntent {
    /// Insert committed text at the caret (latin typing / paste / IME commit).
    Insert(String),
    /// Insert a hard line break (Enter in a multi-line body).
    Newline,
    Backspace,
    Delete,
    /// Delete the current selection (cut). No-op when the selection is collapsed.
    DeleteSelection,
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

/// The pinned visual style of an edit, captured by the host at open and re-applied
/// to the editor's default style each frame so the in-edit body matches the
/// committed look. `base_size` is in world units (zoom is applied per paint);
/// `family` is a family label resolved through [`registered_family_name`].
#[derive(Clone, Debug, PartialEq)]
pub struct TextEditorStyle {
    pub weight: u16,
    pub italic: bool,
    pub family: String,
    pub base_size: f32,
    pub color: Color,
}

/// A shared inline text edit over one text unit (a Toile element body or a Trame
/// cell). App-specific concerns (which unit, how the style / geometry is derived,
/// how a commit is dispatched) live in the host wrapper; this core owns only the
/// PlainEditor, its layout context, the intent queue and the pinned style.
pub struct TextEditor {
    editor: PlainEditor<Brush>,
    /// The editor's own layout context (PlainEditor re-shapes through it). Kept
    /// here so the editor is self-contained and never borrows the shared
    /// `TextCache` layout context.
    layout_cx: LayoutContext<Brush>,
    /// Font-free actions awaiting a paint-time flush (`FontContext` needed).
    intents: Vec<EditIntent>,
    /// The body at open time, to suppress a no-op commit on close.
    initial: String,
    /// Whether the caret has been seeded to the text end (done on first flush).
    seeded: bool,
    /// Pinned style, re-applied every frame in [`flush`](Self::flush).
    style: TextEditorStyle,
}

impl TextEditor {
    /// Open an editor seeded with `text` and the pinned `style`. The caret is
    /// placed at the end of the seeded body on the first flush (Trame parity).
    pub fn new(text: &str, style: TextEditorStyle) -> Self {
        let mut editor = PlainEditor::<Brush>::new(style.base_size);
        editor.set_text(text);
        editor.set_alignment(Alignment::Start);
        Self {
            editor,
            layout_cx: LayoutContext::new(),
            intents: Vec::new(),
            initial: text.to_owned(),
            seeded: false,
            style,
        }
    }

    /// Re-pin the style (the host may recompute it per frame, e.g. on a font /
    /// size change). Applied at the next [`flush`](Self::flush).
    pub fn set_style(&mut self, style: TextEditorStyle) {
        self.style = style;
    }

    /// Record an action to apply at the next paint-time flush.
    pub fn push(&mut self, intent: EditIntent) {
        self.intents.push(intent);
    }

    // -- Named intent constructors -------------------------------------------
    // Readable one-liners for the common editing operations; each just queues
    // the matching `EditIntent`, applied at the next `flush`. Callers that need
    // an uncovered variant still use `push` directly.

    /// Insert literal text at the caret (typing / paste).
    pub fn insert(&mut self, s: &str) {
        self.push(EditIntent::Insert(s.to_owned()));
    }
    /// Insert a line break at the caret.
    pub fn newline(&mut self) {
        self.push(EditIntent::Newline);
    }
    /// Delete the selection, else the char before the caret.
    pub fn backspace(&mut self) {
        self.push(EditIntent::Backspace);
    }
    /// Delete the selection, else the char after the caret.
    pub fn delete_forward(&mut self) {
        self.push(EditIntent::Delete);
    }
    /// Delete the current selection only (no-op when empty).
    pub fn delete_selection(&mut self) {
        self.push(EditIntent::DeleteSelection);
    }
    /// Move the caret left, collapsing any selection.
    pub fn move_left(&mut self) {
        self.push(EditIntent::Left {
            select: false,
            word: false,
        });
    }
    /// Move the caret right, collapsing any selection.
    pub fn move_right(&mut self) {
        self.push(EditIntent::Right {
            select: false,
            word: false,
        });
    }
    /// Extend the selection one place left.
    pub fn extend_left(&mut self) {
        self.push(EditIntent::Left {
            select: true,
            word: false,
        });
    }
    /// Extend the selection one place right.
    pub fn extend_right(&mut self) {
        self.push(EditIntent::Right {
            select: true,
            word: false,
        });
    }
    /// Move the caret to the start of the line.
    pub fn move_home(&mut self) {
        self.push(EditIntent::LineStart { select: false });
    }
    /// Move the caret to the end of the line.
    pub fn move_end(&mut self) {
        self.push(EditIntent::LineEnd { select: false });
    }
    /// Extend the selection to the start of the line.
    pub fn extend_home(&mut self) {
        self.push(EditIntent::LineStart { select: true });
    }
    /// Extend the selection to the end of the line.
    pub fn extend_end(&mut self) {
        self.push(EditIntent::LineEnd { select: true });
    }
    /// Move the caret up one line.
    pub fn move_up(&mut self) {
        self.push(EditIntent::Up { select: false });
    }
    /// Move the caret down one line.
    pub fn move_down(&mut self) {
        self.push(EditIntent::Down { select: false });
    }
    /// Extend the selection up one line.
    pub fn extend_up(&mut self) {
        self.push(EditIntent::Up { select: true });
    }
    /// Extend the selection down one line.
    pub fn extend_down(&mut self) {
        self.push(EditIntent::Down { select: true });
    }
    /// Select the whole buffer.
    pub fn select_all(&mut self) {
        self.push(EditIntent::SelectAll);
    }
    /// Set the live IME composition preview (CompositionUpdate).
    pub fn set_compose(&mut self, text: &str) {
        self.push(EditIntent::Compose(text.to_owned()));
    }
    /// Finalize the IME composition into committed text (CompositionEnd).
    pub fn commit_compose(&mut self, text: &str) {
        self.push(EditIntent::ComposeEnd(text.to_owned()));
    }
    /// Place the caret at a local-space (text-relative) point (click / drag start).
    pub fn point_down(&mut self, x: f32, y: f32) {
        self.push(EditIntent::PointerDown { x, y });
    }
    /// Extend the selection to a local-space point (drag / shift-click).
    pub fn point_drag(&mut self, x: f32, y: f32) {
        self.push(EditIntent::PointerDrag { x, y });
    }
    /// Select the word at a local-space point (double click).
    pub fn word_at(&mut self, x: f32, y: f32) {
        self.push(EditIntent::WordAt { x, y });
    }

    /// Replace the whole buffer synchronously (no font context needed), caret
    /// re-seeded to the end at the next flush. For a host that owns the text
    /// elsewhere and pushes it in wholesale (Trame's formula bar, which holds DOM
    /// focus + caret while the in-cell editor is only the render / commit vehicle).
    /// `initial` is left untouched, so [`committed_text`](Self::committed_text)
    /// still compares against the open-time value.
    pub fn set_text(&mut self, text: &str) {
        if self.text() != text {
            self.editor.set_text(text);
            self.seeded = false;
        }
    }

    /// The current buffer text (IME preedit excluded). Font-free.
    pub fn text(&self) -> String {
        self.editor.text().chars().collect()
    }

    /// The committed body, or `None` when it is unchanged from open (no-op
    /// commit). IME preedit is excluded. Requires no font context.
    pub fn committed_text(&self) -> Option<String> {
        let text = self.text();
        (text != self.initial).then_some(text)
    }

    /// The currently selected text, or `None` when the selection is collapsed
    /// (or an IME composition is active). Font-free: reads the buffer over the
    /// selection byte range, so it can be called synchronously inside a native
    /// clipboard `copy` / `cut` event.
    pub fn selected_text(&self) -> Option<String> {
        self.editor.selected_text().map(str::to_owned)
    }

    /// Push the pinned style + geometry onto the editor for this paint, then apply
    /// every queued intent through the driver. Must run before rendering so the
    /// layout reflects the latest edits. `wrap_w` is the wrap width in screen
    /// pixels; `zoom` bakes into the font size.
    pub fn flush(&mut self, font_cx: &mut FontContext, wrap_w: f32, zoom: f64) {
        // Re-apply style + geometry each frame: zoom (hence font size + wrap
        // width) can change mid-edit, and PlainEditor holds no camera.
        let styles = self.editor.edit_styles();
        styles.insert(StyleProperty::FontSize(self.style.base_size * zoom as f32));
        styles.insert(StyleProperty::FontWeight(FontWeight::new(
            self.style.weight as f32,
        )));
        styles.insert(StyleProperty::FontStyle(if self.style.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }));
        // Resolve the family to the same registered face the static render uses, so
        // the in-edit body matches its committed look.
        let face = registered_family_name(Some(self.style.family.as_str()));
        styles.insert(StyleProperty::FontStack(FontStack::Single(
            FontFamily::Named(face.into()),
        )));
        styles.insert(StyleProperty::Brush(Brush::Solid(self.style.color)));
        self.editor.set_width(Some(wrap_w.max(1.0)));

        // Scope the driver so its borrow of `font_cx` / `layout_cx` ends before the
        // layout refresh below.
        {
            let intents = std::mem::take(&mut self.intents);
            let mut drv = self.editor.driver(font_cx, &mut self.layout_cx);
            if !self.seeded {
                // Open with the caret at the end of the seeded body (Trame parity).
                drv.move_to_text_end();
                self.seeded = true;
            }
            for intent in intents {
                match intent {
                    EditIntent::Insert(s) => drv.insert_or_replace_selection(&s),
                    EditIntent::Newline => drv.insert_or_replace_selection("\n"),
                    EditIntent::Backspace => drv.backdelete(),
                    EditIntent::Delete => drv.delete(),
                    EditIntent::DeleteSelection => drv.delete_selection(),
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

        // Rebuild the layout before the caller renders. `edit_styles()` and
        // `set_width()` above mark the layout dirty every frame, but only a driver
        // operation refreshes it -- so on a frame with no intents (the common
        // steady-caret case) the layout would stay dirty and `try_layout()` /
        // `cursor_geometry()` would return `None`, painting neither glyphs nor
        // caret. Refreshing here keeps both live every frame.
        self.editor.refresh_layout(font_cx, &mut self.layout_cx);
    }

    /// The caret rectangle in local editor coordinates, for the transparent
    /// textarea pipe / IME anchor. Call after [`flush`](Self::flush).
    pub fn caret_local(&mut self) -> Option<BoundingBox> {
        self.editor.cursor_geometry(CARET_WIDTH)
    }

    /// The total laid-out text height in local (screen) pixels, or `None` before
    /// the first layout. Lets a host grow its box to fit wrapped lines. Call after
    /// [`flush`](Self::flush).
    pub fn layout_height(&self) -> Option<f32> {
        self.editor.try_layout().map(|l| l.height())
    }

    /// Draw the selection band, glyphs and caret at `origin` (screen space). The
    /// editor's local coordinates are offset by `origin`. `selection` is the base
    /// selection tint (drawn at 30% alpha behind the text); `show_caret` gates the
    /// blinking caret bar (the caller toggles it on a timer). The caret takes the
    /// style colour, not a theme caret: the body ink is fixed (e.g. a sticky's dark
    /// ink on a light card), so a theme caret could be invisible against it.
    ///
    /// Everything is drawn in the editor's local (origin-relative) frame, so a
    /// caller can rotate the whole overlay by pushing a transform on the surface
    /// before calling (Toile rotated elements). The selection band and caret use
    /// `fill_path` (not `fill_rect`) precisely so they follow such a rotation --
    /// `fill_rect` only maps an axis-aligned bounding box. Call after
    /// [`flush`](Self::flush).
    pub fn render<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        origin: Point,
        selection: Color,
        show_caret: bool,
    ) {
        let dx = origin.x;
        let dy = origin.y;

        // Selection band behind the text.
        let sel = Brush::Solid(selection.with_alpha(0.30));
        for (bb, _line) in self.editor.selection_geometry() {
            surface.fill_path(&rect_path(offset_box(bb, dx, dy)), &sel);
        }

        // Glyph runs.
        let brush = Brush::Solid(self.style.color);
        if let Some(layout) = self.editor.try_layout() {
            for line in layout.lines() {
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else {
                        continue;
                    };
                    let tx = dx + run.offset() as f64;
                    let ty = dy + run.baseline() as f64;
                    surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));

                    // Underline decoration. parley underlines the IME preedit
                    // range itself (a `StyleProperty::Underline` on the composing
                    // text), so drawing the run's underline here restores the IME
                    // preedit hint that `draw_glyphs` alone drops. Drawn as a thin
                    // filled rect (not `stroke_line`) so it follows a caller
                    // rotation like the glyphs and caret. Placed 2px below the
                    // baseline, matching the static cell decoration.
                    if run.style().underline.is_some() {
                        let adv = run.advance() as f64;
                        let uy = ty + 2.0;
                        let bar = Rect::new(tx, uy, tx + adv, uy + 1.0);
                        surface.fill_path(&rect_path(bar), &brush);
                    }
                }
            }
        }

        // Caret bar (blinking: only on the "on" half of the cycle).
        if show_caret && let Some(bb) = self.editor.cursor_geometry(CARET_WIDTH) {
            surface.fill_path(&rect_path(offset_box(bb, dx, dy)), &brush);
        }
    }
}

/// Translate a parley `BoundingBox` (local editor space) into a screen `Rect`.
fn offset_box(bb: BoundingBox, dx: f64, dy: f64) -> Rect {
    Rect::new(bb.x0 + dx, bb.y0 + dy, bb.x1 + dx, bb.y1 + dy)
}

/// A closed rectangle as a `BezPath`, so a filled rect follows the surface
/// transform stack (including rotation) instead of collapsing to its AABB.
fn rect_path(r: Rect) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((r.x0, r.y0));
    p.line_to((r.x1, r.y0));
    p.line_to((r.x1, r.y1));
    p.line_to((r.x0, r.y1));
    p.close_path();
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> TextEditorStyle {
        TextEditorStyle {
            weight: 400,
            italic: false,
            family: "Sans".to_owned(),
            base_size: 16.0,
            color: Color::from_rgba8(0, 0, 0, 255),
        }
    }

    /// Drive the core in a native host test (system fonts, no browser). Char
    /// motion + edits are deterministic here, which is what the Trame parity
    /// corpus relies on (`ambre-trame/src/ui/grid` tests).
    fn left() -> EditIntent {
        EditIntent::Left {
            select: false,
            word: false,
        }
    }

    #[test]
    fn char_motion_then_insert_lands_at_caret() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("hello world", style());
        ed.flush(&mut fcx, 1000.0, 1.0); // seeds caret at end
        ed.push(left());
        ed.push(left());
        ed.push(EditIntent::Insert("X".to_owned()));
        ed.flush(&mut fcx, 1000.0, 1.0);
        assert_eq!(ed.text(), "hello worXld");
    }

    #[test]
    fn backspace_and_delete_selection() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("abc", style());
        ed.flush(&mut fcx, 1000.0, 1.0);
        ed.push(EditIntent::Backspace);
        ed.flush(&mut fcx, 1000.0, 1.0);
        assert_eq!(ed.text(), "ab");
        ed.push(EditIntent::SelectAll);
        ed.push(EditIntent::DeleteSelection);
        ed.flush(&mut fcx, 1000.0, 1.0);
        assert_eq!(ed.text(), "");
    }

    #[test]
    fn select_all_reports_whole_buffer() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("hello world", style());
        ed.flush(&mut fcx, 1000.0, 1.0);
        ed.push(EditIntent::SelectAll);
        ed.flush(&mut fcx, 1000.0, 1.0);
        assert_eq!(ed.selected_text().as_deref(), Some("hello world"));
    }

    #[test]
    fn newline_makes_multi_line_text() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("a", style());
        ed.flush(&mut fcx, 1000.0, 1.0);
        ed.push(EditIntent::Newline);
        ed.push(EditIntent::Insert("b".to_owned()));
        ed.flush(&mut fcx, 1000.0, 1.0);
        assert_eq!(ed.text(), "a\nb");
    }

    /// The IME preedit hint: parley underlines the composing range, and `render`
    /// relies on that to redraw the underline `draw_glyphs` drops. Assert the
    /// composing run carries the underline decoration (and that `ComposeEnd`
    /// clears it) so the render path has something to draw.
    #[test]
    fn ime_preedit_run_is_underlined() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("", style());
        ed.flush(&mut fcx, 1000.0, 1.0);
        ed.push(EditIntent::Compose("abc".to_owned()));
        ed.flush(&mut fcx, 1000.0, 1.0);

        let underlined = ed
            .editor
            .try_layout()
            .expect("layout after flush")
            .lines()
            .flat_map(|l| l.items())
            .any(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run.style().underline.is_some(),
                _ => false,
            });
        assert!(underlined, "composing preedit run should be underlined");

        ed.push(EditIntent::ComposeEnd("abc".to_owned()));
        ed.flush(&mut fcx, 1000.0, 1.0);
        assert_eq!(ed.text(), "abc");
        let still_underlined = ed
            .editor
            .try_layout()
            .expect("layout after flush")
            .lines()
            .flat_map(|l| l.items())
            .any(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run.style().underline.is_some(),
                _ => false,
            });
        assert!(
            !still_underlined,
            "committed text should not stay underlined"
        );
    }
}
