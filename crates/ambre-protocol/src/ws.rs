// ============================================================================
// WebSocket protocol -- single multiplexed connection per client session.
//
// Endpoint: GET /api/ws -> upgrade WebSocket
// One connection carries messages for ALL open documents. Demux by
// document_id in each message. A Murmure channel travels as its documents and
// has no wire message of its own (`SRV-102`).
// ============================================================================

use crate::messages::{DeltaAuthor, Relay, Seq, Signature};
use crate::version::CURRENT_VERSION;
use crate::{Ephemeral, SeqUpdate, Update};
use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{
    Bounded, Bounds, DocType, FixedBytes, KeyId, Ms, Payload, PublicKey, Refusal, RefusalCode,
    TimeSpan,
};
use serde::{Deserialize, Serialize};

/// The relay's throttle window on replays: it answers nothing to a second
/// [`WsClientMsg::RequestMissing`] of one document on one connection inside
/// it (`SRV-086`). The relay and the Sync worker both read this one value.
/// The worker spaces its asks past it, and a const assertion there fails the
/// build when the window outgrows that spacing (`SYNC-191`).
pub const REPLAY_MIN_INTERVAL: TimeSpan<Ms> = TimeSpan::<Ms>::new(1_000);

// ---------------------------------------------------------------------------
// Client -> Server
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug)]
pub enum WsClientMsg {
    /// Start receiving real-time events for a document.
    /// Server adds this doc to the connection's subscription set.
    /// Server responds with Subscribed { document_id }.
    Subscribe { document_id: Id<Document> },

    /// Stop receiving events for a document.
    /// Server removes this doc from the subscription set.
    Unsubscribe { document_id: Id<Document> },

    /// Encrypted CRDT delta. document_id is inside update.public_data.
    Update(Update),

    /// Encrypted ephemeral (cursor/presence). document_id is inside public_data.
    Ephemeral(Ephemeral),

    /// Request missing updates for a specific document after a known seq.
    RequestMissing {
        document_id: Id<Document>,
        after_seq: Seq<Relay>,
    },

    /// The access token, as the first frame of the socket. The socket address
    /// carries none, so no proxy record and no log line holds it
    /// (`ambre-spec-session.md`, `SRV-031`). The relay reads nothing else
    /// until this frame is accepted.
    Authenticate { token: Credential<AccessToken> },

    /// The author's answer to a [`WsServerMsg::ClockRefused`], naming the
    /// refused delta by its signature. The relay drops this socket's later
    /// deltas of the document until it reads this answer to the delta that
    /// set the fence (`ambre-spec-sequencer.md`, `SRV-169`).
    ClockResync {
        document_id: Id<Document>,
        signature: FixedBytes<Signature>,
    },
}

/// The session tokens on the wire. Each holds its text in an
/// [`ambre_core::Secret`], so `Debug` prints no byte of it and a logged frame
/// logs no token (`SRV-031`). They live in the Secret module of `ambre-core`,
/// whose hand-written serde keeps the bare text this wire carried before.
pub use ambre_core::{AccessToken, Credential, RefreshToken};

/// The code of a WebSocket close frame, 1000 to 4999 (RFC 6455 section 7.4):
/// the marker of a [`Bounded<CloseCode>`]. The relay closes a refused upgrade
/// with a code of the application range, and Sync reads the same constants,
/// so the two never drift. A socket that failed without a close has no code:
/// Sync carries it as `None`.
pub enum CloseCode {}

impl Bounds for CloseCode {
    type Repr = u16;
    const MIN: u16 = 1000;
    const MAX: u16 = 4999;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CloseCode");
    const REFUSAL: Refusal = Refusal::new("a close code lies in 1000..=4999, not ", "");
}

/// The codes the relay closes a refused upgrade with, and the reason text it
/// sends beside each. A trait, since a crate adds no inherent item to
/// [`Bounded`], which `ambre-core` owns.
pub trait RelayClose: Sized {
    /// An upgrade refused on authentication (expired or rotated token): the
    /// client renews its token and reconnects (`SRV-046`). Mirrors HTTP 401.
    const UNAUTHORIZED: Self;

    /// An upgrade refused for a key setup still owed. A renewal cannot cure
    /// it, so the client neither renews nor retries (`SRV-164`). Mirrors the
    /// HTTP 403 of `SETUP_REQUIRED`.
    const SETUP_REQUIRED: Self;

    /// An upgrade refused for a client of an older wire version (`SRV-269`).
    /// The relay sends one frame of its own version first, which that client
    /// reads as an update to install (`SYNC-137`). Neither a renewal nor a
    /// retry cures it.
    const UPDATE_REQUIRED: Self;

    /// The reason text the relay sends beside a refusal code, `None` for a
    /// code the relay never sends.
    fn reason(self) -> Option<&'static str>;
}

impl RelayClose for Bounded<CloseCode> {
    const UNAUTHORIZED: Self = Self::of_u16::<4401>();
    const SETUP_REQUIRED: Self = Self::of_u16::<4403>();
    const UPDATE_REQUIRED: Self = Self::of_u16::<4426>();

