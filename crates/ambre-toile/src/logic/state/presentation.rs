//! Presentation sequence + playback for [`ToileState`](super::ToileState)
//! (`toile-presentation`). A presentation is not a separate app: a frame is an
//! ordinary element and this side panel orders frames into a slide thread.
//!
//! The split that drives this module. The **sequence** (which frames, in what
//! order) is **CRDT**: a `LoroMovableList<Id<Frame>>` under a `presentation` map,
//! shared and convergent. The **playback** (which slide this viewer is on, focus
//! mode) is **local/ephemeral**: a plain field on `ToileState`, never in Loro and
//! never synced. This mirrors the durable-intent vs momentary-gesture split
//! already used for presence (`toile-presence`).

use ambre_core::index::{Len, Pos};
use std::collections::HashSet;

use ambre_core::id::{Element, Frame, Id, User};
use ambre_core::{Char, char_len};
use loro::{LoroDoc, LoroMovableList, LoroText, LoroValue};

#[cfg(feature = "pdf")]
use kurbo::Rect;

#[cfg(feature = "pdf")]
use crate::logic::interop::{PageSize, PdfPage};
#[cfg(feature = "pdf")]
use crate::ui::viewport::{FIT_PAD, ViewportTransform};

use crate::logic::command::{
    Broadcast, DeckFrame, DeckView, PlaybackState, PresencePayload, PresentFocus, PresentMode,
    Slide, SlideChange, SlidePosition, TextDelta, ToileError, ToileEvent, ToileOutcome,
};
use crate::logic::model::{ElementKind, NodeId};
use crate::logic::text_merge::{Merge, merge_edits};

use super::codec::has_type;
use super::codec::str_of;
use super::names::{ConfigKey, ElementKey, Fields, Root, config};
use super::{ToileState, write_edits};
use ambre_core::{FracMs, TimeSpan};

/// The slide transition length a fresh board is seeded with.
/// The deck's `transition_ms` register carries an editable value over it, and a
/// register holding anything else reads as this (`TOILE-FEAT-028`).
pub const DEFAULT_TRANSITION: TimeSpan<FracMs> = TimeSpan::<FracMs>::new(300.0);

impl ToileState {
    // -- sequence (CRDT,) -------------------------------------------------

    /// Replace the shared slide sequence with `order`. Repeated frame ids are
    /// de-duplicated (idempotent membership). A dangling Id<Frame> is left in the
    /// list and skipped at derivation like a dangling connector, so no
    /// frame-existence check happens here. CRDT and undoable: one `doc.commit()`
    /// is one undo item. The local playback cursor is clamped to the new deck
    /// length.
    pub(super) fn set_slide_order(&mut self, order: &[Id<Frame>]) -> ToileOutcome {
        // De-duplicate, keeping first occurrence (idempotent membership).
        let mut target: Vec<&Id<Frame>> = Vec::new();
        for id in order {
            if !target.contains(&id) {
                target.push(id);
            }
        }

        let list = self.order_list();
        // Skip a no-op set so an unchanged order produces neither a commit nor a
        // delta (it would otherwise be a spurious undo item).
        let current = slide_order_of(&list);
        if current.len() == target.len() && current.iter().zip(&target).all(|(a, b)| a == *b) {
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
            let _ = list.push(id.as_str());
        }
        self.doc.commit();

        // Clamp the local cursor to the new deck length (empty deck -> 0).
        self.playback.active = self.playback.active.within(Len::of(target.len()));
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
            .filter(|id| self.is_frame(&Id::<Element>::from((*id).clone())))
            .map(|id| self.deck_frame(id))
            .collect();
        let off_deck: Vec<DeckFrame> = self
            .layer_order()
            .iter()
            .filter_map(|node| match node {
                NodeId::Element(id) => self.frame(id),
                NodeId::Connector(_) => None,
            })
            .filter(|frame| !deck.contains(frame))
            .map(|frame| self.deck_frame(&frame))
            .collect();
        ToileOutcome::display_only(vec![ToileEvent::DeckData(DeckView { slides, off_deck })])
    }

