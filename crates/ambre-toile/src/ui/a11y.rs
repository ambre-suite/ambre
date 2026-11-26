// ambre-toile/src/ui/a11y.rs
// ============================================================================
// What a screen reader hears of the board (`ambre-spec-accessibility.md`,
// `CHROME-023`). The focused unit of a board is its selection: one element
// is announced by its kind and its text, several by their count.
//
// Pure text -- host-tested, no `cfg(wasm32)`, no DOM. The view writes the
// result into the shared live region.
// ============================================================================

use crate::message::{DisplayElement, DisplayGeom, ElementKind, ShapeKind};
use ambre_bus::DocType;
use ambre_core::Validated;
use ambre_core::id::{Element, Id};
use ambre_core::index::Len;

/// The longest text an announcement carries. A sticky holding a page of notes
/// is announced by its start, not read whole on every click.
const TEXT_LIMIT: Len<ambre_core::Char> = Len::<ambre_core::Char>::new(120);

/// The announcement for `selection`, given the display data of its elements.
/// One element whose data has not arrived yet is not announced: the data
/// follows within a round trip, and the view announces then.
#[must_use]
pub fn selection_announcement(
    selection: &[Id<Element>],
    data: &[DisplayElement],
) -> Option<String> {
    match selection {
        [] => Some(ambre_commands::tr!("Nothing selected", "Aucune s\u{e9}lection").to_owned()),
        [id] => data.iter().find(|e| e.id == *id).map(element_announcement),
        many => Some(ambre_commands::trf!(
            "{} elements selected",
            "{} \u{e9}l\u{e9}ments s\u{e9}lectionn\u{e9}s",
            many.len()
        )),
    }
}

/// One element: its kind, its text when it carries one, and its lock.
fn element_announcement(element: &DisplayElement) -> String {
    let mut out = kind_name(element).to_owned();
    if let Some(text) = element_text(&element.geom).filter(|t| !t.trim().is_empty()) {
        out.push_str(", ");
        out.extend(text.trim().chars().take(TEXT_LIMIT.to_usize()));
    }
    if element.locked == crate::message::LockState::Locked {
        out.push_str(ambre_commands::tr!(", locked", ", verrouill\u{e9}"));
    }
    out
}

fn kind_name(element: &DisplayElement) -> &'static str {
    match element.kind {
        ElementKind::Sticky => ambre_commands::tr!("Sticky note", "Note adh\u{e9}sive"),
        ElementKind::Shape(ShapeKind::Rect) => ambre_commands::tr!("Rectangle", "Rectangle"),
        ElementKind::Shape(ShapeKind::Ellipse) => ambre_commands::tr!("Ellipse", "Ellipse"),
        ElementKind::Shape(ShapeKind::Triangle) => ambre_commands::tr!("Triangle", "Triangle"),
        ElementKind::Shape(ShapeKind::Line) => ambre_commands::tr!("Line", "Ligne"),
        ElementKind::Shape(ShapeKind::Path) => ambre_commands::tr!("Path", "Trac\u{e9}"),
        ElementKind::Text => ambre_commands::tr!("Text", "Texte"),
        ElementKind::Image => ambre_commands::tr!("Image", "Image"),
        ElementKind::Frame => ambre_commands::tr!("Frame", "Cadre"),
        ElementKind::Group => ambre_commands::tr!("Group", "Groupe"),
        ElementKind::Drawing => ambre_commands::tr!("Drawing", "Dessin"),
        ElementKind::Embed => ambre_commands::tr!("Web page", "Page web"),
        ElementKind::DocEmbed => match element.geom {
            DisplayGeom::DocEmbed {
                doc_type: DocType::Trame,
                ..
            } => ambre_commands::tr!("Embedded spreadsheet", "Classeur int\u{e9}gr\u{e9}"),
            DisplayGeom::DocEmbed {
                doc_type: DocType::Toile,
                ..
            } => ambre_commands::tr!("Embedded whiteboard", "Tableau blanc int\u{e9}gr\u{e9}"),
            _ => ambre_commands::tr!("Embedded document", "Document int\u{e9}gr\u{e9}"),
        },
    }
}

/// The text a reader is owed: a note's or a text's body, a frame's title, a
/// web page's address. The other kinds carry none.
fn element_text(geom: &DisplayGeom) -> Option<&str> {
    match geom {
        DisplayGeom::Sticky(s) | DisplayGeom::Text(s) | DisplayGeom::Frame(s) => Some(s),
        DisplayGeom::Embed(url) => url.as_ref().map(Validated::as_str),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ElementStyle;
    use ambre_draw::{Affine, Rect};

    fn element(
        n: u32,
        kind: ElementKind,
        geom: DisplayGeom,
        locked: crate::message::LockState,
    ) -> DisplayElement {
        DisplayElement {
            id: Id::<Element>::mint_from(
                ambre_core::PeerId::new(1),
                crate::message::IdCounter::new(n),
            ),
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

    /// `SETTINGS-052`: a word of the Toile chrome shows in the interface
    /// language of the page: the board announces its selection in French.
    #[test]
    fn a_board_announces_its_selection_in_the_interface_language() {
        let note = element(
            1,
            ElementKind::Sticky,
            DisplayGeom::Sticky("Lait".into()),
            crate::message::LockState::Locked,
        );
        let a = note.id.clone();
        let data = vec![note];
        ambre_commands::show_locale(ambre_commands::Locale::Fr);
        let french = selection_announcement(std::slice::from_ref(&a), &data);
        let none = selection_announcement(&[], &data);
        let many = selection_announcement(&[a.clone(), a.clone()], &data);
        let address = crate::message::AddressError::NotHttps.message();
        ambre_commands::show_locale(ambre_commands::Locale::En);
        assert_eq!(
            french.as_deref(),
            Some("Note adh\u{e9}sive, Lait, verrouill\u{e9}")
        );
        assert_eq!(none.as_deref(), Some("Aucune s\u{e9}lection"));
        assert_eq!(
            many.as_deref(),
            Some("2 \u{e9}l\u{e9}ments s\u{e9}lectionn\u{e9}s")
        );
        assert!(address.starts_with("Seules"), "{address}");
        assert_eq!(
            selection_announcement(std::slice::from_ref(&a), &data).as_deref(),
            Some("Sticky note, Lait, locked")
        );
    }

    /// `CHROME-023`: a board announces its selection. One element says its
    /// kind and its text, several say their count, and none says so.
    #[test]
    fn a_board_announces_its_selection() {
        let note = element(
            1,
            ElementKind::Sticky,
            DisplayGeom::Sticky(" Buy milk ".into()),
            crate::message::LockState::Unlocked,
        );
        let shape = element(
            2,
            ElementKind::Shape(ShapeKind::Ellipse),
            DisplayGeom::Ellipse,
            crate::message::LockState::Locked,
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
            crate::message::LockState::Unlocked,
        );
        let id = long.id.clone();
        assert_eq!(selection_announcement(std::slice::from_ref(&id), &[]), None);
        let said = selection_announcement(&[id], &[long]).unwrap();
        assert_eq!(said.len(), "Text, ".len() + TEXT_LIMIT.to_usize());
    }
}
