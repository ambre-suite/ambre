//! Why a conversion produced nothing, as closed sets the threads share.
//!
//! Three threads meet a conversion failure, each in its own terms. The
//! converter module meets a codec error, the Logic thread a trap of the
//! module, the Sync worker a fetch that did not arrive. Each one maps its failure to a variant here and logs
//! the detail where it meets it. The bus carries the variant alone, and the
//! view makes its words (`BUS-046`). A variant quotes nothing of the file, so
//! no text of a document rides with the failure (`INTEROP-004`).

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::status::ApiFailure;

/// Why the bytes of a file did not read as the format named. A module that
/// could not write its file answers one of these too.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileRefusal {
    /// The request that crossed into the module did not decode.
    Request,
    /// The bytes are not UTF-8 text.
    NotText,
    /// The file is a legacy or a password-protected file.
    Protected,
    /// The file is not the package the format names: no ZIP archive.
    NotAPackage,
    /// The package holds another kind of document than the format named.
    OtherDocument,
    /// A part the format requires is missing.
    PartMissing,
    /// A part, the markup or a record does not parse.
    Malformed,
    /// The file passes a safety bound: too many parts, cells or bytes.
    PastBound,
    /// The file holds nothing to convert: no sheet, no slide.
    Empty,
    /// The time zone the conversion names is not one this build knows.
    UnknownZone,
    /// The module could not write the file.
    Unwritable,
    /// The module loaded does not carry the family of the conversion.
    NotCarried,
}

impl fmt::Display for FileRefusal {
    /// The words a person reads for the refusal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Request => "the converter could not read the request",
            Self::NotText => "the file is not UTF-8 text",
            Self::Protected => "the file is a legacy or password-protected file",
            Self::NotAPackage => "the file is not a package of this format",
            Self::OtherDocument => "the file holds another kind of document",
            Self::PartMissing => "a part the format requires is missing",
            Self::Malformed => "a part of the file does not read",
            Self::PastBound => "the file is larger than a conversion accepts",
            Self::Empty => "the file holds nothing to convert",
            Self::UnknownZone => "the time zone is not one this build knows",
            Self::Unwritable => "the file could not be written",
            Self::NotCarried => "the converter loaded does not convert that format",
        })
    }
}

/// The debug form is the words of [`fmt::Display`]: one table of words, which
/// the Main instance carries once (`ambre-spec-budgets.md section 5`).
impl fmt::Debug for FileRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Why the Sync worker did not hand over the converter module of a family.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConverterRefusal {
    /// The build names no digest for the family.
    NoDigest,
    /// The worker runs on a transport that fetches nothing.
    NoFetch,
    /// The fetch got no module.
    NotFetched,
    /// The bytes fetched are not the ones the build names.
    DigestMismatch,
    /// The module arrived and did not load.
    NotLoaded,
}

impl fmt::Display for ConverterRefusal {
    /// The words a person reads for the refusal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoDigest => "this build names no converter of that kind",
            Self::NoFetch => "this session fetches no converter",
            Self::NotFetched => "the converter did not download",
            Self::DigestMismatch => "the converter downloaded is not the one this build names",
            Self::NotLoaded => "the converter did not load",
        })
    }
}

/// The debug form is the words of [`fmt::Display`]: one table of words, which
/// the Main instance carries once (`ambre-spec-budgets.md section 5`).
impl fmt::Debug for ConverterRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Why a conversion stopped after it read the request, or never started.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    /// The converter module is not loaded, and this conversion waits on it.
    ConverterUnavailable,
    /// The converter module is still loading, and its queue is full.
    ConverterBusy,
    /// The Sync worker did not hand over the module, for this reason.
    ConverterRefused(ConverterRefusal),
    /// The module trapped, or could not be called.
    ConverterStopped,
    /// The module refused the file or the write, for this reason.
    FileRefused(FileRefusal),
    /// The module answered bytes no request reads, or another answer than
    /// the one the request asks for.
    ConverterAnswered,
    /// The module loaded serves another family than the conversion.
    OtherFamily,
    /// Another thread or another path serves this conversion, and the command
    /// reached the wrong one.
    Misrouted,
    /// This build carries no writer for the format.
    NotInBuild,
    /// The format is not available yet.
    NotAvailable,
    /// A font the pages need is still loading.
    FontsLoading,
    /// The document holds nothing to convert.
    NothingToConvert,
    /// The sheet or the element the conversion names is gone.
    SourceGone,
    /// The engine could not read the document it is asked to write.
    DocumentUnreadable,
    /// The form between the document and the module did not encode or
    /// decode.
    FormUnreadable,
    /// The file could not be sealed for the native format.
    SealFailed,
    /// The conversion passes a safety bound: elements, depth or pixels.
    PastBound,
    /// The upload of a media file the conversion carries failed.
    UploadFailed(ApiFailure),
}

impl fmt::Display for StopCause {
    /// The words a person reads for the stop.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConverterUnavailable => f.write_str("the converter is not loaded"),
            Self::ConverterBusy => {
                f.write_str("the converter is still loading: try again in a moment")
            }
            Self::ConverterRefused(refusal) => refusal.fmt(f),
            Self::ConverterStopped => f.write_str("the converter stopped"),
            Self::FileRefused(refusal) => refusal.fmt(f),
            Self::ConverterAnswered => f.write_str("the converter answered no file"),
            Self::OtherFamily => f.write_str("the converter loaded serves another format"),
            Self::Misrouted => f.write_str("this file is converted elsewhere"),
            Self::NotInBuild => f.write_str("this build cannot write that format"),
            Self::NotAvailable => f.write_str("that format is not available yet"),
            Self::FontsLoading => f.write_str("the fonts are still loading"),
            Self::NothingToConvert => f.write_str("there is nothing to convert"),
            Self::SourceGone => f.write_str("what was to be converted no longer exists"),
            Self::DocumentUnreadable => f.write_str("the document could not be read"),
            Self::FormUnreadable => f.write_str("the document did not encode for the converter"),
            Self::SealFailed => f.write_str("the file could not be encrypted"),
            Self::PastBound => f.write_str("the file passes a safety limit"),
            Self::UploadFailed(failure) => write!(f, "a media upload failed: {failure}"),
        }
    }
}

/// The debug form is the words of [`fmt::Display`]: one table of words, which
/// the Main instance carries once (`ambre-spec-budgets.md section 5`).
impl fmt::Debug for StopCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
