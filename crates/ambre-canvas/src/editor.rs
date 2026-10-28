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
// handling records font-free [`EditIntent`]s in a queue. The paint pass drains
// and applies them through the PlainEditor driver, then renders caret / selection
// / glyphs. Reading the committed buffer needs no font context, so a view can
// emit its commit from the queue-flush.
//
// Coordinate spaces: PlainEditor lays out in its own local frame. The origin
// sits at the text's top-left screen corner, with the zoom baked into the font
// size and the wrap width. The host converts screen pointers to this local frame
// before queueing a pointer intent. This module therefore stays font-free at
// input time and purely geometric at paint time.
//
// A host may pin spans: ranges drawn in a look of their own. PlainEditor lays
// its text out with one style, so the spans get a layout of their own, and
// every geometry is read from it (`CANVAS-078`).
// ============================================================================

use ambre_bus::{ByteOffset, ByteSpan};
use ambre_core::{DocPoint, DocPx, ViewPx, ZoomFactor};
use ambre_draw::{
    Affine, Alignment, BezPath, BoundingBox, Brush, Color, DrawSurface, FontContext, FontFamily,
    FontFamilyName, FontStyle, FontWeight, LayoutContext, PlainEditor, Point, PositionedLayoutItem,
    Rect, StyleProperty,
};
use parley::{Affinity, AlignmentOptions, Cursor, Layout, Selection};

use crate::registered_family_name;
use crate::text_cache::{SpanShape, break_indented, push_shape};

/// Caret bar width in screen pixels.
pub const CARET_WIDTH: ViewPx = ViewPx::new(1.5);

/// What a caret motion does to the selection: the caret moves alone, or the
/// selection extends with it (Shift).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extend {
    Move,
    Select,
}

impl Extend {
    /// `Select` when `shift` holds, `Move` otherwise.
    pub fn select_if(shift: bool) -> Self {
        if shift { Extend::Select } else { Extend::Move }
    }
}

/// How far a horizontal caret motion steps: one character, or one word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stride {
    Char,
    Word,
}

impl Stride {
    /// `Word` when `word` holds, `Char` otherwise.
    pub fn word_if(word: bool) -> Self {
        if word { Stride::Word } else { Stride::Char }
    }
}

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
        select: Extend,
        word: Stride,
    },
    Right {
        select: Extend,
        word: Stride,
    },
    /// Vertical caret motion across wrapped lines; `select` extends (Shift).
    Up {
        select: Extend,
    },
    Down {
        select: Extend,
    },
    /// Caret to the start / end of the current visual line; `select` extends.
    LineStart {
        select: Extend,
    },
    LineEnd {
        select: Extend,
    },
    SelectAll,
    /// Place the caret at a local-space point (single click / drag start).
    PointerDown(DocPoint),
    /// Extend the selection to a local-space point (drag / shift-click).
    PointerDrag(DocPoint),
    /// Select the word under a local-space point (double click).
    WordAt(DocPoint),
    /// Live IME composition preview for this pass (replaces the previous).
    Compose(String),
    /// IME composition committed: finalize the preedit as real text.
    ComposeEnd(String),
    /// Replace the bytes `start..end` of the buffer with `text`, the caret
    /// after it. A range off a char boundary changes nothing.
    Replace {
        start: ByteOffset,
        end: ByteOffset,
        text: String,
    },
}

/// The pinned visual style of an edit, captured by the host at open and re-applied
/// to the editor's default style each frame. The in-edit body then matches the
/// committed look. `base_size` is in world units (zoom is applied per paint);
/// `family` resolves to a face through [`registered_family_name`].
#[derive(Clone, Debug, PartialEq)]
pub struct TextEditorStyle {
    pub weight: ambre_core::FontWeight,
    pub italic: ambre_core::Slant,
    pub family: ambre_core::FontFamily,
    pub base_size: ambre_core::DocPx,
    pub color: Color,
    /// Draw a 1px underline under the body (`TOILE-UI-187`). Pinned from the
    /// host element's style so the in-edit body matches the committed look.
    pub underline: ambre_core::Underline,
    /// Draw a 1px strikethrough through the body (`TOILE-UI-187`).
    pub strikethrough: ambre_core::Strikethrough,
}

/// Whether a draw of the editor shows its caret bar: the phase of the blink
/// the caller keeps on a timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caret {
    /// The bar is drawn.
    Shown,
    /// The bar is not drawn: the blink is in its off phase, or the editor is
    /// not focused.
    Hidden,
}

impl Caret {
    /// The caret shown when `visible` holds, hidden otherwise.
    #[must_use]
    pub const fn shown_if(visible: bool) -> Self {
        if visible { Self::Shown } else { Self::Hidden }
    }
}

/// One range of the buffer drawn in a look of its own, over the pinned style.
/// The host pins the spans as it pins the style, and the core keeps none of
/// its own (`CANVAS-042`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextSpan {
    /// The bytes of the buffer the span covers.
    pub range: ByteSpan,
    /// A weight in place of the pinned one.
    pub weight: Option<ambre_core::FontWeight>,
    pub italic: ambre_core::Slant,
    /// A family in place of the pinned one, resolved the same way.
    pub family: Option<ambre_core::FontFamily>,
    /// An ink in place of the pinned one.
    pub color: Option<Color>,
    pub underline: ambre_core::Underline,
    /// A tint drawn behind the range.
    pub background: Option<Color>,
    /// How far right each paragraph holding bytes of the range starts,
    /// before the zoom (`CANVAS-085`).
    pub indent: DocPx,
}

/// The buffer laid out with the spans, and the text it was laid out from.
struct Styled {
    text: String,
    layout: Layout<Brush>,
}

/// The selection the last line move over the spans left, with the column it
/// aims at, and the text and geometry it was read from.
struct LineGoal {
    text: String,
    geometry: (ViewPx, ZoomFactor),
    selection: Selection,
}

