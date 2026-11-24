//! The identity of each character a step names, across the replays that put
//! characters back (`UNDO-024`).
//!
//! Loro mints a new identity for each character an insertion writes. An undo
//! of a deletion so puts the characters back under new identities. A cursor
//! of another step that named one of them then reads a dead character, and
//! that step writes nothing. The history notes each old identity against the
//! new one, as `Renames` does for a container a replay created again.
//!
//! A replay reads the identities of the characters it takes out from the text
//! itself. A commit of the user has already taken them out when the history
//! reads it, so their identities come from the operations of that commit.

use std::cell::OnceCell;
use std::collections::BTreeMap;

use ambre_core::index::{Axis, Len, Pos};
use loro::cursor::Side;
use loro::json::{JsonChange, JsonOpContent, TextOp as JsonTextOp};
use loro::{ContainerID, ID, IdSpan, LoroDoc, LoroText};

use super::Chars;

/// The index space of the entities of a Loro text: its characters and the
/// anchors of its marks. A text operation counts its places in it.
#[derive(Debug, PartialEq, Eq)]
enum Entities {}

impl Axis for Entities {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("Entities");
}

/// `len` characters whose identities follow each other from `start`, one
/// counter apart, in the order of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IdRun {
    start: ID,
    len: Len<Chars>,
}

impl IdRun {
    /// The identity `offset` characters after the first one.
    fn at(self, offset: Len<Chars>) -> Option<ID> {
        Some(self.start.inc(i32::try_from(offset.to_usize()).ok()?))
    }

    /// The `len` characters from `offset` on.
    fn slice(self, offset: Len<Chars>, len: Len<Chars>) -> Option<Self> {
        Some(Self {
            start: self.at(offset)?,
            len,
        })
    }
}

impl IdRun {
    /// How many characters `id` stands after the first one, `None` when the
    /// run does not hold it.
    pub(super) fn offset_of(self, id: ID) -> Option<Len<Chars>> {
        if id.peer != self.start.peer {
            return None;
        }
        let offset = Len::of(usize::try_from(id.counter.checked_sub(self.start.counter)?).ok()?);
        (offset < self.len).then_some(offset)
    }

    /// How many characters the run holds.
    pub(super) fn len(self) -> Len<Chars> {
        self.len
    }
}

/// Append `run` to `runs`, merged into the last run when it follows it.
fn push(runs: &mut Vec<IdRun>, run: IdRun) {
    if run.len.is_zero() {
        return;
    }
    match runs.last_mut() {
        Some(last) if last.at(last.len) == Some(run.start) => last.len += run.len,
        _ => runs.push(run),
    }
}

/// How many characters `runs` covers.
pub(super) fn total(runs: &[IdRun]) -> Len<Chars> {
    runs.iter().fold(Len::ZERO, |sum, run| sum + run.len)
}

/// The identities of the `len` characters of `text` from `at`, read one by
/// one: the characters may come from several insertions. `None` when a place
/// names no character.
pub(super) fn read(text: &LoroText, at: Pos<Chars>, len: Len<Chars>) -> Option<Vec<IdRun>> {
    let mut runs = Vec::new();
    for place in at.to_usize()..(at + len).to_usize() {
        let id = text.get_cursor(place, Side::Middle)?.id?;
        push(
            &mut runs,
            IdRun {
                start: id,
                len: Len::ONE,
            },
        );
    }
    Some(runs)
}

/// The identities of the characters one insertion per run just wrote into
/// `text` from `at`. One insertion mints identities one counter apart, so the
/// first character of each run names the whole run.
pub(super) fn written(
    text: &LoroText,
    at: Pos<Chars>,
    runs: impl Iterator<Item = Len<Chars>>,
) -> Option<Vec<IdRun>> {
    let mut out = Vec::new();
    let mut place = at;
    for len in runs {
        let start = text.get_cursor(place.to_usize(), Side::Middle)?.id?;
        push(&mut out, IdRun { start, len });
        place += len;
    }
    Some(out)
}

