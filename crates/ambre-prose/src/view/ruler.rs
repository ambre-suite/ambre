//! The ruler's state, published to the chrome
//! (`features/ambre-spec-prose-ruler.md section 3`).
//!
//! The ruler is a DOM element fixed above the page. This view knows where the
//! page column sits and which block holds the caret. It publishes a
//! [`RulerState`], and the element renders it. A drag lives in the element as
//! a [`RulerDrag`], which writes nothing. Its release comes back here as one
//! [`RulerRelease`] and leaves as one command (`PROSE-UI-063`,
//! `PROSE-UI-076`).

use ambre_core::Pt;
use ambre_core::{Doc, Px, View};

use crate::events::{ParagraphStyle, ProseCommand, tree_id};
use crate::ui::ProseUiState;

/// One handle of the ruler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulerHandle {
    /// Where the first line starts, from the left indent.
    IndentFirst,
    /// Where every line of the paragraph starts.
    IndentLeft,
    /// Where every line of the paragraph ends.
    IndentRight,
}

ambre_core::define_flag! {
    /// Whether the ruler shows its handles.
    pub enum RulerHandles {
        /// The handles are hidden.
        Hidden,
        /// The handles show the caret block's indents.
        Visible,
    }
}

/// What the ruler shows, rebuilt on every frame from the caret and the column.
///
/// The positions are in pixels of the view, from the canvas's corner. The
/// ruler sits on the canvas's top edge. The chrome moves it onto the page by
/// the canvas origin, and moves a pointer back by the same origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RulerState {
    /// Hidden with no caret, or with a caret in a block the page column
    /// does not measure.
    pub visible: RulerHandles,
    /// Where the page's content column starts on the canvas.
    pub page_left: Px<View>,
    /// How wide the content column is on the canvas.
    pub content_width: Px<View>,
    /// How wide one point is on the canvas, at the zoom the column is drawn at.
    pub point_width: Px<View>,
    /// The caret block's indents as the page lays it out (`PROSE-UI-075`).
    pub indent_first: Pt,
    pub indent_left: Pt,
    pub indent_right: Pt,
}

impl RulerState {
    /// A length in points, on the canvas.
    fn on_canvas(&self, length: Pt) -> Px<View> {
        self.point_width * length.get()
    }

    /// A length on the canvas, in points.
    fn in_points(&self, length: Px<View>) -> Pt {
        Pt::new(length / self.point_width)
    }

    /// Where a handle sits on the canvas.
    #[must_use]
    pub fn handle_x(&self, handle: RulerHandle) -> Px<View> {
        match handle {
            RulerHandle::IndentFirst => {
                self.page_left + self.on_canvas(self.indent_left + self.indent_first)
            }
            RulerHandle::IndentLeft => self.page_left + self.on_canvas(self.indent_left),
            RulerHandle::IndentRight => {
                self.page_left + self.content_width - self.on_canvas(self.indent_right)
            }
        }
    }

    /// The indent a handle dropped at `x` stands for, in whole points. It
    /// stays inside the content column and never passes the other handle, so
    /// no drag leaves a paragraph without room for its text (`PROSE-UI-228`,
    /// `PROSE-UI-229`).
    #[must_use]
    pub fn points_at(&self, handle: RulerHandle, x: Px<View>) -> Pt {
        let held = match handle {
            RulerHandle::IndentFirst => self.indent_first,
            RulerHandle::IndentLeft => self.indent_left,
            RulerHandle::IndentRight => self.indent_right,
        };
        let column = self.in_points(self.content_width);
        let (raw, other) = match handle {
            // The first line counts from the left indent, and both side
            // indents narrow the room it has.
            RulerHandle::IndentFirst => (
                self.in_points(x - self.page_left) - self.indent_left,
                self.indent_left + self.indent_right,
            ),
            RulerHandle::IndentLeft => (self.in_points(x - self.page_left), self.indent_right),
            RulerHandle::IndentRight => (
                self.in_points(self.page_left + self.content_width - x),
                self.indent_left,
            ),
        };
        // A pointer that is not a number, or a point of no width, leaves the
        // indent where it is.
        if !raw.is_finite() || !column.is_finite() {
            return held;
        }
        let room = (column - other - MIN_TEXT).max(Pt::ZERO);
        raw.round().clamp(Pt::ZERO, room.floor())
    }
}

