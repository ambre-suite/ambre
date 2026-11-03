// ambre-toile/src/ui/presenter.rs
// ============================================================================
// Pure model for the presenter overlay panel (toile-presentation). Two host-
// testable helpers the wasm-only `chrome::PresenterPanel` builds on:
//   - `slide_counter_label`: the "k / n" slide counter text.
//   - `notes_delta`: a full-textarea replacement reduced to one char-CRDT splice
//     (`TextDelta`). It is computed over unicode codepoints, so its indices line
//     up with the loro `LoroText` positions the Logic engine clamps
//     (presentation.rs `edit_notes`). A `<textarea>` only exposes its whole value
//     on `input`, so the panel diffs the old and new strings into the minimal
//     splice here.
// No DOM, no Loro: this stays in `ui` and is host-tested, like `ui::find`.
// ============================================================================

use ambre_core::index::{Len, Pos};
use ambre_core::{Char, Utf16, char_len, char_to_utf16, utf16_to_char};

use ambre_bus::app::toile::Broadcast;
use ambre_core::id::{Id, User};

use crate::logic::command::{Slide, SlidePosition, TextDelta};
use ambre_core::Redacted;

/// An intent emitted by the presenter panel. The launcher maps each variant to a
/// `ToileCommand` sent to Logic. `Prev`/`Next` -> `PrevSlide`/`NextSlide`,
/// `Exit` -> `ExitPresent`, and `EditNotes` -> `EditNotes { frame, base, delta }`, the
/// launcher supplying the active `frame` from the last slide change. Then
/// `ToggleBroadcast` -> `SetPresentBroadcast`, and `Follow` -> `FollowPresenter`
/// ("Follow presenter").
#[derive(Clone, PartialEq, Eq)]
pub enum PresenterIntent {
    Prev,
    Next,
    Exit,
    /// A splice, and the note it was measured against (`TOILE-FEAT-088`).
    EditNotes {
        base: String,
        delta: TextDelta,
    },
    /// Toggle whether this viewer broadcasts its playback position to peers.
    ToggleBroadcast(Broadcast),
    /// Follow the given presenter (`Some`) or stop following (`None`).
    Follow(Option<Id<User>>),
}

/// The note and the splice's insert print as their lengths alone (`ZK-006`).
impl std::fmt::Debug for PresenterIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prev => f.write_str("Prev"),
            Self::Next => f.write_str("Next"),
            Self::Exit => f.write_str("Exit"),
            Self::EditNotes { base, delta } => f
                .debug_struct("EditNotes")
                .field("base", &Redacted::new(base))
                .field("at", &delta.at)
                .field("delete", &delta.delete)
                .field("insert", &Redacted::new(&delta.insert))
                .finish(),
            Self::ToggleBroadcast(b) => f.debug_tuple("ToggleBroadcast").field(b).finish(),
            Self::Follow(who) => f.debug_tuple("Follow").field(who).finish(),
        }
    }
}

/// The presenter panel's slide counter, `1`-based for humans: `"3 / 10"`. An
/// empty deck reads `"0 / 0"` rather than `"1 / 0"`.
pub fn slide_counter_label(slide: SlidePosition) -> String {
    let count = slide.count();
    let human = if count.is_zero() {
        Len::<Slide>::ZERO
    } else {
        slide.index().number()
    };
    format!("{human} / {count}")
}

