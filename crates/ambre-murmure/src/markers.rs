//! The style marks of a draft (the Murmure composer spec).
//!
//! A Markdown marker becomes its mark the moment it closes, and its chars
//! leave the draft (`MURMURE-197`). A marker typed in code stays text
//! (`MURMURE-198`). A chord puts a style on the selection, or takes it off
//! (`MURMURE-431`). The marks follow the text through each edit, as the
//! mentions do (`MURMURE-553`).

use ambre_bus::{StyleMark, TextStyle};
use ambre_commands::{Key, KeyChar};
use ambre_core::{
    ByteCount, ByteOffset, ByteSpan, CharOffset, CharSpan, ViewPoint, byte_to_char, char_len,
};

use crate::Region;
use crate::style::linkable;

/// The chord of each style, `Ctrl` with this key, the one Prose binds to the
/// same mark (`MURMURE-431`).
pub const MARK_CHORDS: [(KeyChar, TextStyle); 3] = [
    (KeyChar::B, TextStyle::Bold),
    (KeyChar::I, TextStyle::Italic),
    (KeyChar::E, TextStyle::Code),
];

/// The controls of the format bar, in the order Prose shows the same marks,
/// each with the title Prose gives its mark (`MURMURE-432`).
pub const MARK_BAR: [(TextStyle, &str); 3] = [
    (TextStyle::Bold, "Bold"),
    (TextStyle::Italic, "Italic"),
    (TextStyle::Code, "Inline code"),
];

/// How much of a selection a style covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    Whole,
    Part,
    None,
}

/// What the page draws of the format bar: each control's style and cover
/// of the selection, hung over the caret (`MURMURE-432`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkBar {
    pub controls: Vec<(TextStyle, Cover)>,
    /// The top-left of the caret, in pixels of the region's canvas.
    pub at: ViewPoint,
    /// The region the bar hangs over.
    pub region: Region,
}

/// How much of the chars `range` the runs of `style` in `styles` cover.
pub fn cover(styles: &[StyleMark], style: &TextStyle, range: CharSpan) -> Cover {
    let runs = crate::style::normalized(styles);
    let mut runs = runs.iter().filter(|run| run.style == *style);
    if runs
        .clone()
        .any(|run| run.range.start <= range.start && range.end <= run.range.end)
    {
        Cover::Whole
    } else if runs.any(|run| run.range.start < range.end && range.start < run.range.end) {
        Cover::Part
    } else {
        Cover::None
    }
}

/// The style of a chord's key, `None` for a key no style takes.
pub fn chord_style(key: Key) -> Option<TextStyle> {
    let Key::Char(key) = key else {
        return None;
    };
    MARK_CHORDS
        .iter()
        .find(|(chord, _)| *chord == key)
        .map(|(_, style)| style.clone())
}

/// The chars a key closes a marker with. Only such a key asks for a look.
pub fn closes(text: &str) -> bool {
    matches!(text, "*" | "_" | "`" | ")")
}

/// A marker the caret just closed: the bytes it spans, open marker to close
/// marker, and the text between them, which takes the style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closed {
    pub span: ByteSpan,
    /// The bytes of the text that stays, inside `span`.
    pub inner: ByteSpan,
    pub style: TextStyle,
}

/// A draft once the markers of a [`Closed`] leave it, in two steps. Named
/// fields, so the two drafts never swap at a read.
///
/// `Debug` prints the length of each draft alone: a draft is plaintext a
/// member typed.
#[derive(Clone, PartialEq, Eq)]
pub struct Unmarked {
    /// The draft with the close marker gone alone.
    pub between: String,
    /// The draft with both markers gone.
    pub after: String,
}

impl std::fmt::Debug for Unmarked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Unmarked")
            .field("between", &ByteCount::from_len(self.between.len()))
            .field("after", &ByteCount::from_len(self.after.len()))
            .finish()
    }
}

