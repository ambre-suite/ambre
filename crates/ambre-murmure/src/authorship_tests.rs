//! The authorship check of a segment delta against its contract rows
//! (`features/ambre-spec-murmure-messages.md section 3`).
//!
//! Every member here writes under a registry entry that names its user. So a
//! later session of one author resolves to that user, as the relay serves it.

use crate::Receipt;
use ambre_core::Version;
use std::collections::HashMap;

use ambre_bus::{MessageDraft, MurmureEvent, Timestamp};
use ambre_core::authorship::PeerEntry;
use ambre_core::id::{Channel, Document, Id, User};
use ambre_core::{DeltaBytes, FixedBytes, Payload, PeerEntryBytes, Seq};
use loro::{Container, ExportMode, LoroDoc, LoroMap, LoroValue, ValueOrContainer, VersionVector};

use crate::schema::{MessageKey, Root};
use crate::{
    ChannelState, ChannelType, JSON_SCHEMA_VERSION, JsonUpdate, MurmureError, Opening,
    OpeningChannel, Refusal, Signer,
};

const NOW: Timestamp = Timestamp::new(1_700_000_000_000);

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

/// A registry entry naming `user`. The signature is Sync's to check, so the
/// engine reads only the user.
fn entry(user: Id<User>) -> PeerEntryBytes {
    PeerEntryBytes::from(&PeerEntry {
        user,
        nonce: FixedBytes::from_array([7; 16]),
        signature: FixedBytes::from_array([9; 64]),
    })
}

/// One member of a channel, with the user its sessions write for.
struct Member {
    state: ChannelState,
    user: Id<User>,
}

impl Member {
    /// Take a fresh PeerID on every live document, as a new session does.
    fn new_session(&mut self) {
        for id in [
            self.state.meta_id().document_id(),
            self.state.active_id().document_id(),
        ] {
            let peer = crate::session::next_peer();
            self.state
                .adopt_session(id, peer, entry(self.user))
                .expect("adopt");
        }
    }

    /// The active segment of the channel.
    fn segment(&self) -> Id<Document> {
        self.state.active_id().document_id()
    }

    fn doc(&self) -> &LoroDoc {
        self.state.doc(self.segment()).expect("segment")
    }

    /// Who signs what this member writes in the active segment.
    fn signer(&self) -> Signer {
        Signer {
            peer: self.state.peer(self.segment()).expect("a session"),
            user: Some(self.user),
        }
    }

    /// Every operation written since `before`, as a segment delta travels.
    fn json_since(&self, before: &VersionVector) -> Payload<DeltaBytes> {
        let doc = self.doc();
        doc.commit();
        crate::delta::encode(
            &doc.export_json_updates_without_peer_compression(before, &doc.oplog_vv()),
        )
        .expect("encode")
    }

    /// The first message of the active segment, to write into it.
    fn first_message(&self) -> crate::schema::Fields<MessageKey> {
        let Some(message) = self
            .doc()
            .get_list(Root::Messages.as_str())
            .get(0)
            .and_then(crate::schema::fields_of)
        else {
            panic!("a message is a map container");
        };
        message
    }

    /// Every document of the channel, as it reads now.
    fn everything(&self) -> Vec<LoroValue> {
        self.state
            .document_ids()
            .into_iter()
            .filter_map(|id| self.state.doc(id).map(LoroDoc::get_deep_value))
            .collect()
    }
}

/// A channel just created by a member of its own.
fn founder(channel_type: ChannelType) -> Member {
    let mut founder = Member {
        state: ChannelState::create(Id::<Channel>::mint(), channel_type, None, NOW)
            .expect("create"),
        user: Id::<User>::mint(),
    };
    founder.new_session();
    founder
}

/// A second member, opened from the documents `of` holds now.
fn join(of: &Member) -> Member {
    let state = &of.state;
    let segments = HashMap::from([(
        state.active_id(),
        state.snapshot(of.segment()).expect("segment"),
    )]);
    let meta = state.snapshot(state.meta_id().document_id()).expect("meta");
    let mut member = Member {
        state: ChannelState::open(state.channel_id(), state.meta_id(), &meta, &segments)
            .expect("open"),
        user: Id::<User>::mint(),
    };
    member.new_session();
    member
}

