//! Board -> file, the outbound half of `toile-import-export`.
//!
//! The SVG serializer is a pure function over a slice of
//! [`ExportElement`](crate::logic::interop::ExportElement) and nothing built that
//! slice from a live board, so no user could export anything. This is that
//! reader: it walks the paint order, projects each element to the export shape,
//! and hands the bytes back as one display event.
//!
//! Read-only by construction (`TOILE-IO-024`): every access here is a `get`, and
//! the outcome carries no CRDT update.

use kurbo::{BezPath, Rect};

use crate::logic::command::{ToileEvent, ToileOutcome};
use crate::logic::interop::{ExportElement, export_svg};
use crate::logic::model::{ElementId, ElementKind, ElementStyle, NodeId, ShapeKind};

use super::ToileState;
use super::codec::{bin_of, f64_of, kind_from_tags, str_of};

/// The name an SVG export carries. The board's own title is a launcher concern
/// (the Logic thread holds a document, not its listing entry), so the file is
/// named for what it holds.
const SVG_FILENAME: &str = "board.svg";

impl ToileState {
    /// Serialize the board to an SVG document (`TOILE-IO-021`). Emits one
    /// [`ExportComplete`](ToileEvent::ExportComplete) carrying the bytes, which
    /// the UI thread hands to the browser (`TOILE-IO-085`).
    ///
    /// An empty board still exports: a valid, empty SVG is an answer, and
    /// refusing one would leave the user wondering whether the gesture arrived.
    pub(super) fn export_svg_document(&self) -> ToileOutcome {
        let svg = export_svg(&self.export_elements());
        ToileOutcome::display_only(vec![ToileEvent::ExportComplete {
            filename: SVG_FILENAME.to_owned(),
            data: svg.into_bytes(),
        }])
    }

    /// Every element of the board, in paint order, as the serializer's shape.
    /// Connectors are not elements and do not appear here; a group carries no
    /// geometry of its own and the serializer skips it, so its members export
    /// flat -- both are gaps `TOILE-IO-025` records.
    fn export_elements(&self) -> Vec<ExportElement> {
        self.layer_order()
            .iter()
            .filter_map(|node| match node {
                NodeId::Element(id) => self.export_element(id),
                NodeId::Connector(_) => None,
            })
            .collect()
    }

    /// One element, read from the CRDT exactly as the display pass reads it: the
    /// stored box for anything with geometry, the derived union for a group.
    fn export_element(&self, id: &ElementId) -> Option<ExportElement> {
        let em = self.element_map(id)?;
        let type_tag = str_of(&em, "type");
        let kind = kind_from_tags(&type_tag, ShapeKind::from_tag(&str_of(&em, "shape")));
        let rect = if type_tag == "group" {
            self.bounds.get(id).copied()?
        } else {
            let (x, y) = (f64_of(&em, "x"), f64_of(&em, "y"));
            Rect::new(x, y, x + f64_of(&em, "w"), y + f64_of(&em, "h"))
        };
        Some(ExportElement {
            kind,
            rect,
            rotation: f64_of(&em, "rot"),
            style: ElementStyle::from_bytes(&bin_of(&em, "style")),
            path: match kind {
                ElementKind::Shape(ShapeKind::Path) => BezPath::from_svg(&str_of(&em, "path")).ok(),
                _ => None,
            },
            text: match kind {
                ElementKind::Text | ElementKind::Sticky => Some(str_of(&em, "text")),
                _ => None,
            },
            media_hash: match kind {
                ElementKind::Image => Some(str_of(&em, "hash")),
                _ => None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::model::{ElementKind, ShapeKind};

    use super::super::ToileState;

    fn board_with_a_rect() -> ToileState {
        let mut st = ToileState::new();
        st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(10.0, 20.0),
            size: Size::new(30.0, 40.0),
            content: InitContent::None,
            fill: None,
        });
        st
    }

    fn exported(st: &mut ToileState) -> (String, Vec<u8>) {
        let out = st.handle_command(ToileCommand::ExportSvg);
        assert!(
            out.update.is_none(),
            "an export must emit no CRDT delta (`TOILE-IO-024`)"
        );
        out.display
            .iter()
            .find_map(|e| match e {
                ToileEvent::ExportComplete { filename, data } => {
                    Some((filename.clone(), data.clone()))
                }
                _ => None,
            })
            .expect("the export answers with the file")
    }

    /// The board reaches the serializer: an element on the board is an element in
    /// the file, at the coordinates it holds (`TOILE-IO-021`).
    #[test]
    fn the_board_exports_as_an_svg_document() {
        let mut st = board_with_a_rect();
        let (name, data) = exported(&mut st);
        let svg = String::from_utf8(data).expect("the document is text");
        assert_eq!(name, "board.svg");
        assert!(svg.starts_with("<svg "), "a standalone document: {svg}");
        assert!(
            svg.contains("<rect x=\"10\" y=\"20\" width=\"30\" height=\"40\""),
            "the rect leaves at its own place: {svg}"
        );
    }

    /// Reading the board twice yields the same bytes, and leaves the board as it
    /// was (`TOILE-IO-004`, `TOILE-IO-024`).
    #[test]
    fn exporting_twice_yields_the_same_bytes_and_changes_nothing() {
        let mut st = board_with_a_rect();
        let before = st.element_count();
        let (_, first) = exported(&mut st);
        let (_, second) = exported(&mut st);
        assert_eq!(first, second);
        assert_eq!(st.element_count(), before);
    }

    /// An empty board still answers with a file: a gesture that produces nothing
    /// leaves the user unable to tell it arrived.
    #[test]
    fn an_empty_board_exports_an_empty_document() {
        let mut st = ToileState::new();
        let (_, data) = exported(&mut st);
        let svg = String::from_utf8(data).expect("the document is text");
        assert!(svg.starts_with("<svg "), "{svg}");
        assert!(svg.ends_with("</svg>\n"), "{svg}");
    }
}