/// The narrowest text a drag may leave between the two handles.
const MIN_TEXT: Pt = Pt::new(36.0);

/// A handle in motion, held by the element. It paints the handle at the
/// pointer and writes nothing until [`RulerDrag::release`] (`PROSE-UI-076`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerDrag {
    pub handle: RulerHandle,
    /// Where the handle is painted, on the canvas.
    pub at: Px<View>,
}

impl RulerDrag {
    /// A press on a handle.
    #[must_use]
    pub fn press(handle: RulerHandle, x: Px<View>) -> Self {
        Self { handle, at: x }
    }

    /// The pointer moved: the handle follows it, and nothing else happens.
    pub fn move_to(&mut self, x: Px<View>) {
        self.at = x;
    }

    /// The release: the one value the whole gesture stands for.
    #[must_use]
    pub fn release(self, ruler: &RulerState) -> RulerRelease {
        RulerRelease {
            handle: self.handle,
            points: ruler.points_at(self.handle, self.at),
        }
    }
}

/// A released handle, and the indent it set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerRelease {
    pub handle: RulerHandle,
    pub points: Pt,
}

impl ProseUiState {
    /// The ruler as it stands. The handles follow the block the caret is in
    /// (`PROSE-UI-064`) and show the indents it is laid out with, not its
    /// stored keys (`PROSE-UI-075`). The scale is the page's content column,
    /// at the zoom and the offset the column is drawn at (`PROSE-UI-065`).
    #[must_use]
    pub fn ruler(&self) -> RulerState {
        let layout = &self.shaped.layout;
        let zoom = self.viewport.zoom;
        let mut ruler = RulerState {
            visible: RulerHandles::Hidden,
            page_left: self.column.left(&self.viewport) + layout.margin_left.zoomed(zoom),
            content_width: layout.content_width.zoomed(zoom),
            point_width: Pt::new(1.0).to_doc_px().zoomed(zoom),
            indent_first: Pt::ZERO,
            indent_left: Pt::ZERO,
            indent_right: Pt::ZERO,
        };
        let style = self
            .cursor
            .as_ref()
            .map(|cursor| tree_id(cursor.block_id))
            .filter(|id| self.flow_index(id).is_some())
            .and_then(|id| self.shaped.blocks.get(&id))
            .and_then(|run| run.data().style().cloned());
        if let Some(style) = style {
            ruler.visible = RulerHandles::Visible;
            // A hanging indent is laid out as none (`PROSE-FEAT-310`).
            ruler.indent_first = style.indent_first.max(Px::<Doc>::ZERO).to_pt();
            ruler.indent_left = style.indent_left.to_pt();
            ruler.indent_right = style.indent_right.to_pt();
        }
        ruler
    }

    /// The one command a released handle stands for. It carries the caret
    /// block's property that moved and no other, so a concurrent change to
    /// another property survives (`PROSE-FEAT-025`). A release that moved
    /// nothing writes nothing (`PROSE-UI-063`).
    #[must_use]
    pub fn ruler_release(&self, release: RulerRelease) -> Option<ProseCommand> {
        let ruler = self.ruler();
        if ruler.visible == RulerHandles::Hidden || !release.points.is_finite() {
            return None;
        }
        let block_id = self.cursor.as_ref()?.block_id;
        let mut style = ParagraphStyle::default();
        let points = Some(release.points);
        match release.handle {
            RulerHandle::IndentFirst if release.points != ruler.indent_first => {
                style.indent_first = points;
            }
            RulerHandle::IndentLeft if release.points != ruler.indent_left => {
                style.indent_left = points;
            }
            RulerHandle::IndentRight if release.points != ruler.indent_right => {
                style.indent_right = points;
            }
            _ => return None,
        }
        Some(ProseCommand::SetParagraphStyle { block_id, style })
    }
}

#[cfg(test)]
mod tests {
    use super::{RulerDrag, RulerHandle, RulerHandles, RulerRelease, RulerState};
    use crate::events::{BlockInit, CursorPosition, ParagraphStyle, ProseCommand};
    use crate::logic::ProseState;
    use crate::ui::ProseUiState;
    use crate::view::tests::{append, laid_out};
    use ambre_bus::app::prose::DocChar;
    use ambre_core::Pt;
    use ambre_core::index::Pos;
    use ambre_core::{Px, View};

