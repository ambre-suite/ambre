//! Track changes: the interception of typing and deleting, and the accept and
//! reject commands (`features/ambre-spec-prose-track-changes.md`).
//!
//! A suggestion is the real text plus a mark of its own key, and an entry in
//! `annots` naming its author. An intercepted edit is one commit, so an undo
//! takes back the text, the mark and the entry together.

use ambre_bus::app::prose::{DocCount, DocOffset, DocSpan};
use ambre_core::SuggestionId;
use loro::{LoroMap, TreeID};

use crate::doc;
use crate::events::{ProseCommand, ProseEvent, ProseOutcome, ProseRefusal, SuggestionKind};
use crate::suggest;

use super::ProseState;

/// What resolving a suggestion does with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    Reject,
    Accept,
}

/// The suggestion this session is extending, and the span it covers in one
/// block. The next edit joins it when it touches that span
/// (`PROSE-FEAT-085`), and starts another when it does not (`PROSE-FEAT-086`).
#[derive(Clone, Copy, Debug)]
pub(super) struct Burst {
    id: SuggestionId,
    kind: SuggestionKind,
    block: TreeID,
    start: DocOffset,
    end: DocOffset,
}

/// What an intercepted delete does to one segment (`PROSE-FEAT-267`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Treatment {
    /// The caller's own pending insertion: the text goes (`PROSE-FEAT-087`).
    Remove,
    /// Already proposed for deletion: left as it is (`PROSE-FEAT-268`).
    Keep,
    /// Anything else: marked as a suggested deletion (`PROSE-FEAT-088`).
    Mark,
}

/// Whether suggestion mode lets a command through. Typing and deleting are
/// intercepted; reading, moving, commenting, the suggestion commands and the
/// history pass as they are. Every structure and format command is refused.
pub(super) fn passes_while_suggesting(command: &ProseCommand) -> bool {
    !command.is_mutating()
        || matches!(
            command,
            ProseCommand::InsertText { .. }
                | ProseCommand::DeleteText { .. }
                | ProseCommand::DeleteRange { .. }
                | ProseCommand::StepTableCell { .. }
                | ProseCommand::StampModified { .. }
                | ProseCommand::AddComment { .. }
                | ProseCommand::ReplyToComment { .. }
                | ProseCommand::ResolveComment { .. }
                | ProseCommand::ReopenComment { .. }
                | ProseCommand::DeleteComment { .. }
                | ProseCommand::DeleteReply { .. }
                | ProseCommand::EditCommentReply { .. }
                | ProseCommand::EnableTrackChanges { .. }
                | ProseCommand::AcceptSuggestion { .. }
                | ProseCommand::RejectSuggestion { .. }
                | ProseCommand::AcceptAllSuggestions
                | ProseCommand::RejectAllSuggestions
                | ProseCommand::Undo
                | ProseCommand::Redo
        )
}

/// What resolving one suggestion did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Settled {
    /// The suggestion was no longer in the document: nothing was written.
    Gone,
    /// Its spans and its entry were acted on.
    Written,
}

impl ProseState {
    /// Turn suggestion mode on or off. Nothing pending is touched
    /// (`PROSE-FEAT-089`), and the burst closes: the next edit starts a
    /// suggestion of its own.
    pub(super) fn enable_track_changes(&mut self, enabled: bool) -> ProseOutcome {
        if enabled && self.my_user_id.is_none() {
            return ProseOutcome::refused(ProseRefusal::NoIdentity);
        }
        self.suggesting = enabled;
        self.burst = None;
        ProseOutcome {
            display: vec![self.suggestions_event()],
            update: None,
            ephemeral: None,
        }
    }

    /// Whether this session is suggesting.
    #[must_use]
    pub fn is_suggesting(&self) -> bool {
        self.suggesting
    }

    /// Every pending suggestion, as the view lists it.
    #[must_use]
    pub fn suggestions(&self) -> Vec<crate::events::Suggestion> {
        suggest::pending(&self.doc)
    }

    pub(super) fn suggestions_event(&self) -> ProseEvent {
        ProseEvent::SuggestionsUpdated {
            suggesting: self.suggesting,
            suggestions: self.suggestions(),
        }
    }

    /// Typing while suggesting: the text goes in, marked as a suggested
    /// insertion (`PROSE-FEAT-265`), stamped with the session's identity
    /// (`PROSE-FEAT-262`).
    pub(super) fn suggest_insert(
        &mut self,
        block: &TreeID,
        pos: DocOffset,
        text: &str,
    ) -> ProseOutcome {
        if let Some(refused) = self.refuse_suggesting_in(block) {
            return refused;
        }
        let Some(body) = self.body_of(block) else {
            return Self::err_no_block(block);
        };
        if pos.to_usize() > body.len_unicode() {
            return Self::err_bad_range();
        }
        let length = DocCount::of(text);
        if length.is_zero() {
            return ProseOutcome::nothing();
        }
        let id = match self.joining(SuggestionKind::Insertion, block, pos, pos) {
            Some(id) => id,
            None => match self.write_entry() {
                Ok(id) => id,
                Err(e) => return Self::err_write(e),
            },
        };
        let end = pos + length;
        let inserted = DocSpan::new(pos, end);
        if let Err(e) = body.insert(pos.to_usize(), text) {
            return Self::err_write(e);
        }
        if let Err(e) = suggest::strip_inherited(&body, inserted) {
            return Self::err_write(e);
        }
        let key = suggest::mark_key(SuggestionKind::Insertion, &id);
        if let Err(e) = body.mark(inserted.to_usize_range(), key.as_str(), id.to_hyphenated()) {
            return Self::err_write(e);
        }
        self.extend_burst(id, SuggestionKind::Insertion, block, pos, end);
        self.commit_with(Vec::new())
    }

