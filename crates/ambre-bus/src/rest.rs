//! Relayed service plane -- the correlatable request/response family.
//!
//! Most P3 traffic is a pure relay. The UI asks for an operation, and the Logic
//! thread forwards it verbatim to the Sync thread, which performs the REST,
//! crypto and WebSocket work. The response travels back the same way. Modelling
//! each step as its own enum variant produced four near-identical enums linked
//! by ~300 lines of hand-written 1:1 mapping (the "P3 triplication").
//!
//! Instead, this family is defined **once** here and carried by thin envelopes:
//!
//! ```text
//! LogicCommand::Rest { id, cmd: RestCommand }   // UI    -> Logic
//! SyncCommand ::Rest { id, cmd: RestCommand }   // Logic -> Sync
//! LogicEvent  ::Rest { id, evt: RestEvent }     // Sync  -> Logic
//! UiEvent     ::Rest { id, evt: RestEvent }     // Logic -> UI
//! ```
//!
//! The Logic dispatcher collapses to a single forwarding arm, plus a tiny hook
//! for the few service ops that also mutate Logic state. `Logout` is one: it
//! clears the document registry. Adding a service op = one `RestCommand` variant +
//! one `RestEvent` variant, edited in one place.
//!
//! # What is *not* here
//! - **Content plane** (`AppCommand` / `AppEvent`, `MurmureCommand` /
//!   `MurmureEvent`): document edits and viewport data, routed by `DocumentId`.
//!   Channel commands are routed by `ChannelId` inside the Logic thread. See
//!   [`crate::logic`].
//! - **Control** (`OpenDocument`, `CloseDocument`, `OpenChannel`,
//!   `CloseChannel`): fire-and-forget, no single response to correlate -- kept
//!   as explicit envelope variants.
//! - **Server push** (`Remote*`, `ConnectionStatus`, `SyncWarning`,
//!   `MemberRevoked`, `*ParticipantJoined/Left`): unsolicited, no originating
//!   request -- kept explicit.
//! - **Errors** (`AuthError`, `ApiError`): may be either a response or a
//!   spontaneous server error; kept explicit for now (correlating errors by
//!   `RequestId` is a later refinement).

use core::sync::atomic::{AtomicU64, Ordering};
use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

use crate::{DocType, DocumentId, MeetingId, Role, Timestamp, UserId};

// -----------------------------------------------------------------------------
// Secret-carrying IPC fields (H5)
// -----------------------------------------------------------------------------

/// Secret byte material crossing the UI -> Logic -> Sync boundary: a passphrase, or a
/// document's plaintext on its way to or from a seal.
///
/// The second use is deliberate. A `Vec<u8>` would relay the same bytes, and also render
/// them in a `Debug` line and leave its copy in the heap after the hop. In a feature about
/// a document at rest, that is the wrong default.
///
/// Wraps [`Zeroizing<Vec<u8>>`], so the bytes are wiped when the value is dropped anywhere
/// along the relay -- including the placeholder handlers that merely bind-and-drop. `Debug`
/// is redacted, so the secret can never leak into a log line. The byte contents are exposed
/// only through [`Deref`] to `[u8]`, which the crypto call sites already consume.
#[derive(Clone, Default)]
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    /// Wrap owned bytes as a zeroizing secret.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

impl From<Vec<u8>> for SecretBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl Deref for SecretBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBytes([redacted; {} bytes])", self.0.len())
    }
}

impl Serialize for SecretBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SecretBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(Vec::<u8>::deserialize(deserializer)?))
    }
}

/// Secret UTF-8 material (the 24-word BIP39 recovery phrase) travelling across
/// the IPC boundary. Same guarantees as [`SecretBytes`], wrapping
/// [`Zeroizing<String>`] and exposing the text through [`Deref`] to `str`.
#[derive(Clone, Default)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    /// Wrap an owned string as a zeroizing secret.
    pub fn new(s: String) -> Self {
        Self(Zeroizing::new(s))
    }
}

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl Deref for SecretString {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretString([redacted; {} bytes])", self.0.len())
    }
}

impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

// -----------------------------------------------------------------------------
// RequestId -- request/response correlation token
// -----------------------------------------------------------------------------

