//! Find and replace (`features/ambre-spec-prose-find-replace.md`).
//!
//! The session is Logic-thread state and never enters the document
//! (`FIND-001`). A search reads the body blocks in document order. A match lies
//! inside one block (`PROSE-FEAT-034`), and matches never overlap
//! (`PROSE-FEAT-035`). Every offset is a count of code points, the unit a
//! block's text is indexed in (`PROSE-045`).

use ambre_bus::app::find::{MatchCount, MatchIndex};
use ambre_bus::app::prose::{DocSpan, FindRefusal};
use ambre_core::StepDirection;
use loro::TreeID;

use crate::doc;
use crate::events::{
    FindMatch, FindQuery, ProseEvent, ProseOutcome, ProseRefusal, block_id, tree_id,
};

use super::ProseState;

/// The open search: the query, what it found, and the match the walk is on.
/// An emptied field opens none: it closes the open one (`FIND-005`).
#[derive(Debug)]
pub(super) struct FindSession {
    query: FindQuery,
    matches: Vec<FindMatch>,
    current: Option<MatchIndex>,
}

/// The match spans inside one block's text, honouring case and whole-word,
/// never overlapping (`PROSE-FEAT-035`).
///
/// The case fold maps one code point to one, so a span in the folded text is
/// the same span in the original. A code point whose lower case is longer
/// than one is compared as it is.
pub(crate) fn match_offsets(hay: &str, query: &FindQuery) -> Vec<DocSpan> {
    let fold = |c: char| -> char {
        if query.options.case == ambre_core::CaseMatch::Sensitive {
            return c;
        }
        let mut lower = c.to_lowercase();
        match (lower.next(), lower.next()) {
            (Some(one), None) => one,
            _ => c,
        }
    };
    let hay: Vec<char> = hay.chars().map(fold).collect();
    let needle: Vec<char> = query.text.as_str().chars().map(fold).collect();
    let mut found = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return found;
    }
    let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
    let mut at = 0;
    while at + needle.len() <= hay.len() {
        let end = at + needle.len();
        let hit = hay
            .get(at..end)
            .is_some_and(|window| window == needle.as_slice());
        let bounded = query.options.word == ambre_core::WordMatch::Anywhere
            || (!at
                .checked_sub(1)
                .and_then(|before| hay.get(before))
                .is_some_and(is_word)
                && !hay.get(end).is_some_and(is_word));
        if hit && bounded {
            found.push(DocSpan::from_usize_range(at..end));
            // Past the whole match: the next one starts after it.
            at = end;
        } else {
            at += 1;
        }
    }
    found
}

impl ProseState {
    /// Every match in the document, in block order: body blocks only
    /// (`PROSE-FEAT-033`, `PROSE-FEAT-150`).
    pub(super) fn find_all(&self, query: &FindQuery) -> Vec<FindMatch> {
        let tree = doc::blocks(&self.doc);
        self.document_order()
            .into_iter()
            .filter(|id| doc::block_type_of(&tree, id).is_some_and(|t| t.has_body()))
            .flat_map(|id| {
                let text = doc::get_block_text(&self.doc, &id).to_string();
                match_offsets(&text, query)
                    .into_iter()
                    .map(move |range| FindMatch {
                        block_id: block_id(id),
                        range,
                    })
            })
            .collect()
    }

    /// Open a search. It reads the document and writes nothing
    /// (`PROSE-FEAT-151`). No query is the emptied field: it closes the open
    /// search and clears the highlight, so a step after it answers
    /// [`FindRefusal::NoSearch`] (`FIND-005`).
    pub(super) fn find_start(&mut self, query: Option<FindQuery>) -> ProseOutcome {
        let Some(query) = query else {
            return self.find_close();
        };
        if query.options.pattern == crate::events::PatternKind::Regex {
            return ProseOutcome::refused(ProseRefusal::FindRefused(FindRefusal::RegexUnbuilt));
        }
        let matches = self.find_all(&query);
        let current = (!matches.is_empty()).then_some(MatchIndex::ZERO);
        self.find = Some(FindSession {
            query,
            matches,
            current,
        });
        self.find_results()
    }

    /// Move the current index one match on, wrapping at either end. The list
    /// and the document are left alone (`PROSE-FEAT-152`).
    pub(super) fn find_step(&mut self, direction: StepDirection) -> ProseOutcome {
        let Some(session) = self.find.as_mut() else {
            return Self::err_no_search();
        };
        let count = MatchCount::from_len(session.matches.len());
        session.current = session.current.and_then(|at| at.stepped(direction, count));
        ProseOutcome::display_only(vec![ProseEvent::FindUpdated {
            current: session.current,
        }])
    }

