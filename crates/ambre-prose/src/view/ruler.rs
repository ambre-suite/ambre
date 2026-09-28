//! The ruler's state, published to the chrome
//! (`features/ambre-spec-prose-ruler.md section 3`).
//!
//! The ruler is a DOM element fixed above the page. This view knows where the
//! page column sits and which block holds the caret. It publishes a
//! [`RulerState`], and the element renders it. A drag lives in the element as
//! a [`RulerDrag`], which writes nothing. Its release comes back here as one
//! [`RulerRelease`] and leaves as one command (`PROSE-UI-063`,
//! `PROSE-UI-076`).

use crate::events::{ParagraphStyle, ProseCommand, tree_id};
use crate::layout::measure::PX_PER_PT;
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

/// What the ruler shows, rebuilt on every frame from the caret and the column.
///
/// The view publishes canvas pixels. The host converts them to CSS pixels
/// before the element sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RulerState {
    /// `false` hides the handles: no caret, or a caret in a block the page
    /// column does not measure.
    pub visible: bool,
    /// The top of the canvas, where the ruler sits.
    pub top_px: f32,
    /// Where the page's content column starts on screen.
    pub page_left_px: f32,
    /// How wide the content column is on screen.
    pub content_width_px: f32,
    /// How many screen pixels one point takes, at the zoom in force.
    pub px_per_pt: f32,
    /// The caret block's indents as the page lays it out, in points
    /// (`PROSE-UI-075`).
    pub indent_first: f32,
    pub indent_left: f32,
    pub indent_right: f32,
}

impl RulerState {
    /// Where a handle sits on screen.
    #[must_use]
    pub fn handle_x(&self, handle: RulerHandle) -> f32 {
        match handle {
            RulerHandle::IndentFirst => {
                self.page_left_px + (self.indent_left + self.indent_first) * self.px_per_pt
            }
            RulerHandle::IndentLeft => self.page_left_px + self.indent_left * self.px_per_pt,
            RulerHandle::IndentRight => {
                self.page_left_px + self.content_width_px - self.indent_right * self.px_per_pt
            }
        }
    }

    /// The indent a handle dropped at `x` stands for, in whole points. It
    /// stays inside the content column and never passes the other handle, so
    /// no drag leaves a paragraph without room for its text (`PROSE-UI-228`,
    /// `PROSE-UI-229`).
    #[must_use]
    pub fn points_at(&self, handle: RulerHandle, x: f32) -> f32 {
        if self.px_per_pt <= 0.0 || !x.is_finite() {
            return match handle {
                RulerHandle::IndentFirst => self.indent_first,
                RulerHandle::IndentLeft => self.indent_left,
                RulerHandle::IndentRight => self.indent_right,
            };
        }
        let column_pt = self.content_width_px / self.px_per_pt;
        let (raw, other) = match handle {
            // The first line counts from the left indent, and both side
            // indents narrow the room it has.
            RulerHandle::IndentFirst => (
                (x - self.page_left_px) / self.px_per_pt - self.indent_left,
                self.indent_left + self.indent_right,
            ),
            RulerHandle::IndentLeft => {
                ((x - self.page_left_px) / self.px_per_pt, self.indent_right)
            }
            RulerHandle::IndentRight => (
                (self.page_left_px + self.content_width_px - x) / self.px_per_pt,
                self.indent_left,
            ),
        };
        let room = (column_pt - other - MIN_TEXT_PT).max(0.0);
        raw.round().clamp(0.0, room.floor())
    }
}

/// The narrowest text a drag may leave between the two handles, in points.
const MIN_TEXT_PT: f32 = 36.0;

/// A handle in motion, held by the element. It paints the handle at the
/// pointer and writes nothing until [`RulerDrag::release`] (`PROSE-UI-076`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerDrag {
    pub handle: RulerHandle,
    /// Where the handle is painted, on screen.
    pub at_px: f32,
}

impl RulerDrag {
    /// A press on a handle.
    #[must_use]
    pub fn press(handle: RulerHandle, x: f32) -> Self {
        Self { handle, at_px: x }
    }

    /// The pointer moved: the handle follows it, and nothing else happens.
    pub fn move_to(&mut self, x: f32) {
        self.at_px = x;
    }

