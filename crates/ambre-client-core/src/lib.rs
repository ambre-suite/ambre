//! The core types that the client alone uses, apart from `ambre-core`, which
//! the relay, the wire and the SFU link too (`ARCH-117`).
//!
//! - [`docfile`]: the `.ambre` container in its two lineages -- plaintext and
//!   passphrase-sealed -- shared by every application
//!   (`ambre-spec-docfile.md`)
//! - [`export_file`]: the name and the bytes of a file an import reads or an
//!   export writes
//! - [`locale`]: the document locale, the reader locale and the forms each one
//!   formats numbers and dates in (`ambre-spec-locale.md`)
//!
//! The markers of the container that `ambre-core` itself names stay there:
//! the version space, the salt role and the digest role
//! (`ambre_core::container`).

#![forbid(unsafe_code)]

pub mod docfile;
/// The name and the bytes of a file an import reads or an export writes.
pub mod export_file;
pub mod locale;

pub use docfile::{
    AMBRE_MAGIC, AMBRE_SEALED_VERSION, AMBRE_VERSION, AmbreHeader, ContainerFile, ContainerKind,
    DocFileError, HEADER_LEN, SEAL_NONCE_LEN, SEALED_AAD_LEN, SEALED_HEADER_LEN, SealedAmbre,
    classify, into_lineage, migrate_ambre, parse_sealed_header, read_ambre_plaintext,
    read_ambre_plaintext_as, sealed_aad, write_ambre_plaintext, write_ambre_sealed,
};
pub use export_file::{ExportExtension, ExportFilename, ExportFilenameError, FileBytes};
pub use locale::{
    CalendarNames, DateAmbiguity, DateOrder, DocumentLocale, Grouping, LocaleError, MonthDay,
    NumberConvention, NumberForms, ReaderLocale, TypedDate, resolve,
};
