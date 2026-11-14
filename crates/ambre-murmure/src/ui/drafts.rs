//! The drafts kept on the device (the Murmure composer spec, section 3).
//!
//! A draft is work the user has not sent, so a reload finds it
//! (`MURMURE-210`). The shelf holds the draft of every channel and thread
//! the user wrote in, and hands each change to the local store the launcher
//! owns. The store keeps the rows apart from the documents, so no eviction
//! reaches them (`MURMURE-214`). Nothing here sends a draft anywhere else
//! (`MURMURE-211`).
//!
//! The key of a row starts with the user's identifier. So a second person
//! signed in on the device reads none of the first's.

use ambre_core::Count;
use ambre_core::index::{Pos, Span};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::message::{Mention, MentionTarget, MessageDraft, StyleMark, TextStyle};
use ambre_bus::DraftKey;
use ambre_core::id::{Channel, Id, Message, User};
use ambre_core::{Char, Opaque, OpaqueKind, OpaqueLength, SealedAtRest, Seq, SeqKind};
use serde::{Deserialize, Serialize};

use crate::logic::schema::Mark;
use crate::ui::composer::DraftOf;

/// What the shelf asks the local store to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftWrite {
    /// Keep `row` under `key`, in place of the row there.
    Put {
        key: DraftKey,
        row: Opaque<DraftRows>,
    },
    /// Delete the row under `key`.
    Delete { key: DraftKey },
    /// Delete every row whose key falls in `lower..=upper`: a channel's
    /// draft and its threads' (`MURMURE-215`).
    DeleteRange { lower: DraftKey, upper: DraftKey },
    /// Delete every row of every user (`MURMURE-216`).
    DeleteAll,
}

/// The kind of a draft as a row of the store keeps it: the JSON text the
/// shelf encodes, `Opaque<DraftRows>`. The store seals its bytes, and hands
/// them back to be read. It holds what the user typed, so `Debug` prints the
/// length alone, and it has no serde form: it leaves the page sealed.
pub enum DraftRows {}

impl OpaqueKind for DraftRows {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DraftRow");
    const LENGTH: OpaqueLength = OpaqueLength::Bytes;
}

impl SealedAtRest for DraftRows {}

/// Where the shelf's writes go: the local store on the page, nowhere in a
/// test.
pub type DraftSink = Rc<dyn Fn(DraftWrite)>;

/// The loads and losses a draft shelf went through: what a view repaints for
/// when it moves.
pub enum ShelfVersion {}

impl SeqKind for ShelfVersion {
    type IssueRight = ();
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ShelfVersion");
}

/// How many losses of a draft shelf a reader applied, so its next read
/// starts past them.
pub enum ShelfLosses {}

impl ambre_core::NarrowCount for ShelfLosses {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("LossCount");
}

/// The drafts the device keeps, shared by every region of the page and the
/// page's relay. A clone is the same shelf.
#[derive(Clone, Default)]
pub struct DraftShelf(Rc<RefCell<Shelf>>);

#[derive(Default)]
struct Shelf {
    /// Whose drafts these are, once the session names them.
    user: Option<Id<User>>,
    drafts: HashMap<DraftOf, MessageDraft>,
    /// Drafts written before the user was known, kept once they are.
    unsaved: Vec<DraftOf>,
    /// Every loss so far, in order. A composer drops what it holds of each
    /// one it has not read yet, so it never writes a forgotten draft back.
    losses: Vec<Loss>,
    /// Counts the loads and the losses, which a view repaints for. A draft
    /// put moves it not: the region that typed it already shows it.
    version: Seq<ShelfVersion>,
    sink: Option<DraftSink>,
}

/// Drafts the shelf deleted, which no composer may keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loss {
    /// The drafts of a channel and of its threads (`MURMURE-215`).
    Channel(Id<Channel>),
    /// Every draft: the user signed out (`MURMURE-216`).
    All,
}

impl Shelf {
    fn write(&self, write: DraftWrite) {
        if let Some(sink) = &self.sink {
            sink(write);
        }
    }

    /// Hand the store the draft `of` as the shelf holds it now.
    fn save(&self, user: Id<User>, of: DraftOf) {
        let key = row_key(user, of);
        match self.drafts.get(&of) {
            Some(draft) => self.write(DraftWrite::Put {
                key,
                row: encode(draft),
            }),
            None => self.write(DraftWrite::Delete { key }),
        }
    }
}

