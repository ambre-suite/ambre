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

use ambre_core::hash::HashMap;

use ambre_core::index::{Len, Pos};
use loro::cursor::{Cursor, Side};
use loro::{ContainerID, InternalString, LoroDoc, LoroText, LoroValue, PeerID, VersionVector};

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

    /// A cursor on the character just before `at`, which points right after
    /// it. None at the start of the text.
    fn before(text: &LoroText, at: Pos<Chars>) -> Self {
        match at.prev() {
            Some(prev) => Self::at(text, prev, Side::Right),
            None => Self {
                cursor: None,
                found: None,
            },
        }
    }

    /// Read where the cursor points now. A character a peer took out reads
    /// as `None`: Loro answers it with a new cursor.
    fn read(&mut self, doc: &LoroDoc) {
        self.found = self.cursor.as_ref().and_then(|cursor| {
            let answer = doc.get_cursor_pos(cursor).ok()?;
            answer
                .update
                .is_none()
                .then(|| Pos::from_usize(answer.current.pos))
        });
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
    /// `content` is gone from the text: this peer took it out. `before` and
    /// `anchor` name the characters that stood right before and right after
    /// it. A later step may take a neighbour out and put it back under a new
    /// identity. The cursor on it then names a dead character, and the other
    /// neighbour, or the recorded place, stands in.
    Gone {
        before: Mark,
        anchor: Mark,
        content: Rich,
        hint: Pos<Chars>,
    },
    /// This peer set the mark `key` to `after` over `len` characters,
    /// which held `before` run by run. `seen` is the version the commit left.
    /// A peer typed each character between `first` and `last` that `seen`
    /// does not hold, and the replay leaves it (`UNDO-016`).
    Marked {
        first: Mark,
        last: Mark,
        len: Len<Chars>,
        key: InternalString,
        before: Vec<(Len<Chars>, Option<LoroValue>)>,
        after: Option<LoroValue>,
        hint: Pos<Chars>,
        seen: VersionVector,
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
        // Read once per event, and only when the event marked: a version
        // vector holds one entry per peer.
        let marks = edits.iter().any(|edit| matches!(edit, Edit::Marked { .. }));
        let seen = if marks {
            doc.oplog_vv()
        } else {
            VersionVector::default()
        };
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
                    before: Mark::before(&text, at),
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
                        seen: seen.clone(),
                    }
                }
            })
            .collect();
        Self {
            text: id.clone(),
            ops,
        }
    }

    /// Read every cursor of the write, before the replay writes anything. A
    /// cursor on a gone character makes Loro commit what is pending, and the
    /// replay must not be half committed. A text a replay created again reads
    /// nothing: its cursors name the old characters.
    pub(super) fn read(&mut self, doc: &LoroDoc, renames: &Renames) {
        if renames.resolve(&self.text) != self.text {
            return;
        }
        for op in &mut self.ops {
            match op {
                TextOp::Standing { first, last, .. } | TextOp::Marked { first, last, .. } => {
                    first.read(doc);
                    last.read(doc);
                }
                TextOp::Gone { before, anchor, .. } => {
                    before.read(doc);
                    anchor.read(doc);
                }
            }
        }
    }

    /// Replay the write: `ops` in `order`, each one toggled. What stands goes,
    /// what went comes back, and a mark goes back to `before` on an undo or
    /// to `after` on a redo. A mark skips each character a peer changed since. `shifts` follows the places this replay already
    /// moved in each text.
    pub(super) fn write(
        &mut self,
        doc: &LoroDoc,
        undo: Direction,
        renames: &Renames,
        shifts: &mut HashMap<ContainerID, Shifts>,
    ) {
        let id = renames.resolve(&self.text);
        let cells = if id == self.text {
            Cells::Recorded
        } else {
            Cells::Recreated
        };
        let text = doc.get_text(id.clone());
        if text.is_deleted() {
            return;
        }
        let shift = shifts.entry(id.clone()).or_default();
        let own = doc.peer_id();
        let ops: Vec<&mut TextOp> = match undo {
            Direction::Back => self.ops.iter_mut().rev().collect(),
            Direction::Forward => self.ops.iter_mut().collect(),
        };
        for op in ops {
            let next = op.write(&text, undo, cells, own, shift);
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

/// The edits a replay already made in one text, in order. A place read before
/// the replay moves through them to where it stands now.
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

ambre_core::define_flag! {
    /// Whether a text still holds the characters its cursors name.
    enum Cells {
        /// The text the edit was recorded in.
        Recorded,
        /// A text a replay created again by value: new characters, so the
        /// recorded place stands in for each cursor.
        Recreated,
    }
}

/// Where the `len` characters `first` and `last` name start now. They must
/// still stand together: a peer's character among them leaves the op alone
/// (`UNDO-007`).
fn span(
    first: &Mark,
    last: &Mark,
    len: Len<Chars>,
    hint: Pos<Chars>,
    cells: Cells,
    shift: &Shifts,
) -> Option<Pos<Chars>> {
    if cells == Cells::Recreated {
        return Some(hint);
    }
    let (a, b) = (first.found?, last.found?);
    (b >= a && b.len_from(a) + Len::ONE == len).then(|| shift.follow(a))
}

/// The plain characters `text` holds over the `len` characters from `at`.
fn read_plain(text: &LoroText, at: Pos<Chars>, len: Len<Chars>) -> Option<String> {
    text.slice(at.to_usize(), (at + len).to_usize()).ok()
}

/// The value of the mark `key`, run by run, as a step records it. `None`
/// stands where the mark is off.
type Values = [(Len<Chars>, Option<LoroValue>)];

/// The value of the mark `key` over the `len` characters of `text` from `at`.
fn read_values(
    text: &LoroText,
    at: Pos<Chars>,
    len: Len<Chars>,
    key: &InternalString,
) -> Option<Vec<(Len<Chars>, Option<LoroValue>)>> {
    let delta = text
        .slice_delta(
            at.to_usize(),
            (at + len).to_usize(),
            loro::cursor::PosType::Unicode,
        )
        .ok()?;
    Some(Rich::from_delta(&delta).values(Pos::ZERO, len, key))
}

/// One walk over the runs of a `Values`, character by character.
struct Walk<'a> {
    runs: std::slice::Iter<'a, (Len<Chars>, Option<LoroValue>)>,
    left: Len<Chars>,
    value: Option<&'a Option<LoroValue>>,
}

impl<'a> Walk<'a> {
    fn new(values: &'a Values) -> Self {
        let mut walk = Self {
            runs: values.iter(),
            left: Len::ZERO,
            value: None,
        };
        walk.fill();
        walk
    }

    /// Step to the next run that holds a character, or to the end.
    fn fill(&mut self) {
        while self.left.is_zero() {
            let Some((len, value)) = self.runs.next() else {
                self.value = None;
                return;
            };
            self.left = *len;
            self.value = Some(value);
        }
    }

    fn take(&mut self, by: Len<Chars>) {
        self.left = self.left.saturating_sub(by);
        self.fill();
    }
}

/// The pieces of `to` to write where the text still holds `from`, each one
/// with its offset into the range. A character whose value differs from
/// `from` holds a write a peer made since: it takes nothing (`UNDO-013`).
fn agreed(
    now: &Values,
    from: &Values,
    to: &Values,
) -> Vec<(Pos<Chars>, Len<Chars>, Option<LoroValue>)> {
    let mut out: Vec<(Pos<Chars>, Len<Chars>, Option<LoroValue>)> = Vec::new();
    let mut walks = [Walk::new(now), Walk::new(from), Walk::new(to)];
    let mut at = Pos::<Chars>::ZERO;
    loop {
        let [now, from, to] = &walks;
        let (Some(held), Some(want), Some(value)) = (now.value, from.value, to.value) else {
            return out;
        };
        let step = now.left.min(from.left).min(to.left);
        if held == want {
            match out.last_mut() {
                Some((start, len, last)) if *start + *len == at && last == value => {
                    *len += step;
                }
                _ => out.push((at, step, value.clone())),
            }
        }
        at += step;
        for walk in &mut walks {
            walk.take(step);
        }
    }
}

/// The stretches of `text` that hold the `len` characters a format step
/// marked, in order, each one a place and a length. `first` and `last` name
/// the ends. Between them, a character `seen` does not hold was typed by a
/// peer since, and splits the range (`UNDO-016`). This peer's own characters
/// count as the step's: an undo of its own deletion puts them back under new
/// identities. `None` when the ends are gone, or when the count differs from
/// `len`: a peer took a character out, and no offset can be trusted.
fn stretches(
    text: &LoroText,
    first: &Mark,
    last: &Mark,
    len: Len<Chars>,
    seen: &VersionVector,
    own: PeerID,
    shift: &Shifts,
) -> Option<Vec<(Pos<Chars>, Len<Chars>)>> {
    let (a, b) = (first.found?, last.found?);
    if b < a {
        return None;
    }
    let start = shift.follow(a);
    let end = shift.follow(b.next());
    let mut out: Vec<(Pos<Chars>, Len<Chars>)> = Vec::new();
    let mut total = Len::<Chars>::ZERO;
    for at in start.to_usize()..end.to_usize() {
        let id = text.get_cursor(at, Side::Middle)?.id?;
        if id.peer != own && !seen.includes_id(id) {
            continue;
        }
        let at = Pos::<Chars>::from_usize(at);
        match out.last_mut() {
            Some((from, run)) if *from + *run == at => *run += Len::ONE,
            _ => out.push((at, Len::ONE)),
        }
        total += Len::ONE;
    }
    (total == len).then_some(out)
}

/// The places in the text of the `run` characters from `offset` on, counted
/// in the step's own characters, split at each stretch boundary.
fn place(
    stretches: &[(Pos<Chars>, Len<Chars>)],
    offset: Pos<Chars>,
    run: Len<Chars>,
) -> Vec<(Pos<Chars>, Len<Chars>)> {
    let (begin, finish) = (offset.to_usize(), (offset + run).to_usize());
    let mut out = Vec::new();
    let mut base = 0_usize;
    for (at, size) in stretches {
        let size = size.to_usize();
        let lo = begin.max(base);
        let hi = finish.min(base + size);
        if lo < hi {
            out.push((*at + Len::of(lo - base), Len::of(hi - lo)));
        }
        base += size;
    }
    out
}

impl TextOp {
    /// Toggle the op in `text`, and return its new state when it moved.
    fn write(
        &mut self,
        text: &LoroText,
        undo: Direction,
        cells: Cells,
        own: PeerID,
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
                let at = span(first, last, len, *hint, cells, shift)?;
                if read_plain(text, at, len)? != content.plain() {
                    return None;
                }
                text.delete(at.to_usize(), len.to_usize()).ok()?;
                shift.note(at, len, Len::ZERO);
                Some(Self::Gone {
                    before: Mark::before(text, at),
                    anchor: Mark::at(text, at, Side::Left),
                    content: content.clone(),
                    hint: at,
                })
            }
            Self::Gone {
                before,
                anchor,
                content,
                hint,
            } => {
                let size = Len::<Chars>::of(text.len_unicode());
                let found = anchor.found.or(before.found);
                let at = match found.filter(|_| cells == Cells::Recorded) {
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
                seen,
            } => {
                let stretches = if cells == Cells::Recreated {
                    vec![(*hint, *len)]
                } else {
                    stretches(text, first, last, *len, seen, own, shift)?
                };
                let set = vec![(*len, after.clone())];
                let (from, to) = match undo {
                    Direction::Back => (&set, &*before),
                    Direction::Forward => (&*before, &set),
                };
                let mut now = Vec::new();
                for (at, run) in &stretches {
                    now.extend(read_values(text, *at, *run, key)?);
                }
                for (offset, run, value) in agreed(&now, from, to) {
                    for (start, piece) in place(&stretches, offset, run) {
                        let range = start.to_usize()..(start + piece).to_usize();
                        let _ = match &value {
                            Some(value) => text.mark(range, key.as_str(), value.clone()),
                            None => text.unmark(range, key.as_str()),
                        };
                    }
                }
                None
            }
        }
    }
}
