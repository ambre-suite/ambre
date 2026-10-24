// ambre-toile/src/ui/link.rs
// ============================================================================
// Following a link on an element (toile-links). The one board gesture that
// writes nothing. It reads a register the painted element carries, then moves a
// camera, opens a tab, or opens a document.
//
// The three decisions live here, pure, so the host tier runs them. They are
// whether a click is asking to follow, which element it is asking about, and
// what following that element means.
// ============================================================================

use ambre_bus::app::toile::{
    DisplayElement, DisplayNode, DisplayViewport, ElementId, LinkTarget, PresentMode,
};
use ambre_commands::Modifiers;
use ambre_core::{DocType, DocumentId};
use ambre_draw::{Point, Rect};

/// Side of the link badge, in screen pixels. Painted in screen space so it is
/// the same size at every zoom, like the selection handles.
pub const BADGE_PX: f64 = 14.0;

/// Gap between the badge and the element's top-right corner, in screen pixels.
const BADGE_GAP: f64 = 3.0;

/// What following a link does. None of the three writes to the document
/// (`TOILE-063`) or enters the undo history (`TOILE-115`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// Fit the camera to that frame's box (`TOILE-113`). The frame is named,
    /// not measured. It may sit outside the culled frame, which is exactly the
    /// case a link to it is for. So the board is asked for its bounds, and a
    /// frame it no longer holds answers nothing (`TOILE-064`).
    GoToFrame(ElementId),
    /// Hand the address to the shell, which opens it in a sandboxed tab. The
    /// board never fetches it (`TOILE-065`, `TOILE-110`).
    OpenUrl(ambre_core::LinkUrl),
    /// Ask the shell to open another document (`TOILE-114`).
    OpenDocument {
        document_id: DocumentId,
        doc_type: DocType,
    },
}

impl LinkAction {
    /// What this target means when followed.
    pub fn of(link: &LinkTarget) -> Self {
        match link {
            LinkTarget::Frame(id) => Self::GoToFrame(id.clone()),
            LinkTarget::Url(u) => Self::OpenUrl(u.clone()),
            LinkTarget::Document {
                document_id,
                doc_type,
            } => Self::OpenDocument {
                document_id: *document_id,
                doc_type: *doc_type,
            },
        }
    }
}

/// A navigation the board cannot perform itself: the shell opens a tab or
/// another document. The board never fetches either (`TOILE-110`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellNav {
    Url(ambre_core::LinkUrl),
    Document {
        document_id: DocumentId,
        doc_type: DocType,
    },
}

/// A link edit the inspector emitted: `Some` sets a target, `None` clears the
/// register. The view attaches the element the DOM panel cannot name, exactly
/// as it does for a style edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkEdit(pub Option<LinkTarget>);

impl LinkAction {
    /// The half of a follow that leaves the board, or `None` for a jump the
    /// camera performs here.
    pub fn shell_nav(self) -> Option<ShellNav> {
        match self {
            Self::GoToFrame(_) => None,
            Self::OpenUrl(u) => Some(ShellNav::Url(u)),
            Self::OpenDocument {
                document_id,
                doc_type,
            } => Some(ShellNav::Document {
                document_id,
                doc_type,
            }),
        }
    }
}

/// Whether a click is asking to follow rather than to select (`TOILE-112`).
/// A plain click with the select tool selects, always: a link that swallowed it
/// would make a linked element uneditable, with no refusal to observe. It
/// follows on a modifier, or on any click in presentation mode, where editing
/// is off.
pub fn wants_follow(mods: Modifiers, mode: PresentMode) -> bool {
    mods.primary || mode.is_presenting()
}

/// The badge of a linked element, in screen space: a small square just outside
/// its top-right corner.
pub fn badge_rect(screen_bounds: Rect) -> Rect {
    let x1 = screen_bounds.x1 + BADGE_GAP + BADGE_PX;
    let y0 = screen_bounds.y0 - BADGE_GAP - BADGE_PX;
    Rect::new(x1 - BADGE_PX, y0, x1, y0 + BADGE_PX)
}

/// The link of the top-most element under `world`, with its id. Locked and
/// unlinked elements are skipped; nodes are back-to-front, so the walk runs in
/// reverse. `pick` is the world reach of the screen pick floor.
pub fn link_at(
    display: &DisplayViewport,
    world: Point,
    pick: super::viewport::BoardLength,
) -> Option<(ElementId, LinkTarget)> {
    display
        .nodes
        .iter()
        .rev()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => Some(e),
            DisplayNode::Connector(_) => None,
        })
        .find(|e| e.link.is_some() && super::hit::exact_hit(e, world, pick))
        .and_then(|e| e.link.clone().map(|l| (e.id.clone(), l)))
}

