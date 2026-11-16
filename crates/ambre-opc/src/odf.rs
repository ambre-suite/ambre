//! The OpenDocument package of ODF 1.3, part 2. It is the same ZIP archive
//! under the same bounds. A `mimetype` entry and a manifest index it, in
//! place of the content types and the relationships.
//!
//! The spreadsheet and the text codecs read and write it, and the
//! presentation codec reads it, each under its own [`OdfType`].

use std::io::{Cursor, Write};

use ambre_core::Payload;

use crate::{Archive, OpcError, PackageBytes, PartBytes, PartName, xml};

/// The name of the entry that names the document type, stored first.
const MIMETYPE: &str = "mimetype";

/// The name of the manifest, which lists every part with its media type.
const MANIFEST: &str = "META-INF/manifest.xml";

/// A media type of the OpenDocument package: the type of a whole document,
/// as its `mimetype` entry spells it, or the type of one part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OdfType(&'static str);

impl OdfType {
    /// A spreadsheet document, `.ods`.
    pub const SPREADSHEET: Self = Self("application/vnd.oasis.opendocument.spreadsheet");
    /// A text document, `.odt`.
    pub const TEXT: Self = Self("application/vnd.oasis.opendocument.text");
    /// A presentation document, `.odp`.
    pub const PRESENTATION: Self = Self("application/vnd.oasis.opendocument.presentation");
    /// An XML part of the package: the content, the styles, the metadata.
    pub const XML: Self = Self("text/xml");

    /// The media type of a picture part, as a codec names its encoding: a
    /// manifest lists each part with its type.
    #[must_use]
    pub const fn picture(media_type: &'static str) -> Self {
        Self(media_type)
    }

    /// The media type as the package spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// An open OpenDocument package: the archive, its type checked. Its index
/// is the manifest, and a reader names the parts it reads, so the package
/// reads no index beyond the type.
pub struct OdfPackage<'a> {
    archive: Archive<'a>,
}

impl OdfPackage<'_> {
    /// The name of every part, as [`Archive::part_names`] lists them.
    #[must_use]
    pub fn part_names(&self) -> Vec<PartName> {
        self.archive.part_names()
    }

    /// The bytes of the part `name`, as [`Archive::part`] reads them.
    ///
    /// # Errors
    ///
    /// Every error of [`Archive::part`].
    pub fn part(&mut self, name: &PartName) -> Result<Option<Payload<PartBytes>>, OpcError> {
        self.archive.part(name)
    }
}

/// The package `bytes` hold, when it is a document of the type `kind`.
///
/// The `mimetype` entry names the type. A package without one names it in
/// the root entry of its manifest, which the format allows.
///
/// # Errors
///
/// Every error of [`Archive::open`]. [`OpcError::OtherDocument`] for a
/// package of another type, and [`OpcError::MissingPart`] for a package that
/// names no type.
pub fn open(bytes: &[u8], kind: OdfType) -> Result<OdfPackage<'_>, OpcError> {
    let mut archive = Archive::open(bytes)?;
    let named = match archive.part(&PartName(MIMETYPE.to_owned()))? {
        Some(entry) => String::from_utf8_lossy(entry.as_bytes()).trim().to_owned(),
        None => manifest_root_type(&mut archive)?.ok_or(OpcError::MissingPart(MIMETYPE))?,
    };
    if named != kind.as_str() {
        return Err(OpcError::OtherDocument);
    }
    Ok(OdfPackage { archive })
}

/// The media type the manifest gives the package root, if it gives one.
fn manifest_root_type(archive: &mut Archive<'_>) -> Result<Option<String>, OpcError> {
    let Some(bytes) = archive.part(&PartName(MANIFEST.to_owned()))? else {
        return Ok(None);
    };
    let entries = xml::elements(bytes.as_bytes()).map_err(|_| OpcError::Malformed(MANIFEST))?;
    Ok(entries
        .into_iter()
        .filter(|entry| entry.name == "file-entry")
        .find(|entry| entry.attr("full-path") == Some("/"))
        .and_then(|entry| entry.attr("media-type").map(str::to_owned)))
}

/// An OpenDocument package being written: its type and its parts. The
/// archive it writes is the same for the same parts, every entry dated at
/// the epoch of a ZIP archive.
pub struct OdfWriter {
    kind: OdfType,
    parts: Vec<(PartName, OdfType, Payload<PartBytes>)>,
}

impl OdfWriter {
    /// An empty package of the type `kind`.
    #[must_use]
    pub const fn new(kind: OdfType) -> Self {
        Self {
            kind,
            parts: Vec::new(),
        }
    }

    /// Add the part `name` of the media type `kind`, holding `bytes`.
    pub fn part(&mut self, name: PartName, kind: OdfType, bytes: Payload<PartBytes>) {
        self.parts.push((name, kind, bytes));
    }

    /// The archive. The `mimetype` entry comes first and stored, where a
    /// reader that sniffs the type looks for it. The manifest follows, then
    /// every part, deflated.
    ///
    /// # Errors
    ///
    /// [`OpcError::WriteFailed`] when the ZIP writer fails.
    pub fn finish(self) -> Result<Payload<PackageBytes>, OpcError> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default());
        let deflated = stored.compression_method(zip::CompressionMethod::Deflated);
        let mut put = |name: &str,
                       options: zip::write::SimpleFileOptions,
                       bytes: &[u8]|
         -> Result<(), OpcError> {
            zip.start_file(name, options)
                .map_err(|_| OpcError::WriteFailed)?;
            zip.write_all(bytes).map_err(|_| OpcError::WriteFailed)
        };
        put(MIMETYPE, stored, self.kind.as_str().as_bytes())?;
        put(MANIFEST, deflated, self.manifest().as_bytes())?;
        for (name, _, bytes) in &self.parts {
            put(name.as_str(), deflated, bytes.as_bytes())?;
        }
        let cursor = zip.finish().map_err(|_| OpcError::WriteFailed)?;
        Ok(Payload::new(cursor.into_inner()))
    }

    fn manifest(&self) -> Payload<PartBytes> {
        let mut out = xml::XmlWriter::new();
        out.open(
            "manifest:manifest",
            &[
                (
                    "xmlns:manifest",
                    "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0",
                ),
                ("manifest:version", "1.3"),
            ],
        );
        out.empty(
            "manifest:file-entry",
            &[
                ("manifest:full-path", "/"),
                ("manifest:version", "1.3"),
                ("manifest:media-type", self.kind.as_str()),
            ],
        );
        for (name, kind, _) in &self.parts {
            out.empty(
                "manifest:file-entry",
                &[
                    ("manifest:full-path", name.as_str()),
                    ("manifest:media-type", kind.as_str()),
                ],
            );
        }
        out.close("manifest:manifest");
        out.finish()
    }
}