    /// Deleting while suggesting, over one block or across several. The range
    /// is cut into segments by what covers each one before any text is
    /// touched (`PROSE-FEAT-267`). No block is joined or removed: the text
    /// stays until someone accepts.
    pub(super) fn suggest_delete(
        &mut self,
        anchor: (TreeID, DocOffset),
        focus: (TreeID, DocOffset),
    ) -> ProseOutcome {
        let pieces = match self.pieces(anchor, focus) {
            Ok(pieces) => pieces,
            Err(outcome) => return outcome,
        };
        if pieces.iter().all(|(_, start, end)| start == end) {
            return ProseOutcome::nothing();
        }
        let me = self.my_user_id;
        let mut plan: Vec<(TreeID, DocOffset, DocOffset, Treatment)> = Vec::new();
        for (block, start, end) in &pieces {
            for run in suggest::runs(&self.doc, block, *start, *end) {
                let treatment = self.treatment(&run, me);
                plan.push((*block, run.start, run.end, treatment));
            }
        }
        let marked: Vec<&(TreeID, DocOffset, DocOffset, Treatment)> = plan
            .iter()
            .filter(|(.., treatment)| *treatment == Treatment::Mark)
            .collect();
        let id = match marked.first() {
            None => None,
            Some((block, start, end, _)) => {
                match self.joining(SuggestionKind::Deletion, block, *start, *end) {
                    Some(id) => Some(id),
                    None => match self.write_entry() {
                        Ok(id) => Some(id),
                        Err(e) => return Self::err_write(e),
                    },
                }
            }
        };
        // Back to front, so a removal never moves an offset still to come.
        for (block, start, end, treatment) in plan.iter().rev() {
            let body = doc::get_block_text(&self.doc, block);
            let range = DocSpan::new(*start, *end).to_usize_range();
            let written = match (treatment, id) {
                (Treatment::Remove, _) => body.delete(range.start, range.len()),
                (Treatment::Mark, Some(id)) => body.mark(
                    range,
                    suggest::mark_key(SuggestionKind::Deletion, &id).as_str(),
                    id.to_hyphenated(),
                ),
                _ => Ok(()),
            };
            if let Err(e) = written {
                return Self::err_write(e);
            }
        }
        self.after_delete(&plan, id);
        self.commit_with(Vec::new())
    }

    /// Apply every span of suggestion `id`, or cancel it (`PROSE-FEAT-083`,
    /// `PROSE-FEAT-273`). A suggestion no longer in the document is nothing
    /// to do, so a second accept changes nothing (`PROSE-FEAT-084`,
    /// `PROSE-FEAT-274`).
    pub(super) fn resolve_suggestion(
        &mut self,
        id: &SuggestionId,
        verdict: Verdict,
    ) -> ProseOutcome {
        match self.settle_suggestion(id, verdict) {
            Ok(Settled::Gone) => ProseOutcome::nothing(),
            Ok(Settled::Written) => {
                let event = self.suggestions_event();
                self.commit_with(vec![event])
            }
            Err(e) => Self::err_write(e),
        }
    }

    /// Resolve every pending suggestion in one commit, each exactly as its
    /// own command would (`PROSE-FEAT-276`). The set is the one pending when
    /// the command starts: the command runs to its end before any remote
    /// delta is read.
    pub(super) fn resolve_all(&mut self, verdict: Verdict) -> ProseOutcome {
        let ids: Vec<SuggestionId> = self
            .suggestions()
            .into_iter()
            .map(|s| s.suggestion_id)
            .collect();
        let mut wrote = false;
        for id in &ids {
            match self.settle_suggestion(id, verdict) {
                Ok(settled) => wrote |= settled == Settled::Written,
                Err(e) => return Self::err_write(e),
            }
        }
        if !wrote {
            return ProseOutcome::nothing();
        }
        let event = self.suggestions_event();
        self.commit_with(vec![event])
    }

    /// Resolve one suggestion without committing, and say whether anything
    /// was written.
    ///
    /// Each span is acted on from its mark, never from the current text around
    /// it (`PROSE-FEAT-010`). Only this identity's key is read, so a span
    /// another suggestion carries is left alone (`PROSE-FEAT-271`,
    /// `PROSE-FEAT-275`). A rejected insertion leaves no trace: its text, its
    /// mark and its entry all go (`PROSE-FEAT-004`).
    fn settle_suggestion(
        &mut self,
        id: &SuggestionId,
        verdict: Verdict,
    ) -> Result<Settled, doc::WriteError> {
        let spans: Vec<_> = suggest::spans(&self.doc, &suggest::text_blocks(&self.doc))
            .into_iter()
            .filter(|(other, ..)| other == id)
            .collect();
        let entry = suggest::entry(&self.doc, id);
        if spans.is_empty() && entry.is_none() {
            return Ok(Settled::Gone);
        }
        for (_, kind, block, start, end) in spans.iter().rev() {
            let body = doc::get_block_text(&self.doc, block);
            let range = DocSpan::new(*start, *end).to_usize_range();
            let removes = matches!(
                (kind, verdict),
                (SuggestionKind::Insertion, Verdict::Reject)
                    | (SuggestionKind::Deletion, Verdict::Accept)
            );
            let written = if removes {
                body.delete(range.start, range.len())
            } else {
                body.unmark(range, suggest::mark_key(*kind, id).as_str())
            };
            written?;
        }
        if entry.is_some() {
            doc::annots(&self.doc).delete(&id.to_hyphenated())?;
        }
        if self.burst.is_some_and(|burst| burst.id == *id) {
            self.burst = None;
        }
        Ok(Settled::Written)
    }

