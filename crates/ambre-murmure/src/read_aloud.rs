//! What a screen reader hears of the composer (the Murmure composer spec).
//!
//! The draft is drawn on the canvas and the pipe holds no text, so a screen
//! reader hears the keys and nothing of the draft. After each frame, the
//! composer says what its last key did, through the polite live region
//! (`MURMURE-571` to `MURMURE-574`).

use ambre_core::index::{Len, Pos, Span};
use ambre_core::{Byte, Char};

use crate::markers::DraftEdit;

/// The longest text one announcement carries. A long draft is said by its
/// start, as a Prose block is.
const TEXT_LIMIT: Len<Char> = Len::<Char>::new(200);

/// What the last key of a frame asks the composer to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Say {
    /// A move by a char: the char after the caret.
    Char,
    /// A move by a word: the word after the caret.
    Word,
    /// A move by a line, or to its start or end: the line of the caret.
    Line,
    /// A delete: the text it took.
    Removed,
    /// The whole draft, as the field takes the focus or a history step lands.
    Draft,
}

/// Where the caret of a draft stands once a key landed, in bytes of the
/// text after the key. Three names, so the selection and the line drawn
/// never swap at a call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaretAt {
    /// The caret, `None` with no caret.
    pub caret: Option<Pos<Byte>>,
    /// The selection, `None` with none.
    pub selection: Option<Span<Byte>>,
    /// The line the layout draws around the caret, `None` with no layout.
    pub drawn: Option<Span<Byte>>,
}

/// What a screen reader hears of the draft once the key that asked for
/// `say` made `edit`. `at` places the caret in the text after it. A move
/// that leaves a selection says the text selected.
pub fn said(say: Say, edit: DraftEdit<'_>, at: CaretAt) -> Option<String> {
    let CaretAt {
        caret,
        selection,
        drawn,
    } = at;
    let text = edit.after;
    let moved = matches!(say, Say::Char | Say::Word | Say::Line);
    if moved && let Some(selected) = selection.and_then(|range| text.get(range.to_usize_range())) {
        return Some(format!("{}, selected", clipped(selected)));
    }
    let after = caret.and_then(|at| text.get(at.to_usize()..));
    match say {
        Say::Char => after.map(|rest| char_name(rest.chars().next())),
        Say::Word => after.map(|rest| {
            let word = rest.split(char::is_whitespace).next().unwrap_or_default();
            if word.is_empty() {
                char_name(rest.chars().next())
            } else {
                clipped(word)
            }
        }),
        // The line drawn, a wrapped one on a narrow screen. With no layout
        // to read, the line between two breaks.
        Say::Line => drawn
            .and_then(|range| text.get(range.to_usize_range()))
            .or_else(|| caret.and_then(|at| line_at(text, at)))
            .map(spoken),
        Say::Removed => removed(edit).map(|gone| {
            let mut chars = gone.chars();
            match (chars.next(), chars.next()) {
                (Some(one), None) => char_name(Some(one)),
                _ => clipped(gone),
            }
        }),
        Say::Draft => Some(spoken(text)),
    }
}

/// The logical line of `text` that holds the byte `at`.
fn line_at(text: &str, at: Pos<Byte>) -> Option<&str> {
    let at = at.to_usize();
    let start = text.get(..at)?.rfind('\n').map_or(0, |n| n + 1);
    let end = text.get(at..)?.find('\n').map_or(text.len(), |n| at + n);
    text.get(start..end)
}

/// The chars a delete took from the draft in `edit`. `None` unless the edit
/// took one run of chars out and typed none.
fn removed<'a>(edit: DraftEdit<'a>) -> Option<&'a str> {
    let DraftEdit { before, after } = edit;
    if after.len() >= before.len() {
        return None;
    }
    let prefix: usize = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let (rest_before, rest_after) = (before.get(prefix..)?, after.get(prefix..)?);
    let suffix: usize = rest_before
        .chars()
        .rev()
        .zip(rest_after.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let gone = rest_before.get(..rest_before.len() - suffix)?;
    (rest_after.len() == suffix && !gone.is_empty()).then_some(gone)
}

/// A text as said: its start, or `blank` when it holds only spaces.
fn spoken(text: &str) -> String {
    if text.trim().is_empty() {
        "blank".to_owned()
    } else {
        clipped(text.trim())
    }
}

/// The start of `text`, at most `TEXT_LIMIT` chars.
fn clipped(text: &str) -> String {
    text.chars().take(TEXT_LIMIT.to_usize()).collect()
}

