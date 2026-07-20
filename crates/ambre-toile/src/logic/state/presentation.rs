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

use ambre_bus::UserId;
use loro::{LoroMap, LoroMovableList, LoroText, LoroValue};

#[cfg(feature = "pdf")]
use kurbo::Rect;

#[cfg(feature = "pdf")]
use crate::logic::interop::{PageSize, PdfPage};
#[cfg(feature = "pdf")]
use crate::ui::viewport::{FIT_PAD, ViewportTransform};

use crate::logic::command::{
    PresencePayload, SlideChange, TextDelta, ToileError, ToileEvent, ToileOutcome,
};
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

    /// Step to the next slide, saturating at the last (§1.B). Local only. A
    /// manual navigation takes control: any active follow is cleared first
    /// (§1.B "Follow presenter"). Broadcasts the new position when broadcasting.
    pub(super) fn next_slide(&mut self) -> ToileOutcome {
        let old = self.playback.active;
        let len = self.slide_len();
        if len > 0 && self.playback.active + 1 < len {
            self.playback.active += 1;
        }
        self.manual_nav_outcome(old)
    }

    /// Step to the previous slide, saturating at the first (§1.B). Local only.
    /// Takes control (clears any follow) and broadcasts, like [`next_slide`].
    pub(super) fn prev_slide(&mut self) -> ToileOutcome {
        let old = self.playback.active;
        self.playback.active = self.playback.active.saturating_sub(1);
        self.manual_nav_outcome(old)
    }

    /// Jump to `idx`, clamped to the deck length (§1.B). Local only. Takes
    /// control (clears any follow) and broadcasts, like [`next_slide`].
    pub(super) fn goto_slide(&mut self, idx: usize) -> ToileOutcome {
        let old = self.playback.active;
        let len = self.slide_len();
        self.playback.active = if len == 0 { 0 } else { idx.min(len - 1) };
        self.manual_nav_outcome(old)
    }

    /// Build the outcome for a manual navigation from `old` to the just-updated
    /// `playback.active` (§1.B): emit the `SlideChanged` glide, clear any active
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

    /// Enter presentation (focus) mode, resetting to the first slide (§1.B).
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
    /// slide index, but only when broadcasting is on (§1.B "Follow presenter").
    /// The frame carries `present_slide: Some(active)` and no cursor / selection:
    /// it transiently clears this presenter's cursor halo on followers until the
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

    /// Toggle whether this viewer broadcasts its playback position (§1.B). Local
    /// role only, no CRDT delta. When turning on while in present mode, the
    /// current slide is broadcast immediately so followers snap without waiting
    /// for the next navigation.
    pub(super) fn set_present_broadcast(&mut self, on: bool) -> ToileOutcome {
        self.broadcasting = on;
        let ephemeral = if on {
            self.slide_broadcast_frame()
        } else {
            None
        };
        ToileOutcome::display_and_ephemeral(
            vec![ToileEvent::PresentBroadcastChanged(on)],
            ephemeral,
        )
    }

    /// Follow the presenter `who` (or stop with `None`), a local role only (§1.B).
    /// No CRDT delta. If `Some(u)` and a broadcast slide from `u` is already known,
    /// snap to it at once (glide via `SlideChanged`); otherwise the follower snaps
    /// on the next inbound frame.
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
    /// on `EnterPresent`) to `to_idx` (§1.C / §1.D): the two frame snapshots the
    /// Smart Animate solver tweens plus the target frame's world rect the camera
    /// fits. An out-of-range or dangling index yields an empty snapshot / `None`
    /// rect, so a missing frame degrades to a plain fade rather than a panic.
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
        }
    }

    /// Leave presentation mode (§1.B). Local only. Broadcasting is turned off on
    /// exit (a viewer no longer presenting has no position to share); `following`
    /// is independent of present mode and left untouched.
    pub(super) fn exit_present(&mut self) -> ToileOutcome {
        self.present = false;
        let mut display = vec![ToileEvent::PresentModeChanged(false)];
        if self.broadcasting {
            self.broadcasting = false;
            display.push(ToileEvent::PresentBroadcastChanged(false));
        }
        ToileOutcome::display_only(display)
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

    // -- PDF export pages (toile-interop §3.A / §7) ----------------------------

    /// Build one [`PdfPage`] per presentation slide for an export at `page` size
    /// (`toile-interop §3.A` / §7). Each page's camera fits the frame's world
    /// rect into the page with the same `FIT_PAD` the on-screen present glide
    /// uses (`ui::view`), and the page is clipped to that frame rect, so the PDF
    /// matches what the audience sees. A dangling `FrameId` (present in the deck
    /// but with no element) is skipped like a dangling connector (§5.A). An empty
    /// deck falls back to a single, unclipped page fitted to the whole board
    /// content; a board with no content at all yields no pages, and the caller's
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
        });
        out.display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated(id) => Some(id),
                _ => None,
            })
            .expect("frame creation yields an id")
    }

    /// One page is produced per slide in the deck, in playback order, each
    /// clipped to its frame rect (§7).
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
    /// derivation, so it produces no page (§5.A).
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