/// A shared inline text edit over one text unit (a Toile element body or a Trame
/// cell). App-specific concerns live in the host wrapper: which unit, how the
/// style / geometry is derived, how a commit is dispatched. This core owns only
/// the PlainEditor, its layout context, the intent queue and the pinned style.
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
    /// The spans the host pinned, laid out at each flush.
    spans: Vec<TextSpan>,
    /// The layout of the spans, `None` with no span.
    styled: Option<Styled>,
    /// The geometry of the last flush, which the layout of the spans reuses.
    wrap_w: ViewPx,
    zoom: ZoomFactor,
    align: Alignment,
    /// The column a run of line moves over the spans keeps (`CANVAS-080`).
    line_goal: Option<LineGoal>,
}

impl TextEditor {
    /// Open an editor seeded with `text` and the pinned `style`. The caret is
    /// placed at the end of the seeded body on the first flush (Trame parity).
    pub fn new(text: &str, style: TextEditorStyle) -> Self {
        let mut editor = PlainEditor::<Brush>::new(style.base_size.get());
        editor.set_text(text);
        editor.set_alignment(Alignment::Start);
        Self {
            editor,
            layout_cx: LayoutContext::new(),
            intents: Vec::new(),
            initial: text.to_owned(),
            seeded: false,
            style,
            spans: Vec::new(),
            styled: None,
            wrap_w: ViewPx::new(1.0),
            zoom: ZoomFactor::ONE,
            align: Alignment::Start,
            line_goal: None,
        }
    }

    /// Range the lines differently inside the wrap width. The default is
    /// [`Alignment::Start`]; a Toile connector caption centres itself on the
    /// arrow, so it asks for [`Alignment::Center`]. Set once at open: the flush
    /// re-applies the style and the geometry, never the alignment.
    pub fn set_alignment(&mut self, align: Alignment) {
        self.editor.set_alignment(align);
        self.align = align;
    }

    /// Re-pin the style (the host may recompute it per frame, for example on a font /
    /// size change). Applied at the next [`flush`](Self::flush).
    pub fn set_style(&mut self, style: TextEditorStyle) {
        self.style = style;
    }

    /// The pinned style, for the host and for tests.
    pub fn style(&self) -> &TextEditorStyle {
        &self.style
    }

    /// Pin the spans the buffer is drawn with, in place of those pinned
    /// before, and lay them out at the geometry of the last flush. A range
    /// past the buffer or off a char boundary draws nothing (`CANVAS-078`).
    pub fn set_spans(&mut self, font_cx: &mut FontContext, spans: Vec<TextSpan>) {
        self.spans = spans;
        self.lay_out_spans(font_cx);
    }

