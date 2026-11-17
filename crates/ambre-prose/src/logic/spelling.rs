//! Spell checking (`features/ambre-spec-prose-spell-check.md`).
//!
//! The state here is the reader's session alone and never enters the
//! document (`PROSE-FEAT-106`). The Logic thread cuts the text into words
//! and keeps where each one sits. The converter thread looks the words up in
//! the spelling module of the document's language. So the Logic thread never
//! waits for a dictionary (`PROSE-FEAT-041`, `PROSE-FEAT-103`).
//!
//! One check is in flight per document at a time. The blocks edited while it
//! runs wait for the next one, so a fast typist sends one check per answer,
//! never one per keystroke.

use std::collections::{HashMap, HashSet};

use ambre_bus::rest::Request;
use ambre_convert::spelling::{
    CheckedWord, Dictionary, MAX_WORD_CHARS, SpellCheck, SpellSuggest, SpellWord, WordChars,
};
use ambre_convert::{Conversion, Converted, JobFailure};
use ambre_core::index::{Pos, Span};
use ambre_core::{Count, LangTag, NarrowCount, Seq, Validated};
use loro::TreeID;
use unicode_segmentation::UnicodeSegmentation;

use crate::doc;
use crate::events::{BlockId, ProseEvent, ProseOutcome, block_id, tree_id};
use crate::message::DocChar;

use super::ProseState;

/// The most words one check hands the converter thread. The blocks past it
/// wait for the next check, so one answer never holds the thread long.
const WORDS_PER_CHECK: Count<WordsInCheck> = Count::new(4096);

/// The distinct words one check carries.
enum WordsInCheck {}

impl NarrowCount for WordsInCheck {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("WordsInCheckCount");
}

/// Which blocks the next check reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    /// The blocks edited since their last check.
    Stale,
    /// Every block: at the start, and once the dictionary changed.
    Whole,
}

/// One misspelled word of a block, as the check found it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Misspelling {
    range: Span<DocChar>,
    word: Validated<SpellWord>,
}

/// The words of one block a check carries.
struct CheckedBlock {
    block: TreeID,
    words: Vec<(Span<DocChar>, Validated<SpellWord>, Pos<CheckedWord>)>,
}

/// The check in flight: its ticket and the blocks it carries.
struct InFlight {
    ticket: Seq<Request>,
    blocks: Vec<CheckedBlock>,
}

/// Whether the spelling module of the document's language answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Module {
    /// It answers, or nothing asked it yet.
    Answers,
    /// It failed to load. No check is sent again for this language in this
    /// session, so a missing module costs one fetch, not one per pass.
    Failed,
}

/// The reader's spelling session for one document.
pub(super) struct Spelling {
    /// The reader's own language, which the check falls back on when the
    /// document names none.
    reader: Option<Validated<LangTag>>,
    /// The dictionary the document is checked against, or `None` when its
    /// language has none.
    dictionary: Option<Dictionary>,
    module: Module,
    /// The blocks to check again. A block a check carries leaves it, so a
    /// block back in it was edited since that check left.
    stale: HashSet<TreeID>,
    rescan: Scan,
    checking: Option<InFlight>,
    /// The suggestions asked for, by ticket.
    suggesting: HashMap<Seq<Request>, (BlockId, Span<DocChar>)>,
    /// The misspelled words of each block, as the last check found them.
    errors: HashMap<TreeID, Vec<Misspelling>>,
    /// The words this reader ignored for the session (`PROSE-FEAT-334`).
    ignored: HashSet<Validated<SpellWord>>,
    /// The jobs the Logic dispatcher hands the converter thread.
    outbox: Vec<(Seq<Request>, Conversion)>,
}

impl Default for Spelling {
    fn default() -> Self {
        Self {
            reader: None,
            dictionary: None,
            module: Module::Answers,
            stale: HashSet::new(),
            rescan: Scan::Whole,
            checking: None,
            suggesting: HashMap::new(),
            errors: HashMap::new(),
            ignored: HashSet::new(),
            outbox: Vec::new(),
        }
    }
}

/// The words of `text`, each with its range in code points. A word holds a
/// letter and no digit, as the word count reads a word (UAX #29). A run past
/// [`MAX_WORD_CHARS`] is no word of any dictionary, and stays out.
fn words(text: &str) -> Vec<(Span<DocChar>, Validated<SpellWord>)> {
    let mut at = 0_usize;
    let mut found = Vec::new();
    for piece in text.split_word_bounds() {
        let len = piece.chars().count();
        let start = at;
        at += len;
        if Count::<WordChars>::of(len) > MAX_WORD_CHARS
            || !piece.chars().any(char::is_alphabetic)
            || piece.chars().any(|c| c.is_numeric() || c == '_')
        {
            continue;
        }
        if let Ok(word) = Validated::<SpellWord>::parse(piece) {
            found.push((Span::<DocChar>::from_usize_range(start..at), word));
        }
    }
    found
}

