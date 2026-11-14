//! The signed membership log of one document
//! (`ambre-spec-membership.md section 3`).
//!
//! The relay stays the authority on access, and the log lets this client check
//! what it serves. The walk starts at the first entry, which must hash to the
//! anchor the wrapped key carried (`SYNC-104`). It checks each entry against the
//! state the entries before it built, and stops at the first that fails. A row
//! the walk never reached stands on the relay's word alone (`SYNC-103`).

use std::collections::{HashMap, HashSet};

use ambre_bus::MemberStanding;
use ambre_core::id::{Document, Id, User};
use ambre_core::{DocumentRank, ExchangePublicKey, FixedBytes, KeyId, PublicKey, Role, Seq};
use ambre_protocol::error::SerializationError;
use ambre_protocol::membership::ENTRY_SIGNATURE_LEN;
use ambre_protocol::{
    EntryDigest, LogPlaces, MembershipAct, MembershipEntry, Signature, place_after,
};
use ed25519_dalek::SigningKey;

/// The rule of the owning application that accepts a member's own entry
/// (`SYNC-106`). Documents have none yet, so every self-join fails there.
pub(crate) type SelfJoinRule = fn(&MembershipEntry) -> bool;

/// The rule documents apply: no self-join is valid.
pub(crate) fn no_self_join(_: &MembershipEntry) -> bool {
    false
}

/// The rule the `meta` of a public channel applies once a catalogue this
/// client holds lists the channel. A member joins alone, at no role above
/// editor (the Murmure spaces spec, `SRV-108`). The catalogue is the
/// admission; the relay checks the space membership behind it.
pub(crate) fn public_channel_self_join(entry: &MembershipEntry) -> bool {
    matches!(entry.role, Role::Editor | Role::Viewer)
}

/// What the log of a space says that a channel of it must heed (the Murmure
/// spaces spec). A space admin re-keys a public channel without being a
/// member of it. A space leave removes a member from each channel, with no
/// entry in the channel's own log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SpaceWord {
    /// Every key the space log granted a writing role, at any point.
    pub(crate) writers: HashSet<FixedBytes<PublicKey>>,
    /// The members a valid entry of the space log removed, and no later one
    /// added back.
    pub(crate) removed: HashSet<Id<User>>,
}

impl SpaceWord {
    /// What the walked log of a space says. An unanchored log verifies
    /// nobody, so it grants no writer.
    pub(crate) fn of(log: &Log) -> Self {
        Self {
            writers: log.signers().cloned().unwrap_or_default(),
            removed: log
                .held
                .iter()
                .filter(|(_, held)| !held.present)
                .map(|(member, _)| *member)
                .collect(),
        }
    }
}

/// The rule one document's log walks under (`SYNC-106`): its self-join rule,
/// and the word of the space a catalogue lists it in.
#[derive(Clone)]
pub(crate) struct LogRule {
    pub(crate) self_join: SelfJoinRule,
    pub(crate) space: Option<SpaceWord>,
}

impl LogRule {
    /// The rule of a document no catalogue lists: no self-join, no space.
    pub(crate) fn document() -> Self {
        Self {
            self_join: no_self_join,
            space: None,
        }
    }
}

/// What the log says of one member once the walk applied its entries.
#[derive(Clone)]
struct Held {
    role: Role,
    /// The signing key the entry that added the member named.
    key: FixedBytes<PublicKey>,
    /// The exchange key the entry that added the member named: the one key
    /// a rotation wraps the next key for (`SYNC-198`).
    exchange: FixedBytes<ExchangePublicKey>,
    /// Still a member: no later valid entry removed them.
    present: bool,
    /// The key the entry that added the member wraps (`SYNC-123`).
    wraps: Option<FixedBytes<KeyId>>,
}

/// The digest of one entry, as the next entry names it. `None` for an entry
/// that does not encode, which no decoded entry is.
pub(crate) fn digest(entry: &MembershipEntry) -> Option<FixedBytes<EntryDigest>> {
    let encoded = entry.encode().ok()?;
    Some(ambre_crypto::hash::entry_digest(&encoded))
}

/// The place and the previous digest the next entry of a log takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LogHead {
    /// The place the next entry is signed for.
    pub position: Seq<LogPlaces>,
    /// The digest of the last entry, absent on an empty log.
    pub previous: Option<FixedBytes<EntryDigest>>,
}

impl LogHead {
    /// The head of an empty log: the next entry creates it.
    pub(crate) const EMPTY: Self = Self {
        position: Seq::<LogPlaces>::ZERO,
        previous: None,
    };

    /// Whether the log holds no entry yet.
    pub(crate) fn is_empty(self) -> bool {
        self.position == Seq::ZERO
    }

