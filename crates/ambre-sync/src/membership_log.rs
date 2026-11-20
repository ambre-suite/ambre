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
/// admission. [`Log::walk_under`] also requires the joiner present in the
/// space log, under the keys the self-join names (`SYNC-227`).
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
    /// The keys of the members the space holds present with a writing role
    /// at its latest head. Only these sign a key link of a channel at every
    /// head of it (audit `N29`).
    pub(crate) writing: HashSet<FixedBytes<PublicKey>>,
    /// The members a valid entry of the space log removed, and no later one
    /// added back.
    pub(crate) removed: HashSet<Id<User>>,
    /// Each present member of the space, under the keys the entry that added
    /// them named. A self-join of a channel counts only under them
    /// (`SYNC-227`).
    pub(crate) present: HashMap<Id<User>, MemberKeys>,
}

/// The signing key and the exchange key one entry named for a member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemberKeys {
    pub(crate) key: FixedBytes<PublicKey>,
    pub(crate) exchange: FixedBytes<ExchangePublicKey>,
}

impl SpaceWord {
    /// Whether `exchange` is the exchange key of a member the space holds
    /// present with a writing role at its latest head. Only such a member
    /// re-keys a channel it is no member of (audit `P57`).
    pub(crate) fn writes_under(&self, exchange: &FixedBytes<ExchangePublicKey>) -> bool {
        self.present
            .values()
            .any(|keys| keys.exchange == *exchange && self.writing.contains(&keys.key))
    }

    /// What the walked log of a space says. An unanchored log verifies
    /// nobody, so it grants no writer.
    pub(crate) fn of(log: &Log) -> Self {
        Self {
            writers: log.signers().cloned().unwrap_or_default(),
            writing: if log.anchored {
                log.held
                    .values()
                    .filter(|held| held.present && writing_rank(held.role).is_some())
                    .map(|held| held.key)
                    .collect()
            } else {
                HashSet::new()
            },
            removed: log
                .held
                .iter()
                .filter(|(_, held)| !held.present)
                .map(|(member, _)| *member)
                .collect(),
            present: log
                .held
                .iter()
                .filter(|(_, held)| held.present)
                .map(|(member, held)| {
                    let keys = MemberKeys {
                        key: held.key,
                        exchange: held.exchange,
                    };
                    (*member, keys)
                })
                .collect(),
        }
    }

    /// Whether the space holds the member a self-join adds present, under
    /// the signing key and the exchange key the self-join names (`SYNC-227`).
    /// The relay alone cannot sign a space entry, so it cannot add a member
    /// of its choice under a key of its own.
    pub(crate) fn admits_joiner(&self, entry: &MembershipEntry) -> bool {
        self.present.get(&entry.member).is_some_and(|keys| {
            keys.key == entry.member_key && keys.exchange == entry.member_exchange_key
        })
    }
}

/// The signing keys whose deltas and snapshots this client applies
/// (`SYNC-211`).
///
/// The set is every key the verified log granted a writing role, at any
/// point. The set only grows, so peers that hold one log give one delta one
/// verdict (`AUTHOR-007`). A demotion thus never erases history that one
/// peer applied before it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Writers {
    /// No anchored log: a local document, or one older than the log. Its
    /// deltas stand on their seal alone, as its links do (`SYNC-121`).
    #[default]
    Unchecked,
    /// The keys the walk granted a writing role.
    Granted(HashSet<FixedBytes<PublicKey>>),
}

impl Writers {
    /// What the walked `log` grants.
    pub(crate) fn of(log: &Log) -> Self {
        log.signers()
            .map_or(Self::Unchecked, |keys| Self::Granted(keys.clone()))
    }

    /// What the walked `log` grants under `rule`. A log a space lists, and
    /// whose creation no writer of that space signed, grants no writer
    /// (audit `N16`, `SYNC-230`). An empty word is a space this client has
    /// not walked yet, and decides nothing.
    pub(crate) fn under(log: &Log, rule: &LogRule) -> Self {
        match Self::of(log) {
            Self::Granted(_) if rule.root_of(log) == Root::Stranger => {
                Self::Granted(HashSet::new())
            }
            writers => writers,
        }
    }

    /// Whether a delta or a snapshot `signer` signed may reach the document.
    pub(crate) fn admits(&self, signer: &FixedBytes<PublicKey>) -> bool {
        match self {
            Self::Unchecked => true,
            Self::Granted(keys) => keys.contains(signer),
        }
    }

    /// Whether the writers `later`, which a refresh of the members walked,
    /// may replace these (`SYNC-214`). The set only grows, so a later walk
    /// keeps every key this one granted. A walk the relay re-rooted, or one
    /// it left unanchored by dropping our row, keeps none of them.
    pub(crate) fn grows_into(&self, later: &Self) -> bool {
        match (self, later) {
            (Self::Unchecked, _) => true,
            (Self::Granted(_), Self::Unchecked) => false,
            (Self::Granted(held), Self::Granted(walked)) => held.is_subset(walked),
        }
    }
}

/// The signing keys whose confirmation of a snapshot an open trusts
/// (`AUTHOR-030`).
///
/// The set is the key of each member the log holds present. A reader holds
/// the deltas as a writer does, so either may confirm. The relay cannot sign
/// an entry, so it cannot add a key of its own. A removed member, and a key
/// its member replaced, confirm nothing (`AUTHOR-035`).
///
/// Each key keeps the keys that signed a binding of it. A confirmer that an
/// editor bound does not count for that editor. Else the editor invites a
/// second account of their own to confirm their snapshot (`AUTHOR-034`,
/// audit `W203`). An owner's snapshot counts the confirmers the owner
/// bound: the owner ruled so on 2026-11-19 (`AUTHOR-036`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Confirmers {
    /// No anchored log: a local document, or one older than the log. A
    /// confirmation then stands on its signature alone.
    #[default]
    Unchecked,
    /// The keys of the present members, and the keys of the present owners.
    Granted(ConfirmerKeys),
}

