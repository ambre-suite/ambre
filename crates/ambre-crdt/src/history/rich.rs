//! The plain copy of one text container: its characters in runs, each run
//! with the marks it carries.
//!
//! The mirror keeps one per text, so a step can read what a commit deleted. A
//! container captured by value carries its body with its marks. Every position
//! counts Unicode scalars, the unit Loro events and the Prose engine use.

use std::collections::BTreeMap;

use ambre_core::FixedBytes;
use ambre_core::crdt::TextDigest;
use ambre_core::hash::TableHasher;
use ambre_core::index::{Axis, Len, Pos};
use loro::{InternalString, LoroText, LoroValue, TextDelta};

use super::Chars;

/// The index space of the runs of one text.
#[derive(Debug, PartialEq, Eq)]
enum Runs {}

impl Axis for Runs {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Runs");
}

/// The marks of one run, by key. A sorted map, so two runs compare by
/// content whatever order Loro handed the keys out in.
pub(super) type Marks = BTreeMap<InternalString, LoroValue>;

/// One run: characters that carry the same marks.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Run {
    pub(super) text: String,
    pub(super) marks: Marks,
    /// How many characters `text` holds, kept beside it: a body edit walks
    /// the runs once per item of its delta.
    size: Len<Chars>,
}

impl Run {
    /// A run of `text` with `marks`.
    fn new(text: String, marks: Marks) -> Self {
        let size = Len::of(text.chars().count());
        Self { text, marks, size }
    }

    /// Append `text`, which carries the same marks.
    fn extend(&mut self, text: &str) {
        self.text.push_str(text);
        self.size += Len::of(text.chars().count());
    }

    /// The byte index of the character at `at`, or the end. A run of ASCII
    /// holds one byte per character, and needs no walk.
    fn byte(&self, at: Pos<Chars>) -> usize {
        if self.size.to_usize() == self.text.len() {
            return at.to_usize().min(self.text.len());
        }
        byte_at(&self.text, at)
    }
}

/// The characters of one text, in runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Rich {
    runs: Vec<Run>,
}

/// The marks an event or a delta hands out, as a sorted map. A `Null` value
/// means the mark is off, so it carries no key.
fn marks_of<'a>(
    attributes: Option<impl IntoIterator<Item = (&'a String, &'a LoroValue)>>,
) -> Marks {
    attributes
        .into_iter()
        .flatten()
        .filter(|(_, value)| !matches!(value, LoroValue::Null))
        .map(|(key, value)| (InternalString::from(key.as_str()), value.clone()))
        .collect()
}

/// The one hasher every digest of the process reads: the key of the process
/// and one seed, so two digests of the same characters agree.
fn digest_key() -> &'static TableHasher {
    static KEY: std::sync::OnceLock<TableHasher> = std::sync::OnceLock::new();
    KEY.get_or_init(TableHasher::default)
}

/// The byte index of the character at `at` in `text`, or the end.
fn byte_at(text: &str, at: Pos<Chars>) -> usize {
    text.char_indices()
        .nth(at.to_usize())
        .map_or(text.len(), |(byte, _)| byte)
}

impl Rich {
    /// The runs of a whole delta, as `LoroText::to_delta` reads it.
    pub(super) fn from_delta(delta: &[TextDelta]) -> Self {
        let mut rich = Self::default();
        for item in delta {
            if let TextDelta::Insert { insert, attributes } = item {
                rich.push(insert, marks_of(attributes.as_ref()));
            }
        }
        rich
    }

    /// One run of `text` with `marks`.
    pub(super) fn one(text: &str, marks: Marks) -> Self {
        let mut rich = Self::default();
        rich.push(text, marks);
        rich
    }

    /// Append `text` with `marks`, merged into the last run when it carries
    /// the same marks.
    fn push(&mut self, text: &str, marks: Marks) {
        if text.is_empty() {
            return;
        }
        match self.runs.last_mut() {
            Some(last) if last.marks == marks => last.extend(text),
            _ => self.runs.push(Run::new(text.to_owned(), marks)),
        }
    }

