//! Presentation sequence + playback for [`ToileState`](super::ToileState)
//! (`toile-features` §1 / §5). A presentation is not a separate app: a frame is
//! an ordinary element and this side panel orders frames into a slide thread.
//!
//! The split that drives this module (§0/§1): the **sequence** (which frames, in
//! what order) is **CRDT** -- a `LoroMovableList<FrameId>` under a `presentation`
//! map, shared and convergent; the **playback** (which slide this viewer is on,
//! focus mode) is **local/ephemeral** -- a plain field on `ToileState`, never in
//! Loro and never synced. This mirrors the durable-intent vs momentary-gesture
//! split already used for presence (`toile-logic §7`).

use loro::{LoroMap, LoroMovableList, LoroText, LoroValue};

use crate::logic::command::{SlideChange, TextDelta, ToileError, ToileEvent, ToileOutcome};
use crate::logic::model::{ElementId, FrameId};

use super::{CHILDREN, CONFIG, NOTES, ORDER, PRESENTATION, ToileState};

impl ToileState {
    // -- sequence (CRDT, §1.A) -------------------------------------------------

    /// Replace the shared slide sequence with `order` (§1.A). Repeated frame ids
    /// are de-duplicated (idempotent membership); a dangling FrameId is left in
    /// the list and skipped at derivation like a dangling connector (§5.A), so no
    /// frame-existence check happens here. CRDT and undoable: one `doc.commit()`
    /// is one undo item (§8 bis). The local playback cursor is clamped to the new
    /// deck length.
    pub(super) fn set_slide_order(&mut self, order: &[FrameId]) -> ToileOutcome {
        // De-duplicate, keeping first occurrence (§1.A idempotent membership).
        let mut target: Vec<&str> = Vec::new();
        for id in order {
            if !target.contains(&id.as_str()) {
                target.push(id.as_str());
            }
        }

        let list = self.order_list_or_seed();
        // Skip a no-op set so an unchanged order produces neither a commit nor a
        // delta (it would otherwise be a spurious undo item).
        let current = slide_order_of(&list);
        if current.len() == target.len() && current.iter().zip(&target).all(|(a, b)| a == b) {
            return ToileOutcome::display_only(Vec::new());
        }
        // Clear then re-push: LoroMovableList merges concurrent replacements, and
        // the deck is small enough that a full rewrite is cheaper than diffing.
        if let LoroValue::List(items) = list.get_deep_value()
            && !items.is_empty()
        {
            let _ = list.delete(0, items.len());
        }
        for id in &target {
            let _ = list.push(*id);
        }
        self.doc.commit();

        // Clamp the local cursor to the new deck length (empty deck -> 0).
        let len = target.len();
        self.playback.active = self.playback.active.min(len.saturating_sub(1));
        self.emit_with_update(Vec::new())
    }

    /// The shared slide sequence, in playback order. Empty if no deck exists.
    /// Test / integration accessor.
    pub fn slide_order(&self) -> Vec<FrameId> {
        let Some(list) = self.order_list() else {
            return Vec::new();
        };
        slide_order_of(&list)
            .into_iter()
            .map(|s| ElementId(s.to_owned()))
            .collect()
    }

    // -- playback (local, §1.B) ------------------------------------------------

    /// Step to the next slide, saturating at the last (§1.B). Local only.
    pub(super) fn next_slide(&mut self) -> ToileOutcome {
        let old = self.playback.active;
        let len = self.slide_len();
        if len > 0 && self.playback.active + 1 < len {
            self.playback.active += 1;
        }
        let change = self.slide_change(Some(old), self.playback.active);
        ToileOutcome::display_only(vec![ToileEvent::SlideChanged(change)])
    }

    /// Step to the previous slide, saturating at the first (§1.B). Local only.
    pub(super) fn prev_slide(&mut self) -> ToileOutcome {
        let old = self.playback.active;
        self.playback.active = self.playback.active.saturating_sub(1);
        let change = self.slide_change(Some(old), self.playback.active);
        ToileOutcome::display_only(vec![ToileEvent::SlideChanged(change)])
    }

    /// Jump to `idx`, clamped to the deck length (§1.B). Local only.
    pub(super) fn goto_slide(&mut self, idx: usize) -> ToileOutcome {
        let old = self.playback.active;
        let len = self.slide_len();
        self.playback.active = if len == 0 { 0 } else { idx.min(len - 1) };
        let change = self.slide_change(Some(old), self.playback.active);
        ToileOutcome::display_only(vec![ToileEvent::SlideChanged(change)])
    }

    /// Enter presentation (focus) mode, resetting to the first slide (§1.B).
    /// Local only.
    pub(super) fn enter_present(&mut self, focus: bool) -> ToileOutcome {
        self.present = true;
        self.playback.focus = focus;
        self.playback.active = 0;
        // No source frame on entry: `from` is empty so slide 0's content fades in.
        let change = self.slide_change(None, 0);
        ToileOutcome::display_only(vec![
            ToileEvent::PresentModeChanged(true),
            ToileEvent::SlideChanged(change),
        ])
    }

