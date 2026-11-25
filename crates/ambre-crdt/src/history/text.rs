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

use ambre_core::FixedBytes;
use ambre_core::crdt::TextDigest;
use ambre_core::hash::HashMap;

use ambre_core::index::{Len, Pos, Span};
use loro::cursor::{Cursor, Side};
use loro::{ContainerID, InternalString, LoroDoc, LoroText, LoroValue, VersionVector};

use super::ident::{self, IdRun, Idents};
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
    /// as `None`: Loro answers it with a new cursor. A character a replay of
    /// this peer put back is read under its new identity, and the cursor
    /// follows it from then on (`UNDO-024`).
    fn read(&mut self, doc: &LoroDoc, idents: &Idents) {
        self.found = None;
        let Some(cursor) = &self.cursor else {
            return;
        };
        let Ok(answer) = doc.get_cursor_pos(cursor) else {
            return;
        };
        if answer.update.is_none() {
            self.found = Some(Pos::from_usize(answer.current.pos));
            return;
        }
        let Some(id) = cursor.id.and_then(|id| idents.resolve(id)) else {
            return;
        };
        let moved = Cursor::new(Some(id), cursor.container.clone(), cursor.side, 0);
        if let Ok(answer) = doc.get_cursor_pos(&moved)
            && answer.update.is_none()
        {
            self.found = Some(Pos::from_usize(answer.current.pos));
            self.cursor = Some(moved);
        }
    }

    /// Where the character stands now, in the replay under way. A dead
    /// character may be one that an earlier op of this replay put back. The
    /// replay knows where it wrote it (`UNDO-024`).
    /// Loro is not asked again, so the replay is never half committed.
    fn place(&self, idents: &Idents, shift: &Shifts) -> Option<Pos<Chars>> {
        if let Some(found) = self.found {
            return Some(shift.follow(found));
        }
        let id = self.cursor.as_ref()?.id?;
        shift.placed(idents.resolve(id)?)
    }
}

/// The index space of the edits one replay made in one text.
#[derive(Debug, PartialEq, Eq)]
enum Edits {}

impl ambre_core::index::Axis for Edits {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Edits");
}