/// Correlation token tying a [`RestCommand`] to the [`RestEvent`] that answers it.
///
/// Allocated in the Main instance via [`RequestId::next`] and echoed unchanged by
/// the Sync worker. Lets a UI with several P3 operations in flight
/// route each response to its call-site, and apply per-request timeout / retry. The
/// previous fire-and-forget, stringly-correlated model could do neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub u64);

impl RequestId {
    /// The id of an answer that no request asked for, such as the Sync
    /// worker's silent re-login. The counter starts at 1, so no request
    /// carries it (`BUS-004`).
    pub const UNSOLICITED: Self = RequestId(0);

    /// Allocate the next process-wide-unique id.
    ///
    /// Backed by one `AtomicU64` that the UI and Logic threads share over the
    /// Main instance's heap. The Sync worker has its own memory and so its own
    /// counter, and it never calls this (`BUS-004`).
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        RequestId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

// -----------------------------------------------------------------------------
// Business-shared response types (zero crypto -- names already decrypted by Sync)
// -----------------------------------------------------------------------------

/// Summary of a document (for lists and navigation).
///
/// The `name` is decrypted by the Sync thread -- the Logic and UI threads see it
/// in plaintext.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub document_id: DocumentId,
    pub doc_type: DocType,
    /// Decrypted from `name_enc` by the Sync thread.
    pub name: Option<String>,
    pub role: Role,
    /// Epoch milliseconds.
    pub updated_at: u64,
    /// Total member count (routing metadata, plaintext). `> 1` => shared.
    pub member_count: u32,
}

/// Member of a document or channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberInfo {
    pub user_id: UserId,
    pub display_name: String,
    pub email: String,
    pub role: Role,
    /// What the signed membership log says of this row
    /// (`ambre-spec-membership.md`, `SYNC-103`).
    pub standing: MemberStanding,
}

/// What the signed membership log says of one member row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemberStanding {
    /// A chain of valid entries links the row to the first entry of the log.
    Verified,
    /// No valid chain reaches the row: it stands on the relay's word alone
    /// (`SYNC-108`).
    Unverified,
    /// The log's latest valid entry for the member removes them, whatever the
    /// relay serves (`SYNC-109`).
    Revoked,
}

/// Where a row sits in the local store. Its seal is bound to it, so a row
/// moved to another place stops opening (`ambre-spec-residency.md` section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowPlace {
    /// A snapshot or a delta of this document.
    Document(DocumentId),
    /// The draft the store keeps under this key.
    Draft(String),
}

/// One row of the local store, as the store holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedRow {
    pub place: RowPlace,
    pub sealed: Vec<u8>,
}

/// User (search result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: UserId,
    pub display_name: String,
    pub email: String,
}

// -----------------------------------------------------------------------------
// RestCommand -- relayed service request (UI -> Logic -> Sync)
// -----------------------------------------------------------------------------

