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

use serde::{Deserialize, Serialize};

use crate::{DocType, DocumentId, MeetingId, Role, Timestamp, UserId};

// -----------------------------------------------------------------------------
// Secret-carrying IPC fields (H5)
// -----------------------------------------------------------------------------

/// Secret bytes and secret text that cross the UI -> Logic -> Sync boundary.
/// They carry a plaintext on its way to or from a seal, and the recovery
/// phrase. Both
/// live in `ambre-core` on [`ambre_core::Secret`], so a crate that cannot
/// depend on the bus holds them too. A passphrase travels as
/// [`ambre_core::Passphrase`].
pub use ambre_core::{SecretBytes, SecretString};

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
    pub name: Option<ambre_core::Name>,
    pub role: Role,
    /// Epoch milliseconds.
    pub updated_at: crate::Timestamp,
    /// Total member count (routing metadata, plaintext). `> 1` => shared.
    pub member_count: ambre_core::MemberCount,
}

/// Member of a document or channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberInfo {
    pub user_id: UserId,
    pub display_name: ambre_core::DisplayName,
    pub email: ambre_core::Email,
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

/// The key of a draft row in the local store, or a bound of a range of
/// such keys. The Murmure shelf spells it; the store and the seal only carry
/// it. It orders as its text, which is the order the store ranges keys in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DraftKey(String);

impl DraftKey {
    /// The key spelled as `text`.
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The two bounds that hold every key of the store.
    pub fn every() -> (Self, Self) {
        (Self(String::new()), Self("\u{10FFFF}".to_owned()))
    }

    /// The key's text, for the store call and the seal's associated data.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DraftKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a row sits in the local store. Its seal is bound to it, so a row
/// moved to another place stops opening (`ambre-spec-residency.md` section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowPlace {
    /// A snapshot or a delta of this document.
    Document(DocumentId),
    /// The draft the store keeps under this key.
    Draft(DraftKey),
}

/// One row of the local store, as the store holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedRow {
    pub place: RowPlace,
    pub sealed: ambre_core::SealedRowBytes,
}

