//! Three-way merge of a plain text, for a note body and an arrow's caption.
//!
//! An editor closes on `typed`, opened on `base`, while the document may hold
//! `stored`, a peer's edit of the same `base`. [`merge_text`] keeps both
//! changes. Logic writes its result (`TOILE-UI-381`, `TOILE-263`), and the
//! view paints the same result the moment the editor closes (`TOILE-UI-382`).
//! It is pure and holds no CRDT, so the UI may call it (`UI-THREAD-051`).
//! Both threads then compute the same text from the same three strings.

use ambre_core::{ByteCount, CharCount, CharOffset};

/// Edit steps past which a diff stops looking for the shortest script. A
/// longer change becomes one hunk over everything between the common prefix
/// and the common suffix, which bounds the cost of a close.
const EDIT_CAP: usize = 256;

/// Which change a hunk comes from. `Stored` sorts first, so where the two
/// changes collide, the peer's text precedes the typed text.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Stored,
    Typed,
}

/// One run of the base, `start..end` in characters, replaced by `text`. An
/// empty run is an insertion before `start`.
struct Hunk {
    start: CharOffset,
    end: CharOffset,
    text: String,
    side: Side,
}

/// One step of an edit script over the trimmed middle of the base.
enum Step {
    /// The base character at this index goes.
    Delete(CharOffset),
    /// The character comes in before the base character at this index.
    Insert(CharOffset, char),
}

/// One edit of the stored text: `delete` characters from `at` go, and `insert`
/// comes in their place. Positions count characters, and each edit's `at`
/// reads the text with the edits before it already applied.
#[derive(PartialEq, Eq)]
pub struct Edit {
    pub at: CharOffset,
    pub delete: CharCount,
    pub insert: String,
}

/// The inserted text prints as its length alone (`ZK-006`).
impl std::fmt::Debug for Edit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Edit")
            .field("at", &self.at)
            .field("delete", &self.delete)
            .field("insert", &super::TextLen(&self.insert))
            .finish()
    }
}

/// A merge: the text it yields, and the edits that turn the stored text into
/// it. Logic writes the edits, so the CRDT is spared a diff of its own over the
/// whole body (`TOILE-UI-383`).
pub struct Merge {
    pub text: String,
    pub edits: Vec<Edit>,
}

/// The text that keeps the change from `base` to `stored` and the change from
/// `base` to `typed`. Where the two touch different runs of the base, both
/// apply. Where they replace the same run differently, the stored run comes
/// first and the typed run follows, so no word of either is lost.
pub fn merge_text(base: &str, stored: &str, typed: &str) -> String {
    merge_edits(base, stored, typed).text
}

/// The merge [`merge_text`] yields, with the edits that bring `stored` to it.
/// The stored text only ever gains the typed change, so an edit is a typed
/// run or a typed run placed after the peer's.
pub fn merge_edits(base: &str, stored: &str, typed: &str) -> Merge {
    if stored == typed || typed == base {
        return Merge {
            text: stored.to_owned(),
            edits: Vec::new(),
        };
    }
    // Neither side touches what all three texts share at their ends, so only
    // the middle is decoded and diffed. A close then costs little more than a
    // copy of the body, however long it is (`TOILE-UI-383`).
    let (head, tail) = common_ends(base, stored, typed);
    fn middle(s: &str, head: ByteCount, tail: ByteCount) -> &str {
        s.get(head.to_usize()..s.len() - tail.to_usize())
            .unwrap_or_default()
    }
    let mut merge = merge_middle(
        middle(base, head, tail),
        middle(stored, head, tail),
        middle(typed, head, tail),
    );
    // A Rust string slices at bytes, so the ends cross into bytes here only.
    let (head, tail) = (head.to_usize(), tail.to_usize());
    let shift = base.get(..head).unwrap_or_default().chars().count();
    for e in &mut merge.edits {
        e.at += CharCount::from_len(shift);
    }
    let mut text = String::with_capacity(head + merge.text.len() + tail);
    text.push_str(base.get(..head).unwrap_or_default());
    text.push_str(&merge.text);
    text.push_str(base.get(base.len() - tail..).unwrap_or_default());
    Merge {
        text,
        edits: merge.edits,
    }
}