    /// The head after an entry with `digest` lands at this one.
    pub(crate) const fn after(self, digest: FixedBytes<EntryDigest>) -> Self {
        Self {
            position: self.position.next(),
            previous: Some(digest),
        }
    }
}

/// One document's log, as far as a chain of valid entries reaches.
pub(crate) struct Log {
    held: HashMap<Id<User>, Held>,
    /// Every key the log granted a writing role, at any point.
    signers: HashSet<FixedBytes<PublicKey>>,
    /// Every document key a valid entry names, when its author held a
    /// writing role as it signed (`SYNC-200`).
    named_keys: HashSet<FixedBytes<KeyId>>,
    /// Whether the wrapped key carried an anchor at all.
    anchored: bool,
    /// The place and the previous digest the next entry takes.
    next: LogHead,
}

impl Log {
    /// Walks `entries` from the first, which must hash to `anchor`, and stops
    /// at the first entry that fails a check.
    pub(crate) fn walk(
        document: Id<Document>,
        anchor: Option<FixedBytes<EntryDigest>>,
        entries: &[MembershipEntry],
        self_join: SelfJoinRule,
    ) -> Self {
        let mut log = Self {
            held: HashMap::new(),
            signers: HashSet::new(),
            named_keys: HashSet::new(),
            anchored: anchor.is_some(),
            next: LogHead {
                position: place_after(entries.len()),
                previous: entries.last().and_then(digest),
            },
        };
        let mut previous = None;
        for (place, entry) in entries.iter().enumerate() {
            let fits = entry.document == document
                && place_after(place) == entry.position
                && entry.well_formed()
                && entry.previous == previous;
            let this = digest(entry);
            let valid = fits
                && match previous {
                    None => this.is_some() && this == anchor && log.creates(entry),
                    Some(_) => log.allows(entry, self_join),
                };
            if !valid {
                break;
            }
            if let Some(key) = log.writer_names(entry) {
                log.named_keys.insert(key);
            }
            log.apply(entry);
            previous = this;
        }
        log
    }

    /// [`Log::walk`] under `rule`, then what the space says. A space writer
    /// may sign a key link of the channel (the Murmure spaces spec). A member
    /// the space removed reads as removed here too, though no entry of this
    /// log says so.
    pub(crate) fn walk_under(
        document: Id<Document>,
        anchor: Option<FixedBytes<EntryDigest>>,
        entries: &[MembershipEntry],
        rule: &LogRule,
    ) -> Self {
        let mut log = Self::walk(document, anchor, entries, rule.self_join);
        if let Some(space) = &rule.space {
            log.signers.extend(space.writers.iter().cloned());
            for member in &space.removed {
                if let Some(held) = log.held.get_mut(member) {
                    held.present = false;
                }
            }
        }
        log
    }

    /// Whether `entry` is a valid first entry: the creator adds themselves as
    /// owner, and signs it.
    fn creates(&self, entry: &MembershipEntry) -> bool {
        entry.act == MembershipAct::Create
            && entry.author == entry.member
            && entry.role == Role::Owner
            && signed_by(entry, &entry.member_key)
    }

    /// Whether the state so far allows `entry`, and its author signed it.
    fn allows(&self, entry: &MembershipEntry, self_join: SelfJoinRule) -> bool {
        let author = self.held.get(&entry.author).filter(|held| held.present);
        let target = self.held.get(&entry.member).filter(|held| held.present);
        let author_rank = author.and_then(|held| held.role.document_rank());
        let granted = entry.role.document_rank();
        let allowed = match entry.act {
            MembershipAct::Create => false,
            // An editor or an owner adds a member at or below their own rank,
            // and touches no member above it, as the relay checks.
            MembershipAct::Invite => match (author_rank, granted) {
                (Some(by), Some(role)) => {
                    by >= DocumentRank::Editor
                        && role <= by
                        && target
                            .is_none_or(|held| held.role.document_rank().is_some_and(|t| t <= by))
                }
                _ => false,
            },
            MembershipAct::ChangeRole => {
                author_rank == Some(DocumentRank::Owner) && target.is_some() && granted.is_some()
            }
            // A member removes themselves, and an owner removes anyone.
            MembershipAct::Revoke => {
                target.is_some()
                    && (entry.author == entry.member || author_rank == Some(DocumentRank::Owner))
            }
            MembershipAct::SelfJoin => {
                entry.author == entry.member && granted.is_some() && self_join(entry)
            }
        };
        let key = match entry.act {
            MembershipAct::SelfJoin => Some(&entry.member_key),
            _ => author.map(|held| &held.key),
        };
        allowed && key.is_some_and(|key| signed_by(entry, key))
    }