    /// Whether `id` still names a frame on the board.
    pub(super) fn is_frame(&self, id: &Id<Element>) -> bool {
        self.element_map(id)
            .is_some_and(|em| has_type(&em, ElementKind::Frame))
    }

    /// The frame `id` names, or `None` when no element carries `id` or the
    /// element is not a frame. The one checked conversion from an element id
    /// to a frame id. `Id::<Frame>::from_text` decodes a stored text, and the
    /// engine checks the kind again where that id lands.
    pub fn frame(&self, id: &Id<Element>) -> Option<Id<Frame>> {
        self.is_frame(id)
            .then(|| Id::<Frame>::from_text(id.as_str().to_owned()))
    }

    /// One deck row: the frame's id and its title, empty until it is named.
    fn deck_frame(&self, id: &Id<Frame>) -> DeckFrame {
        let name = self
            .element_map(&id.clone().into())
            .and_then(|em| str_of(&em, ElementKey::Name))
            .unwrap_or_default();
        DeckFrame {
            id: id.clone(),
            name,
        }
    }

    /// The shared slide sequence, in playback order. Empty if no deck exists.
    /// Test / integration accessor.
    pub fn slide_order(&self) -> Vec<Id<Frame>> {
        slide_order_of(&self.order_list())
    }

    // -- playback (local,) ------------------------------------------------

    /// Step to the next slide, saturating at the last. Local only. A
    /// manual navigation takes control: any active follow is cleared first
    /// ("Follow presenter"). Broadcasts the new position when broadcasting.
    pub(super) fn next_slide(&mut self) -> ToileOutcome {
        let old = self.playback.active;
        let next = old.next();
        if next < self.slide_len().as_end() {
            self.playback.active = next;
        }
        self.manual_nav_outcome(old)
    }

    /// Step to the previous slide, saturating at the first. Local only.
    /// Takes control (clears any follow) and broadcasts, like [`next_slide`].
    pub(super) fn prev_slide(&mut self) -> ToileOutcome {
        let old = self.playback.active;
        self.playback.active = old.prev().unwrap_or(Pos::<Slide>::ZERO);
        self.manual_nav_outcome(old)
    }

    /// Jump to `idx`, clamped to the deck length. Local only. Takes
    /// control (clears any follow) and broadcasts, like [`next_slide`].
    pub(super) fn goto_slide(&mut self, idx: Pos<Slide>) -> ToileOutcome {
        let old = self.playback.active;
        self.playback.active = idx.within(self.slide_len());
        self.manual_nav_outcome(old)
    }

    /// Build the outcome for a manual navigation from `old` to the just-updated
    /// `playback.active`. Emit the `SlideChanged` glide, clear any active
    /// follow first ("take control", prepending `FollowingChanged(None)`), and
    /// attach a broadcast sample when broadcasting.
    fn manual_nav_outcome(&mut self, old: Pos<Slide>) -> ToileOutcome {
        let mut display = Vec::new();
        if self.following.is_some() {
            self.following = None;
            display.push(ToileEvent::FollowingChanged(None));
        }
        let change = self.slide_change(Some(old), self.playback.active);
        display.push(ToileEvent::SlideChanged(change));
        ToileOutcome::display_and_ephemeral(display, self.slide_broadcast_sample())
    }

    /// Enter presentation (focus) mode, resetting to the first slide.
    /// Local only. Broadcasts slide 0 when already broadcasting.
    pub(super) fn enter_present(&mut self, focus: PresentFocus) -> ToileOutcome {
        self.present = PresentMode::Presenting;
        self.playback.focus = focus;
        self.playback.active = Pos::<Slide>::ZERO;
        // No source frame on entry: `from` is empty so slide 0's content fades in.
        let change = self.slide_change(None, Pos::<Slide>::ZERO);
        ToileOutcome::display_and_ephemeral(
            vec![
                ToileEvent::PresentModeChanged(PresentMode::Presenting),
                ToileEvent::SlideChanged(change),
            ],
            self.slide_broadcast_sample(),
        )
    }