    /// A header, a footer or a note is refused while suggesting. The pending
    /// list walks the flow alone, so a suggestion there could never be
    /// accepted.
    fn refuse_suggesting_in(&self, block: &TreeID) -> Option<ProseOutcome> {
        doc::is_aside(&doc::blocks(&self.doc), block)
            .then(|| ProseOutcome::refused(ProseRefusal::NotWhileSuggesting))
    }

    /// The per-block segments of a delete from `anchor` to `focus`, in
    /// document order.
    fn pieces(
        &self,
        anchor: (TreeID, DocOffset),
        focus: (TreeID, DocOffset),
    ) -> Result<Vec<(TreeID, DocOffset, DocOffset)>, ProseOutcome> {
        for (block, _) in [anchor, focus] {
            if let Some(refused) = self.refuse_suggesting_in(&block) {
                return Err(refused);
            }
        }
        let length_of = |block: &TreeID| -> Option<DocOffset> {
            let body = self.body_of(block)?;
            Some(DocCount::from_len(body.len_unicode()).as_end())
        };
        if anchor.0 == focus.0 {
            let length = length_of(&anchor.0).ok_or_else(|| Self::err_no_block(&anchor.0))?;
            let (start, end) = (anchor.1.min(focus.1), anchor.1.max(focus.1));
            if end > length {
                return Err(Self::err_bad_range());
            }
            return Ok(vec![(anchor.0, start, end)]);
        }
        let order = self.document_order();
        let at = |block: &TreeID| order.iter().position(|id| id == block);
        let (Some(anchor_at), Some(focus_at)) = (at(&anchor.0), at(&focus.0)) else {
            return Err(Self::err_no_block(&anchor.0));
        };
        let ((head, head_off), (tail, tail_off)) = if anchor_at <= focus_at {
            (anchor, focus)
        } else {
            (focus, anchor)
        };
        let (first, last) = (anchor_at.min(focus_at), anchor_at.max(focus_at));
        let mut out = Vec::new();
        for block in order.get(first..=last).unwrap_or_default() {
            let Some(length) = length_of(block) else {
                continue;
            };
            let start = if *block == head {
                head_off
            } else {
                DocOffset::ZERO
            };
            let end = if *block == tail { tail_off } else { length };
            if start > length || end > length {
                return Err(Self::err_bad_range());
            }
            out.push((*block, start, end.max(start)));
        }
        Ok(out)
    }

    /// What a delete does to one run, read from the entries its marks point
    /// at and never from where the run sits (`PROSE-FEAT-260`).
    fn treatment(&self, run: &suggest::Run, me: Option<ambre_core::UserId>) -> Treatment {
        let owned_by_me = |id: &SuggestionId| {
            suggest::entry(&self.doc, id)
                .and_then(|entry| suggest::owner(&entry))
                .is_some_and(|owner| Some(owner) == me)
        };
        if run
            .marks
            .iter()
            .any(|(kind, _)| *kind == SuggestionKind::Deletion)
        {
            return Treatment::Keep;
        }
        let mine = run
            .marks
            .iter()
            .any(|(kind, id)| *kind == SuggestionKind::Insertion && owned_by_me(id));
        if mine {
            Treatment::Remove
        } else {
            Treatment::Mark
        }
    }

    /// The suggestion an edit of `kind` over `start..end` in `block` joins.
    /// It is the open burst, when that burst is of the same kind and block,
    /// touches the range, and is still in the document.
    fn joining(
        &self,
        kind: SuggestionKind,
        block: &TreeID,
        start: DocOffset,
        end: DocOffset,
    ) -> Option<SuggestionId> {
        let burst = self.burst?;
        let touches =
            burst.kind == kind && burst.block == *block && start <= burst.end && end >= burst.start;
        (touches && suggest::entry(&self.doc, &burst.id).is_some()).then_some(burst.id)
    }

    fn extend_burst(
        &mut self,
        id: SuggestionId,
        kind: SuggestionKind,
        block: &TreeID,
        start: DocOffset,
        end: DocOffset,
    ) {
        self.burst = Some(match self.burst {
            Some(burst) if burst.id == id => Burst {
                start: burst.start.min(start),
                end: match kind {
                    // Typed text pushes the end of the burst along with it.
                    SuggestionKind::Insertion => burst.end + end.count_from(start),
                    SuggestionKind::Deletion => burst.end.max(end),
                },
                ..burst
            },
            _ => Burst {
                id,
                kind,
                block: *block,
                start,
                end,
            },
        });
    }

