//! Fundamental types shared by all Ambre crates.
//!
//! - [`Uuid`]: UUIDv7 wrapper (`[u8; 16]`)
//! - [`Id`]: the strongly-typed identifier, one generic type over an entity
//!   marker of [`id`], such as `Id<Document>` or `Id<Element>`
//! - [`DocType`]: enum of document types (Prose, Trame, Toile, Murmure)
//! - [`PublicKey`], [`ExchangePublicKey`]: `[u8; 32]` wrappers for Ed25519 and X25519
//! - [`Seq`]: a number one issuer hands out going up, such as the relay's
//!   `Seq<Relay>`
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

/// The serde paths the `define_flag!` macro names, so a crate that expands
/// it needs no serde dependency of its own.
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
pub mod call;
pub mod channel_kind;
/// Colours typed by their packing: [`Rgba`], [`Rgb`].
pub mod color;
pub mod count;
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
/// A small set of named bits, one generic type over a marker: [`Flags`].
pub mod flags;
/// Font attributes every application shares: [`FontSize`].
pub mod font;
pub mod id;
/// Index families: a position, a count, a difference, a span and a list, one
/// generic type each over an axis marker.
pub mod index;
pub mod key_id;
pub mod lang;
pub mod layout;
/// The target of a link: [`LinkUrl`].
pub mod link;
pub mod locale;
pub mod media;
pub mod member_count;
/// Two-way choices shared across crates: [`StepDirection`], [`HistoryStep`].
pub mod mode;
pub mod name;
pub mod object_key;
/// Text a log never prints, one generic type over a kind marker: [`Opaque`].
pub mod opaque;
pub mod ordinal;
pub mod payload;
pub mod provider;
pub mod public_key;
pub mod rank;
pub mod redacted;
pub mod refused;
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
pub use bounded::{
    Bounded, BoundedRepr, Bounds, Defaulted, OutOfRange, Refusal, Saturates, UnitFraction,
};
pub use calendar::{Month, Year};
pub use call::{
    ChunkCounter, ClearChunk, KeyGeneration, KeyIdentifier, KeyIdentifierBytes, Ratchets,
    SENDER_INDEX_LIMIT, SealedChunk, SenderIndex, Track,
};
pub use channel_kind::{ChannelDocumentKind, SegmentState};
pub use color::{Alpha, Opacity, Rgb, Rgba};
pub use count::{
    Cells, Count, CountColumn, CountKind, CountRepr, Divisor, Elements, IndexedAs, Recounts,
};
pub use credential::ProviderUrls;
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
pub use envelope::{
    Aad, ChainHash, Ciphertext, CommitmentTag, EntryDigest, Nonce, Signature, Transcript,
};
pub use error::{DocTypeError, Error, UuidError, VersionError};
pub use export_file::{ExportExtension, ExportFilename, ExportFilenameError, FileBytes};
pub use fingerprint::KeyFingerprints;
pub use fixed_bytes::{ByteArray, DebugForm, FixedBytes, FixedBytesError, FixedRole};
#[cfg(feature = "fixtures")]
pub use fixtures::{
    BoardFrames, Connectors, DocBlocks, FIXTURE_IMPORT_PEER, FixtureGen, FixtureManifest,
    FixtureName, FixtureSeed, FixtureShape, Formulas, Generator, Peers, ShapeBody, Sheets,
    fixture_peer_count, seeded_document_id, seeded_id,
};
pub use flags::{Flags, FlagsKind, FlagsRepr};
pub use font::{
    FontFamily, FontPoints, FontSize, FontWeight, Slant, Strikethrough, Underline, Weight,
    WordBreak,
};
pub use id::{DocumentScoped, Entity, Id, IdRepr, TextKeyed};
pub use key_id::{KEY_ID_LEN, KeyId};
pub use lang::{LangTag, NotALangTag, primary_subtag};
pub use link::LinkUrl;
pub use locale::{
    CalendarNames, DateAmbiguity, DateOrder, DocumentLocale, Grouping, LocaleError, MonthDay,
    NumberConvention, NumberForms, ReaderLocale, TypedDate, resolve,
};
pub use media::{
    ContentHash, ContentHashError, KnownMediaType, MEDIA_ADDRESS_LEN, MediaAddress, MediaType,
    Octets,
};
pub use member_count::Members;
pub use mode::{
    AfterDrop, Availability, CaseMatch, Creation, DocumentScope, FinalizeEnd, HistoryStep,
    LinkState, PasteForm, RotationDebt, StepDirection, ThreadState, Toggle, Visibility, WordMatch,
};
pub use name::{
    DisplayName, InvalidDisplayName, NAME_MAX_CHARS, NAME_TOO_LONG, Name, NameTooLong,
    cut_display_name_to_fit, cut_name_to_fit, display_handle, name_fits, name_is_empty,
    name_plaintext, trimmed_name,
};
pub use object_key::ObjectKey;
pub use opaque::{Opaque, OpaqueKind, OpaqueLength, SealedAtRest, WireOpaque};
pub use ordinal::{Ordinal, OrdinalKind};
pub use payload::{
    Becomes, ClipboardBytes, ConverterFamily, ConverterModule, DeltaBytes, EphemeralBytes,
    FontBytes, FormatMagic, LoroPayload, LoroPayloadRef, MediaBytes, ModuleDigest, Payload,
    PayloadKind, PickedContainer, PickedFile, PlaintextContainer, RgbaPixels, SealedContainer,
    SealedPreviousKey, SealedRowBytes, SnapshotBytes, ThumbnailPng,
};
pub use provider::OidcProvider;
pub use public_key::{ExchangePublicKey, PublicKey};
pub use rank::{Rank, RankKind, RankRepr};
pub use redacted::Redacted;
pub use refused::{Refused, Refuses};
pub use role::{DocumentRank, OrganizationRank, Role};
pub use scale::Scale;
pub use schema::{
    SCHEMA_VERSION_ABSENT, Schema, SchemaError, SchemaMigration, SchemaVerdict, migrate_schema,
    migration_peer, scaffold_peer, verdict,
};
pub use search::{SearchQuery, UserSearch};
pub use secret::{
    AccessToken, Confidential, ConfidentialKind, Credential, CredentialKind, OidcCode, OidcState,
    Passphrase, Plaintext, PresignedUrl, RecoveryWords, RefreshToken, Secret, WireConfidential,
    WireCredential,
};
pub use seq::{
    Before, CountsSince, Deltas, Issuer, Relay, Seq, SeqColumn, SeqKind, SharedIssuer, Wire,
};
pub use space::{
    Board, CanvasCss, Css, Device, Doc, Image, ScalesTo, Space, StoredBoard, Surface, Texel, View,
};
pub use span::{
    Authored, ClockTick, ClockUnit, Day, Failures, FracDay, FracMs, FrameRate, Instant,
    LamportStamp, Months, Ms, Ns, PageLoad, Relayed, Sec, SheetDateTime, SheetDay, StoredAsI64,
    Tick, TimeBase, TimeSpan, TimeUnit, UnixMs, UnixSec,
};
pub use static_key::{
    AssetPaths, Attributes, Canvases, Categories, CommandNames, Commands, CssClasses, Icons,
    KeyKind, StaticKey, StorageKeys,
};
pub use status::{
    ApiFailure, HttpStatus, RefusalCode, RotationStatus, SnapshotStatus, StatusWord,
    UnreadableCause, WriteStatus,
};
pub use text_offset::{
    Byte, Char, RedactedChars, Utf16, byte_span_to_chars, byte_to_char, char_len,
    char_span_to_bytes, char_to_byte, char_to_utf16, utf16_to_char,
};
pub use unit::{
    Mm, PX_PER_MM, Point, PrintLength, PrintUnit, Pt, Px, Radians, Size, Vector, ViewSpeed,
};
pub use uuid::Uuid;
pub use validated::{Displayed, Rule, Validated};
pub use version::{Current, OpenRange, Version, VersionKind, VersionRepr};