    /// The pinned spans, for the host and for tests.
    pub fn spans(&self) -> &[TextSpan] {
        &self.spans
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
            select: Extend::Move,
            word: Stride::Char,
        });
    }
    /// Move the caret right, collapsing any selection.
    pub fn move_right(&mut self) {
        self.push(EditIntent::Right {
            select: Extend::Move,
            word: Stride::Char,
        });
    }
    /// Extend the selection one place left.
    pub fn extend_left(&mut self) {
        self.push(EditIntent::Left {
            select: Extend::Select,
            word: Stride::Char,
        });
    }
    /// Extend the selection one place right.
    pub fn extend_right(&mut self) {
        self.push(EditIntent::Right {
            select: Extend::Select,
            word: Stride::Char,
        });
    }
    /// Move the caret to the start of the line.
    pub fn move_home(&mut self) {
        self.push(EditIntent::LineStart {
            select: Extend::Move,
        });
    }
    /// Move the caret to the end of the line.
    pub fn move_end(&mut self) {
        self.push(EditIntent::LineEnd {
            select: Extend::Move,
        });
    }
    /// Extend the selection to the start of the line.
    pub fn extend_home(&mut self) {
        self.push(EditIntent::LineStart {
            select: Extend::Select,
        });
    }
    /// Extend the selection to the end of the line.
    pub fn extend_end(&mut self) {
        self.push(EditIntent::LineEnd {
            select: Extend::Select,
        });
    }
    /// Move the caret up one line.
    pub fn move_up(&mut self) {
        self.push(EditIntent::Up {
            select: Extend::Move,
        });
    }
    /// Move the caret down one line.
    pub fn move_down(&mut self) {
        self.push(EditIntent::Down {
            select: Extend::Move,
        });
    }
    /// Extend the selection up one line.
    pub fn extend_up(&mut self) {
        self.push(EditIntent::Up {
            select: Extend::Select,
        });
    }
    /// Extend the selection down one line.
    pub fn extend_down(&mut self) {
        self.push(EditIntent::Down {
            select: Extend::Select,
        });
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
    pub fn point_down(&mut self, at: DocPoint) {
        self.push(EditIntent::PointerDown(at));
    }
    /// Extend the selection to a local-space point (drag / shift-click).
    pub fn point_drag(&mut self, at: DocPoint) {
        self.push(EditIntent::PointerDrag(at));
    }
    /// Select the word at a local-space point (double click).
    pub fn word_at(&mut self, at: DocPoint) {
        self.push(EditIntent::WordAt(at));
    }

    /// Replace the whole buffer synchronously (no font context needed), caret
    /// re-seeded to the end at the next flush. For a host that owns the text
    /// elsewhere and pushes it in wholesale. Trame's formula bar does this: it
    /// holds DOM focus and caret while the in-cell editor is only the render /
    /// commit vehicle. `initial` is left untouched, so
    /// [`committed_text`](Self::committed_text) still compares against the
    /// open-time value.
    pub fn set_text(&mut self, text: &str) {
        if self.text() != text {
            self.editor.set_text(text);
            self.seeded = false;
        }
    }

    /// Whether an intent waits for the next flush, so [`Self::text`] may not
    /// hold it yet.
    pub fn queued(&self) -> bool {
        !self.intents.is_empty()
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

    /// The body the editor was opened on. A host that merges the commit into a
    /// shared body names it as the common ancestor of the two edits.
    pub fn initial_text(&self) -> &str {
        &self.initial
    }

    /// The currently selected text, or `None` when the selection is collapsed
    /// (or an IME composition is active). Font-free: reads the buffer over the
    /// selection byte range, so it can be called synchronously inside a native
    /// clipboard `copy` / `cut` event.
    pub fn selected_text(&self) -> Option<String> {
        self.editor.selected_text().map(str::to_owned)
    }

    /// The bytes the selection covers, or `None` while it is collapsed or a
    /// composition holds. Font-free, and as of the last flush.
    pub fn selection(&self) -> Option<ByteSpan> {
        let selection = self.editor.raw_selection();
        let held = !selection.is_collapsed() && self.editor.raw_compose().is_none();
        held.then(|| ByteSpan::from_usize_range(selection.text_range()))
    }

    /// The byte offset of the caret, or `None` while a selection or a
    /// composition holds. Font-free, and as of the last flush.
    pub fn caret(&self) -> Option<ByteOffset> {
        let selection = self.editor.raw_selection();
        let collapsed = selection.is_collapsed() && self.editor.raw_compose().is_none();
        collapsed.then(|| ByteOffset::new(selection.focus().index()))
    }

    /// Push the pinned style + geometry onto the editor for this paint, then apply
    /// every queued intent through the driver. Must run before rendering so the
    /// layout reflects the latest edits. `wrap_w` is the wrap width in screen
    /// pixels; `zoom` bakes into the font size.
    pub fn flush(&mut self, font_cx: &mut FontContext, wrap_w: ViewPx, zoom: ZoomFactor) {
        // Re-apply style + geometry each frame: zoom (hence font size + wrap
        // width) can change mid-edit, and PlainEditor holds no camera.
        let base = self.base_styles(zoom);
        let styles = self.editor.edit_styles();
        for property in base {
            styles.insert(property);
        }
        let wrap_w = wrap_w.max(ViewPx::new(1.0));
        self.editor.set_width(Some(wrap_w.get()));
        (self.wrap_w, self.zoom) = (wrap_w, zoom);
        // The geometry may have moved, so the layout of the spans waits for
        // the next intent that reads it.
        self.styled = None;

        // Scope the driver so its borrow of `font_cx` / `layout_cx` ends before the
        // layout refresh below.
        {
            let intents = std::mem::take(&mut self.intents);
            if !self.seeded {
                // Open with the caret at the end of the seeded body (Trame parity).
                self.editor
                    .driver(font_cx, &mut self.layout_cx)
                    .move_to_text_end();
                self.seeded = true;
            }
            for intent in intents {
                if !self.spans.is_empty() && reads_geometry(&intent) {
                    self.apply_by_spans(font_cx, &intent);
                    continue;
                }
                let mut drv = self.editor.driver(font_cx, &mut self.layout_cx);
                match intent {
                    EditIntent::Insert(s) => drv.insert_or_replace_selection(&s),
                    EditIntent::Newline => drv.insert_or_replace_selection("\n"),
                    EditIntent::Backspace => drv.backdelete(),
                    EditIntent::Delete => drv.delete(),
                    EditIntent::DeleteSelection => drv.delete_selection(),
                    EditIntent::Left { select, word } => match (select, word) {
                        (Extend::Move, Stride::Char) => drv.move_left(),
                        (Extend::Select, Stride::Char) => drv.select_left(),
                        (Extend::Move, Stride::Word) => drv.move_word_left(),
                        (Extend::Select, Stride::Word) => drv.select_word_left(),
                    },
                    EditIntent::Right { select, word } => match (select, word) {
                        (Extend::Move, Stride::Char) => drv.move_right(),
                        (Extend::Select, Stride::Char) => drv.select_right(),
                        (Extend::Move, Stride::Word) => drv.move_word_right(),
                        (Extend::Select, Stride::Word) => drv.select_word_right(),
                    },
                    EditIntent::Up { select } => {
                        if select == Extend::Select {
                            drv.select_up()
                        } else {
                            drv.move_up()
                        }
                    }
                    EditIntent::Down { select } => {
                        if select == Extend::Select {
                            drv.select_down()
                        } else {
                            drv.move_down()
                        }
                    }
                    EditIntent::LineStart { select } => {
                        if select == Extend::Select {
                            drv.select_to_line_start()
                        } else {
                            drv.move_to_line_start()
                        }
                    }
                    EditIntent::LineEnd { select } => {
                        if select == Extend::Select {
                            drv.select_to_line_end()
                        } else {
                            drv.move_to_line_end()
                        }
                    }
                    EditIntent::SelectAll => drv.select_all(),
                    EditIntent::PointerDown(at) => drv.move_to_point(at.x.get(), at.y.get()),
                    EditIntent::PointerDrag(at) => {
                        drv.extend_selection_to_point(at.x.get(), at.y.get());
                    }
                    EditIntent::WordAt(at) => drv.select_word_at_point(at.x.get(), at.y.get()),
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
                    EditIntent::Replace { start, end, text } => {
                        // The driver selects nothing off a char boundary, so
                        // only a selection of the exact range is replaced.
                        drv.select_byte_range(start.get(), end.get());
                        if drv.editor.raw_selection().text_range() == (start.get()..end.get()) {
                            drv.insert_or_replace_selection(&text);
                        }
                    }
                }
            }
        }

        // Rebuild the layout before the caller renders. `edit_styles()` and
        // `set_width()` above mark the layout dirty every frame, but only a driver
        // operation refreshes it. On a frame with no intents (the common
        // steady-caret case) the layout would stay dirty. `try_layout()` /
        // `cursor_geometry()` would then return `None`, painting neither glyphs
        // nor caret. Refreshing here keeps both live every frame.
        self.editor.refresh_layout(font_cx, &mut self.layout_cx);
        self.lay_out_spans(font_cx);
    }

    /// The pinned style as layout properties, with the zoom baked into the
    /// size. The family resolves to the registered face the static render
    /// uses, so the in-edit body matches its committed look.
    fn base_styles(&self, zoom: ZoomFactor) -> [StyleProperty<'static, Brush>; 5] {
        let face = registered_family_name(Some(&self.style.family));
        [
            StyleProperty::FontSize(self.style.base_size.get() * zoom.get() as f32),
            StyleProperty::FontWeight(FontWeight::new(f32::from(self.style.weight.get()))),
            StyleProperty::FontStyle(if self.style.italic == ambre_core::Slant::Italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            }),
            StyleProperty::FontFamily(FontFamily::Single(FontFamilyName::Named(face.into()))),
            StyleProperty::Brush(Brush::Solid(self.style.color)),
        ]
    }

    /// Lay the buffer out again with the pinned spans, or drop that layout
    /// with no span. The composition keeps its underline (`CANVAS-040`).
    fn lay_out_spans(&mut self, font_cx: &mut FontContext) {
        self.styled = None;
        if self.spans.is_empty() {
            return;
        }
        let text = self.editor.raw_text().to_owned();
        let base = self.base_styles(self.zoom);
        let compose = self.editor.raw_compose().clone();
        let mut builder = self.layout_cx.ranged_builder(font_cx, &text, 1.0, true);
        for property in base {
            builder.push_default(property);
        }
        let mut indents = Vec::new();
        for span in &self.spans {
            let shape = SpanShape {
                range: span.range,
                weight: span.weight,
                italic: span.italic,
                family: span.family.clone(),
                underline: span.underline,
                indent: 0,
            };
            let placed = push_shape(&mut builder, &text, &shape);
            if placed && span.indent > DocPx::ZERO {
                indents.push((shape.range, span.indent.zoomed(self.zoom).get()));
            }
            if let Some(color) = span.color.filter(|_| placed) {
                builder.push(
                    StyleProperty::Brush(Brush::Solid(color)),
                    shape.range.to_usize_range(),
                );
            }
        }
        if let Some(compose) = compose {
            builder.push(StyleProperty::Underline(true), compose);
        }
        let mut layout = builder.build(&text);
        break_indented(&mut layout, &text, self.wrap_w.get(), &indents);
        layout.align(self.align, AlignmentOptions::default());
        self.styled = Some(Styled { text, layout });
    }

    /// Apply an intent whose outcome hangs on the geometry through the layout
    /// of the spans. A click then lands on the char drawn under it, and a line
    /// move follows the lines drawn (`CANVAS-078`).
    fn apply_by_spans(&mut self, font_cx: &mut FontContext, intent: &EditIntent) {
        let stale = self
            .styled
            .as_ref()
            .is_none_or(|styled| styled.text != self.editor.raw_text());
        if stale {
            self.lay_out_spans(font_cx);
        }
        let Some(styled) = &self.styled else {
            return;
        };
        let layout = &styled.layout;
        let held = laid_on(self.editor.raw_selection(), layout);
        // A line move from where the last one left keeps its column. Any other
        // move, an edit or a new geometry starts a fresh one (`CANVAS-080`).
        let geometry = (self.wrap_w, self.zoom);
        let goal = self.line_goal.take().filter(|goal| {
            let ends = |s: &Selection| (s.anchor().index(), s.focus().index());
            goal.text == styled.text
                && goal.geometry == geometry
                && ends(&goal.selection) == ends(&held)
        });
        let from = goal.map_or(held, |goal| goal.selection);
        let next = match *intent {
            EditIntent::Up { select } => from.previous_line(layout, select == Extend::Select),
            EditIntent::Down { select } => from.next_line(layout, select == Extend::Select),
            EditIntent::LineStart { select } => held.line_start(layout, select == Extend::Select),
            EditIntent::LineEnd { select } => held.line_end(layout, select == Extend::Select),
            EditIntent::PointerDown(at) => Selection::from_point(layout, at.x.get(), at.y.get()),
            EditIntent::PointerDrag(at) => held.extend_to_point(layout, at.x.get(), at.y.get()),
            EditIntent::WordAt(at) => Selection::word_from_point(layout, at.x.get(), at.y.get()),
            _ => return,
        };
        if matches!(intent, EditIntent::Up { .. } | EditIntent::Down { .. }) {
            self.line_goal = Some(LineGoal {
                text: styled.text.clone(),
                geometry,
                selection: next,
            });
        }
        let (anchor, focus) = (next.anchor().index(), next.focus().index());
        let mut drv = self.editor.driver(font_cx, &mut self.layout_cx);
        if anchor == focus {
            drv.move_to_byte(focus);
        } else {
            drv.select_byte_range(anchor, focus);
        }
    }

    /// The caret rectangle in local editor coordinates, for the transparent
    /// textarea pipe / IME anchor. Call after [`flush`](Self::flush).
    pub fn caret_local(&mut self) -> Option<BoundingBox> {
        let plain = self.editor.cursor_geometry(CARET_WIDTH.get())?;
        let Some(styled) = &self.styled else {
            return Some(plain);
        };
        let layout = &styled.layout;
        let focus = laid_on(self.editor.raw_selection(), layout).focus();
        Some(on_its_line(
            layout,
            focus.geometry(layout, CARET_WIDTH.get()),
        ))
    }

    /// The bytes of the line drawn around the caret: a wrapped line, not the
    /// text between two breaks. `None` while a selection or a composition
    /// holds, or before the first layout. Call after [`flush`](Self::flush).
    pub fn caret_line(&mut self) -> Option<ByteSpan> {
        self.caret()?;
        // The caret's own box, so a caret at a wrap reads the line it is
        // drawn on, whichever side of the wrap its affinity holds.
        let caret = self.caret_local()?;
        let mid = ((caret.y0 + caret.y1) / 2.0) as f32;
        let layout = self.layout()?;
        let line = layout
            .lines()
            .find(|line| mid < line.metrics().block_max_coord)
            .or_else(|| layout.lines().last())?;
        Some(ByteSpan::from_usize_range(line.text_range()))
    }

    /// The total laid-out text height in local (screen) pixels, or `None` before
    /// the first layout. Lets a host grow its box to fit wrapped lines. Call after
    /// [`flush`](Self::flush).
    pub fn layout_height(&self) -> Option<ViewPx> {
        let height = match &self.styled {
            Some(styled) => styled.layout.height(),
            None => self.editor.try_layout()?.height(),
        };
        Some(ViewPx::new(height))
    }

    /// The layout the editor draws: the spans' own while it holds spans, the
    /// plain one otherwise (`CANVAS-082`). A host draws beside the text from
    /// it. `None` before the first layout. Call after [`flush`](Self::flush).
    pub fn layout(&self) -> Option<&Layout<Brush>> {
        match &self.styled {
            Some(styled) => Some(&styled.layout),
            None => self.editor.try_layout(),
        }
    }

    /// Draw the selection band, glyphs and caret at `origin` (screen space). The
    /// editor's local coordinates are offset by `origin`. `selection` is the base
    /// selection tint, drawn at 30% alpha behind the text. `caret` gates the
    /// blinking caret bar, which the caller toggles on a timer. The caret takes
    /// the style colour, not a theme caret. The body ink is fixed, such as a
    /// sticky's dark ink on a light card, so a theme caret could be invisible
    /// against it.
    ///
    /// Everything is drawn in the editor's local (origin-relative) frame. A
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
        caret: Caret,
    ) {
        let dx = origin.x;
        let dy = origin.y;

        // The tint of each span, then the selection band, behind the text.
        // With spans, every geometry is read from their layout (`CANVAS-078`).
        let sel = Brush::Solid(selection.with_alpha(0.30));
        let band = match &self.styled {
            Some(styled) => {
                let layout = &styled.layout;
                for span in &self.spans {
                    let Some(tint) = span.background else {
                        continue;
                    };
                    let tint = Brush::Solid(tint);
                    for (bb, _line) in covered(layout, span.range).geometry(layout) {
                        surface.fill_path(&rect_path(offset_box(bb, dx, dy)), &tint);
                    }
                }
                laid_on(self.editor.raw_selection(), layout).geometry(layout)
            }
            None => self.editor.selection_geometry(),
        };
        for (bb, _line) in band {
            surface.fill_path(&rect_path(offset_box(bb, dx, dy)), &sel);
        }

        // Glyph runs, each in the ink of its own style.
        let brush = Brush::Solid(self.style.color);
        let layout = match &self.styled {
            Some(styled) => Some(&styled.layout),
            None => self.editor.try_layout(),
        };
        if let Some(layout) = layout {
            for line in layout.lines() {
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else {
                        continue;
                    };
                    let tx = dx + run.offset() as f64;
                    let ty = dy + run.baseline() as f64;
                    let ink = &run.style().brush;
                    surface.draw_glyphs(&run, ink, Affine::translate((tx, ty)));

                    // Underline decoration. parley underlines the IME preedit
                    // range itself, through a `StyleProperty::Underline` on the
                    // composing text. Drawing the run's underline here restores
                    // the IME preedit hint that `draw_glyphs` alone drops. Drawn
                    // as a thin filled rect (not `stroke_line`) so it follows a
                    // caller rotation like the glyphs and caret. Placed 2px below
                    // the baseline, matching the static cell decoration.
                    if run.style().underline.is_some() {
                        let adv = run.advance() as f64;
                        let uy = ty + 2.0;
                        let bar = Rect::new(tx, uy, tx + adv, uy + 1.0);
                        surface.fill_path(&rect_path(bar), ink);
                    }
                    // Pinned decorations from the host's own style
                    // (`TOILE-UI-187`): the committed body's underline and
                    // strikethrough, drawn in the body ink. Entering edit does
                    // not drop them.
                    let underlined = self.style.underline == ambre_core::Underline::Underlined;
                    let struck = self.style.strikethrough == ambre_core::Strikethrough::Struck;
                    if underlined || struck {
                        let adv = run.advance() as f64;
                        let font_size = run.run().font_size() as f64;
                        let mut bar_at = |ay: f64| {
                            let bar = Rect::new(tx, ay, tx + adv, ay + 1.0);
                            surface.fill_path(&rect_path(bar), &brush);
                        };
                        if underlined {
                            bar_at(ty + 2.0);
                        }
                        if struck {
                            bar_at(ty - font_size * 0.3);
                        }
                    }
                }
            }
        }

        // Caret bar (blinking: only on the "on" half of the cycle).
        if caret == Caret::Shown
            && let Some(bb) = self.caret_local()
        {
            surface.fill_path(&rect_path(offset_box(bb, dx, dy)), &brush);
        }
    }
}

