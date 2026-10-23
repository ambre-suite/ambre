// ambre-toile/src/ui/a11y.rs
// ============================================================================
// What a screen reader hears of the board (`ambre-spec-accessibility.md`,
// `CHROME-023`). The focused unit of a board is its selection: one element
// is announced by its kind and its text, several by their count.
//
// Pure text -- host-tested, no `cfg(wasm32)`, no DOM. The view writes the
// result into the shared live region.
// ============================================================================

use ambre_bus::DocType;
use ambre_bus::app::toile::{
    DisplayElement, DisplayGeom, ElementId, ElementKind, ShapeKind, WebAddress,
};

/// The longest text an announcement carries. A sticky holding a page of notes
/// is announced by its start, not read whole on every click.
const TEXT_LIMIT: usize = 120;

/// The announcement for `selection`, given the display data of its elements.
/// One element whose data has not arrived yet is not announced: the data
/// follows within a round trip, and the view announces then.
#[must_use]
pub fn selection_announcement(selection: &[ElementId], data: &[DisplayElement]) -> Option<String> {
    match selection {
        [] => Some("Nothing selected".to_owned()),
        [id] => data.iter().find(|e| e.id == *id).map(element_announcement),
        many => Some(format!("{} elements selected", many.len())),
    }
}

/// One element: its kind, its text when it carries one, and its lock.
fn element_announcement(element: &DisplayElement) -> String {
    let mut out = kind_name(element).to_owned();
    if let Some(text) = element_text(&element.geom).filter(|t| !t.trim().is_empty()) {
        out.push_str(", ");
        out.extend(text.trim().chars().take(TEXT_LIMIT));
    }
    if element.locked {
        out.push_str(", locked");
    }
    out
}

fn kind_name(element: &DisplayElement) -> &'static str {
    match element.kind {
        ElementKind::Sticky => "Sticky note",
        ElementKind::Shape(ShapeKind::Rect) => "Rectangle",
        ElementKind::Shape(ShapeKind::Ellipse) => "Ellipse",
        ElementKind::Shape(ShapeKind::Triangle) => "Triangle",
        ElementKind::Shape(ShapeKind::Line) => "Line",
        ElementKind::Shape(ShapeKind::Path) => "Path",
        ElementKind::Text => "Text",
        ElementKind::Image => "Image",
        ElementKind::Frame => "Frame",
        ElementKind::Group => "Group",
        ElementKind::Drawing => "Drawing",
        ElementKind::Embed => "Web page",
        ElementKind::DocEmbed => match element.geom {
            DisplayGeom::DocEmbed {
                doc_type: DocType::Trame,
                ..
            } => "Embedded spreadsheet",
            DisplayGeom::DocEmbed {
                doc_type: DocType::Toile,
                ..
            } => "Embedded whiteboard",
            _ => "Embedded document",
        },
    }
}

/// The text a reader is owed: a note's or a text's body, a frame's title, a
/// web page's address. The other kinds carry none.
fn element_text(geom: &DisplayGeom) -> Option<&str> {
    match geom {
        DisplayGeom::Sticky(s) | DisplayGeom::Text(s) | DisplayGeom::Frame(s) => Some(s),
        DisplayGeom::Embed(url) => url.as_ref().map(WebAddress::as_str),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::ElementStyle;
    use ambre_draw::{Affine, Rect};

    fn element(n: u32, kind: ElementKind, geom: DisplayGeom, locked: bool) -> DisplayElement {
        DisplayElement {
            id: ElementId::mint(ambre_core::PeerId(1), n),
            kind,
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            locked,
            style: ElementStyle::default(),
            geom,
            link: None,
            group: None,
        }
    }

    /// `CHROME-023`: a board announces its selection. One element says its
    /// kind and its text, several say their count, and none says so.
    #[test]
    fn a_board_announces_its_selection() {
        let note = element(
            1,
            ElementKind::Sticky,
            DisplayGeom::Sticky(" Buy milk ".into()),
            false,
        );
        let shape = element(
            2,
            ElementKind::Shape(ShapeKind::Ellipse),
            DisplayGeom::Ellipse,
            true,
        );
        let (a, b) = (note.id.clone(), shape.id.clone());
        let data = vec![note, shape];

        assert_eq!(
            selection_announcement(std::slice::from_ref(&a), &data).as_deref(),
            Some("Sticky note, Buy milk")
        );
        assert_eq!(
            selection_announcement(std::slice::from_ref(&b), &data).as_deref(),
            Some("Ellipse, locked")
        );
        assert_eq!(
            selection_announcement(&[a, b], &data).as_deref(),
            Some("2 elements selected")
        );
        assert_eq!(
            selection_announcement(&[], &data).as_deref(),
            Some("Nothing selected")
        );
    }

    /// One element whose data has not arrived is announced when it does, and
    /// a long body is cut rather than read whole.
    #[test]
    fn an_element_is_announced_once_its_data_arrives_and_never_whole() {
        let long = element(
            1,
            ElementKind::Text,
            DisplayGeom::Text("x".repeat(500)),
            false,
        );
        let id = long.id.clone();
        assert_eq!(selection_announcement(std::slice::from_ref(&id), &[]), None);
        let said = selection_announcement(&[id], &[long]).unwrap();
        assert_eq!(said.len(), "Text, ".len() + TEXT_LIMIT);
    }
}
