//! The OpenDocument text codec (`ambre-spec-prose-import-export.md
//! section 3`).
//!
//! Two pure functions over the Word model of `ambre-docx`. [`read`] turns
//! the bytes of an `.odt` package into a [`WordDocument`] and a report of
//! what it approximated or dropped, in the words of the Word reader.
//! [`write`] turns a [`WordDocument`] back into bytes. Neither reads a clock,
//! a file or the network, and neither knows an application.
//!
//! The model is Word's on purpose. An OpenDocument text and the same text
//! saved from Word then meet one mapping onto Prose's form, and one report:
//! the second format cannot drift from the first (`PROSE-IO-088`).
//!
//! The `codec` feature carries the reader, the writer and their ZIP and XML
//! dependencies. Without it the crate is its error alone.

#[cfg(feature = "codec")]
mod package;
#[cfg(feature = "codec")]
pub mod read;
#[cfg(feature = "codec")]
pub mod write;
#[cfg(feature = "codec")]
pub mod xml;

#[cfg(feature = "codec")]
use ambre_core::{FileBytes, Payload};
#[cfg(feature = "codec")]
pub use ambre_docx::model::WordDocument;
#[cfg(feature = "codec")]
use ambre_docx::report::WordReport;

/// The media type of an OpenDocument text, which its `mimetype` part spells.
pub const MEDIA_TYPE: &str = "application/vnd.oasis.opendocument.text";

/// Why a package produced no document. No variant quotes the file: its
/// bytes and its part names are the user's content (`INTEROP-004`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OdtError {
    #[error("the file is not a ZIP package")]
    NotAZip,
    #[error("the package holds no OpenDocument text")]
    NotText,
    #[error("a required part is missing: {0}")]
    MissingPart(&'static str),
    #[error("a part is encrypted or stored in a way the reader does not read")]
    UnreadablePart,
    #[error("the package holds more parts than the reader opens")]
    TooManyParts,
    #[error("a part inflates past the bound the reader holds")]
    TooLarge,
    #[cfg(feature = "codec")]
    #[error(transparent)]
    Xml(#[from] xml::XmlError),
    #[error("the package could not be written")]
    WriteFailed,
}

/// A document read, and what the reading carried and lost.
#[cfg(feature = "codec")]
#[derive(Debug)]
pub struct Decoded {
    pub document: WordDocument,
    pub report: WordReport,
}

/// The document an `.odt` or `.ott` package holds.
///
/// # Errors
///
/// An [`OdtError`] when the bytes are no OpenDocument text, or a part the
/// document needs does not read. A package past a bound fails too, before it
/// holds the memory it asks for.
#[cfg(feature = "codec")]
pub fn read(bytes: &[u8]) -> Result<Decoded, OdtError> {
    let mut package = package::Package::open(bytes)?;
    if !package.media_type().is_some_and(is_text) {
        return Err(OdtError::NotText);
    }
    // The reader never opens a macro library: it names it alone.
    let macros = package.has_macros();
    let content = package
        .part("content.xml")?
        .ok_or(OdtError::MissingPart("content.xml"))?;
    let content = xml::parse(&content)?;
    let styles = match package.part("styles.xml")? {
        Some(bytes) => Some(xml::parse(&bytes)?),
        None => None,
    };
    let meta = match package.part("meta.xml")? {
        Some(bytes) => Some(xml::parse(&bytes)?),
        None => None,
    };
    let parts = read::Parts {
        content: &content,
        styles: styles.as_ref(),
        meta: meta.as_ref(),
    };
    let (document, report) = read::read_parts(&parts, &mut package, macros)?;
    Ok(Decoded { document, report })
}

/// The bytes of `document` as an `.odt` package. The same document writes
/// the same bytes.
///
/// # Errors
///
/// [`OdtError::WriteFailed`] when the ZIP writer fails.
#[cfg(feature = "codec")]
pub fn write(document: &WordDocument) -> Result<Payload<FileBytes>, OdtError> {
    let parts = write::write_parts(document);
    package::write(&parts)
}

/// Whether a media type names a text or a text template.
#[cfg(feature = "codec")]
fn is_text(media_type: &str) -> bool {
    matches!(
        media_type,
        MEDIA_TYPE | "application/vnd.oasis.opendocument.text-template"
    )
}