impl DraftShelf {
    /// Send every write from now on to `sink`.
    pub fn set_sink(&self, sink: DraftSink) {
        self.0.borrow_mut().sink = Some(sink);
    }

    /// Take the rows the store keeps for `user`, read back after a reload.
    /// A draft typed before they landed keeps its text, and is saved now.
    pub fn load(&self, user: Id<User>, rows: Vec<(DraftKey, Opaque<DraftRows>)>) {
        let mut shelf = self.0.borrow_mut();
        shelf.user = Some(user);
        shelf.version = shelf.version.next();
        for (key, row) in rows {
            if let (Some(of), Some(draft)) = (parse_key(user, &key), decode(&row)) {
                shelf.drafts.entry(of).or_insert(draft);
            }
        }
        for of in std::mem::take(&mut shelf.unsaved) {
            shelf.save(user, of);
        }
    }

    /// The draft the device keeps `of` a channel or a thread.
    pub fn get(&self, of: DraftOf) -> Option<MessageDraft> {
        self.0.borrow().drafts.get(&of).cloned()
    }

    /// Keep `draft` as the draft `of` a channel or a thread, or delete it
    /// with `None`. A draft the shelf already holds writes nothing.
    pub fn put(&self, of: DraftOf, draft: Option<MessageDraft>) {
        let mut shelf = self.0.borrow_mut();
        if shelf.drafts.get(&of) == draft.as_ref() {
            return;
        }
        match draft {
            Some(draft) => shelf.drafts.insert(of, draft),
            None => shelf.drafts.remove(&of),
        };
        match shelf.user {
            Some(user) => shelf.save(user, of),
            None => shelf.unsaved.push(of),
        }
    }

    /// Delete the drafts of `channel` and of its threads: the user lost it
    /// (`MURMURE-215`). Another channel's stay (`MURMURE-217`).
    pub fn forget_channel(&self, channel: Id<Channel>) {
        let mut shelf = self.0.borrow_mut();
        shelf.drafts.retain(|of, _| of.channel() != channel);
        shelf.unsaved.retain(|of| of.channel() != channel);
        shelf.losses.push(Loss::Channel(channel));
        shelf.version = shelf.version.next();
        if let Some(user) = shelf.user {
            let lower = row_key(user, DraftOf::Channel(channel));
            let upper = DraftKey::new(format!("{lower}{SEPARATOR}\u{10FFFF}"));
            shelf.write(DraftWrite::DeleteRange { lower, upper });
        }
    }

    /// Delete every draft on the device: the user signed out, and the next
    /// person finds none (`MURMURE-216`).
    pub fn sign_out(&self) {
        let mut shelf = self.0.borrow_mut();
        shelf.user = None;
        shelf.drafts.clear();
        shelf.unsaved.clear();
        shelf.losses.push(Loss::All);
        shelf.version = shelf.version.next();
        shelf.write(DraftWrite::DeleteAll);
    }

    /// Moves at each load and each loss.
    pub fn version(&self) -> Seq<ShelfVersion> {
        self.0.borrow().version
    }

    /// The losses after the first `seen`, and how many there are in all.
    pub fn losses_since(&self, seen: Count<ShelfLosses>) -> (Vec<Loss>, Count<ShelfLosses>) {
        let shelf = self.0.borrow();
        let losses = usize::try_from(seen.get())
            .ok()
            .and_then(|seen| shelf.losses.get(seen..))
            .unwrap_or_default()
            .to_vec();
        (losses, Count::<ShelfLosses>::of(shelf.losses.len()))
    }
}

/// What parts a row key: the user, the channel, the root of a thread.
const SEPARATOR: char = '/';

/// The first key of `user`'s rows, and the last: every row of theirs falls
/// between the two.
pub fn user_range(user: Id<User>) -> (DraftKey, DraftKey) {
    (
        DraftKey::new(format!("{user}{SEPARATOR}")),
        DraftKey::new(format!("{user}{SEPARATOR}\u{10FFFF}")),
    )
}

/// The key of the row that keeps the draft `of`, among `user`'s.
fn row_key(user: Id<User>, of: DraftOf) -> DraftKey {
    DraftKey::new(match of {
        DraftOf::Channel(channel) => format!("{user}{SEPARATOR}{channel}"),
        DraftOf::Thread(channel, root) => {
            format!("{user}{SEPARATOR}{channel}{SEPARATOR}{root}")
        }
    })
}