    fn reason(self) -> Option<&'static str> {
        if self == Self::UNAUTHORIZED {
            Some("unauthorized")
        } else if self == Self::SETUP_REQUIRED {
            Some("setup required")
        } else if self == Self::UPDATE_REQUIRED {
            Some("update required")
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Server -> Client
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug)]
pub enum WsServerMsg {
    /// Subscription confirmed.
    Subscribed { document_id: Id<Document> },

    /// Encrypted CRDT delta with server-assigned seq.
    Update { seq: Seq<Relay>, update: Update },

    /// Encrypted ephemeral.
    Ephemeral(Ephemeral),

    /// A peer stopped watching a document: they unsubscribed, or their
    /// connection closed. Broadcast on the document topic to everyone still
    /// there, so a departure is a message and not a timeout
    /// (`1-Foundations/ambre-spec-presence.md`, `PRESENCE-014`).
    ///
    /// It carries what the relay already routes on -- a document id and an
    /// author key -- and nothing else. No content, and no claim about why the
    /// peer left (`ambre-spec-zero-knowledge.md`). The recipient resolves the
    /// key to an `Id<User>` exactly as it does for an ephemeral. A departure is
    /// attributed by the same spoof-proof path as a cursor.
    ///
    /// Best-effort, like presence itself: a lost `PeerLeft` leaves the peer to
    /// age out of the live set.
    PeerLeft {
        document_id: Id<Document>,
        pub_key: FixedBytes<PublicKey>,
    },

    /// Response to RequestMissing.
    MissingUpdates {
        document_id: Id<Document>,
        updates: Vec<SeqUpdate>,
    },

    /// Too many deltas accumulated -- client must compact.
    SnapshotRequired { document_id: Id<Document> },

    /// New snapshot available for download via S3. It is pending: a member who
    /// holds the deltas it covers checks it and confirms it (`SRV-129`).
    SnapshotAvailable {
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        /// The highest delta its author says it folds in.
        covered_seq: Seq<Relay>,
    },

    /// The document key was rotated: every REMAINING member must re-fetch its
    /// freshly wrapped `doc_key_enc` from `/members` and swap it in. Broadcast on
    /// the document topic. Carries no user identity: it is not a revocation
    /// signal. No recipient can mistake it for "you were revoked" (that is the
    /// separate, targeted `DocumentRevoked`). Rotation typically follows a revoke,
    /// but the two are distinct events.
    KeyRotationRequired { document_id: Id<Document> },

    /// The authenticated user was removed from a document. Pushed ONLY to the
    /// revoked user, on their per-user topic (`user:{id}`), by the server's
    /// revoke handler. The client closes the document locally. Distinct from
    /// `KeyRotationRequired` so a member managing keys never revokes themselves.
    DocumentRevoked { document_id: Id<Document> },

    /// A document was just shared with this user. Pushed on the per-user topic
    /// (`user:{id}`), which every connection subscribes to, so the invitee learns
    /// about the new document without polling. The client refreshes its document
    /// list and notifies the user.
    DocumentShared {
        document_id: Id<Document>,
        doc_type: DocType,
    },

    /// A refused wire message, named by its code alone (`SRV-183`). It names
    /// the document it concerns, and none for a frame that does not decode or
    /// a connection over its rate. The detail stays in the relay's log.
    Error {
        document_id: Option<Id<Document>>,
        code: RefusalCode,
    },

    /// A delta refused for its key: the document owes a rotation, and every key
    /// it held is one a departed member holds too (`ambre-spec-server.md`,
    /// `SRV-123`). Sent to the author alone. The delta took no sequence number
    /// and consumed no clock (`SRV-125`). So the author seals it again under
    /// the next key, at the same clock, and sends it again (`SYNC-129`).
    KeyRefused {
        document_id: Id<Document>,
        key_id: FixedBytes<KeyId>,
        clock: Seq<DeltaAuthor>,
    },

    /// A delta refused for its clock: another session of the same author took
    /// it first (`ambre-spec-sequencer.md`, `SRV-037`). Sent to the author
    /// alone, naming the delta by its signature. The author seals that delta
    /// again from the `expected` clock on (`SYNC-147`).
    ClockRefused {
        document_id: Id<Document>,
        clock: Seq<DeltaAuthor>,
        expected: Seq<DeltaAuthor>,
        signature: FixedBytes<Signature>,
    },
    /// The members of this document changed: someone joined, left, or took
    /// another role (`SRV-177`). It names the document alone; a client that
    /// shows the members asks the relay for the rows again.
    ///
    /// Appended last, so the wire tags of the variants above do not move.
    MembersChanged { document_id: Id<Document> },

    /// A replay ended before its end: the relay could not read its index or
    /// a delta (`ambre-spec-sequencer.md`, `SRV-184`). The pages sent before
    /// it stand, and `after_seq` is the last delta they delivered. The client
    /// asks again from there (`SYNC-153`).
    ReplayStopped {
        document_id: Id<Document>,
        after_seq: Seq<Relay>,
    },

