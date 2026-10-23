//! One entry of a document's signed membership log
//! (`ambre-spec-membership.md section 3`).
//!
//! An invite, a revoke and a role change each add one entry, signed by the
//! member who acts (`SYNC-132`). Each names the digest of the entry before it
//! (`SYNC-102`). A client walks the log from its first entry, and so checks
//! what the relay serves (`SYNC-103`). The relay stores the entries and checks
//! the signature, and computes no digest.

use ambre_core::{DocumentId, KeyId, PublicKey, Role, UserId};
use serde::{Deserialize, Serialize};

use crate::error::SerializationError;
use crate::messages::Signature;

/// The domain the signed bytes open with, so an entry's signature serves no
/// other use.
pub const ENTRY_DOMAIN: &[u8] = b"ambre-membership-entry-v1";

/// The length of an Ed25519 signature.
pub const ENTRY_SIGNATURE_LEN: usize = 64;

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
    creator: UserId,
    members: &[UserId],
) -> impl Iterator<Item = (UserId, MembershipAct, Role)> + '_ {
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

/// One signed change of membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipEntry {
    /// The document whose membership changes.
    pub document: DocumentId,
    /// The place of this entry in the log, from zero. The relay appends an
    /// entry at its own place alone, so two racing writers never fork the log.
    pub position: u64,
    /// The member the entry adds, removes or changes.
    pub member: UserId,
    /// The member's signing key, so an entry that member signs later checks
    /// against the log alone.
    pub member_key: ambre_core::PublicKey,
    /// What the entry records.
    pub act: MembershipAct,
    /// The member's role after the entry.
    pub role: Role,
    /// The key an invite or a self-join wraps (`SYNC-123`), and the key a
    /// creation or a revoke installs.
    pub key: Option<KeyId>,
    /// The digest of the entry before this one, absent on the first alone.
    pub previous: Option<EntryDigest>,
    /// The member who made the change and signed it.
    pub author: UserId,
    /// The author's signature over [`MembershipEntry::signed_bytes`]. It
    /// encodes as a length-prefixed byte string, as the `Vec<u8>` it replaced
    /// did, so a stored entry and its digest keep their bytes.
    pub signature: Signature,
}

/// The fields a signature covers: every field but the signature.
#[derive(Serialize)]
struct Signed<'a> {
    document: &'a DocumentId,
    position: u64,
    member: &'a UserId,
    member_key: &'a PublicKey,
    act: MembershipAct,
    role: Role,
    key: &'a Option<KeyId>,
    previous: &'a Option<EntryDigest>,
    author: &'a UserId,
}