/// A char as a screen reader names it, `blank` past the end.
fn char_name(c: Option<char>) -> String {
    match c {
        None => "blank".to_owned(),
        Some(' ') => "space".to_owned(),
        Some('\n') => "new line".to_owned(),
        Some('\t') => "tab".to_owned(),
        Some(c) => c.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key that moved the caret: the text stays.
    fn kept(text: &str) -> DraftEdit<'_> {
        DraftEdit {
            before: text,
            after: text,
        }
    }

    /// The caret at the byte `at`, with no selection and no layout.
    fn caret(at: usize) -> CaretAt {
        CaretAt {
            caret: Some(Pos::<Byte>::from_usize(at)),
            ..CaretAt::default()
        }
    }

    /// A move by a char says the char after the caret, by its name for a
    /// space, a line break or the end. A move by a word says the word
    /// (`MURMURE-571`).
    #[test]
    fn a_move_says_the_char_or_the_word_after_the_caret() {
        let text = "one two\nthree";
        let say = |say, at| said(say, kept(text), caret(at));
        assert_eq!(say(Say::Char, 0).as_deref(), Some("o"));
        assert_eq!(say(Say::Char, 3).as_deref(), Some("space"));
        assert_eq!(say(Say::Char, 7).as_deref(), Some("new line"));
        assert_eq!(say(Say::Char, text.len()).as_deref(), Some("blank"));
        assert_eq!(say(Say::Word, 4).as_deref(), Some("two"));
        assert_eq!(say(Say::Word, 3).as_deref(), Some("space"));
        assert_eq!(
            said(Say::Char, kept(text), CaretAt::default()),
            None,
            "no caret, nothing said"
        );
        assert_eq!(
            say(Say::Char, 99),
            None,
            "a caret past the text says nothing"
        );
    }

    /// A move by a line says the line of the caret, between its breaks, and
    /// an empty line says `blank` (`MURMURE-571`).
    #[test]
    fn a_line_move_says_the_line_of_the_caret() {
        let text = "one\n\nthree";
        let say = |at| said(Say::Line, kept(text), caret(at));
        assert_eq!(say(1).as_deref(), Some("one"));
        assert_eq!(say(4).as_deref(), Some("blank"));
        assert_eq!(say(text.len()).as_deref(), Some("three"));
    }

    /// A line move on a draft the layout wraps says the line drawn, not the
    /// text between its breaks (`MURMURE-576`).
    #[test]
    fn a_line_move_says_the_line_drawn() {
        let text = "one two three\nfour";
        let say = |drawn: Option<std::ops::Range<usize>>| {
            let at = CaretAt {
                drawn: drawn.map(Span::<Byte>::from_usize_range),
                ..caret(5)
            };
            said(Say::Line, kept(text), at)
        };
        assert_eq!(say(Some(4..8)).as_deref(), Some("two"));
        assert_eq!(say(None).as_deref(), Some("one two three"), "no layout");
        assert_eq!(
            say(Some(4..99)).as_deref(),
            Some("one two three"),
            "a line off the text falls back to the breaks"
        );
    }

    /// A delete says what it took: a char by its name, more by their text.
    /// An edit that took nothing says nothing (`MURMURE-572`).
    #[test]
    fn a_delete_says_what_it_took() {
        let take = |before, after| {
            said(
                Say::Removed,
                DraftEdit { before, after },
                CaretAt::default(),
            )
        };
        assert_eq!(take("one two", "one tw").as_deref(), Some("o"));
        assert_eq!(take("one two", "onetwo").as_deref(), Some("space"));
        assert_eq!(take("one two", "one ").as_deref(), Some("two"));
        assert_eq!(take("aaa", "aa").as_deref(), Some("a"), "a repeated char");
        assert_eq!(take("é!", "!").as_deref(), Some("é"));
        assert_eq!(take("one", "one!"), None, "a key that typed");
        assert_eq!(take("one", "ox"), None, "a replace is not a delete");
        assert_eq!(take("one", "one"), None);
    }

    /// A move that leaves a selection says the text selected
    /// (`MURMURE-573`).
    #[test]
    fn a_selection_says_its_text() {
        let text = "one two";
        let at = CaretAt {
            selection: Some(Span::<Byte>::from_usize_range(4..7)),
            ..CaretAt::default()
        };
        let said = said(Say::Char, kept(text), at);
        assert_eq!(said.as_deref(), Some("two, selected"));
    }

    /// The draft is said whole, trimmed and clipped, and an empty one says
    /// `blank` (`MURMURE-574`).
    #[test]
    fn the_draft_is_said_by_its_start() {
        let draft = |after: &str| {
            let edit = DraftEdit { before: "", after };
            said(Say::Draft, edit, CaretAt::default())
        };
        assert_eq!(draft("  hi \n").as_deref(), Some("hi"));
        assert_eq!(draft(" ").as_deref(), Some("blank"));
        let long = "a".repeat(TEXT_LIMIT.to_usize() + 5);
        assert_eq!(
            draft(&long).map(|s| Len::<Char>::of(s.chars().count())),
            Some(TEXT_LIMIT)
        );
    }
}