fn refusal(result: Result<Vec<MurmureEvent>, MurmureError>) -> Option<Refusal> {
    match result {
        Err(MurmureError::Refused(refusal)) => Some(refusal),
        _ => None,
    }
}

// --- MURMURE-223, MURMURE-227 ---

#[test]
fn a_segment_delta_travels_as_a_json_update_of_the_pinned_version() {
    let mut alice = founder(ChannelType::Public);
    let sent = alice
        .state
        .send(&draft("hello"), alice.user, NOW)
        .expect("send");
    let update: serde_json::Value =
        serde_json::from_slice(sent.delta.as_bytes()).expect("a JSON update");
    assert_eq!(update["schema_version"], JSON_SCHEMA_VERSION.get());
    // The peers stay uncompressed, so each operation names its PeerID.
    assert!(update["peers"].is_null());

    // The first update of a segment created here is a delta like any other.
    let mut created = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create");
    let active = created.active_id().document_id();
    let first = created
        .adopt_session(
            active,
            crate::session::next_peer(),
            entry(Id::<User>::mint()),
        )
        .expect("adopt")
        .expect("the first update");
    let first: serde_json::Value = serde_json::from_slice(first.as_bytes()).expect("a JSON update");
    assert_eq!(first["schema_version"], JSON_SCHEMA_VERSION.get());
}

// --- MURMURE-224 ---

#[test]
fn a_binary_delta_on_a_segment_is_refused() {
    let mut alice = founder(ChannelType::Public);
    let mut bob = join(&alice);
    let before = alice.doc().oplog_vv();
    alice
        .state
        .send(&draft("binary"), alice.user, NOW)
        .expect("send");
    let binary = Payload::<DeltaBytes>::new(
        alice
            .doc()
            .export(ExportMode::updates(&before))
            .expect("export"),
    );
    let was = bob.everything();
    let segment = bob.segment();
    assert_eq!(
        refusal(
            bob.state
                .apply_signed(segment, Seq::new(1), &binary, alice.signer())
        ),
        Some(Refusal::Binary)
    );
    assert_eq!(bob.everything(), was);
    // A refused delta still counts as delivered.
    assert_eq!(bob.state.delivered(segment), Seq::new(1));
}

// --- MURMURE-228 ---

#[test]
fn a_json_update_of_another_version_is_refused_and_asks_an_update() {
    let mut alice = founder(ChannelType::Public);
    let mut bob = join(&alice);
    let sent = alice
        .state
        .send(&draft("from a newer build"), alice.user, NOW)
        .expect("send");
    let mut update: serde_json::Value =
        serde_json::from_slice(sent.delta.as_bytes()).expect("json");
    update["schema_version"] = serde_json::Value::from(JSON_SCHEMA_VERSION.get() + 1);
    let newer = Payload::<DeltaBytes>::new(serde_json::to_vec(&update).expect("bytes"));
    let was = bob.everything();
    let segment = bob.segment();
    let refused = refusal(
        bob.state
            .apply_signed(segment, Seq::new(1), &newer, alice.signer()),
    )
    .expect("refused");
    assert_eq!(
        refused,
        Refusal::Version(Version::<JsonUpdate>::new(JSON_SCHEMA_VERSION.get() + 1))
    );
    assert!(refused.asks_update());
    assert!(!Refusal::ForeignAuthor.asks_update());
    assert_eq!(bob.everything(), was);
}

// --- MURMURE-078, MURMURE-082 ---