    /// Postcard-encode a presence sample carrying the current active slide
    /// index, but only when broadcasting is on ("Follow presenter"). The
    /// sample carries `present_slide: Some(active)` and no cursor / selection. It
    /// transiently clears this presenter's cursor halo on followers until the
    /// next cursor sample, which is acceptable (presenters emit cursor / laser
    /// continuously). `None` when not broadcasting or on an encode failure.
    fn slide_broadcast_sample(&self) -> Option<ambre_core::Payload<ambre_core::EphemeralBytes>> {
        if !self.broadcasting.is_on() {
            return None;
        }
        let payload = PresencePayload {
            present_slide: Some(self.playback.active),
            ..Default::default()
        };
        match postcard::to_allocvec(&payload) {
            Ok(bytes) => Some(ambre_core::Payload::<ambre_core::EphemeralBytes>::new(
                bytes,
            )),
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
    /// Turning it off broadcasts one last sample carrying no slide. The peers
    /// then stop offering this viewer to follow at once, rather than at the
    /// presence timeout (`TOILE-UI-315`).
    pub(super) fn set_present_broadcast(&mut self, wanted: Broadcast) -> ToileOutcome {
        let was = self.broadcasting;
        self.broadcasting = wanted;
        let ephemeral = if wanted.is_on() {
            self.slide_broadcast_sample()
        } else if was.is_on() {
            silent_presence_frame()
        } else {
            None
        };
        ToileOutcome::display_and_ephemeral(
            vec![ToileEvent::PresentBroadcastChanged(wanted)],
            ephemeral,
        )
    }

    /// Follow the presenter `who` (or stop with `None`), a local role only. No CRDT
    /// delta. If `Some(u)` and a broadcast slide from `u` is already known, snap to
    /// it at once (glide via `SlideChanged`). Otherwise the follower snaps on the
    /// next inbound sample.
    pub(super) fn follow_presenter(&mut self, who: Option<Id<User>>) -> ToileOutcome {
        self.following = who;
        let mut display = vec![ToileEvent::FollowingChanged(who)];
        if let Some(u) = who
            && let Some(idx) = self.peer_presence(&u).and_then(|p| p.present_slide)
        {
            let old = self.playback.active;
            let clamped = idx.within(self.slide_len());
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
    pub(super) fn slide_change(
        &self,
        from_idx: Option<Pos<Slide>>,
        to_idx: Pos<Slide>,
    ) -> SlideChange {
        let order = self.slide_order();
        let slide = |i: Pos<Slide>| i.to_index().and_then(|i| order.get(i));
        let snap = |i: Pos<Slide>| {
            slide(i)
                .map(|f| self.frame_snapshot(&f.clone().into()))
                .unwrap_or_default()
        };
        let frame = slide(to_idx).cloned();
        SlideChange {
            position: SlidePosition::new(to_idx, Len::<Slide>::of(order.len())),
            from: from_idx.map(snap).unwrap_or_default(),
            to: snap(to_idx),
            to_rect: slide(to_idx).and_then(|f| self.element_rect(&f.clone().into())),
            // A slide with no notes shows none.
            notes: frame
                .as_ref()
                .and_then(|f| self.notes(f))
                .unwrap_or_default(),
            frame,
            transition: self.transition(),
        }
    }

    /// Write the deck's transition length, for a test that needs a board
    /// configured otherwise. No command writes this register yet: the surface
    /// that would edit it is not built (this spec section 6).
    #[cfg(test)]
    pub(crate) fn set_transition_ms_for_test(&mut self, ms: f64) {
        let _ = config(&self.doc).insert(ConfigKey::TransitionMs, ms);
        self.doc.commit();
    }

    /// The deck's own transition length, in milliseconds (`TOILE-FEAT-028`).
    ///
    /// A config the board does not carry reads as the default. So does a value
    /// that is not a plain length. A negative or absurd one would make every
    /// slide change instant or unending, on a register any peer can write.
    pub(super) fn transition(&self) -> std::time::Duration {
        // The read keeps the value within `0..=10_000` ms, finite.
        std::time::Duration::from_secs_f64(self.transition_length().get() / 1000.0)
    }

    /// The `transition_ms` register as the document holds it, or the default
    /// (see [`Self::transition`]).
    pub(super) fn transition_length(&self) -> TimeSpan<FracMs> {
        const MAX_TRANSITION: TimeSpan<FracMs> = TimeSpan::<FracMs>::new(10_000.0);
        config(&self.doc)
            .value(ConfigKey::TransitionMs)
            .and_then(|v| match v {
                LoroValue::Double(ms) => Some(ms),
                LoroValue::I64(ms) => Some(ms as f64),
                _ => None,
            })
            .map(TimeSpan::<FracMs>::new)
            .filter(|ms| ms.is_finite() && (TimeSpan::<FracMs>::ZERO..=MAX_TRANSITION).contains(ms))
            .unwrap_or(DEFAULT_TRANSITION)
    }

    /// Leave presentation mode. Local only. Broadcasting is turned off on
    /// exit (a viewer no longer presenting has no position to share); `following`
    /// is independent of present mode and left untouched.
    pub(super) fn exit_present(&mut self) -> ToileOutcome {
        self.present = PresentMode::Editing;
        let mut display = vec![ToileEvent::PresentModeChanged(PresentMode::Editing)];
        if !self.broadcasting.is_on() {
            return ToileOutcome::display_only(display);
        }
        self.broadcasting = Broadcast::Private;
        display.push(ToileEvent::PresentBroadcastChanged(Broadcast::Private));
        // Say so on the wire too: a presenter who left the mode is no longer
        // one to follow (`TOILE-UI-315`).
        ToileOutcome::display_and_ephemeral(display, silent_presence_frame())
    }

    /// Number of slides in the shared deck (`0` if absent). Local read.
    pub(super) fn slide_len(&self) -> Len<Slide> {
        match self.order_list().get_deep_value() {
            LoroValue::List(items) => Len::<Slide>::of(items.len()),
            _ => Len::<Slide>::ZERO,
        }
    }

    /// Current playback state: the mode, then the slide and the focus.
    /// Test accessor.
    pub fn playback(&self) -> (PresentMode, PlaybackState) {
        (self.present, self.playback)
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
        frame: &Id<Frame>,
        base: &str,
        delta: &TextDelta,
    ) -> ToileOutcome {
        let element = Id::<Element>::from(frame.clone());
        let Some(em) = self.element_map(&element) else {
            return self.err_no_element(&element);
        };
        // Only a frame carries notes; `children` is seeded at frame birth.
        if em.get(ElementKey::Children).is_none() {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::ContentKindMismatch,
            )]);
        }
        // Skip a pure no-op so an empty delta produces neither a commit nor an
        // undo item.
        if delta.delete.is_zero() && delta.insert.is_empty() {
            return ToileOutcome::display_only(Vec::new());
        }
        let Some(text) = notes_text_or_seed(&em) else {
            return ToileOutcome::display_only(Vec::new());
        };
        let stored = text.to_string();
        if stored != base {
            let typed = splice(base, delta);
            let Merge {
                text: merged,
                edits,
            } = merge_edits(base, &stored, &typed);
            if edits.is_empty() {
                return ToileOutcome::display_only(Vec::new());
            }
            let _ = write_edits(&text, &edits);
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
        let (at, del) = delta.clamped(Len::<Char>::of(text.len_unicode()));
        if !del.is_zero() {
            let _ = text.delete(at.to_usize(), del.to_usize());
        }
        if !delta.insert.is_empty() {
            let _ = text.insert(at.to_usize(), &delta.insert);
        }
        self.doc.commit();
        self.emit_with_update(Vec::new())
    }

    /// The current speaker notes of `frame` as a plain string, `None` when the
    /// frame has no notes yet or does not exist.
    pub fn notes(&self, frame: &Id<Frame>) -> Option<String> {
        self.element_map(&frame.clone().into())
            .and_then(|em| em.get(ElementKey::Notes))
            .and_then(|v| v.into_container().ok())
            .and_then(|c| c.into_text().ok())
            .map(|t| t.to_string())
    }

    // -- PDF export pages (toile-pdf-export) ----------------------------

    /// Build one [`PdfPage`] per presentation slide for an export at `page` size
    /// (`toile-pdf-export`). Each page's camera fits the frame's world rect into
    /// the page with the same `FIT_PAD` the on-screen present glide uses
    /// (`ui::view`). The page is clipped to that frame rect, so the PDF matches
    /// what the audience sees.
    ///
    /// A dangling `Id<Frame>` (present in the deck but with no element) is
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
            let mut cam = ViewportTransform::new(ambre_draw::Size::new(
                f64::from(page.width.get()),
                f64::from(page.height.get()),
            ));
            cam.fit_bounds(rect, FIT_PAD);
            cam
        };

        let slides: Vec<(Id<Frame>, Rect)> = self
            .slide_order()
            .into_iter()
            .filter_map(|f| self.element_rect(&f.clone().into()).map(|r| (f, r)))
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
    pub(super) fn drop_slides(&mut self, deleted: &HashSet<&Id<Element>>) -> bool {
        let list = self.order_list();
        let before = slide_order_of(&list);
        if before.is_empty() {
            return false;
        }
        let kept: Vec<&Id<Frame>> = before
            .iter()
            .filter(|id| !deleted.contains(&Id::<Element>::from((*id).clone())))
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
        self.playback.active = self.playback.active.within(Len::of(kept.len()));
        true
    }

    // -- container helpers -----------------------------------------------------

    /// The slide sequence: a root container, so it exists in every peer's
    /// document from the start and is created by nobody (`TOILE-029`).
    fn order_list(&self) -> LoroMovableList {
        self.doc.get_movable_list(Root::PresentationOrder.as_str())
    }
}

/// A presence sample carrying no slide: what a viewer sends when it stops
/// broadcasting (`TOILE-UI-315`). The peers read the whole live set, so a
/// sample naming no slide is what drops this viewer from the presenters.
fn silent_presence_frame() -> Option<ambre_core::Payload<ambre_core::EphemeralBytes>> {
    match postcard::to_allocvec(&PresencePayload::default()) {
        Ok(bytes) => Some(ambre_core::Payload::<ambre_core::EphemeralBytes>::new(
            bytes,
        )),
        Err(e) => {
            tracing::warn!("Toile: the end-of-broadcast frame failed to encode: {e}");
            None
        }
    }
}

/// Write the presentation defaults into the config root map (`TOILE-029`).
///
/// Called from board construction. The map is a root container, so this
/// writes registers into a container every peer already addresses, never a
/// container of its own.
pub(super) fn seed_presentation_config(doc: &LoroDoc) {
    let cfg = config(doc);
    if cfg.get(ConfigKey::TransitionMs).is_none() {
        let _ = cfg.insert(ConfigKey::TransitionMs, DEFAULT_TRANSITION.get());
        let _ = cfg.insert(ConfigKey::Autoplay, false);
        let _ = cfg.insert(ConfigKey::Loop, false);
    }
}

/// `base` with `delta` applied, its offsets clamped to `base` the way the
/// handler clamps them to the stored note.
fn splice(base: &str, delta: &TextDelta) -> String {
    let (at, del) = delta.clamped(char_len(base));
    let mut out: String = base.chars().take(at.to_usize()).collect();
    out.push_str(&delta.insert);
    out.extend(base.chars().skip((at + del).to_usize()));
    out
}

/// The frame's `notes` LoroText, seeding an empty one on first use. The
/// `em` map is already known to be a frame (checked by the caller). `None`
/// when the map holds no text and refuses a new one, and the edit is dropped.
fn notes_text_or_seed(em: &Fields<ElementKey>) -> Option<LoroText> {
    if let Some(text) = em
        .get(ElementKey::Notes)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_text().ok())
    {
        return Some(text);
    }
    em.insert_container(ElementKey::Notes, LoroText::new()).ok()
}

