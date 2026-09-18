// ambre-toile/src/ui/edit.rs
// ============================================================================
// Inline text editor for sticky / text elements (toile-text-editing). A thin Toile
// wrapper over the shared `ambre_canvas::TextEditor` core (parley `PlainEditor`
// + font-free `EditIntent` queue, driven at paint time when a `FontContext` is
// live). Trame's cell editor wraps the same core.
//
// This wrapper owns only the Toile-specific concerns. Those are which element is
// under edit (`ElementId`), and how the pinned style / size is derived from the
// element. The derivation is sticky ink + auto-size, matching the static render
// in `render.rs`. Every editing operation delegates to the core.
// ============================================================================

use ambre_bus::app::toile::{ConnectorId, DisplayElement, ElementId, ElementStyle};
use ambre_canvas::{Palette, TextEditor, TextEditorStyle};
use ambre_draw::{Alignment, BoundingBox, Color, DrawSurface, FontContext, Point};

// Re-export the shared intent type so the view keeps its `super::edit::EditIntent`
// import path (the enum lives in the shared core now).
pub use ambre_canvas::EditIntent;

/// Inner padding (screen px at zoom 1) between the element border and the text,
/// matching the sticky / text body inset.
const TEXT_PAD: f64 = 4.0;

/// What an inline edit is writing into: an element's body, or the caption of a
/// connector (`toile-connectors`). The two differ in where the box comes from
/// and which command the commit sends; everything between open and close is the
/// same editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    Element(ElementId),
    ConnectorLabel(ConnectorId),
}

impl EditTarget {
    /// The element under edit, or `None` when a caption is.
    pub fn element(&self) -> Option<&ElementId> {
        match self {
            Self::Element(id) => Some(id),
            Self::ConnectorLabel(_) => None,
        }
    }

    /// The connector whose caption is under edit, or `None` when an element is.
    pub fn connector(&self) -> Option<&ConnectorId> {
        match self {
            Self::ConnectorLabel(id) => Some(id),
            Self::Element(_) => None,
        }
    }
}

/// An in-progress inline edit of one sticky / text element, or of one connector
/// caption.
pub struct InlineEditor {
    /// What is being edited (its body is authoritative here until close).
    target: EditTarget,
    /// The shared editor core (PlainEditor + intent queue + pinned style).
    core: TextEditor,
}

impl InlineEditor {
    /// Open an editor on `elem`, seeded with its current body `text`. `is_sticky`
    /// switches to the sticky presentation. The body then uses the fixed readable
    /// ink: its `fill` is the card colour, not the text. The font size tracks the
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
            target: EditTarget::Element(elem.id.clone()),
            core: TextEditor::new(text, style),
        }
    }

    /// Open an editor on a connector's caption, seeded with `text`. A caption
    /// has no element to take a style from. It uses the default body font at a
    /// fixed world size and the theme's text ink. The words are centred in its
    /// box, so they sit on the arrow rather than beside it
    /// (`toile-connectors`).
    pub fn open_label(id: ConnectorId, text: &str, palette: &Palette) -> Self {
        let font = ElementStyle::default().font;
        let style = TextEditorStyle {
            weight: font.weight,
            italic: font.italic,
            family: font.family,
            base_size: super::label::LABEL_FONT_SIZE,
            color: palette.cell_text,
        };
        let mut core = TextEditor::new(text, style);
        core.set_alignment(Alignment::Center);
        Self {
            target: EditTarget::ConnectorLabel(id),
            core,
        }
    }

    /// What is currently under edit.
    pub fn target(&self) -> &EditTarget {
        &self.target
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