#[test]
fn a_message_carries_the_author_who_signed_it_and_no_other() {
    let mut alice = founder(ChannelType::Public);
    let mut bob = join(&alice);
    let carol = Id::<User>::mint();
    let segment = bob.segment();
    // Written first, so no refused write below rides in its delta.
    let own = alice
        .state
        .send(&draft("mine"), alice.user, NOW)
        .expect("send");

    let forged = alice
        .state
        .send(&draft("as carol"), carol, NOW)
        .expect("send");
    let was = bob.everything();
    assert_eq!(
        refusal(
            bob.state
                .apply_signed(segment, Seq::new(1), &forged.delta, alice.signer())
        ),
        Some(Refusal::ForeignAuthor)
    );
    assert_eq!(bob.everything(), was);

    // A message with no author names nobody, so it is no signer's either.
    let before = alice.doc().oplog_vv();
    let bare = crate::schema::Fields::<MessageKey>::of(
        alice
            .doc()
            .get_list(Root::Messages.as_str())
            .push_container(LoroMap::new())
            .expect("push"),
    );
    bare.insert(MessageKey::Id, "no author").expect("insert");
    let anonymous = alice.json_since(&before);
    assert_eq!(
        refusal(
            bob.state
                .apply_signed(segment, Seq::new(2), &anonymous, alice.signer())
        ),
        Some(Refusal::ForeignAuthor)
    );

    // A signer the directory does not name attributes nothing.
    let unnamed = Signer {
        user: None,
        ..alice.signer()
    };
    assert_eq!(
        refusal(
            bob.state
                .apply_signed(segment, Seq::new(3), &own.delta, unnamed)
        ),
        Some(Refusal::Unattributed)
    );
    // Nor does an operation under another PeerID than the signer's.
    let elsewhere = Signer {
        peer: ambre_core::PeerId::new(alice.signer().peer.get() + 1),
        ..alice.signer()
    };
    assert_eq!(
        refusal(
            bob.state
                .apply_signed(segment, Seq::new(4), &own.delta, elsewhere)
        ),
        Some(Refusal::NotSigners)
    );
    assert_eq!(bob.everything(), was);

    bob.state
        .apply_signed(segment, Seq::new(5), &own.delta, alice.signer())
        .expect("accepted");
    let messages = bob.state.messages(bob.state.active_id()).expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].author, alice.user);
}

// --- MURMURE-085, MURMURE-030 ---

#[test]
fn an_edit_of_another_authors_message_is_refused_and_changes_nothing() {
    let mut alice = founder(ChannelType::Public);
    let mut bob = join(&alice);
    let sent = alice
        .state
        .send(&draft("alice wrote this"), alice.user, NOW)
        .expect("send");
    let segment = bob.segment();
    bob.state
        .apply_signed(segment, Seq::new(1), &sent.delta, alice.signer())
        .expect("accepted");

    let before = bob.doc().oplog_vv();
    let Some(ValueOrContainer::Container(Container::Text(content))) =
        bob.first_message().get(MessageKey::Content)
    else {
        panic!("the content is a text");
    };
    content.insert(0, "bob: ").expect("type");
    let edit = bob.json_since(&before);

    let was = alice.everything();
    assert_eq!(
        refusal(
            alice
                .state
                .apply_signed(segment, Seq::new(2), &edit, bob.signer())
        ),
        Some(Refusal::ForeignMessage)
    );
    assert_eq!(alice.everything(), was);
    assert_eq!(
        alice.state.messages(alice.state.active_id()).expect("read")[0].text,
        "alice wrote this"
    );
}

// --- MURMURE-089 ---

#[test]
fn a_delete_of_another_authors_message_is_refused() {
    let mut alice = founder(ChannelType::Public);
    let mut bob = join(&alice);
    let sent = alice
        .state
        .send(&draft("keep me"), alice.user, NOW)
        .expect("send");
    let segment = bob.segment();
    bob.state
        .apply_signed(segment, Seq::new(1), &sent.delta, alice.signer())
        .expect("accepted");
    let was = alice.everything();

    // A delete marks the message: a write into a map bob did not create.
    let before = bob.doc().oplog_vv();
    bob.first_message()
        .insert(MessageKey::Deleted, true)
        .expect("mark");
    let marked = bob.json_since(&before);
    assert_eq!(
        refusal(
            alice
                .state
                .apply_signed(segment, Seq::new(2), &marked, bob.signer())
        ),
        Some(Refusal::ForeignMessage)
    );

    // Taking it out of the list is no delete at all.
    let before = bob.doc().oplog_vv();
    bob.doc()
        .get_list(Root::Messages.as_str())
        .delete(0, 1)
        .expect("remove");
    let removed = bob.json_since(&before);
    assert_eq!(
        refusal(
            alice
                .state
                .apply_signed(segment, Seq::new(3), &removed, bob.signer())
        ),
        Some(Refusal::Removed)
    );
    assert_eq!(alice.everything(), was);
}

