//! Fundamental types shared by all Ambre crates.
//!
//! - [`Uuid`]: UUIDv7 wrapper (`[u8; 16]`)
//! - [`Id`]: the strongly-typed identifier, one generic type over an entity
//!   marker of [`id`], such as `Id<Document>` or `Id<Element>`
//! - [`DocType`]: enum of document types (Prose, Trame, Toile, Murmure)
//! - [`PublicKey`], [`ExchangePublicKey`]: `[u8; 32]` wrappers for Ed25519 and X25519
//! - [`Seq`]: the relay's sequence number of a document
//! - [`Version`]: version byte for serialization
//! - [`schema`]: the persisted schema version and its migration walk
//! - [`authorship`]: the session nonce and the peer registry entry
//! - [`crdt`]: the typed root names and map keys of a CRDT document
//! - [`Error`]: core error types
//! - [`docfile`]: the `.ambre` container header in its two lineages -- plaintext
//!   and passphrase-sealed -- shared by every app
//! - [`fixtures`] (feature `fixtures`, host-only): the document-fixture
//!   generator contract and its sidecar manifest

// No hand-written `unsafe` in this crate (constitution pillar). Any future
// `unsafe` block breaks the build rather than silently landing.
#![forbid(unsafe_code)]

/// The serde paths the `define_flag!` and `define_tally!` macros name, so a
/// crate that expands one needs no serde dependency of its own.
#[doc(hidden)]
pub mod __serde {
    pub use serde::{Deserialize, Deserializer, Serialize, Serializer};
}

/// The activity summary an update may carry, and the counters it moves
/// (`ambre-spec-residency.md`).
pub mod activity;
pub mod authorship;
pub mod bounded;
pub mod calendar;
pub mod channel_kind;
/// Colours typed by their packing: [`Rgba`], [`Rgb`].
pub mod color;
pub mod crdt;
pub mod credential;
pub mod doc_type;
pub mod docfile;
pub mod email;
pub mod encoded;
pub mod envelope;
pub mod error;
/// The name and the bytes of a file an import reads or an export writes.
pub mod export_file;
pub mod fingerprint;
/// Fixed-width byte values, one generic type over a role marker.
pub mod fixed_bytes;
/// Fixture machinery (`ambre-spec-fixtures.md`). Behind the non-default
/// `fixtures` feature so no client build can reach it (I4).
#[cfg(feature = "fixtures")]
pub mod fixtures;
/// Two-variant enums for a `bool` that crosses a store, a wire or a public struct.
pub mod flag;
/// Font attributes every application shares: [`FontSize`].
pub mod font;
pub mod generation;
pub mod id;
/// Index families: a position, a count, a difference, a span and a list, one
/// generic type each over an axis marker.
pub mod index;
pub mod key_id;
pub mod lang;
pub mod layout;
/// The target of a link: [`LinkUrl`].
pub mod link;
pub mod media;
pub mod member_count;
/// Two-way choices shared across crates: [`StepDirection`], [`HistoryStep`].
pub mod mode;
pub mod name;
pub mod object_key;
pub mod ordinal;
pub mod payload;
pub mod provider;
pub mod public_key;
pub mod redacted;
/// Which documents a client holds, and which leave first
/// (`ambre-spec-residency.md`).
pub mod residency;
pub mod role;
pub mod scale;
/// The persisted schema version: the register, the read rule and the migration
/// walk every application shares (`ambre-spec-schema-version.md`).
pub mod schema;
pub mod search;
pub mod secret;
pub mod seq;
/// Serde adapters for enum serialization as lowercase strings.
pub mod serde_enums;
/// Serde adapters for identifier serialization as hyphenated UUID strings.
pub mod serde_id;
pub mod space;
pub mod span;
pub mod static_key;
pub mod status;
pub mod tally;
pub mod text_offset;
/// Lengths and angles, one type per unit and per pixel space.
pub mod unit;
pub mod uuid;
pub mod validated;
pub mod version;