/// What an anchored log grants a confirmer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConfirmerKeys {
    /// The key of each present member, with the keys that signed a binding
    /// of it, every binding kept.
    members: HashMap<FixedBytes<PublicKey>, HashSet<FixedBytes<PublicKey>>>,
    /// The key of each present member whose role is owner.
    owners: HashSet<FixedBytes<PublicKey>>,
}

impl Confirmers {
    /// What the walked `log` grants. The log is read at the head of its
    /// walk, which holds every removal up to the snapshot's horizon. A
    /// member removed after the horizon thus counts for no snapshot: the
    /// open fails closed and checks it after the replay.
    pub(crate) fn of(log: &Log) -> Self {
        if !log.anchored {
            return Self::Unchecked;
        }
        let present = || log.held.values().filter(|held| held.present);
        Self::Granted(ConfirmerKeys {
            members: present()
                .map(|held| {
                    let binders = log.binders.get(&held.key).cloned().unwrap_or_default();
                    (held.key, binders)
                })
                .collect(),
            owners: present()
                .filter(|held| held.role == Role::Owner)
                .map(|held| held.key)
                .collect(),
        })
    }

    /// The grant of a log whose present members hold `members`, each with
    /// the keys that bound it, and whose present owners hold `owners`.
    #[cfg(test)]
    pub(crate) fn granted(
        members: HashMap<FixedBytes<PublicKey>, HashSet<FixedBytes<PublicKey>>>,
        owners: HashSet<FixedBytes<PublicKey>>,
    ) -> Self {
        Self::Granted(ConfirmerKeys { members, owners })
    }

    /// Whether a confirmation `confirmer` signed may stand for a member other
    /// than `author`, the author of the snapshot. The log holds `confirmer`
    /// as a present member (`AUTHOR-030`, `AUTHOR-035`). `author` signed none
    /// of its bindings, unless `author` is a present owner (`AUTHOR-034`,
    /// `AUTHOR-036`).
    pub(crate) fn admits(
        &self,
        confirmer: &FixedBytes<PublicKey>,
        author: &FixedBytes<PublicKey>,
    ) -> bool {
        match self {
            Self::Unchecked => true,
            Self::Granted(keys) => keys.members.get(confirmer).is_some_and(|binders| {
                keys.owners.contains(author) || !binders.contains(author)
            }),
        }
    }
}

/// One stretch of the log where a key held a writing role. It holds every
/// head from the place `from` up to the place `until`, which it excludes.
/// An open stretch runs to the end of the walk.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WriterSpan {
    from: Seq<LogPlaces>,
    until: Option<Seq<LogPlaces>>,
}

impl WriterSpan {
    /// Whether the head at `place` falls inside the stretch.
    fn covers(&self, place: Seq<LogPlaces>) -> bool {
        self.from <= place && self.until.is_none_or(|until| place < until)
    }
}

/// Who may sign a key link, and at which head (`SYNC-121`, `SYNC-204`).
///
/// A link names the head of the log its rotator read. It counts only when
/// that head is one the valid walk passes through, and its signer held a
/// writing role there. So a writer the log revoked signs no link past its
/// revoke. A present writer of the space signs a link of a channel at any
/// head of it.
///
/// A removal from the space leaves no entry in the channel log, so no head
/// of the channel marks it. The stretch of such a signer therefore closes at
/// the last key a signer still in force built on (audit `N29`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum LinkSigners {
    /// No anchored log: a local document, or one older than the log. Its
    /// links stand on their seal alone.
    #[default]
    Unchecked,
    /// The stretches of the anchored log where each key could sign.
    Granted(WriterHistory),
}

/// The heads of the valid walk, and the writing stretches of each key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WriterHistory {
    /// Every head the valid walk passes through, the empty one first.
    heads: Vec<LogHead>,
    /// The stretches where each key held a writing role, oldest first.
    spans: HashMap<FixedBytes<PublicKey>, Vec<WriterSpan>>,
    /// The present writers of the space whose word governs this log.
    space: HashSet<FixedBytes<PublicKey>>,
    /// The keys whose stretch a removal from the space closed, and the
    /// heads where each one signed.
    closed: HashMap<FixedBytes<PublicKey>, Closed>,
}

/// Where a signer whose stretch a removal from the space closed signed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Closed {
    /// A former writer of the space: at every head of the channel.
    FormerSpaceWriter,
    /// A writer of this log the space removed: inside its own stretches.
    RemovedMember,
}

impl LinkSigners {
    /// Whether `signer` may sign a link that names `head`.
    pub(crate) fn signs(&self, signer: &FixedBytes<PublicKey>, head: LogHead) -> bool {
        match self {
            Self::Unchecked => true,
            Self::Granted(history) => {
                history.heads.iter().rev().any(|h| *h == head)
                    && (history.space.contains(signer)
                        || history.closed.get(signer) == Some(&Closed::FormerSpaceWriter)
                        || history
                            .spans
                            .get(signer)
                            .is_some_and(|spans| spans.iter().any(|s| s.covers(head.position))))
            }
        }
    }

    /// Whether `signer` still signs on its own. A removal from the space
    /// closes the stretch of a signer. Its link then counts only once a link
    /// still in force starts from the key it installs (audit `N29`).
    pub(crate) fn standing(&self, signer: &FixedBytes<PublicKey>) -> LinkStanding {
        match self {
            Self::Granted(history) if history.closed.contains_key(signer) => LinkStanding::Closed,
            Self::Unchecked | Self::Granted(_) => LinkStanding::InForce,
        }
    }
}

/// Whether a link a signer of the log signed stands on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkStanding {
    /// Its signer is still in force, or a link in force built on its key.
    InForce,
    /// A removal from the space closed the stretch of its signer, and no
    /// link in force built on its key yet (audit `N29`).
    Closed,
}

/// A document key a writer named in a valid entry, at the place of that
/// entry (`SYNC-200`, `SYNC-221`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NamedKey {
    pub(crate) place: Seq<LogPlaces>,
    pub(crate) key: FixedBytes<KeyId>,
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

    /// Where the walked `log` starts, against the space this rule heeds
    /// (audit `N16`). An empty word is a space this client has not walked
    /// yet, and decides nothing. A genuine space log grants its creator at
    /// the least.
    pub(crate) fn root_of(&self, log: &Log) -> Root {
        match &self.space {
            Some(word) if !word.writers.is_empty() => {
                if log.creator().is_some_and(|key| word.writers.contains(key)) {
                    Root::SpaceWriter
                } else {
                    Root::Stranger
                }
            }
            _ => Root::Free,
        }
    }
}

