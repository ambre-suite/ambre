//! One entry of a document's signed membership log
//! (`ambre-spec-membership.md section 3`).
//!
//! An invite, a revoke and a role change each add one entry, signed by the
//! member who acts (`SYNC-132`). Each names the digest of the entry before it
//! (`SYNC-102`). A client walks the log from its first entry, and so checks
//! what the relay serves (`SYNC-103`). The relay stores the entries and checks
//! the signature, and computes no digest.

use ambre_core::id::{Document, Id, User};
use ambre_core::{
    Encoded, EncodedLogEntry, ExchangePublicKey, FixedBytes, KeyId, PublicKey, Role, Seq,
};
use serde::{Deserialize, Serialize};

use crate::error::SerializationError;
use crate::messages::{DeltaAuthor, Signature};
use crate::transcript::SignedTranscript;

/// The domain the signed bytes open with, so an entry's signature serves no
/// other use. Version 2 signs the member's exchange key, and version 3 the
/// cap of a revoke. An entry of an older version checks under no key, so a
/// log verifies nothing past one.
pub const ENTRY_DOMAIN: &[u8] = b"ambre-membership-entry-v3";

/// The length of an Ed25519 signature.
pub const ENTRY_SIGNATURE_LEN: usize = 64;

/// The most members a direct message holds, its creator included
/// (`ambre-spec-budgets.md` section 6). Each member costs the relay one
/// lookup and one signature check, so the cap bounds one request.
pub const DIRECT_MESSAGE_MAX_MEMBERS: usize = 50;

/// What one entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MembershipAct {
    /// The document is created: the creator is its first member.
    Create,
    /// A member adds another, with a copy of the key.
    Invite,
    /// A member is removed, by an owner or by themselves.
    Revoke,
    /// An owner changes the role of a member.
    ChangeRole,
    /// A member adds themselves, valid only under a rule of the owning
    /// application (`SYNC-106`).
    SelfJoin,
}

/// The digest of one entry, which the next entry names. `ambre-crypto` seals
/// it beside a wrapped key, so it lives in `ambre-core`.
pub use ambre_core::envelope::EntryDigest;

/// The acts of a log that leaves every member an editor and no owner, in
/// order. `creator` creates the document as its owner, invites each of
/// `members` as an editor, then lowers its own role to editor.
///
/// The writer signs these acts and the relay expects them, so both read the
/// one sequence. Each act is `(member, act, role after it)`, and `creator`
/// authors them all.
pub fn ownerless_log(
    creator: Id<User>,
    members: &[Id<User>],
) -> impl Iterator<Item = (Id<User>, MembershipAct, Role)> + '_ {
    std::iter::once((creator, MembershipAct::Create, Role::Owner))
        .chain(
            members
                .iter()
                .map(|&member| (member, MembershipAct::Invite, Role::Editor)),
        )
        .chain(std::iter::once((
            creator,
            MembershipAct::ChangeRole,
            Role::Editor,
        )))
}

/// The places of the entries of a document's membership log. A call key
/// names one too, so `ambre-core` holds the marker (`CALL-159`).
pub use ambre_core::LogPlaces;

/// The place a log of `length` entries gives its next entry. A length past
/// `u64::MAX` saturates, which no log reaches.
#[must_use]
pub fn place_after(length: usize) -> Seq<LogPlaces> {
    Seq::new(u64::try_from(length).unwrap_or(u64::MAX))
}

/// The head of a membership log: the place its next entry takes, and the
/// digest of its last entry. The head of an empty log names no digest.
///
/// An update carries the head its author last read, under its signature
/// (`SYNC-206`). A peer that reads a head past its own reads the log again.
/// So it sees an entry the relay withheld from it (`SYNC-207`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogHead {
    /// The place the next entry is signed for: the count of entries.
    pub position: Seq<LogPlaces>,
    /// The digest of the last entry, absent on an empty log.
    pub previous: Option<FixedBytes<EntryDigest>>,
}

impl LogHead {
    /// The head of an empty log: the next entry creates it.
    pub const EMPTY: Self = Self {
        position: Seq::<LogPlaces>::ZERO,
        previous: None,
    };