    /// A delta refused before it took a sequence number, for its size, its
    /// shape, its signature or the sender's role (`ambre-spec-broadcast.md`,
    /// `SRV-185`). Sent to the author alone, naming the delta by its signature.
    /// The relay holds nothing of it, so the author holds an edit the relay
    /// lacks (`SYNC-155`).
    DeltaRefused {
        document_id: Id<Document>,
        signature: FixedBytes<Signature>,
        code: RefusalCode,
    },

    /// A pending snapshot a member reported, which the relay dropped
    /// (`ambre-spec-sequencer.md`, `SRV-133`). Broadcast on the document
    /// topic. It names the keys of its author and of its reporter, which the
    /// client resolves as it does for a cursor. `after` says whether it was
    /// the final snapshot of a segment, which a member then finalizes again.
    SnapshotDropped {
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        author: FixedBytes<PublicKey>,
        reporter: FixedBytes<PublicKey>,
        after: ambre_core::AfterDrop,
    },

    /// A thread document took a delta (`ambre-spec-broadcast.md`,
    /// `SRV-161`). Published on the topic of its channel, so it reaches every
    /// socket watching a document of that channel. It names the thread alone
    /// (`SRV-162`): a client that holds the thread hears the delta on its own
    /// topic, and any other fetches the summaries.
    ThreadActivity { document_id: Id<Document> },
}

// ---------------------------------------------------------------------------
// Wire encoding (postcard + version byte)
// ---------------------------------------------------------------------------

/// The role of one socket frame: the version byte, then a postcard message.
/// `Payload<WireFrame>`. The encoders write one and the decoders read one, so
/// a byte string from elsewhere does not reach a decoder untyped. It holds
/// sealed envelopes, so `Debug` prints the length alone (`ZK-006`).
pub enum WireFrame {}

impl ambre_core::PayloadKind for WireFrame {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("WireFrame");
}

/// Encode a client message for the wire.
///
/// # Panics
///
/// When postcard cannot serialize `msg`. Every field of [`WsClientMsg`] is a
/// postcard type, so the encoder fails only when the allocator does.
#[expect(
    clippy::expect_used,
    reason = "every field is a postcard type and a Vec takes any length, so the encode cannot fail"
)]
pub fn encode_client_msg(msg: &WsClientMsg) -> Payload<WireFrame> {
    let buf = vec![CURRENT_VERSION.get()];
    Payload::new(postcard::to_extend(msg, buf).expect("postcard encode WsClientMsg"))
}

/// Decode a client message from the wire.
///
/// # Errors
///
/// * [`WireDecodeError::Empty`] on no bytes at all.
/// * [`WireDecodeError::UnsupportedVersion`] when the first byte is not the
///   wire version.
/// * [`WireDecodeError::Postcard`] when the body does not decode.
pub fn decode_client_msg(frame: &Payload<WireFrame>) -> Result<WsClientMsg, WireDecodeError> {
    let Some((&version, body)) = frame.as_bytes().split_first() else {
        return Err(WireDecodeError::Empty);
    };
    match version {
        v if v == CURRENT_VERSION.get() => Ok(postcard::from_bytes(body)?),
        v => Err(WireDecodeError::UnsupportedVersion(v)),
    }
}

/// Encode a server message for the wire.
///
/// # Panics
///
/// When postcard cannot serialize `msg`, for the reason
/// [`encode_client_msg`] states.
#[expect(
    clippy::expect_used,
    reason = "every field is a postcard type and a Vec takes any length, so the encode cannot fail"
)]
pub fn encode_server_msg(msg: &WsServerMsg) -> Payload<WireFrame> {
    let buf = vec![CURRENT_VERSION.get()];
    Payload::new(postcard::to_extend(msg, buf).expect("postcard encode WsServerMsg"))
}

/// Decode a server message from the wire.
///
/// # Errors
///
/// The three errors [`decode_client_msg`] raises, on the same three inputs.
pub fn decode_server_msg(frame: &Payload<WireFrame>) -> Result<WsServerMsg, WireDecodeError> {
    let Some((&version, body)) = frame.as_bytes().split_first() else {
        return Err(WireDecodeError::Empty);
    };
    match version {
        v if v == CURRENT_VERSION.get() => Ok(postcard::from_bytes(body)?),
        v => Err(WireDecodeError::UnsupportedVersion(v)),
    }
}

#[derive(Debug)]
pub enum WireDecodeError {
    Empty,
    UnsupportedVersion(u8),
    Postcard(postcard::Error),
}

impl WireDecodeError {
    /// Whether the frame came from a peer on a newer wire version than this
    /// build's. Only an update cures it (`ambre-spec-sync.md`, `SYNC-137`).
    /// An older peer is the relay's lag, never the user's to fix.
    pub fn is_from_newer_peer(&self) -> bool {
        matches!(self, WireDecodeError::UnsupportedVersion(v) if *v > CURRENT_VERSION.get())
    }
}

impl From<postcard::Error> for WireDecodeError {
    fn from(e: postcard::Error) -> Self {
        WireDecodeError::Postcard(e)
    }
}
