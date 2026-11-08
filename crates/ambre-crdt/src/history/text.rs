//! The text half of a step: each edit kept as the characters it put in or
//! took out, placed by Loro cursors (`ambre-spec-undo.md section 3`).
//!
//! A cursor names a character by its Loro identity, so it follows the
//! character through every edit a peer makes around it. An undo of an
//! insertion takes out the characters this peer put in, found by their
//! cursors, and never a character a peer typed. An undo of a deletion puts the
//! characters back beside the one that stood after them.
//!
//! A text a replay created again by value has new characters. A cursor into
//! the old one then names nothing, so the place the edit was recorded at
//! stands in for it.

use std::collections::HashMap;

use ambre_core::index::{Len, Pos};
use loro::cursor::{Cursor, Side};
use loro::{ContainerID, InternalString, LoroDoc, LoroText, LoroValue};

use super::rich::{Edit, Rich};
use super::{Chars, Renames};

/// A cursor and where it pointed when a replay read it, before the replay
/// wrote anything.
#[derive(Debug, Clone)]
pub(super) struct Mark {
    cursor: Option<Cursor>,
    /// The place the cursor read, `None` until read or when it names no
    /// character that still stands.
    found: Option<Pos<Chars>>,
}

impl Mark {
    /// A cursor taken from `text` at `at`, on `side`.
    fn at(text: &LoroText, at: Pos<Chars>, side: Side) -> Self {
        Self {
            cursor: text.get_cursor(at.to_usize(), side),
            found: None,
        }
    }

    /// Read where the cursor points now. A character a peer took out reads
    /// as `None`: Loro answers it with a new cursor.
    fn read(&mut self, doc: &LoroDoc, live: Live) {
        self.found = self.cursor.as_ref().and_then(|cursor| {
            let answer = doc.get_cursor_pos(cursor).ok()?;
            match (live, answer.update) {
                (Live::Needed, Some(_)) => None,
                _ => Some(Pos::from_usize(answer.current.pos)),
            }
        });
    }
}

ambre_core::define_flag! {
    /// Whether a cursor must still name a standing character.
    enum Live {
        /// A gone character still gives the place it stood at.
        Any,
        /// A gone character gives no place.
        Needed,
    }
}

/// One edit of a text, in the state the history last left it.
#[derive(Debug, Clone)]
pub(super) enum TextOp {
    /// `content` stands in the text: this peer put it there. `first` and
    /// `last` name its first and last character.
    Standing {
        first: Mark,
        last: Mark,
        content: Rich,
        hint: Pos<Chars>,
    },
    /// `content` is gone from the text: this peer took it out. `anchor` names
    /// the character that stood right after it.
    Gone {
        anchor: Mark,
        content: Rich,
        hint: Pos<Chars>,
    },
    /// This peer set the mark `key` to `after` over `content`'s characters,
    /// which held `before` run by run.
    Marked {
        first: Mark,
        last: Mark,
        len: Len<Chars>,
        key: InternalString,
        before: Vec<(Len<Chars>, Option<LoroValue>)>,
        after: Option<LoroValue>,
        hint: Pos<Chars>,
    },
}

/// What one commit did to one text.
#[derive(Debug, Clone)]
pub(super) struct TextWrite {
    pub(super) text: ContainerID,
    pub(super) ops: Vec<TextOp>,
}

impl TextWrite {
    /// The ops of the edits one event made, their cursors read from `doc`,
    /// which already holds the commit.
    pub(super) fn record(doc: &LoroDoc, id: &ContainerID, edits: Vec<Edit>) -> Self {
        let text = doc.get_text(id.clone());
        let ops = edits
            .into_iter()
            .map(|edit| match edit {
                Edit::Put { at, content } => {
                    let end = (at + content.len()).prev().unwrap_or(at);
                    TextOp::Standing {
                        first: Mark::at(&text, at, Side::Middle),
                        last: Mark::at(&text, end, Side::Middle),
                        content,
                        hint: at,
                    }
                }
                Edit::Took { at, content } => TextOp::Gone {
                    anchor: Mark::at(&text, at, Side::Left),
                    content,
                    hint: at,
                },
                Edit::Marked {
                    at,
                    len,
                    key,
                    before,
                    after,
                } => {
                    let end = (at + len).prev().unwrap_or(at);
                    TextOp::Marked {
                        first: Mark::at(&text, at, Side::Middle),
                        last: Mark::at(&text, end, Side::Middle),
                        len,
                        key,
                        before,
                        after,
                        hint: at,
                    }
                }
            })
            .collect();
        Self {
            text: id.clone(),
            ops,
        }
    }

    /// Read every cursor of the write, before the replay writes anything: a
    /// cursor on a gone character makes Loro commit what is pending, and the
    /// replay must not be half committed. A text a replay created again
    /// reads nothing, its cursors name the old characters.
    pub(super) fn read(&mut self, doc: &LoroDoc, renames: &Renames) {
        if renames.resolve(&self.text) != self.text {
            return;
        }
        for op in &mut self.ops {
            match op {
                TextOp::Standing { first, last, .. } | TextOp::Marked { first, last, .. } => {
                    first.read(doc, Live::Needed);
                    last.read(doc, Live::Needed);
                }
                TextOp::Gone { anchor, .. } => anchor.read(doc, Live::Any),
            }
        }
    }