/// The bytes the three texts share at their start and at their end. Both cut
/// at a character boundary, and they never overlap in any of the three.
fn common_ends(base: &str, stored: &str, typed: &str) -> (ByteCount, ByteCount) {
    let (a, b, c) = (base.as_bytes(), stored.as_bytes(), typed.as_bytes());
    let same = |((x, y), z): &((&u8, &u8), &u8)| x == y && y == z;
    let mut head = a.iter().zip(b).zip(c).take_while(same).count();
    // The three agree byte for byte up to `head`, so a boundary in one is a
    // boundary in all.
    while !base.is_char_boundary(head) {
        head -= 1;
    }
    let room = a.len().min(b.len()).min(c.len()) - head;
    let mut tail = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .zip(c.iter().rev())
        .take(room)
        .take_while(same)
        .count();
    while !base.is_char_boundary(base.len() - tail) {
        tail -= 1;
    }
    (ByteCount::from_len(head), ByteCount::from_len(tail))
}

/// [`merge_edits`] over texts that share no start and no end, positions
/// counted from the start of the middle.
fn merge_middle(base: &str, stored: &str, typed: &str) -> Merge {
    let base: Vec<char> = base.chars().collect();
    let stored: Vec<char> = stored.chars().collect();
    let typed: Vec<char> = typed.chars().collect();
    let mut hunks = diff(&base, &stored, Side::Stored);
    hunks.extend(diff(&base, &typed, Side::Typed));
    hunks.sort_by_key(|h| (h.start, h.end, h.side));

    let mut out = String::with_capacity(4 * (stored.len() + typed.len()));
    let mut edits = Vec::new();
    // The characters written so far: where the next edit lands in the stored
    // text once the edits before it apply.
    let mut pos = 0;
    let mut at = CharOffset::ZERO;
    let mut rest = hunks.as_slice();
    while let Some(first) = rest.first() {
        let (group, tail, end) = cluster(rest);
        let kept = run(&base, at, first.start);
        out.extend(kept);
        pos += kept.len();
        let mine = region(&base, group, Side::Stored, first.start, end);
        let theirs = region(&base, group, Side::Typed, first.start, end);
        match (mine, theirs) {
            (Some(s), Some(t)) if s == t => {
                pos += s.chars().count();
                out.push_str(&s);
            }
            (Some(s), Some(t)) => {
                pos += s.chars().count();
                out.push_str(&s);
                out.push_str(&t);
                let insert = Edit {
                    at: CharOffset::from_usize(pos),
                    delete: CharCount::ZERO,
                    insert: t,
                };
                pos += insert.insert.chars().count();
                edits.push(insert);
            }
            (Some(s), None) => {
                pos += s.chars().count();
                out.push_str(&s);
            }
            (None, Some(t)) => {
                out.push_str(&t);
                let replace = Edit {
                    at: CharOffset::from_usize(pos),
                    delete: CharCount::from_len(run(&base, first.start, end).len()),
                    insert: t,
                };
                pos += replace.insert.chars().count();
                edits.push(replace);
            }
            (None, None) => {}
        }
        at = end;
        rest = tail;
    }
    out.extend(run(&base, at, CharOffset::from_usize(base.len())));
    Merge { text: out, edits }
}

/// The base characters `start..end`, or none when the run is out of range.
fn run(base: &[char], start: CharOffset, end: CharOffset) -> &[char] {
    base.get(start.to_usize()..end.to_usize())
        .unwrap_or_default()
}

/// The leading hunks of a sorted list that collide, the hunks after them, and
/// the base index their group ends at. Two runs collide when they overlap, and
/// two insertions collide when they land at the same index. An insertion at
/// the edge of a run does not collide with it: it lands before or after it.
fn cluster(hunks: &[Hunk]) -> (&[Hunk], &[Hunk], CharOffset) {
    let Some(first) = hunks.first() else {
        return (&[], hunks, CharOffset::ZERO);
    };
    let mut end = first.end;
    let mut points = first.start == first.end;
    let mut len = 1;
    for h in hunks.iter().skip(1) {
        let point = h.start == h.end;
        if !(h.start < end || (points && point && h.start == first.start)) {
            break;
        }
        end = end.max(h.end);
        points &= point;
        len += 1;
    }
    let (group, rest) = hunks.split_at(len);
    (group, rest, end)
}

/// The base run `start..end` with the hunks of one side applied, or `None`
/// when that side leaves the run alone.
fn region(
    base: &[char],
    group: &[Hunk],
    side: Side,
    start: CharOffset,
    end: CharOffset,
) -> Option<String> {
    let mut hunks = group.iter().filter(|h| h.side == side).peekable();
    hunks.peek()?;
    let mut out = String::new();
    let mut at = start;
    for h in hunks {
        out.extend(run(base, at, h.start));
        out.push_str(&h.text);
        at = h.end;
    }
    out.extend(run(base, at, end));
    Some(out)
}