    /// The key the valid `entry` names, when its author held a writing role
    /// as they signed it. A creator does; a self-joiner signs for themselves
    /// alone, so a self-join names no key a writer chose (`SYNC-200`).
    fn writer_names(&self, entry: &MembershipEntry) -> Option<FixedBytes<KeyId>> {
        let rank = match entry.act {
            MembershipAct::Create => Some(DocumentRank::Owner),
            MembershipAct::SelfJoin => None,
            MembershipAct::Invite | MembershipAct::ChangeRole | MembershipAct::Revoke => self
                .held
                .get(&entry.author)
                .filter(|held| held.present)
                .and_then(|held| held.role.document_rank()),
        };
        entry
            .key
            .filter(|_| rank.is_some_and(|rank| rank >= DocumentRank::Editor))
    }

    /// Applies one valid entry to the state.
    fn apply(&mut self, entry: &MembershipEntry) {
        match entry.act {
            MembershipAct::Create | MembershipAct::Invite | MembershipAct::SelfJoin => {
                self.held.insert(
                    entry.member,
                    Held {
                        role: entry.role,
                        key: entry.member_key,
                        exchange: entry.member_exchange_key,
                        present: true,
                        wraps: entry.key,
                    },
                );
            }
            MembershipAct::ChangeRole => {
                if let Some(held) = self.held.get_mut(&entry.member) {
                    held.role = entry.role;
                }
            }
            MembershipAct::Revoke => {
                if let Some(held) = self.held.get_mut(&entry.member) {
                    held.present = false;
                }
            }
        }
        if let Some(held) = self.held.get(&entry.member)
            && held.present
            && held
                .role
                .document_rank()
                .is_some_and(|r| r >= DocumentRank::Editor)
        {
            self.signers.insert(held.key);
        }
    }

    /// What the log says of `member`, and the role its latest valid entry
    /// gives them (`SYNC-108`, `SYNC-109`, `SYNC-110`).
    pub(crate) fn standing(&self, member: Id<User>) -> (MemberStanding, Option<Role>) {
        match self.held.get(&member) {
            Some(held) if held.present => (MemberStanding::Verified, Some(held.role)),
            Some(_) => (MemberStanding::Revoked, None),
            None => (MemberStanding::Unverified, None),
        }
    }

    /// The keys a key link may be signed by (`SYNC-121`): every key the log
    /// granted a writing role. `None` for a document whose wrap carried no
    /// anchor, which predates the log.
    pub(crate) fn signers(&self) -> Option<&HashSet<FixedBytes<PublicKey>>> {
        self.anchored.then_some(&self.signers)
    }

    /// The document keys a writer named in a valid entry (`SYNC-200`). `None`
    /// for a document whose wrap carried no anchor: no entry of it is
    /// checked, so none names a key.
    pub(crate) fn named_keys(&self) -> Option<&HashSet<FixedBytes<KeyId>>> {
        self.anchored.then_some(&self.named_keys)
    }

    /// The signing key of `member` while the log keeps them a member: the key a call
    /// checks a join against (`CALL-019`).
    pub(crate) fn present_key(&self, member: Id<User>) -> Option<FixedBytes<PublicKey>> {
        self.held
            .get(&member)
            .filter(|held| held.present)
            .map(|held| held.key)
    }

    /// The exchange key a valid entry signed for `member` while the log keeps
    /// them a member (`SYNC-198`). `None` for a member the walk never
    /// reached, so a row the relay alone serves names no key to wrap for.
    pub(crate) fn exchange_key(&self, member: Id<User>) -> Option<FixedBytes<ExchangePublicKey>> {
        self.held
            .get(&member)
            .filter(|held| held.present)
            .map(|held| held.exchange)
    }

    /// The key the entry that added `member` wraps, when the log reaches it.
    pub(crate) fn wraps(&self, member: Id<User>) -> Option<FixedBytes<KeyId>> {
        self.held
            .get(&member)
            .filter(|held| held.present)
            .and_then(|held| held.wraps)
    }

    /// The place and the previous digest the next entry takes. They follow
    /// every entry served, valid or not: the relay appends at its own end.
    pub(crate) fn next(&self) -> LogHead {
        self.next
    }
}

/// Whether `key` signed `entry`.
fn signed_by(entry: &MembershipEntry, key: &FixedBytes<PublicKey>) -> bool {
    entry.signed_bytes().is_ok_and(|bytes| {
        ambre_crypto::signature::verify_message(key, &bytes, &entry.signature).is_ok()
    })
}

