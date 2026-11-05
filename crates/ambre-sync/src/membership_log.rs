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
use ambre_core::{DocumentRank, FixedBytes, KeyId, PublicKey, Role, Seq};
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

    /// Applies one valid entry to the state.
    fn apply(&mut self, entry: &MembershipEntry) {
        match entry.act {
            MembershipAct::Create | MembershipAct::Invite | MembershipAct::SelfJoin => {
                self.held.insert(
                    entry.member,
                    Held {
                        role: entry.role,
                        key: entry.member_key,
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
        ambre_crypto::signature::verify_message(key, bytes.as_bytes(), &entry.signature).is_ok()
    })
}

/// What an entry records, before its place in the log and its signature.
pub(crate) struct Draft {
    pub document: Id<Document>,
    pub member: Id<User>,
    pub member_key: FixedBytes<PublicKey>,
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
        act: draft.act,
        role: draft.role,
        key: draft.key,
        previous: next.previous,
        author,
        signature: FixedBytes::<Signature>::from_array([0; ENTRY_SIGNATURE_LEN]),
    };
    let bytes = entry.signed_bytes()?;
    entry.signature = ambre_crypto::signature::sign_message(signing_key, bytes.as_bytes());
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
            act,
            role,
            key: Some(FixedBytes::from_array([log.len() as u8; 16])),
        };
        log.push(sign(draft, next, by.id, &by.key).unwrap());
    }

    /// A log Alice created, where she invited Bob as an editor.
    pub(crate) fn created(alice: &Person, bob: &Person) -> Vec<MembershipEntry> {
        let mut log = Vec::new();
        append(&mut log, alice, alice, MembershipAct::Create, Role::Owner);
        append(&mut log, alice, bob, MembershipAct::Invite, Role::Editor);
        log
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