/// Where a log starts, against the space a catalogue lists it in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Root {
    /// No space governs the log, or this client holds no word of it yet.
    #[default]
    Free,
    /// A writer of the space signed the creation.
    SpaceWriter,
    /// No writer of the space signed the creation (audit `N16`). The log,
    /// its key and its deltas stand on no member's word.
    Stranger,
}

/// What the log says of one member once the walk applied its entries.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// The rank of `role` when it is a writing role: editor or above.
fn writing_rank(role: Role) -> Option<DocumentRank> {
    role.document_rank()
        .filter(|rank| *rank >= DocumentRank::Editor)
}

/// The digest of one entry, as the next entry names it. `None` for an entry
/// that does not encode, which no decoded entry is.
pub(crate) fn digest(entry: &MembershipEntry) -> Option<FixedBytes<EntryDigest>> {
    let encoded = entry.encode().ok()?;
    Some(ambre_crypto::hash::entry_digest(&encoded))
}

/// The place and the previous digest the next entry of a log takes. An
/// update carries it too (`SYNC-206`), so the type lives on the wire.
pub(crate) use ambre_protocol::LogHead;

/// The heads a served log passes through, one per entry it serves, in order.
/// The walk stops at an entry that fails a check, and these heads do not.
/// A peer signs the head of the log the relay served it, valid or not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ServedHeads {
    /// The digest of each served entry, from the first.
    digests: Vec<FixedBytes<EntryDigest>>,
    /// The last entry served: the one each update names beside its head
    /// (`SYNC-218`).
    last_entry: Option<MembershipEntry>,
    /// The members the walk holds. They decide whether an entry a head
    /// names past this log was signed by a member allowed to sign it
    /// (`SYNC-219`). Empty until a walk fills them.
    vouchers: HashMap<Id<User>, Held>,
    /// The heads the valid walk passes through, from the empty head. Only
    /// these clear a head under check (audit `W202`). Empty until a walk
    /// fills them.
    walked: Vec<LogHead>,
}

impl ServedHeads {
    /// The heads of `entries`, up to the first that does not encode, which
    /// no decoded entry is. No walk ran, so no member vouches for an entry.
    pub(crate) fn of(entries: &[MembershipEntry]) -> Self {
        let digests: Vec<_> = entries.iter().map_while(digest).collect();
        let last_entry = digests
            .len()
            .checked_sub(1)
            .and_then(|last| entries.get(last))
            .cloned();
        Self {
            digests,
            last_entry,
            vouchers: HashMap::new(),
            walked: Vec::new(),
        }
    }

    /// The entry the head of the whole served log names, absent on an empty
    /// log (`SYNC-218`).
    pub(crate) fn last_entry(&self) -> Option<&MembershipEntry> {
        self.last_entry.as_ref()
    }

    /// Whether `entry` proves `head`, a head of `document` (`SYNC-219`).
    /// The entry sits at the place before the head and hashes to its
    /// digest. Its author signed it, under an act this log allows that
    /// author. A self-join proves nothing: any user may sign one and keep
    /// it from the relay.
    pub(crate) fn vouches(
        &self,
        document: Id<Document>,
        head: LogHead,
        entry: Option<&MembershipEntry>,
    ) -> bool {
        proves(&self.vouchers, document, head, entry)
    }

    /// The head of the whole served log.
    pub(crate) fn last(&self) -> LogHead {
        LogHead {
            position: place_after(self.digests.len()),
            previous: self.digests.last().copied(),
        }
    }

    /// Whether the valid walk of the served log passes through `head`: the
    /// empty head, or the head after one of its valid entries. A served entry
    /// that the walk refused clears no head under check (`SYNC-207`, audit
    /// `W202`).
    pub(crate) fn walked_through(&self, head: LogHead) -> bool {
        head == LogHead::EMPTY || self.walked.contains(&head)
    }

    /// Whether the served log passes through `head`: the empty head, or an
    /// entry at the place before it with its digest (`SYNC-207`). A head
    /// past the served log, or one that forks from it, is not held. The
    /// search starts at the end, where a current peer's head sits.
    pub(crate) fn holds(&self, head: LogHead) -> bool {
        match head.previous {
            None => head.is_empty(),
            Some(previous) => self
                .digests
                .iter()
                .enumerate()
                .rev()
                .any(|(place, d)| *d == previous && place_after(place).next() == head.position),
        }
    }
}