/// The draft a row key of `user` names, `None` for another user's key or one
/// that does not parse.
fn parse_key(user: Id<User>, key: &DraftKey) -> Option<DraftOf> {
    let mut parts = key.as_str().split(SEPARATOR);
    if Id::<User>::parse_str(parts.next()?).ok()? != user {
        return None;
    }
    let channel = Id::<Channel>::parse_str(parts.next()?).ok()?;
    let of = match parts.next() {
        None => DraftOf::Channel(channel),
        Some(root) => DraftOf::Thread(channel, Id::<Message>::parse_str(root).ok()?),
    };
    parts.next().is_none().then_some(of)
}

/// A draft as a row keeps it.
#[derive(Serialize, Deserialize)]
struct Row {
    text: String,
    mentions: Vec<RowMention>,
    styles: Vec<RowStyle>,
}

/// A mention of a row: a member, or the channel with no member.
#[derive(Serialize, Deserialize)]
struct RowMention {
    user: Option<Id<User>>,
    start: Pos<Char>,
    end: Pos<Char>,
}

/// A style of a row, by its mark name, with the address of a link. A name
/// this build does not know reads as `None`.
#[derive(Serialize, Deserialize)]
struct RowStyle {
    #[serde(with = "mark_name")]
    style: Option<Mark>,
    link: Option<ambre_core::LinkUrl>,
    start: Pos<Char>,
    end: Pos<Char>,
}

/// A style name kept as the word of its mark. An older or a newer build may
/// write a word this one does not know. That word must not refuse the row.
mod mark_name {
    use ambre_core::crdt::CrdtName;
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::logic::schema::Mark;

    pub(super) fn serialize<S: Serializer>(mark: &Option<Mark>, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_str(mark.map_or("", Mark::as_str))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        input: D,
    ) -> Result<Option<Mark>, D::Error> {
        let word = std::borrow::Cow::<str>::deserialize(input)?;
        Ok(Mark::parse(&word))
    }
}

/// The row that keeps `draft`.
fn encode(draft: &MessageDraft) -> Opaque<DraftRows> {
    let mentions = draft
        .mentions
        .iter()
        .map(|mention| RowMention {
            user: match mention.target {
                MentionTarget::User(user) => Some(user),
                MentionTarget::Channel => None,
            },
            start: mention.range.start,
            end: mention.range.end,
        })
        .collect();
    let styles = draft
        .styles
        .iter()
        .map(|mark| {
            let (style, link) = match &mark.style {
                TextStyle::Bold => (Mark::Bold, None),
                TextStyle::Italic => (Mark::Italic, None),
                TextStyle::Code => (Mark::Code, None),
                TextStyle::Link(address) => (Mark::Link, Some(address.clone())),
                TextStyle::CodeBlock => (Mark::CodeBlock, None),
                TextStyle::Quote => (Mark::Quote, None),
                TextStyle::List => (Mark::List, None),
            };
            RowStyle {
                style: Some(style),
                link,
                start: mark.range.start,
                end: mark.range.end,
            }
        })
        .collect();
    let row = Row {
        text: draft.text.clone(),
        mentions,
        styles,
    };
    Opaque::<DraftRows>::new(serde_json::to_string(&row).unwrap_or_default())
}