/// Whether an intent's outcome hangs on where the text is drawn, so that with
/// spans it is read from their layout (`CANVAS-078`).
fn reads_geometry(intent: &EditIntent) -> bool {
    matches!(
        intent,
        EditIntent::Up { .. }
            | EditIntent::Down { .. }
            | EditIntent::LineStart { .. }
            | EditIntent::LineEnd { .. }
            | EditIntent::PointerDown(_)
            | EditIntent::PointerDrag(_)
            | EditIntent::WordAt(_)
    )
}

/// `selection`, held by the editor's own layout, laid on `layout`, a layout
/// of the same text.
fn laid_on(selection: &Selection, layout: &Layout<Brush>) -> Selection {
    let at = |cursor: Cursor| Cursor::from_byte_index(layout, cursor.index(), cursor.affinity());
    Selection::new(at(selection.anchor()), at(selection.focus()))
}

/// `caret`, a box `layout` gives a cursor, moved right by the start of the
/// line it is drawn on. Parley 0.12 leaves a line's start out of a caret,
/// though it draws, hits and selects the glyphs past it. An indented line
/// would draw its caret left of its text (`CANVAS-085`).
fn on_its_line(layout: &Layout<Brush>, caret: BoundingBox) -> BoundingBox {
    let mid = ((caret.y0 + caret.y1) / 2.0) as f32;
    let start = layout
        .lines()
        .find(|line| mid < line.metrics().block_max_coord)
        .or_else(|| layout.lines().last())
        .map_or(0.0, |line| f64::from(line.metrics().inline_min_coord));
    BoundingBox::new(caret.x0 + start, caret.y0, caret.x1 + start, caret.y1)
}