    /// The length of each run, in order: one insertion of
    /// [`write_into`](Self::write_into) each.
    pub(super) fn run_lens(&self) -> impl Iterator<Item = Len<Chars>> + '_ {
        self.runs.iter().map(|run| run.size)
    }

    /// Append the characters of `other`, with their marks.
    fn append(&mut self, other: &Self) {
        for run in &other.runs {
            self.push(&run.text, run.marks.clone());
        }
    }

    /// How many characters the text holds.
    pub(super) fn len(&self) -> Len<Chars> {
        self.runs.iter().fold(Len::ZERO, |sum, run| sum + run.size)
    }

    /// The characters alone, with no mark.
    pub(super) fn plain(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }

    /// The digest of the characters alone, marks left out. Two texts with
    /// the same characters share it, however the runs split them. The digest
    /// is keyed by the hash key of the process, so a peer can neither learn
    /// nor aim at it (`UNDO-014`).
    pub(super) fn digest(&self) -> FixedBytes<TextDigest> {
        use std::hash::BuildHasher as _;
        let key = digest_key();
        // A keystroke is one run, and is digested where it stands: the
        // characters are copied only when marks split them.
        let value = match self.runs.as_slice() {
            [run] => key.hash_one(run.text.as_str()),
            _ => key.hash_one(self.plain().as_str()),
        };
        FixedBytes::from_array(value.to_le_bytes())
    }

    /// Split the run that straddles `at`, and return the index of the run
    /// that starts there: the run count when `at` is the end.
    fn cut(&mut self, at: Pos<Chars>) -> Pos<Runs> {
        let mut start = Pos::<Chars>::ZERO;
        for index in 0..self.runs.len() {
            let found = Pos::<Runs>::from_usize(index);
            let Some(run) = self.runs.get(index) else {
                break;
            };
            let len = run.size;
            if at == start {
                return found;
            }
            if at < start + len {
                let head = at.len_from(start);
                let byte = run.byte(head.as_end());
                let tail = Run {
                    text: run.text.get(byte..).unwrap_or_default().to_owned(),
                    marks: run.marks.clone(),
                    size: len.saturating_sub(head),
                };
                if let Some(run) = self.runs.get_mut(index) {
                    run.text.truncate(byte);
                    run.size = head;
                }
                self.runs.insert(index + 1, tail);
                return found.next();
            }
            start += len;
        }
        Pos::from_usize(self.runs.len())
    }

    /// Merge each run into the one before it when both carry the same marks.
    fn tidy(&mut self) {
        let runs = std::mem::take(&mut self.runs);
        // The mirror keeps one per text for the life of the document: room for
        // the runs it had, never the four a first push reserves.
        self.runs = Vec::with_capacity(runs.len());
        for run in runs {
            if run.text.is_empty() {
                continue;
            }
            match self.runs.last_mut() {
                Some(last) if last.marks == run.marks => {
                    last.text.push_str(&run.text);
                    last.size += run.size;
                }
                _ => self.runs.push(run),
            }
        }
    }

    /// The `len` characters from `at` on, with their marks.
    pub(super) fn slice(&self, at: Pos<Chars>, len: Len<Chars>) -> Self {
        let end = at + len;
        let mut out = Self::default();
        let mut start = Pos::<Chars>::ZERO;
        for run in &self.runs {
            if start >= end {
                break;
            }
            let stop = start + run.size;
            if stop > at {
                let from = Pos::from_usize(at.to_usize().saturating_sub(start.to_usize()));
                let to = Pos::from_usize(end.min(stop).to_usize() - start.to_usize());
                let text = run
                    .text
                    .get(run.byte(from)..run.byte(to))
                    .unwrap_or_default();
                out.push(text, run.marks.clone());
            }
            start = stop;
        }
        out
    }

    /// Take out the `len` characters from `at` on, and put `with` there.
    pub(super) fn splice(&mut self, at: Pos<Chars>, len: Len<Chars>, with: &Self) {
        let first = self.cut(at).to_usize();
        let last = self.cut(at + len).to_usize();
        let end = last.min(self.runs.len());
        self.runs
            .splice(first.min(end)..end, with.runs.iter().cloned());
        self.tidy();
    }

    /// Set the mark `key` to `value` on the `len` characters from `at` on;
    /// `None` takes it off.
    pub(super) fn mark(
        &mut self,
        at: Pos<Chars>,
        len: Len<Chars>,
        key: &InternalString,
        value: Option<&LoroValue>,
    ) {
        let first = self.cut(at).to_usize();
        let last = self.cut(at + len).to_usize();
        for run in self.runs.iter_mut().take(last).skip(first) {
            match value {
                Some(value) => {
                    run.marks.insert(key.clone(), value.clone());
                }
                None => {
                    run.marks.remove(key);
                }
            }
        }
        self.tidy();
    }

    /// The value of the mark `key` over the `len` characters from `at` on, run
    /// by run. Each length comes with its value, `None` where the mark is off.
    pub(super) fn values(
        &self,
        at: Pos<Chars>,
        len: Len<Chars>,
        key: &InternalString,
    ) -> Vec<(Len<Chars>, Option<LoroValue>)> {
        let mut out: Vec<(Len<Chars>, Option<LoroValue>)> = Vec::new();
        for run in self.slice(at, len).runs {
            let value = run.marks.get(key).cloned();
            let len = run.size;
            match out.last_mut() {
                Some((sum, last)) if *last == value => *sum += len,
                _ => out.push((len, value)),
            }
        }
        out
    }

    /// Write the text into `text` at `at`, with its marks. A mark the
    /// insertion picks up from a neighbour is taken off, so the characters
    /// come back with the marks they had and no other.
    pub(super) fn write_into(&self, text: &LoroText, at: Pos<Chars>) {
        let mut cursor = at;
        for run in &self.runs {
            let len = run.size;
            if text.insert(cursor.to_usize(), &run.text).is_err() {
                return;
            }
            let range = cursor.to_usize()..(cursor + len).to_usize();
            for (key, value) in &run.marks {
                let _ = text.mark(range.clone(), key.as_str(), value.clone());
            }
            cursor += len;
        }
        // The marks the inserted characters now carry, against the ones they
        // should: a key a neighbour lent them comes off.
        let Ok(now) = text.slice_delta(
            at.to_usize(),
            cursor.to_usize(),
            loro::cursor::PosType::Unicode,
        ) else {
            return;
        };
        let got = Self::from_delta(&now);
        let mut offset = at;
        for run in got.runs {
            let len = run.size;
            let want = self.slice(Pos::from_usize(offset.to_usize() - at.to_usize()), len);
            for key in run.marks.keys() {
                let lent = want.runs.iter().all(|w| !w.marks.contains_key(key));
                if lent {
                    let _ = text.unmark(offset.to_usize()..(offset + len).to_usize(), key.as_str());
                }
            }
            offset += len;
        }
    }
}