/// User (search result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: UserId,
    pub display_name: ambre_core::DisplayName,
    pub email: ambre_core::Email,
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
        /// The account passphrase. Wiped on drop by [`ambre_core::Passphrase`]
        /// once the Sync thread finishes deriving the user key.
        passphrase: ambre_core::Passphrase,
    },
    /// First-time account setup. -> `AccountCreated { recovery_words }`.
    /// `email`/`display_name` are the user-supplied identity; the server records
    /// them verbatim instead of a hardcoded placeholder (N3). `email` is the
    /// handle other members resolve when sharing a document.
    SetupAccount {
        passphrase: ambre_core::Passphrase,
        email: ambre_core::Email,
        display_name: ambre_core::DisplayName,
    },
    /// Dev-only: authenticate as `username` (created on the fly server-side),
    /// deriving keys deterministically from the username. Triggered by the
    /// `?user=<name>` URL param to test multi-user behaviour on one machine.
    /// Rejected unless the server runs with `DEV_MODE`. -> `SessionReady`.
    DevLogin { username: ambre_core::DisplayName },
    /// Change the account passphrase. -> `PassphraseChanged`.
    ChangePassphrase {
        current_passphrase: ambre_core::Passphrase,
        new_passphrase: ambre_core::Passphrase,
    },
    /// Account recovery via 24-word BIP39 phrase. -> `AccountRecovered`.
    RecoverAccount {
        recovery_words: SecretString,
        new_passphrase: ambre_core::Passphrase,
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
        name: ambre_core::Name,
    },
    /// Delete a document. The Logic thread drops its in-memory state (hook). -> `DocumentDeleted`.
    DeleteDocument { document_id: DocumentId },
    /// Rename a document. -> `DocumentRenamed`.
    RenameDocument {
        document_id: DocumentId,
        name: ambre_core::Name,
    },

    // -- Sharing -----------------------------------------------------------------
    /// Phase 1 of the TOFU two-phase invite (`ambre-spec-key-verification.md`):
    /// resolve the invitee and fetch their public key **without wrapping**, so
    /// the user can verify the fingerprint before any Document Key is shared.
    /// -> `InviteeKey`.
    FetchInviteeKey {
        document_id: DocumentId,
        invitee_email: ambre_core::Email,
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
        invitee_email: Option<ambre_core::Email>,
        invitee: UserId,
        role: Role,
        expect_exchange_pub_key: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
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
    SearchUsers { query: ambre_core::UserSearch },

    // -- Media -------------------------------------------------------------------
    /// Upload a media file. -> `MediaUploaded`.
    ///
    /// It names no media type: the application keeps its own, and the relay
    /// learns nothing of the blob beyond its size (`SYNC-053`).
    UploadMedia {
        document_id: DocumentId,
        /// Raw (unencrypted) media bytes.
        data: ambre_core::MediaBytes,
    },
    /// Download a media file. -> `MediaDownloaded`.
    DownloadMedia {
        document_id: DocumentId,
        content_hash: ambre_core::ContentHash,
    },
    /// Erase this user's upload of a media file. -> `MediaErased`.
    ///
    /// The relay removes the object once its last uploader asks
    /// (`SRV-139`). A refusal leaves nothing of this user's to erase, so it
    /// answers `MediaErased` too; only a failure to reach the relay is an error.
    EraseMedia {
        document_id: DocumentId,
        content_hash: ambre_core::ContentHash,
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
        passphrase: ambre_core::Passphrase,
    },
    /// Open a sealed `.ambre` with `passphrase`. -> `DocFileOpened`
    /// / `DocFileOpenFailed`.
    OpenDocFile {
        /// A whole sealed container, header included.
        sealed: ambre_core::SealedContainer,
        passphrase: ambre_core::Passphrase,
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
    CreateMeeting { name: ambre_core::Name },
    /// Join a meeting. -> `MeetingJoined`.
    JoinMeeting { meeting_id: MeetingId },
    /// Leave a meeting. -> `MeetingLeft`.
    LeaveMeeting { meeting_id: MeetingId },
    /// End a meeting for all participants. -> `MeetingEnded`.
    EndMeeting { meeting_id: MeetingId },

    // -- Provider sign-in ----------------------------------------------------------
    /// Ask the relay for the address of `provider`'s authorization page.
    /// -> `ProviderAddress`.
    ProviderSignIn { provider: ambre_core::OidcProvider },
    /// Exchange the `code` and `state` a provider returned with. -> `ProviderSignedIn`.
    /// The session tokens stay on the Sync worker; the socket stays closed until
    /// the keys are unlocked or set up.
    ProviderReturn {
        provider: ambre_core::OidcProvider,
        code: ambre_core::OidcCode,
        state: ambre_core::OidcState,
    },
}

// -----------------------------------------------------------------------------
// RestEvent -- relayed service response (Sync -> Logic -> UI)
// -----------------------------------------------------------------------------

ambre_core::define_flag! {
    /// Whether the account holds its sealed keys on the relay (`CHROME-096`):
    /// what follows a provider sign-in is an unlock or a setup.
    pub enum KeySetup {
        /// No key is stored yet: the next step sets the keys up.
        Absent,
        /// The keys are stored: the next step unlocks them.
        Stored,
    }
}

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
        display_name: ambre_core::DisplayName,
        email: ambre_core::Email,
    },
    /// First connection: no keys on the server -- the UI must show onboarding.
    AccountSetupRequired,
    /// Display `recovery_words` **exactly once**; they cannot be retrieved later.
    AccountCreated {
        recovery_words: SecretString,
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
        display_name: ambre_core::DisplayName,
        email: ambre_core::Email,
        exchange_pub_key: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
        fingerprint: ambre_core::KeyFingerprint,
    },
    /// The answer to `FetchMemberKey`: the key the relay serves for that
    /// user, and its fingerprint, for the same comparison with the pin.
    MemberKey {
        user_id: UserId,
        exchange_pub_key: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
        fingerprint: ambre_core::KeyFingerprint,
    },
    InviteSent {
        document_id: DocumentId,
        user_id: UserId,
        display_name: ambre_core::DisplayName,
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
        content_hash: ambre_core::ContentHash,
    },
    MediaDownloaded {
        document_id: DocumentId,
        content_hash: ambre_core::ContentHash,
        /// Decrypted raw media bytes, ready for display.
        data: ambre_core::MediaBytes,
    },
    MediaErased {
        document_id: DocumentId,
        content_hash: ambre_core::ContentHash,
    },

    // -- The native file (`.ambre`) ----------------------------------------------
    /// The sealed file, ready for the user's disk.
    DocFileSealed {
        data: ambre_core::SealedContainer,
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
        sealed: ambre_core::SealedRowBytes,
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
        url: ambre_core::ProviderUrl,
        state: ambre_core::OidcState,
    },
    /// The provider return was exchanged. The tokens stayed on the Sync worker;
    /// `keys_stored` says whether the next step is an unlock or a setup.
    ProviderSignedIn {
        user_id: UserId,
        display_name: ambre_core::DisplayName,
        email: ambre_core::Email,
        keys_stored: KeySetup,
    },
}
