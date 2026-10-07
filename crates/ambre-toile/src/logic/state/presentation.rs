//! Presentation sequence + playback for [`ToileState`](super::ToileState)
//! (`toile-presentation`). A presentation is not a separate app: a frame is an
//! ordinary element and this side panel orders frames into a slide thread.
//!
//! The split that drives this module. The **sequence** (which frames, in what
//! order) is **CRDT**: a `LoroMovableList<FrameId>` under a `presentation` map,
//! shared and convergent. The **playback** (which slide this viewer is on, focus
//! mode) is **local/ephemeral**: a plain field on `ToileState`, never in Loro and
//! never synced. This mirrors the durable-intent vs momentary-gesture split
//! already used for presence (`toile-presence`).

use std::collections::HashSet;

use ambre_bus::UserId;
use loro::{LoroDoc, LoroMap, LoroMovableList, LoroText, LoroValue};

#[cfg(feature = "pdf")]
use kurbo::Rect;

#[cfg(feature = "pdf")]
use crate::logic::interop::{PageSize, PdfPage};
#[cfg(feature = "pdf")]
use crate::ui::viewport::{FIT_PAD, ViewportTransform};

use crate::logic::command::{
    DeckFrame, DeckView, PresencePayload, SlideChange, TextDelta, ToileError, ToileEvent,
    ToileOutcome,
};
use crate::logic::model::{ElementId, FrameId, NodeId};
use crate::logic::text_merge::merge_text;

use super::codec::str_of;
use super::{CHILDREN, NOTES, PRESENTATION_CONFIG, PRESENTATION_ORDER, ToileState};

/// The slide transition length a fresh board is seeded with, in milliseconds.
/// The deck's `transition_ms` register carries an editable value over it, and a
/// register holding anything else reads as this (`TOILE-FEAT-028`).
pub const DEFAULT_TRANSITION_MS: f64 = 300.0;

impl ToileState {
    // -- sequence (CRDT,) -------------------------------------------------

    /// Replace the shared slide sequence with `order`. Repeated frame ids are
    /// de-duplicated (idempotent membership). A dangling FrameId is left in the
    /// list and skipped at derivation like a dangling connector, so no
    /// frame-existence check happens here. CRDT and undoable: one `doc.commit()`
    /// is one undo item. The local playback cursor is clamped to the new deck
    /// length.
    pub(super) fn set_slide_order(&mut self, order: &[FrameId]) -> ToileOutcome {
        // De-duplicate, keeping first occurrence (idempotent membership).
        let mut target: Vec<&str> = Vec::new();
        for id in order {
            if !target.contains(&id.as_str()) {
                target.push(id.as_str());
            }
        }

        let list = self.order_list();
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
        self.emit_with_update(vec![ToileEvent::DeckChanged])
    }

    /// The board's frames for the deck panel (`TOILE-FEAT-026`): the deck in
    /// playback order, then every other frame in paint order. A pure read -- no
    /// commit, no delta, no undo item.
    ///
    /// A slide naming a frame the board no longer holds is dropped here as well
    /// as at deletion (`TOILE-FEAT-027`). A deck merged from a peer that
    /// deleted a frame concurrently then still lists only frames the panel can
    /// show.
    pub(super) fn read_deck(&self) -> ToileOutcome {
        let deck = self.slide_order();
        let slides: Vec<DeckFrame> = deck
            .iter()
            .filter(|id| self.is_frame(id))
            .map(|id| self.deck_frame(id))
            .collect();
        let off_deck: Vec<DeckFrame> = self
            .layer_order()
            .iter()
            .filter_map(|node| match node {
                NodeId::Element(id) => Some(id),
                NodeId::Connector(_) => None,
            })
            .filter(|id| self.is_frame(id) && !deck.contains(id))
            .map(|id| self.deck_frame(id))
            .collect();
        ToileOutcome::display_only(vec![ToileEvent::DeckData(DeckView { slides, off_deck })])
    }