    /// The burst after a delete: a marked deletion extends or opens one, and
    /// removing the caller's own typed text shrinks the insertion it came
    /// from.
    fn after_delete(
        &mut self,
        plan: &[(TreeID, DocOffset, DocOffset, Treatment)],
        id: Option<SuggestionId>,
    ) {
        let removed = plan
            .iter()
            .filter(|(.., treatment)| *treatment == Treatment::Remove)
            .fold(DocCount::ZERO, |sum, (_, start, end, _)| {
                sum + end.count_from(*start)
            });
        match id {
            Some(id) => {
                let first = plan.iter().find(|(.., t)| *t == Treatment::Mark);
                if let Some((block, start, end, _)) = first {
                    self.extend_burst(id, SuggestionKind::Deletion, block, *start, *end);
                }
            }
            None => {
                if let Some(burst) = self.burst.as_mut()
                    && burst.kind == SuggestionKind::Insertion
                {
                    burst.end = burst.end.saturating_sub(removed).max(burst.start);
                }
            }
        }
    }

    /// A new suggestion's entry: its kind and its author, from the session's
    /// identity and never from a command (`PROSE-FEAT-262`,
    /// `PROSE-FEAT-082`).
    fn write_entry(&self) -> Result<SuggestionId, doc::WriteError> {
        let user = self.my_user_id.ok_or(doc::WriteError::NoIdentity)?;
        let id: SuggestionId = self.ids.mint();
        let entry = doc::annots(&self.doc)
            .insert_container(&id.to_hyphenated(), LoroMap::new())
            .map(doc::Fields::<doc::AnnotKey>::of)?;
        entry.insert(doc::AnnotKey::Kind, doc::AnnotKind::Suggestion.as_str())?;
        entry.insert(
            doc::AnnotKey::Author,
            self.my_name
                .clone()
                .map(ambre_core::DisplayName::into_string)
                .unwrap_or_default(),
        )?;
        entry.insert(doc::AnnotKey::User, user.to_string())?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BlockId, ProseEvent, Suggestion};
    use ambre_core::UserId;
    use proptest::prelude::*;

    const ALICE: UserId = UserId::from_bytes([1; 16]);
    const BOB: UserId = UserId::from_bytes([2; 16]);

    fn refusal(outcome: &ProseOutcome) -> Option<ProseRefusal> {
        outcome.display.iter().find_map(|event| match event {
            ProseEvent::Error(error) => Some(error.reason.clone()),
            _ => None,
        })
    }

    /// A document of one paragraph per text, written by `user` with the
    /// mode off.
    fn document(
        peer: ambre_core::PeerId,
        user: UserId,
        texts: &[&str],
    ) -> (ProseState, Vec<BlockId>) {
        let mut state = ProseState::with_peer(peer).expect("a document");
        state.set_user_id(user);
        state.set_display_name(ambre_core::DisplayName::new(&format!("user {peer}")).unwrap());
        let first = state.root_blocks()[0];
        let mut ids = vec![first];
        for _ in texts.iter().skip(1) {
            let last = *ids.last().expect("a block");
            let at = state.text_of(&last).unwrap_or_default().chars().count() as u32;
            let outcome = state.handle_command(ProseCommand::SplitBlock {
                block_id: last,
                pos: DocOffset::new(at),
            });
            let new = outcome
                .display
                .iter()
                .find_map(|e| match e {
                    ProseEvent::BlockSplit { new_block, .. } => Some(*new_block),
                    _ => None,
                })
                .expect("a split");
            ids.push(new);
        }
        for (id, text) in ids.iter().zip(texts) {
            state.handle_command(ProseCommand::InsertText {
                block_id: *id,
                pos: DocOffset::ZERO,
                text: (*text).to_owned(),
            });
        }
        (state, ids)
    }

    fn join(from: &ProseState, peer: ambre_core::PeerId, user: UserId) -> ProseState {
        let mut state = ProseState::with_peer(peer).expect("a document");
        state.set_user_id(user);
        state
            .import_snapshot(&from.export_snapshot())
            .expect("the seed imports");
        state
    }

    fn suggest(state: &mut ProseState) {
        state.handle_command(ProseCommand::EnableTrackChanges { enabled: true });
    }