    /// Build the [`SlideChange`] payload for a transition from `from_idx` (absent
    /// on `EnterPresent`) to `to_idx` (§1.C / §1.D): the two frame snapshots the
    /// Smart Animate solver tweens plus the target frame's world rect the camera
    /// fits. An out-of-range or dangling index yields an empty snapshot / `None`
    /// rect, so a missing frame degrades to a plain fade rather than a panic.
    fn slide_change(&self, from_idx: Option<usize>, to_idx: usize) -> SlideChange {
        let order = self.slide_order();
        let snap = |i: usize| {
            order
                .get(i)
                .map(|f| self.frame_snapshot(f))
                .unwrap_or_default()
        };
        let frame = order.get(to_idx).cloned();
        SlideChange {
            index: to_idx,
            from: from_idx.map(snap).unwrap_or_default(),
            to: snap(to_idx),
            to_rect: order.get(to_idx).and_then(|f| self.element_rect(f)),
            count: order.len(),
            notes: frame.as_ref().map(|f| self.notes(f)).unwrap_or_default(),
            frame,
        }
    }

    /// Leave presentation mode (§1.B). Local only.
    pub(super) fn exit_present(&mut self) -> ToileOutcome {
        self.present = false;
        ToileOutcome::display_only(vec![ToileEvent::PresentModeChanged(false)])
    }

    /// Number of slides in the shared deck (`0` if absent). Local read.
    pub(super) fn slide_len(&self) -> usize {
        self.order_list()
            .map(|list| match list.get_deep_value() {
                LoroValue::List(items) => items.len(),
                _ => 0,
            })
            .unwrap_or(0)
    }

    /// Current playback state: `(present, active, focus)`. Test accessor.
    pub fn playback(&self) -> (bool, usize, bool) {
        (self.present, self.playback.active, self.playback.focus)
    }

    // -- speaker notes (CRDT, §1.D) --------------------------------------------

    /// Apply a char-CRDT splice to a frame's collaborative speaker notes (§1.D).
    /// Notes are a `LoroText` in the frame element's payload, seeded lazily on the
    /// first edit (like the deck's `order` list). The splice indices are clamped
    /// to the current length, so an out-of-range editor edit is bounded rather
    /// than rejected. CRDT and undoable: one `doc.commit()` is one undo item
    /// (§8 bis). A missing element or a non-frame element (no `children` field) is
    /// a rejected no-op, mirroring `set_text`'s kind guard (§6).
    pub(super) fn edit_notes(&mut self, frame: &FrameId, delta: &TextDelta) -> ToileOutcome {
        let Some(em) = self.element_map(frame) else {
            return self.err_no_element(frame);
        };
        // Only a frame carries notes; `children` is seeded at frame birth (§2.C).
        if em.get(CHILDREN).is_none() {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::ContentKindMismatch,
            )]);
        }
        // Skip a pure no-op so an empty delta produces neither a commit nor an
        // undo item.
        if delta.delete == 0 && delta.insert.is_empty() {
            return ToileOutcome::display_only(Vec::new());
        }
        let text = notes_text_or_seed(&em);
        // Clamp to the current length: `at` and the deletion span never run past
        // the end of the buffer.
        let len = text.len_unicode();
        let at = delta.at.min(len);
        let del = delta.delete.min(len - at);
        if del > 0 {
            let _ = text.delete(at, del);
        }
        if !delta.insert.is_empty() {
            let _ = text.insert(at, &delta.insert);
        }
        self.doc.commit();
        self.emit_with_update(Vec::new())
    }

    /// The current speaker notes of `frame` as a plain string (empty if the frame
    /// has no notes yet or does not exist). Test / integration accessor.
    pub fn notes(&self, frame: &FrameId) -> String {
        self.element_map(frame)
            .and_then(|em| em.get(NOTES))
            .and_then(|v| v.into_container().ok())
            .and_then(|c| c.into_text().ok())
            .map(|t| t.to_string())
            .unwrap_or_default()
    }

    // -- container helpers -----------------------------------------------------

    /// The `presentation.order` movable list, if the deck has been seeded.
    fn order_list(&self) -> Option<LoroMovableList> {
        self.doc
            .get_map(PRESENTATION)
            .get(ORDER)?
            .into_container()
            .ok()?
            .into_movable_list()
            .ok()
    }

    /// The `presentation.order` movable list, creating the `presentation` map,
    /// its `order` list and default `config` on first use.
    fn order_list_or_seed(&mut self) -> LoroMovableList {
        if let Some(list) = self.order_list() {
            return list;
        }
        let pres = self.doc.get_map(PRESENTATION);
        // Default config (§1.A): the fields exist so later slices can edit them.
        if pres.get(CONFIG).is_none()
            && let Ok(cfg) = pres.insert_container(CONFIG, LoroMap::new())
        {
            let _ = cfg.insert("transition_ms", 300.0);
            let _ = cfg.insert("autoplay", false);
            let _ = cfg.insert("loop", false);
        }
        pres.insert_container(ORDER, LoroMovableList::new())
            .expect("seed presentation order list")
    }
}

/// The frame's `notes` LoroText, seeding an empty one on first use (§1.D). The
/// `em` map is already known to be a frame (checked by the caller).
fn notes_text_or_seed(em: &LoroMap) -> LoroText {
    if let Some(text) = em
        .get(NOTES)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_text().ok())
    {
        return text;
    }
    em.insert_container(NOTES, LoroText::new())
        .expect("seed frame notes text")
}

/// Read a movable list's string entries in order.
fn slide_order_of(list: &LoroMovableList) -> Vec<String> {
    let mut out = Vec::new();
    if let LoroValue::List(items) = list.get_deep_value() {
        for v in items.iter() {
            if let LoroValue::String(s) = v {
                out.push(s.to_string());
            }
        }
    }
    out
}
