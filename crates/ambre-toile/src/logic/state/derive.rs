//! Read-only derivation for [`ToileState`](super::ToileState) (§9): turning the
//! CRDT + derived caches into the culled, z-ordered [`DisplayViewport`] the UI
//! thread paints. Split out of `state/mod.rs`; this impl block only reads
//! `self` and never mutates the document. Shared helpers come from
//! [`super::codec`].

use std::collections::HashSet;

use kurbo::{Affine, BezPath, Rect};
use loro::LoroMap;

use crate::logic::command::{
    DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ToileEvent,
};
use crate::logic::model::{
    ConnectorId, ElementId, ElementKind, ElementStyle, ShapeKind, StrokePoint,
};
use crate::logic::stroke::smooth_stroke;

use super::ToileState;
use super::codec::{
    bin_of, bool_of, doc_type_from_tag, f64_of, kind_from_tags, read_connector, str_of,
};

impl ToileState {
    pub(super) fn viewport_event(&self) -> ToileEvent {
        ToileEvent::ViewportData(self.derive_viewport())
    }

    /// Build the culled, z-ordered `DisplayViewport` for the current viewport.
    pub(super) fn derive_viewport(&self) -> DisplayViewport {
        let visible: HashSet<ElementId> = self.index.query_visible(&self.viewport).collect();
        let mut nodes = Vec::new();
        for node in &self.layers {
            match node {
                crate::logic::model::NodeId::Element(id) if visible.contains(id) => {
                    if let Some(de) = self.display_element(id) {
                        nodes.push(DisplayNode::Element(de));
                    }
                }
                crate::logic::model::NodeId::Connector(id) => {
                    if let Some(dc) = self.display_connector(id) {
                        nodes.push(DisplayNode::Connector(dc));
                    }
                }
                crate::logic::model::NodeId::Element(_) => {} // culled
            }
        }
        DisplayViewport {
            viewport: self.viewport,
            nodes,
        }
    }

    fn display_element(&self, id: &ElementId) -> Option<DisplayElement> {
        let em = self.element_map(id)?;
        // A group has no stored geometry: its bounds are the derived member union
        // (§2.F bis). Read them from the derived cache.
        let is_group = str_of(&em, "type") == "group";
        let (x, y, w, h) = if is_group {
            let r = self.bounds.get(id).copied().unwrap_or_default();
            (r.x0, r.y0, r.width(), r.height())
        } else {
            (
                f64_of(&em, "x"),
                f64_of(&em, "y"),
                f64_of(&em, "w"),
                f64_of(&em, "h"),
            )
        };
        let rot = f64_of(&em, "rot");
        let locked = bool_of(&em, "locked");
        let style = ElementStyle::from_bytes(&bin_of(&em, "style"));
        let type_tag = str_of(&em, "type");
        let shape_kind = ShapeKind::from_tag(&str_of(&em, "shape"));
        let kind = kind_from_tags(&type_tag, shape_kind);
        let bounds = Rect::new(x, y, x + w, y + h);
        // World transform: rotation about the AABB center.
        let center = bounds.center();
        let transform = Affine::translate(center.to_vec2())
            * Affine::rotate(rot)
            * Affine::translate(-center.to_vec2());
        let geom = self.geom_for(&em, kind, bounds);
        Some(DisplayElement {
            id: id.clone(),
            kind,
            transform,
            bounds,
            locked,
            style,
            geom,
        })
    }

    fn geom_for(&self, em: &LoroMap, kind: ElementKind, bounds: Rect) -> DisplayGeom {
        match kind {
            ElementKind::Shape(ShapeKind::Rect) => DisplayGeom::Rect,
            ElementKind::Shape(ShapeKind::Ellipse) => DisplayGeom::Ellipse,
            ElementKind::Shape(ShapeKind::Triangle) => DisplayGeom::Triangle,
            ElementKind::Shape(ShapeKind::Line) => {
                let mut p = BezPath::new();
                p.move_to((bounds.x0, bounds.y0));
                p.line_to((bounds.x1, bounds.y1));
                DisplayGeom::Path(p)
            }
            ElementKind::Shape(ShapeKind::Path) => {
                DisplayGeom::Path(BezPath::from_svg(&str_of(em, "path")).unwrap_or_default())
            }
            ElementKind::Image => DisplayGeom::Image(str_of(em, "hash")),
            ElementKind::Embed => DisplayGeom::Embed(str_of(em, "url")),
            ElementKind::DocEmbed => {
                // Parse back the reference; on any parse failure fall to Frame so
                // the node still paints a placeholder rather than vanishing.
                match (
                    ambre_core::Uuid::parse_str(&str_of(em, "doc_id")),
                    doc_type_from_tag(&str_of(em, "doc_type")),
                ) {
                    (Ok(document_id), Some(doc_type)) => DisplayGeom::DocEmbed {
                        document_id,
                        doc_type,
                        focused: false,
                    },
                    _ => DisplayGeom::Frame,
                }
            }
            ElementKind::Frame => DisplayGeom::Frame,
            ElementKind::Group => DisplayGeom::Group,
            ElementKind::Sticky | ElementKind::Text => DisplayGeom::Text(str_of(em, "text")),
            ElementKind::Drawing => {
                let pts: Vec<StrokePoint> =
                    postcard::from_bytes(&bin_of(em, "points")).unwrap_or_default();
                let mut out = Vec::new();
                smooth_stroke(&pts, &mut out);
                DisplayGeom::Stroke(out)
            }
        }
    }

    fn display_connector(&self, id: &ConnectorId) -> Option<DisplayConnector> {
        let cm = self.connector_map(id)?;
        let conn = read_connector(id.clone(), &cm)?;
        let path = conn.route(&self.bounds)?; // dropped if an endpoint is gone (§5.A)
        Some(DisplayConnector {
            id: id.clone(),
            path,
            style: ElementStyle::default(),
            arrows: conn.arrows,
        })
    }
}
