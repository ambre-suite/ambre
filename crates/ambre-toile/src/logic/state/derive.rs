//! Read-only derivation for [`ToileState`](super::ToileState): turning the
//! CRDT + derived caches into the culled, z-ordered [`DisplayViewport`] the UI
//! thread paints. Split out of `state/mod.rs`; this impl block only reads
//! `self` and never mutates the document. Shared helpers come from
//! [`super::codec`].

use std::collections::HashSet;

use kurbo::{Affine, BezPath, Rect};
use loro::LoroMap;

use crate::logic::animate::{FrameSnapshot, SnapElement};
use crate::logic::command::{
    DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ToileEvent,
    Viewport,
};
use crate::logic::connector::outline_meets;
use crate::logic::model::{
    ConnectorId, ElementId, ElementKind, ElementStyle, LinkTarget, ShapeKind, StrokePoint,
};
use crate::logic::stroke::smooth_stroke;

use super::ToileState;
use super::codec::{
    bin_of, body_of, bool_of, connector_label, doc_type_from_tag, f64_of, kind_from_tags,
    read_connector, str_of,
};

impl ToileState {
    pub(super) fn viewport_event(&self) -> ToileEvent {
        ToileEvent::ViewportData(self.derive_viewport())
    }

    /// Build the culled, z-ordered `DisplayViewport` for the current viewport.
    pub(super) fn derive_viewport(&self) -> DisplayViewport {
        self.derive_viewport_at(self.viewport)
    }

    /// Build the culled, z-ordered `DisplayViewport` for an **arbitrary** camera
    /// `vp`, without touching `self.viewport`. The presentation / PDF-export
    /// passes drive one board through a sequence of slide cameras. They must not
    /// mutate-then-restore `self.viewport`, which would race a concurrent
    /// `SetViewport` from the live UI thread. Read-only, so the same
    /// `ToileState` can be walked from any camera.
    pub(super) fn derive_viewport_at(&self, vp: Viewport) -> DisplayViewport {
        let visible: HashSet<ElementId> = self.index.query_visible(&vp).collect();
        let frame = vp.to_world_rect();
        let wires = self.wires();
        // An arrow is culled like an element (`TOILE-045`). The box of its
        // cached ends goes first, before the document is read. Then the
        // routed outline decides, since a diagonal's box overstates it
        // (`TOILE-207`). Its reach widens the frame, so a caption counts.
        let meets = |id: &ConnectorId| {
            wires.get(id).and_then(|w| {
                let wide = frame.inflate(w.reach, w.reach);
                let hull = w.conn.hull(&self.bounds)?;
                let path = hull.overlaps(wide).then(|| w.conn.route(&self.bounds))??;
                outline_meets(&path, wide).then_some(path)
            })
        };
        let mut nodes = Vec::new();
        // Every visible element the order lists, so the omitted remainder is the
        // difference, not a second pass over the element map (`TOILE-058`).
        // Borrowed from the order rather than cloned: this walk runs on every
        // camera, and an `ElementId` owns a heap string.
        let mut listed: HashSet<&ElementId> = HashSet::new();
        for node in &self.layers {
            match node {
                crate::logic::model::NodeId::Element(id) if visible.contains(id) => {
                    listed.insert(id);
                    if let Some(de) = self.display_element(id) {
                        nodes.push(DisplayNode::Element(de));
                    }
                }
                crate::logic::model::NodeId::Connector(id) => {
                    if let Some(path) = meets(id)
                        && let Some(dc) = self.display_connector(id, path)
                    {
                        nodes.push(DisplayNode::Connector(dc));
                    }
                }
                crate::logic::model::NodeId::Element(_) => {} // culled
            }
        }
        // An element the document holds but the order omits still paints, last,
        // in an order every peer computes alike: by id (`TOILE-058`, `TOILE-121`).
        // The order lists every visible element on a board no concurrent change
        // tore, so the common camera stops on the count and sorts nothing.
        if listed.len() < visible.len() {
            let mut omitted: Vec<&ElementId> =
                visible.iter().filter(|id| !listed.contains(*id)).collect();
            omitted.sort();
            for id in omitted {
                if let Some(de) = self.display_element(id) {
                    nodes.push(DisplayNode::Element(de));
                }
            }
        }
        DisplayViewport {
            version: self.viewport_version,
            viewport: vp,
            // One map lookup per frame: the register is board scaffolding,
            // and a peer can rewrite it at any time (`TOILE-UI-279`).
            grid_unit: f64_of(&self.doc.get_map(super::BOARD), "grid_size"),
            reactions: self.reaction_tallies(&visible),
            pins: self.comment_pins(&visible, &vp),
            nodes,
        }
    }