impl ProseState {
    /// Note the blocks a layout re-measured: their text may have changed.
    pub(super) fn spelling_touched(&mut self, blocks: &[TreeID]) {
        self.spelling.stale.extend(blocks.iter().copied());
    }

    /// Take the reader's own language, the fallback of a document that names
    /// none.
    pub fn set_spelling_reader(&mut self, reader: Option<Validated<LangTag>>) {
        self.spelling.reader = reader;
    }

    /// The dictionary the document's language names, else the reader's
    /// (`PROSE-FEAT-111`).
    fn spelling_dictionary(&self) -> Option<Dictionary> {
        doc::read_locale(&self.doc)
            .or_else(|| self.spelling.reader.clone())
            .and_then(|tag| Dictionary::of_language(&tag))
    }

    /// The checks and the suggestions this document owes the converter
    /// thread, and what the view is told now. The Logic dispatcher calls it
    /// once a pass, sends each job and never waits for it (`PROSE-FEAT-041`).
    pub fn spell_pass(&mut self) -> (Vec<(Seq<Request>, Conversion)>, ProseOutcome) {
        let mut display = Vec::new();
        let dictionary = self.spelling_dictionary();
        if dictionary != self.spelling.dictionary {
            self.spelling.dictionary = dictionary;
            self.spelling.module = Module::Answers;
            self.spelling.checking = None;
            self.spelling.rescan = Scan::Whole;
            display.extend(self.clear_errors());
        }
        if self.spelling.checking.is_none()
            && self.spelling.module == Module::Answers
            && let Some(dictionary) = self.spelling.dictionary
        {
            display.extend(self.start_check(dictionary));
        }
        (
            std::mem::take(&mut self.spelling.outbox),
            ProseOutcome::display_only(display),
        )
    }

    /// Drop every error: the view underlines nothing.
    fn clear_errors(&mut self) -> Vec<ProseEvent> {
        self.spelling
            .errors
            .drain()
            .filter(|(_, errors)| !errors.is_empty())
            .map(|(block, _)| ProseEvent::Misspelled {
                block_id: block_id(block),
                ranges: Vec::new(),
            })
            .collect()
    }

    /// Cut the stale blocks into words and owe the converter thread one
    /// check of them. A block with no word, or gone, is answered at once.
    fn start_check(&mut self, dictionary: Dictionary) -> Vec<ProseEvent> {
        if std::mem::replace(&mut self.spelling.rescan, Scan::Stale) == Scan::Whole {
            self.spelling.stale = self.document_order().into_iter().collect();
        }
        let mut stale: Vec<TreeID> = self.spelling.stale.iter().copied().collect();
        stale.sort_unstable();
        let mut display = Vec::new();
        let mut asked: Vec<Validated<SpellWord>> = Vec::new();
        let mut index: HashMap<Validated<SpellWord>, Pos<CheckedWord>> = HashMap::new();
        let mut blocks = Vec::new();
        for block in stale {
            if Count::<WordsInCheck>::of(asked.len()) >= WORDS_PER_CHECK {
                break;
            }
            self.spelling.stale.remove(&block);
            let Some(text) = self.body_of(&block).map(|body| body.to_string()) else {
                display.extend(self.set_errors(block, Vec::new()));
                continue;
            };
            let found: Vec<_> = words(&text)
                .into_iter()
                .map(|(range, word)| {
                    let at = *index.entry(word.clone()).or_insert_with(|| {
                        asked.push(word.clone());
                        Pos::<CheckedWord>::from_usize(asked.len() - 1)
                    });
                    (range, word, at)
                })
                .collect();
            if found.is_empty() {
                display.extend(self.set_errors(block, Vec::new()));
                continue;
            }
            blocks.push(CheckedBlock {
                block,
                words: found,
            });
        }
        if !blocks.is_empty() {
            let ticket = ambre_bus::next_request();
            self.spelling.outbox.push((
                ticket,
                Conversion::CheckSpelling(SpellCheck {
                    dictionary,
                    words: asked,
                }),
            ));
            self.spelling.checking = Some(InFlight { ticket, blocks });
        }
        display
    }

    /// Hold `errors` for `block`, the ignored words left out, and tell the
    /// view when that changed what it underlines.
    fn set_errors(&mut self, block: TreeID, errors: Vec<Misspelling>) -> Option<ProseEvent> {
        let errors: Vec<Misspelling> = errors
            .into_iter()
            .filter(|error| !self.spelling.ignored.contains(&error.word))
            .collect();
        let before = self
            .spelling
            .errors
            .get(&block)
            .map_or(&[][..], Vec::as_slice);
        if before == errors.as_slice() {
            return None;
        }
        let ranges = errors.iter().map(|error| error.range).collect();
        if errors.is_empty() {
            self.spelling.errors.remove(&block);
        } else {
            self.spelling.errors.insert(block, errors);
        }
        Some(ProseEvent::Misspelled {
            block_id: block_id(block),
            ranges,
        })
    }