    /// Replay the write: `ops` in `order`, each one toggled. What stands goes,
    /// what went comes back, and a mark goes back to `before` on an undo or
    /// to `after` on a redo. `shifts` follows the places this replay already
    /// moved in each text.
    pub(super) fn write(
        &mut self,
        doc: &LoroDoc,
        undo: Direction,
        renames: &Renames,
        shifts: &mut HashMap<ContainerID, Shifts>,
    ) {
        let id = renames.resolve(&self.text);
        let renamed = id != self.text;
        let text = doc.get_text(id.clone());
        if text.is_deleted() {
            return;
        }
        let shift = shifts.entry(id.clone()).or_default();
        let ops: Vec<&mut TextOp> = match undo {
            Direction::Back => self.ops.iter_mut().rev().collect(),
            Direction::Forward => self.ops.iter_mut().collect(),
        };
        for op in ops {
            let next = op.write(&text, undo, renamed, shift);
            if let Some(next) = next {
                *op = next;
            }
        }
    }
}

ambre_core::define_flag! {
    /// Which way a replay walks the history.
    pub(super) enum Direction {
        /// A redo: the edit is made again.
        Forward,
        /// An undo: the edit is taken back.
        Back,
    }
}

/// The edits a replay already made in one text, in order, so a place read
/// before the replay can be moved to where it stands now.
#[derive(Debug, Default)]
pub(super) struct Shifts {
    edits: Vec<(Pos<Chars>, Len<Chars>, Len<Chars>)>,
}

impl Shifts {
    /// Note that `taken` characters went at `at` and `put` came in there.
    fn note(&mut self, at: Pos<Chars>, taken: Len<Chars>, put: Len<Chars>) {
        self.edits.push((at, taken, put));
    }

    /// Where `place`, read before the replay, stands now.
    fn follow(&self, place: Pos<Chars>) -> Pos<Chars> {
        self.edits.iter().fold(place, |place, (at, taken, put)| {
            if place >= *at + *taken {
                Pos::from_usize(place.to_usize() - taken.to_usize() + put.to_usize())
            } else if place > *at {
                *at
            } else {
                place
            }
        })
    }
}

/// The plain characters `text` holds over the `len` characters from `at`.
fn read_plain(text: &LoroText, at: Pos<Chars>, len: Len<Chars>) -> Option<String> {
    text.slice(at.to_usize(), (at + len).to_usize()).ok()
}

impl TextOp {
    /// Toggle the op in `text`, and return its new state when it moved.
    fn write(
        &mut self,
        text: &LoroText,
        undo: Direction,
        renamed: bool,
        shift: &mut Shifts,
    ) -> Option<Self> {
        match self {
            Self::Standing {
                first,
                last,
                content,
                hint,
            } => {
                let len = content.len();
                // The characters must stand together and read as written:
                // a peer's character inside them is never taken out
                // (`UNDO-007`).
                let at = if renamed {
                    Some(*hint)
                } else {
                    match (first.found, last.found) {
                        (Some(a), Some(b))
                            if b.to_usize() + 1 >= a.to_usize()
                                && b.to_usize() + 1 - a.to_usize() == len.to_usize() =>
                        {
                            Some(shift.follow(a))
                        }
                        _ => None,
                    }
                }?;
                if read_plain(text, at, len)? != content.plain() {
                    return None;
                }
                text.delete(at.to_usize(), len.to_usize()).ok()?;
                shift.note(at, len, Len::ZERO);
                Some(Self::Gone {
                    anchor: Mark::at(text, at, Side::Left),
                    content: content.clone(),
                    hint: at,
                })
            }
            Self::Gone {
                anchor,
                content,
                hint,
            } => {
                let size = Len::<Chars>::of(text.len_unicode());
                let at = match anchor.found.filter(|_| !renamed) {
                    Some(at) => shift.follow(at),
                    None => (*hint).min(size.as_end()),
                };
                let len = content.len();
                content.write_into(text, at);
                shift.note(at, Len::ZERO, len);
                let end = (at + len).prev().unwrap_or(at);
                Some(Self::Standing {
                    first: Mark::at(text, at, Side::Middle),
                    last: Mark::at(text, end, Side::Middle),
                    content: content.clone(),
                    hint: at,
                })
            }
            Self::Marked {
                first,
                last,
                len,
                key,
                before,
                after,
                hint,
            } => {
                let at = if renamed {
                    Some(*hint)
                } else {
                    match (first.found, last.found) {
                        (Some(a), Some(b))
                            if b.to_usize() + 1 >= a.to_usize()
                                && b.to_usize() + 1 - a.to_usize() == len.to_usize() =>
                        {
                            Some(shift.follow(a))
                        }
                        _ => None,
                    }
                }?;
                let runs: Vec<(Len<Chars>, Option<LoroValue>)> = match undo {
                    Direction::Back => before.clone(),
                    Direction::Forward => vec![(*len, after.clone())],
                };
                let mut from = at;
                for (run, value) in runs {
                    let range = from.to_usize()..(from + run).to_usize();
                    let _ = match value {
                        Some(value) => text.mark(range, key.as_str(), value),
                        None => text.unmark(range, key.as_str()),
                    };
                    from += run;
                }
                None
            }
        }
    }
}
