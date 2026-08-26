// ambre-toile/src/ui/edit.rs
// ============================================================================
// Inline text editor for sticky / text elements (toile-text-editing). A thin Toile
// wrapper over the shared `ambre_canvas::TextEditor` core (parley `PlainEditor`
// + font-free `EditIntent` queue, driven at paint time when a `FontContext` is
// live). Trame's cell editor wraps the same core.
//
// This wrapper owns only the Toile-specific concerns: which element is under
// edit (`ElementId`), and how the pinned style / size is derived from the
// element (sticky ink + auto-size, matching the static render in `render.rs`).
// Every editing operation delegates to the core.
// ============================================================================

use ambre_bus::app::toile::{DisplayElement, ElementId};
use ambre_canvas::{Palette, TextEditor, TextEditorStyle};
use ambre_draw::{BoundingBox, Color, DrawSurface, FontContext, Point};

// Re-export the shared intent type so the view keeps its `super::edit::EditIntent`
// import path (the enum lives in the shared core now).
pub use ambre_canvas::EditIntent;

/// Inner padding (screen px at zoom 1) between the element border and the text,
/// matching the sticky / text body inset.
const TEXT_PAD: f64 = 4.0;

/// An in-progress inline edit of one sticky / text element.
pub struct InlineEditor {
    /// The element being edited (its body is authoritative here until close).
    id: ElementId,
    /// The shared editor core (PlainEditor + intent queue + pinned style).
    core: TextEditor,
}

impl InlineEditor {
    /// Open an editor on `elem`, seeded with its current body `text`. `is_sticky`
    /// switches to the sticky presentation: the body uses the fixed readable ink
    /// (its `fill` is the card colour, not the text) and the font size tracks the
    /// note size, matching the static render (`render.rs`).
    pub fn open(elem: &DisplayElement, text: &str, palette: &Palette, is_sticky: bool) -> Self {
        let font = &elem.style.font;
        // Text colour: same precedence as the static render so the in-edit
        // body matches its committed look.
        let color =
            super::render::resolved_text_ink(is_sticky, elem.style.text_color, elem.style.fill)
                .map(argb_text)
                .unwrap_or(palette.cell_text);
        // Size: sticky auto-sizes from bounds unless overridden.
        let base_size =
            super::render::sticky_or_style_font_size(is_sticky, &elem.style, elem.bounds).max(1.0);
        let style = TextEditorStyle {
            weight: font.weight,
            italic: font.italic,
            family: font.family.clone(),
            base_size,
            color,
        };
        Self {
            id: elem.id.clone(),
            core: TextEditor::new(text, style),
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
        self.core.push(intent);
    }

    /// The committed body, or `None` when it is unchanged from open (no-op
    /// commit). IME preedit is excluded. Requires no font context.
    pub fn committed_text(&self) -> Option<String> {
        self.core.committed_text()
    }

    /// The currently selected text, or `None` when the selection is collapsed
    /// (or an IME composition is active). Font-free: safe inside a native
    /// clipboard `copy` / `cut` event.
    pub fn selected_text(&self) -> Option<String> {
        self.core.selected_text()
    }

    /// Push the pinned style + geometry onto the editor for this paint, then
    /// apply every queued intent. Must run before rendering. `wrap_w` is the wrap
    /// width in screen pixels; `zoom` bakes into the font size.
    pub fn flush(&mut self, font_cx: &mut FontContext, wrap_w: f32, zoom: f64) {
        self.core.flush(font_cx, wrap_w, zoom);
    }

    /// The caret rectangle in local editor coordinates, for the transparent
    /// textarea pipe / IME anchor. Call after [`flush`](Self::flush).
    pub fn caret_local(&mut self) -> Option<BoundingBox> {
        self.core.caret_local()
    }

    /// Draw the selection band, glyphs and caret over the element (screen space).
    /// `origin` is the element's top-left screen corner + padding; the editor's
    /// local coordinates are offset by it. `show_caret` gates the blinking caret
    /// bar (the caller toggles it on a timer,). Call after [`flush`](Self::flush).
    pub fn render<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        origin: Point,
        palette: &Palette,
        show_caret: bool,
    ) {
        self.core
            .render(surface, origin, palette.selection, show_caret);
    }
}

/// ARGB `u32` text color to an opaque `Color` (text ignores element opacity).
fn argb_text(v: u32) -> Color {
    let r = ((v >> 16) & 0xff) as u8;
    let g = ((v >> 8) & 0xff) as u8;
    let b = (v & 0xff) as u8;
    Color::from_rgba8(r, g, b, 255)
}