/// The new identity of each character a replay put back, by its old one.
/// The key is the first old identity of a run, so a lookup finds its run by
/// one range query.
#[derive(Debug, Default, Clone)]
pub(super) struct Idents {
    to: BTreeMap<ID, (Len<Chars>, ID)>,
}

impl Idents {
    /// The identity `id` took at the next replay that put it back, if any.
    fn next(&self, id: ID) -> Option<ID> {
        let (start, (len, new)) = self.to.range(..=id).next_back()?;
        if start.peer != id.peer {
            return None;
        }
        let offset = id.counter.checked_sub(start.counter)?;
        (usize::try_from(offset).ok()? < len.to_usize()).then(|| new.inc(offset))
    }

    /// The identity `id` stands under now, `None` when no replay put it back.
    /// A character put back twice follows the chain to its last identity.
    pub(super) fn resolve(&self, id: ID) -> Option<ID> {
        let mut now = self.next(id)?;
        // Each link points from a dead character to a younger one, so the
        // chain holds no loop. The bound guards the walk all the same.
        for _ in 0..self.to.len() {
            match self.next(now) {
                Some(next) => now = next,
                None => break,
            }
        }
        Some(now)
    }

    /// Note that the characters `old` names stand under `new` from now on.
    /// Both cover the same characters in order. Two lists of different
    /// lengths note nothing: no offset in them can be trusted.
    pub(super) fn note(&mut self, old: &[IdRun], new: &[IdRun]) {
        if total(old) != total(new) {
            return;
        }
        let mut news = new.iter().copied();
        let mut fresh = news.next();
        for run in old {
            let mut done = Len::<Chars>::ZERO;
            while done < run.len {
                let Some(target) = fresh else {
                    return;
                };
                let piece = run.len.saturating_sub(done).min(target.len);
                if let Some(from) = run.slice(done, piece) {
                    self.to.insert(from.start, (piece, target.start));
                }
                done += piece;
                fresh = if piece < target.len {
                    target.slice(piece, target.len.saturating_sub(piece))
                } else {
                    news.next()
                };
            }
        }
    }
}

/// One local commit as the history reads it: its span of operations, and
/// those operations read from the oplog on the first need.
pub(super) struct Commit<'a> {
    doc: &'a LoroDoc,
    span: Option<IdSpan>,
    changes: OnceCell<Vec<JsonChange>>,
}

/// One piece of the text a commit edits, in the places of the text before
/// the commit. A piece the commit wrote has no such place.
#[derive(Debug, Clone, Copy)]
struct Piece {
    old: Option<Pos<Entities>>,
    len: Len<Entities>,
}

impl Piece {
    /// The part of the piece from `offset` on.
    fn tail(self, offset: Len<Entities>) -> Self {
        Self {
            old: self.old.map(|old| old + offset),
            len: self.len.saturating_sub(offset),
        }
    }

    /// The part of the piece before `offset`.
    fn head(self, offset: Len<Entities>) -> Self {
        Self {
            old: self.old,
            len: offset,
        }
    }
}

/// The index in `pieces` where the place `at` starts, the piece across it cut
/// in two. `None` past the end.
fn cut(pieces: &mut Vec<Piece>, at: Pos<Entities>) -> Option<usize> {
    let mut start = Pos::<Entities>::ZERO;
    for index in 0..pieces.len() {
        let piece = *pieces.get(index)?;
        if start == at {
            return Some(index);
        }
        let end = start + piece.len;
        if at < end {
            let offset = at.len_from(start);
            *pieces.get_mut(index)? = piece.head(offset);
            pieces.insert(index + 1, piece.tail(offset));
            return Some(index + 1);
        }
        start = end;
    }
    (start == at).then_some(pieces.len())
}

/// A text far longer than any document, so the piece of the old text never
/// ends inside a commit.
const UNBOUNDED: Len<Entities> = Len::new(u32::MAX / 4);

