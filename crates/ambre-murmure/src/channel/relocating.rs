//! A new message the relay refused for a locked segment
//! (`features/ambre-spec-murmure-segments.md section 3`).
//!
//! The relay locks a closing segment once a member holds its finalize lease.
//! It refuses every later delta of that segment (`SRV-115`). The deltas it
//! refused reached no other member. So the segment goes back to the state the
//! relay holds, and each new message they carried goes to the active segment.
//! It keeps its identifier, so the view moves it and never shows it twice.
//! An edit or a delete they carried of an older message is replayed as an
//! amendment, so nothing the author wrote is lost.

use std::collections::HashMap;

use crate::message::{Deletion, Mention, MessageDraft, MessageView, MurmureEvent, Notice};
use ambre_core::id::{Channel, Document, Id, Message, Segment};
use loro::{LoroDoc, LoroMap, LoroText, VersionVector};

use super::{ChannelState, Sent, read_messages, summary_of};
use crate::delta;
use crate::error::MurmureError;
use crate::schema::{self, MessageKey, NoticeKind, Root};
use crate::{mention, reaction, style};

/// What a relocation produced: the send of the moved messages, and the
/// changes of older messages to write again as amendments.
#[derive(Debug)]
pub struct Relocated {
    /// The moved messages, written in the active segment. `None` when the
    /// refused deltas created no message.
    pub sent: Option<Sent>,
    /// One per older message the refused deltas edited or deleted, in segment
    /// order (`MURMURE-073`).
    pub replay: Vec<Replay>,
}

/// An edit or a delete the lock refused, to write again as an amendment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    /// The locked segment that holds the message.
    pub segment: Id<Segment>,
    /// The message the author changed.
    pub message: Id<Message>,
    /// The body the author wrote, `None` for a delete.
    pub draft: Option<MessageDraft>,
}

impl ChannelState {
    /// The relay refused `refused` for the lock of the segment `document_id`.
    /// Each is a JSON update this client sent, in the order they left
    /// (`MURMURE-263`).
    ///
    /// The segment drops every operation of this client from the first one
    /// refused. Each message they created is written again in the active
    /// segment, with its identifier, its stamp, its body and its reactions.
    /// Each edit or delete of an older message comes back as a [`Replay`].
    /// The segment counts as locked from now on, so every later change of it
    /// goes to its amendments.
    ///
    /// # Errors
    /// - [`MurmureError::NotHeld`]: the channel holds no such segment.
    /// - [`MurmureError::NotActive`]: `document_id` is the active segment,
    ///   which no lock refuses.
    /// - [`MurmureError::NoSession`]: the PeerID of the active segment has
    ///   not landed. Nothing changed.
    /// - [`MurmureError::Refused`]: a refused delta reads as no JSON update.
    ///   Nothing changed.
    /// - [`MurmureError::Crdt`]: Loro refused the fork or a write.
    pub fn relocate_locked(
        &mut self,
        document_id: Id<Document>,
        refused: &[ambre_core::Payload<ambre_core::DeltaBytes>],
    ) -> Result<Relocated, MurmureError> {
        if document_id == self.active.id {
            return Err(MurmureError::NotActive(document_id));
        }
        // Only a fold seals amendments, and it erases them next. `meta` then
        // marks them folded, and their settle replays what the seal refused.
        if self.is_amendments(document_id) {
            return Ok(Relocated {
                sent: None,
                replay: Vec::new(),
            });
        }
        if self.active.session.is_none() {
            return Err(MurmureError::NoSession(self.active.id));
        }
        let mut from = VersionVector::new();
        for payload in refused {
            let (start, _) = delta::range(&delta::read(payload)?);
            for (&peer, &counter) in start.iter() {
                if from.get(&peer).is_none_or(|c| *c > counter) {
                    from.insert(peer, counter);
                }
            }
        }
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let mut kept = doc.oplog_vv();
        for (&peer, &counter) in from.iter() {
            if kept.get(&peer).is_some_and(|c| *c > counter) {
                kept.insert(peer, counter);
            }
        }
        let rewound = doc.fork_at(&doc.vv_to_frontiers(&kept))?;
        let channel = self.channel_id;
        let locked = Id::<Segment>::from_document(document_id);
        let stays: HashMap<Id<Message>, MessageView> = read_messages(&rewound, channel)
            .into_iter()
            .map(|m| (m.id, m))
            .collect();
        let (moved, kept_views): (Vec<MessageView>, Vec<MessageView>) = read_messages(doc, channel)
            .into_iter()
            .partition(|m| !stays.contains_key(&m.id));
        let replay = kept_views
            .into_iter()
            .filter_map(|now| replay_of(locked, stays.get(&now.id)?, now))
            .collect();
        self.rewind(document_id, rewound, &kept)?;
        self.locked.insert(locked);
        if moved.is_empty() {
            return Ok(Relocated { sent: None, replay });
        }
        let segment = self.active_id();
        let (doc, session) = self.active.session()?;
        for view in &moved {
            rewrite(doc, channel, view)?;
        }
        let delta = session.export_json(doc)?;
        let marked: Vec<Mention> = moved.iter().flat_map(|m| m.mentions.clone()).collect();
        let sent = Sent {
            segment,
            document: segment.document_id(),
            delta,
            summary: summary_of(&marked),
            event: MurmureEvent::Messages {
                segment,
                messages: moved,
            },
        };
        Ok(Relocated {
            sent: Some(sent),
            replay,
        })
    }

