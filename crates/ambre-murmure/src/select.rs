//! A text selection across the messages of a list (the Murmure message list
//! spec).
//!
//! The text is read-only, so a selection has two ends and no caret. Each end
//! is a row and a byte of the body that row paints. A row key outlives a load
//! above, where an index does not. The paint resolves a pointer into an end,
//! since only the layout knows which byte lies under it.

use ambre_bus::{Deletion, Timestamp};
use ambre_core::id::{Id, User};
use ambre_core::{ByteCount, ByteOffset, ByteSpan, Point, Px, View};

use crate::list::{ListRow, ListRowVec, MessageList, RowKey, UNKNOWN_MEMBER, row_of};

/// One end of a selection: a row, and a byte of the body it paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    pub row: RowKey,
    pub byte: ByteOffset,
}

/// A selection: the anchor stays where the drag began, and the focus
/// follows the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: Spot,
    pub focus: Spot,
}

/// What a region selects, and the points the next paint resolves.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Selecting {
    /// The selection the last paint resolved.
    pub selection: Option<TextSelection>,
    /// Where a press began, in the region's pixels: the anchor to come.
    pub press: Option<Point<Px<View>>>,
    /// Where the drag is now: the focus to come.
    pub drag: Option<Point<Px<View>>>,
    /// Where a click or a tap that moved nothing landed: the paint looks for
    /// a link under it (`MURMURE-567`).
    pub click: Option<Point<Px<View>>>,
}

impl TextSelection {
    /// The ends in the order of the list, as a row index and a byte each.
    /// `None` once a row of an end left the list.
    fn ordered(
        &self,
        keys: &ListRowVec<RowKey>,
    ) -> Option<((ListRow, ByteOffset), (ListRow, ByteOffset))> {
        let at = |spot: Spot| {
            let row = row_of(keys, spot.row)?;
            Some((row, spot.byte))
        };
        let (anchor, focus) = (at(self.anchor)?, at(self.focus)?);
        Some(if anchor <= focus {
            (anchor, focus)
        } else {
            (focus, anchor)
        })
    }

    /// The bytes it selects of the row `row`, whose body is `len` bytes
    /// long. `None` when it selects none of them, or `row` left the list.
    pub fn range_in(
        &self,
        keys: &ListRowVec<RowKey>,
        row: RowKey,
        len: ByteCount,
    ) -> Option<ByteSpan> {
        let ((first, from), (last, to)) = self.ordered(keys)?;
        let i = row_of(keys, row)?;
        if i < first || i > last {
            return None;
        }
        let end_of_body = len.as_end();
        let start = if i == first {
            from.min(end_of_body)
        } else {
            ByteOffset::ZERO
        };
        let end = if i == last {
            to.min(end_of_body)
        } else {
            end_of_body
        };
        let span = ByteSpan::new(start, end);
        (!span.is_empty()).then_some(span)
    }