    /// Whether `id` still names a frame on the board.
    pub(super) fn is_frame(&self, id: &ElementId) -> bool {
        self.element_map(id)
            .is_some_and(|em| str_of(&em, "type") == "frame")
    }

    /// One deck row: the frame's id and its title, empty until it is named.
    fn deck_frame(&self, id: &ElementId) -> DeckFrame {
        let name = self
            .element_map(id)
            .map(|em| str_of(&em, "name"))
            .unwrap_or_default();
        DeckFrame {
            id: id.clone(),
            name,
        }
    }

    /// The shared slide sequence, in playback order. Empty if no deck exists.
    /// Test / integration accessor.
    pub fn slide_order(&self) -> Vec<FrameId> {
        slide_order_of(&self.order_list())
            .into_iter()
            .map(|s| ElementId(s.to_owned()))
            .collect()
    }

    // -- playback (local,) ------------------------------------------------

    /// Step to the next slide, saturating at the last. Local only. A
    /// manual navigation takes control: any active follow is cleared first
    /// ("Follow presenter"). Broadcasts the new position when broadcasting.
    pub(super) fn next_slide(&mut self) -> ToileOutcome {
        let old = self.playback.active;
        let len = self.slide_len();
        if len > 0 && self.playback.active + 1 < len {
            self.playback.active += 1;
        }
        self.manual_nav_outcome(old)
    }

    /// Step to the previous slide, saturating at the first. Local only.
    /// Takes control (clears any follow) and broadcasts, like [`next_slide`].
    pub(super) fn prev_slide(&mut self) -> ToileOutcome {
        let old = self.playback.active;
        self.playback.active = self.playback.active.saturating_sub(1);
        self.manual_nav_outcome(old)
    }

    /// Jump to `idx`, clamped to the deck length. Local only. Takes
    /// control (clears any follow) and broadcasts, like [`next_slide`].
    pub(super) fn goto_slide(&mut self, idx: usize) -> ToileOutcome {
        let old = self.playback.active;
        let len = self.slide_len();
        self.playback.active = if len == 0 { 0 } else { idx.min(len - 1) };
        self.manual_nav_outcome(old)
    }

    /// Build the outcome for a manual navigation from `old` to the just-updated
    /// `playback.active`. Emit the `SlideChanged` glide, clear any active
    /// follow first ("take control", prepending `FollowingChanged(None)`), and
    /// attach a broadcast frame when broadcasting.
    fn manual_nav_outcome(&mut self, old: usize) -> ToileOutcome {
        let mut display = Vec::new();
        if self.following.is_some() {
            self.following = None;
            display.push(ToileEvent::FollowingChanged(None));
        }
        let change = self.slide_change(Some(old), self.playback.active);
        display.push(ToileEvent::SlideChanged(change));
        ToileOutcome::display_and_ephemeral(display, self.slide_broadcast_frame())
    }

    /// Enter presentation (focus) mode, resetting to the first slide.
    /// Local only. Broadcasts slide 0 when already broadcasting.
    pub(super) fn enter_present(&mut self, focus: bool) -> ToileOutcome {
        self.present = true;
        self.playback.focus = focus;
        self.playback.active = 0;
        // No source frame on entry: `from` is empty so slide 0's content fades in.
        let change = self.slide_change(None, 0);
        ToileOutcome::display_and_ephemeral(
            vec![
                ToileEvent::PresentModeChanged(true),
                ToileEvent::SlideChanged(change),
            ],
            self.slide_broadcast_frame(),
        )
    }