/// The selection that covers the bytes `range` of `layout`.
fn covered(layout: &Layout<Brush>, range: ByteSpan) -> Selection {
    Selection::new(
        Cursor::from_byte_index(layout, range.start.get(), Affinity::Downstream),
        Cursor::from_byte_index(layout, range.end.get(), Affinity::Upstream),
    )
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

    fn doc_point(x: f32, y: f32) -> DocPoint {
        DocPoint::new(ambre_core::DocPx::new(x), ambre_core::DocPx::new(y))
    }

    fn style() -> TextEditorStyle {
        TextEditorStyle {
            weight: ambre_core::FontWeight::REGULAR,
            italic: ambre_core::Slant::Upright,
            family: ambre_core::FontFamily::Sans,
            base_size: ambre_core::DocPx::new(16.0),
            color: Color::from_rgba8(0, 0, 0, 255),
            underline: ambre_core::Underline::Plain,
            strikethrough: ambre_core::Strikethrough::Plain,
        }
    }

    /// Drive the core in a native host test (system fonts, no browser). Char
    /// motion + edits are deterministic here, which is what the Trame parity
    /// corpus relies on (`ambre-trame/src/ui/grid` tests).
    fn left() -> EditIntent {
        EditIntent::Left {
            select: Extend::Move,
            word: Stride::Char,
        }
    }

    #[test]
    fn char_motion_then_insert_lands_at_caret() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("hello world", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE); // seeds caret at end
        ed.push(left());
        ed.push(left());
        ed.push(EditIntent::Insert("X".to_owned()));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.text(), "hello worXld");
    }

    #[test]
    fn backspace_and_delete_selection() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("abc", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.push(EditIntent::Backspace);
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.text(), "ab");
        ed.push(EditIntent::SelectAll);
        ed.push(EditIntent::DeleteSelection);
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.text(), "");
    }

    /// An edit that leaves the buffer as it opened commits nothing. The close
    /// path asks for a body and gets `None`, so no command is sent for a caret
    /// that only moved (`toile-connectors`, `toile-text-editing`).
    #[test]
    fn an_unchanged_buffer_commits_nothing() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("cost", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.push(left());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.committed_text(), None, "a caret move is not an edit");
        ed.push(EditIntent::Insert("!".to_owned()));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.committed_text().as_deref(), Some("cos!t"));
    }

    #[test]
    fn select_all_reports_whole_buffer() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("hello world", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.push(EditIntent::SelectAll);
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.selected_text().as_deref(), Some("hello world"));
    }

    #[test]
    fn newline_makes_multi_line_text() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("a", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.push(EditIntent::Newline);
        ed.push(EditIntent::Insert("b".to_owned()));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
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
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.push(EditIntent::Compose("abc".to_owned()));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);

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
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
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

    /// Recording moves nothing: the buffer reads the same until the flush
    /// (`CANVAS-049`).
    #[test]
    fn recording_an_action_leaves_the_buffer_alone() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("abc", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.push(EditIntent::Insert("X".to_owned()));
        ed.push(EditIntent::Backspace);
        ed.push(EditIntent::SelectAll);
        assert_eq!(ed.text(), "abc", "a recorded action applied itself early");
        assert_eq!(ed.selected_text(), None);
        assert_eq!(ed.committed_text(), None);
    }

    /// Every recorded action lands at paint time, in order: two inserts and a
    /// motion between them all show in the buffer after one flush
    /// (`CANVAS-047`).
    #[test]
    fn every_recorded_action_applies_at_the_flush() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("ab", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.push(EditIntent::Insert("1".to_owned()));
        ed.push(left());
        ed.push(left());
        ed.push(EditIntent::Insert("2".to_owned()));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.text(), "a2b1");
    }

    /// A replace swaps the exact bytes it names and leaves the caret after
    /// its text. A range off a char boundary or past the end changes nothing,
    /// and a selection reads no caret (`CANVAS-077`).
    #[test]
    fn a_replace_swaps_its_range_and_puts_the_caret_after_it() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("hé @bo x", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.caret().map(ByteOffset::get), Some("hé @bo x".len()));
        let replace = |start, end, text: &str| EditIntent::Replace {
            start: ByteOffset::new(start),
            end: ByteOffset::new(end),
            text: text.to_owned(),
        };
        ed.push(replace(4, 7, "@Bob"));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.text(), "hé @Bob x");
        assert_eq!(
            ed.caret().map(ByteOffset::get),
            Some(8),
            "the caret follows the new text"
        );
        ed.push(replace(2, 3, "!"));
        ed.push(replace(0, 99, "!"));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.text(), "hé @Bob x", "a broken range replaced text");
        ed.push(EditIntent::SelectAll);
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.caret(), None, "a selection is no caret");
    }

    /// The flush drains the queue: nothing is left behind, and a second flush
    /// with no new action changes nothing (`CANVAS-050`).
    #[test]
    fn a_flush_leaves_the_queue_empty() {
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("ab", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.push(EditIntent::Insert("X".to_owned()));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert!(ed.intents.is_empty(), "an action stayed in the queue");
        assert_eq!(ed.text(), "abX");
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.text(), "abX", "a second flush typed the key again");
    }

    /// Under a caller transform, the selection band and the caret are paths,
    /// so the transform carries them. No `fill_rect` is recorded, which maps a
    /// bounding box alone (`CANVAS-041`).
    #[test]
    fn the_caret_and_selection_are_drawn_as_paths() {
        use ambre_draw::testing::{DrawOp, RecordingSurface};
        let mut fcx = FontContext::default();
        let mut ed = TextEditor::new("abc", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.push(EditIntent::Left {
            select: Extend::Select,
            word: Stride::Char,
        });
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.selected_text().as_deref(), Some("c"));

        let selection = Color::from_rgba8(0, 0, 255, 255);
        let mut surface = RecordingSurface::new();
        surface.push_transform(Affine::rotate(0.5));
        ed.render(
            &mut surface,
            Point::new(10.0, 20.0),
            selection,
            Caret::Shown,
        );
        surface.pop_transform();

        let band = selection.with_alpha(0.30);
        let ink = style().color;
        let paths = |c: Color| {
            surface.count(|op| matches!(op, DrawOp::FillPath { color: Some(k) } if *k == c))
        };
        assert!(paths(band) >= 1, "the selection band is not a path");
        assert!(paths(ink) >= 1, "the caret is not a path");
        assert_eq!(
            surface.count(|op| matches!(op, DrawOp::FillRect { .. })),
            0,
            "a fill_rect maps only its bounding box under a rotation"
        );
    }

    /// The faces the suite ships, which the spans pick among.
    fn shipped() -> FontContext {
        const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        let faces = [
            "NotoSans-Regular.ttf",
            "NotoSans-Bold.ttf",
            "NotoSans-Italic.ttf",
            "DejaVuSansMono.ttf",
        ];
        crate::fonts::font_context_from_bytes(
            faces
                .iter()
                .map(|file| std::fs::read(format!("{DIR}{file}")).expect("a shipped face"))
                .map(ambre_core::FontBytes::new),
        )
    }

    fn span(range: std::ops::Range<usize>) -> TextSpan {
        TextSpan {
            range: ByteSpan::from_usize_range(range),
            ..TextSpan::default()
        }
    }

    /// The caret of the editor, in its local frame.
    fn caret_x(ed: &mut TextEditor) -> f64 {
        ed.caret_local().expect("a caret").x0
    }

    /// A bold span widens what it covers. The caret, a line move and a click
    /// all read where the span draws the text, not the plain layout
    /// (`CANVAS-078`).
    #[test]
    fn a_click_lands_on_the_char_a_span_draws_under_it() {
        let mut fcx = shipped();
        let mut ed = TextEditor::new(&"w".repeat(20), style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        let plain = caret_x(&mut ed);
        let bold = TextSpan {
            weight: Some(ambre_core::FontWeight::BOLD),
            ..span(0..20)
        };
        ed.set_spans(&mut fcx, vec![bold]);
        let wide = caret_x(&mut ed);
        assert!(wide > plain * 1.05, "the caret ignores the span");
        for _ in 0..4 {
            ed.move_left();
        }
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.caret().map(ByteOffset::get), Some(16));
        let sixteenth = ed.caret_local().expect("a caret");
        ed.move_end();
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.caret().map(ByteOffset::get), Some(20));
        assert!(
            (caret_x(&mut ed) - wide).abs() < 0.5,
            "End read the plain line"
        );
        let y = (sixteenth.y0 + sixteenth.y1) / 2.0;
        ed.point_down(doc_point(sixteenth.x0 as f32, y as f32));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(
            ed.caret().map(ByteOffset::get),
            Some(16),
            "the click read the plain layout"
        );
        ed.set_spans(&mut fcx, Vec::new());
        assert!(
            (caret_x(&mut ed) - sixteenth.x0 * plain / wide).abs() < 1.0,
            "with no span, the plain layout serves again"
        );
    }

    /// A span's indent starts the lines of its paragraph past it, zoomed
    /// with the text. The glyphs, a click and the caret all read them there
    /// (`CANVAS-085`).
    #[test]
    fn an_indented_paragraph_draws_and_puts_its_caret_past_the_indent() {
        let mut fcx = shipped();
        let mut ed = TextEditor::new("top\nheld", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::positive(2.0));
        let flat = ed.caret_local().expect("a caret");
        let indent = TextSpan {
            indent: DocPx::new(12.0),
            ..span(4..8)
        };
        ed.set_spans(&mut fcx, vec![indent]);
        let end = ed.caret_local().expect("a caret");
        assert!(
            (end.x0 - flat.x0 - 24.0).abs() < 0.5,
            "the zoomed indent moves the caret"
        );
        let y = (end.y0 + end.y1) / 2.0;
        ed.point_down(doc_point(25.0, y as f32));
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::positive(2.0));
        assert_eq!(
            ed.caret(),
            Some(ByteOffset::new(4)),
            "a click past the indent lands at the start"
        );
        assert!(
            (caret_x(&mut ed) - 24.0).abs() < 0.5,
            "the caret stands past the indent"
        );
        let starts: Vec<f32> = ed
            .layout()
            .expect("a layout")
            .lines()
            .flat_map(|line| line.items())
            .filter_map(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => Some(run.offset()),
                PositionedLayoutItem::InlineBox(_) => None,
            })
            .collect();
        let [top, held] = starts.as_slice() else {
            panic!("a run a line, got {starts:?}");
        };
        assert!(*top < 0.5 && (held - 24.0).abs() < 0.5, "got {starts:?}");
    }

    /// The layout handed out is the one the editor draws: the spans' own
    /// while it holds spans, the plain one otherwise (`CANVAS-082`).
    #[test]
    fn the_layout_handed_out_is_the_one_drawn() {
        let mut fcx = shipped();
        let mut ed = TextEditor::new(&"w".repeat(20), style());
        assert!(ed.layout().is_none(), "no layout before the first flush");
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        let plain = ed.layout().expect("a layout").full_width();
        let bold = TextSpan {
            weight: Some(ambre_core::FontWeight::BOLD),
            ..span(0..20)
        };
        ed.set_spans(&mut fcx, vec![bold]);
        let wide = ed.layout().expect("a layout").full_width();
        assert!(wide > plain * 1.05, "the plain layout over spans");
        ed.set_spans(&mut fcx, Vec::new());
        let back = ed.layout().expect("a layout").full_width();
        assert!((back - plain).abs() < 0.5, "the spans' layout with no span");
    }

    /// The caret's line is the one drawn: a wrapped line, not the text
    /// between two breaks. At a wrap, End reads the line it ended, and no
    /// caret reads no line (`CANVAS-083`).
    #[test]
    fn the_caret_line_is_the_line_drawn() {
        let mut fcx = shipped();
        let mut ed = TextEditor::new("wwww", style());
        assert_eq!(ed.caret_line(), None, "no line before the first flush");
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        let word = ed.layout().expect("a layout").full_width();
        ed.set_text("wwww wwww wwww");
        // Room for one word and its space, never two.
        ed.flush(&mut fcx, ViewPx::new(word * 1.5), ZoomFactor::ONE);
        assert_eq!(ed.layout().expect("a layout").len(), 3, "three lines drawn");
        assert_eq!(ed.caret().map(ByteOffset::get), Some(14));
        assert_eq!(
            ed.caret_line().map(ByteSpan::to_usize_range),
            Some(10..14),
            "the last line drawn"
        );
        ed.move_up();
        ed.flush(&mut fcx, ViewPx::new(word * 1.5), ZoomFactor::ONE);
        assert_eq!(
            ed.caret_line().map(ByteSpan::to_usize_range),
            Some(5..10),
            "the middle line"
        );
        ed.move_end();
        ed.flush(&mut fcx, ViewPx::new(word * 1.5), ZoomFactor::ONE);
        assert_eq!(
            ed.caret_line().map(ByteSpan::to_usize_range),
            Some(5..10),
            "End stays on its line"
        );
        ed.push(EditIntent::SelectAll);
        ed.flush(&mut fcx, ViewPx::new(word * 1.5), ZoomFactor::ONE);
        assert_eq!(ed.caret_line(), None, "a selection reads no line");
    }

    /// Over the spans, a run of line moves keeps its column through a short
    /// line, in one flush or across several. A move of another kind aims at
    /// a new column (`CANVAS-080`).
    #[test]
    fn a_line_move_over_spans_keeps_its_column() {
        let mut fcx = shipped();
        let ten = "w".repeat(10);
        let mut ed = TextEditor::new(&format!("{ten}\nw\n{ten}"), style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.set_spans(&mut fcx, vec![span(0..23)]);
        assert_eq!(ed.caret().map(ByteOffset::get), Some(23));
        ed.move_up();
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(
            ed.caret().map(ByteOffset::get),
            Some(12),
            "the short line holds the caret"
        );
        ed.move_up();
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(
            ed.caret().map(ByteOffset::get),
            Some(10),
            "the column survived the short line"
        );
        ed.move_down();
        ed.move_down();
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(
            ed.caret().map(ByteOffset::get),
            Some(23),
            "two moves in one flush keep it too"
        );
        // A move of another kind aims at its own column.
        for _ in 0..9 {
            ed.move_left();
        }
        ed.move_up();
        ed.move_up();
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.caret().map(ByteOffset::get), Some(1));
    }

    /// Each span draws its range in its own look. That is a face of its
    /// weight, its slant or its family, an ink, an underline and a tint
    /// (`CANVAS-079`).
    #[test]
    fn a_span_draws_its_range_in_its_own_look() {
        use ambre_draw::testing::{DrawOp, RecordingSurface};
        let mut fcx = shipped();
        let faces = |spans: Vec<TextSpan>, fcx: &mut FontContext| {
            let mut ed = TextEditor::new("ab cd", style());
            ed.flush(fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
            ed.set_spans(fcx, spans);
            let layout = &ed.styled.as_ref().expect("a layout of the spans").layout;
            let mut ids: Vec<u64> = Vec::new();
            for line in layout.lines() {
                for item in line.items() {
                    if let PositionedLayoutItem::GlyphRun(run) = item {
                        ids.push(run.run().font().data.id());
                    }
                }
            }
            ids.dedup();
            ids.len()
        };
        let bold = TextSpan {
            weight: Some(ambre_core::FontWeight::BOLD),
            ..span(0..2)
        };
        let italic = TextSpan {
            italic: ambre_core::Slant::Italic,
            ..span(0..2)
        };
        let code = TextSpan {
            family: Some(ambre_core::FontFamily::Mono),
            ..span(0..2)
        };
        for (look, spans) in [("weight", bold), ("slant", italic), ("family", code)] {
            assert_eq!(faces(vec![spans], &mut fcx), 2, "the {look} takes no face");
        }

        let (ink, tint) = (
            Color::from_rgba8(200, 0, 0, 255),
            Color::from_rgba8(0, 200, 0, 255),
        );
        let link = TextSpan {
            color: Some(ink),
            underline: ambre_core::Underline::Underlined,
            background: Some(tint),
            ..span(3..5)
        };
        let mut ed = TextEditor::new("ab cd", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        ed.set_spans(&mut fcx, vec![link]);
        let mut surface = RecordingSurface::new();
        ed.render(
            &mut surface,
            Point::new(0.0, 0.0),
            Color::from_rgba8(0, 0, 255, 255),
            Caret::Hidden,
        );
        let glyphs = |c: Color| {
            surface.count(|op| matches!(op, DrawOp::Glyphs { color: Some(k), .. } if *k == c))
        };
        let paths = |c: Color| {
            surface.count(|op| matches!(op, DrawOp::FillPath { color: Some(k) } if *k == c))
        };
        assert_eq!(glyphs(ink), 1, "the span's ink");
        assert_eq!(glyphs(style().color), 1, "the rest keeps the pinned ink");
        assert_eq!(paths(ink), 1, "the underline, in the span's ink");
        assert_eq!(paths(tint), 1, "the tint behind the span");
    }

    /// The spans are the host's: an edit moves none, and a range the edit
    /// left past the buffer draws nothing (`CANVAS-042`).
    #[test]
    fn an_edit_moves_no_span() {
        let mut fcx = shipped();
        let mut ed = TextEditor::new("abcd", style());
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        let pinned = vec![TextSpan {
            weight: Some(ambre_core::FontWeight::BOLD),
            ..span(2..4)
        }];
        ed.set_spans(&mut fcx, pinned.clone());
        ed.push(EditIntent::Backspace);
        ed.push(EditIntent::Backspace);
        ed.flush(&mut fcx, ViewPx::new(1000.0), ZoomFactor::ONE);
        assert_eq!(ed.text(), "ab");
        assert_eq!(ed.spans(), pinned, "the core moved a span");
        let mut surface = ambre_draw::testing::RecordingSurface::new();
        ed.render(
            &mut surface,
            Point::new(0.0, 0.0),
            style().color,
            Caret::Shown,
        );
        assert_eq!(ed.caret().map(ByteOffset::get), Some(2));
    }
}
