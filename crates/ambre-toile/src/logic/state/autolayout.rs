//! Auto-layout spec register for [`ToileState`](super::ToileState)
//! (`toile-auto-layout`). A frame can carry an [`AutoLayout`] spec so its
//! children are positioned by the pure `solve_layout` solver
//! ([`crate::logic::autolayout`]) rather than by absolute coordinates.
//!
//! The **spec is CRDT** -- a single LWW value register (`Binary(postcard(
//! AutoLayout))`) under the frame element map's `autolayout` key. Being a plain
//! value register, not a container, concurrent sets from two peers converge by
//! Loro's map register LWW. No birth-seeding is needed, and an absent key means
//! free positioning. The **solved positions are derived on render, never
//! persisted**. Persisting them would be redundant derived state that could
//! desync from the spec, exactly like connector routing (`toile-connectors`).

use loro::LoroValue;

use crate::logic::autolayout::AutoLayout;
use crate::logic::command::{ToileError, ToileEvent, ToileOutcome};
use ambre_core::id::{Element, Frame, Id};

use super::ToileState;
use super::codec::has_type;
use super::names::ElementKey;
use crate::logic::model::ElementKind;

impl ToileState {
    /// Set (or clear, with `None`) a frame's auto-layout spec. A single LWW
    /// register on the frame element; convergent and undoable (one `doc.commit()`
    /// is one undo item,). A missing element or a non-frame element is a
    /// rejected no-op, mirroring `edit_notes` / `set_text`'s kind guard. A
    /// pure no-op (clearing an absent spec, or re-writing a byte-identical spec)
    /// produces neither a commit nor a delta.
    pub(super) fn set_auto_layout(
        &mut self,
        frame: &Id<Frame>,
        layout: Option<AutoLayout>,
    ) -> ToileOutcome {
        let element = Id::<Element>::from(frame.clone());
        let Some(em) = self.element_map(&element) else {
            return self.err_no_element(&element);
        };
        // Only a frame carries an auto-layout spec. A group also has `children`,
        // so the guard is on the concrete `type`, not on membership.
        if !has_type(&em, ElementKind::Frame) {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::ContentKindMismatch,
            )]);
        }

        match layout {
            Some(l) => {
                let bytes = match postcard::to_allocvec(&l) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!("Toile: auto-layout encode failed: {e}");
                        return ToileOutcome::display_only(Vec::new());
                    }
                };
                // Skip a byte-identical re-write so an unchanged spec is not a
                // spurious commit / undo item.
                if let Some(cur) = em
                    .get(ElementKey::Autolayout)
                    .and_then(|v| v.into_value().ok())
                    && let LoroValue::Binary(cur) = cur
                    && cur.as_slice() == bytes.as_slice()
                {
                    return ToileOutcome::display_only(Vec::new());
                }
                let _ = em.insert(ElementKey::Autolayout, LoroValue::Binary(bytes.into()));
            }
            None => {
                // Nothing to clear if the frame has no spec.
                if em.get(ElementKey::Autolayout).is_none() {
                    return ToileOutcome::display_only(Vec::new());
                }
                let _ = em.delete(ElementKey::Autolayout);
            }
        }
        self.doc.commit();
        // No dedicated event: the solved positions are re-derived on the next
        // viewport pass (Phase 1/2 wiring). A layout edit repaints via the
        // ordinary viewport event.
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// The current auto-layout spec of `frame`, or `None` if it has none / is not
    /// a frame / does not exist. Test / derivation accessor.
    pub fn auto_layout(&self, frame: &Id<Frame>) -> Option<AutoLayout> {
        let em = self.element_map(&frame.clone().into())?;
        let LoroValue::Binary(bytes) = em.get(ElementKey::Autolayout)?.into_value().ok()? else {
            return None;
        };
        postcard::from_bytes(&bytes).ok()
    }
}