// Re-export the main types for convenient use.
pub use authorship::{
    IdCounter, OpCounter, PeerEntry, PeerEntryBytes, PeerId, PeerKey, SessionNonce,
};
pub use bounded::{Bounded, BoundedRepr, Bounds, Defaulted, OutOfRange, Refusal, UnitFraction};
pub use calendar::{Month, Year};
pub use channel_kind::{ChannelDocumentKind, SegmentState};
pub use color::{Alpha, Opacity, Rgb, Rgba};
pub use credential::ProviderUrl;
pub use doc_type::DocType;
pub use docfile::{
    AMBRE_MAGIC, AMBRE_SEALED_VERSION, AMBRE_VERSION, AmbreHeader, Container, ContainerFile,
    ContainerKind, DocFileError, FileDigest, FileSalt, HEADER_LEN, SALT_LEN, SEAL_NONCE_LEN,
    SEALED_AAD_LEN, SEALED_HEADER_LEN, SealedAmbre, classify, migrate_ambre, parse_sealed_header,
    read_ambre_plaintext, read_ambre_plaintext_as, sealed_aad, write_ambre_plaintext,
    write_ambre_sealed,
};
pub use email::Email;
pub use encoded::{
    Encoded, EncodedKeyLink, EncodedKind, EncodedLogEntry, EscrowedKey, MASTER_KEY_SALT_LEN,
    MasterKeySalt, SealedName, SealedShare, WrappedDocKey, WrappedMasterKey,
};
pub use envelope::{Aad, ChainHash, Ciphertext, CommitmentTag, EntryDigest, Nonce, Signature};
pub use error::{DocTypeError, Error, UuidError, VersionError};
pub use export_file::{ExportExtension, ExportFilename, ExportFilenameError, FileBytes};
pub use fingerprint::KeyFingerprint;
pub use fixed_bytes::{ByteArray, DebugForm, FixedBytes, FixedBytesError, FixedRole};
#[cfg(feature = "fixtures")]
pub use fixtures::{
    BoardFrames, Connectors, DocBlocks, FIXTURE_IMPORT_PEER, FixtureGen, FixtureManifest,
    FixtureName, FixtureSeed, FixtureShape, Formulas, Generator, Peers, ShapeBody, Sheets,
    fixture_peer_count, seeded_document_id,
};
pub use font::{
    FontFamily, FontSize, FontWeight, InvalidFontSize, Slant, Strikethrough, Underline, Weight,
    WordBreak,
};
pub use generation::{Generation, GenerationKind, Issuer, SharedIssuer};
pub use id::{DocumentScoped, Entity, Id, IdRepr};
pub use key_id::{KEY_ID_LEN, KeyId};
pub use lang::LangTag;
pub use link::LinkUrl;
pub use media::{
    ByteSize, ContentHash, ContentHashError, KnownMediaType, MEDIA_ADDRESS_LEN, MediaAddress,
    MediaType,
};
pub use member_count::{MemberCountError, Members};
pub use mode::{
    AfterDrop, Availability, CaseMatch, Creation, DocumentScope, FinalizeEnd, HistoryStep,
    LinkState, PasteForm, RotationDebt, StepDirection, ThreadState, Toggle, Visibility, WordMatch,
};
pub use name::{
    DisplayName, DisplayNameRule, InvalidDisplayName, NAME_MAX_CHARS, NAME_TOO_LONG, Name,
    NameLength, NameTooLong, name_fits,
};
pub use object_key::ObjectKey;
pub use ordinal::{Ordinal, OrdinalKind};
pub use payload::{
    ClipboardBytes, DeltaBytes, EphemeralBytes, FontBytes, FormatMagic, LoroPayload,
    LoroPayloadRef, MediaBytes, Payload, PayloadKind, PickedContainer, PickedFile,
    PlaintextContainer, RgbaPixels, SealedContainer, SealedPreviousKey, SealedRowBytes,
    SnapshotBytes, ThumbnailPng,
};
pub use provider::OidcProvider;
pub use public_key::{ExchangePublicKey, PublicKey};
pub use redacted::Redacted;
pub use role::{DocumentRank, OrganizationRank, Role};
pub use scale::Scale;
pub use schema::{
    SCHEMA_VERSION_ABSENT, Schema, SchemaError, SchemaMigration, SchemaVerdict, migrate_schema,
    migration_peer, scaffold_peer, verdict,
};
pub use search::{SearchQuery, SearchQueryRule, UserSearch, UserSearchRule};
pub use secret::{
    AccessToken, Credential, CredentialKind, OidcCode, OidcState, Passphrase, PresignedUrl,
    RefreshToken, Secret, SecretBytes, SecretString, WireCredential,
};
pub use seq::{DeltaCount, Seq};
pub use space::{
    Board, CanvasCss, Css, Device, Doc, Image, ScaleSpace, Space, StoredBoard, Surface, Texel, View,
};
pub use span::{
    Authored, Failures, FracMs, FrameRate, Instant, LamportStamp, Ms, Ns, PageLoad, Rfc3339, Sec,
    StoredAsI64, Tick, TimeBase, TimeSpan, TimeUnit, UnixMs, UnixSec,
};
pub use static_key::{Canvases, Categories, Commands, Icons, KeyKind, StaticKey};
pub use status::{
    ApiFailure, HttpStatus, RefusalCode, RotationStatus, SnapshotStatus, StatusError,
    UnreadableCause, WriteStatus,
};
pub use tally::{CellCount, Elements, Recounts, Tally, TallyKind};
pub use text_offset::{
    Byte, Char, RedactedChars, Utf16, byte_span_to_chars, byte_to_char, char_len,
    char_span_to_bytes, char_to_byte, char_to_utf16, utf16_to_char,
};
pub use unit::{
    Mm, PX_PER_MM, Point, PrintLength, PrintUnit, Pt, Px, Radians, Size, Vector, ViewSpeed,
};
pub use uuid::Uuid;
pub use validated::{Rule, Validated};
pub use version::{Current, OpenRange, Version, VersionKind, VersionRepr};