    /// End the session, and clear the highlight (`PROSE-FEAT-158`,
    /// `PROSE-FEAT-289`).
    pub(super) fn find_close(&mut self) -> ProseOutcome {
        self.find = None;
        ProseOutcome::display_only(vec![ProseEvent::FindResults {
            matches: Vec::new(),
            current: None,
        }])
    }

    /// Replace the current match, then search again: the index stays where it
    /// was, on the match that followed the one replaced (`PROSE-FEAT-153`).
    pub(super) fn replace_one(&mut self, replacement: &str) -> ProseOutcome {
        let Some(session) = self.find.as_ref() else {
            return Self::err_no_search();
        };
        let Some(hit) = session
            .current
            .and_then(|at| session.matches.get(at.to_index()?))
            .copied()
        else {
            return Self::err_no_search();
        };
        let at = session.current;
        // A collaborator's change may have moved or removed the match since
        // the view saw it. A range no fresh search reports is not replaced:
        // the view gets the fresh list instead.
        if !self.find_all(&session.query).contains(&hit) {
            return ProseOutcome::display_only(self.refresh_matches(at));
        }
        if let Err(outcome) = self.replace_range(&tree_id(hit.block_id), hit.range, replacement) {
            return outcome;
        }
        let mut outcome = self.commit_with(Vec::new());
        outcome.display.extend(self.refresh_matches(at));
        outcome
    }

    /// Replace every match, from the end of the document backwards, in one
    /// commit and so one undo item (`PROSE-FEAT-036`, `FIND-013`).
    pub(super) fn replace_all(&mut self, replacement: &str) -> ProseOutcome {
        let Some(session) = self.find.as_ref() else {
            return Self::err_no_search();
        };
        // Search again first: the list the view saw may predate a remote
        // change, and a stale range would replace the wrong characters.
        let matches = self.find_all(&session.query);
        if matches.is_empty() {
            return ProseOutcome::display_only(self.refresh_matches(None));
        }
        for hit in matches.iter().rev() {
            if let Err(outcome) = self.replace_range(&tree_id(hit.block_id), hit.range, replacement)
            {
                return outcome;
            }
        }
        let mut outcome = self.commit_with(Vec::new());
        outcome.display.extend(self.refresh_matches(None));
        outcome
    }

    /// Delete one range of one block and insert the replacement in its place,
    /// uncommitted.
    fn replace_range(
        &mut self,
        id: &TreeID,
        range: DocSpan,
        replacement: &str,
    ) -> Result<(), ProseOutcome> {
        let Some(body) = self.body_of(id) else {
            return Err(Self::err_no_block(id));
        };
        let (start, end) = (range.start.to_usize(), range.end.to_usize());
        if end > body.len_unicode() || start > end {
            return Err(Self::err_bad_range());
        }
        if start < end {
            body.delete(start, end - start).map_err(Self::err_write)?;
        }
        if !replacement.is_empty() {
            body.insert(start, replacement).map_err(Self::err_write)?;
        }
        Ok(())
    }

    /// Search again under the same query after a write, and answer with the
    /// new list. The index is kept when it still names a match.
    fn refresh_matches(&mut self, keep: Option<MatchIndex>) -> Vec<ProseEvent> {
        let Some(query) = self.find.as_ref().map(|s| s.query.clone()) else {
            return Vec::new();
        };
        let matches = self.find_all(&query);
        let count = MatchCount::from_len(matches.len());
        let current = match keep {
            Some(at) if at < count.as_end() => Some(at),
            _ if !count.is_zero() => Some(MatchIndex::ZERO),
            _ => None,
        };
        if let Some(session) = self.find.as_mut() {
            session.matches = matches;
            session.current = current;
        }
        self.find_results().display
    }

    /// The open session as the view is told it.
    fn find_results(&self) -> ProseOutcome {
        let (matches, current) = self
            .find
            .as_ref()
            .map(|s| (s.matches.clone(), s.current))
            .unwrap_or_default();
        ProseOutcome::display_only(vec![ProseEvent::FindResults { matches, current }])
    }

    fn err_no_search() -> ProseOutcome {
        ProseOutcome::refused(ProseRefusal::FindRefused(FindRefusal::NoSearch))
    }

    /// The open query, for a test.
    #[cfg(test)]
    fn find_query(&self) -> Option<&FindQuery> {
        self.find.as_ref().map(|s| &s.query)
    }
}

