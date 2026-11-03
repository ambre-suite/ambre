//! Vector boolean operations for [`ToileState`](super::ToileState)
//! (`toile-vector-paths`): union / subtract / intersect / exclude over the
//! selected shape / path elements, producing one new `shape:path` element.
//!
//! The geometry math is the pure [`boolean_op`](crate::logic::boolean::boolean_op)
//! solver. This handler only resolves the selection to world-space paths, writes
//! the result element, and deletes the sources unless the caller keeps them. It
//! does all of that in a **single commit**, so the whole operation is one undo
//! item. A boolean is an ordinary geometry edit: the result is a plain element, no
//! special CRDT state (`toile-elements`).

use kurbo::{Affine, BezPath, Ellipse, Shape};
use loro::{LoroMap, LoroValue};

use crate::logic::boolean::{FLATTEN_TOL, boolean_op};
use crate::logic::command::{BoolOp, Sources, ToileError, ToileEvent, ToileOutcome};
use ambre_core::id::{Element, Id};

use crate::logic::model::{ElementKind, ElementStyle, NodeId, ShapeKind};

use super::ToileState;
use super::codec::has_type;
use super::codec::{
    bin_of, locked, path_of, rotation_of, shape_of, store_corner, store_size, stored_rect,
};
use super::names::{ElementKey, Fields, Root};

impl ToileState {
    /// Combine the selected shape / path elements with `op`. Needs at least
    /// two eligible (unlocked shape / path) elements; other kinds and locked
    /// elements are ignored. The result is one new `shape:path` element carrying
    /// the combined geometry and the first source's style. When `sources` is
    /// `Consume` the inputs are deleted in the same commit. A fully cancelled
    /// (empty) result is a rejected no-op.
    pub(super) fn boolean_op(
        &mut self,
        op: BoolOp,
        ids: &[Id<Element>],
        keep: Sources,
    ) -> ToileOutcome {
        // Resolve the eligible sources to world-space paths, in selection order.
        let mut paths = Vec::new();
        let mut sources = Vec::new();
        let mut style_bytes: Option<Vec<u8>> = None;
        for id in ids {
            let Some(em) = self.element_map(id) else {
                continue;
            };
            if locked(&em) {
                continue;
            }
            let Some(path) = element_world_path(&em) else {
                continue; // not a shape / path element
            };
            if style_bytes.is_none() {
                style_bytes = bin_of(&em, ElementKey::Style);
            }
            paths.push(path);
            sources.push(id.clone());
        }
        if paths.len() < 2 {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::BooleanNeedsTwo,
            )]);
        }

        let result = boolean_op(op, &paths, FLATTEN_TOL);
        if result.elements().is_empty() {
            return ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::BooleanEmpty)]);
        }
        let rect = result.bounding_box();

        // Write the result element. `path` is stored in absolute world coords,
        // exactly as `derive::geom_for` reads it back (the element's rotation is
        // identity here, so no local/world reframe is needed).
        let id = match self.mint_element_id() {
            Ok(id) => id,
            Err(e) => return ToileOutcome::display_only(vec![ToileEvent::Error(e)]),
        };
        let elems = self.doc.get_map(Root::Elements.as_str());
        let Ok(em) = elems
            .insert_container(id.as_str(), LoroMap::new())
            .map(Fields::<ElementKey>::of)
        else {
            return self.err_no_element(&id);
        };
        let _ = em.insert(
            ElementKey::Type,
            ElementKind::Shape(ShapeKind::Path).type_tag(),
        );
        let _ = em.insert(ElementKey::Shape, ShapeKind::Path.tag());
        store_corner(&em, rect.origin());
        store_size(&em, rect.size());
        let _ = em.insert(ElementKey::Rot, 0.0);
        let _ = em.insert(ElementKey::Locked, false);
        let style = style_bytes.unwrap_or_else(|| ElementStyle::default().to_bytes());
        let _ = em.insert(ElementKey::Style, LoroValue::Binary(style.into()));
        let _ = em.insert(ElementKey::Path, result.to_svg());
        let _ = self
            .doc
            .get_movable_list(Root::Layers.as_str())
            .push(NodeId::Element(id.clone()).encode().as_str());

        // Consume the sources in the same commit so the op is one undo item.
        if keep == Sources::Consume {
            self.delete_from_doc(&sources);
        }
        self.doc.commit();

        self.rebuild_caches();
        self.emit_with_update(vec![
            ToileEvent::ElementCreated {
                id,
                correlate: None,
            },
            self.viewport_event(),
        ])
    }
}