    /// The release: the one value the whole gesture stands for.
    #[must_use]
    pub fn release(self, ruler: &RulerState) -> RulerRelease {
        RulerRelease {
            handle: self.handle,
            points: ruler.points_at(self.handle, self.at_px),
        }
    }
}

/// A released handle, and the indent it set, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerRelease {
    pub handle: RulerHandle,
    pub points: f32,
}

impl ProseUiState {
    /// The ruler as it stands. The handles follow the block the caret is in
    /// (`PROSE-UI-064`) and show the indents it is laid out with, not its
    /// stored keys (`PROSE-UI-075`). The scale is the page's content column,
    /// at the zoom and the offset the column is drawn at (`PROSE-UI-065`).
    #[must_use]
    pub fn ruler(&self) -> RulerState {
        let layout = &self.shaped.layout;
        let scale = self.viewport.scale();
        let mut ruler = RulerState {
            visible: false,
            top_px: 0.0,
            page_left_px: self.column.left_px(&self.viewport) + layout.margin_left_px * scale,
            content_width_px: layout.content_width_px * scale,
            px_per_pt: PX_PER_PT * scale,
            indent_first: 0.0,
            indent_left: 0.0,
            indent_right: 0.0,
        };
        let style = self
            .cursor
            .as_ref()
            .map(|cursor| tree_id(cursor.block_id))
            .filter(|id| self.flow_index(id).is_some())
            .and_then(|id| self.shaped.blocks.get(&id))
            .and_then(|run| run.data().style().cloned());
        if let Some(style) = style {
            ruler.visible = true;
            // A hanging indent is laid out as none (`PROSE-FEAT-310`).
            ruler.indent_first = style.indent_first_px.max(0.0) / PX_PER_PT;
            ruler.indent_left = style.indent_left_px / PX_PER_PT;
            ruler.indent_right = style.indent_right_px / PX_PER_PT;
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
        if !ruler.visible || !release.points.is_finite() {
            return None;
        }
        let block_id = self.cursor.as_ref()?.block_id;
        let mut style = ParagraphStyle::default();
        match release.handle {
            RulerHandle::IndentFirst if release.points != ruler.indent_first => {
                style.indent_first = Some(release.points);
            }
            RulerHandle::IndentLeft if release.points != ruler.indent_left => {
                style.indent_left = Some(release.points);
            }
            RulerHandle::IndentRight if release.points != ruler.indent_right => {
                style.indent_right = Some(release.points);
            }
            _ => return None,
        }
        Some(ProseCommand::SetParagraphStyle { block_id, style })
    }
}

#[cfg(test)]
mod tests {
    use super::{RulerDrag, RulerHandle, RulerRelease};
    use crate::events::{BlockInit, CursorPosition, ParagraphStyle, ProseCommand};
    use crate::logic::ProseState;
    use crate::ui::ProseUiState;
    use crate::view::tests::{append, laid_out};

    /// A document of two paragraphs, the second indented, and a view on it.
    fn indented() -> (ProseState, ProseUiState, [crate::events::BlockId; 2]) {
        let (mut state, _) = laid_out(0, 0);
        let plain = append(&mut state, BlockInit::Paragraph, "plain text");
        let quoted = append(&mut state, BlockInit::Paragraph, "indented text");
        state.handle_command(ProseCommand::SetParagraphStyle {
            block_id: quoted,
            style: ParagraphStyle {
                indent_first: Some(24.0),
                indent_left: Some(36.0),
                indent_right: Some(18.0),
                ..ParagraphStyle::default()
            },
        });
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(state.shaped_layout());
        (state, ui, [plain, quoted])
    }

    fn caret_in(ui: &mut ProseUiState, block_id: crate::events::BlockId) {
        ui.cursor = Some(CursorPosition {
            block_id,
            offset: 0,
            anchor: None,
        });
    }