    /// The answer of a check or of a suggestion this document asked for, or
    /// `None` when the ticket is not one of its own.
    pub fn handle_spelled(
        &mut self,
        ticket: Seq<Request>,
        answer: &Result<Converted, JobFailure>,
    ) -> Option<ProseOutcome> {
        if let Some((block, range)) = self.spelling.suggesting.remove(&ticket) {
            let suggestions = match answer {
                Ok(Converted::Suggestions(found)) => found.clone(),
                _ => Vec::new(),
            };
            return Some(ProseOutcome::display_only(vec![
                ProseEvent::SpellingSuggestions {
                    block_id: block,
                    range,
                    suggestions,
                },
            ]));
        }
        if self.spelling.checking.as_ref()?.ticket != ticket {
            return None;
        }
        let checked = self.spelling.checking.take()?;
        let Ok(Converted::Misspelled(unknown)) = answer else {
            tracing::warn!("Logic: the spelling module did not answer a check");
            self.spelling.module = Module::Failed;
            return Some(ProseOutcome::nothing());
        };
        let unknown: HashSet<Pos<CheckedWord>> = unknown.iter().copied().collect();
        let mut display = Vec::new();
        for block in checked.blocks {
            // A block edited since the check left waits for the next check:
            // its ranges no longer name its words.
            if self.spelling.stale.contains(&block.block) {
                continue;
            }
            let errors = block
                .words
                .into_iter()
                .filter(|(_, _, at)| unknown.contains(at))
                .map(|(range, word, _)| Misspelling { range, word })
                .collect();
            display.extend(self.set_errors(block.block, errors));
        }
        Some(ProseOutcome::display_only(display))
    }

    /// The error of `block` whose range is `range`, as the last check found
    /// it, when the text still holds that word there.
    fn error_at(&self, block: BlockId, range: Span<DocChar>) -> Option<Misspelling> {
        let id = tree_id(block);
        let error = self
            .spelling
            .errors
            .get(&id)?
            .iter()
            .find(|error| error.range == range)?
            .clone();
        let text = self.body_of(&id)?.to_string();
        let held: String = text
            .chars()
            .skip(range.start.to_usize())
            .take(range.len().to_usize())
            .collect();
        (held == error.word.as_str()).then_some(error)
    }

    /// Ask the converter thread for the corrections of the misspelled word at
    /// `range` of `block` (`PROSE-FEAT-105`). A range that names no error now
    /// answers no suggestion. It writes nothing.
    pub(super) fn suggest_spelling(
        &mut self,
        block: BlockId,
        range: Span<DocChar>,
    ) -> ProseOutcome {
        let (Some(error), Some(dictionary)) =
            (self.error_at(block, range), self.spelling.dictionary)
        else {
            return ProseOutcome::display_only(vec![ProseEvent::SpellingSuggestions {
                block_id: block,
                range,
                suggestions: Vec::new(),
            }]);
        };
        let ticket = ambre_bus::next_request();
        self.spelling.suggesting.insert(ticket, (block, range));
        self.spelling.outbox.push((
            ticket,
            Conversion::SuggestSpelling(SpellSuggest {
                dictionary,
                word: error.word,
            }),
        ));
        ProseOutcome::nothing()
    }

    /// Replace the misspelled word at `range` of `block` with `replacement`,
    /// as an ordinary edit that peers receive (`PROSE-FEAT-109`). A range
    /// that names no error now is refused, so no text outside a misspelled
    /// word is touched (`PROSE-FEAT-110`).
    pub(super) fn correct_spelling(
        &mut self,
        block: BlockId,
        range: Span<DocChar>,
        replacement: &Validated<SpellWord>,
    ) -> ProseOutcome {
        if self.error_at(block, range).is_none() {
            return Self::err_bad_range();
        }
        if let Err(outcome) = self.replace_range(&tree_id(block), range, replacement.as_str()) {
            return outcome;
        }
        self.commit_with(Vec::new())
    }

    /// Stop underlining the word at `range` of `block` for this reader, in
    /// this document, for the session (`PROSE-FEAT-334`). It writes nothing,
    /// so no peer receives anything (`PROSE-FEAT-108`).
    pub(super) fn ignore_spelling(&mut self, block: BlockId, range: Span<DocChar>) -> ProseOutcome {
        let Some(error) = self.error_at(block, range) else {
            return ProseOutcome::nothing();
        };
        self.spelling.ignored.insert(error.word);
        let mut blocks: Vec<TreeID> = self.spelling.errors.keys().copied().collect();
        blocks.sort_unstable();
        let display = blocks
            .into_iter()
            .filter_map(|id| {
                let errors = self.spelling.errors.get(&id).cloned().unwrap_or_default();
                self.set_errors(id, errors)
            })
            .collect();
        ProseOutcome::display_only(display)
    }
}

#[cfg(test)]
mod tests;
