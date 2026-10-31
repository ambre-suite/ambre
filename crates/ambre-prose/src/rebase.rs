//! Two edits made from one text, each rewritten to apply after the other
//! (`features/ambre-spec-prose-text-editing.md section 3`).
//!
//! The view sends an edit before it has seen what the Logic thread applied
//! meanwhile. The Logic thread, in turn, applies changes the view has not
//! seen yet.
//! Both sides run the same transform, so both land on one text. The Logic
//! thread's change goes first when both touch one position.
//!
//! An edit is a [`TextEdit`]: an insertion when `delta` is positive, a
//! deletion of `-delta` code points when it is negative. A list of edits is
//! sequential: each one counts positions in the text the previous one left.

use ambre_bus::app::prose::{DocCount, DocDelta, DocOffset};

use crate::events::{BlockId, TextEdit};

/// The code points an edit covers.
fn len(edit: &TextEdit) -> DocCount {
    edit.delta.magnitude()
}

fn insertion(block_id: BlockId, at: DocOffset, n: DocCount) -> TextEdit {
    TextEdit {
        block_id,
        at,
        delta: DocDelta::from_count(n),
    }
}

/// A deletion of `n` code points, or `None` when there is nothing to delete.
fn deletion(block_id: BlockId, at: DocOffset, n: DocCount) -> Option<TextEdit> {
    (!n.is_zero()).then(|| TextEdit {
        block_id,
        at,
        delta: DocDelta::from_count(n).reversed(),
    })
}

/// Where a position lands once a deletion of `[at, at + n)` applied.
fn through_deletion(pos: DocOffset, at: DocOffset, n: DocCount) -> DocOffset {
    if pos <= at {
        pos
    } else if pos >= at + n {
        pos - n
    } else {
        at
    }
}

/// Where a position of `block` lands once `edit` applied. An insertion at the
/// position goes before it.
#[must_use]
pub(crate) fn through(pos: DocOffset, block: BlockId, edit: &TextEdit) -> DocOffset {
    if edit.block_id != block {
        return pos;
    }
    if !edit.delta.is_negative() {
        if edit.at <= pos {
            return pos + len(edit);
        }
        return pos;
    }
    through_deletion(pos, edit.at, len(edit))
}

/// Where a position lands once a run of edits applied, one after another.
#[must_use]
pub(crate) fn through_all(pos: DocOffset, block: BlockId, edits: &[TextEdit]) -> DocOffset {
    edits
        .iter()
        .fold(pos, |pos, edit| through(pos, block, edit))
}

/// Where the end of a range lands once `edits` applied. An insertion at the
/// end stays outside the range.
#[must_use]
pub(crate) fn end_through_all(pos: DocOffset, block: BlockId, edits: &[TextEdit]) -> DocOffset {
    edits.iter().fold(pos, |pos, edit| {
        if edit.block_id == block && !edit.delta.is_negative() && edit.at == pos {
            pos
        } else {
            through(pos, block, edit)
        }
    })
}

/// Two edits made from one text: the view's `c`, and the Logic thread's `o`.
/// Answers `c` rewritten to apply after `o`, and `o` to apply after `c`.
///
/// A deletion takes the text a concurrent insertion put inside its range. An
/// insertion inside a concurrent deletion survives where the deletion was.
#[must_use]
pub(crate) fn transform(c: &TextEdit, o: &TextEdit) -> (Option<TextEdit>, Vec<TextEdit>) {
    if c.block_id != o.block_id {
        return (Some(*c), vec![*o]);
    }
    let block = c.block_id;
    let (p, n) = (c.at, len(c));
    let (q, m) = (o.at, len(o));
    match (!c.delta.is_negative(), !o.delta.is_negative()) {
        (true, true) => {
            if q <= p {
                (Some(insertion(block, p + m, n)), vec![*o])
            } else {
                (Some(*c), vec![insertion(block, q + n, m)])
            }
        }
        (true, false) => {
            if p <= q {
                (Some(*c), deletion(block, q + n, m).into_iter().collect())
            } else if p >= q + m {
                (Some(insertion(block, p - m, n)), vec![*o])
            } else {
                // The insertion survives where the deletion was, and the
                // deletion goes around it.
                let before = deletion(block, q, p.count_from(q));
                let after = deletion(block, q + n, (q + m).count_from(p));
                (
                    Some(insertion(block, q, n)),
                    before.into_iter().chain(after).collect(),
                )
            }
        }
        (false, true) => {
            if q <= p {
                (deletion(block, p + m, n), vec![*o])
            } else if q >= p + n {
                (Some(*c), vec![insertion(block, q - n, m)])
            } else {
                // The deletion takes the text inserted inside it.
                (deletion(block, p, n + m), Vec::new())
            }
        }
        (false, false) => {
            let start = through_deletion(p, q, m);
            let c_after = deletion(
                block,
                start,
                through_deletion(p + n, q, m).count_from(start),
            );
            let start = through_deletion(q, p, n);
            let o_after = deletion(
                block,
                start,
                through_deletion(q + m, p, n).count_from(start),
            );
            (c_after, o_after.into_iter().collect())
        }
    }
}