    /// Capture a frame's animatable set as a [`FrameSnapshot`] for Smart Animate
    /// (`toile-presentation`). The elements are the frame's **declared**
    /// `children`, membership being explicit, not spatial. Each carries the
    /// unrotated element rect, its rotation, and the style opacity / fill the
    /// solver interpolates. The rect is the raw `x/y/w/h` (not the
    /// rotation-expanded `bounds` cache) with rotation kept separate, mirroring
    /// [`display_element`](Self::display_element) so the tween matches what the
    /// renderer paints. A child whose element map is gone is skipped. Read-only:
    /// this derives the two snapshots the camera pass feeds to `smart_animate`.
    pub fn frame_snapshot(&self, frame: &ElementId) -> FrameSnapshot {
        let elements = self
            .children_of(frame)
            .into_iter()
            .filter_map(|id| {
                let em = self.element_map(&id)?;
                let style = self.style_read(&bin_of(&em, "style"));
                // A group carries no stored geometry: its rect is the derived
                // member-union AABB, read from the bounds cache.
                let bounds = if str_of(&em, "type") == "group" {
                    self.bounds.get(&id).copied().unwrap_or_default()
                } else {
                    let (x, y, w, h) = (
                        f64_of(&em, "x"),
                        f64_of(&em, "y"),
                        f64_of(&em, "w"),
                        f64_of(&em, "h"),
                    );
                    Rect::new(x, y, x + w, y + h)
                };
                Some(SnapElement {
                    id,
                    bounds,
                    rotation: f64_of(&em, "rot"),
                    opacity: style.opacity,
                    fill: style.fill,
                })
            })
            .collect();
        FrameSnapshot { elements }
    }

    /// The unrotated world rect (`x/y/w/h`, or the derived member union for a
    /// group) of `id`, or `None` if the element map is gone. Mirrors the geometry
    /// branch of [`display_element`](Self::display_element); used by the
    /// presentation camera pass as the fit target for a slide.
    pub(crate) fn element_rect(&self, id: &ElementId) -> Option<Rect> {
        let em = self.element_map(id)?;
        if str_of(&em, "type") == "group" {
            Some(self.bounds.get(id).copied().unwrap_or_default())
        } else {
            let (x, y, w, h) = (
                f64_of(&em, "x"),
                f64_of(&em, "y"),
                f64_of(&em, "w"),
                f64_of(&em, "h"),
            );
            Some(Rect::new(x, y, x + w, y + h))
        }
    }