    /// A length in points, on the canvas.
    fn on_canvas(ruler: &RulerState, points: f32) -> Px<View> {
        ruler.point_width * points
    }

    /// A document of two paragraphs, the second indented, and a view on it.
    fn indented() -> (ProseState, ProseUiState, [crate::events::BlockId; 2]) {
        let (mut state, _) = laid_out(0, 0);
        let plain = append(&mut state, BlockInit::Paragraph, "plain text");
        let quoted = append(&mut state, BlockInit::Paragraph, "indented text");
        state.handle_command(ProseCommand::SetParagraphStyle {
            block_id: quoted,
            style: ParagraphStyle {
                indent_first: Some(ambre_core::Pt::new(24.0)),
                indent_left: Some(ambre_core::Pt::new(36.0)),
                indent_right: Some(ambre_core::Pt::new(18.0)),
                ..ParagraphStyle::default()
            },
        });
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(state.shaped_layout());
        (state, ui, [plain, quoted])
    }

    fn caret_in(ui: &mut ProseUiState, block_id: crate::events::BlockId) {
        ui.cursor = Some(CursorPosition {
            block_id,
            offset: Pos::<DocChar>::ZERO,
            anchor: None,
        });
    }

    /// `PROSE-UI-064`, `PROSE-UI-075`: the handles follow the caret's block,
    /// and show what the page lays it out with.
    #[test]
    fn the_handles_show_the_caret_blocks_laid_out_indents() {
        let (_, mut ui, [plain, quoted]) = indented();
        assert_eq!(ui.ruler().visible, RulerHandles::Hidden);
        caret_in(&mut ui, quoted);
        let ruler = ui.ruler();
        assert_eq!(ruler.visible, RulerHandles::Visible);
        assert!((ruler.indent_first.get() - 24.0).abs() < 1e-3);
        assert!((ruler.indent_left.get() - 36.0).abs() < 1e-3);
        assert!((ruler.indent_right.get() - 18.0).abs() < 1e-3);
        // A block with no stored indent shows the zero it is laid out with.
        caret_in(&mut ui, plain);
        let ruler = ui.ruler();
        assert_eq!(ruler.visible, RulerHandles::Visible);
        assert_eq!(
            (ruler.indent_first, ruler.indent_left, ruler.indent_right),
            (Pt::ZERO, Pt::ZERO, Pt::ZERO)
        );
    }

    /// `PROSE-UI-065`: the scale is the content column, wherever the column
    /// sits and at any zoom.
    #[test]
    fn the_scale_follows_the_column_at_any_zoom() {
        let (_, mut ui, [_, quoted]) = indented();
        caret_in(&mut ui, quoted);
        for zoom in [0.5, 1.0, 1.75] {
            ui.viewport.zoom =
                ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(f64::from(zoom));
            let ruler = ui.ruler();
            let layout = &ui.shaped.layout;
            let left = ui.column.left(&ui.viewport).get() + layout.margin_left.get() * zoom;
            assert!((ruler.page_left.get() - left).abs() < 1e-3);
            assert!((ruler.content_width.get() - layout.content_width.get() * zoom).abs() < 1e-3);
            // A handle placed at its value reads back the same value.
            for handle in [
                RulerHandle::IndentFirst,
                RulerHandle::IndentLeft,
                RulerHandle::IndentRight,
            ] {
                let back = ruler.points_at(handle, ruler.handle_x(handle));
                let value = match handle {
                    RulerHandle::IndentFirst => ruler.indent_first,
                    RulerHandle::IndentLeft => ruler.indent_left,
                    RulerHandle::IndentRight => ruler.indent_right,
                };
                assert!(
                    (back - value).abs().get() < 1e-3,
                    "{handle:?} at zoom {zoom}"
                );
            }
        }
    }

