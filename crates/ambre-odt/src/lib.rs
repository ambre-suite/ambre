//! The OpenDocument text codec (`ambre-spec-prose-import-export.md
//! section 3`).
//!
//! Two pure functions over the Word model of `ambre-docx`. [`read`] turns
//! the bytes of an `.odt` package into a [`WordDocument`] and a report. The
//! report names what the reader approximated or dropped, in the words of the
//! Word reader. [`write`] turns a [`WordDocument`] back into bytes. Neither
//! reads a clock, a file or the network, and neither knows an application.
//!
//! The model is Word's on purpose. An OpenDocument text and its Word twin
//! meet one mapping onto Prose's form and one report. So the second format
//! cannot drift from the first.
//!
//! The `codec` feature carries the reader, the writer and their ZIP and XML
//! dependencies. Without it the crate is its error alone.

#[cfg(feature = "codec")]
pub mod read;
#[cfg(feature = "codec")]
pub mod write;
#[cfg(feature = "codec")]
pub mod xml;

#[cfg(feature = "codec")]
use ambre_client_core::FileBytes;
#[cfg(feature = "codec")]
use ambre_core::Payload;
#[cfg(feature = "codec")]
pub use ambre_docx::model::WordDocument;
#[cfg(feature = "codec")]
use ambre_docx::report::WordReport;
#[cfg(feature = "codec")]
use ambre_opc::PartName;
#[cfg(feature = "codec")]
use ambre_opc::odf::{self, OdfType, OdfWriter};

/// Why a package produced no document. No variant quotes the file: its
/// bytes and its part names are the user's content (`INTEROP-004`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OdtError {
    #[cfg(feature = "codec")]
    #[error(transparent)]
    Package(#[from] ambre_opc::OpcError),
    #[cfg(feature = "codec")]
    #[error(transparent)]
    Xml(#[from] xml::XmlError),
    #[error("the package holds no OpenDocument text")]
    NotText,
    #[error("a required part is missing: {0}")]
    MissingPart(&'static str),
    /// The walk of the document visits more nodes or writes more text than
    /// its bound: copies of a repeated row, cell or space mark.
    #[error("the document builds past its bound")]
    TooLarge,
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
    let mut package = odf::open(bytes, OdfType::TEXT)?;
    // The reader never opens a macro library or a script: it names it alone.
    let macros = package.part_names().iter().any(|name| {
        let name = name.as_str().to_ascii_lowercase();
        name.starts_with("basic/") || name.starts_with("scripts/")
    });
    let content = package
        .part(&part_name("content.xml")?)?
        .ok_or(OdtError::MissingPart("content.xml"))?;
    let content = xml::parse(&content)?;
    let styles = match package.part(&part_name("styles.xml")?)? {
        Some(bytes) => Some(xml::parse(&bytes)?),
        None => None,
    };
    let meta = match package.part(&part_name("meta.xml")?)? {
        Some(bytes) => Some(xml::parse(&bytes)?),
        None => None,
    };
    let parts = read::Parts {
        content: &content,
        styles: styles.as_ref(),
        meta: meta.as_ref(),
    };
    let mut pictures = PackagePictures(&mut package);
    let (document, report) = read::read_parts(&parts, &mut pictures, macros)?;
    Ok(Decoded { document, report })
}

/// The bytes of `document` as an `.odt` package. The same document writes
/// the same bytes.
///
/// # Errors
///
/// [`OdtError::Package`] when the ZIP writer fails.
#[cfg(feature = "codec")]
pub fn write(document: &WordDocument) -> Result<Payload<FileBytes>, OdtError> {
    let parts = write::write_parts(document);
    let mut package = OdfWriter::new(OdfType::TEXT);
    package.part(part_name("content.xml")?, OdfType::XML, parts.content);
    package.part(part_name("styles.xml")?, OdfType::XML, parts.styles);
    package.part(part_name("meta.xml")?, OdfType::XML, parts.meta);
    for (path, media_type, bytes) in parts.pictures {
        package.part(part_name(&path)?, OdfType::picture(media_type), bytes);
    }
    Ok(package.finish()?.into_role())
}

#[cfg(feature = "codec")]
fn part_name(name: &str) -> Result<PartName, OdtError> {
    PartName::new(name).ok_or(OdtError::MissingPart("a part name"))
}

/// The pictures of an open package, which the reader asks for by path.
#[cfg(feature = "codec")]
struct PackagePictures<'p, 'a>(&'p mut odf::OdfPackage<'a>);

#[cfg(feature = "codec")]
impl read::Pictures for PackagePictures<'_, '_> {
    fn picture(
        &mut self,
        path: &str,
    ) -> Result<Option<Payload<ambre_docx::model::ImageBytes>>, OdtError> {
        let Some(name) = PartName::new(path) else {
            return Ok(None);
        };
        Ok(self
            .0
            .part(&name)?
            .map(|bytes| Payload::new(bytes.as_bytes().to_vec())))
    }
}