    /// Whether the log holds no entry yet.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.position == Seq::ZERO
    }

    /// The head after an entry with `digest` lands at this one.
    #[must_use]
    pub const fn after(self, digest: FixedBytes<EntryDigest>) -> Self {
        Self {
            position: self.position.next(),
            previous: Some(digest),
        }
    }

    /// Whether `other` names a log this head has not reached: a later place,
    /// or the same place with another last entry. A head behind this one
    /// names nothing new.
    #[must_use]
    pub fn is_behind(self, other: Self) -> bool {
        other.position > self.position
            || (other.position == self.position && other.previous != self.previous)
    }
}

/// One signed change of membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipEntry {
    /// The document whose membership changes.
    pub document: Id<Document>,
    /// The place of this entry in the log, from zero. The relay appends an
    /// entry at its own place alone, so two racing writers never fork the log.
    pub position: Seq<LogPlaces>,
    /// The member the entry adds, removes or changes.
    pub member: Id<User>,
    /// The member's signing key, so an entry that member signs later checks
    /// against the log alone.
    pub member_key: FixedBytes<PublicKey>,
    /// The member's exchange key. A rotation wraps the next key for the key
    /// the log signs, never for one the relay alone serves (`SYNC-197`).
    pub member_exchange_key: FixedBytes<ExchangePublicKey>,
    /// What the entry records.
    pub act: MembershipAct,
    /// The member's role after the entry.
    pub role: Role,
    /// The key an invite or a self-join wraps (`SYNC-123`), and the key a
    /// creation or a revoke installs.
    pub key: Option<FixedBytes<KeyId>>,
    /// The digest of the entry before this one, absent on the first alone.
    pub previous: Option<FixedBytes<EntryDigest>>,
    /// The member who made the change and signed it.
    pub author: Id<User>,
    /// On a revoke, the highest clock of the revoked member's deltas its
    /// author applied (`SYNC-300`). A delta of that member past the cap
    /// counts nowhere. `None` on every other act.
    pub cap: Option<Seq<DeltaAuthor>>,
    /// The author's signature over [`MembershipEntry::signed_bytes`]. It
    /// encodes as its 64 bytes, with no length prefix.
    pub signature: FixedBytes<Signature>,
}

/// The fields a signature covers: every field but the signature.
#[derive(Serialize)]
struct Signed<'a> {
    document: &'a Id<Document>,
    position: Seq<LogPlaces>,
    member: &'a Id<User>,
    member_key: &'a FixedBytes<PublicKey>,
    member_exchange_key: &'a FixedBytes<ExchangePublicKey>,
    act: MembershipAct,
    role: Role,
    key: &'a Option<FixedBytes<KeyId>>,
    previous: &'a Option<FixedBytes<EntryDigest>>,
    author: &'a Id<User>,
    cap: &'a Option<Seq<DeltaAuthor>>,
}