/// Two lists of edits made from one text: the view's `cs`, and the Logic
/// thread's `os`. Answers `cs` rewritten after `os`, and `os` after `cs`.
#[must_use]
pub(crate) fn transform_all(cs: &[TextEdit], os: &[TextEdit]) -> (Vec<TextEdit>, Vec<TextEdit>) {
    let mut cs = cs.to_vec();
    let mut out = Vec::with_capacity(os.len());
    for o in os {
        let mut current = vec![*o];
        let mut next_cs = Vec::with_capacity(cs.len());
        for c in cs {
            let mut c_now = Some(c);
            let mut next = Vec::with_capacity(current.len());
            for part in current {
                match c_now {
                    Some(held) => {
                        let (c_after, parts) = transform(&held, &part);
                        c_now = c_after;
                        next.extend(parts);
                    }
                    None => next.push(part),
                }
            }
            current = next;
            next_cs.extend(c_now);
        }
        cs = next_cs;
        out.extend(current);
    }
    (cs, out)
}

/// The edits that undo `edits`, last first. What a deletion took comes back
/// as an insertion of as many code points.
#[must_use]
pub(crate) fn inverse(edits: &[TextEdit]) -> Vec<TextEdit> {
    edits
        .iter()
        .rev()
        .map(|edit| TextEdit {
            delta: edit.delta.reversed(),
            ..*edit
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: BlockId = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(0));
    const B: BlockId = BlockId::new(ambre_core::PeerId::new(1), ambre_core::OpCounter::new(1));

    fn ins(at: u32, n: u32) -> TextEdit {
        insertion(A, DocOffset::new(at), DocCount::new(n))
    }

    fn del(at: u32, n: u32) -> TextEdit {
        TextEdit {
            block_id: A,
            at: DocOffset::new(at),
            delta: DocDelta::new(-i64::from(n)),
        }
    }

    /// Apply a run of edits to a text, marking what each inserts with `tag`.
    fn apply(text: &str, edits: &[TextEdit], tag: char) -> String {
        let mut chars: Vec<char> = text.chars().collect();
        for edit in edits {
            let at = edit.at.to_usize();
            let n = edit.delta.magnitude().to_usize();
            if edit.delta.is_negative() {
                chars.drain(at..at + n);
            } else {
                for _ in 0..n {
                    chars.insert(at, tag);
                }
            }
        }
        chars.into_iter().collect()
    }

    /// Every pair converges: `c` then `o'` reads the same as `o` then `c'`.
    #[test]
    fn both_orders_land_on_one_text() {
        let text = "abcdefghij";
        let mut edits = Vec::new();
        for at in 0..=10 {
            edits.push(ins(at, 2));
            for n in 1..=(10 - at).min(4) {
                edits.push(del(at, n));
            }
        }
        for c in &edits {
            for o in &edits {
                let (c_after, o_after) = transform(c, o);
                let c_first = apply(&apply(text, &[*c], 'C'), &o_after, 'O');
                let c_after: Vec<TextEdit> = c_after.into_iter().collect();
                let o_first = apply(&apply(text, &[*o], 'O'), &c_after, 'C');
                assert_eq!(c_first, o_first, "c {c:?}, o {o:?}");
            }
        }
    }

    #[test]
    fn two_lists_land_on_one_text() {
        let text = "abcdefghij";
        let cs = [ins(3, 2), del(1, 3), ins(6, 1)];
        let os = [del(2, 4), ins(0, 3), del(5, 2)];
        let (c_after, o_after) = transform_all(&cs, &os);
        let c_first = apply(&apply(text, &cs, 'C'), &o_after, 'O');
        let o_first = apply(&apply(text, &os, 'O'), &c_after, 'C');
        assert_eq!(c_first, o_first);
    }

    #[test]
    fn the_logic_threads_insertion_goes_first_at_one_position() {
        let (c_after, o_after) = transform(&ins(4, 2), &ins(4, 3));
        assert_eq!(c_after, Some(ins(7, 2)));
        assert_eq!(o_after, vec![ins(4, 3)]);
        assert_eq!(through(DocOffset::new(4), A, &ins(4, 3)), DocOffset::new(7));
    }

    #[test]
    fn another_block_is_left_alone() {
        let other = insertion(B, DocOffset::ZERO, DocCount::new(5));
        assert_eq!(
            transform(&ins(2, 1), &other),
            (Some(ins(2, 1)), vec![other])
        );
        assert_eq!(through(DocOffset::new(2), A, &other), DocOffset::new(2));
    }

    #[test]
    fn the_inverse_takes_the_edits_back_last_first() {
        let edits = [ins(1, 2), del(0, 1)];
        assert_eq!(inverse(&edits), vec![ins(0, 1), del(1, 2)]);
        let back = apply(&apply("abcdef", &edits, 'X'), &inverse(&edits), 'Y');
        assert_eq!(back.len(), 6);
    }
}
