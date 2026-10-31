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
use ambre_core::Rgba;
use ambre_core::{Px, View};
use ambre_draw::{Alignment, BoundingBox, Color, DrawSurface, FontContext, Point};

// Re-export the shared intent type so the view keeps its `super::edit::EditIntent`
// import path (the enum lives in the shared core now).
pub use ambre_canvas::EditIntent;

use super::walk::{TEXT_PAD, TextHost};

/// What an inline edit is writing into: an element's body, or the caption of a
/// connector (`toile-connectors`). The two differ in where the box comes from
/// and which command the commit sends; everything between open and close is the
/// same editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    Element(ElementId),
    ConnectorLabel(ConnectorId),
    /// A frame's title, edited on the board itself (`TOILE-261`). Distinct from
    /// `Element`: a frame has no body, and what commits is a rename.
    FrameTitle(ElementId),
}

impl EditTarget {
    /// The element whose body is under edit, or `None` for a caption and for a
    /// frame's title. Neither is a body the content pass paints.
    pub fn element(&self) -> Option<&ElementId> {
        match self {
            Self::Element(id) => Some(id),
            Self::ConnectorLabel(_) | Self::FrameTitle(_) => None,
        }
    }

    /// The connector whose caption is under edit, or `None` when an element is.
    pub fn connector(&self) -> Option<&ConnectorId> {
        match self {
            Self::ConnectorLabel(id) => Some(id),
            Self::Element(_) | Self::FrameTitle(_) => None,
        }
    }