/// The hunks that turn `base` into `side`. The common prefix and suffix are
/// trimmed first, so a single typed run costs no search at all.
fn diff(base: &[char], side: &[char], tag: Side) -> Vec<Hunk> {
    let head = base.iter().zip(side).take_while(|(a, b)| a == b).count();
    let a = base.get(head..).unwrap_or_default();
    let b = side.get(head..).unwrap_or_default();
    let tail = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let a = a.get(..a.len() - tail).unwrap_or_default();
    let b = b.get(..b.len() - tail).unwrap_or_default();
    let whole = || Hunk {
        start: CharOffset::from_usize(head),
        end: CharOffset::from_usize(head + a.len()),
        text: b.iter().collect(),
        side: tag,
    };
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Vec::new(),
        (true, false) | (false, true) => vec![whole()],
        (false, false) => match myers(a, b) {
            Some(steps) => hunks(steps, CharCount::from_len(head), tag),
            None => vec![whole()],
        },
    }
}

/// Group a script into hunks, shifting each index by `head`. Adjacent steps
/// share one hunk.
fn hunks(steps: Vec<Step>, head: CharCount, tag: Side) -> Vec<Hunk> {
    let mut out: Vec<Hunk> = Vec::new();
    for step in steps {
        let (at, gone, ch) = match step {
            Step::Delete(i) => (i + head, CharCount::new(1), None),
            Step::Insert(i, c) => (i + head, CharCount::ZERO, Some(c)),
        };
        match out.last_mut() {
            Some(h) if h.end == at => {
                h.end += gone;
                h.text.extend(ch);
            }
            _ => out.push(Hunk {
                start: at,
                end: at + gone,
                text: ch.into_iter().collect(),
                side: tag,
            }),
        }
    }
    out
}

/// The entry of a Myers frontier for diagonal `k`.
fn reach(v: &[isize], k: isize, offset: isize) -> Option<isize> {
    v.get(usize::try_from(k + offset).ok()?).copied()
}

/// Whether the path to diagonal `k` at depth `d` comes down from `k + 1`, an
/// insertion, rather than across from `k - 1`, a deletion.
fn comes_down(v: &[isize], k: isize, d: isize, offset: isize) -> Option<bool> {
    if k == -d {
        return Some(true);
    }
    if k == d {
        return Some(false);
    }
    Some(reach(v, k - 1, offset)? < reach(v, k + 1, offset)?)
}

/// The shortest edit script from `a` to `b` (Myers, 1986), or `None` past
/// [`EDIT_CAP`] steps.
fn myers(a: &[char], b: &[char]) -> Option<Vec<Step>> {
    let n = isize::try_from(a.len()).ok()?;
    let m = isize::try_from(b.len()).ok()?;
    let max = isize::try_from((a.len() + b.len()).min(EDIT_CAP)).ok()?;
    let offset = max + 1;
    let mut v = vec![0_isize; usize::try_from(2 * max + 3).ok()?];
    let mut trace = Vec::new();
    for d in 0..=max {
        // Depth `d` reads diagonals `-d - 1..=d + 1` only, so the trace keeps
        // that window and not the whole frontier. The copy grows with the depth
        // reached, not with the cap.
        let window =
            usize::try_from(offset - d - 1).ok()?..=usize::try_from(offset + d + 1).ok()?;
        trace.push(v.get(window)?.to_vec());
        for k in (-d..=d).step_by(2) {
            let mut x = if comes_down(&v, k, d, offset)? {
                reach(&v, k + 1, offset)?
            } else {
                reach(&v, k - 1, offset)? + 1
            };
            let mut y = x - k;
            while x < n && y < m && a.get(x.unsigned_abs()) == b.get(y.unsigned_abs()) {
                x += 1;
                y += 1;
            }
            *v.get_mut(usize::try_from(k + offset).ok()?)? = x;
            if x >= n && y >= m {
                return backtrack(&trace, b, n, m);
            }
        }
    }
    None
}

/// Walk the frontiers back from the end of both texts to the script. The
/// frontier kept at depth `d` starts at diagonal `-d - 1`.
fn backtrack(trace: &[Vec<isize>], b: &[char], n: isize, m: isize) -> Option<Vec<Step>> {
    let (mut x, mut y) = (n, m);
    let mut steps = Vec::new();
    for (d, v) in trace.iter().enumerate().rev() {
        let d = isize::try_from(d).ok()?;
        let offset = d + 1;
        let k = x - y;
        let down = comes_down(v, k, d, offset)?;
        let prev_k = if down { k + 1 } else { k - 1 };
        let prev_x = reach(v, prev_k, offset)?;
        let prev_y = prev_x - prev_k;
        if d > 0 {
            let at = CharOffset::from_usize(usize::try_from(prev_x).ok()?);
            steps.push(if down {
                Step::Insert(at, *b.get(usize::try_from(prev_y).ok()?)?)
            } else {
                Step::Delete(at)
            });
        }
        x = prev_x;
        y = prev_y;
    }
    steps.reverse();
    Some(steps)
}