/// Read the slide order's frame ids, in order.
fn slide_order_of(list: &LoroMovableList) -> Vec<Id<Frame>> {
    let mut out = Vec::new();
    if let LoroValue::List(items) = list.get_deep_value() {
        for v in items.iter() {
            if let LoroValue::String(s) = v {
                out.push(Id::<Frame>::from_text(s.to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod frame_tests {
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::model::ElementKind;
    use crate::logic::state::ToileState;
    use ambre_core::id::{Element, Frame, Id};

    fn create(st: &mut ToileState, kind: ElementKind) -> Id<Element> {
        let out = st.handle_command(ToileCommand::CreateElement {
            kind,
            at: Point::ZERO,
            size: Size::new(100.0, 100.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        out.display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id),
                _ => None,
            })
            .expect("creation yields an id")
    }

    /// A frame reads back as its frame id; a sticky and an unknown id do not.
    #[test]
    fn only_an_existing_frame_reads_as_a_frame() {
        let mut st = ToileState::new();
        let frame = create(&mut st, ElementKind::Frame);
        let sticky = create(&mut st, ElementKind::Sticky);
        assert_eq!(
            st.frame(&frame),
            Some(Id::<Frame>::from_text(frame.as_str().to_owned()))
        );
        assert_eq!(st.frame(&sticky), None, "a sticky is not a frame");
        assert_eq!(
            st.frame(&Id::<Element>::from_text("ghost".to_owned())),
            None,
            "an unknown id is not a frame"
        );
    }
}

#[cfg(test)]
#[cfg(feature = "pdf")]
mod pdf_tests {
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::interop::PageSize;
    use crate::logic::model::ElementKind;
    use crate::logic::state::ToileState;
    use ambre_core::id::{Element, Frame, Id};

    /// Create a frame at `(x, y)` sized `w x h` and return its minted id.
    fn frame_at(st: &mut ToileState, x: f64, y: f64, w: f64, h: f64) -> Id<Frame> {
        let out = st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Frame,
            at: Point::new(x, y),
            size: Size::new(w, h),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        let id = out
            .display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id),
                _ => None,
            })
            .expect("frame creation yields an id");
        st.frame(&id).expect("a frame")
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

    /// A dangling `Id<Frame>` in the deck (no such element) is skipped at
    /// derivation, so it produces no page.
    #[test]
    fn dangling_frame_is_skipped() {
        let mut st = ToileState::new();
        let a = frame_at(&mut st, 0.0, 0.0, 200.0, 100.0);
        st.handle_command(ToileCommand::SetSlideOrder {
            order: vec![a, Id::<Frame>::from_text("ghost".to_owned())],
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

    /// Create a rect shape at `(x, y)` sized `w x h` and return its minted id.
    fn shape_at(st: &mut ToileState, x: f64, y: f64, w: f64, h: f64) -> Id<Element> {
        let out = st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(crate::logic::model::ShapeKind::Rect),
            at: Point::new(x, y),
            size: Size::new(w, h),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        out.display
            .into_iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id),
                _ => None,
            })
            .expect("shape creation yields an id")
    }

    /// Record one page onto a surface that keeps every op.
    fn record(page: &crate::logic::interop::PdfPage) -> ambre_draw::testing::RecordingSurface {
        let mut s = ambre_draw::testing::RecordingSurface::new();
        crate::logic::interop::paint_page(
            &mut s,
            page,
            ambre_canvas::Theme::Light.palette(),
            &mut ambre_draw::FontContext::default(),
            &mut ambre_canvas::TextCache::new(),
            &std::collections::HashMap::new(),
        );
        s
    }

    /// Covers `TOILE-IO-030`: a shape just under the frame is in view of the
    /// page camera, and the page clip leaves it out.
    ///
    /// A wide frame on a 16:9 page leaves room above and below it, so the
    /// neighbour is drawn by the walk. The clip is the frame's own rect.
    #[test]
    fn the_page_clip_leaves_the_neighbour_out() {
        use crate::message::DisplayNode;
        use ambre_draw::testing::DrawOp;

        let mut st = ToileState::new();
        let a = frame_at(&mut st, 0.0, 0.0, 200.0, 50.0);
        let near = shape_at(&mut st, 0.0, 60.0, 200.0, 20.0);
        st.handle_command(ToileCommand::SetSlideOrder {
            order: vec![a.clone()],
        });

        let pages = st.pdf_pages(PageSize::default());
        let page = &pages[0];
        assert!(
            page.viewport
                .nodes
                .iter()
                .any(|n| matches!(n, DisplayNode::Element(e) if e.id == near)),
            "the neighbour is in view of the page camera"
        );
        // The walk clips in board units, under the page camera.
        let frame = st.bounds_of(&a.clone().into()).expect("frame bounds");
        let neighbour = st.bounds_of(&near).expect("shape bounds");

        let ops = record(page);
        let Some(rect) = ops.ops().iter().find_map(|op| match op {
            DrawOp::PushClip { rect } => Some(rect),
            _ => None,
        }) else {
            panic!("the page carries a clip: {:?}", ops.ops());
        };
        assert_eq!(*rect, frame, "the clip is the frame rect");
        assert!(
            rect.intersect(neighbour).area() <= 0.0,
            "the clip keeps part of the neighbour: {rect:?} {neighbour:?}"
        );
    }

    /// Covers `TOILE-IO-040`: the pages follow the deck, in either order.
    ///
    /// Two frames are read back in both orders, so a builder that sorted by
    /// id or by creation fails one of the two.
    #[test]
    fn the_pages_follow_the_playback_order() {
        let mut st = ToileState::new();
        let a = frame_at(&mut st, 0.0, 0.0, 200.0, 100.0);
        let b = frame_at(&mut st, 500.0, 0.0, 200.0, 100.0);
        let ra = st.bounds_of(&a.clone().into());
        let rb = st.bounds_of(&b.clone().into());

        for order in [vec![b.clone(), a.clone()], vec![a.clone(), b.clone()]] {
            st.handle_command(ToileCommand::SetSlideOrder {
                order: order.clone(),
            });
            let clips: Vec<_> = st
                .pdf_pages(PageSize::default())
                .iter()
                .map(|p| p.clip)
                .collect();
            let want: Vec<_> = order
                .iter()
                .map(|f| if *f == a { ra } else { rb })
                .collect();
            assert_eq!(clips, want, "deck {order:?}");
        }
    }

    /// Covers `TOILE-IO-047`: a page draws no grid line, where the same
    /// viewport walked for editing does.
    #[test]
    fn a_page_carries_no_grid() {
        use ambre_draw::testing::{DrawOp, RecordingSurface};

        let mut st = ToileState::new();
        let a = frame_at(&mut st, 0.0, 0.0, 200.0, 100.0);
        st.handle_command(ToileCommand::SetSlideOrder { order: vec![a] });
        let pages = st.pdf_pages(PageSize::default());

        let page = record(&pages[0]);
        let mut board = RecordingSurface::new();
        crate::ui::render::paint_content(
            &mut board,
            &pages[0].viewport,
            &pages[0].camera,
            ambre_canvas::Theme::Light.palette(),
            &mut ambre_draw::FontContext::default(),
            &mut ambre_canvas::TextCache::new(),
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            crate::message::PresentMode::Editing,
        );
        let lines = |s: &RecordingSurface| s.count(|op| matches!(op, DrawOp::StrokeLine { .. }));
        assert!(lines(&board).get() > 0, "the editing walk draws the grid");
        assert_eq!(lines(&page).get(), 0, "the page drew a grid line");
    }
}