/// One document's log, as far as a chain of valid entries reaches.
pub(crate) struct Log {
    held: HashMap<Id<User>, Held>,
    /// Every key the log granted a writing role, at any point.
    signers: HashSet<FixedBytes<PublicKey>>,
    /// Where each key held a writing role, head by head (`SYNC-204`).
    history: WriterHistory,
    /// Every document key a valid entry names while its author writes. Each
    /// key sits at the place of its entry, in log order (`SYNC-200`,
    /// `SYNC-221`).
    named_at: Vec<NamedKey>,
    /// The place of the latest valid entry by which a member left on their
    /// own (`SYNC-222`).
    departed: Option<Seq<LogPlaces>>,
    /// The latest revoke a writer signed of another member, with the key it
    /// installed (`SYNC-224`, `SYNC-225`).
    revoked: Option<NamedKey>,
    /// Every key that signed a valid entry (`SYNC-205`).
    authors: HashSet<FixedBytes<PublicKey>>,
    /// The member each signing key a valid entry named was bound to, the
    /// latest binding kept (`SYNC-215`).
    bound: HashMap<FixedBytes<PublicKey>, Id<User>>,
    /// The keys that signed a binding of each key, every binding kept
    /// (`AUTHOR-034`).
    binders: HashMap<FixedBytes<PublicKey>, HashSet<FixedBytes<PublicKey>>>,
    /// The key that signed the valid first entry, the creation.
    creator: Option<FixedBytes<PublicKey>>,
    /// Whether the wrapped key carried an anchor at all.
    anchored: bool,
    /// The anchor the walk started from.
    anchor: Option<FixedBytes<EntryDigest>>,
    /// The place and the previous digest the next entry takes.
    next: LogHead,
    /// The heads of every entry served, valid or not.
    heads: ServedHeads,
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
            history: WriterHistory {
                heads: vec![LogHead::EMPTY],
                ..WriterHistory::default()
            },
            named_at: Vec::new(),
            departed: None,
            revoked: None,
            authors: HashSet::new(),
            bound: HashMap::new(),
            binders: HashMap::new(),
            creator: None,
            anchored: anchor.is_some(),
            anchor,
            next: LogHead {
                position: place_after(entries.len()),
                previous: entries.last().and_then(digest),
            },
            heads: ServedHeads::of(entries),
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
                let named = NamedKey {
                    place: entry.position,
                    key,
                };
                log.named_at.push(named);
                // A revoke installs the key it names, and the revoked member
                // holds every key before it (`SYNC-224`).
                if entry.act == MembershipAct::Revoke {
                    log.revoked = Some(named);
                }
            }
            if let Some(key) = log.signer_of(entry) {
                if previous.is_none() {
                    log.creator = Some(key);
                }
                log.authors.insert(key);
                // Who bound the key this entry names, read before the entry
                // applies (`AUTHOR-034`).
                if matches!(
                    entry.act,
                    MembershipAct::Create | MembershipAct::Invite | MembershipAct::SelfJoin
                ) {
                    log.binders.entry(entry.member_key).or_default().insert(key);
                }
            }
            // A revoke its member signed is a departure (`SYNC-127`).
            if entry.act == MembershipAct::Revoke && entry.author == entry.member {
                log.departed = Some(entry.position);
            }
            log.apply(entry);
            previous = this;
            if let Some(this) = this {
                log.history.heads.push(LogHead {
                    position: entry.position.next(),
                    previous: Some(this),
                });
            }
        }
        log
    }

    /// [`Log::walk`] under `rule`, then what the space says. A self-join
    /// counts only for a present member of the space, under the keys the
    /// space log binds (`SYNC-227`). A space writer may sign a key link of
    /// the channel (the Murmure spaces spec). A member the space removed
    /// reads as removed here too, though no entry of this log says so.
    pub(crate) fn walk_under(
        document: Id<Document>,
        anchor: Option<FixedBytes<EntryDigest>>,
        entries: &[MembershipEntry],
        rule: &LogRule,
    ) -> Self {
        // A self-join the space does not admit ends the chain, as any invalid
        // entry does (`SYNC-227`). No space word admits none: the space log
        // is not at hand, so the check fails closed.
        let reach = entries
            .iter()
            .position(|entry| {
                entry.act == MembershipAct::SelfJoin
                    && !rule
                        .space
                        .as_ref()
                        .is_some_and(|space| space.admits_joiner(entry))
            })
            .unwrap_or(entries.len());
        let mut log = Self::walk(
            document,
            anchor,
            entries.get(..reach).unwrap_or(entries),
            rule.self_join,
        );
        // The relay appends at its own end, past every entry served.
        log.next = LogHead {
            position: place_after(entries.len()),
            previous: entries.last().and_then(digest),
        };
        log.heads = ServedHeads::of(entries);
        if let Some(space) = &rule.space {
            log.signers.extend(space.writers.iter().cloned());
            // Only a present writer of the space signs at every head. A
            // former one, and a writer of this log the space removed, keep a
            // closed stretch (audit `N29`).
            log.history.space.extend(space.writing.iter().cloned());
            for member in &space.removed {
                if let Some(held) = log.held.get_mut(member) {
                    held.present = false;
                    log.history.closed.insert(held.key, Closed::RemovedMember);
                }
            }
            for key in space.writers.difference(&space.writing) {
                log.history.closed.insert(*key, Closed::FormerSpaceWriter);
            }
            for key in &space.writing {
                log.history.closed.remove(key);
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
        allowed(&self.held, entry, self_join)
    }

    /// The key the valid `entry` names, when its author held a writing role
    /// as they signed it. A creator does; a self-joiner signs for themselves
    /// alone, so a self-join names no key a writer chose (`SYNC-200`). A
    /// departure names none either: the leaver would choose the key the
    /// members who stay keep (`SYNC-130`, `SYNC-222`).
    fn writer_names(&self, entry: &MembershipEntry) -> Option<FixedBytes<KeyId>> {
        let rank = match entry.act {
            MembershipAct::Create => Some(DocumentRank::Owner),
            MembershipAct::SelfJoin => None,
            MembershipAct::Revoke if entry.author == entry.member => None,
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

    /// The key that signed the valid `entry`, read before it applies: the
    /// member's own for a creation and a self-join, the author's otherwise.
    fn signer_of(&self, entry: &MembershipEntry) -> Option<FixedBytes<PublicKey>> {
        match entry.act {
            MembershipAct::Create | MembershipAct::SelfJoin => Some(entry.member_key),
            MembershipAct::Invite | MembershipAct::ChangeRole | MembershipAct::Revoke => {
                self.held.get(&entry.author).map(|held| held.key)
            }
        }
    }

    /// The signing key of `member` while the log keeps them a writer.
    fn writer_key(&self, member: Id<User>) -> Option<FixedBytes<PublicKey>> {
        self.held
            .get(&member)
            .filter(|held| held.present && writing_rank(held.role).is_some())
            .map(|held| held.key)
    }

    /// Applies one valid entry to the state. A change of who writes takes
    /// effect at the head after the entry (`SYNC-204`).
    fn apply(&mut self, entry: &MembershipEntry) {
        let before = self.writer_key(entry.member);
        self.apply_act(entry);
        let after = self.writer_key(entry.member);
        if before != after {
            let place = entry.position.next();
            if let Some(span) = before
                .and_then(|key| self.history.spans.get_mut(&key))
                .and_then(|spans| spans.last_mut())
                .filter(|span| span.until.is_none())
            {
                span.until = Some(place);
            }
            if let Some(key) = after {
                self.history.spans.entry(key).or_default().push(WriterSpan {
                    from: place,
                    until: None,
                });
            }
        }
    }

    /// Applies the act of one valid entry to the state.
    fn apply_act(&mut self, entry: &MembershipEntry) {
        match entry.act {
            MembershipAct::Create | MembershipAct::Invite | MembershipAct::SelfJoin => {
                self.bound.insert(entry.member_key, entry.member);
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

    /// The key that signed the creation, when the walk reached it.
    pub(crate) fn creator(&self) -> Option<&FixedBytes<PublicKey>> {
        self.creator.as_ref()
    }

    /// Who may sign a key link, head by head (`SYNC-204`). Unchecked for a
    /// document whose wrap carried no anchor, which predates the log.
    pub(crate) fn link_signers(&self) -> LinkSigners {
        if self.anchored {
            LinkSigners::Granted(self.history.clone())
        } else {
            LinkSigners::Unchecked
        }
    }

    /// The document keys a writer named in a valid entry, each at the place
    /// of that entry, in log order (`SYNC-200`, `SYNC-221`). `None` for a
    /// document whose wrap carried no anchor: no entry of it is checked, so
    /// none names a key.
    pub(crate) fn named_at(&self) -> Option<&[NamedKey]> {
        self.anchored.then_some(self.named_at.as_slice())
    }

    /// The place of the latest departure the log holds: a valid entry by
    /// which a member left on their own (`SYNC-222`). The leaver holds every
    /// key in force before it, and the debt it leaves is the relay's word.
    pub(crate) fn departed_at(&self) -> Option<Seq<LogPlaces>> {
        self.departed
    }

    /// The latest revoke a writer signed of another member, and the key it
    /// installed (`SYNC-224`). The revoked member holds every key before
    /// that one. Only that key, or a key the chain leads back to it,
    /// seals for the members who stay.
    pub(crate) fn revoked_at(&self) -> Option<NamedKey> {
        self.revoked
    }

    /// Whether a valid entry of an anchored log was signed by the key `pin`
    /// names (`SYNC-205`). The relay cannot forge that signature, and the
    /// entry names the digest before it back to the first. So the anchor
    /// the walk started from is the one the pinned inviter's log starts from.
    pub(crate) fn signed_under(&self, pin: &FixedBytes<ambre_core::InviterPin>) -> bool {
        self.anchored
            && self
                .authors
                .iter()
                .any(|key| ambre_crypto::inviter_pin(key) == *pin)
    }

    /// Whether the log lets `key` stand for `member` in the directory that
    /// names the author of a delta or a presence sample (`SYNC-215`). A
    /// valid entry must have bound that key to that member. An unanchored
    /// log checks nothing, and lets every row of the relay stand.
    pub(crate) fn binds(&self, key: &FixedBytes<PublicKey>, member: Id<User>) -> bool {
        !self.anchored || self.bound.get(key) == Some(&member)
    }

    /// The pins of the signing keys of every member the log keeps: the roots
    /// an accepted Murmure log adds to the session (`SYNC-237`). An
    /// unanchored log verifies nobody, so it adds none.
    pub(crate) fn member_pins(&self) -> Vec<FixedBytes<ambre_core::InviterPin>> {
        if !self.anchored {
            return Vec::new();
        }
        self.held
            .values()
            .filter(|held| held.present)
            .map(|held| ambre_crypto::inviter_pin(&held.key))
            .collect()
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

    /// Whether a valid entry signed `key` as the exchange key of a member the
    /// log keeps (`SYNC-201`). The relay serves the exchange key of each row
    /// outside any signature, so a wrapper check reads this answer instead.
    pub(crate) fn signs_exchange_key(&self, key: &FixedBytes<ExchangePublicKey>) -> bool {
        self.held
            .values()
            .any(|held| held.present && held.exchange == *key)
    }

    /// The key the entry that added `member` wraps, when the log reaches it.
    pub(crate) fn wraps(&self, member: Id<User>) -> Option<FixedBytes<KeyId>> {
        self.held
            .get(&member)
            .filter(|held| held.present)
            .and_then(|held| held.wraps)
    }

    /// Whether the log binds `key` to a member other than `member`. An
    /// invite of `member` under that key is refused, since no valid entry
    /// could carry it (audit `P58`).
    pub(crate) fn binds_elsewhere(&self, member: Id<User>, key: &FixedBytes<PublicKey>) -> bool {
        key_held_elsewhere(&self.held, member, key)
    }

    /// The anchor the walk started from.
    pub(crate) fn anchor(&self) -> Option<FixedBytes<EntryDigest>> {
        self.anchor
    }

    /// The head after the last valid entry of the walk: the empty head when
    /// no entry is valid (audit `P27`).
    pub(crate) fn walked_head(&self) -> LogHead {
        self.history.heads.last().copied().unwrap_or(LogHead::EMPTY)
    }

    /// Whether the valid walk passes through `head`: the empty head, or the
    /// head after one of its valid entries (audit `P27`).
    pub(crate) fn walked_through(&self, head: LogHead) -> bool {
        self.history.heads.contains(&head)
    }

    /// The place and the previous digest the next entry takes. They follow
    /// every entry served, valid or not: the relay appends at its own end.
    pub(crate) fn next(&self) -> LogHead {
        self.next
    }

    /// Whether `head` lies past this log, and `entry` proves it: a member
    /// this log allows signed the entry the head names (`SYNC-219`,
    /// `CALL-137`). The served heads are read in place, with no copy.
    pub(crate) fn proves_past(
        &self,
        document: Id<Document>,
        head: LogHead,
        entry: Option<&MembershipEntry>,
    ) -> bool {
        !self.heads.holds(head) && proves(&self.held, document, head, entry)
    }

    /// Whether this log passes through the head of `earlier`: it holds every
    /// entry `earlier` served, and maybe more (`CALL-138`). A log the relay
    /// rolled back, or forked, does not.
    pub(crate) fn extends(&self, earlier: &Self) -> bool {
        self.heads.holds(earlier.next)
    }

    /// The heads of every entry served, which a signed head is checked
    /// against (`SYNC-207`), and the members that vouch for an entry past
    /// them (`SYNC-219`).
    pub(crate) fn heads(&self) -> ServedHeads {
        ServedHeads {
            vouchers: self.held.clone(),
            walked: self.history.heads.clone(),
            ..self.heads.clone()
        }
    }
}

/// Whether `entry` proves `head`, a head of `document`, against `vouchers`,
/// the members a walk holds (`SYNC-219`).
fn proves(
    vouchers: &HashMap<Id<User>, Held>,
    document: Id<Document>,
    head: LogHead,
    entry: Option<&MembershipEntry>,
) -> bool {
    entry.is_some_and(|entry| {
        entry.document == document
            && entry.position.next() == head.position
            && entry.well_formed()
            && digest(entry).is_some_and(|d| head.previous == Some(d))
            && allowed(vouchers, entry, no_self_join)
    })
}

/// Whether a member other than `member`, among `held`, holds `key` (audit
/// `P58`).
fn key_held_elsewhere(
    held: &HashMap<Id<User>, Held>,
    member: Id<User>,
    key: &FixedBytes<PublicKey>,
) -> bool {
    held.iter()
        .any(|(user, other)| *user != member && other.key == *key)
}

/// Whether `held`, the members a walk holds, allows `entry`, and its author
/// signed it.
fn allowed(
    held: &HashMap<Id<User>, Held>,
    entry: &MembershipEntry,
    self_join: SelfJoinRule,
) -> bool {
    let author = held.get(&entry.author).filter(|held| held.present);
    let target = held.get(&entry.member).filter(|held| held.present);
    let author_rank = author.and_then(|held| held.role.document_rank());
    let granted = entry.role.document_rank();
    let permitted = match entry.act {
        MembershipAct::Create => false,
        // An editor or an owner adds a member at or below their own rank.
        // Only an owner re-invites a present member, as the relay checks
        // (`SRV-266`): the re-invite replaces that member's keys
        // (`SYNC-217`).
        MembershipAct::Invite => match (author_rank, granted) {
            (Some(by), Some(role)) => {
                by >= DocumentRank::Editor
                    && role <= by
                    && (target.is_none() || by == DocumentRank::Owner)
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
        // A self-join signs under the key it names itself, so it never
        // replaces a present member's keys (`SYNC-217`).
        MembershipAct::SelfJoin => {
            entry.author == entry.member
                && target.is_none()
                && granted.is_some()
                && self_join(entry)
        }
    };
    // A key binds one member alone. An entry that binds the key of another
    // member would make the latest binding win, and drop that member's
    // deltas from the directory (audit `P58`).
    let binds = matches!(entry.act, MembershipAct::Invite | MembershipAct::SelfJoin);
    let taken = binds && key_held_elsewhere(held, entry.member, &entry.member_key);
    let key = match entry.act {
        MembershipAct::SelfJoin => Some(&entry.member_key),
        _ => author.map(|held| &held.key),
    };
    permitted && !taken && key.is_some_and(|key| signed_by(entry, key))
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
    use ambre_core::Seq;
    use ambre_protocol::LogPlaces;

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
        for (seed, (member, exchange)) in (100u8..).zip(members) {
            let next = LogHead {
                position: place_after(log.len()),
                previous: log.last().and_then(digest),
            };
            // A key binds one member alone (audit `P58`).
            let draft = Draft {
                document: DOC,
                member: *member,
                member_key: Person::new(seed).public(),
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

    /// The walk of `log`, a log of [`DOC`] rooted at its own first entry.
    pub(crate) fn walk(log: &[MembershipEntry]) -> Log {
        Log::walk(DOC, anchor(log), log, no_self_join)
    }

    /// `AUTHOR-030`: an anchored log grants the confirmation of a snapshot
    /// to the keys it bound to a member, and to no other key. A log with no
    /// anchor checks no confirmer.
    #[test]
    fn an_anchored_log_grants_confirmation_to_its_members_alone() {
        let (alice, bob, stranger) = (Person::new(1), Person::new(2), Person::new(9));
        let entries = created(&alice, &bob);
        let granted = Confirmers::of(&walk(&entries));
        // The author is a member who bound no one.
        let author = Person::new(5).public();
        assert!(granted.admits(&alice.public(), &author));
        assert!(granted.admits(&bob.public(), &author));
        assert!(!granted.admits(&stranger.public(), &author));
        let unanchored = Confirmers::of(&Log::walk(DOC, None, &entries, no_self_join));
        assert_eq!(unanchored, Confirmers::Unchecked);
    }

    /// `SYNC-282`, audit `P58`: an editor signs an invite of a third user
    /// under the signing key of Alice. The walk stops at the entry, so
    /// Alice's key stays bound to Alice, and an invite under a key of its
    /// own still walks.
    #[test]
    fn a_key_bound_to_one_member_binds_no_other() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let entries = created(&alice, &bob);
        let mut stolen = entries.clone();
        let next = LogHead {
            position: place_after(stolen.len()),
            previous: stolen.last().and_then(digest),
        };
        let draft = Draft {
            document: DOC,
            member: carol.id,
            member_key: alice.public(),
            member_exchange_key: FixedBytes::from_array([3; 32]),
            act: MembershipAct::Invite,
            role: Role::Viewer,
            key: None,
        };
        stolen.push(sign(draft, next, bob.id, &bob.key).unwrap());
        let log = walk(&stolen);
        assert_eq!(
            log.walked_head(),
            walk(&entries).walked_head(),
            "the walk stops"
        );
        assert!(log.binds_elsewhere(carol.id, &alice.public()));
        assert!(!log.binds_elsewhere(alice.id, &alice.public()));
        let mut honest = entries;
        append(
            &mut honest,
            &bob,
            &carol,
            MembershipAct::Invite,
            Role::Viewer,
        );
        assert_ne!(
            walk(&honest).walked_head(),
            log.walked_head(),
            "carol's own key walks"
        );
    }

    /// `AUTHOR-034`, audit `W203`: Mallory, an editor, invites a second
    /// account of hers. Its key confirms no snapshot Mallory authored, and
    /// still confirms one Alice authored.
    #[test]
    fn a_confirmer_the_author_bound_does_not_count_for_that_author() {
        let (alice, bob, mallory, guest) = (
            Person::new(1),
            Person::new(2),
            Person::new(3),
            Person::new(4),
        );
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &mallory,
            MembershipAct::Invite,
            Role::Editor,
        );
        append(
            &mut entries,
            &mallory,
            &guest,
            MembershipAct::Invite,
            Role::Viewer,
        );
        let granted = Confirmers::of(&walk(&entries));
        assert!(!granted.admits(&guest.public(), &mallory.public()));
        assert!(granted.admits(&guest.public(), &alice.public()));
        assert!(granted.admits(&bob.public(), &mallory.public()));
    }

    /// `AUTHOR-036`, the owner's ruling of 2026-11-19: Alice, the owner,
    /// invited Bob, and Bob's key confirms a snapshot Alice authored. An
    /// owner demoted to editor loses the exemption.
    #[test]
    fn an_owner_counts_the_confirmers_the_owner_invited() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Owner,
        );
        assert!(Confirmers::of(&walk(&entries)).admits(&bob.public(), &alice.public()));
        append(
            &mut entries,
            &carol,
            &alice,
            MembershipAct::ChangeRole,
            Role::Editor,
        );
        let demoted = Confirmers::of(&walk(&entries));
        assert_eq!(walk(&entries).walked_head().position, place_after(4));
        assert!(
            !demoted.admits(&bob.public(), &alice.public()),
            "Alice, an editor now, bound Bob"
        );
        assert!(demoted.admits(&bob.public(), &carol.public()));
    }

    /// `AUTHOR-035`: a member the log removed confirms no snapshot, whoever
    /// authored it. A member still present does.
    #[test]
    fn a_removed_member_confirms_nothing() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Editor,
        );
        append(&mut entries, &alice, &bob, MembershipAct::Revoke, Role::Editor);
        let log = walk(&entries);
        assert_eq!(log.walked_head().position, place_after(4), "the revoke walks");
        let granted = Confirmers::of(&log);
        assert!(!granted.admits(&bob.public(), &carol.public()));
        assert!(granted.admits(&carol.public(), &alice.public()));
    }

    /// A present space writer signs a key link of a channel at each head the
    /// walk passes through, at no head past the walk. The empty head before
    /// the first entry counts as one.
    #[test]
    fn a_space_writer_signs_a_link_at_each_head_of_the_walk() {
        let (alice, bob, governor) = (Person::new(1), Person::new(2), Person::new(9));
        let entries = created(&alice, &bob);
        let mut word = SpaceWord::default();
        word.writers.insert(governor.public());
        word.writing.insert(governor.public());
        let rule = LogRule {
            space: Some(word),
            ..LogRule::document()
        };
        let log = Log::walk_under(DOC, anchor(&entries), &entries, &rule);
        let signers = log.link_signers();
        assert!(signers.signs(&governor.public(), LogHead::EMPTY));
        assert!(signers.signs(&governor.public(), log.next()));
        let past = LogHead {
            position: log.next().position.next(),
            previous: log.next().previous,
        };
        assert!(!signers.signs(&governor.public(), past));
        assert!(!signers.signs(&bob.public(), LogHead::EMPTY));
        assert_eq!(
            signers.standing(&governor.public()),
            LinkStanding::InForce,
            "his stretch is open"
        );
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
        let walked: Option<HashSet<_>> = view
            .named_at()
            .map(|named| named.iter().map(|n| n.key).collect());
        assert_eq!(walked, Some(named));
        assert_eq!(Log::walk(DOC, None, &log, |_| true).named_at(), None);
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

    /// Holds `SYNC-211` at the walk: the writers are the keys a valid entry
    /// granted a writing role, at any point. A viewer never is one, a writer
    /// removed or demoted stays one, and a log with no anchor checks nobody.
    #[test]
    fn a_writer_once_granted_stays_a_writer_and_a_viewer_never_is() {
        let (alice, bob, carol, dave) = (
            Person::new(1),
            Person::new(2),
            Person::new(3),
            Person::new(4),
        );
        let mut log = created(&alice, &bob);
        append(
            &mut log,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Viewer,
        );
        append(
            &mut log,
            &alice,
            &bob,
            MembershipAct::ChangeRole,
            Role::Viewer,
        );
        append(&mut log, &alice, &bob, MembershipAct::Revoke, Role::Viewer);
        let writers = Writers::of(&walk(&log));
        assert!(writers.admits(&alice.public()), "the owner");
        assert!(
            writers.admits(&bob.public()),
            "a writer demoted, then removed"
        );
        assert!(!writers.admits(&carol.public()), "a viewer");
        assert!(!writers.admits(&dave.public()), "a key the log never named");
        // An entry the walk refuses grants nothing: a viewer cannot raise
        // themselves.
        append(
            &mut log,
            &carol,
            &carol,
            MembershipAct::ChangeRole,
            Role::Editor,
        );
        assert!(!Writers::of(&walk(&log)).admits(&carol.public()));
        let unanchored = Log::walk(DOC, None, &log, no_self_join);
        assert_eq!(Writers::of(&unanchored), Writers::Unchecked);
        assert!(Writers::Unchecked.admits(&dave.public()));
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

    /// Holds `SYNC-227` (audit `N24`). A channel self-join counts only for a
    /// present member of the space, under the signing key and the exchange
    /// key the space log binds. The relay can append a self-join under a key
    /// of its own. That self-join stops the walk: no rotation wraps for it,
    /// no call admits it, and the entries after it verify nothing.
    #[test]
    fn a_self_join_counts_only_for_a_present_member_of_the_space_under_its_keys() {
        let (alice, bob, carol, dave, mallory) = (
            Person::new(1),
            Person::new(2),
            Person::new(3),
            Person::new(4),
            Person::new(9),
        );
        // The space: Alice created it, invited Dave and Bob, then removed Bob.
        let mut space = Vec::new();
        append(
            &mut space,
            &alice,
            &alice,
            MembershipAct::Create,
            Role::Owner,
        );
        append(
            &mut space,
            &alice,
            &dave,
            MembershipAct::Invite,
            Role::Viewer,
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
            &bob,
            MembershipAct::Revoke,
            Role::Viewer,
        );
        let word = SpaceWord::of(&walk(&space));
        let rule = |space: Option<SpaceWord>| LogRule {
            self_join: public_channel_self_join,
            space,
        };
        // The channel: Carol created it, `member` joins under `keys`, signed
        // by `signer`, then Carol invites Eve, whose row stands past the join.
        let eve = Person::new(5);
        let channel = |member: &Person, keys: MemberKeys, signer: &Person| {
            let mut log = Vec::new();
            append(&mut log, &carol, &carol, MembershipAct::Create, Role::Owner);
            let mut joined = draft(member, MembershipAct::SelfJoin, Role::Editor);
            joined.member_key = keys.key;
            joined.member_exchange_key = keys.exchange;
            let next = LogHead {
                position: place_after(log.len()),
                previous: log.last().and_then(digest),
            };
            log.push(sign(joined, next, member.id, &signer.key).unwrap());
            append(&mut log, &carol, &eve, MembershipAct::Invite, Role::Viewer);
            log
        };
        let own = |p: &Person| MemberKeys {
            key: p.public(),
            exchange: p.exchange(),
        };

        // Dave, present in the space under his own keys, joins.
        let genuine = channel(&dave, own(&dave), &dave);
        let view = Log::walk_under(DOC, anchor(&genuine), &genuine, &rule(Some(word.clone())));
        assert_eq!(view.present_key(dave.id), Some(dave.public()));
        assert_eq!(view.exchange_key(dave.id), Some(dave.exchange()));
        assert_eq!(view.standing(eve.id).0, MemberStanding::Verified);

        let refused = [
            // The relay names Dave under its own keys.
            (
                "a key of the relay",
                channel(&dave, own(&mallory), &mallory),
                Some(word.clone()),
                dave.id,
            ),
            // Dave's signing key, the relay's exchange key.
            (
                "a swapped exchange key",
                channel(
                    &dave,
                    MemberKeys {
                        key: dave.public(),
                        exchange: mallory.exchange(),
                    },
                    &dave,
                ),
                Some(word.clone()),
                dave.id,
            ),
            // A member of the relay's choice, whom the space never held.
            (
                "no member of the space",
                channel(&mallory, own(&mallory), &mallory),
                Some(word.clone()),
                mallory.id,
            ),
            // Bob, whom the space removed, under his own keys.
            (
                "a removed member",
                channel(&bob, own(&bob), &bob),
                Some(word.clone()),
                bob.id,
            ),
            // No space word at hand: the check fails closed.
            ("no space word", genuine.clone(), None, dave.id),
        ];
        for (case, log, space, member) in refused {
            let view = Log::walk_under(DOC, anchor(&log), &log, &rule(space));
            assert_eq!(
                view.standing(member).0,
                MemberStanding::Unverified,
                "{case}"
            );
            assert_eq!(view.present_key(member), None, "{case}: no call admits it");
            assert_eq!(
                view.exchange_key(member),
                None,
                "{case}: no rotation wraps for it"
            );
            assert_eq!(
                view.standing(eve.id).0,
                MemberStanding::Unverified,
                "{case}"
            );
            // The relay appends at its own end, past every entry served.
            assert_eq!(
                view.next(),
                Log::walk(DOC, anchor(&log), &log, no_self_join).next(),
                "{case}"
            );
            assert!(view.heads().holds(view.next()), "{case}");
        }
    }

    /// Holds `SYNC-217`: no entry but an owner's replaces the keys of a
    /// present member. An editor's re-invite and a self-join that name new
    /// keys for present Bob stop the walk, and Bob keeps the keys his invite
    /// named.
    #[test]
    fn no_entry_but_an_owners_replaces_a_present_members_keys() {
        let (alice, bob, carol, mallory) = (
            Person::new(1),
            Person::new(2),
            Person::new(3),
            Person::new(9),
        );
        let mut base = created(&alice, &bob);
        append(
            &mut base,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Editor,
        );
        // Carol, an editor, re-invites Bob under Mallory's keys.
        let mut by_editor = base.clone();
        let mut swapped = draft(&bob, MembershipAct::Invite, Role::Editor);
        swapped.member_key = mallory.public();
        swapped.member_exchange_key = mallory.exchange();
        append_draft(&mut by_editor, &carol, swapped);
        // A self-join for Bob, signed under the key it names itself.
        let mut joined = base.clone();
        let mut forged = draft(&bob, MembershipAct::SelfJoin, Role::Editor);
        forged.member_key = mallory.public();
        forged.member_exchange_key = mallory.exchange();
        let next = LogHead {
            position: place_after(joined.len()),
            previous: joined.last().and_then(digest),
        };
        joined.push(sign(forged, next, bob.id, &mallory.key).unwrap());
        for log in [&by_editor, &joined] {
            let view = Log::walk(DOC, anchor(log), log, |_| true);
            assert_eq!(view.present_key(bob.id), Some(bob.public()));
            assert_eq!(view.exchange_key(bob.id), Some(bob.exchange()));
            assert!(!view.signers().unwrap().contains(&mallory.public()));
        }
        // An owner's re-invite of a present member still lands, under keys
        // no other member holds (`SYNC-282`).
        let renewal = Person::new(4);
        let mut by_owner = base.clone();
        let mut renewed = draft(&bob, MembershipAct::Invite, Role::Editor);
        renewed.member_key = renewal.public();
        renewed.member_exchange_key = renewal.exchange();
        append_draft(&mut by_owner, &alice, renewed);
        let view = walk(&by_owner);
        assert_eq!(view.present_key(bob.id), Some(renewal.public()));
        assert_eq!(view.exchange_key(bob.id), Some(renewal.exchange()));
    }
}