/// Build the world-space outline of a shape / path element as a [`BezPath`], or
/// `None` when the element is not a shape kind. The local outline per
/// [`ShapeKind`] is transformed by the same rotation-about-center affine the
/// renderer uses (`derive::display_element`).
fn element_world_path(em: &Fields<ElementKey>) -> Option<BezPath> {
    if !has_type(em, ElementKind::Shape(ShapeKind::Rect)) {
        return None;
    }
    let bounds = stored_rect(em);
    let kind = shape_of(em);
    let local = match kind {
        ShapeKind::Rect => bounds.to_path(FLATTEN_TOL.get().get()),
        ShapeKind::Ellipse => Ellipse::from_rect(bounds).to_path(FLATTEN_TOL.get().get()),
        ShapeKind::Triangle => crate::ui::hit::triangle_path(bounds),
        ShapeKind::Line => {
            let mut p = BezPath::new();
            p.move_to((bounds.x0, bounds.y0));
            p.line_to((bounds.x1, bounds.y1));
            p
        }
        ShapeKind::Path => path_of(em).unwrap_or_default(),
    };
    // World transform: rotation about the AABB center (derive.rs:122-124).
    let rot = rotation_of(em);
    let center = bounds.center();
    let transform = Affine::translate(center.to_vec2())
        * Affine::rotate(rot.get())
        * Affine::translate(-center.to_vec2());
    Some(transform * local)
}

#[cfg(test)]
mod tests {
    use kurbo::{Point, Size};

    use crate::logic::command::{
        BoolOp, InitContent, Sources, ToileCommand, ToileError, ToileEvent,
    };
    use crate::logic::model::{ElementKind, ShapeKind};
    use crate::logic::state::ToileState;
    use ambre_core::id::{Element, Id};

    fn add_rect(st: &mut ToileState, x: f64, y: f64, w: f64, h: f64) -> Id<Element> {
        let out = st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(x, y),
            size: Size::new(w, h),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        match out.display.into_iter().find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id),
            _ => None,
        }) {
            Some(id) => id,
            None => panic!("no ElementCreated"),
        }
    }

    #[test]
    fn union_of_two_rects_makes_one_path_element() {
        let mut st = ToileState::new();
        let a = add_rect(&mut st, 0.0, 0.0, 10.0, 10.0);
        let b = add_rect(&mut st, 5.0, 0.0, 10.0, 10.0);
        let out = st.handle_command(ToileCommand::BooleanOp {
            op: BoolOp::Union,
            ids: vec![a, b],
            sources: Sources::Consume,
        });
        // Sources consumed, one result element remains.
        assert_eq!(
            st.element_count(),
            ambre_core::Tally::<ambre_core::Elements>::new(1)
        );
        assert!(out.update.is_some(), "should broadcast a CRDT delta");
    }

    #[test]
    fn keep_sources_leaves_inputs() {
        let mut st = ToileState::new();
        let a = add_rect(&mut st, 0.0, 0.0, 10.0, 10.0);
        let b = add_rect(&mut st, 5.0, 0.0, 10.0, 10.0);
        st.handle_command(ToileCommand::BooleanOp {
            op: BoolOp::Union,
            ids: vec![a, b],
            sources: Sources::Keep,
        });
        assert_eq!(
            st.element_count(),
            ambre_core::Tally::<ambre_core::Elements>::new(3)
        ); // two sources + result
    }

    #[test]
    fn boolean_is_one_undo_item() {
        let mut st = ToileState::new();
        let a = add_rect(&mut st, 0.0, 0.0, 10.0, 10.0);
        let b = add_rect(&mut st, 5.0, 0.0, 10.0, 10.0);
        st.handle_command(ToileCommand::BooleanOp {
            op: BoolOp::Union,
            ids: vec![a, b],
            sources: Sources::Consume,
        });
        assert_eq!(
            st.element_count(),
            ambre_core::Tally::<ambre_core::Elements>::new(1)
        );
        // One undo restores both sources and removes the result.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(
            st.element_count(),
            ambre_core::Tally::<ambre_core::Elements>::new(2)
        );
    }

    #[test]
    fn fewer_than_two_eligible_is_rejected() {
        let mut st = ToileState::new();
        let a = add_rect(&mut st, 0.0, 0.0, 10.0, 10.0);
        let out = st.handle_command(ToileCommand::BooleanOp {
            op: BoolOp::Union,
            ids: vec![a],
            sources: Sources::Consume,
        });
        assert_eq!(
            st.element_count(),
            ambre_core::Tally::<ambre_core::Elements>::new(1)
        );
        assert!(matches!(
            out.display.first(),
            Some(ToileEvent::Error(ToileError::BooleanNeedsTwo))
        ));
    }
}