    fn type_at(state: &mut ProseState, block: BlockId, pos: DocOffset, text: &str) -> ProseOutcome {
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos,
            text: text.to_owned(),
        })
    }

    fn delete(state: &mut ProseState, block: BlockId, range: (u32, u32)) -> ProseOutcome {
        state.handle_command(ProseCommand::DeleteText {
            block_id: block,
            range: DocSpan::new(DocOffset::new(range.0), DocOffset::new(range.1)),
        })
    }

    fn text(state: &ProseState, block: BlockId) -> String {
        state.text_of(&block).unwrap_or_default()
    }

    fn only(state: &ProseState) -> Suggestion {
        let all = state.suggestions();
        assert_eq!(all.len(), 1, "one suggestion in {all:?}");
        all.into_iter().next().expect("one")
    }

    fn spans(s: &Suggestion) -> Vec<(u32, u32)> {
        s.spans
            .iter()
            .map(|span| (span.start.get(), span.end.get()))
            .collect()
    }

    fn exchange(
        a: &mut ProseState,
        b: &mut ProseState,
        from_a: &ProseOutcome,
        from_b: &ProseOutcome,
    ) {
        if let Some(delta) = &from_b.update {
            a.apply_remote(delta);
        }
        if let Some(delta) = &from_a.update {
            b.apply_remote(delta);
        }
    }

    /// Typing while suggesting lands the text, marked as an insertion by the
    /// session's user (`PROSE-FEAT-263`, `PROSE-FEAT-265`, `PROSE-FEAT-262`).
    #[test]
    fn typed_text_becomes_a_suggested_insertion_by_its_author() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello"]);
        suggest(&mut state);
        type_at(&mut state, ids[0], DocOffset::new(5), " world");
        assert_eq!(text(&state, ids[0]), "Hello world");
        let s = only(&state);
        assert_eq!(s.kind, SuggestionKind::Insertion);
        assert_eq!(s.user_id, ALICE);
        assert_eq!(s.author.clone().unwrap(), "user 1");
        assert_eq!(spans(&s), vec![(5, 11)]);
    }

    /// A suggestion is the document's own text under a mark: the block holds
    /// one copy of it, and nothing else does (`PROSE-FEAT-257`).
    #[test]
    fn a_suggestion_is_a_mark_over_the_text_itself() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["ab"]);
        suggest(&mut state);
        type_at(&mut state, ids[0], DocOffset::new(1), "X");
        assert_eq!(text(&state, ids[0]), "aXb");
        assert_eq!(state.root_blocks().len(), 1);
        let marks = format!("{:?}", state.marks_of(&ids[0]));
        assert!(marks.contains("si:"), "{marks}");
    }

    /// With the mode off, an edit reaches the text with no mark, and turning
    /// it off applies and cancels nothing (`PROSE-FEAT-264`, `PROSE-FEAT-089`).
    #[test]
    fn turning_the_mode_off_leaves_the_pending_ones_and_marks_nothing_new() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello"]);
        suggest(&mut state);
        type_at(&mut state, ids[0], DocOffset::new(5), "!");
        let before = state.suggestions();
        state.handle_command(ProseCommand::EnableTrackChanges { enabled: false });
        assert_eq!(state.suggestions(), before);
        type_at(&mut state, ids[0], DocOffset::ZERO, ">");
        assert_eq!(text(&state, ids[0]), ">Hello!");
        assert_eq!(state.suggestions().len(), 1);
        assert_eq!(spans(&only(&state)), vec![(6, 7)]);
    }

    /// Adjacent keystrokes share one identity, and a keystroke away from the
    /// burst starts another (`PROSE-FEAT-086`).
    #[test]
    fn a_keystroke_away_from_the_burst_starts_a_new_suggestion() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello world"]);
        suggest(&mut state);
        type_at(&mut state, ids[0], DocOffset::new(5), ",");
        type_at(&mut state, ids[0], DocOffset::new(6), " my");
        assert_eq!(state.suggestions().len(), 1);
        type_at(&mut state, ids[0], DocOffset::ZERO, ">");
        assert_eq!(state.suggestions().len(), 2);
    }

    /// Typing inside another peer's suggestion never grows it
    /// (`PROSE-FEAT-266`).
    #[test]
    fn typing_inside_another_suggestion_leaves_its_span_alone() {
        let (mut alice, ids) = document(ambre_core::PeerId(1), ALICE, &["ab"]);
        let mut bob = join(&alice, ambre_core::PeerId(2), BOB);
        suggest(&mut alice);
        suggest(&mut bob);
        let typed = type_at(&mut alice, ids[0], DocOffset::new(1), "XYZ");
        bob.apply_remote(&typed.update.expect("a delta"));
        type_at(&mut bob, ids[0], DocOffset::new(2), "q");
        let all = bob.suggestions();
        let alices = all.iter().find(|s| s.user_id == ALICE).expect("Alice's");
        let bobs = all.iter().find(|s| s.user_id == BOB).expect("Bob's");
        assert_eq!(text(&bob, ids[0]), "aXqYZb");
        assert_eq!(spans(alices), vec![(1, 2), (3, 5)]);
        assert_eq!(spans(bobs), vec![(2, 3)]);
    }

    /// With the mode off, text typed inside a suggestion is plain text: it
    /// joins no suggestion (`PROSE-FEAT-304`).
    #[test]
    fn plain_typing_inside_a_suggestion_joins_none() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["ab"]);
        suggest(&mut state);
        type_at(&mut state, ids[0], DocOffset::new(1), "XY");
        state.handle_command(ProseCommand::EnableTrackChanges { enabled: false });
        type_at(&mut state, ids[0], DocOffset::new(2), "q");
        assert_eq!(text(&state, ids[0]), "aXqYb");
        assert_eq!(spans(&only(&state)), vec![(1, 2), (3, 4)]);
        let id = only(&state).suggestion_id;
        state.handle_command(ProseCommand::RejectSuggestion { suggestion_id: id });
        assert_eq!(text(&state, ids[0]), "aqb");
    }

    /// A delete over the caller's own suggested text and over original text
    /// is cut in two. The first goes, and the second is marked
    /// (`PROSE-FEAT-267`, `PROSE-FEAT-087`, `PROSE-FEAT-088`).
    #[test]
    fn a_delete_removes_its_own_suggestion_and_marks_the_rest() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello"]);
        suggest(&mut state);
        type_at(&mut state, ids[0], DocOffset::new(5), "XY");
        delete(&mut state, ids[0], (3, 7));
        assert_eq!(text(&state, ids[0]), "Hello");
        let s = only(&state);
        assert_eq!(s.kind, SuggestionKind::Deletion);
        assert_eq!(spans(&s), vec![(3, 5)]);
    }

    /// Consecutive backspaces grow one deletion rather than several.
    #[test]
    fn backspaces_in_a_row_are_one_deletion() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello"]);
        suggest(&mut state);
        for at in (2..5).rev() {
            delete(&mut state, ids[0], (at, at + 1));
        }
        assert_eq!(spans(&only(&state)), vec![(2, 5)]);
    }

    /// Deleting another peer's insertion marks it, so the span carries both
    /// marks: ownership is read from the entry, never from the position
    /// (`PROSE-FEAT-258`, `PROSE-FEAT-260`).
    #[test]
    fn deleting_another_peers_insertion_marks_it_rather_than_removing_it() {
        let (mut alice, ids) = document(ambre_core::PeerId(1), ALICE, &["ab"]);
        let mut bob = join(&alice, ambre_core::PeerId(2), BOB);
        suggest(&mut alice);
        suggest(&mut bob);
        let typed = type_at(&mut alice, ids[0], DocOffset::new(1), "XY");
        bob.apply_remote(&typed.update.expect("a delta"));
        delete(&mut bob, ids[0], (1, 3));
        assert_eq!(text(&bob, ids[0]), "aXYb");
        let kinds: Vec<SuggestionKind> = bob.suggestions().iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![SuggestionKind::Insertion, SuggestionKind::Deletion]
        );
        for s in bob.suggestions() {
            assert_eq!(spans(&s), vec![(1, 3)]);
        }
    }

    /// A delete over text already proposed for deletion leaves that mark as it
    /// is (`PROSE-FEAT-268`).
    #[test]
    fn a_delete_never_takes_over_another_peers_deletion() {
        let (mut alice, ids) = document(ambre_core::PeerId(1), ALICE, &["abcd"]);
        let mut bob = join(&alice, ambre_core::PeerId(2), BOB);
        suggest(&mut alice);
        suggest(&mut bob);
        let marked = delete(&mut alice, ids[0], (1, 3));
        bob.apply_remote(&marked.update.expect("a delta"));
        delete(&mut bob, ids[0], (0, 4));
        let all = bob.suggestions();
        let alices = all.iter().find(|s| s.user_id == ALICE).expect("Alice's");
        let bobs = all.iter().find(|s| s.user_id == BOB).expect("Bob's");
        assert_eq!(spans(alices), vec![(1, 3)]);
        assert_eq!(spans(bobs), vec![(0, 1), (3, 4)]);
    }

    /// A delete across a paragraph break is one suggestion over several
    /// spans, and accepting it acts on all of them
    /// (`PROSE-FEAT-259`, `PROSE-FEAT-083`).
    #[test]
    fn a_deletion_across_a_break_is_one_suggestion_and_one_accept() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello", "world"]);
        suggest(&mut state);
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[0], DocOffset::new(3)),
            focus: (ids[1], DocOffset::new(2)),
        });
        let s = only(&state);
        assert_eq!(s.spans.len(), 2);
        assert_eq!(text(&state, ids[0]), "Hello");
        state.handle_command(ProseCommand::AcceptSuggestion {
            suggestion_id: s.suggestion_id,
        });
        assert_eq!(text(&state, ids[0]), "Hel");
        assert_eq!(text(&state, ids[1]), "rld");
        assert!(state.suggestions().is_empty());
    }

    /// Rejecting a deletion across a break restores every span
    /// (`PROSE-FEAT-273`, `PROSE-FEAT-272`).
    #[test]
    fn rejecting_a_deletion_across_a_break_keeps_every_span() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello", "world"]);
        suggest(&mut state);
        state.handle_command(ProseCommand::DeleteRange {
            anchor: (ids[1], DocOffset::new(2)),
            focus: (ids[0], DocOffset::new(3)),
        });
        let id = only(&state).suggestion_id;
        state.handle_command(ProseCommand::RejectSuggestion { suggestion_id: id });
        assert_eq!(text(&state, ids[0]), "Hello");
        assert_eq!(text(&state, ids[1]), "world");
        assert!(state.suggestions().is_empty());
        assert!(!format!("{:?}", state.marks_of(&ids[1])).contains("sd:"));
    }

    /// The four outcomes, each applied twice: the second changes nothing
    /// (`PROSE-FEAT-269`, `PROSE-FEAT-270`, `PROSE-FEAT-004`,
    /// `PROSE-FEAT-084`, `PROSE-FEAT-274`, `PROSE-FEAT-010`).
    #[test]
    fn each_outcome_is_the_recorded_change_and_idempotent() {
        let cases = [
            (true, true, "Hello world"),
            (true, false, "Hello"),
            (false, true, "Hel"),
            (false, false, "Hello"),
        ];
        for (insertion, accept, expected) in cases {
            let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello"]);
            suggest(&mut state);
            if insertion {
                type_at(&mut state, ids[0], DocOffset::new(5), " world");
            } else {
                delete(&mut state, ids[0], (3, 5));
            }
            let id = only(&state).suggestion_id;
            let command = || {
                if accept {
                    ProseCommand::AcceptSuggestion { suggestion_id: id }
                } else {
                    ProseCommand::RejectSuggestion { suggestion_id: id }
                }
            };
            state.handle_command(command());
            assert_eq!(text(&state, ids[0]), expected);
            assert!(state.suggestions().is_empty());
            let again = state.handle_command(command());
            assert!(again.update.is_none(), "a second resolve writes nothing");
            assert_eq!(text(&state, ids[0]), expected);
            let marks = format!("{:?}", state.marks_of(&ids[0]));
            assert!(!marks.contains("si:") && !marks.contains("sd:"), "{marks}");
        }
    }

    /// Resolving one suggestion leaves another over the same text
    /// untouched (`PROSE-FEAT-271`, `PROSE-FEAT-275`).
    #[test]
    fn resolving_one_suggestion_leaves_the_other_one_its_span() {
        for accept in [true, false] {
            let (mut alice, ids) = document(ambre_core::PeerId(1), ALICE, &["abcd"]);
            let mut bob = join(&alice, ambre_core::PeerId(2), BOB);
            suggest(&mut alice);
            suggest(&mut bob);
            let a = delete(&mut alice, ids[0], (0, 1));
            let b = delete(&mut bob, ids[0], (2, 4));
            exchange(&mut alice, &mut bob, &a, &b);
            let alices = alice
                .suggestions()
                .into_iter()
                .find(|s| s.user_id == ALICE)
                .expect("Alice's")
                .suggestion_id;
            alice.handle_command(if accept {
                ProseCommand::AcceptSuggestion {
                    suggestion_id: alices,
                }
            } else {
                ProseCommand::RejectSuggestion {
                    suggestion_id: alices,
                }
            });
            let left = only(&alice);
            assert_eq!(left.user_id, BOB);
            let shift = u32::from(accept);
            assert_eq!(spans(&left), vec![(2 - shift, 4 - shift)]);
        }
    }

    /// A span carrying both marks loses its text when the insertion is
    /// rejected, or when the deletion is accepted.
    #[test]
    fn a_span_under_both_marks_goes_when_either_side_removes_it() {
        for (resolve_insertion, accept) in [(true, false), (false, true)] {
            let (mut alice, ids) = document(ambre_core::PeerId(1), ALICE, &["ab"]);
            let mut bob = join(&alice, ambre_core::PeerId(2), BOB);
            suggest(&mut alice);
            suggest(&mut bob);
            let typed = type_at(&mut alice, ids[0], DocOffset::new(1), "XY");
            bob.apply_remote(&typed.update.expect("a delta"));
            delete(&mut bob, ids[0], (1, 3));
            let kind = if resolve_insertion {
                SuggestionKind::Insertion
            } else {
                SuggestionKind::Deletion
            };
            let id = bob
                .suggestions()
                .into_iter()
                .find(|s| s.kind == kind)
                .expect("the suggestion")
                .suggestion_id;
            bob.handle_command(if accept {
                ProseCommand::AcceptSuggestion { suggestion_id: id }
            } else {
                ProseCommand::RejectSuggestion { suggestion_id: id }
            });
            assert_eq!(text(&bob, ids[0]), "ab");
        }
    }

    /// Accept-all and reject-all resolve each pending suggestion as its own
    /// command would, and touch no other text
    /// (`PROSE-FEAT-276`, `PROSE-FEAT-277`).
    #[test]
    fn resolving_all_is_each_resolved_alone() {
        for accept in [true, false] {
            let build = || {
                let (mut state, ids) =
                    document(ambre_core::PeerId(1), ALICE, &["one two three", "four"]);
                suggest(&mut state);
                type_at(&mut state, ids[0], DocOffset::ZERO, ">");
                delete(&mut state, ids[0], (5, 8));
                type_at(&mut state, ids[1], DocOffset::new(4), "!");
                (state, ids)
            };
            let (mut all, ids) = build();
            all.handle_command(if accept {
                ProseCommand::AcceptAllSuggestions
            } else {
                ProseCommand::RejectAllSuggestions
            });
            let (mut each, _) = build();
            for s in each.suggestions() {
                each.handle_command(if accept {
                    ProseCommand::AcceptSuggestion {
                        suggestion_id: s.suggestion_id,
                    }
                } else {
                    ProseCommand::RejectSuggestion {
                        suggestion_id: s.suggestion_id,
                    }
                });
            }
            for id in &ids {
                assert_eq!(text(&all, *id), text(&each, *id));
            }
            let expected = if accept {
                [">one  three", "four!"]
            } else {
                ["one two three", "four"]
            };
            assert_eq!([text(&all, ids[0]), text(&all, ids[1])], expected);
            assert!(all.suggestions().is_empty());
        }
    }

    /// A suggestion keeps its identity across a remote delta and a peer's
    /// edit inside it (`PROSE-FEAT-082`).
    #[test]
    fn a_suggestion_keeps_its_identity_on_every_peer() {
        let (mut alice, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello"]);
        let mut bob = join(&alice, ambre_core::PeerId(2), BOB);
        suggest(&mut alice);
        let typed = type_at(&mut alice, ids[0], DocOffset::new(5), " world");
        bob.apply_remote(&typed.update.expect("a delta"));
        let id = only(&alice).suggestion_id;
        assert_eq!(only(&bob).suggestion_id, id);
        let edit = type_at(&mut bob, ids[0], DocOffset::ZERO, ">");
        alice.apply_remote(&edit.update.expect("a delta"));
        assert_eq!(only(&alice).suggestion_id, id);
    }

    /// One peer accepts while the other rejects the same suggestion: both end
    /// on one document, the change applied whole or not at all
    /// (`PROSE-FEAT-011`).
    #[test]
    fn a_concurrent_accept_and_reject_converge_on_one_outcome() {
        for insertion in [true, false] {
            let (mut alice, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello"]);
            suggest(&mut alice);
            if insertion {
                type_at(&mut alice, ids[0], DocOffset::new(5), " world");
            } else {
                delete(&mut alice, ids[0], (1, 4));
            }
            let mut bob = join(&alice, ambre_core::PeerId(2), BOB);
            let id = only(&alice).suggestion_id;
            let a = alice.handle_command(ProseCommand::AcceptSuggestion { suggestion_id: id });
            let b = bob.handle_command(ProseCommand::RejectSuggestion { suggestion_id: id });
            exchange(&mut alice, &mut bob, &a, &b);
            assert_eq!(text(&alice, ids[0]), text(&bob, ids[0]));
            let expected = if insertion { "Hello" } else { "Ho" };
            assert_eq!(text(&alice, ids[0]), expected, "the removal wins");
            assert!(alice.suggestions().is_empty() && bob.suggestions().is_empty());
        }
    }

    /// Structure and format commands wait for the mode to be off.
    #[test]
    fn a_structure_command_is_refused_while_suggesting() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello"]);
        suggest(&mut state);
        for command in [
            ProseCommand::SplitBlock {
                block_id: ids[0],
                pos: DocOffset::new(2),
            },
            ProseCommand::ApplyMark {
                block_id: ids[0],
                range: DocSpan::from_usize_range(0..2),
                key: "b".into(),
                value: None,
            },
        ] {
            let outcome = state.handle_command(command);
            assert_eq!(refusal(&outcome), Some(ProseRefusal::NotWhileSuggesting));
        }
        assert_eq!(state.root_blocks().len(), 1);
    }

    /// A session with no user cannot suggest: an edit it made would name no
    /// author (`PROSE-FEAT-262`).
    #[test]
    fn a_session_with_no_user_cannot_suggest() {
        let mut state = ProseState::with_peer(ambre_core::PeerId(1)).expect("a document");
        let outcome = state.handle_command(ProseCommand::EnableTrackChanges { enabled: true });
        assert_eq!(refusal(&outcome), Some(ProseRefusal::NoIdentity));
        assert!(!state.is_suggesting());
    }

    /// One undo takes back the text, its mark and its entry together.
    #[test]
    fn an_undo_takes_back_the_whole_suggestion() {
        let (mut state, ids) = document(ambre_core::PeerId(1), ALICE, &["Hello"]);
        suggest(&mut state);
        type_at(&mut state, ids[0], DocOffset::new(5), "!");
        state.handle_command(ProseCommand::Undo);
        assert_eq!(text(&state, ids[0]), "Hello");
        assert!(state.suggestions().is_empty());
        assert!(crate::suggest::entry(&state.doc, &SuggestionId::new()).is_none());
        assert_eq!(doc::annots(&state.doc).len(), 0);
    }

    proptest! {
        /// Type and delete in any order while suggesting, then accept all.
        /// The text is what plain edits would have left, and no suggestion
        /// mark stays behind (`PROSE-FEAT-010`).
        #[test]
        fn accepting_everything_is_the_edits_made_plainly(
            edits in proptest::collection::vec((any::<bool>(), 0_u32..20, 1_u32..4), 1..16)
        ) {
            let (mut tracked, ids) = document(ambre_core::PeerId(1), ALICE, &["abcdefghij"]);
            let (mut plain, plain_ids) = document(ambre_core::PeerId(1), ALICE, &["abcdefghij"]);
            suggest(&mut tracked);
            for (insert, at, len) in edits {
                // The tracked text still holds what it proposes to delete, so
                // positions are taken on the plain text and mapped over.
                let visible = visible_positions(&tracked, ids[0]);
                let body = visible.len() as u32;
                if insert || body == 0 {
                    let at = at.min(body);
                    let real = visible.get(at as usize).copied()
                        .unwrap_or_else(|| text(&tracked, ids[0]).chars().count() as u32);
                    let typed = "x".repeat(len as usize);
                    type_at(&mut tracked, ids[0], DocOffset::new(real), &typed);
                    type_at(&mut plain, plain_ids[0], DocOffset::new(at), &typed);
                } else {
                    let start = at.min(body - 1);
                    let end = (start + 1).min(body);
                    let real = visible[start as usize];
                    delete(&mut tracked, ids[0], (real, real + 1));
                    delete(&mut plain, plain_ids[0], (start, end));
                }
            }
            tracked.handle_command(ProseCommand::AcceptAllSuggestions);
            prop_assert_eq!(text(&tracked, ids[0]), text(&plain, plain_ids[0]));
            let marks = format!("{:?}", tracked.marks_of(&ids[0]));
            prop_assert!(!marks.contains("si:") && !marks.contains("sd:"), "{}", marks);
        }
    }

    /// The offsets of the characters no deletion covers, in order.
    fn visible_positions(state: &ProseState, block: BlockId) -> Vec<u32> {
        crate::suggest::runs(
            &state.doc,
            &crate::events::tree_id(block),
            DocOffset::ZERO,
            DocOffset::MAX,
        )
        .into_iter()
        .filter(|run| {
            run.marks
                .iter()
                .all(|(kind, _)| *kind != SuggestionKind::Deletion)
        })
        .flat_map(|run| run.start.get()..run.end.get())
        .collect()
    }
}
