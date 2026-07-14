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

use loro::{LoroMap, LoroMovableList, LoroValue};

use crate::logic::command::{ToileEvent, ToileOutcome};
use crate::logic::model::{ElementId, FrameId};

use super::{CONFIG, ORDER, PRESENTATION, ToileState};

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
        let len = self.slide_len();
        if len > 0 && self.playback.active + 1 < len {
            self.playback.active += 1;
        }
        ToileOutcome::display_only(vec![ToileEvent::SlideChanged(self.playback.active)])
    }

    /// Step to the previous slide, saturating at the first (§1.B). Local only.
    pub(super) fn prev_slide(&mut self) -> ToileOutcome {
        self.playback.active = self.playback.active.saturating_sub(1);
        ToileOutcome::display_only(vec![ToileEvent::SlideChanged(self.playback.active)])
    }

    /// Jump to `idx`, clamped to the deck length (§1.B). Local only.
    pub(super) fn goto_slide(&mut self, idx: usize) -> ToileOutcome {
        let len = self.slide_len();
        self.playback.active = if len == 0 { 0 } else { idx.min(len - 1) };
        ToileOutcome::display_only(vec![ToileEvent::SlideChanged(self.playback.active)])
    }

    /// Enter presentation (focus) mode, resetting to the first slide (§1.B).
    /// Local only.
    pub(super) fn enter_present(&mut self, focus: bool) -> ToileOutcome {
        self.present = true;
        self.playback.focus = focus;
        self.playback.active = 0;
        ToileOutcome::display_only(vec![
            ToileEvent::PresentModeChanged(true),
            ToileEvent::SlideChanged(0),
        ])
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
