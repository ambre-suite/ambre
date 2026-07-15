// ambre-toile/src/ui/presenter.rs
// ============================================================================
// Pure model for the presenter overlay panel (toile-features §1.D). Two host-
// testable helpers the wasm-only `chrome::PresenterPanel` builds on:
//   - `slide_counter_label`: the "k / n" slide counter text.
//   - `notes_delta`: a full-textarea replacement reduced to one char-CRDT splice
//     (`TextDelta`), computed over unicode codepoints so its indices line up with
//     the loro `LoroText` positions the Logic engine clamps (presentation.rs
//     `edit_notes`). A `<textarea>` only exposes its whole value on `input`, so
//     the panel diffs the old and new strings into the minimal splice here.
// No DOM, no Loro: this stays in `ui` and is host-tested, like `ui::find`.
// ============================================================================

use crate::logic::command::TextDelta;

/// The presenter panel's slide counter, `1`-based for humans: `"3 / 10"`. An
/// empty deck (`count == 0`) reads `"0 / 0"` rather than `"1 / 0"`.
pub fn slide_counter_label(index: usize, count: usize) -> String {
    let human = if count == 0 { 0 } else { index + 1 };
    format!("{human} / {count}")
}

/// Reduce a full-textarea replacement (`old` -> `new`) to the single minimal
/// splice that turns one into the other, or `None` when they are equal (a no-op
/// the caller skips, matching `edit_notes`' empty-delta guard). The splice is
/// expressed in **unicode codepoints** (not bytes): a common-prefix and a
/// common-suffix are trimmed, and what remains in the middle is the deleted span
/// (`old`) and the inserted text (`new`). This yields one contiguous edit, which
/// is exactly what a typed keystroke or a paste produces; it is not a minimal
/// edit script for arbitrary rewrites, but the panel only ever sees incremental
/// textarea input.
pub fn notes_delta(old: &str, new: &str) -> Option<TextDelta> {
    if old == new {
        return None;
    }
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();

    // Common prefix length (in codepoints).
    let max_prefix = old.len().min(new.len());
    let mut prefix = 0;
    while prefix < max_prefix && old[prefix] == new[prefix] {
        prefix += 1;
    }

    // Common suffix length, not overlapping the shared prefix on either side.
    let mut suffix = 0;
    let max_suffix = max_prefix - prefix;
    while suffix < max_suffix && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix] {
        suffix += 1;
    }

    let delete = old.len() - prefix - suffix;
    let insert: String = new[prefix..new.len() - suffix].iter().collect();
    Some(TextDelta {
        at: prefix,
        delete,
        insert,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_label_is_one_based() {
        assert_eq!(slide_counter_label(0, 10), "1 / 10");
        assert_eq!(slide_counter_label(2, 10), "3 / 10");
        assert_eq!(slide_counter_label(9, 10), "10 / 10");
    }

    #[test]
    fn counter_label_empty_deck() {
        assert_eq!(slide_counter_label(0, 0), "0 / 0");
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
                at: 3,
                delete: 0,
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
                at: 0,
                delete: 0,
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
                at: 2,
                delete: 1,
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
                at: 2,
                delete: 2,
                insert: "Z".to_owned()
            })
        );
    }

    #[test]
    fn delta_from_empty() {
        assert_eq!(
            notes_delta("", "hi"),
            Some(TextDelta {
                at: 0,
                delete: 0,
                insert: "hi".to_owned()
            })
        );
    }

    #[test]
    fn delta_to_empty() {
        assert_eq!(
            notes_delta("hi", ""),
            Some(TextDelta {
                at: 0,
                delete: 2,
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
                at: 2,
                delete: 0,
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
                at: 1,
                delete: 1,
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
                at: 2,
                delete: 1,
                insert: String::new()
            })
        );
    }
}