/// What an entry records, before its place in the log and its signature.
pub(crate) struct Draft {
    pub document: Id<Document>,
    pub member: Id<User>,
    pub member_key: FixedBytes<PublicKey>,
    /// The member's exchange key, which the entry signs (`SYNC-197`).
    pub member_exchange_key: FixedBytes<ExchangePublicKey>,
    pub act: MembershipAct,
    pub role: Role,
    pub key: Option<FixedBytes<KeyId>>,
}

/// Signs `draft` as the entry after `next`, by `author` (`SYNC-132`).
///
/// # Errors
///
/// The encoder's error, which a well-formed entry never reaches.
pub(crate) fn sign(
    draft: Draft,
    next: LogHead,
    author: Id<User>,
    signing_key: &SigningKey,
) -> Result<MembershipEntry, SerializationError> {
    let mut entry = MembershipEntry {
        document: draft.document,
        position: next.position,
        member: draft.member,
        member_key: draft.member_key,
        member_exchange_key: draft.member_exchange_key,
        act: draft.act,
        role: draft.role,
        key: draft.key,
        previous: next.previous,
        author,
        signature: FixedBytes::<Signature>::from_array([0; ENTRY_SIGNATURE_LEN]),
    };
    let bytes = entry.signed_bytes()?;
    entry.signature = ambre_crypto::signature::sign_message(signing_key, &bytes);
    Ok(entry)
}

/// The form an entry takes in a request: its encoding, in base64.
///
/// # Errors
///
/// The encoder's error.
pub(crate) fn encode(
    entry: &MembershipEntry,
) -> Result<ambre_core::Encoded<ambre_core::EncodedLogEntry>, SerializationError> {
    entry.encode()
}