// Host-only: proptest does not build on wasm32, and the merge is pure.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    /// An edit's debug form prints the insert's length, none of it.
    #[test]
    fn an_edit_debug_form_prints_no_text() {
        let edit = Edit {
            at: CharOffset::new(3),
            delete: CharCount::new(1),
            insert: "wire funds".to_owned(),
        };
        let form = format!("{edit:?}");
        assert!(!form.contains("funds"), "{form}");
        assert!(form.contains("<10 chars>"), "{form}");
    }

    #[test]
    fn two_runs_apart_both_apply() {
        assert_eq!(
            merge_text("buy milk", "please buy milk", "buy milk and eggs"),
            "please buy milk and eggs"
        );
        assert_eq!(
            merge_text("cost", "unit cost", "cost in EUR"),
            "unit cost in EUR"
        );
    }

    #[test]
    fn edits_inside_a_long_text_both_apply() {
        let base = "alpha beta gamma delta epsilon";
        let stored = "alpha BETA gamma delta epsilon";
        let typed = "alpha beta gamma delta EPSILON!";
        assert_eq!(
            merge_text(base, stored, typed),
            "alpha BETA gamma delta EPSILON!"
        );
    }

    #[test]
    fn a_deletion_and_an_edit_elsewhere_both_apply() {
        assert_eq!(
            merge_text("one two three", "one three", "one two three four"),
            "one three four"
        );
    }

    #[test]
    fn a_clash_keeps_both_words_peer_first() {
        assert_eq!(merge_text("red", "pink", "cyan"), "pinkcyan");
    }

    #[test]
    fn the_same_change_on_both_sides_lands_once() {
        assert_eq!(merge_text("a b", "a X b", "a X b c"), "a X b c");
    }

    #[test]
    fn an_insert_at_the_edge_of_a_deletion_survives() {
        assert_eq!(
            merge_text("keep gone", "keep ", "keep NEW gone"),
            "keep NEW "
        );
    }

    #[test]
    fn an_untouched_side_yields_the_other() {
        assert_eq!(merge_text("x", "x", "y"), "y");
        assert_eq!(merge_text("x", "y", "x"), "y");
        assert_eq!(merge_text("x", "y", "y"), "y");
    }

    #[test]
    fn multibyte_text_merges_by_character() {
        assert_eq!(merge_text("été", "un été", "été chaud"), "un été chaud");
    }

    #[test]
    fn a_change_past_the_cap_still_keeps_both_changes() {
        let base: String = "ab".repeat(400);
        let stored = format!("X{base}");
        let typed: String = base.replace('a', "c");
        let merged = merge_text(&base, &stored, &typed);
        assert!(merged.starts_with('X'), "the peer's insertion was lost");
        assert!(merged.ends_with(&typed), "the typed text was lost");
    }

    proptest::proptest! {
        /// A diff applied to its base rebuilds the other text exactly, on a
        /// small alphabet, so the texts share many characters.
        #[test]
        fn a_diff_rebuilds_its_target(base in "[abc ]{0,40}", side in "[abc ]{0,40}") {
            let base: Vec<char> = base.chars().collect();
            let side: Vec<char> = side.chars().collect();
            let hunks = diff(&base, &side, Side::Typed);
            let end = CharOffset::from_usize(base.len());
            let rebuilt = region(&base, &hunks, Side::Typed, CharOffset::ZERO, end)
                .unwrap_or_else(|| base.iter().collect());
            proptest::prop_assert_eq!(rebuilt, side.iter().collect::<String>());
        }

        /// The edits of a merge, replayed on the stored text in order, yield
        /// the merged text: what Logic writes is what the view echoes. The
        /// alphabet carries a multibyte character, so a position counted in
        /// bytes fails.
        #[test]
        fn the_edits_of_a_merge_rebuild_it_from_the_stored_text(
            base in "[ab é]{0,30}",
            stored in "[ab é]{0,30}",
            typed in "[ab é]{0,30}",
        ) {
            let merge = merge_edits(&base, &stored, &typed);
            let mut text: Vec<char> = stored.chars().collect();
            for e in &merge.edits {
                let (at, delete) = (e.at.to_usize(), e.delete.to_usize());
                proptest::prop_assert!(at + delete <= text.len());
                text.splice(at..at + delete, e.insert.chars());
            }
            proptest::prop_assert_eq!(text.into_iter().collect::<String>(), merge.text);
        }
    }
}