/// A correlatable service request. Carried inside `LogicCommand::Rest` and
/// forwarded verbatim as `SyncCommand::Rest`; the Sync thread answers with the
/// matching [`RestEvent`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestCommand {
    // -- Session -----------------------------------------------------------------
    /// Bootstrap auth after OIDC callback. -> `SessionReady` / `AccountSetupRequired`.
    Unlock {
        /// UTF-8 passphrase bytes. Zeroized on drop by [`SecretBytes`] once the
        /// Sync thread finishes deriving the user key.
        passphrase: SecretBytes,
    },
    /// First-time account setup. -> `AccountCreated { recovery_words }`.
    /// `email`/`display_name` are the user-supplied identity; the server records
    /// them verbatim instead of a hardcoded placeholder (N3). `email` is the
    /// handle other members resolve when sharing a document.
    SetupAccount {
        passphrase: SecretBytes,
        email: String,
        display_name: String,
    },
    /// Dev-only: authenticate as `username` (created on the fly server-side),
    /// deriving keys deterministically from the username. Triggered by the
    /// `?user=<name>` URL param to test multi-user behaviour on one machine.
    /// Rejected unless the server runs with `DEV_MODE`. -> `SessionReady`.
    DevLogin { username: String },
    /// Change the account passphrase. -> `PassphraseChanged`.
    ChangePassphrase {
        current_passphrase: SecretBytes,
        new_passphrase: SecretBytes,
    },
    /// Account recovery via 24-word BIP39 phrase. -> `AccountRecovered`.
    RecoverAccount {
        recovery_words: SecretString,
        new_passphrase: SecretBytes,
    },
    /// Log out. The Logic thread clears its document registry on dispatch (hook),
    /// then the Sync thread zeroizes all keys. -> `LoggedOut`.
    Logout,

    // -- Document CRUD -----------------------------------------------------------
    /// List accessible documents. -> `DocumentList`.
    ListDocuments,
    /// Create a document. -> `DocumentCreated`.
    CreateDocument {
        doc_type: DocType,
        /// Plaintext name (encrypted by the Sync thread before upload).
        name: String,
    },
    /// Delete a document. The Logic thread drops its in-memory state (hook). -> `DocumentDeleted`.
    DeleteDocument { document_id: DocumentId },
    /// Rename a document. -> `DocumentRenamed`.
    RenameDocument {
        document_id: DocumentId,
        name: String,
    },

    // -- Sharing -----------------------------------------------------------------
    /// Phase 1 of the TOFU two-phase invite (`ambre-spec-key-verification.md`):
    /// resolve the invitee and fetch their public key **without wrapping**, so
    /// the user can verify the fingerprint before any Document Key is shared.
    /// -> `InviteeKey`.
    FetchInviteeKey {
        document_id: DocumentId,
        invitee_email: String,
    },
    /// Fetch the exchange key of a user the client already names, such as a
    /// member of a space, **without wrapping**. The user compares it with the
    /// pin before a direct message wraps a key for them. -> `MemberKey`.
    FetchMemberKey { user_id: UserId },
    /// Phase 2 of the invite. `expect_exchange_pub_key` is the key the user confirmed
    /// (pinned) in phase 1. Sync re-fetches the invitee's key and wraps the Document Key
    /// **only if it still matches**. That closes the TOCTOU hole a compromised server holds
    /// between the two phases. On mismatch the invite fails with an explicit error. ->
    /// `InviteSent`.
    ///
    /// `invitee` is the user phase 1 answered, whom Sync finds again or
    /// refuses the invite. `now` is the instant the user confirmed, since
    /// Logic has no clock. A direct message of two stamps its notice of
    /// the third member with it (the Murmure channels spec).
    ///
    /// `invitee_email` is `None` for a member known by id alone, such as a
    /// member of a space: Sync then fetches the keys of `invitee` itself.
    InviteToDocument {
        document_id: DocumentId,
        invitee_email: Option<String>,
        invitee: UserId,
        role: Role,
        expect_exchange_pub_key: [u8; 32],
        now: Timestamp,
    },
    /// Revoke a member (triggers the full key-rotation sequence). -> `RevocationComplete`.
    RevokeFromDocument {
        document_id: DocumentId,
        user_id: UserId,
    },
    /// Leave a document: this user's own removal, and nothing else. The members
    /// who stay pay the rotation it leaves owed (`SYNC-127`). -> `DocumentLeft`.
    LeaveDocument { document_id: DocumentId },
    /// List current members. -> `DocumentMembers`.
    ListDocumentMembers { document_id: DocumentId },
    /// Update a member's role. -> `MemberRoleUpdated`.
    UpdateDocumentMemberRole {
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    },

    // -- Users -------------------------------------------------------------------
    /// Search users by email prefix. -> `UserSearchResults`.
    SearchUsers { query: String },

    // -- Media -------------------------------------------------------------------
    /// Upload a media file. -> `MediaUploaded`.
    ///
    /// It names no media type: the application keeps its own, and the relay
    /// learns nothing of the blob beyond its size (`SYNC-053`).
    UploadMedia {
        document_id: DocumentId,
        /// Raw (unencrypted) media bytes.
        data: Vec<u8>,
    },
    /// Download a media file. -> `MediaDownloaded`.
    DownloadMedia {
        document_id: DocumentId,
        content_hash: String,
    },
    /// Erase this user's upload of a media file. -> `MediaErased`.
    ///
    /// The relay removes the object once its last uploader asks
    /// (`SRV-139`). A refusal leaves nothing of this user's to erase, so it
    /// answers `MediaErased` too; only a failure to reach the relay is an error.
    EraseMedia {
        document_id: DocumentId,
        content_hash: String,
    },

    // -- The native file (`.ambre`) ----------------------------------------------
    //
    // Neither of these reaches the relay, and they are the only service requests that do
    // not. They are here because the *hops* are what they need. Sealing is cryptography, so
    // it runs in the Sync worker and nowhere else (`ARCH-004`). This plane is how the
    // interface asks that worker for something and gets a correlated answer back. Nothing
    // about them touches a session: a signed-out user, or an offline one, still seals and
    // opens their own files (`ambre-spec-docfile.md section 5`).
    //
    /// Seal a plaintext `.ambre` under `passphrase`. -> `DocFileSealed`
    /// / `DocFileSealFailed`.
    SealDocFile {
        /// A whole plaintext container, header included.
        plaintext: SecretBytes,
        passphrase: SecretBytes,
    },
    /// Open a sealed `.ambre` with `passphrase`. -> `DocFileOpened`
    /// / `DocFileOpenFailed`.
    OpenDocFile {
        /// A whole sealed container, header included.
        sealed: Vec<u8>,
        passphrase: SecretBytes,
    },

    // -- The local store (`ambre-spec-residency.md` section 3) -------------------
    //
    // Served inside the Sync worker too, under a key the session's seed derives
    // there and nowhere else (`RESID-026`). Without a session there is no key,
    // and both answer `LocalStoreLocked`.
    //
    /// Seal one row of the local store. -> `LocalRowSealed` / `LocalStoreLocked`.
    SealLocalRow {
        place: RowPlace,
        plaintext: SecretBytes,
    },
    /// Open rows the local store holds. -> `LocalRowsOpened` /
    /// `LocalStoreLocked`.
    OpenLocalRows { rows: Vec<SealedRow> },

    // -- Meetings (Rendez-Vous) --------------------------------------------------
    /// Create an ephemeral meeting room. -> `MeetingCreated`.
    CreateMeeting { name: String },
    /// Join a meeting. -> `MeetingJoined`.
    JoinMeeting { meeting_id: MeetingId },
    /// Leave a meeting. -> `MeetingLeft`.
    LeaveMeeting { meeting_id: MeetingId },
    /// End a meeting for all participants. -> `MeetingEnded`.
    EndMeeting { meeting_id: MeetingId },

    // -- Provider sign-in ----------------------------------------------------------
    /// Ask the relay for the address of `provider`'s authorization page.
    /// -> `ProviderAddress`. `provider` is a lowercase ASCII name; any other
    /// value is refused before a request leaves the Sync worker.
    ProviderSignIn { provider: String },
    /// Exchange the `code` and `state` a provider returned with. -> `ProviderSignedIn`.
    /// The session tokens stay on the Sync worker; the socket stays closed until
    /// the keys are unlocked or set up.
    ProviderReturn {
        provider: String,
        code: String,
        state: String,
    },
}