#[cfg(test)]
mod tests {
    fn pairs(spans: Vec<ambre_bus::app::prose::DocSpan>) -> Vec<(u32, u32)> {
        spans
            .into_iter()
            .map(|span| (span.start.get(), span.end.get()))
            .collect()
    }

    use super::*;
    use crate::events::{BlockId, BlockInit, ProseCommand};
    use ambre_bus::app::prose::DocOffset;

    fn query(text: &str) -> Option<FindQuery> {
        FindQuery::new(text.to_owned(), crate::events::FindOptions::default())
    }

    /// The query for a non-empty `text`, for the scan alone.
    fn needle(text: &str) -> FindQuery {
        query(text).expect("a non-empty text")
    }

    /// A document of the given paragraphs, in order.
    fn document(paragraphs: &[&str]) -> (ProseState, Vec<BlockId>) {
        let mut state = ProseState::new().expect("a fresh document");
        let mut ids = state.root_blocks();
        for _ in 1..paragraphs.len() {
            let last = ids.last().copied();
            state.handle_command(ProseCommand::InsertBlock {
                parent_id: None,
                after_id: last,
                init: BlockInit::Paragraph,
                style_id: None,
            });
            ids = state.root_blocks();
        }
        for (id, text) in ids.iter().zip(paragraphs) {
            state.handle_command(ProseCommand::InsertText {
                block_id: *id,
                pos: DocOffset::ZERO,
                text: (*text).to_owned(),
            });
        }
        (state, ids)
    }

    fn results(outcome: &ProseOutcome) -> (Vec<FindMatch>, Option<MatchIndex>) {
        outcome
            .display
            .iter()
            .rev()
            .find_map(|e| match e {
                ProseEvent::FindResults { matches, current } => Some((matches.clone(), *current)),
                _ => None,
            })
            .expect("a result list")
    }

    fn texts(state: &ProseState, ids: &[BlockId]) -> Vec<String> {
        ids.iter()
            .map(|id| state.text_of(id).unwrap_or_default())
            .collect()
    }

    #[test]
    fn matches_never_overlap_and_resume_past_the_end() {
        assert_eq!(
            pairs(match_offsets("aaaa", &needle("aa"))),
            vec![(0, 2), (2, 4)]
        );
        assert_eq!(pairs(match_offsets("aaa", &needle("aa"))), vec![(0, 2)]);
    }

    #[test]
    fn case_and_whole_word_narrow_the_match() {
        let mut q = needle("Cat");
        assert_eq!(pairs(match_offsets("cat Cat scatter", &q)).len(), 3);
        q.options.case = ambre_core::CaseMatch::Sensitive;
        assert_eq!(pairs(match_offsets("cat Cat scatter", &q)), vec![(4, 7)]);
        q.options.case = ambre_core::CaseMatch::Insensitive;
        q.options.word = ambre_core::WordMatch::WholeWord;
        assert_eq!(
            pairs(match_offsets("cat Cat scatter", &q)),
            vec![(0, 3), (4, 7)]
        );
    }

    #[test]
    fn offsets_count_code_points() {
        assert_eq!(
            pairs(match_offsets(
                "\u{e9}t\u{e9} \u{e9}t\u{e9}",
                &needle("\u{c9}T\u{c9}")
            )),
            vec![(0, 3), (4, 7)]
        );
    }

    #[test]
    fn every_match_is_reported_in_document_order() {
        let (mut state, ids) = document(&["one two one", "two one"]);
        let (matches, current) = results(&state.handle_command(ProseCommand::FindStart {
            query: query("one"),
        }));
        let expected = vec![
            FindMatch {
                block_id: ids[0],
                range: DocSpan::from_usize_range(0..3),
            },
            FindMatch {
                block_id: ids[0],
                range: DocSpan::from_usize_range(8..11),
            },
            FindMatch {
                block_id: ids[1],
                range: DocSpan::from_usize_range(4..7),
            },
        ];
        assert_eq!(matches, expected);
        assert_eq!(current, Some(MatchIndex::new(0)));
    }

    #[test]
    fn a_query_across_a_block_boundary_never_matches() {
        let (mut state, _) = document(&["the end", "of it"]);
        for text in ["end\nof", "endof", "end of"] {
            let outcome = state.handle_command(ProseCommand::FindStart { query: query(text) });
            assert!(results(&outcome).0.is_empty(), "{text:?} matched");
        }
    }