impl MembershipEntry {
    /// The bytes the author signs: the domain, then every field but the
    /// signature.
    ///
    /// # Errors
    ///
    /// [`SerializationError::Postcard`] from the encoder.
    pub fn signed_bytes(&self) -> Result<SignedTranscript, SerializationError> {
        let signed = Signed {
            document: &self.document,
            position: self.position,
            member: &self.member,
            member_key: &self.member_key,
            member_exchange_key: &self.member_exchange_key,
            act: self.act,
            role: self.role,
            key: &self.key,
            previous: &self.previous,
            author: &self.author,
            cap: &self.cap,
        };
        let mut bytes = ENTRY_DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(&signed)?);
        Ok(SignedTranscript::new(bytes))
    }

    /// Whether the entry's shape is one a log can hold: the first entry alone
    /// lacks a previous digest. A signature of another length does not
    /// decode.
    #[must_use]
    pub fn well_formed(&self) -> bool {
        (self.position == Seq::ZERO) == self.previous.is_none()
    }

    /// The bytes a request carries and the relay stores.
    ///
    /// # Errors
    ///
    /// [`SerializationError::Postcard`] from the encoder.
    pub fn encode(&self) -> Result<Encoded<EncodedLogEntry>, SerializationError> {
        Ok(Encoded::new(postcard::to_allocvec(self)?))
    }

    /// Reads back what [`Self::encode`] wrote, and nothing else. The relay
    /// stores the bytes it was sent, and the next entry's digest covers the
    /// encoding. Bytes that decode to the same entry but differ from its
    /// encoding are refused (`SRV-227`).
    ///
    /// # Errors
    ///
    /// * [`SerializationError::Postcard`] for bytes that are not an entry.
    /// * [`SerializationError::NotCanonical`] for bytes past the entry, or a
    ///   field written in a longer form than its encoding.
    pub fn decode(encoded: &Encoded<EncodedLogEntry>) -> Result<Self, SerializationError> {
        let entry: Self = postcard::from_bytes(encoded.as_bytes())?;
        if entry.encode()? != *encoded {
            return Err(SerializationError::NotCanonical);
        }
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> MembershipEntry {
        MembershipEntry {
            document: Id::<Document>::from_bytes([1; 16]),
            position: Seq::<LogPlaces>::new(1),
            member: Id::<User>::from_bytes([2; 16]),
            member_key: FixedBytes::from_array([3; 32]),
            member_exchange_key: FixedBytes::from_array([10; 32]),
            act: MembershipAct::Invite,
            role: Role::Editor,
            key: Some(FixedBytes::from_array([4; 16])),
            previous: Some(FixedBytes::from_array([5; 32])),
            author: Id::<User>::from_bytes([6; 16]),
            cap: None,
            signature: FixedBytes::from_array([7; ENTRY_SIGNATURE_LEN]),
        }
    }

    /// Holds `SYNC-102`: an entry names its document, its member, its act and
    /// the digest before it, and each is under the signature. Changing any
    /// field changes the bytes signed, and the signature itself does not.
    #[test]
    fn every_field_but_the_signature_is_signed() {
        let base = entry();
        let signed = base.signed_bytes().unwrap();
        assert!(signed.as_bytes().starts_with(ENTRY_DOMAIN));
        let changes: [fn(&mut MembershipEntry); 11] = [
            |e| e.document = Id::<Document>::from_bytes([9; 16]),
            |e| e.position = Seq::<LogPlaces>::new(2),
            |e| e.member = Id::<User>::from_bytes([9; 16]),
            |e| e.member_key = FixedBytes::from_array([9; 32]),
            |e| e.member_exchange_key = FixedBytes::from_array([9; 32]),
            |e| e.act = MembershipAct::Revoke,
            |e| e.role = Role::Viewer,
            |e| e.key = None,
            |e| e.previous = Some(FixedBytes::from_array([9; 32])),
            |e| e.author = Id::<User>::from_bytes([9; 16]),
            |e| e.cap = Some(Seq::<DeltaAuthor>::new(9)),
        ];
        for change in changes {
            let mut changed = base.clone();
            change(&mut changed);
            assert_ne!(changed.signed_bytes().unwrap(), signed);
        }
        let mut resigned = base.clone();
        resigned.signature = FixedBytes::from_array([8; ENTRY_SIGNATURE_LEN]);
        assert_eq!(resigned.signed_bytes().unwrap(), signed);
    }

    /// The signed fields as the hand-written `LogPosition` wrote them: a
    /// transparent `u64` in the place of the position.
    #[derive(Serialize)]
    struct OldSigned<'a> {
        document: &'a Id<Document>,
        position: u64,
        member: &'a Id<User>,
        member_key: &'a FixedBytes<PublicKey>,
        member_exchange_key: &'a FixedBytes<ExchangePublicKey>,
        act: MembershipAct,
        role: Role,
        key: &'a Option<FixedBytes<KeyId>>,
        previous: &'a Option<FixedBytes<EntryDigest>>,
        author: &'a Id<User>,
        cap: &'a Option<Seq<DeltaAuthor>>,
    }

    /// A place writes the bytes the hand-written `LogPosition` wrote, so the
    /// signed transcript, and the digest the next entry names, keep theirs.
    #[test]
    fn a_place_keeps_the_signed_bytes_of_the_type_it_replaced() {
        for raw in [0, 1, 300, u64::MAX] {
            let mut sent = entry();
            sent.position = Seq::<LogPlaces>::new(raw);
            let old = OldSigned {
                document: &sent.document,
                position: raw,
                member: &sent.member,
                member_key: &sent.member_key,
                member_exchange_key: &sent.member_exchange_key,
                act: sent.act,
                role: sent.role,
                key: &sent.key,
                previous: &sent.previous,
                author: &sent.author,
                cap: &sent.cap,
            };
            let mut bytes = ENTRY_DOMAIN.to_vec();
            bytes.extend(postcard::to_allocvec(&old).unwrap());
            assert_eq!(sent.signed_bytes().unwrap().as_bytes(), bytes.as_slice());
            let place = Seq::<LogPlaces>::new(raw);
            assert_eq!(
                postcard::to_allocvec(&place).unwrap(),
                postcard::to_allocvec(&raw).unwrap()
            );
        }
        assert_eq!(format!("{:?}", Seq::<LogPlaces>::new(3)), "LogPosition(3)");
        assert_eq!(place_after(0), Seq::ZERO);
        assert_eq!(place_after(2), Seq::<LogPlaces>::new(2));
        assert_eq!(place_after(2).next(), Seq::<LogPlaces>::new(3));
    }

    /// The first entry alone has no previous digest, and an entry travels
    /// whole through its encoding.
    #[test]
    fn an_entry_names_a_previous_digest_unless_it_is_the_first() {
        let mut first = entry();
        assert!(first.well_formed());
        first.position = Seq::<LogPlaces>::ZERO;
        assert!(!first.well_formed());
        first.previous = None;
        assert!(first.well_formed());
        let sent = entry();
        assert_eq!(
            MembershipEntry::decode(&sent.encode().unwrap()).unwrap(),
            sent
        );
    }

    /// The signature closes the encoding as its bytes alone, with no length
    /// prefix: the type fixes the length. A signature of another length does
    /// not decode.
    #[test]
    fn the_signature_closes_the_encoding_as_its_bare_bytes() {
        let sent = entry().encode().unwrap().into_bytes();
        let tail = sent.len() - ENTRY_SIGNATURE_LEN;
        assert_ne!(sent[tail - 1], ENTRY_SIGNATURE_LEN as u8);
        assert_eq!(sent[tail..], [7; ENTRY_SIGNATURE_LEN]);
        let short = sent[..sent.len() - 1].to_vec();
        assert!(MembershipEntry::decode(&Encoded::new(short)).is_err());
    }

    /// `SRV-227`: an entry reads from its own encoding alone. A byte past its
    /// end, and a position written in a longer varint, are refused.
    #[test]
    fn an_entry_reads_from_its_own_encoding_alone() {
        let sent = entry().encode().unwrap().into_bytes();
        let mut trailing = sent.clone();
        trailing.push(0);
        assert_eq!(
            MembershipEntry::decode(&Encoded::new(trailing)),
            Err(SerializationError::NotCanonical)
        );
        // The position follows the 16 bytes of the document id. Position 1 is
        // the varint `01`; `81 00` is the same value in a longer form.
        assert_eq!(sent[16], 0x01);
        let mut longer = sent[..16].to_vec();
        longer.extend([0x81, 0x00]);
        longer.extend(&sent[17..]);
        assert!(MembershipEntry::decode(&Encoded::new(longer)).is_err());
    }

    /// An ownerless log opens with its creation, and leaves the creator and
    /// every member an editor, with no owner.
    #[test]
    fn an_ownerless_log_leaves_every_member_an_editor() {
        let creator = Id::<User>::mint();
        let members = [Id::<User>::mint(), Id::<User>::mint()];
        let acts: Vec<_> = ownerless_log(creator, &members).collect();
        assert_eq!(
            acts.first(),
            Some(&(creator, MembershipAct::Create, Role::Owner))
        );
        assert_eq!(acts.len(), members.len() + 2);
        let mut roles = std::collections::HashMap::new();
        for (member, _, role) in acts {
            roles.insert(member, role);
        }
        assert_eq!(roles.len(), members.len() + 1);
        assert!(roles.values().all(|role| *role == Role::Editor));
    }
}