// --- MURMURE-225 ---

#[test]
fn a_later_session_of_the_author_edits_its_own_message() {
    let mut alice = founder(ChannelType::Public);
    alice
        .state
        .send(&draft("draft"), alice.user, NOW)
        .expect("send");
    let mut carol = join(&alice);
    let segment = carol.segment();

    // A reload: the same user under a new PeerID, its entry riding along.
    alice.new_session();
    let before = carol.doc().oplog_vv();
    let Some(ValueOrContainer::Container(Container::Text(content))) =
        alice.first_message().get(MessageKey::Content)
    else {
        panic!("the content is a text");
    };
    content.insert(5, " final").expect("type");
    let edit = alice.json_since(&before);
    carol
        .state
        .apply_signed(segment, Seq::new(2), &edit, alice.signer())
        .expect("accepted");
    // What the check read is what landed: every operation of the session,
    // and the messages the author holds.
    let peer = alice.signer().peer;
    assert_eq!(
        carol.doc().oplog_vv().get(&peer.get()),
        alice.doc().oplog_vv().get(&peer.get())
    );
    assert_eq!(
        carol
            .doc()
            .get_list(Root::Messages.as_str())
            .get_deep_value(),
        alice
            .doc()
            .get_list(Root::Messages.as_str())
            .get_deep_value()
    );
    assert_eq!(
        carol.state.messages(carol.state.active_id()).expect("read")[0].text,
        "draft final"
    );
}

#[test]
fn a_registry_entry_its_signer_may_not_write_is_refused() {
    let alice = founder(ChannelType::Public);
    let bob = join(&alice);
    let segment = bob.segment();
    let peers = bob
        .doc()
        .get_map(ambre_core::crdt::FoundationRoot::Peers.as_str());

    // An entry under another PeerID would hand bob what alice created.
    let before = bob.doc().oplog_vv();
    peers
        .insert(
            ambre_core::PeerKey::of(alice.signer().peer).as_str(),
            entry(bob.user).into_bytes(),
        )
        .expect("insert");
    let foreign_key = bob.json_since(&before);
    // An entry under bob's own PeerID that names alice.
    let before = bob.doc().oplog_vv();
    peers
        .insert(
            ambre_core::PeerKey::of(bob.signer().peer).as_str(),
            entry(alice.user).into_bytes(),
        )
        .expect("insert");
    let foreign_user = bob.json_since(&before);

    let mut dave = join(&alice);
    let was = dave.everything();
    for (seq, delta) in [(1, &foreign_key), (2, &foreign_user)] {
        assert_eq!(
            refusal(
                dave.state
                    .apply_signed(segment, Seq::new(seq), delta, bob.signer())
            ),
            Some(Refusal::ForeignEntry)
        );
    }
    assert_eq!(dave.everything(), was);
}

// --- MURMURE-079 ---