/// One edit of a text, in the state the history last left it.
#[derive(Debug, Clone)]
pub(super) enum TextOp {
    /// `len` characters stand in the text: this peer put them there. `first`
    /// and `last` name the first and the last of them. The step keeps their
    /// digest, never the characters, since the text holds them. An undo reads
    /// them back with their marks before it takes them out. An import so
    /// costs its step a few bytes per edit, not a copy of the file.
    Standing {
        first: Mark,
        last: Mark,
        len: Len<Chars>,
        digest: FixedBytes<TextDigest>,
        hint: Pos<Chars>,
    },
    /// `content` is gone from the text: this peer took it out. `before` and
    /// `anchor` name the characters that stood right before and right after
    /// it. A later step may take a neighbour out and put it back under a new
    /// identity. The cursor on it then names a dead character, and the other
    /// neighbour, or the recorded place, stands in. `ids` holds the old
    /// identity of each character of `content`, or nothing when the commit
    /// could not read them. A replay that puts them back notes the new
    /// identities against these (`UNDO-024`).
    Gone {
        before: Mark,
        anchor: Mark,
        content: Rich,
        hint: Pos<Chars>,
        ids: Vec<IdRun>,
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
    /// which already holds the commit. `deleted` names the characters the
    /// commit took out of the text, in order, when the commit could read
    /// them.
    pub(super) fn record(
        doc: &LoroDoc,
        id: &ContainerID,
        edits: Vec<Edit>,
        deleted: Option<Vec<IdRun>>,
    ) -> Self {
        let text = doc.get_text(id.clone());
        // Read once per event, and only when the event marked: a version
        // vector holds one entry per peer.
        let marks = edits.iter().any(|edit| matches!(edit, Edit::Marked { .. }));
        let seen = if marks {
            doc.oplog_vv()
        } else {
            VersionVector::default()
        };
        // The identities of the characters each deletion took out, in the
        // order of the deletions: the event lists them from left to right.
        let lens: Vec<Len<Chars>> = edits
            .iter()
            .filter_map(|edit| match edit {
                Edit::Took { content, .. } => Some(content.len()),
                _ => None,
            })
            .collect();
        let mut gone = deleted
            .and_then(|runs| ident::split(&runs, &lens))
            .unwrap_or_default()
            .into_iter();
        let ops = edits
            .into_iter()
            .map(|edit| match edit {
                Edit::Put { at, content } => TextOp::standing(&text, at, &content),
                Edit::Took { at, content } => TextOp::Gone {
                    before: Mark::before(&text, at),
                    anchor: Mark::at(&text, at, Side::Left),
                    content,
                    hint: at,
                    ids: gone.next().unwrap_or_default(),
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
        let idents = &renames.chars;
        for op in &mut self.ops {
            match op {
                TextOp::Standing { first, last, .. } | TextOp::Marked { first, last, .. } => {
                    first.read(doc, idents);
                    last.read(doc, idents);
                }
                TextOp::Gone { before, anchor, .. } => {
                    before.read(doc, idents);
                    anchor.read(doc, idents);
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
        renames: &mut Renames,
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
        let ops: Vec<&mut TextOp> = match undo {
            Direction::Back => self.ops.iter_mut().rev().collect(),
            Direction::Forward => self.ops.iter_mut().collect(),
        };
        for op in ops {
            let next = op.write(doc, &text, undo, cells, shift, &mut renames.chars);
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
    /// Each run of characters the replay put back, where it wrote them, and
    /// how many edits it had made then.
    put: Vec<(IdRun, Pos<Chars>, Pos<Edits>)>,
}

impl Shifts {
    /// Note that `taken` characters went at `at` and `put` came in there.
    fn note(&mut self, at: Pos<Chars>, taken: Len<Chars>, put: Len<Chars>) {
        self.edits.push((at, taken, put));
    }

    /// Note that the replay just wrote the characters `runs` names from `at`.
    fn put(&mut self, runs: &[IdRun], at: Pos<Chars>) {
        let done = Pos::from_usize(self.edits.len());
        let mut place = at;
        for run in runs {
            self.put.push((*run, place, done));
            place += run.len();
        }
    }

    /// Where the character `id` stands now, when this replay put it back.
    fn placed(&self, id: loro::ID) -> Option<Pos<Chars>> {
        self.put.iter().find_map(|(run, at, done)| {
            let offset = run.offset_of(id)?;
            let later = self.edits.get(done.to_usize()..)?;
            Some(Self::through(later, *at + offset))
        })
    }

    /// Where `place`, read before the replay, stands now.
    fn follow(&self, place: Pos<Chars>) -> Pos<Chars> {
        Self::through(&self.edits, place)
    }

    /// Where `place` stands once `edits` moved it.
    fn through(edits: &[(Pos<Chars>, Len<Chars>, Len<Chars>)], place: Pos<Chars>) -> Pos<Chars> {
        edits.iter().fold(place, |place, (at, taken, put)| {
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
    idents: &Idents,
) -> Option<Pos<Chars>> {
    if cells == Cells::Recreated {
        return Some(hint);
    }
    let (a, b) = (first.place(idents, shift)?, last.place(idents, shift)?);
    (b >= a && b.len_from(a) + Len::ONE == len).then_some(a)
}

/// The characters `text` holds over the `len` characters from `at`, with
/// their marks.
fn read_rich(text: &LoroText, at: Pos<Chars>, len: Len<Chars>) -> Option<Rich> {
    let delta = text
        .slice_delta(
            at.to_usize(),
            (at + len).to_usize(),
            loro::cursor::PosType::Unicode,
        )
        .ok()?;
    Some(Rich::from_delta(&delta))
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
/// marked, in order, each one a place and a length. `ends` spans from the
/// first of them to past the last, where they stand now. Between them, a character `seen` does not hold was typed by a
/// peer since, and splits the range (`UNDO-016`). This peer's own characters
/// count as the step's: an undo of its own deletion puts them back under new
/// identities. `None` when the count differs from
/// `len`: a peer took a character out, and no offset can be trusted.
fn stretches(
    doc: &LoroDoc,
    text: &LoroText,
    ends: Span<Chars>,
    len: Len<Chars>,
    seen: &VersionVector,
) -> Option<Vec<(Pos<Chars>, Len<Chars>)>> {
    let (start, end) = (ends.start, ends.end);
    if end <= start {
        return None;
    }
    let own = doc.peer_id();
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
    /// The op of `content`, which stands in `text` from `at`: its two
    /// cursors and its digest, read from `text`, which already holds it.
    fn standing(text: &LoroText, at: Pos<Chars>, content: &Rich) -> Self {
        let len = content.len();
        let end = (at + len).prev().unwrap_or(at);
        Self::Standing {
            first: Mark::at(text, at, Side::Middle),
            last: Mark::at(text, end, Side::Middle),
            len,
            digest: content.digest(),
            hint: at,
        }
    }

    /// Toggle the op in `text`, and return its new state when it moved.
    fn write(
        &mut self,
        doc: &LoroDoc,
        text: &LoroText,
        undo: Direction,
        cells: Cells,
        shift: &mut Shifts,
        idents: &mut Idents,
    ) -> Option<Self> {
        match self {
            Self::Standing {
                first,
                last,
                len,
                digest,
                hint,
            } => {
                let len = *len;
                let at = span(first, last, len, *hint, cells, shift, idents)?;
                // The characters come back with the marks they carry now, so
                // a redo keeps a mark a peer set on them meanwhile.
                let content = read_rich(text, at, len)?;
                if content.digest() != *digest {
                    return None;
                }
                let ids = ident::read(text, at, len).unwrap_or_default();
                text.delete(at.to_usize(), len.to_usize()).ok()?;
                shift.note(at, len, Len::ZERO);
                Some(Self::Gone {
                    before: Mark::before(text, at),
                    anchor: Mark::at(text, at, Side::Left),
                    content,
                    hint: at,
                    ids,
                })
            }
            Self::Gone {
                before,
                anchor,
                content,
                hint,
                ids,
            } => {
                let size = Len::<Chars>::of(text.len_unicode());
                // The characters go back before the one that stood after
                // them, or right after the one that stood before them
                // (`UNDO-025`).
                let found = anchor
                    .place(idents, shift)
                    .or_else(|| before.place(idents, shift).map(Pos::next));
                let at = match found.filter(|_| cells == Cells::Recorded) {
                    Some(at) => at.min(size.as_end()),
                    None => (*hint).min(size.as_end()),
                };
                let len = content.len();
                content.write_into(text, at);
                shift.note(at, Len::ZERO, len);
                if let Some(new) = ident::written(text, at, content.run_lens()) {
                    idents.note(ids, &new);
                    shift.put(&new, at);
                }
                Some(Self::standing(text, at, content))
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
                    let ends = Span::new(
                        first.place(idents, shift)?,
                        last.place(idents, shift)?.next(),
                    );
                    stretches(doc, text, ends, *len, seen)?
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