impl Closed {
    /// The draft once the markers leave it, and the one between, with the
    /// close marker gone alone. The marks follow the two in turn, so a
    /// mention between the markers stays whole.
    pub fn apply(&self, text: &str) -> Unmarked {
        let (span, inner) = (self.span.to_usize_range(), self.inner.to_usize_range());
        let between = format!("{}{}", &text[..inner.end], &text[span.end..]);
        let after = format!(
            "{}{}{}",
            &text[..span.start],
            &text[inner],
            &text[span.end..]
        );
        Unmarked { between, after }
    }

    /// The chars of the text that stays, once the markers left.
    pub fn range(&self, text: &str) -> CharSpan {
        let start = byte_to_char(text, self.span.start);
        CharSpan::from_count(start, char_len(&text[self.inner.to_usize_range()]))
    }
}

/// Whether `inner` may sit between two markers: some text, and no blank at
/// either end.
fn holds(inner: &str) -> bool {
    !inner.is_empty()
        && !inner.starts_with(char::is_whitespace)
        && !inner.ends_with(char::is_whitespace)
}

/// The marker the text before the byte `caret` closes, on its own line, or
/// `None`. `code` lists the chars inline code covers: a marker in it stays
/// text (`MURMURE-198`).
///
/// `**text**` is bold, `_text_` italic, `` `text` `` inline code, and
/// `[text](address)` a link to an `http` or `https` address.
pub fn closed(text: &str, caret: ByteOffset, code: &[CharSpan]) -> Option<Closed> {
    let before = text.get(..caret.get())?;
    if let Some(block) = code_block(before) {
        return outside(text, block, code);
    }
    let line_start = before.rfind('\n').map_or(0, |at| at + 1);
    let line = &before[line_start..];
    let found = bold(line)
        .or_else(|| italic(line))
        .or_else(|| inline_code(line))
        .or_else(|| link(line))?;
    let shift = |span: ByteSpan| {
        let by = ByteCount::from_len(line_start);
        ByteSpan::new(span.start + by, span.end + by)
    };
    let closed = Closed {
        span: shift(found.span),
        inner: shift(found.inner),
        style: found.style,
    };
    outside(text, closed, code)
}

/// The block marker of the line a line break just ended, the last break
/// before the byte `caret`, or `None` (`MURMURE-563`). Text typed after the
/// break in the same frame leaves it found. `code` is as for [`closed`].
pub fn closed_by_break(text: &str, caret: ByteOffset, code: &[CharSpan]) -> Option<Closed> {
    let end = text.get(..caret.get())?.rfind('\n')? + 1;
    let line_start = text[..end - 1].rfind('\n').map_or(0, |at| at + 1);
    let line = ByteSpan::from_usize_range(line_start..end);
    outside(text, line_marker(text, line)?, code)
}

/// The block marker of the last line of `text`, closed by the send of the
/// draft, or `None` (`MURMURE-563`). `code` is as for [`closed`].
pub fn closed_at_send(text: &str, code: &[CharSpan]) -> Option<Closed> {
    let line_start = text.rfind('\n').map_or(0, |at| at + 1);
    let line = ByteSpan::from_usize_range(line_start..text.len());
    outside(text, line_marker(text, line)?, code)
}

/// `closed`, unless a marker of it touches code (`MURMURE-198`).
fn outside(text: &str, closed: Closed, code: &[CharSpan]) -> Option<Closed> {
    let first = byte_to_char(text, closed.span.start);
    let last = byte_to_char(text, closed.span.end)
        .prev()
        .unwrap_or(CharOffset::ZERO);
    let in_code = code.iter().any(|run| {
        run.contains(first) || run.contains(last) || (first < run.start && run.end <= last)
    });
    (!in_code).then_some(closed)
}

/// The marker of a quote, `> `, or of a list item, `- `, that opens a line
/// of `text`. The bytes `line` hold the line, its line break included.
/// The mark takes the line, and the line must hold some text.
fn line_marker(text: &str, span: ByteSpan) -> Option<Closed> {
    let (start, end) = (span.start.get(), span.end.get());
    let line = text.get(start..end)?;
    // A fence a line above still opens a code block, since a closed one
    // took its fences out: the line is code.
    if text[..start].lines().any(|above| above.starts_with(FENCE)) {
        return None;
    }
    let style = if line.starts_with(QUOTE_MARKER) {
        TextStyle::Quote
    } else if line.starts_with(LIST_MARKER) {
        TextStyle::List
    } else {
        return None;
    };
    let inner = start + QUOTE_MARKER.len()..end;
    (!text[inner.clone()].trim().is_empty()).then_some(Closed {
        span,
        inner: ByteSpan::from_usize_range(inner),
        style,
    })
}