    #[test]
    fn a_table_cell_is_body_text_and_is_searched() {
        let mut state = ProseState::new().expect("a fresh document");
        let root = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertTable {
            parent_id: None,
            after_id: Some(root),
            rows: crate::events::grid::RowCount::new(1),
            cols: crate::events::grid::ColCount::new(1),
        });
        let cell_text = state
            .document_order()
            .into_iter()
            .map(block_id)
            .filter(|id| *id != root)
            .find(|id| state.text_of(id).is_some())
            .expect("the cell's paragraph");
        state.handle_command(ProseCommand::InsertText {
            block_id: cell_text,
            pos: DocOffset::ZERO,
            text: "needle".to_owned(),
        });
        let outcome = state.handle_command(ProseCommand::FindStart {
            query: query("needle"),
        });
        assert_eq!(results(&outcome).0.len(), 1);
    }

    #[test]
    fn opening_walking_and_closing_write_nothing() {
        let (mut state, _) = document(&["a b a b a"]);
        let before = state.document().oplog_vv();
        let opened = state.handle_command(ProseCommand::FindStart { query: query("a") });
        assert!(opened.update.is_none() && opened.ephemeral.is_none());
        let listed = results(&opened).0;
        let walked = state.handle_command(ProseCommand::FindNext);
        assert_eq!(
            walked.display,
            vec![ProseEvent::FindUpdated {
                current: Some(MatchIndex::new(1))
            }]
        );
        assert!(walked.update.is_none() && walked.ephemeral.is_none());
        let back = state.handle_command(ProseCommand::FindPrev);
        assert_eq!(
            back.display,
            vec![ProseEvent::FindUpdated {
                current: Some(MatchIndex::new(0))
            }]
        );
        let wrapped = state.handle_command(ProseCommand::FindPrev);
        assert_eq!(
            wrapped.display,
            vec![ProseEvent::FindUpdated {
                current: Some(MatchIndex::new(2))
            }]
        );
        assert_eq!(state.find.as_ref().map(|s| s.matches.clone()), Some(listed));
        let closed = state.handle_command(ProseCommand::FindClose);
        assert!(closed.update.is_none() && closed.ephemeral.is_none());
        assert_eq!(state.document().oplog_vv(), before);
    }

    /// An empty query answers with no match and no current one, which is how
    /// the panel clears its highlight (`FIND-005`).
    #[test]
    fn an_empty_query_clears_the_highlight() {
        let (mut state, _) = document(&["needle"]);
        let found = state.handle_command(ProseCommand::FindStart {
            query: query("needle"),
        });
        assert_eq!(results(&found).0.len(), 1);
        let cleared = state.handle_command(ProseCommand::FindStart { query: query("") });
        assert_eq!(results(&cleared), (Vec::new(), None));
        assert!(cleared.update.is_none());
    }

    /// An emptied field closes the search, as in Trame and Toile: a step or a
    /// replace after it answers that no search is open (`FIND-005`).
    #[test]
    fn an_emptied_field_closes_the_search() {
        let (mut state, _) = document(&["needle"]);
        state.handle_command(ProseCommand::FindStart {
            query: query("needle"),
        });
        let cleared = state.handle_command(ProseCommand::FindStart { query: None });
        assert_eq!(results(&cleared), (Vec::new(), None));
        assert!(cleared.update.is_none());
        let after = [
            ProseCommand::FindNext,
            ProseCommand::FindPrev,
            ProseCommand::ReplaceOne {
                replacement: "pin".to_owned(),
            },
            ProseCommand::ReplaceAll {
                replacement: "pin".to_owned(),
            },
        ];
        for command in after {
            let label = format!("{command:?}");
            let outcome = state.handle_command(command);
            assert!(
                matches!(
                    outcome.display.as_slice(),
                    [ProseEvent::Error(e)]
                        if e.reason == ProseRefusal::FindRefused(FindRefusal::NoSearch)
                ),
                "{label}: {:?}",
                outcome.display
            );
        }
    }

    /// A fresh search starts at the first hit. A resume point an edit took out
    /// of the list restarts there too, and nothing is replaced (`FIND-015`).
    #[test]
    fn a_resume_point_edited_away_restarts_at_the_first_hit() {
        let (mut state, ids) = document(&["needle one", "needle two", "needle three"]);
        let opened = state.handle_command(ProseCommand::FindStart {
            query: query("needle"),
        });
        assert_eq!(results(&opened).1, Some(MatchIndex::ZERO));
        state.handle_command(ProseCommand::FindNext);
        state.handle_command(ProseCommand::FindNext);
        for id in &ids[1..] {
            state.handle_command(ProseCommand::DeleteText {
                block_id: *id,
                range: DocSpan::from_usize_range(0..6),
            });
        }
        let outcome = state.handle_command(ProseCommand::ReplaceOne {
            replacement: "pin".to_owned(),
        });
        let (matches, current) = results(&outcome);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].block_id, ids[0]);
        assert_eq!(current, Some(MatchIndex::new(0)));
        assert!(outcome.update.is_none(), "a stale hit is not replaced");
        assert_eq!(texts(&state, &ids), vec!["needle one", " two", " three"]);
    }

    #[test]
    fn closing_drops_the_session_and_clears_the_highlight() {
        let (mut state, _) = document(&["x"]);
        state.handle_command(ProseCommand::FindStart { query: query("x") });
        let closed = state.handle_command(ProseCommand::FindClose);
        assert_eq!(results(&closed), (Vec::new(), None));
        assert!(state.find.is_none());
        let walked = state.handle_command(ProseCommand::FindNext);
        assert!(matches!(
            walked.display.as_slice(),
            [ProseEvent::Error(e)] if e.reason == ProseRefusal::FindRefused(FindRefusal::NoSearch)
        ));
    }

    #[test]
    fn replace_one_replaces_exactly_the_current_range() {
        let (mut state, ids) = document(&["cat cat", "cat"]);
        state.handle_command(ProseCommand::FindStart {
            query: query("cat"),
        });
        state.handle_command(ProseCommand::FindNext);
        let outcome = state.handle_command(ProseCommand::ReplaceOne {
            replacement: "dog".to_owned(),
        });
        assert!(outcome.update.is_some());
        assert_eq!(texts(&state, &ids), vec!["cat dog", "cat"]);
        let (matches, current) = results(&outcome);
        assert_eq!(matches.len(), 2);
        assert_eq!(current, Some(MatchIndex::new(1)));
        assert_eq!(state.find_query(), query("cat").as_ref());
    }

    #[test]
    fn replace_all_replaces_every_match_and_nothing_else() {
        let (mut state, ids) = document(&["a-cat-b-cat", "cat!", "none"]);
        state.handle_command(ProseCommand::FindStart {
            query: query("cat"),
        });
        let outcome = state.handle_command(ProseCommand::ReplaceAll {
            replacement: "tiger".to_owned(),
        });
        assert_eq!(
            texts(&state, &ids),
            vec!["a-tiger-b-tiger", "tiger!", "none"]
        );
        assert_eq!(results(&outcome), (Vec::new(), None));
        assert_eq!(state.find_query(), query("cat").as_ref());
    }

    #[test]
    fn replacing_backwards_keeps_every_later_position_valid() {
        // A replacement longer than its match shifts what follows in its
        // block. Applied forwards, the second range would land in the middle
        // of the first replacement.
        let (mut state, ids) = document(&["ab ab ab"]);
        state.handle_command(ProseCommand::FindStart { query: query("ab") });
        state.handle_command(ProseCommand::ReplaceAll {
            replacement: "abab".to_owned(),
        });
        assert_eq!(texts(&state, &ids), vec!["abab abab abab"]);
    }

    #[test]
    fn a_replace_all_is_one_undo_item() {
        let (mut state, ids) = document(&["x x", "x"]);
        state.handle_command(ProseCommand::FindStart { query: query("x") });
        state.handle_command(ProseCommand::ReplaceAll {
            replacement: "yy".to_owned(),
        });
        state.handle_command(ProseCommand::Undo);
        assert_eq!(texts(&state, &ids), vec!["x x", "x"]);
    }

    #[test]
    fn a_regular_expression_is_refused_rather_than_read_literally() {
        let (mut state, _) = document(&["a.c"]);
        let mut q = query("a.c");
        if let Some(q) = q.as_mut() {
            q.options.pattern = crate::events::PatternKind::Regex;
        }
        let outcome = state.handle_command(ProseCommand::FindStart { query: q });
        assert!(matches!(outcome.display.as_slice(), [ProseEvent::Error(_)]));
        assert!(state.find.is_none());
    }

    #[test]
    fn the_text_of_a_band_is_never_matched() {
        let (mut state, _) = document(&["body needle"]);
        state.handle_command(ProseCommand::SetHeaderEnabled {
            shown: crate::events::BandState::On,
        });
        let header = state.bands().header.expect("the header");
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: DocOffset::ZERO,
            text: "needle".to_owned(),
        });
        let outcome = state.handle_command(ProseCommand::FindStart {
            query: query("needle"),
        });
        let (matches, _) = results(&outcome);
        assert_eq!(matches.len(), 1);
        assert_ne!(matches[0].block_id, header);
    }
}