    /// Postcard-encode an ephemeral presence frame carrying the current active
    /// slide index, but only when broadcasting is on ("Follow presenter"). The
    /// frame carries `present_slide: Some(active)` and no cursor / selection. It
    /// transiently clears this presenter's cursor halo on followers until the
    /// next cursor sample, which is acceptable (presenters emit cursor / laser
    /// continuously). `None` when not broadcasting or on an encode failure.
    fn slide_broadcast_frame(&self) -> Option<Vec<u8>> {
        if !self.broadcasting {
            return None;
        }
        let payload = PresencePayload {
            present_slide: Some(self.playback.active),
            ..Default::default()
        };
        match postcard::to_allocvec(&payload) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                tracing::warn!("Toile: slide broadcast encode failed: {e}");
                None
            }
        }
    }

    /// Toggle whether this viewer broadcasts its playback position. Local
    /// role only, no CRDT delta. When turning on while in present mode, the
    /// current slide is broadcast immediately so followers snap without waiting
    /// for the next navigation.
    /// Turning it off broadcasts one last frame carrying no slide. The peers
    /// then stop offering this viewer to follow at once, rather than at the
    /// presence timeout (`TOILE-UI-315`).
    pub(super) fn set_present_broadcast(&mut self, on: bool) -> ToileOutcome {
        let was = self.broadcasting;
        self.broadcasting = on;
        let ephemeral = if on {
            self.slide_broadcast_frame()
        } else if was {
            silent_presence_frame()
        } else {
            None
        };
        ToileOutcome::display_and_ephemeral(
            vec![ToileEvent::PresentBroadcastChanged(on)],
            ephemeral,
        )
    }

    /// Follow the presenter `who` (or stop with `None`), a local role only. No CRDT
    /// delta. If `Some(u)` and a broadcast slide from `u` is already known, snap to
    /// it at once (glide via `SlideChanged`). Otherwise the follower snaps on the
    /// next inbound frame.
    pub(super) fn follow_presenter(&mut self, who: Option<UserId>) -> ToileOutcome {
        self.following = who;
        let mut display = vec![ToileEvent::FollowingChanged(who)];
        if let Some(u) = who
            && let Some(idx) = self.peer_presence(&u).and_then(|p| p.present_slide)
        {
            let old = self.playback.active;
            let len = self.slide_len();
            let clamped = if len == 0 { 0 } else { idx.min(len - 1) };
            if clamped != old {
                self.playback.active = clamped;
                let change = self.slide_change(Some(old), clamped);
                display.push(ToileEvent::SlideChanged(change));
            }
        }
        ToileOutcome::display_only(display)
    }

    /// Build the [`SlideChange`] payload for a transition from `from_idx` (absent
    /// on `EnterPresent`) to `to_idx`. That is the two frame snapshots the Smart
    /// Animate solver tweens, plus the target frame's world rect the camera fits.
    /// An out-of-range or dangling index yields an empty snapshot / `None` rect,
    /// so a missing frame degrades to a plain fade rather than a panic.
    pub(super) fn slide_change(&self, from_idx: Option<usize>, to_idx: usize) -> SlideChange {
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
            transition_ms: self.transition_ms(),
        }
    }

    /// Write the deck's transition length, for a test that needs a board
    /// configured otherwise. No command writes this register yet: the surface
    /// that would edit it is not built (this spec section 6).
    #[cfg(test)]
    pub(crate) fn set_transition_ms_for_test(&mut self, ms: f64) {
        let _ = self
            .doc
            .get_map(PRESENTATION_CONFIG)
            .insert("transition_ms", ms);
        self.doc.commit();
    }

    /// The deck's own transition length, in milliseconds (`TOILE-FEAT-028`).
    ///
    /// A config the board does not carry reads as the default. So does a value
    /// that is not a plain length. A negative or absurd one would make every
    /// slide change instant or unending, on a register any peer can write.
    fn transition_ms(&self) -> f64 {
        const MAX_TRANSITION_MS: f64 = 10_000.0;
        self.doc
            .get_map(PRESENTATION_CONFIG)
            .get("transition_ms")
            .and_then(|v| v.into_value().ok())
            .and_then(|v| match v {
                LoroValue::Double(ms) => Some(ms),
                LoroValue::I64(ms) => Some(ms as f64),
                _ => None,
            })
            .filter(|ms| ms.is_finite() && (0.0..=MAX_TRANSITION_MS).contains(ms))
            .unwrap_or(DEFAULT_TRANSITION_MS)
    }

    /// Leave presentation mode. Local only. Broadcasting is turned off on
    /// exit (a viewer no longer presenting has no position to share); `following`
    /// is independent of present mode and left untouched.
    pub(super) fn exit_present(&mut self) -> ToileOutcome {
        self.present = false;
        let mut display = vec![ToileEvent::PresentModeChanged(false)];
        if !self.broadcasting {
            return ToileOutcome::display_only(display);
        }
        self.broadcasting = false;
        display.push(ToileEvent::PresentBroadcastChanged(false));
        // Say so on the wire too: a presenter who left the mode is no longer
        // one to follow (`TOILE-UI-315`).
        ToileOutcome::display_and_ephemeral(display, silent_presence_frame())
    }

    /// Number of slides in the shared deck (`0` if absent). Local read.
    pub(super) fn slide_len(&self) -> usize {
        match self.order_list().get_deep_value() {
            LoroValue::List(items) => items.len(),
            _ => 0,
        }
    }

    /// Current playback state: `(present, active, focus)`. Test accessor.
    pub fn playback(&self) -> (bool, usize, bool) {
        (self.present, self.playback.active, self.playback.focus)
    }

    // -- speaker notes (CRDT,) --------------------------------------------

    /// Apply a char-CRDT splice to a frame's collaborative speaker notes. Notes
    /// are a `LoroText` in the frame element's payload, seeded lazily on the first
    /// edit (like the deck's `order` list). The splice indices are clamped to the
    /// current length, so an out-of-range editor edit is bounded rather than
    /// rejected. CRDT and undoable: one `doc.commit()` is one undo item. A missing
    /// element or a non-frame element (no `children` field) is a rejected no-op,
    /// mirroring `set_text`'s kind guard.
    ///
    /// The splice is measured against `base`, the note the panel held. When the
    /// stored note is still `base`, the splice applies as sent. Otherwise a peer
    /// wrote meanwhile: the typed note is merged into the stored one, and the
    /// panel is told the merged note (`TOILE-FEAT-088`).
    pub(super) fn edit_notes(
        &mut self,
        frame: &FrameId,
        base: &str,
        delta: &TextDelta,
    ) -> ToileOutcome {
        let Some(em) = self.element_map(frame) else {
            return self.err_no_element(frame);
        };
        // Only a frame carries notes; `children` is seeded at frame birth.
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
        let stored = text.to_string();
        if stored != base {
            let typed = splice(base, delta);
            let merged = merge_text(base, &stored, &typed);
            if merged == stored {
                return ToileOutcome::display_only(Vec::new());
            }
            let _ = text.update(&merged, loro::UpdateOptions::default());
            self.doc.commit();
            // The panel shows `typed`, and the note is now `merged`.
            let told = (merged != typed).then(|| ToileEvent::NotesChanged {
                frame: frame.clone(),
                notes: merged,
            });
            return self.emit_with_update(told.into_iter().collect());
        }
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

    // -- PDF export pages (toile-pdf-export) ----------------------------

    /// Build one [`PdfPage`] per presentation slide for an export at `page` size
    /// (`toile-pdf-export`). Each page's camera fits the frame's world rect into
    /// the page with the same `FIT_PAD` the on-screen present glide uses
    /// (`ui::view`). The page is clipped to that frame rect, so the PDF matches
    /// what the audience sees.
    ///
    /// A dangling `FrameId` (present in the deck but with no element) is
    /// skipped like a dangling connector. An empty deck
    /// falls back to a single, unclipped page fitted to the whole board content.
    /// A board with no content at all yields no pages, and the caller's
    /// [`export_pdf`](crate::logic::interop::export_pdf) reports
    /// [`Empty`](crate::logic::interop::PdfExportError::Empty). Read-only: every
    /// page is derived through [`derive_viewport_at`](Self::derive_viewport_at)
    /// without touching `self.viewport`.
    #[cfg(feature = "pdf")]
    pub fn pdf_pages(&self, page: PageSize) -> Vec<PdfPage> {
        // The camera that frames `rect` into the page, padded like the live
        // present glide so the slide never touches the page edge.
        let fit = |rect: Rect| {
            let mut cam = ViewportTransform::new(page.width, page.height);
            cam.fit_bounds(rect, FIT_PAD);
            cam
        };

        let slides: Vec<(FrameId, Rect)> = self
            .slide_order()
            .into_iter()
            .filter_map(|f| self.element_rect(&f).map(|r| (f, r)))
            .collect();

        if slides.is_empty() {
            // Empty deck: one page framing the whole board (unclipped). No
            // content at all yields no page, so the export reports `Empty`.
            return match self.content_bounds() {
                Some(bounds) => {
                    let camera = fit(bounds);
                    vec![PdfPage {
                        viewport: self.derive_viewport_at(camera.to_bus_viewport()),
                        camera,
                        clip: None,
                    }]
                }
                None => Vec::new(),
            };
        }

        slides
            .into_iter()
            .map(|(_frame, rect)| {
                let camera = fit(rect);
                PdfPage {
                    viewport: self.derive_viewport_at(camera.to_bus_viewport()),
                    camera,
                    clip: Some(rect),
                }
            })
            .collect()
    }

    /// The union AABB of every element's world bounds, or `None` if the board
    /// holds no element. Frames the empty-deck PDF fallback page.
    #[cfg(feature = "pdf")]
    fn content_bounds(&self) -> Option<Rect> {
        self.bounds.values().copied().reduce(|acc, r| acc.union(r))
    }

    /// Drop every slide whose frame is in `deleted` (`TOILE-FEAT-027`), without
    /// committing. The caller owns the commit, so a deletion and the deck it
    /// prunes are one undo item and can never be seen apart.
    ///
    /// A deletion naming no slide writes nothing: deleting a note must leave
    /// the deck untouched, and an empty deck has nothing to prune.
    pub(super) fn drop_slides(&mut self, deleted: &HashSet<&ElementId>) -> bool {
        let list = self.order_list();
        let before = slide_order_of(&list);
        if before.is_empty() {
            return false;
        }
        let dead: HashSet<&str> = deleted.iter().map(|id| id.as_str()).collect();
        let kept: Vec<String> = before
            .iter()
            .filter(|id| !dead.contains(id.as_str()))
            .cloned()
            .collect();
        if kept.len() == before.len() {
            return false;
        }
        if let LoroValue::List(items) = list.get_deep_value()
            && !items.is_empty()
        {
            let _ = list.delete(0, items.len());
        }
        for id in &kept {
            let _ = list.push(id.as_str());
        }
        // Clamp this viewer's cursor, as a shorter deck does anywhere else.
        self.playback.active = self.playback.active.min(kept.len().saturating_sub(1));
        true
    }

    // -- container helpers -----------------------------------------------------

    /// The slide sequence: a root container, so it exists in every peer's
    /// document from the start and is created by nobody (`TOILE-029`).
    fn order_list(&self) -> LoroMovableList {
        self.doc.get_movable_list(PRESENTATION_ORDER)
    }
}