/// The marker of a quote at the start of a line.
const QUOTE_MARKER: &str = "> ";
/// The marker of a list item at the start of a line, as long as a quote's.
const LIST_MARKER: &str = "- ";
/// The fence of a code block.
const FENCE: &str = "```";

/// A code block closing `before`: a fence at the start of a line, the code,
/// then the fence typed last. A line break just inside a fence leaves with
/// it, and the code must hold some text.
fn code_block(before: &str) -> Option<Closed> {
    let open_end = before.strip_suffix(FENCE)?;
    if open_end.ends_with('`') {
        return None;
    }
    let open = open_end.rfind(FENCE)?;
    let at_line_start = open == 0 || open_end[..open].ends_with('\n');
    let mut inner = open + FENCE.len()..open_end.len();
    if open_end[inner.clone()].starts_with('\n') {
        inner.start += 1;
    }
    if inner.start < inner.end && open_end[inner.clone()].ends_with('\n') {
        inner.end -= 1;
    }
    let code = &open_end[inner.clone()];
    (at_line_start && !code.trim().is_empty() && !code.starts_with('`')).then_some(Closed {
        span: ByteSpan::from_usize_range(open..before.len()),
        inner: ByteSpan::from_usize_range(inner),
        style: TextStyle::CodeBlock,
    })
}

/// `**text**` closing `line`.
fn bold(line: &str) -> Option<Closed> {
    let open_end = line.strip_suffix("**")?;
    let open = open_end.rfind("**")?;
    let inner = open + 2..open_end.len();
    holds(&line[inner.clone()]).then_some(Closed {
        span: ByteSpan::from_usize_range(open..line.len()),
        inner: ByteSpan::from_usize_range(inner),
        style: TextStyle::Bold,
    })
}

/// `_text_` closing `line`, its open marker at a word's start, so a
/// `snake_case_name` stays text.
fn italic(line: &str) -> Option<Closed> {
    let open_end = line.strip_suffix('_')?;
    let open = open_end.rfind('_')?;
    let starts = line[..open]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric() && c != '_');
    let inner = open + 1..open_end.len();
    (starts && holds(&line[inner.clone()])).then_some(Closed {
        span: ByteSpan::from_usize_range(open..line.len()),
        inner: ByteSpan::from_usize_range(inner),
        style: TextStyle::Italic,
    })
}

/// `` `text` `` closing `line`. Three backticks open a block, which this
/// story leaves to text.
fn inline_code(line: &str) -> Option<Closed> {
    let open_end = line.strip_suffix('`')?;
    let open = open_end.rfind('`')?;
    let inner = open + 1..open_end.len();
    let text = &line[inner.clone()];
    (!text.trim().is_empty() && !line[..open].ends_with('`')).then_some(Closed {
        span: ByteSpan::from_usize_range(open..line.len()),
        inner: ByteSpan::from_usize_range(inner),
        style: TextStyle::Code,
    })
}

/// `[text](address)` closing `line`, with an address a link may hold
/// (`MURMURE-561`).
fn link(line: &str) -> Option<Closed> {
    let open_end = line.strip_suffix(')')?;
    let middle = open_end.rfind("](")?;
    let address = &open_end[middle + 2..];
    let open = open_end[..middle].rfind('[')?;
    let inner = open + 1..middle;
    let text = &line[inner.clone()];
    let address = ambre_core::LinkUrl::from(address);
    (holds(text) && !text.contains(']') && linkable(&address)).then(|| Closed {
        span: ByteSpan::from_usize_range(open..line.len()),
        inner: ByteSpan::from_usize_range(inner),
        style: TextStyle::Link(address),
    })
}

