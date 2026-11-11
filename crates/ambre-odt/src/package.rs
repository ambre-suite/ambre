//! The OpenDocument package: a ZIP archive whose `mimetype` entry names the
//! document kind and whose `META-INF/manifest.xml` lists the parts.
//!
//! The reader inflates a part only when asked, and holds each part and the
//! whole package under the bounds of the office packages
//! (`ambre-spec-budgets.md section 6`), checked while the bytes inflate.

use std::io::{Cursor, Read, Write};

use ambre_core::{FileBytes, Payload};
use ambre_docx::model::ImageBytes;

use crate::OdtError;
use crate::read::Pictures;
use crate::write::WrittenParts;
use crate::xml::{PartBytes, XmlWriter};

/// The most entries a package may hold, as the office packages bound it.
const MAX_PARTS: usize = 4096;
/// The most bytes one part may inflate to.
const MAX_PART_SIZE: u64 = 32 * 1024 * 1024;
/// The most bytes every part read may inflate to, together.
const MAX_INFLATED: u64 = 64 * 1024 * 1024;

/// The eight bytes an OLE compound file opens on: a legacy or an encrypted
/// office document.
const COMPOUND_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

pub struct Package<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
    inflated: u64,
    media_type: Option<String>,
}

impl<'a> Package<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, OdtError> {
        if bytes.starts_with(&COMPOUND_FILE) {
            return Err(OdtError::NotText);
        }
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| OdtError::NotAZip)?;
        if archive.len() > MAX_PARTS {
            return Err(OdtError::TooManyParts);
        }
        let mut package = Self {
            archive,
            inflated: 0,
            media_type: None,
        };
        package.media_type = package
            .part("mimetype")?
            .and_then(|bytes| String::from_utf8(bytes.as_bytes().to_vec()).ok())
            .map(|text| text.trim().to_owned());
        Ok(package)
    }

    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }

    /// Whether the package carries a macro library or a script.
    pub fn has_macros(&self) -> bool {
        self.archive
            .file_names()
            .filter_map(Result::ok)
            .any(|name| name.starts_with("Basic/") || name.starts_with("Scripts/"))
    }

    pub fn part(&mut self, name: &str) -> Result<Option<Payload<PartBytes>>, OdtError> {
        let Some(at) = self.archive.index_for_name(name) else {
            return Ok(None);
        };
        let file = self
            .archive
            .by_index(at)
            .map_err(|_| OdtError::UnreadablePart)?;
        if file.encrypted() {
            return Err(OdtError::UnreadablePart);
        }
        let cap = MAX_PART_SIZE.min(MAX_INFLATED.saturating_sub(self.inflated));
        let mut bytes = Vec::new();
        // One byte past the cap tells a part at the cap from one past it,
        // whatever size the archive's header claims.
        file.take(cap.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| OdtError::UnreadablePart)?;
        let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if read > cap {
            return Err(OdtError::TooLarge);
        }
        self.inflated = self.inflated.saturating_add(read);
        Ok(Some(Payload::new(bytes)))
    }
}

impl Pictures for Package<'_> {
    fn picture(&mut self, path: &str) -> Result<Option<Payload<ImageBytes>>, OdtError> {
        Ok(self
            .part(path)?
            .map(|bytes| Payload::new(bytes.as_bytes().to_vec())))
    }
}

/// The archive of `parts`: the `mimetype` entry first and stored, as ODF
/// requires, then the manifest and the parts. Every entry carries the one
/// date of a ZIP archive's epoch, so the same parts write the same bytes.
pub fn write(parts: &WrittenParts) -> Result<Payload<FileBytes>, OdtError> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    let deflated = stored.compression_method(zip::CompressionMethod::Deflated);
    let failed = |_| OdtError::WriteFailed;
    zip.start_file("mimetype", stored).map_err(failed)?;
    zip.write_all(crate::MEDIA_TYPE.as_bytes())
        .map_err(|_| OdtError::WriteFailed)?;
    let mut put = |name: &str, bytes: &[u8]| -> Result<(), OdtError> {
        zip.start_file(name, deflated)
            .map_err(|_| OdtError::WriteFailed)?;
        zip.write_all(bytes).map_err(|_| OdtError::WriteFailed)
    };
    put("META-INF/manifest.xml", manifest(parts).as_bytes())?;
    put("content.xml", parts.content.as_bytes())?;
    put("styles.xml", parts.styles.as_bytes())?;
    put("meta.xml", parts.meta.as_bytes())?;
    for (path, _, bytes) in &parts.pictures {
        put(path, bytes.as_bytes())?;
    }
    let cursor = zip.finish().map_err(|_| OdtError::WriteFailed)?;
    Ok(Payload::new(cursor.into_inner()))
}

fn manifest(parts: &WrittenParts) -> Payload<PartBytes> {
    let mut xml = XmlWriter::new();
    xml.open(
        "manifest:manifest",
        &[
            (
                "xmlns:manifest",
                "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0",
            ),
            ("manifest:version", "1.3"),
        ],
    );
    xml.empty(
        "manifest:file-entry",
        &[
            ("manifest:full-path", "/"),
            ("manifest:version", "1.3"),
            ("manifest:media-type", crate::MEDIA_TYPE),
        ],
    );
    for name in ["content.xml", "styles.xml", "meta.xml"] {
        xml.empty(
            "manifest:file-entry",
            &[
                ("manifest:full-path", name),
                ("manifest:media-type", "text/xml"),
            ],
        );
    }
    for (path, media_type, _) in &parts.pictures {
        xml.empty(
            "manifest:file-entry",
            &[
                ("manifest:full-path", path),
                ("manifest:media-type", media_type),
            ],
        );
    }
    xml.close("manifest:manifest");
    xml.finish()
}