/// A presence frame carrying no slide: what a viewer sends when it stops
/// broadcasting (`TOILE-UI-315`). The peers read the whole live set, so a
/// sample naming no slide is what drops this viewer from the presenters.
fn silent_presence_frame() -> Option<Vec<u8>> {
    match postcard::to_allocvec(&PresencePayload::default()) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            tracing::warn!("Toile: the end-of-broadcast frame failed to encode: {e}");
            None
        }
    }
}

/// Write the presentation defaults into the config root map (`TOILE-029`).
///
/// Called from board construction and from the schema step that carries an
/// older board. The map is a root container, so this writes registers into a
/// container every peer already addresses, never a container of its own.
pub(crate) fn seed_presentation_config(doc: &LoroDoc) {
    let cfg = doc.get_map(PRESENTATION_CONFIG);
    if cfg.get("transition_ms").is_none() {
        let _ = cfg.insert("transition_ms", DEFAULT_TRANSITION_MS);
        let _ = cfg.insert("autoplay", false);
        let _ = cfg.insert("loop", false);
    }
}

/// `base` with `delta` applied, its offsets clamped to `base` the way the
/// handler clamps them to the stored note.
fn splice(base: &str, delta: &TextDelta) -> String {
    let len = base.chars().count();
    let at = delta.at.min(len);
    let del = delta.delete.min(len - at);
    let mut out: String = base.chars().take(at).collect();
    out.push_str(&delta.insert);
    out.extend(base.chars().skip(at + del));
    out
}