    /// The frame whose title is under edit, or `None` otherwise. The title pass
    /// reads it, so the band is not painted twice while it is edited.
    pub fn frame_title(&self) -> Option<&ElementId> {
        match self {
            Self::FrameTitle(id) => Some(id),
            Self::Element(_) | Self::ConnectorLabel(_) => None,
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
    /// Open an editor on `elem`, seeded with its current body `text`. A `Sticky`
    /// host switches to the sticky presentation. The body then uses the fixed readable
    /// ink: its `fill` is the card colour, not the text. The font size tracks the
    /// note size, matching the static render (`render.rs`).
    pub fn open(elem: &DisplayElement, text: &str, palette: &Palette, host: TextHost) -> Self {
        let font = &elem.style.font;
        // Text colour: same precedence as the static render so the in-edit
        // body matches its committed look. The element opacity is baked into
        // the alpha, as the committed pass does (`TOILE-UI-186`).
        let color = super::walk::resolved_text_ink(host, elem.style.text_color, elem.style.fill)
            .map(|c| ink_color(c, elem.style.opacity))
            .unwrap_or(palette.cell_text);
        // Size: the one the board draws, sticky auto-size and the free text
        // default included, so the body keeps its height under edit
        // (`TOILE-UI-188`).
        let base_size = super::walk::sticky_or_style_font_size(host, &elem.style, elem.bounds);
        let style = TextEditorStyle {
            weight: font.weight,
            italic: font.italic,
            family: font.face().clone(),
            base_size,
            color,
            underline: font.underline,
            strikethrough: font.strikethrough,
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
            family: font.face().clone(),
            base_size: super::label::LABEL_FONT_SIZE,
            color: palette.cell_text,
            underline: ambre_core::Underline::Plain,
            strikethrough: ambre_core::Strikethrough::Plain,
        };
        let mut core = TextEditor::new(text, style);
        core.set_alignment(Alignment::Center);
        Self {
            target: EditTarget::ConnectorLabel(id),
            core,
        }
    }

    /// Open the editor on a frame's title (`TOILE-261`). `base_size` is world
    /// units and the paint applies the zoom, so the caller hands the size the
    /// band reads at the current camera.
    pub fn open_frame_title(
        id: ElementId,
        title: &str,
        base_size: ambre_core::Px<ambre_core::Doc>,
        palette: &Palette,
    ) -> Self {
        let font = ElementStyle::default().font;
        let style = TextEditorStyle {
            weight: font.weight,
            italic: font.italic,
            family: font.face().clone(),
            base_size,
            color: palette.cell_text,
            underline: ambre_core::Underline::Plain,
            strikethrough: ambre_core::Strikethrough::Plain,
        };
        Self {
            target: EditTarget::FrameTitle(id),
            core: TextEditor::new(title, style),
        }
    }

    /// What is currently under edit.
    pub fn target(&self) -> &EditTarget {
        &self.target
    }

    /// The pinned style the editor applies, for tests.
    pub fn style(&self) -> &TextEditorStyle {
        self.core.style()
    }

    /// Inner padding in screen pixels at the given zoom: the walk's board-unit
    /// inset (`TEXT_PAD`) under the camera. The editor lays out in screen
    /// pixels, so its caret keeps one width at every zoom, and lands where the
    /// walk's body does.
    pub fn pad(
        zoom: ambre_core::Scale<ambre_core::Doc, ambre_core::View>,
    ) -> ambre_core::Px<ambre_core::Surface> {
        super::viewport::ViewportTransform::to_surface(TEXT_PAD, zoom)
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

    /// The body the editor opened on, which a body commit names as its base
    /// (`TOILE-UI-381`).
    pub fn opened_text(&self) -> &str {
        self.core.initial_text()
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
    pub fn flush(
        &mut self,
        font_cx: &mut FontContext,
        wrap_w: Px<View>,
        zoom: ambre_core::Scale<ambre_core::Doc, ambre_core::View>,
    ) {
        self.core.flush(font_cx, wrap_w, zoom);
    }

    /// The caret rectangle in local editor coordinates, for the transparent
    /// textarea pipe / IME anchor. Call after [`flush`](Self::flush).
    pub fn caret_local(&mut self) -> Option<BoundingBox> {
        self.core.caret_local()
    }

    /// Draw the selection band, glyphs and caret over the element (screen space).
    /// `origin` is the element's top-left screen corner + padding; the editor's
    /// local coordinates are offset by it. `caret` gates the blinking caret
    /// bar (the caller toggles it on a timer,). Call after [`flush`](Self::flush).
    pub fn render<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        origin: Point,
        palette: &Palette,
        caret: ambre_canvas::Caret,
    ) {
        self.core.render(surface, origin, palette.selection, caret);
    }
}

/// A text colour to a `Color`, pre-multiplying the element `opacity` into the
/// alpha channel (`TOILE-UI-186`), as the committed pass does.
fn ink_color(v: Rgba, opacity: ambre_core::Opacity) -> Color {
    super::walk::style_color(v, opacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{DisplayGeom, ElementKind, FontSpec};
    use ambre_canvas::Theme;
    use kurbo::Affine;

    fn elem(style: ElementStyle, kind: ElementKind) -> DisplayElement {
        DisplayElement {
            id: ElementId("a".to_owned()),
            kind,
            transform: Affine::IDENTITY,
            bounds: kurbo::Rect::new(0.0, 0.0, 120.0, 80.0),
            locked: ambre_bus::app::toile::LockState::Unlocked,
            style,
            geom: DisplayGeom::Text("hi".to_owned()),
            link: None,
            group: None,
        }
    }

    fn palette() -> &'static Palette {
        Theme::Light.palette()
    }

    /// `TOILE-261`: the editor opens on a frame's title, carrying the title it
    /// is about to replace. The frame is not an element under edit, because its
    /// body is not what is being typed. The content pass skips the band alone.
    #[test]
    fn the_editor_opens_on_a_frames_title() {
        let id = ElementId("f".to_owned());
        let editor = InlineEditor::open_frame_title(
            id.clone(),
            "Agenda",
            ambre_core::Px::<ambre_core::Doc>::new(14.0),
            palette(),
        );
        assert_eq!(editor.target().frame_title(), Some(&id));
        assert_eq!(editor.target().element(), None);
        assert_eq!(editor.target().connector(), None);
        assert_eq!(editor.style().base_size.get(), 14.0);
        // Nothing to commit until the user types: an edit that leaves the title
        // as it was writes no rename (`TOILE-250`).
        assert_eq!(editor.committed_text(), None);
    }

    /// `TOILE-262`: a frame nobody named opens on an empty title, never on the
    /// placeholder the band paints. An edit that types nothing commits nothing,
    /// so the placeholder never reaches the document.
    #[test]
    fn an_untouched_placeholder_commits_nothing() {
        let editor = InlineEditor::open_frame_title(
            ElementId("f".to_owned()),
            "",
            ambre_core::Px::<ambre_core::Doc>::new(14.0),
            palette(),
        );
        assert_eq!(editor.committed_text(), None);
    }

    /// `TOILE-UI-186` / `TOILE-UI-187`: the pinned style carries the committed
    /// opacity, underline and strikethrough, so entering edit drops none of them.
    #[test]
    fn the_pinned_style_carries_opacity_and_decorations() {
        let style = ElementStyle {
            text_color: Some(Rgba::from_bits(0xff00_00ff)),
            opacity: ambre_core::Opacity::new(0.5).unwrap(),
            font: FontSpec {
                underline: ambre_core::Underline::Underlined,
                strikethrough: ambre_core::Strikethrough::Struck,
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        let e = elem(style, ElementKind::Text);
        let editor = InlineEditor::open(&e, "hi", palette(), TextHost::Text);
        let pinned = editor.style();
        assert_eq!(pinned.underline, ambre_core::Underline::Underlined);
        assert_eq!(pinned.strikethrough, ambre_core::Strikethrough::Struck);
        // Opacity is baked into the ink alpha: opaque red at 0.5 -> alpha 128.
        assert_eq!(pinned.color.to_rgba8().a, 128);
    }

    /// `TOILE-UI-188`: a text element on an automatic size keeps under edit
    /// the size the board draws it at, the default 12 points.
    #[test]
    fn an_automatic_size_text_keeps_its_drawn_size_under_edit() {
        let style = ElementStyle {
            font: FontSpec {
                size: ambre_bus::app::toile::FontSizeChoice::AUTO,
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        let e = elem(style, ElementKind::Text);
        let drawn =
            super::super::walk::sticky_or_style_font_size(TextHost::Text, &e.style, e.bounds);
        let editor = InlineEditor::open(&e, "hi", palette(), TextHost::Text);
        assert_eq!(editor.style().base_size, drawn);
        assert_eq!(drawn, ambre_core::Px::<ambre_core::Doc>::new(16.0));
    }
}