/// Reduce a full-textarea replacement (`old` -> `new`) to the single minimal
/// splice that turns one into the other. `None` when they are equal, a no-op the
/// caller skips, matching `edit_notes`' empty-delta guard. The splice is
/// expressed in **unicode codepoints**, not bytes. A common-prefix and a
/// common-suffix are trimmed, and what remains in the middle is the deleted span
/// (`old`) and the inserted text (`new`). This yields one contiguous edit, which
/// is exactly what a typed keystroke or a paste produces. It is not a minimal
/// edit script for arbitrary rewrites, but the panel only ever sees incremental
/// textarea input.
pub fn notes_delta(old: &str, new: &str) -> Option<TextDelta> {
    if old == new {
        return None;
    }
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();

    // Common prefix length (in codepoints).
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();

    // Common suffix length, not overlapping the shared prefix on either side.
    let suffix = old
        .iter()
        .skip(prefix)
        .rev()
        .zip(new.iter().skip(prefix).rev())
        .take_while(|(a, b)| a == b)
        .count();

    let delete = old.len() - prefix - suffix;
    let insert: String = new
        .iter()
        .skip(prefix)
        .take(new.len() - prefix - suffix)
        .collect();
    Some(TextDelta {
        at: Pos::<Char>::from_usize(prefix),
        delete: Len::<Char>::of(delete),
        insert,
    })
}