    /// The text a copy puts on the clipboard: each message it reaches under
    /// its author and its time (`MURMURE-426`). A band, a notice and a
    /// deleted message hold no text a member wrote, so they add nothing.
    /// `None` when it reaches no such text.
    pub fn copied(
        &self,
        list: &MessageList,
        name_of: &dyn Fn(Id<User>) -> Option<ambre_core::DisplayName>,
        time_of: &dyn Fn(Timestamp) -> String,
    ) -> Option<String> {
        let keys = list.keys();
        let mut parts = Vec::new();
        for (i, row) in list.rows().iter_enumerated() {
            let Some(message) = row.message() else {
                continue;
            };
            if message.deleted == Deletion::Deleted || message.notice.is_some() {
                continue;
            }
            // The body may end on the edited mark, which is no text of its.
            let body = row.words(name_of, time_of).body;
            let text = message.shown(name_of).len();
            let Some(span) = keys
                .get(i)
                .and_then(|&key| self.range_in(&keys, key, ByteCount::from_len(body.len())))
            else {
                continue;
            };
            let (start, end) = (span.start.get(), span.end.get());
            let Some(words) = body.get(start..end.min(text)).filter(|w| !w.is_empty()) else {
                continue;
            };
            let author = name_of(message.author)
                .map(ambre_core::DisplayName::into_string)
                .unwrap_or_else(|| UNKNOWN_MEMBER.to_owned());
            parts.push(format!("{author}, {}\n{words}", time_of(message.ts)));
        }
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::id::Segment;

    use super::*;
    use crate::list::tests::message;

    fn list(bodies: &[&str]) -> MessageList {
        let ada = Id::<User>::mint();
        let mut list = MessageList::default();
        let messages = bodies
            .iter()
            .enumerate()
            .map(|(i, body)| message(ada, ambre_bus::Timestamp::new(i as u64 * 60_000), body))
            .collect();
        list.absorb(Id::<Segment>::mint(), messages);
        list
    }

    fn spot(list: &MessageList, row: u32, byte: usize) -> Spot {
        Spot {
            row: list.keys()[ListRow::new(row)],
            byte: ByteOffset::new(byte),
        }
    }

    fn copy(list: &MessageList, selection: TextSelection) -> Option<String> {
        selection.copied(list, &|_| ambre_core::DisplayName::new("Ada").ok(), &|ts| {
            format!("t{}", ts.get())
        })
    }

    /// A selection reads the same whichever way the drag went, and each
    /// row gives the part of its body between the two ends.
    #[test]
    fn a_selection_reads_the_same_either_way_it_was_dragged() {
        let list = list(&["hello", "brave", "world"]);
        let keys = list.keys();
        let down = TextSelection {
            anchor: spot(&list, 0, 2),
            focus: spot(&list, 2, 3),
        };
        let up = TextSelection {
            anchor: down.focus,
            focus: down.anchor,
        };
        for selection in [down, up] {
            assert_eq!(
                selection.range_in(&keys, keys[ListRow::new(0)], ByteCount::new(5)),
                Some(ByteSpan::from_usize_range(2..5))
            );
            assert_eq!(
                selection.range_in(&keys, keys[ListRow::new(1)], ByteCount::new(5)),
                Some(ByteSpan::from_usize_range(0..5))
            );
            assert_eq!(
                selection.range_in(&keys, keys[ListRow::new(2)], ByteCount::new(5)),
                Some(ByteSpan::from_usize_range(0..3))
            );
        }
        let none = TextSelection {
            anchor: spot(&list, 1, 2),
            focus: spot(&list, 1, 2),
        };
        assert_eq!(
            none.range_in(&keys, keys[ListRow::new(1)], ByteCount::new(5)),
            None,
            "a collapsed one"
        );
        assert_eq!(
            none.range_in(&keys, keys[ListRow::new(0)], ByteCount::new(5)),
            None
        );
    }

    /// `MURMURE-426`: a copy puts each message under its author and its
    /// time, from one character to several messages.
    #[test]
    fn a_copy_puts_each_message_under_its_author_and_time() {
        let list = list(&["hello", "brave", "world"]);
        let one = TextSelection {
            anchor: spot(&list, 0, 1),
            focus: spot(&list, 0, 2),
        };
        assert_eq!(copy(&list, one).as_deref(), Some("Ada, t0\ne"));
        let three = TextSelection {
            anchor: spot(&list, 2, 3),
            focus: spot(&list, 0, 2),
        };
        let copied = copy(&list, three);
        assert_eq!(
            copied.as_deref(),
            Some("Ada, t0\nllo\n\nAda, t60000\nbrave\n\nAda, t120000\nwor")
        );
    }

    /// A deleted message, a notice and the edited mark add no text, and a
    /// selection whose row left the list copies nothing.
    #[test]
    fn a_copy_holds_only_text_a_member_wrote() {
        let mut list = list(&["hello", "", "again"]);
        let mut edited = message(
            Id::<User>::mint(),
            ambre_bus::Timestamp::new(200_000),
            "again",
        );
        edited.edited = ambre_bus::Revision::Edited;
        let mut gone = message(Id::<User>::mint(), ambre_bus::Timestamp::new(100_000), "");
        gone.deleted = ambre_bus::Deletion::Deleted;
        let segment = Id::<Segment>::mint();
        list.absorb(segment, vec![gone, edited]);
        let rows = list.keys().len().get();
        let all = TextSelection {
            anchor: spot(&list, 0, 0),
            focus: spot(&list, rows - 1, 999),
        };
        let copied = copy(&list, all).unwrap_or_default();
        assert!(!copied.contains("deleted"), "{copied}");
        assert!(!copied.contains("edited"), "{copied}");
        assert!(copied.ends_with("\nagain"), "{copied}");
        let stale = TextSelection {
            anchor: Spot {
                row: RowKey::Band(Id::<Segment>::mint()),
                byte: ByteOffset::ZERO,
            },
            focus: all.focus,
        };
        assert_eq!(copy(&list, stale), None);
    }
}