/// The text of a draft before an edit, and after it. A pair of names, so
/// the two texts never swap at a call.
///
/// `Debug` prints the length of each text alone: a draft is plaintext a
/// member typed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DraftEdit<'a> {
    /// The text before the edit.
    pub before: &'a str,
    /// The text after the edit.
    pub after: &'a str,
}

impl std::fmt::Debug for DraftEdit<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DraftEdit")
            .field("before", &ByteCount::from_len(self.before.len()))
            .field("after", &ByteCount::from_len(self.after.len()))
            .finish()
    }
}

/// Where an edit falls, in chars: the common head, and the end of the
/// change in the text before and in the text after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Changed {
    pub head: CharOffset,
    pub old_end: CharOffset,
    pub new_end: CharOffset,
}

/// Where `edit` falls.
pub(crate) fn change(edit: DraftEdit<'_>) -> Changed {
    let (old, new): (Vec<char>, Vec<char>) =
        (edit.before.chars().collect(), edit.after.chars().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    Changed {
        head: CharOffset::from_usize(head),
        old_end: CharOffset::from_usize(old.len() - tail),
        new_end: CharOffset::from_usize(new.len() - tail),
    }
}

/// The styles of the text before `edit` once it made the text after. A run
/// grows or shrinks with an edit inside it, and loses the chars an edit took
/// from its ends. Text typed at either end stays out of it.
pub fn kept_styles(edit: DraftEdit<'_>, styles: &[StyleMark]) -> Vec<StyleMark> {
    let Changed {
        head,
        old_end,
        new_end,
    } = change(edit);
    let shifted = |at: CharOffset| at + (new_end - old_end);
    styles
        .iter()
        .filter_map(|mark| {
            let (start, end) = (mark.range.start, mark.range.end);
            let start = if start < head {
                start
            } else if start >= old_end {
                shifted(start)
            } else {
                new_end
            };
            let end = if end <= head {
                end
            } else if end >= old_end {
                shifted(end)
            } else {
                head
            };
            (start < end).then(|| StyleMark {
                style: mark.style.clone(),
                range: CharSpan::new(start, end),
            })
        })
        .collect()
}

/// `styles` with `style` put on `range`, or taken off it when it already
/// covers every char of it (`MURMURE-431`). An empty range changes nothing.
pub fn toggled(styles: &[StyleMark], style: &TextStyle, range: CharSpan) -> Vec<StyleMark> {
    if range.is_empty() {
        return styles.to_vec();
    }
    let runs = crate::style::normalized(styles);
    if cover(&runs, style, range) != Cover::Whole {
        let mut added = runs;
        added.push(StyleMark {
            style: style.clone(),
            range,
        });
        return crate::style::normalized(&added);
    }
    let mut kept = Vec::with_capacity(runs.len() + 1);
    for run in runs {
        if run.style != *style || run.range.end <= range.start || range.end <= run.range.start {
            kept.push(run);
            continue;
        }
        let parts = [
            CharSpan::new(run.range.start, range.start),
            CharSpan::new(range.end, run.range.end),
        ];
        for part in parts {
            if !part.is_empty() {
                kept.push(StyleMark {
                    style: run.style.clone(),
                    range: part,
                });
            }
        }
    }
    crate::style::normalized(&kept)
}

#[cfg(test)]
mod tests {
    use std::ops::Range;
    use std::slice::from_ref;

    use super::*;

    fn mark(style: TextStyle, range: Range<usize>) -> StyleMark {
        StyleMark {
            style,
            range: CharSpan::from_usize_range(range),
        }
    }

    /// The draft once the marker `text` closes at its end leaves it, and the
    /// chars of the text that took the style.
    fn converted(text: &str) -> Option<(String, Range<usize>, TextStyle)> {
        let closed = closed(text, ByteOffset::from_usize(text.len()), &[])?;
        let after = closed.apply(text).after;
        Some((after, closed.range(text).to_usize_range(), closed.style))
    }

    /// The draft once the break that ends `text` closes the marker of its
    /// line, and the chars of the text that took the style.
    fn broken(text: &str) -> Option<(String, Range<usize>, TextStyle)> {
        let closed = closed_by_break(text, ByteOffset::from_usize(text.len()), &[])?;
        let after = closed.apply(text).after;
        Some((after, closed.range(text).to_usize_range(), closed.style))
    }

    /// Each marker becomes its mark as it closes, and its chars leave the
    /// draft (`MURMURE-197`).
    #[test]
    fn each_marker_becomes_its_mark_as_it_closes() {
        let site = TextStyle::Link("https://ambre.dev".into());
        let cases = [
            ("say **done**", "say done", 4..8, TextStyle::Bold),
            ("an _aside_", "an aside", 3..8, TextStyle::Italic),
            ("run `ls -a`", "run ls -a", 4..9, TextStyle::Code),
            ("see [site](https://ambre.dev)", "see site", 4..8, site),
            ("é **ü ö**", "é ü ö", 2..5, TextStyle::Bold),
        ];
        for (typed, after, range, style) in cases {
            assert_eq!(
                converted(typed),
                Some((after.to_owned(), range, style)),
                "{typed}"
            );
        }

        // The caret closes the marker before it, on its own line.
        let text = "a\n**b** tail";
        let closed = closed(text, ByteOffset::from_usize("a\n**b**".len()), &[])
            .expect("closed at the caret");
        assert_eq!(
            closed.apply(text),
            Unmarked {
                between: "a\n**b tail".to_owned(),
                after: "a\nb tail".to_owned(),
            }
        );
        assert_eq!(closed.range(text), CharSpan::from_usize_range(2..3));
        assert_eq!(
            super::closed("é", ByteOffset::new(1), &[]),
            None,
            "not a char boundary"
        );
    }

    /// Text that only looks like a marker stays text.
    #[test]
    fn a_marker_that_holds_no_text_or_no_address_stays_text() {
        for typed in [
            "****",
            "** x**",
            "**x **",
            "**a\nb**",
            "snake_case_",
            "__",
            "``",
            "```",
            "` `",
            "[x](javascript:alert(1))",
            "[x](ftp://host)",
            "[](https://ambre.dev)",
            "[ x](https://ambre.dev)",
            "[a]b](https://ambre.dev)",
            "plain",
        ] {
            assert_eq!(converted(typed), None, "{typed}");
        }
        assert!(
            converted("call _now_").is_some(),
            "an italic opens at a word's start"
        );
    }

    /// A block marker at the start of a line becomes its mark as it closes
    /// (`MURMURE-563`). A quote or a list item closes on its line break or on
    /// the send, and a code block on its closing fence.
    #[test]
    fn each_block_marker_becomes_its_mark_as_it_closes() {
        let lines = [
            ("> said\n", "said\n", 0..5, TextStyle::Quote),
            ("a\n- item\n", "a\nitem\n", 2..7, TextStyle::List),
            ("> é ü\n", "é ü\n", 0..4, TextStyle::Quote),
        ];
        for (typed, after, range, style) in lines {
            assert_eq!(
                broken(typed),
                Some((after.to_owned(), range, style)),
                "{typed}"
            );
        }
        let blocks = [
            ("```let x```", "let x", 0..5),
            ("hi\n```\nfn a\nfn b\n```", "hi\nfn a\nfn b", 3..12),
        ];
        for (typed, after, range) in blocks {
            assert_eq!(
                converted(typed),
                Some((after.to_owned(), range, TextStyle::CodeBlock)),
                "{typed}"
            );
        }

        // Text typed after the break in the same frame leaves the line found.
        let text = "> one\n two";
        let closed = closed_by_break(text, ByteOffset::from_usize(text.len()), &[])
            .expect("closed at the break");
        assert_eq!(closed.apply(text).after, "one\n two");
        assert_eq!(closed.range(text), CharSpan::from_usize_range(0..4));
        assert_eq!(closed_by_break("no break", ByteOffset::new(8), &[]), None);
        assert_eq!(
            closed_by_break("é\n", ByteOffset::new(1), &[]),
            None,
            "not a char boundary"
        );

        // The send closes the marker of the last line, and only of it.
        let sent = |text: &str| {
            let closed = closed_at_send(text, &[])?;
            Some((
                closed.apply(text).after,
                closed.range(text).to_usize_range(),
                closed.style,
            ))
        };
        let quote = Some(("a\nlast".to_owned(), 2..6, TextStyle::Quote));
        assert_eq!(sent("a\n> last"), quote);
        assert_eq!(
            sent("- one"),
            Some(("one".to_owned(), 0..3, TextStyle::List))
        );
        assert_eq!(sent("> a\nplain"), None, "the last line holds no marker");
    }

    /// A block marker that holds no text, or sits past a line's start,
    /// stays text.
    #[test]
    fn a_block_marker_that_holds_no_text_stays_text() {
        for typed in ["> \n", "-  \n", "a > b\n", ">no space\n", "plain\n"] {
            assert_eq!(broken(typed), None, "{typed:?}");
        }
        for typed in ["``````", "```\n```", "a ```b```", "````b```"] {
            assert_eq!(converted(typed), None, "{typed:?}");
        }
        assert_eq!(closed_at_send("> ", &[]), None);
    }

    /// A block marker typed in a code block stays text (`MURMURE-198`).
    #[test]
    fn a_block_marker_typed_in_a_code_block_stays_text() {
        let text = "fn a\n> b\n";
        assert_eq!(
            closed_by_break(
                text,
                ByteOffset::from_usize(text.len()),
                from_ref(&CharSpan::from_usize_range(0..9))
            ),
            None
        );
        assert_eq!(
            closed_at_send("fn a\n- b", from_ref(&CharSpan::from_usize_range(0..8))),
            None
        );
        assert!(
            closed_by_break(text, ByteOffset::from_usize(text.len()), &[]).is_some(),
            "the same line outside code"
        );
        let text = "```\n> b\n";
        assert_eq!(
            closed_by_break(text, ByteOffset::from_usize(text.len()), &[]),
            None,
            "under a fence still open"
        );
    }

    /// A marker that touches inline code stays text (`MURMURE-198`).
    #[test]
    fn a_marker_typed_in_code_stays_text() {
        let text = "`a **b**`";
        let caret = "`a **b**".len();
        assert_eq!(
            closed(
                text,
                ByteOffset::from_usize(caret),
                from_ref(&CharSpan::from_usize_range(1..8))
            ),
            None,
            "inside a code run"
        );
        let text = "**a`b`c**";
        assert_eq!(
            closed(
                text,
                ByteOffset::from_usize(text.len()),
                from_ref(&CharSpan::from_usize_range(3..4))
            ),
            None,
            "around a code run"
        );
        assert!(
            closed(text, ByteOffset::from_usize(text.len()), &[]).is_some(),
            "the same marker outside code"
        );
        let text = "x **b**";
        assert!(
            closed(
                text,
                ByteOffset::from_usize(text.len()),
                from_ref(&CharSpan::from_usize_range(0..1))
            )
            .is_some(),
            "code beside the marker leaves it a marker"
        );
    }

    /// `Ctrl` with `b`, `i` and `e` names its style, and any other key none
    /// (`MURMURE-431`).
    #[test]
    fn a_chord_names_its_style() {
        assert_eq!(chord_style(Key::char('b')), Some(TextStyle::Bold));
        assert_eq!(chord_style(Key::char('I')), Some(TextStyle::Italic));
        assert_eq!(chord_style(Key::char('e')), Some(TextStyle::Code));
        for key in [
            Key::char('x'),
            Key::Space,
            Key::Enter,
            Key::F(ambre_commands::FunctionKey::F2),
        ] {
            assert_eq!(chord_style(key), None, "{key:?}");
        }
        for key in ["*", "_", "`", ")"] {
            assert!(closes(key), "{key}");
        }
        assert!(!closes("a") && !closes("**"));
    }

    /// A run follows the text through an edit, and text typed at either end
    /// stays out of it.
    #[test]
    fn a_style_follows_the_text_through_an_edit() {
        let bold = |range| vec![mark(TextStyle::Bold, range)];
        let kept = |before, after| kept_styles(DraftEdit { before, after }, &bold(2..6));
        assert_eq!(kept("a bold", "a bold!"), bold(2..6), "typed at its end");
        assert_eq!(kept("a bold", "a Xbold"), bold(3..7), "typed at its start");
        assert_eq!(kept("a bold", "a boXld"), bold(2..7), "typed inside");
        assert_eq!(kept("a bold", "Xa bold"), bold(3..7), "typed before");
        assert_eq!(kept("a bold", "a bd"), bold(2..4), "cut inside");
        assert_eq!(kept("a bold", "a ld"), bold(2..4), "cut at its start");
        assert_eq!(kept("a bold", "a "), [], "cut whole");
        assert_eq!(kept("a bold", "a bXX"), bold(2..5), "chars replaced inside");
        assert_eq!(kept("a bold x", "a bo"), bold(2..4), "cut past its end");
    }

    /// The debug form of an edit names the length of each text, never a
    /// char of it: a draft is plaintext.
    #[test]
    fn an_edit_debugs_its_lengths_alone() {
        let edit = DraftEdit {
            before: "secret draft",
            after: "secret",
        };
        let shown = format!("{edit:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("12") && shown.contains('6'), "{shown}");
        let unmarked = Unmarked {
            between: "a **secret".to_owned(),
            after: "a secret".to_owned(),
        };
        let shown = format!("{unmarked:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("10") && shown.contains('8'), "{shown}");
    }

    /// A chord puts its style on the selection, and takes it off a
    /// selection it covers whole (`MURMURE-431`).
    #[test]
    fn a_chord_puts_a_style_on_or_takes_it_off() {
        let bold = TextStyle::Bold;
        let on = toggled(&[], &bold, CharSpan::from_usize_range(2..4));
        assert_eq!(on, [mark(bold.clone(), 2..4)]);
        assert_eq!(toggled(&on, &bold, CharSpan::from_usize_range(2..4)), []);
        assert_eq!(
            toggled(
                &[mark(bold.clone(), 0..4)],
                &bold,
                CharSpan::from_usize_range(1..2)
            ),
            [mark(bold.clone(), 0..1), mark(bold.clone(), 2..4)]
        );
        assert_eq!(
            toggled(
                &[mark(bold.clone(), 0..4)],
                &bold,
                CharSpan::from_usize_range(3..6)
            ),
            [mark(bold.clone(), 0..6)],
            "a selection half covered takes the style"
        );
        let italic = mark(TextStyle::Italic, 0..4);
        assert_eq!(
            toggled(
                &[italic.clone(), mark(bold.clone(), 0..4)],
                &bold,
                CharSpan::from_usize_range(0..4)
            ),
            from_ref(&italic),
            "another style stays"
        );
        assert_eq!(
            toggled(from_ref(&italic), &bold, CharSpan::from_usize_range(2..2)),
            [italic]
        );
    }

    /// The bar presses a control whose style covers the selection whole,
    /// half-presses one covering part of it, and leaves the rest
    /// (`MURMURE-432`). Another style counts for nothing.
    #[test]
    fn a_control_shows_how_much_of_the_selection_its_style_covers() {
        let bold = TextStyle::Bold;
        let styles = [mark(bold.clone(), 0..2), mark(bold.clone(), 2..4)];
        assert_eq!(
            cover(&styles, &bold, CharSpan::from_usize_range(1..4)),
            Cover::Whole,
            "two runs, one cover"
        );
        assert_eq!(
            cover(&styles, &bold, CharSpan::from_usize_range(3..6)),
            Cover::Part
        );
        assert_eq!(
            cover(&styles, &bold, CharSpan::from_usize_range(4..6)),
            Cover::None,
            "a run ending at it"
        );
        assert_eq!(
            cover(
                &styles,
                &TextStyle::Italic,
                CharSpan::from_usize_range(0..2)
            ),
            Cover::None
        );
        let shown: Vec<TextStyle> = MARK_BAR.iter().map(|(s, _)| s.clone()).collect();
        let chords: Vec<TextStyle> = MARK_CHORDS.iter().map(|(_, s)| s.clone()).collect();
        assert_eq!(shown, chords, "the bar shows the marks a chord takes");
    }
}