/// The draft `row` keeps, `None` for a row that does not parse. A style of a
/// name this build does not know is dropped, and the text stays.
fn decode(row: &Opaque<DraftRows>) -> Option<MessageDraft> {
    // The slice reader, not the `str` one. Main already links it for the
    // JSON update of a delta, so a second reader adds code and no check.
    let row: Row = serde_json::from_slice(row.as_str().as_bytes()).ok()?;
    let mentions = row
        .mentions
        .into_iter()
        .map(|mention| Mention {
            target: mention
                .user
                .map_or(MentionTarget::Channel, MentionTarget::User),
            range: Span::<Char>::new(mention.start, mention.end),
        })
        .collect();
    let styles = row
        .styles
        .into_iter()
        .filter_map(|mark| {
            let style = match (mark.style?, mark.link) {
                (Mark::Bold, _) => TextStyle::Bold,
                (Mark::Italic, _) => TextStyle::Italic,
                (Mark::Code, _) => TextStyle::Code,
                (Mark::Link, Some(address)) if crate::logic::style::linkable(&address) => {
                    TextStyle::Link(address)
                }
                (Mark::CodeBlock, _) => TextStyle::CodeBlock,
                (Mark::Quote, _) => TextStyle::Quote,
                (Mark::List, _) => TextStyle::List,
                (Mark::Link | Mark::Mention, _) => return None,
            };
            Some(StyleMark {
                style,
                range: Span::<Char>::new(mark.start, mark.end),
            })
        })
        .collect();
    Some(MessageDraft {
        text: row.text,
        mentions,
        styles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shelf whose writes a test reads back.
    fn recorded() -> (DraftShelf, Rc<RefCell<Vec<DraftWrite>>>) {
        let shelf = DraftShelf::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let writes = Rc::clone(&log);
        shelf.set_sink(Rc::new(move |write| writes.borrow_mut().push(write)));
        (shelf, log)
    }

    fn draft(text: &str) -> MessageDraft {
        MessageDraft {
            text: text.to_owned(),
            mentions: Vec::new(),
            styles: Vec::new(),
        }
    }

    /// A draft goes through its row whole: its text, a member's mention and
    /// the channel's, and every style. A row that does not parse is no
    /// draft (`MURMURE-210`).
    #[test]
    fn a_row_keeps_the_draft_whole() {
        let ada = Id::<User>::mint();
        let full = MessageDraft {
            text: "hi @Ada @channel see".to_owned(),
            mentions: vec![
                Mention {
                    target: MentionTarget::User(ada),
                    range: Span::<Char>::from_usize_range(3..7),
                },
                Mention {
                    target: MentionTarget::Channel,
                    range: Span::<Char>::from_usize_range(8..16),
                },
            ],
            styles: [
                TextStyle::Bold,
                TextStyle::Italic,
                TextStyle::Code,
                TextStyle::Link("https://a.example".into()),
                TextStyle::CodeBlock,
                TextStyle::Quote,
                TextStyle::List,
            ]
            .into_iter()
            .map(|style| StyleMark {
                style,
                range: Span::<Char>::from_usize_range(0..2),
            })
            .collect(),
        };
        assert_eq!(decode(&encode(&full)), Some(full));
        assert_eq!(decode(&Opaque::<DraftRows>::new("not a row")), None);
    }

    /// The bytes of an opened row are its text, and bytes that are no text
    /// are no row.
    #[test]
    fn an_opened_row_is_its_text_and_other_bytes_are_none() {
        let row = encode(&draft("hi"));
        assert_eq!(
            Opaque::<DraftRows>::from_bytes(row.as_str().as_bytes().to_vec()),
            Some(row)
        );
        assert_eq!(Opaque::<DraftRows>::from_bytes(vec![0xff, 0xfe]), None);
    }

    /// A row's `Debug` form holds no byte of what the user typed, so a log
    /// carries none of it.
    #[test]
    fn a_row_prints_its_length_and_none_of_its_text() {
        let row = encode(&draft("a secret plan"));
        let len = row.as_str().len();
        let printed = format!("{row:?}");
        assert_eq!(printed, format!("DraftRow({len} bytes)"));
        assert!(!printed.contains("secret"));
        let key = DraftKey::new("k".into());
        let put = format!("{:?}", DraftWrite::Put { key, row });
        assert!(!put.contains("secret"));
    }

    /// A row read back from a reload is the draft of its channel or thread,
    /// for its user alone. A draft typed before the rows landed wins, and
    /// is saved once the user is known (`MURMURE-210`).
    #[test]
    fn a_reload_reads_back_the_users_rows() {
        let (me, other) = (Id::<User>::mint(), Id::<User>::mint());
        let (channel, root) = (Id::<Channel>::mint(), Id::<Message>::mint());
        let (typed, thread) = (
            DraftOf::Channel(Id::<Channel>::mint()),
            DraftOf::Thread(channel, root),
        );
        let (shelf, log) = recorded();
        shelf.put(typed, Some(draft("typed")));
        assert!(log.borrow().is_empty(), "no user, no row");
        let rows = vec![
            (
                row_key(me, DraftOf::Channel(channel)),
                encode(&draft("one")),
            ),
            (row_key(me, thread), encode(&draft("two"))),
            (
                row_key(other, DraftOf::Channel(channel)),
                encode(&draft("theirs")),
            ),
            (row_key(me, typed), encode(&draft("older"))),
            (DraftKey::new("junk".to_owned()), encode(&draft("junk"))),
        ];
        shelf.load(me, rows);
        assert_eq!(shelf.get(DraftOf::Channel(channel)), Some(draft("one")));
        assert_eq!(shelf.get(thread), Some(draft("two")));
        assert_eq!(
            shelf.get(typed),
            Some(draft("typed")),
            "the typed draft won"
        );
        let saved = DraftWrite::Put {
            key: row_key(me, typed),
            row: encode(&draft("typed")),
        };
        assert_eq!(*log.borrow(), [saved]);
    }

    /// A change writes its row, an unchanged draft writes nothing, and a
    /// draft taken out deletes its row: a sent message's is gone
    /// (`MURMURE-212`).
    #[test]
    fn a_change_writes_its_row_and_a_send_deletes_it() {
        let me = Id::<User>::mint();
        let of = DraftOf::Channel(Id::<Channel>::mint());
        let (shelf, log) = recorded();
        shelf.load(me, Vec::new());
        shelf.put(of, Some(draft("hi")));
        shelf.put(of, Some(draft("hi")));
        shelf.put(of, None);
        shelf.put(of, None);
        let key = row_key(me, of);
        let expected = [
            DraftWrite::Put {
                key: key.clone(),
                row: encode(&draft("hi")),
            },
            DraftWrite::Delete { key },
        ];
        assert_eq!(*log.borrow(), expected);
        assert_eq!(shelf.get(of), None);
    }

    /// A lost channel takes its draft and its threads' with it, and its
    /// range covers no other channel's row (`MURMURE-217`).
    #[test]
    fn a_lost_channel_takes_its_drafts_and_no_other() {
        let me = Id::<User>::mint();
        let (lost, kept) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let (shelf, log) = recorded();
        shelf.load(me, Vec::new());
        for of in [
            DraftOf::Channel(lost),
            DraftOf::Thread(lost, Id::<Message>::mint()),
            DraftOf::Channel(kept),
            DraftOf::Thread(kept, Id::<Message>::mint()),
        ] {
            shelf.put(of, Some(draft("x")));
        }
        log.borrow_mut().clear();
        shelf.forget_channel(lost);
        let Some(DraftWrite::DeleteRange { lower, upper }) = log.borrow().first().cloned() else {
            panic!("no range deleted");
        };
        let covered = |key: &DraftKey| (&lower..=&upper).contains(&key);
        assert!(covered(&row_key(me, DraftOf::Channel(lost))));
        assert!(covered(&row_key(
            me,
            DraftOf::Thread(lost, Id::<Message>::mint())
        )));
        assert!(!covered(&row_key(me, DraftOf::Channel(kept))));
        assert!(!covered(&row_key(
            me,
            DraftOf::Thread(kept, Id::<Message>::mint())
        )));
        assert_eq!(shelf.get(DraftOf::Channel(lost)), None);
        assert_eq!(shelf.get(DraftOf::Channel(kept)), Some(draft("x")));
    }

    /// A sign-out deletes every row on the device, and the shelf holds no
    /// draft after it (`MURMURE-216`).
    #[test]
    fn a_sign_out_deletes_every_draft() {
        let of = DraftOf::Channel(Id::<Channel>::mint());
        let (shelf, log) = recorded();
        shelf.load(Id::<User>::mint(), Vec::new());
        shelf.put(of, Some(draft("x")));
        shelf.sign_out();
        assert_eq!(log.borrow().last(), Some(&DraftWrite::DeleteAll));
        assert_eq!(shelf.get(of), None);
    }

    /// Every key of a user falls in that user's range, and no other user's
    /// does.
    #[test]
    fn a_users_range_holds_their_keys_alone() {
        let (me, other) = (Id::<User>::mint(), Id::<User>::mint());
        let (lower, upper) = user_range(me);
        let covered = |key: &DraftKey| (&lower..=&upper).contains(&key);
        let of = DraftOf::Thread(Id::<Channel>::mint(), Id::<Message>::mint());
        assert!(covered(&row_key(me, of)));
        assert!(!covered(&row_key(other, of)));
    }
}