/// The frame's `notes` LoroText, seeding an empty one on first use. The
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

#[cfg(all(test, feature = "pdf"))]
mod pdf_tests {
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::interop::PageSize;
    use crate::logic::model::{ElementId, ElementKind, FrameId};
    use crate::logic::state::ToileState;

    /// Create a frame at `(x, y)` sized `w x h` and return its minted id.
    fn frame_at(st: &mut ToileState, x: f64, y: f64, w: f64, h: f64) -> FrameId {
        let out = st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Frame,
            at: Point::new(x, y),
            size: Size::new(w, h),
            content: InitContent::None,
            fill: None,
            correlate: 0,
        });
        out.display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id),
                _ => None,
            })
            .expect("frame creation yields an id")
    }

    /// One page is produced per slide in the deck, in playback order, each
    /// clipped to its frame rect.
    #[test]
    fn one_page_per_slide() {
        let mut st = ToileState::new();
        let a = frame_at(&mut st, 0.0, 0.0, 200.0, 100.0);
        let b = frame_at(&mut st, 500.0, 0.0, 200.0, 100.0);
        st.handle_command(ToileCommand::SetSlideOrder { order: vec![a, b] });

        let pages = st.pdf_pages(PageSize::default());
        assert_eq!(pages.len(), 2, "one page per slide");
        assert!(
            pages.iter().all(|p| p.clip.is_some()),
            "every slide page is clipped to its frame rect"
        );
    }

    /// A dangling `FrameId` in the deck (no such element) is skipped at
    /// derivation, so it produces no page.
    #[test]
    fn dangling_frame_is_skipped() {
        let mut st = ToileState::new();
        let a = frame_at(&mut st, 0.0, 0.0, 200.0, 100.0);
        st.handle_command(ToileCommand::SetSlideOrder {
            order: vec![a, ElementId("ghost".to_owned())],
        });

        let pages = st.pdf_pages(PageSize::default());
        assert_eq!(pages.len(), 1, "the dangling frame yields no page");
        assert!(pages[0].clip.is_some());
    }

    /// An empty deck with board content falls back to a single, unclipped page
    /// fitted to the whole board.
    #[test]
    fn empty_deck_falls_back_to_one_page() {
        let mut st = ToileState::new();
        frame_at(&mut st, 0.0, 0.0, 200.0, 100.0);
        // No SetSlideOrder: the deck is empty.
        let pages = st.pdf_pages(PageSize::default());
        assert_eq!(pages.len(), 1, "empty deck -> one whole-content page");
        assert!(pages[0].clip.is_none(), "the fallback page is unclipped");
    }

    /// A board with no content at all yields no pages (the export then reports
    /// `Empty`).
    #[test]
    fn empty_board_yields_no_pages() {
        let st = ToileState::new();
        assert!(st.pdf_pages(PageSize::default()).is_empty());
    }
}