/// What one text event did to the mirror's copy, read item by item. That is
/// the characters it put in, the characters it took out, and the marks it
/// changed. A place is the one in the text after the event for what stands,
/// and where the gap is for what went.
#[derive(Debug, Clone)]
pub(super) enum Edit {
    /// `content` now stands from `at` on.
    Put { at: Pos<Chars>, content: Rich },
    /// `content` stood at `at` and is gone.
    Took { at: Pos<Chars>, content: Rich },
    /// The mark `key` changed to `after` on the `len` characters from `at`
    /// on. `before` holds the value it had, run by run.
    Marked {
        at: Pos<Chars>,
        len: Len<Chars>,
        key: InternalString,
        before: Vec<(Len<Chars>, Option<LoroValue>)>,
        after: Option<LoroValue>,
    },
}

impl Rich {
    /// Apply one event's delta to the copy, and return what it did.
    pub(super) fn take(&mut self, delta: &[TextDelta]) -> Vec<Edit> {
        let mut edits = Vec::new();
        let mut at = Pos::<Chars>::ZERO;
        for item in delta {
            match item {
                TextDelta::Retain { retain, attributes } => {
                    let len = Len::<Chars>::of(*retain);
                    for (key, value) in attributes.iter().flatten() {
                        let key = InternalString::from(key.as_str());
                        let after = match value {
                            LoroValue::Null => None,
                            value => Some(value.clone()),
                        };
                        let before = self.values(at, len, &key);
                        self.mark(at, len, &key, after.as_ref());
                        edits.push(Edit::Marked {
                            at,
                            len,
                            key,
                            before,
                            after,
                        });
                    }
                    at += len;
                }
                TextDelta::Insert { insert, attributes } => {
                    let content = Self::one(insert, marks_of(attributes.as_ref()));
                    let len = content.len();
                    self.splice(at, Len::ZERO, &content);
                    edits.push(Edit::Put { at, content });
                    at += len;
                }
                TextDelta::Delete { delete } => {
                    let len = Len::<Chars>::of(*delete);
                    let content = self.slice(at, len);
                    self.splice(at, len, &Self::default());
                    // Loro hands a deletion over several runs out as several
                    // deletions at one place. They took out one stretch, and
                    // an undo puts it back as one, in its order.
                    match edits.last_mut() {
                        Some(Edit::Took {
                            at: last,
                            content: taken,
                        }) if *last == at => taken.append(&content),
                        _ => edits.push(Edit::Took { at, content }),
                    }
                }
            }
        }
        edits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> Marks {
        let mut marks = Marks::new();
        marks.insert(InternalString::from("b"), LoroValue::Bool(true));
        marks
    }

    #[test]
    fn a_splice_keeps_the_marks_of_each_side() {
        let mut rich = Rich::one("hello", Marks::new());
        rich.mark(
            Pos::new(1),
            Len::new(2),
            &InternalString::from("b"),
            Some(&LoroValue::Bool(true)),
        );
        rich.splice(Pos::new(2), Len::new(2), &Rich::one("XY", Marks::new()));
        assert_eq!(rich.plain(), "heXYo");
        assert_eq!(rich.slice(Pos::new(1), Len::new(1)), Rich::one("e", bold()));
        assert_eq!(
            rich.slice(Pos::new(2), Len::new(3)),
            Rich::one("XYo", Marks::new())
        );
    }

    #[test]
    fn a_delta_reads_what_it_put_took_and_marked() {
        let mut rich = Rich::one("abcdef", Marks::new());
        let attributes = [("b".to_owned(), LoroValue::Bool(true))]
            .into_iter()
            .collect();
        let edits = rich.take(&[
            TextDelta::Retain {
                retain: 1,
                attributes: Some(attributes),
            },
            TextDelta::Delete { delete: 2 },
            TextDelta::Insert {
                insert: "Z".to_owned(),
                attributes: None,
            },
        ]);
        assert_eq!(rich.plain(), "aZdef");
        assert!(
            matches!(&edits[1], Edit::Took { at, content } if at.get() == 1 && content.plain() == "bc")
        );
        assert!(
            matches!(&edits[2], Edit::Put { at, content } if at.get() == 1 && content.plain() == "Z")
        );
    }
}