impl MembershipEntry {
    /// The bytes the author signs: the domain, then every field but the
    /// signature.
    ///
    /// # Errors
    ///
    /// [`SerializationError::Postcard`] from the encoder.
    pub fn signed_bytes(&self) -> Result<Vec<u8>, SerializationError> {
        let signed = Signed {
            document: &self.document,
            position: self.position,
            member: &self.member,
            member_key: &self.member_key,
            act: self.act,
            role: self.role,
            key: &self.key,
            previous: &self.previous,
            author: &self.author,
        };
        let mut bytes = ENTRY_DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(&signed)?);
        Ok(bytes)
    }

    /// Whether the entry's shape is one a log can hold: the first entry alone
    /// lacks a previous digest. A signature of another length does not
    /// decode.
    #[must_use]
    pub fn well_formed(&self) -> bool {
        (self.position == 0) == self.previous.is_none()
    }

    /// The bytes a request carries and the relay stores.
    ///
    /// # Errors
    ///
    /// [`SerializationError::Postcard`] from the encoder.
    pub fn encode(&self) -> Result<Vec<u8>, SerializationError> {
        Ok(postcard::to_allocvec(self)?)
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
    pub fn decode(bytes: &[u8]) -> Result<Self, SerializationError> {
        let entry: Self = postcard::from_bytes(bytes)?;
        if entry.encode()? != bytes {
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
            document: DocumentId::from_bytes([1; 16]),
            position: 1,
            member: UserId::from_bytes([2; 16]),
            member_key: PublicKey::from_array([3; 32]),
            act: MembershipAct::Invite,
            role: Role::Editor,
            key: Some(KeyId::from_array([4; 16])),
            previous: Some(EntryDigest([5; 32])),
            author: UserId::from_bytes([6; 16]),
            signature: Signature([7; ENTRY_SIGNATURE_LEN]),
        }
    }

    /// Holds `SYNC-102`: an entry names its document, its member, its act and
    /// the digest before it, and each is under the signature. Changing any
    /// field changes the bytes signed, and the signature itself does not.
    #[test]
    fn every_field_but_the_signature_is_signed() {
        let base = entry();
        let signed = base.signed_bytes().unwrap();
        assert!(signed.starts_with(ENTRY_DOMAIN));
        let changes: [fn(&mut MembershipEntry); 9] = [
            |e| e.document = DocumentId::from_bytes([9; 16]),
            |e| e.position = 2,
            |e| e.member = UserId::from_bytes([9; 16]),
            |e| e.member_key = PublicKey::from_array([9; 32]),
            |e| e.act = MembershipAct::Revoke,
            |e| e.role = Role::Viewer,
            |e| e.key = None,
            |e| e.previous = Some(EntryDigest([9; 32])),
            |e| e.author = UserId::from_bytes([9; 16]),
        ];
        for change in changes {
            let mut changed = base.clone();
            change(&mut changed);
            assert_ne!(changed.signed_bytes().unwrap(), signed);
        }
        let mut resigned = base.clone();
        resigned.signature = Signature([8; ENTRY_SIGNATURE_LEN]);
        assert_eq!(resigned.signed_bytes().unwrap(), signed);
    }

    /// The first entry alone has no previous digest, and an entry travels
    /// whole through its encoding.
    #[test]
    fn an_entry_names_a_previous_digest_unless_it_is_the_first() {
        let mut first = entry();
        assert!(first.well_formed());
        first.position = 0;
        assert!(!first.well_formed());
        first.previous = None;
        assert!(first.well_formed());
        let sent = entry();
        assert_eq!(
            MembershipEntry::decode(&sent.encode().unwrap()).unwrap(),
            sent
        );
    }

    /// The signature closes the encoding as a length-prefixed byte string,
    /// the form a `Vec<u8>` wrote, so a stored entry keeps its bytes. A
    /// signature of another length does not decode.
    #[test]
    fn the_signature_keeps_its_length_prefixed_encoding() {
        let sent = entry().encode().unwrap();
        let tail = sent.len() - ENTRY_SIGNATURE_LEN - 1;
        assert_eq!(sent[tail], ENTRY_SIGNATURE_LEN as u8);
        assert_eq!(sent[tail + 1..], [7; ENTRY_SIGNATURE_LEN]);
        let mut short = sent[..tail].to_vec();
        short.push(ENTRY_SIGNATURE_LEN as u8 - 1);
        short.extend([7; ENTRY_SIGNATURE_LEN - 1]);
        assert!(MembershipEntry::decode(&short).is_err());
    }

    /// `SRV-227`: an entry reads from its own encoding alone. A byte past its
    /// end, and a position written in a longer varint, are refused.
    #[test]
    fn an_entry_reads_from_its_own_encoding_alone() {
        let sent = entry().encode().unwrap();
        let mut trailing = sent.clone();
        trailing.push(0);
        assert_eq!(
            MembershipEntry::decode(&trailing),
            Err(SerializationError::NotCanonical)
        );
        // The position follows the 16 bytes of the document id. Position 1 is
        // the varint `01`; `81 00` is the same value in a longer form.
        assert_eq!(sent[16], 0x01);
        let mut longer = sent[..16].to_vec();
        longer.extend([0x81, 0x00]);
        longer.extend(&sent[17..]);
        assert!(MembershipEntry::decode(&longer).is_err());
    }

    /// An ownerless log opens with its creation, and leaves the creator and
    /// every member an editor, with no owner.
    #[test]
    fn an_ownerless_log_leaves_every_member_an_editor() {
        let creator = UserId::new();
        let members = [UserId::new(), UserId::new()];
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