    pub(super) fn display_element(&self, id: &ElementId) -> Option<DisplayElement> {
        let em = self.element_map(id)?;
        // A group has no stored geometry: its bounds are the derived member union
        // Read them from the derived cache.
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
        let style = self.style_read(&bin_of(&em, "style"));
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
            // The link register, absent on an element that carries none
            // (`toile-links`).
            link: self.link_read(&bin_of(&em, "link")),
            // The immediate enclosing group (if any): the UI resolves group-scope
            // selection by walking this edge upward.
            group: self.enclosing_group(id),
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
                    ambre_core::DocumentId::parse_str(&str_of(em, "doc_id")),
                    doc_type_from_tag(&str_of(em, "doc_type")),
                ) {
                    (Ok(document_id), Some(doc_type)) => DisplayGeom::DocEmbed {
                        document_id,
                        doc_type,
                        focused: false,
                    },
                    _ => DisplayGeom::Frame(String::new()),
                }
            }
            ElementKind::Frame => DisplayGeom::Frame(str_of(em, "name")),
            ElementKind::Group => DisplayGeom::Group,
            ElementKind::Sticky => DisplayGeom::Sticky(body_of(em)),
            ElementKind::Text => DisplayGeom::Text(body_of(em)),
            ElementKind::Drawing => {
                let raw = bin_of(em, "points");
                let pts: Vec<StrokePoint> = postcard::from_bytes(&raw).unwrap_or_else(|_| {
                    // A stroke this build cannot decode draws as no stroke at
                    // all, which is an absence the user is owed a word about
                    // (`SCHEMA-003`).
                    self.note_undecodable(&raw);
                    Vec::new()
                });
                let mut out = Vec::new();
                smooth_stroke(&pts, &mut out);
                DisplayGeom::Stroke(out)
            }
        }
    }

    /// Decode a `"style"` register, noting a blob this build cannot read
    /// (`SCHEMA-003`) before falling back to the default look.
    ///
    /// The fallback itself is unavoidable, since an unreadable register cannot be
    /// drawn as anything else. The rule is about the notice, not the pixels.
    pub(super) fn style_read(&self, bytes: &[u8]) -> ElementStyle {
        ElementStyle::try_from_bytes(bytes).unwrap_or_else(|| {
            self.note_undecodable(bytes);
            ElementStyle::default()
        })
    }

    /// Decode a `"link"` register the same way. An empty register is an element
    /// carrying no link, which is not a decode failure.
    fn link_read(&self, bytes: &[u8]) -> Option<LinkTarget> {
        if bytes.is_empty() {
            return None;
        }
        let link = LinkTarget::from_bytes(bytes);
        if link.is_none() {
            self.note_undecodable(bytes);
        }
        link
    }

    /// The arrow `id` as the UI paints it, along the outline the cull routed.
    pub(super) fn display_connector(
        &self,
        id: &ConnectorId,
        path: BezPath,
    ) -> Option<DisplayConnector> {
        let cm = self.connector_map(id)?;
        let conn = read_connector(id.clone(), &cm)?;
        Some(DisplayConnector {
            id: id.clone(),
            path,
            // A connector minted before the field existed reads back an empty
            // binary, which decodes to the default: the same look it had.
            style: self.style_read(&bin_of(&cm, "style")),
            arrows: conn.arrows,
            label: connector_label(&cm),
        })
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{Point, Size};

    use crate::logic::command::{
        DisplayNode, DisplayViewport, InitContent, ToileCommand, ToileEvent, Viewport,
    };
    use crate::logic::model::{ElementId, ElementKind, NodeId, ShapeKind};
    use crate::logic::state::ToileState;

    fn rect_at(x: f64, y: f64) -> ToileCommand {
        ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(x, y),
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: 0,
        }
    }

    fn create_rect(st: &mut ToileState, x: f64) -> ElementId {
        st.handle_command(rect_at(x, 0.0))
            .display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id),
                _ => None,
            })
            .expect("CreateElement must report the element it minted")
    }

    fn painted_elements(vp: &DisplayViewport) -> Vec<ElementId> {
        vp.nodes
            .iter()
            .filter_map(|n| match n {
                DisplayNode::Element(de) => Some(de.id.clone()),
                DisplayNode::Connector(_) => None,
            })
            .collect()
    }

    /// `derive_viewport_at` culls against the camera it is handed, not
    /// `self.viewport`, and leaves `self.viewport` untouched.
    #[test]
    fn derive_viewport_at_uses_the_given_camera() {
        let mut st = ToileState::new();
        st.handle_command(rect_at(0.0, 0.0)); // near origin
        st.handle_command(rect_at(5000.0, 0.0)); // far away

        // The live camera sits at the origin: only the near rect is visible.
        let here = st.derive_viewport();
        assert_eq!(here.nodes.len(), 1, "origin camera sees the near rect only");

        // Borrow a far camera without disturbing the live one.
        let far = Viewport {
            center: Point::new(5000.0, 0.0),
            ..Viewport::default()
        };
        let there = st.derive_viewport_at(far);
        assert_eq!(there.nodes.len(), 1, "far camera sees the far rect only");
        assert_eq!(
            there.viewport, far,
            "the returned viewport is the borrowed one"
        );

        // The live viewport is unchanged: no mutate-then-restore.
        assert_eq!(st.derive_viewport().viewport, here.viewport);
    }

    /// `TOILE-UI-279`: each frame carries the grid unit the board's register
    /// holds, the one the scaffold wrote and then the one a peer wrote over it.
    #[test]
    fn a_frame_carries_the_boards_grid_unit() {
        let st = ToileState::new();
        assert_eq!(st.derive_viewport().grid_unit, 32.0, "the scaffold's unit");
        let _ = st
            .doc
            .get_map(super::super::BOARD)
            .insert("grid_size", 50.0);
        assert_eq!(st.derive_viewport().grid_unit, 50.0, "a rewritten unit");
    }

    /// `TOILE-058` / `TOILE-121`: an element the document holds but the order
    /// omits still paints, last, in an order every peer computes alike (by id).
    #[test]
    fn an_omitted_element_paints_last_in_id_order() {
        let mut st = ToileState::new();
        let listed = create_rect(&mut st, 0.0);
        let omitted: Vec<ElementId> = (1..=4)
            .map(|i| create_rect(&mut st, f64::from(i) * 40.0))
            .collect();
        for id in &omitted {
            st.debug_remove_layer_entry(&NodeId::Element(id.clone()));
        }

        let painted = painted_elements(&st.debug_viewport());
        assert_eq!(painted.len(), 5, "the omitted elements still paint");
        assert_eq!(painted[0], listed, "the listed element keeps its place");
        // The omitted tail is sorted by id: the fallback is deterministic rather
        // than the element map's iteration order (`TOILE-121`).
        let mut tail = painted[1..].to_vec();
        tail.sort();
        assert_eq!(&painted[1..], &tail[..], "the omitted tail is in id order");
    }
}