// -----------------------------------------------------------------------------
// RestEvent -- relayed service response (Sync -> Logic -> UI)
// -----------------------------------------------------------------------------

/// A correlatable service response. Carried inside `LogicEvent::Rest` and
/// forwarded verbatim as `UiEvent::Rest`, echoing the originating
/// [`RequestId`].
///
/// Success responses only: failures arrive as the explicit `AuthError` /
/// `ApiError` envelope variants (which may also be spontaneous).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestEvent {
    // -- Session -----------------------------------------------------------------
    SessionReady {
        user_id: UserId,
        display_name: String,
        email: String,
    },
    /// First connection: no keys on the server -- the UI must show onboarding.
    AccountSetupRequired,
    /// Display `recovery_words` **exactly once**; they cannot be retrieved later.
    AccountCreated {
        recovery_words: String,
    },
    PassphraseChanged,
    AccountRecovered,
    LoggedOut,

    // -- Document CRUD -----------------------------------------------------------
    DocumentList {
        documents: Vec<DocumentSummary>,
    },
    /// A document was created. The Logic thread records its `doc_type` (hook)
    /// until the first snapshot so it can build the matching state.
    DocumentCreated {
        document_id: DocumentId,
        doc_type: DocType,
    },
    DocumentDeleted {
        document_id: DocumentId,
    },
    DocumentRenamed {
        document_id: DocumentId,
    },

    // -- Sharing -----------------------------------------------------------------
    /// Phase 1 answer of the TOFU invite (`ambre-spec-key-verification.md`).
    /// Carries the invitee's raw X25519 `exchange_pub_key` (for the byte
    /// comparison against the `trusted_keys` store, done on the Main thread) and
    /// the already-formatted `fingerprint` string (the chrome never hashes).
    InviteeKey {
        document_id: DocumentId,
        user_id: UserId,
        display_name: String,
        email: String,
        exchange_pub_key: [u8; 32],
        fingerprint: String,
    },
    /// The answer to `FetchMemberKey`: the key the relay serves for that
    /// user, and its fingerprint, for the same comparison with the pin.
    MemberKey {
        user_id: UserId,
        exchange_pub_key: [u8; 32],
        fingerprint: String,
    },
    InviteSent {
        document_id: DocumentId,
        user_id: UserId,
        display_name: String,
    },
    /// Revocation initiated by **this** client is complete.
    RevocationComplete {
        document_id: DocumentId,
    },
    /// This user left the document; the Sync worker closed it.
    DocumentLeft {
        document_id: DocumentId,
    },
    DocumentMembers {
        document_id: DocumentId,
        members: Vec<MemberInfo>,
    },
    MemberRoleUpdated {
        document_id: DocumentId,
        user_id: UserId,
        role: Role,
    },

    // -- Users -------------------------------------------------------------------
    UserSearchResults {
        users: Vec<UserInfo>,
    },

    // -- Media -------------------------------------------------------------------
    MediaUploaded {
        document_id: DocumentId,
        content_hash: String,
    },
    MediaDownloaded {
        document_id: DocumentId,
        content_hash: String,
        /// Decrypted raw media bytes, ready for display.
        data: Vec<u8>,
    },
    MediaErased {
        document_id: DocumentId,
        content_hash: String,
    },

    // -- The native file (`.ambre`) ----------------------------------------------
    /// The sealed file, ready for the user's disk.
    DocFileSealed {
        data: Vec<u8>,
    },
    /// Sealing did not happen, so there is no file. Carries no detail. Two
    /// failures reach here: a key derivation out of memory, and a document past
    /// the AEAD size limit. The user acts on neither differently.
    DocFileSealFailed,
    /// The plaintext container the sealed file held. The caller reads its
    /// header as it would a file off the disk: the seal changes nothing about
    /// dispatch, version recovery or the schema gate.
    DocFileOpened {
        plaintext: SecretBytes,
    },
    /// The file did not open. **Deliberately one variant for two causes:** the
    /// tag cannot tell a wrong passphrase from a modified file, so naming one
    /// would be a guess. The surface says both (`DOCFILE-016`).
    DocFileOpenFailed,

    // -- The local store ---------------------------------------------------------
    /// The row, sealed for the store.
    LocalRowSealed {
        sealed: Vec<u8>,
    },
    /// The rows asked for, in their order. `None` is a row this session cannot
    /// open. It was sealed under another key or at another place, modified, or
    /// held in the clear before the store was sealed (`RESID-027`).
    LocalRowsOpened {
        rows: Vec<Option<SecretBytes>>,
    },
    /// No session holds a key yet, so nothing was sealed or opened.
    LocalStoreLocked,

    // -- Meetings (Rendez-Vous) --------------------------------------------------
    MeetingCreated {
        meeting_id: MeetingId,
    },
    MeetingJoined {
        meeting_id: MeetingId,
    },
    MeetingLeft {
        meeting_id: MeetingId,
    },
    MeetingEnded {
        meeting_id: MeetingId,
    },

    // -- Provider sign-in ----------------------------------------------------------
    /// The provider's authorization address, and the `state` the relay bound to it.
    ProviderAddress {
        url: String,
        state: String,
    },
    /// The provider return was exchanged. The tokens stayed on the Sync worker;
    /// `keys_stored` says whether the next step is an unlock or a setup.
    ProviderSignedIn {
        user_id: UserId,
        display_name: String,
        email: String,
        keys_stored: bool,
    },
}
