//! Auto-layout spec register for [`ToileState`](super::ToileState)
//! (`toile-auto-layout`). A frame can carry an [`AutoLayout`] spec so its
//! children are positioned by the pure `solve_layout` solver
//! ([`crate::logic::autolayout`]) rather than by absolute coordinates.
//!
//! The **spec is CRDT** -- a single LWW value register (`Binary(postcard(
//! AutoLayout))`) under the frame element map's `autolayout` key. Being a plain
//! value register (not a container), concurrent sets from two peers converge by
//! Loro's map register LWW, so no birth-seeding is needed (absent = free
//! positioning). The **solved positions are derived on render, never persisted**
//! persisting them would be redundant derived state that could desync from
//! the spec, exactly like connector routing (`toile-connectors`).

use loro::LoroValue;

use crate::logic::autolayout::AutoLayout;
use crate::logic::command::{ToileError, ToileEvent, ToileOutcome};
use crate::logic::model::FrameId;

use super::codec::str_of;
use super::{AUTOLAYOUT, ToileState};

impl ToileState {
    /// Set (or clear, with `None`) a frame's auto-layout spec. A single LWW
    /// register on the frame element; convergent and undoable (one `doc.commit()`
    /// is one undo item,). A missing element or a non-frame element is a
    /// rejected no-op, mirroring `edit_notes` / `set_text`'s kind guard. A
    /// pure no-op (clearing an absent spec, or re-writing a byte-identical spec)
    /// produces neither a commit nor a delta.
    pub(super) fn set_auto_layout(
        &mut self,
        frame: &FrameId,
        layout: Option<AutoLayout>,
    ) -> ToileOutcome {
        let Some(em) = self.element_map(frame) else {
            return self.err_no_element(frame);
        };
        // Only a frame carries an auto-layout spec. A group also has `children`,
        // so the guard is on the concrete `type`, not on membership.
        if str_of(&em, "type") != "frame" {
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
                if let Some(cur) = em.get(AUTOLAYOUT).and_then(|v| v.into_value().ok())
                    && let LoroValue::Binary(cur) = cur
                    && cur.as_slice() == bytes.as_slice()
                {
                    return ToileOutcome::display_only(Vec::new());
                }
                let _ = em.insert(AUTOLAYOUT, LoroValue::Binary(bytes.into()));
            }
            None => {
                // Nothing to clear if the frame has no spec.
                if em.get(AUTOLAYOUT).is_none() {
                    return ToileOutcome::display_only(Vec::new());
                }
                let _ = em.delete(AUTOLAYOUT);
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
    pub fn auto_layout(&self, frame: &FrameId) -> Option<AutoLayout> {
        let em = self.element_map(frame)?;
        let LoroValue::Binary(bytes) = em.get(AUTOLAYOUT)?.into_value().ok()? else {
            return None;
        };
        postcard::from_bytes(&bytes).ok()
    }
}