#[test]
fn two_peers_accept_and_refuse_the_same_deltas() {
    let mut alice = founder(ChannelType::Public);
    let bob = join(&alice);
    let segment = alice.segment();
    let good = alice
        .state
        .send(&draft("first"), alice.user, NOW)
        .expect("send");
    let forged = alice
        .state
        .send(&draft("as bob"), bob.user, NOW)
        .expect("send");
    let own = alice
        .state
        .send(&draft("again"), alice.user, NOW)
        .expect("send");
    let deltas = [
        (&good.delta, alice.signer()),
        (&forged.delta, alice.signer()),
        (&own.delta, bob.signer()),
    ];

    let mut carol = join(&bob);
    let mut dave = join(&bob);
    let verdicts = |member: &mut Member| -> Vec<Option<Refusal>> {
        deltas
            .iter()
            .zip(1..)
            .map(|((delta, by), seq)| {
                refusal(
                    member
                        .state
                        .apply_signed(segment, Seq::new(seq), delta, *by),
                )
            })
            .collect()
    };
    let on_carol = verdicts(&mut carol);
    assert_eq!(on_carol, verdicts(&mut dave));
    assert_eq!(
        on_carol,
        [
            None,
            Some(Refusal::ForeignAuthor),
            Some(Refusal::NotSigners)
        ]
    );
    assert_eq!(
        carol
            .doc()
            .get_list(Root::Messages.as_str())
            .get_deep_value(),
        dave.doc()
            .get_list(Root::Messages.as_str())
            .get_deep_value()
    );
}

// --- MURMURE-029 ---

#[test]
fn a_delta_refused_while_the_channel_opens_is_left_out() {
    let mut alice = founder(ChannelType::Public);
    let bob = join(&alice);
    let state = &alice.state;
    let meta_id = state.meta_id();
    let segment = alice.segment();
    let meta = state.snapshot(meta_id.document_id()).expect("meta");
    let snapshot = state.snapshot(segment).expect("segment");
    let good = alice
        .state
        .send(&draft("kept"), alice.user, NOW)
        .expect("send");
    let forged = alice
        .state
        .send(&draft("as bob"), bob.user, NOW)
        .expect("send");

    let mut opening = OpeningChannel::new(alice.state.channel_id(), meta_id);
    assert_eq!(
        opening.receive(meta_id.document_id(), (&meta).into()),
        Receipt::Kept
    );
    assert!(matches!(
        opening.ready(meta_id.document_id()),
        Ok(Opening::OpenSegments(_))
    ));
    assert_eq!(opening.receive(segment, (&snapshot).into()), Receipt::Kept);
    opening
        .receive_signed(segment, Seq::new(1), &good.delta, alice.signer())
        .expect("kept until ready");
    opening
        .receive_signed(segment, Seq::new(2), &forged.delta, alice.signer())
        .expect("kept until ready");
    // Bytes that read as no update are refused on arrival.
    assert!(matches!(
        opening.receive_signed(
            segment,
            Seq::new(3),
            &Payload::<DeltaBytes>::new(b"loro".to_vec()),
            alice.signer()
        ),
        Err(MurmureError::Refused(Refusal::Binary))
    ));
    let Ok(Opening::Open(opened)) = opening.ready(segment) else {
        panic!("the channel opens");
    };
    let texts: Vec<String> = opened
        .messages(opened.active_id())
        .expect("read")
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(texts, ["kept"]);
    assert_eq!(opened.delivered(segment), Seq::new(3));
}

#[test]
fn a_meta_delta_under_another_peer_is_refused() {
    let mut alice = founder(ChannelType::Direct);
    let mut bob = join(&alice);
    let named = alice
        .state
        .name_direct(Some(&crate::fit_name("us")))
        .expect("name");
    let meta = named.meta.document_id();
    let by = Signer {
        peer: alice.state.peer(meta).expect("a session"),
        user: Some(alice.user),
    };
    let elsewhere = Signer {
        peer: ambre_core::PeerId::new(by.peer.get() + 1),
        ..by
    };
    assert_eq!(
        refusal(
            bob.state
                .apply_signed(meta, Seq::new(1), &named.delta, elsewhere)
        ),
        Some(Refusal::NotSigners)
    );
    assert_eq!(bob.state.name(), None);
    bob.state
        .apply_signed(meta, Seq::new(2), &named.delta, by)
        .expect("accepted");
    assert_eq!(
        bob.state.name().as_ref().map(ambre_core::Name::as_str),
        Some("us")
    );
}