impl<'a> Commit<'a> {
    /// The commit `span` names in `doc`. With no span, the commit reads no
    /// identity, and each deletion keeps its characters without them.
    pub(super) fn new(doc: &'a LoroDoc, span: Option<IdSpan>) -> Self {
        Self {
            doc,
            span,
            changes: OnceCell::new(),
        }
    }

    /// The document the commit landed in.
    pub(super) fn doc(&self) -> &'a LoroDoc {
        self.doc
    }

    /// The identities of the characters the commit took out of `text`, in the
    /// order they stood. `None` when the operations cannot place them. A
    /// mark written before a deletion in the same commit adds anchors, and
    /// this walk does not follow them.
    pub(super) fn deleted(&self, text: &ContainerID) -> Option<Vec<IdRun>> {
        let span = self.span?;
        let changes = self
            .changes
            .get_or_init(|| self.doc.export_json_in_id_span(span));
        let mut pieces = vec![Piece {
            old: Some(Pos::ZERO),
            len: UNBOUNDED,
        }];
        let mut gone: Vec<(Pos<Entities>, IdRun)> = Vec::new();
        let mut marked = false;
        for op in changes.iter().flat_map(|change| &change.ops) {
            if op.container != *text {
                continue;
            }
            let JsonOpContent::Text(edit) = &op.content else {
                return None;
            };
            match edit {
                JsonTextOp::Insert { pos, text } => {
                    let at = cut(&mut pieces, Pos::from_usize(usize::try_from(*pos).ok()?))?;
                    pieces.insert(
                        at,
                        Piece {
                            old: None,
                            len: Len::of(text.chars().count()),
                        },
                    );
                }
                JsonTextOp::Delete { .. } if marked => return None,
                JsonTextOp::Delete { pos, len, start_id } => {
                    let size = usize::try_from(len.unsigned_abs()).ok()?;
                    // A backward run deletes one place to the left each time,
                    // so it covers the places that end right after `pos`.
                    let first = if *len > 0 {
                        usize::try_from(*pos).ok()?
                    } else {
                        usize::try_from(*pos)
                            .ok()?
                            .checked_add(1)?
                            .checked_sub(size)?
                    };
                    let from = cut(&mut pieces, Pos::from_usize(first))?;
                    let to = cut(&mut pieces, Pos::from_usize(first + size))?;
                    // The identities rise from left to right: the first one
                    // names the leftmost character.
                    let mut offset = Len::<Chars>::ZERO;
                    for piece in pieces.drain(from..to) {
                        let len = Len::<Chars>::of(piece.len.to_usize());
                        if let Some(old) = piece.old {
                            let start = start_id.inc(i32::try_from(offset.to_usize()).ok()?);
                            gone.push((old, IdRun { start, len }));
                        }
                        offset += len;
                    }
                }
                JsonTextOp::Mark { .. } | JsonTextOp::MarkEnd => marked = true,
            }
        }
        gone.sort_unstable_by_key(|(old, _)| *old);
        let mut runs = Vec::new();
        for (_, run) in gone {
            push(&mut runs, run);
        }
        Some(runs)
    }
}

/// The identities of `runs`, cut into consecutive parts of `lens`
/// characters. `None` when the lengths do not add up to the runs.
pub(super) fn split(runs: &[IdRun], lens: &[Len<Chars>]) -> Option<Vec<Vec<IdRun>>> {
    if lens.iter().fold(Len::ZERO, |sum, len| sum + *len) != total(runs) {
        return None;
    }
    let mut out = Vec::with_capacity(lens.len());
    let mut rest = runs.iter().copied();
    let mut current = rest.next();
    for want in lens {
        let mut part = Vec::new();
        let mut left = *want;
        while !left.is_zero() {
            let run = current?;
            let piece = left.min(run.len);
            push(&mut part, run.slice(Len::ZERO, piece)?);
            left = left.saturating_sub(piece);
            current = if piece < run.len {
                run.slice(piece, run.len.saturating_sub(piece))
            } else {
                rest.next()
            };
        }
        out.push(part);
    }
    Some(out)
}