/// Where a caret sits once the text area turns from `before` to `after`
/// under a write the user did not type (`TOILE-UI-316`). Offsets are UTF-16
/// units, as a `<textarea>` counts them. A caret before the change stays, a
/// caret after it shifts with it, and one inside a removed run lands after
/// what replaced it.
pub fn caret_through(before: &str, caret: Pos<Utf16>, after: &str) -> Pos<Utf16> {
    let Some(delta) = notes_delta(before, after) else {
        return caret;
    };
    let pos = utf16_to_char(before, caret);
    let inserted = char_len(&delta.insert);
    let moved = if pos <= delta.at {
        pos
    } else if pos >= delta.at + delta.delete {
        pos - delta.delete + inserted
    } else {
        delta.at + inserted
    };
    char_to_utf16(after, moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::command::Slide;

    /// `TOILE-289`: a notes edit's debug form prints lengths, none of the note.
    #[test]
    fn a_notes_edit_debug_form_prints_no_note() {
        let base = "merger talk".to_owned();
        let delta = notes_delta(&base, "merger talk friday").expect("a splice");
        let form = format!("{:?}", PresenterIntent::EditNotes { base, delta });
        assert!(!form.contains("merger"), "{form}");
        assert!(!form.contains("friday"), "{form}");
        assert!(form.contains("base: 11 chars"), "{form}");
        assert!(form.contains("insert: 7 chars"), "{form}");
    }

    /// `TOILE-UI-316`: the notes caret shifts past a peer's prefix and stays put
    /// under a suffix.
    #[test]
    fn a_caret_keeps_its_place_under_a_peer_write() {
        // A prefix before the caret shifts it; a suffix after it does not.
        assert_eq!(
            caret_through("Hello world", Pos::<Utf16>::new(5), "Dear Hello world"),
            Pos::<Utf16>::new(10)
        );
        assert_eq!(
            caret_through("Hello world", Pos::<Utf16>::new(5), "Hello world!"),
            Pos::<Utf16>::new(5)
        );
        // A caret inside a removed run lands after its replacement.
        assert_eq!(
            caret_through("Hello world", Pos::<Utf16>::new(8), "Hello you"),
            Pos::<Utf16>::new(9)
        );
        // Offsets count UTF-16 units: an astral emoji is two.
        assert_eq!(
            caret_through("\u{1f600}ab", Pos::<Utf16>::new(3), "x\u{1f600}ab"),
            Pos::<Utf16>::new(4)
        );
        assert_eq!(
            caret_through("same", Pos::<Utf16>::new(2), "same"),
            Pos::<Utf16>::new(2)
        );
    }

    fn position(index: u32, count: u32) -> SlidePosition {
        SlidePosition::new(Pos::<Slide>::new(index), Len::<Slide>::new(count))
    }

    #[test]
    fn counter_label_is_one_based() {
        let label = |i, n| slide_counter_label(position(i, n));
        assert_eq!(label(0, 10), "1 / 10");
        assert_eq!(label(2, 10), "3 / 10");
        assert_eq!(label(9, 10), "10 / 10");
    }

    #[test]
    fn counter_label_empty_deck() {
        assert_eq!(slide_counter_label(position(0, 0)), "0 / 0");
    }

    /// A slide position keeps its index inside the deck (`UI-THREAD-060`).
    #[test]
    fn a_slide_position_stays_inside_its_deck() {
        assert_eq!(position(12, 10).index(), Pos::<Slide>::new(9));
        assert_eq!(position(3, 0).index(), Pos::<Slide>::ZERO);
        assert_eq!(slide_counter_label(position(12, 10)), "10 / 10");
    }

    #[test]
    fn delta_none_on_no_change() {
        assert_eq!(notes_delta("hello", "hello"), None);
        assert_eq!(notes_delta("", ""), None);
    }

    #[test]
    fn delta_pure_append() {
        // "abc" -> "abcd": insert "d" at 3, delete nothing.
        assert_eq!(
            notes_delta("abc", "abcd"),
            Some(TextDelta {
                at: Pos::<Char>::new(3),
                delete: Len::<Char>::ZERO,
                insert: "d".to_owned()
            })
        );
    }

    #[test]
    fn delta_pure_prefix_insert() {
        // "bc" -> "abc": insert "a" at 0.
        assert_eq!(
            notes_delta("bc", "abc"),
            Some(TextDelta {
                at: Pos::<Char>::ZERO,
                delete: Len::<Char>::ZERO,
                insert: "a".to_owned()
            })
        );
    }

    #[test]
    fn delta_pure_delete() {
        // "abcd" -> "abd": delete 1 codepoint at 2.
        assert_eq!(
            notes_delta("abcd", "abd"),
            Some(TextDelta {
                at: Pos::<Char>::new(2),
                delete: Len::<Char>::new(1),
                insert: String::new()
            })
        );
    }

    #[test]
    fn delta_mid_string_replace() {
        // "abXYcd" -> "abZcd": at 2, delete "XY" (2), insert "Z".
        assert_eq!(
            notes_delta("abXYcd", "abZcd"),
            Some(TextDelta {
                at: Pos::<Char>::new(2),
                delete: Len::<Char>::new(2),
                insert: "Z".to_owned()
            })
        );
    }

    #[test]
    fn delta_from_empty() {
        assert_eq!(
            notes_delta("", "hi"),
            Some(TextDelta {
                at: Pos::<Char>::ZERO,
                delete: Len::<Char>::ZERO,
                insert: "hi".to_owned()
            })
        );
    }

    #[test]
    fn delta_to_empty() {
        assert_eq!(
            notes_delta("hi", ""),
            Some(TextDelta {
                at: Pos::<Char>::ZERO,
                delete: Len::<Char>::new(2),
                insert: String::new()
            })
        );
    }

    #[test]
    fn delta_multibyte_indices_are_codepoints() {
        // Each "e-acute" is one codepoint but two bytes; inserting "!" after the
        // second must land at codepoint index 2, not a byte offset.
        assert_eq!(
            notes_delta("\u{e9}\u{e9}", "\u{e9}\u{e9}!"),
            Some(TextDelta {
                at: Pos::<Char>::new(2),
                delete: Len::<Char>::ZERO,
                insert: "!".to_owned()
            })
        );
    }

    #[test]
    fn delta_multibyte_mid_replace() {
        // "a-cafe-b" style: replace the middle multibyte run, prefix/suffix
        // trimmed by codepoint. "x\u{e9}y" -> "x\u{fc}y": at 1, delete 1, insert
        // the new codepoint.
        assert_eq!(
            notes_delta("x\u{e9}y", "x\u{fc}y"),
            Some(TextDelta {
                at: Pos::<Char>::new(1),
                delete: Len::<Char>::new(1),
                insert: "\u{fc}".to_owned()
            })
        );
    }

    #[test]
    fn delta_repeated_char_prefers_prefix() {
        // "aaa" -> "aa": one deletion. Prefix eats 2, suffix 0 (capped by the
        // remaining length), so delete 1 at index 2.
        assert_eq!(
            notes_delta("aaa", "aa"),
            Some(TextDelta {
                at: Pos::<Char>::new(2),
                delete: Len::<Char>::new(1),
                insert: String::new()
            })
        );
    }
}