    /// `PROSE-UI-064`, `PROSE-UI-075`: the handles follow the caret's block,
    /// and show what the page lays it out with.
    #[test]
    fn the_handles_show_the_caret_blocks_laid_out_indents() {
        let (_, mut ui, [plain, quoted]) = indented();
        assert!(!ui.ruler().visible);
        caret_in(&mut ui, quoted);
        let ruler = ui.ruler();
        assert!(ruler.visible);
        assert!((ruler.indent_first - 24.0).abs() < 1e-3);
        assert!((ruler.indent_left - 36.0).abs() < 1e-3);
        assert!((ruler.indent_right - 18.0).abs() < 1e-3);
        // A block with no stored indent shows the zero it is laid out with.
        caret_in(&mut ui, plain);
        let ruler = ui.ruler();
        assert!(ruler.visible);
        assert_eq!(
            (ruler.indent_first, ruler.indent_left, ruler.indent_right),
            (0.0, 0.0, 0.0)
        );
    }

    /// `PROSE-UI-065`: the scale is the content column, wherever the column
    /// sits and at any zoom.
    #[test]
    fn the_scale_follows_the_column_at_any_zoom() {
        let (_, mut ui, [_, quoted]) = indented();
        caret_in(&mut ui, quoted);
        for zoom in [0.5, 1.0, 1.75] {
            ui.viewport.zoom = zoom;
            let ruler = ui.ruler();
            let layout = &ui.shaped.layout;
            let left = ui.column.left_px(&ui.viewport) + layout.margin_left_px * zoom;
            assert!((ruler.page_left_px - left).abs() < 1e-3);
            assert!((ruler.content_width_px - layout.content_width_px * zoom).abs() < 1e-3);
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
                assert!((back - value).abs() < 1e-3, "{handle:?} at zoom {zoom}");
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
            drag.move_to(ruler.page_left_px + step as f32);
        }
        drag.move_to(ruler.page_left_px + 72.0 * ruler.px_per_pt);
        // The motion touched nothing.
        assert_eq!(state.export_snapshot(), before);
        let release = drag.release(&ruler);
        assert_eq!(release.points, 72.0);
        let command = ui.ruler_release(release).expect("one command");
        assert_eq!(
            command,
            ProseCommand::SetParagraphStyle {
                block_id: quoted,
                style: ParagraphStyle {
                    indent_left: Some(72.0),
                    ..ParagraphStyle::default()
                },
            }
        );
        state.handle_command(command);
        ui.apply_layout(state.shaped_layout());
        assert!((ui.ruler().indent_left - 72.0).abs() < 1e-3);
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
        let column_pt = ruler.content_width_px / ruler.px_per_pt;
        let far_right = ruler.page_left_px + ruler.content_width_px * 3.0;
        let far_left = ruler.page_left_px - ruler.content_width_px;
        assert_eq!(ruler.points_at(RulerHandle::IndentLeft, far_left), 0.0);
        assert_eq!(ruler.points_at(RulerHandle::IndentRight, far_right), 0.0);
        let left = ruler.points_at(RulerHandle::IndentLeft, far_right);
        assert!(left + ruler.indent_right <= column_pt - 36.0 + 1e-3);
        let right = ruler.points_at(RulerHandle::IndentRight, far_left);
        assert!(right + ruler.indent_left <= column_pt - 36.0 + 1e-3);
        // The first line counts from the left indent, and stops as short.
        assert_eq!(ruler.points_at(RulerHandle::IndentFirst, far_left), 0.0);
        let first = ruler.points_at(RulerHandle::IndentFirst, far_right);
        assert!(first + ruler.indent_left + ruler.indent_right <= column_pt - 36.0 + 1e-3);
        // No caret, no command.
        ui.cursor = None;
        let release = RulerRelease {
            handle: RulerHandle::IndentLeft,
            points: 10.0,
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
        let expected = ruler.page_left_px + (36.0 + 24.0) * ruler.px_per_pt;
        assert!((ruler.handle_x(RulerHandle::IndentFirst) - expected).abs() < 1e-3);
        let mut drag = RulerDrag::press(RulerHandle::IndentFirst, expected);
        drag.move_to(ruler.page_left_px + (36.0 + 48.0) * ruler.px_per_pt);
        let command = ui.ruler_release(drag.release(&ruler));
        assert_eq!(
            command,
            Some(ProseCommand::SetParagraphStyle {
                block_id: quoted,
                style: ParagraphStyle {
                    indent_first: Some(48.0),
                    ..ParagraphStyle::default()
                },
            })
        );
    }
}