/// Every linked element in the culled frame, for the badge pass.
pub fn linked(display: &DisplayViewport) -> impl Iterator<Item = &DisplayElement> {
    display.nodes.iter().filter_map(|n| match n {
        DisplayNode::Element(e) if e.link.is_some() => Some(e),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use ambre_bus::app::toile::{ElementKind, ElementStyle, ShapeKind, Viewport};
    use ambre_draw::Affine;

    use super::*;
    use ambre_bus::app::toile::DisplayGeom;

    fn elem(id: &str, bounds: Rect, link: Option<LinkTarget>) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: ElementId(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds,
            locked: false,
            style: ElementStyle::default(),
            geom: DisplayGeom::Rect,
            link,
            group: None,
        })
    }

    fn viewport(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            version: ambre_bus::ViewportVersion(0),
            grid_unit: 20.0,
            viewport: Viewport::default(),
            reactions: Default::default(),
            pins: Vec::new(),
            nodes,
        }
    }

    /// A plain click with the select tool selects; a modified one follows, and
    /// so does any click once editing is off (`TOILE-112`).
    #[test]
    fn a_plain_click_never_follows_while_editing() {
        assert!(
            !wants_follow(Modifiers::NONE, PresentMode::Editing),
            "a plain click selects"
        );
        assert!(
            wants_follow(Modifiers::PRIMARY, PresentMode::Editing),
            "a modified click follows"
        );
        assert!(
            wants_follow(Modifiers::NONE, PresentMode::Presenting),
            "presentation mode follows"
        );
    }

    /// The three targets each mean one thing, and the frame is named rather
    /// than measured.
    #[test]
    fn each_target_means_one_navigation() {
        let f = ElementId("frame".into());
        assert_eq!(
            LinkAction::of(&LinkTarget::Frame(f.clone())),
            LinkAction::GoToFrame(f)
        );
        assert_eq!(
            LinkAction::of(&LinkTarget::Url("https://example.org/".into())),
            LinkAction::OpenUrl("https://example.org/".into())
        );
        let id = DocumentId::new();
        assert_eq!(
            LinkAction::of(&LinkTarget::Document {
                document_id: id,
                doc_type: DocType::Trame,
            }),
            LinkAction::OpenDocument {
                document_id: id,
                doc_type: DocType::Trame,
            }
        );
    }

    /// A click resolves to the top-most linked element under it, and an
    /// unlinked one over a linked one hides nothing: the walk skips it.
    #[test]
    fn the_click_finds_the_top_most_linked_element() {
        let url = LinkTarget::Url("https://example.org/".into());
        let display = viewport(vec![
            elem(
                "under",
                Rect::new(0.0, 0.0, 100.0, 100.0),
                Some(url.clone()),
            ),
            elem("over", Rect::new(0.0, 0.0, 100.0, 100.0), None),
        ]);
        let hit = link_at(&display, Point::new(50.0, 50.0), crate::ui::hit::pick_floor(&crate::ui::viewport::ViewportTransform::default()));
        assert_eq!(
            hit,
            Some((ElementId("under".into()), url)),
            "an unlinked element on top does not swallow the link below it"
        );
        assert!(link_at(&display, Point::new(500.0, 500.0), crate::ui::hit::pick_floor(&crate::ui::viewport::ViewportTransform::default())).is_none());
    }

    /// A jump stays on the board; the other two leave it for the shell.
    #[test]
    fn only_the_outbound_halves_reach_the_shell() {
        assert_eq!(
            LinkAction::GoToFrame(ElementId("f".into())).shell_nav(),
            None
        );
        assert_eq!(
            LinkAction::OpenUrl("https://example.org/".into()).shell_nav(),
            Some(ShellNav::Url("https://example.org/".into()))
        );
    }

    /// The badge sits just outside the element's top-right corner and keeps its
    /// screen size.
    #[test]
    fn the_badge_sits_outside_the_top_right_corner() {
        let b = badge_rect(Rect::new(10.0, 20.0, 110.0, 120.0));
        assert_eq!(b.width(), BADGE_PX);
        assert_eq!(b.height(), BADGE_PX);
        assert!(b.x0 > 110.0, "to the right of the element");
        assert!(b.y1 < 20.0, "above the element");
    }
}