    /// Hold `rewound` in place of the segment `document_id`, at the version
    /// `kept`. A closing segment writes on under its own PeerID, from the
    /// first counter the relay refused.
    fn rewind(
        &mut self,
        document_id: Id<Document>,
        rewound: LoroDoc,
        kept: &VersionVector,
    ) -> Result<(), MurmureError> {
        if let Some(held) = self.closing.as_mut().filter(|held| held.id == document_id) {
            if let Some(session) = held.session.as_mut() {
                rewound.set_peer_id(session.peer().get())?;
                session.rewind(kept);
            }
            held.doc = rewound;
            return Ok(());
        }
        let segment = Id::<Segment>::from_document(document_id);
        if let Some(doc) = self.loaded.get_mut(&segment) {
            *doc = rewound;
        }
        Ok(())
    }
}

/// The change the refused deltas made to a message the relay holds, `None`
/// when they changed neither its body nor its deletion.
pub(super) fn replay_of(
    segment: Id<Segment>,
    before: &MessageView,
    now: MessageView,
) -> Option<Replay> {
    let draft = if now.deleted == Deletion::Deleted {
        if before.deleted == Deletion::Deleted {
            return None;
        }
        None
    } else {
        if now.text == before.text && now.mentions == before.mentions && now.styles == before.styles
        {
            return None;
        }
        Some(MessageDraft {
            text: now.text,
            mentions: now.mentions,
            styles: now.styles,
        })
    };
    Some(Replay {
        segment,
        message: now.id,
        draft,
    })
}

/// Write `view` as a new message at the end of `doc`, as its author sent it.
/// Only the author's own reactions were refused with it, so only theirs are
/// written.
fn rewrite(doc: &LoroDoc, channel: Id<Channel>, view: &MessageView) -> Result<(), MurmureError> {
    let message = crate::schema::Fields::new(
        crate::schema::root_list(doc, Root::Messages).push_container(LoroMap::new())?,
    );
    message.insert(MessageKey::Id, view.id.to_string())?;
    message.insert(MessageKey::Author, view.author.to_string())?;
    message.insert(MessageKey::Ts, schema::ts_value(view.ts))?;
    let content = message.insert_container(MessageKey::Content, LoroText::new())?;
    if !view.text.is_empty() {
        content.insert(0, &view.text)?;
        mention::write(doc, &content, &view.mentions, channel)?;
        style::write(doc, &content, &view.styles)?;
    }
    let reactions = message.insert_container(MessageKey::Reactions, LoroMap::new())?;
    for kind in &view.reactions {
        for &user in kind.users.iter().filter(|&&user| user == view.author) {
            reactions.insert(
                reaction::ReactionKey::of(user, &kind.emoji).as_str(),
                schema::ts_value(view.ts),
            )?;
        }
    }
    if let Some(Notice::MemberAdded { member }) = view.notice {
        message.insert(MessageKey::Notice, NoticeKind::MemberAdded.as_str())?;
        message.insert(MessageKey::Member, member.to_string())?;
    }
    Ok(())
}