/// The entries a members answer carries, up to the first that does not
/// decode. The walk could not pass it anyway.
pub(crate) fn decode(
    texts: &[ambre_core::Encoded<ambre_core::EncodedLogEntry>],
) -> Vec<MembershipEntry> {
    texts
        .iter()
        .map_while(|text| MembershipEntry::decode(text).ok())
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One user of a test log: an identity and a signing key.
    pub(crate) struct Person {
        pub id: Id<User>,
        pub key: SigningKey,
    }

    impl Person {
        pub(crate) fn new(seed: u8) -> Self {
            Self {
                id: Id::<User>::from_bytes([seed; 16]),
                key: SigningKey::from_bytes(&[seed; 32]),
            }
        }

        pub(crate) fn public(&self) -> FixedBytes<PublicKey> {
            FixedBytes::from_array(self.key.verifying_key().to_bytes())
        }

        /// The exchange key of this person, from the same seed.
        pub(crate) fn exchange(&self) -> FixedBytes<ExchangePublicKey> {
            let secret = x25519_dalek::StaticSecret::from(*self.key.as_bytes());
            FixedBytes::from_array(x25519_dalek::PublicKey::from(&secret).to_bytes())
        }
    }

    pub(crate) const DOC: Id<Document> = Id::<Document>::from_bytes([7; 16]);

    /// Appends the entry `by` signs about `member` to `log`.
    pub(crate) fn append(
        log: &mut Vec<MembershipEntry>,
        by: &Person,
        member: &Person,
        act: MembershipAct,
        role: Role,
    ) {
        let next = LogHead {
            position: place_after(log.len()),
            previous: log.last().and_then(digest),
        };
        let draft = Draft {
            document: DOC,
            member: member.id,
            member_key: member.public(),
            member_exchange_key: member.exchange(),
            act,
            role,
            key: Some(FixedBytes::from_array([log.len() as u8; 16])),
        };
        log.push(sign(draft, next, by.id, &by.key).unwrap());
    }

    /// A walked log whose owner, seed 99, invites each of `members` as an
    /// editor, each under the exchange key given beside it. A rotation reads
    /// those keys as the ones the log signs (`SYNC-198`).
    pub(crate) fn signing(members: &[(Id<User>, FixedBytes<ExchangePublicKey>)]) -> Log {
        let owner = Person::new(99);
        let mut log = Vec::new();
        append(&mut log, &owner, &owner, MembershipAct::Create, Role::Owner);
        for (member, exchange) in members {
            let next = LogHead {
                position: place_after(log.len()),
                previous: log.last().and_then(digest),
            };
            let draft = Draft {
                document: DOC,
                member: *member,
                member_key: owner.public(),
                member_exchange_key: *exchange,
                act: MembershipAct::Invite,
                role: Role::Editor,
                key: None,
            };
            log.push(sign(draft, next, owner.id, &owner.key).unwrap());
        }
        Log::walk(DOC, anchor(&log), &log, no_self_join)
    }

    /// A log Alice created, where she invited Bob as an editor.
    pub(crate) fn created(alice: &Person, bob: &Person) -> Vec<MembershipEntry> {
        let mut log = Vec::new();
        append(&mut log, alice, alice, MembershipAct::Create, Role::Owner);
        append(&mut log, alice, bob, MembershipAct::Invite, Role::Editor);
        log
    }

    /// The bytes one entry signs, and its signature, as version 2 of the
    /// entry writes them, the exchange key included. A change of either
    /// breaks every log a relay stores.
    #[test]
    fn the_signed_bytes_and_the_signature_of_an_entry_stay_pinned() {
        let alice = Person::new(1);
        let log = created(&alice, &Person::new(2));
        let first = log.first().unwrap();
        let transcript = first.signed_bytes().unwrap();
        assert_eq!(
            hex::encode(transcript.as_bytes()),
            "616d6272652d6d656d626572736869702d656e7472792d763207070707070707\
             07070707070707070700010101010101010101010101010101018a88e3dd7409\
             f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5ca4e09292b651\
             c278b9772c569f5fa9bb13d906b46ab68c9df9dc2b4409f8a209000001000000\
             000000000000000000000000000001010101010101010101010101010101"
        );
        assert_eq!(
            hex::encode(first.signature.as_slice()),
            "be4f90107725d2c45ad60ef727944ab4a44ead9ce176abefb301d8830ce95d64\
             4c58fb5ad6745b6c9afb19616b16cd40f031afde1f900b720e10f74e4df8b305"
        );
        assert!(signed_by(first, &alice.public()));
    }

    fn anchor(log: &[MembershipEntry]) -> Option<FixedBytes<EntryDigest>> {
        log.first().and_then(digest)
    }

    fn walk(log: &[MembershipEntry]) -> Log {
        Log::walk(DOC, anchor(log), log, no_self_join)
    }

    /// Holds `SYNC-103`: a row counts as verified only when valid entries
    /// link it to the first. A log rooted elsewhere, or one whose link is
    /// broken, leaves every row past the break unverified.
    #[test]
    fn a_row_is_verified_only_through_a_chain_to_the_first_entry() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut log = created(&alice, &bob);
        append(&mut log, &bob, &carol, MembershipAct::Invite, Role::Viewer);
        let view = walk(&log);
        for member in [alice.id, bob.id, carol.id] {
            assert_eq!(view.standing(member).0, MemberStanding::Verified);
        }
        // Rooted at a creator the relay made: the anchor does not match.
        let mallory = Person::new(9);
        let forged = created(&mallory, &bob);
        let view = Log::walk(DOC, anchor(&log), &forged, no_self_join);
        assert_eq!(view.standing(bob.id).0, MemberStanding::Unverified);
        // An entry whose previous digest is not the one before it.
        let mut broken = log.clone();
        if let Some(entry) = broken.get_mut(2) {
            entry.previous = Some(FixedBytes::from_array([0; 32]));
        }
        let view = walk(&broken);
        assert_eq!(view.standing(bob.id).0, MemberStanding::Verified);
        assert_eq!(view.standing(carol.id).0, MemberStanding::Unverified);
        // No anchor at all: nothing verifies.
        let view = Log::walk(DOC, None, &log, no_self_join);
        assert_eq!(view.standing(alice.id).0, MemberStanding::Unverified);
    }

    /// Appends the entry `by` signs over `draft`, at the next place of `log`.
    fn append_draft(log: &mut Vec<MembershipEntry>, by: &Person, draft: Draft) {
        let next = LogHead {
            position: place_after(log.len()),
            previous: log.last().and_then(digest),
        };
        log.push(sign(draft, next, by.id, &by.key).unwrap());
    }

    /// The draft of `act` about `member` at `role`, in the test document.
    fn draft(member: &Person, act: MembershipAct, role: Role) -> Draft {
        Draft {
            document: DOC,
            member: member.id,
            member_key: member.public(),
            member_exchange_key: member.exchange(),
            act,
            role,
            key: Some(FixedBytes::from_array([0; 16])),
        }
    }

    /// Holds `SYNC-103` at the first entry: only an owner creating the log
    /// for themselves, under their own key, roots it. Any other first entry
    /// verifies nothing, its own member included.
    #[test]
    fn only_an_owner_creating_for_themselves_roots_the_log() {
        let (alice, bob) = (Person::new(1), Person::new(2));
        let firsts = [
            // Not a creation.
            draft(&alice, MembershipAct::Invite, Role::Owner),
            // Below the owning role.
            draft(&alice, MembershipAct::Create, Role::Editor),
            // For another member.
            draft(&bob, MembershipAct::Create, Role::Owner),
            // Under a key that is not the creator's.
            Draft {
                member_key: bob.public(),
                ..draft(&alice, MembershipAct::Create, Role::Owner)
            },
        ];
        for first in firsts {
            let mut log = Vec::new();
            append_draft(&mut log, &alice, first);
            let view = walk(&log);
            for member in [alice.id, bob.id] {
                assert_eq!(view.standing(member).0, MemberStanding::Unverified);
            }
        }
    }

    /// Holds `SYNC-103` past the first entry: an entry signed for another
    /// document ends the chain, even at the right place and link.
    #[test]
    fn an_entry_signed_for_another_document_ends_the_chain() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut log = created(&alice, &bob);
        append_draft(
            &mut log,
            &alice,
            Draft {
                document: Id::<Document>::from_bytes([8; 16]),
                ..draft(&carol, MembershipAct::Invite, Role::Viewer)
            },
        );
        assert_eq!(walk(&log).standing(carol.id).0, MemberStanding::Unverified);
    }

    /// Holds `SYNC-108` at the walk. An editor touches no member above their
    /// own rank. An owner revokes no one who is not a member. Each such entry
    /// ends the chain.
    #[test]
    fn an_entry_beyond_its_authors_standing_ends_the_chain() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut log = created(&alice, &bob);
        append(&mut log, &bob, &alice, MembershipAct::Invite, Role::Viewer);
        assert_eq!(
            walk(&log).standing(alice.id),
            (MemberStanding::Verified, Some(Role::Owner)),
            "the owner keeps the owning role"
        );
        let mut log = created(&alice, &bob);
        append(
            &mut log,
            &alice,
            &carol,
            MembershipAct::Revoke,
            Role::Viewer,
        );
        append(
            &mut log,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Viewer,
        );
        assert_eq!(walk(&log).standing(carol.id).0, MemberStanding::Unverified);
    }

    /// Holds `SYNC-106` for a public channel: a member joins alone at no role
    /// above editor.
    #[test]
    fn a_public_channel_admits_a_self_join_up_to_editor() {
        let dave = Person::new(4);
        let joins = |role| public_channel_self_join(&draft_entry(&dave, role));
        assert!(joins(Role::Viewer));
        assert!(joins(Role::Editor));
        assert!(!joins(Role::Owner));
        assert!(!joins(Role::Admin));
    }

    /// The self-join `member` signs at `role`, as the first entry of a log.
    fn draft_entry(member: &Person, role: Role) -> MembershipEntry {
        let mut log = Vec::new();
        append_draft(
            &mut log,
            member,
            draft(member, MembershipAct::SelfJoin, role),
        );
        log.remove(0)
    }

    /// Holds `SYNC-108` at the walk. An entry another key signed, or that the
    /// author's rank does not allow, ends the chain. The row it would add is
    /// then unverified.
    #[test]
    fn an_entry_its_author_did_not_sign_or_may_not_make_verifies_nothing() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut log = created(&alice, &bob);
        append(&mut log, &bob, &carol, MembershipAct::Invite, Role::Viewer);
        if let Some(entry) = log.get_mut(2) {
            entry.author = alice.id;
        }
        assert_eq!(walk(&log).standing(carol.id).0, MemberStanding::Unverified);

        // An editor may not grant the owning role.
        let mut log = created(&alice, &bob);
        append(&mut log, &bob, &carol, MembershipAct::Invite, Role::Owner);
        assert_eq!(walk(&log).standing(carol.id).0, MemberStanding::Unverified);

        // A viewer may not invite at all.
        let mut log = Vec::new();
        append(&mut log, &alice, &alice, MembershipAct::Create, Role::Owner);
        append(&mut log, &alice, &bob, MembershipAct::Invite, Role::Viewer);
        append(&mut log, &bob, &carol, MembershipAct::Invite, Role::Viewer);
        assert_eq!(walk(&log).standing(carol.id).0, MemberStanding::Unverified);
    }

    /// Holds `SYNC-106`: an entry a member signs to add themselves counts only
    /// under a rule of the owning application. Documents have none.
    #[test]
    fn a_self_join_counts_only_under_the_applications_rule() {
        let (alice, bob, dave) = (Person::new(1), Person::new(2), Person::new(4));
        let mut log = created(&alice, &bob);
        append(
            &mut log,
            &dave,
            &dave,
            MembershipAct::SelfJoin,
            Role::Viewer,
        );
        assert_eq!(walk(&log).standing(dave.id).0, MemberStanding::Unverified);
        let view = Log::walk(DOC, anchor(&log), &log, |_| true);
        assert_eq!(view.standing(dave.id).0, MemberStanding::Verified);
    }

    /// Holds `SYNC-200` in the log. A creation and an invite name their key,
    /// as their authors write. A self-join and a reader's own departure name
    /// none, and neither does an unanchored log.
    #[test]
    fn only_a_writers_entry_names_a_key() {
        let (alice, bob, carol, dave) = (
            Person::new(1),
            Person::new(2),
            Person::new(3),
            Person::new(4),
        );
        let mut log = created(&alice, &bob);
        append(&mut log, &bob, &carol, MembershipAct::Invite, Role::Viewer);
        append(
            &mut log,
            &dave,
            &dave,
            MembershipAct::SelfJoin,
            Role::Viewer,
        );
        append(
            &mut log,
            &carol,
            &carol,
            MembershipAct::Revoke,
            Role::Viewer,
        );
        let view = Log::walk(DOC, anchor(&log), &log, |_| true);
        assert_eq!(view.standing(carol.id).0, MemberStanding::Revoked);
        let named: HashSet<_> = [0u8, 1, 2]
            .into_iter()
            .map(|n| FixedBytes::<KeyId>::from_array([n; 16]))
            .collect();
        assert_eq!(view.named_keys(), Some(&named));
        assert_eq!(Log::walk(DOC, None, &log, |_| true).named_keys(), None);
    }

    /// The owner leaves last and the log holds no member. A newcomer's
    /// self-join then extends it and counts, where the application allows
    /// one.
    #[test]
    fn a_log_its_last_member_left_admits_the_next_self_join() {
        let (alice, dave) = (Person::new(1), Person::new(4));
        let mut log = Vec::new();
        append(&mut log, &alice, &alice, MembershipAct::Create, Role::Owner);
        append(&mut log, &alice, &alice, MembershipAct::Revoke, Role::Owner);
        append(
            &mut log,
            &dave,
            &dave,
            MembershipAct::SelfJoin,
            Role::Editor,
        );
        let view = Log::walk(DOC, anchor(&log), &log, |_| true);
        assert_eq!(
            view.standing(dave.id),
            (MemberStanding::Verified, Some(Role::Editor))
        );
        assert_eq!(view.standing(alice.id).0, MemberStanding::Revoked);
    }

    /// Holds `SYNC-109`: an invite served again after a revoke of the same
    /// member changes nothing. Appended at the end, it does not link, and the
    /// member stays revoked.
    #[test]
    fn an_invite_replayed_after_a_revoke_leaves_the_member_revoked() {
        let (alice, bob) = (Person::new(1), Person::new(2));
        let mut log = created(&alice, &bob);
        append(&mut log, &alice, &bob, MembershipAct::Revoke, Role::Editor);
        assert_eq!(walk(&log).standing(bob.id), (MemberStanding::Revoked, None));
        let replayed = log.get(1).cloned().unwrap();
        log.push(replayed);
        assert_eq!(walk(&log).standing(bob.id), (MemberStanding::Revoked, None));
        // A member who leaves signs their own revoke.
        let mut log = created(&alice, &bob);
        append(&mut log, &bob, &bob, MembershipAct::Revoke, Role::Editor);
        assert_eq!(walk(&log).standing(bob.id).0, MemberStanding::Revoked);
    }

    /// Holds `SYNC-197` at the walk: the exchange key a member holds is the one
    /// the entry that added them signs. A role change keeps it, a revoke
    /// drops it, and an entry with a swapped key no longer checks.
    #[test]
    fn the_exchange_key_is_the_one_the_adding_entry_signs() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut log = created(&alice, &bob);
        let view = walk(&log);
        assert_eq!(view.exchange_key(alice.id), Some(alice.exchange()));
        assert_eq!(view.exchange_key(bob.id), Some(bob.exchange()));
        assert_eq!(view.exchange_key(carol.id), None, "a member never added");
        append_draft(
            &mut log,
            &alice,
            Draft {
                member_exchange_key: carol.exchange(),
                ..draft(&bob, MembershipAct::ChangeRole, Role::Viewer)
            },
        );
        assert_eq!(walk(&log).exchange_key(bob.id), Some(bob.exchange()));
        append(&mut log, &alice, &bob, MembershipAct::Revoke, Role::Viewer);
        assert_eq!(walk(&log).exchange_key(bob.id), None);
        // The key is under the signature: swapping it ends the chain there.
        let mut swapped = created(&alice, &bob);
        if let Some(entry) = swapped.get_mut(1) {
            entry.member_exchange_key = carol.exchange();
        }
        assert_eq!(walk(&swapped).exchange_key(bob.id), None);
    }

    /// Holds `SYNC-110`: the role shown is the one the latest valid entry
    /// gives, whatever the relay's row says.
    #[test]
    fn the_role_is_the_latest_valid_entrys() {
        let (alice, bob) = (Person::new(1), Person::new(2));
        let mut log = created(&alice, &bob);
        assert_eq!(walk(&log).standing(bob.id).1, Some(Role::Editor));
        append(
            &mut log,
            &alice,
            &bob,
            MembershipAct::ChangeRole,
            Role::Viewer,
        );
        assert_eq!(walk(&log).standing(bob.id).1, Some(Role::Viewer));
        // An editor may not change a role: the entry ends the chain.
        append(&mut log, &bob, &bob, MembershipAct::ChangeRole, Role::Owner);
        assert_eq!(walk(&log).standing(bob.id).1, Some(Role::Viewer));
    }

    /// The next entry follows every entry served, and the signers are the
    /// keys the log granted a writing role.
    #[test]
    fn the_next_entry_follows_the_served_log_and_writers_sign_links() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut log = created(&alice, &bob);
        append(
            &mut log,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Viewer,
        );
        let view = walk(&log);
        assert_eq!(
            view.next(),
            LogHead {
                position: Seq::<LogPlaces>::new(3),
                previous: log.last().and_then(digest),
            }
        );
        let signers = view.signers().unwrap();
        assert!(signers.contains(&alice.public()) && signers.contains(&bob.public()));
        assert!(!signers.contains(&carol.public()));
        assert_eq!(view.wraps(bob.id), Some(FixedBytes::from_array([1; 16])));
        assert!(Log::walk(DOC, None, &log, no_self_join).signers().is_none());
        // The entries travel through their request form.
        let texts: Vec<ambre_core::Encoded<ambre_core::EncodedLogEntry>> =
            log.iter().map(|e| encode(e).unwrap()).collect();
        assert_eq!(decode(&texts), log);
    }

    /// A channel's log heeds its space (the Murmure spaces spec). A writer of
    /// the space signs a link of the channel without an entry in it. A member
    /// the space removed reads as removed in the channel.
    #[test]
    fn a_channel_log_heeds_the_word_of_its_space() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut space = created(&alice, &bob);
        append(&mut space, &bob, &bob, MembershipAct::Revoke, Role::Editor);
        let word = SpaceWord::of(&walk(&space));
        assert!(word.writers.contains(&alice.public()));
        assert_eq!(word.removed, HashSet::from([bob.id]));
        let unanchored = Log::walk(DOC, None, &space, no_self_join);
        assert_eq!(
            SpaceWord::of(&unanchored),
            SpaceWord::default(),
            "it says nothing"
        );

        let channel = created(&carol, &bob);
        let rule = LogRule {
            self_join: public_channel_self_join,
            space: Some(word),
        };
        let view = Log::walk_under(DOC, anchor(&channel), &channel, &rule);
        assert_eq!(view.standing(bob.id).0, MemberStanding::Revoked);
        assert_eq!(view.standing(carol.id).0, MemberStanding::Verified);
        assert!(view.signers().unwrap().contains(&alice.public()));
        let alone = Log::walk_under(DOC, anchor(&channel), &channel, &LogRule::document());
        assert_eq!(alone.standing(bob.id).0, MemberStanding::Verified);
        assert!(!alone.signers().unwrap().contains(&alice.public()));
    }

    /// The space speaks from its latest entries (the Murmure spaces spec). A
    /// removal a later join undid removes no one from a channel. A member of
    /// the space alone stays unverified in a channel that holds no entry of
    /// theirs.
    #[test]
    fn a_space_word_follows_its_latest_entries_and_adds_no_member() {
        let (alice, bob, carol, dave) = (
            Person::new(1),
            Person::new(2),
            Person::new(3),
            Person::new(4),
        );
        let mut space = created(&alice, &bob);
        append(
            &mut space,
            &alice,
            &bob,
            MembershipAct::Revoke,
            Role::Editor,
        );
        append(
            &mut space,
            &alice,
            &bob,
            MembershipAct::Invite,
            Role::Viewer,
        );
        append(
            &mut space,
            &alice,
            &dave,
            MembershipAct::Invite,
            Role::Editor,
        );
        let word = SpaceWord::of(&walk(&space));
        assert!(word.removed.is_empty(), "the join undid the removal");

        let channel = created(&carol, &bob);
        let rule = LogRule {
            self_join: public_channel_self_join,
            space: Some(word),
        };
        let view = Log::walk_under(DOC, anchor(&channel), &channel, &rule);
        assert_eq!(view.standing(bob.id).0, MemberStanding::Verified);
        assert_eq!(view.standing(alice.id).0, MemberStanding::Unverified);
        assert_eq!(view.standing(dave.id), (MemberStanding::Unverified, None));
    }
}