    /// `PROSE-UI-063`, `PROSE-UI-076`: a drag of any length is one command,
    /// sent on release, and moving the handle writes nothing.
    #[test]
    fn a_drag_of_any_length_writes_the_paragraph_once() {
        let (mut state, mut ui, [_, quoted]) = indented();
        caret_in(&mut ui, quoted);
        let ruler = ui.ruler();
        let before = state.export_snapshot();
        let mut drag = RulerDrag::press(
            RulerHandle::IndentLeft,
            ruler.handle_x(RulerHandle::IndentLeft),
        );
        for step in 0..500 {
            drag.move_to(ruler.page_left + Px::<View>::new(step as f32));
        }
        drag.move_to(ruler.page_left + on_canvas(&ruler, 72.0));
        // The motion touched nothing.
        assert_eq!(state.export_snapshot(), before);
        let release = drag.release(&ruler);
        assert_eq!(release.points, Pt::new(72.0));
        let command = ui.ruler_release(release).expect("one command");
        assert_eq!(
            command,
            ProseCommand::SetParagraphStyle {
                block_id: quoted,
                style: ParagraphStyle {
                    indent_left: Some(ambre_core::Pt::new(72.0)),
                    ..ParagraphStyle::default()
                },
            }
        );
        state.handle_command(command);
        ui.apply_layout(state.shaped_layout());
        assert!((ui.ruler().indent_left.get() - 72.0).abs() < 1e-3);
        // A release where the handle started writes nothing.
        let ruler = ui.ruler();
        let still = RulerDrag::press(
            RulerHandle::IndentLeft,
            ruler.handle_x(RulerHandle::IndentLeft),
        );
        assert_eq!(ui.ruler_release(still.release(&ruler)), None);
    }

    /// `PROSE-UI-228`, `PROSE-UI-229`: a handle stops inside the column and short of the
    /// other one.
    #[test]
    fn a_handle_stays_inside_the_column_and_short_of_the_other() {
        let (_, mut ui, [_, quoted]) = indented();
        caret_in(&mut ui, quoted);
        let ruler = ui.ruler();
        let column_width = Pt::new(ruler.content_width / ruler.point_width);
        let short = Pt::new(36.0 - 1e-3);
        let far_right = ruler.page_left + ruler.content_width * 3.0;
        let far_left = ruler.page_left - ruler.content_width;
        assert_eq!(ruler.points_at(RulerHandle::IndentLeft, far_left), Pt::ZERO);
        assert_eq!(
            ruler.points_at(RulerHandle::IndentRight, far_right),
            Pt::ZERO
        );
        let left = ruler.points_at(RulerHandle::IndentLeft, far_right);
        assert!(left + ruler.indent_right <= column_width - short);
        let right = ruler.points_at(RulerHandle::IndentRight, far_left);
        assert!(right + ruler.indent_left <= column_width - short);
        // The first line counts from the left indent, and stops as short.
        assert_eq!(
            ruler.points_at(RulerHandle::IndentFirst, far_left),
            Pt::ZERO
        );
        let first = ruler.points_at(RulerHandle::IndentFirst, far_right);
        assert!(first + ruler.indent_left + ruler.indent_right <= column_width - short);
        // No caret, no command.
        ui.cursor = None;
        let release = RulerRelease {
            handle: RulerHandle::IndentLeft,
            points: Pt::new(10.0),
        };
        assert_eq!(ui.ruler_release(release), None);
    }

    /// `PROSE-UI-230`: the first-line handle sits past the left indent, and
    /// its release writes the first-line indent alone.
    #[test]
    fn the_first_line_handle_writes_the_first_line_indent_alone() {
        let (_, mut ui, [_, quoted]) = indented();
        caret_in(&mut ui, quoted);
        let ruler = ui.ruler();
        let expected = ruler.page_left + on_canvas(&ruler, 36.0 + 24.0);
        assert!(
            (ruler.handle_x(RulerHandle::IndentFirst) - expected)
                .abs()
                .get()
                < 1e-3
        );
        let mut drag = RulerDrag::press(RulerHandle::IndentFirst, expected);
        drag.move_to(ruler.page_left + on_canvas(&ruler, 36.0 + 48.0));
        let command = ui.ruler_release(drag.release(&ruler));
        assert_eq!(
            command,
            Some(ProseCommand::SetParagraphStyle {
                block_id: quoted,
                style: ParagraphStyle {
                    indent_first: Some(ambre_core::Pt::new(48.0)),
                    ..ParagraphStyle::default()
                },
            })
        );
    }
}
