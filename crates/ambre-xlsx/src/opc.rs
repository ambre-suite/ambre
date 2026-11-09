//! The OPC package (ECMA-376 part 2): the zip archive, its content types and
//! its relationships. Nothing here knows a spreadsheet.
//!
//! The layer is small on purpose. DOCX, xlsx and PPTX share it, and the three
//! codecs will move it into one crate when the OOXML family module is wired.
//! The API is five reads and three writes:
//!
//! * [`Package::read`], [`Package::part`], [`Package::relationships`],
//!   [`Package::main_part`] and [`Package::part_names`];
//! * [`PackageWriter::add_part`], [`PackageWriter::relate`] and
//!   [`PackageWriter::finish`].

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use ambre_core::{Count, FileBytes, Octets, Payload};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::xml::{self, XmlError};

/// The most one part may inflate to: 256 MiB. A sheet of a million cells
/// stays under 100 MiB of XML; past the cap, the archive is a zip bomb.
pub const MAX_PART: Count<Octets> = Count::new(256 << 20);

/// The most a whole package may inflate to: 512 MiB.
pub const MAX_PACKAGE: Count<Octets> = Count::new(512 << 20);

/// The most entries an archive may list. A workbook of a hundred sheets with
/// their drawings lists a few hundred.
const MAX_ENTRIES: usize = 10_000;

/// Why a package did not read or did not write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpcError {
    /// The bytes are no zip archive, or the archive is damaged.
    NotAnArchive(String),
    /// The archive holds an encrypted entry. An Excel file under a password
    /// is an encrypted compound file, not a zip, and lands here too.
    Encrypted,
    /// A part or the whole archive inflates past its cap.
    TooLarge(Count<Octets>),
    /// A part the package needs is absent.
    MissingPart(PartName),
    /// A content-type or relationship part did not read.
    Malformed(PartName, XmlError),
}

impl std::fmt::Display for OpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnArchive(reason) => write!(f, "the file is no zip archive: {reason}"),
            Self::Encrypted => f.write_str("the file is encrypted"),
            Self::TooLarge(size) => write!(f, "the file inflates past {} bytes", size.get()),
            Self::MissingPart(name) => write!(f, "the file lacks its part {}", name.as_str()),
            Self::Malformed(name, e) => {
                write!(f, "the part {} did not read: {}", name.as_str(), e.reason())
            }
        }
    }
}

impl std::error::Error for OpcError {}

/// The name of a part, as the archive lists it: no leading slash, forward
/// slashes alone, and no `.` or `..` segment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartName(String);

impl PartName {
    /// The part `name` names, a leading slash and backslashes normalized, or
    /// `None` for an empty name or one that climbs out of the package.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        let name = name.replace('\\', "/");
        let name = name.trim_start_matches('/');
        let sound = !name.is_empty()
            && name
                .split('/')
                .all(|seg| !seg.is_empty() && seg != "." && seg != "..");
        sound.then(|| Self(name.to_owned()))
    }

    /// The name as the archive lists it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The key a lookup compares: part names compare case-insensitively.
    fn key(&self) -> String {
        self.0.to_ascii_lowercase()
    }

    /// The relationship part of this part: `xl/_rels/workbook.xml.rels` for
    /// `xl/workbook.xml`.
    fn rels(&self) -> Self {
        match self.0.rsplit_once('/') {
            Some((dir, file)) => Self(format!("{dir}/_rels/{file}.rels")),
            None => Self(format!("_rels/{}.rels", self.0)),
        }
    }

    /// The folder the part sits in, with a trailing slash, or empty at the
    /// root.
    fn folder(&self) -> &str {
        self.0.rsplit_once('/').map_or("", |(dir, _)| dir)
    }

    /// The part a relationship target names, read from this part's folder.
    #[must_use]
    pub fn resolve(source: Option<&Self>, target: &str) -> Option<Self> {
        if let Some(absolute) = target.strip_prefix('/') {
            return Self::new(absolute);
        }
        let mut segments: Vec<&str> = source
            .map(Self::folder)
            .unwrap_or_default()
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        for seg in target.split(['/', '\\']) {
            match seg {
                "" | "." => {}
                ".." => {
                    segments.pop()?;
                }
                seg => segments.push(seg),
            }
        }
        Self::new(&segments.join("/"))
    }
}

/// The type of a relationship, as the URI that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelKind(&'static str);

impl RelKind {
    /// The relationship type `uri` names.
    #[must_use]
    pub const fn new(uri: &'static str) -> Self {
        Self(uri)
    }

    /// The URI of the relationship, under the transitional namespace.
    #[must_use]
    pub const fn uri(self) -> &'static str {
        self.0
    }

    /// Whether `uri` names this relationship, under the transitional or the
    /// strict namespace: both end with the same last segment.
    #[must_use]
    pub fn matches(self, uri: &str) -> bool {
        let last = |u: &str| u.rsplit('/').next().map(str::to_owned);
        last(self.0) == last(uri)
    }

    /// The main part of a package: a workbook, a document or a presentation.
    pub const OFFICE_DOCUMENT: Self =
        Self("http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument");
    pub const CORE_PROPERTIES: Self = Self(
        "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
    );
    pub const EXTENDED_PROPERTIES: Self = Self(
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties",
    );
}

/// One relationship of a part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    /// The id the source part names the target by, such as `rId1`.
    pub id: String,
    /// The URI of its type, as the file wrote it.
    pub kind: String,
    pub target: Target,
}

/// Where a relationship points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A part of the package.
    Part(PartName),
    /// A resource outside it: a web address, another file.
    External(String),
}

/// A package read from its bytes. The reader inflates the XML parts up front,
/// under [`MAX_PART`] and [`MAX_PACKAGE`]. It lists every other part by name
/// alone, so a picture costs nothing to a reader that drops it.
#[derive(Debug, Clone)]
pub struct Package {
    xml_parts: BTreeMap<String, (PartName, Vec<u8>)>,
    names: Vec<PartName>,
}

impl Package {
    /// The package `bytes` hold.
    ///
    /// # Errors
    ///
    /// [`OpcError::NotAnArchive`] for bytes no zip reader reads,
    /// [`OpcError::Encrypted`] for an encrypted entry, and
    /// [`OpcError::TooLarge`] for a part or a package past its cap.
    pub fn read(bytes: &Payload<FileBytes>) -> Result<Self, OpcError> {
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_bytes()))
            .map_err(|e| OpcError::NotAnArchive(e.to_string()))?;
        if archive.len() > MAX_ENTRIES {
            return Err(OpcError::NotAnArchive(
                "the archive lists too many entries".to_owned(),
            ));
        }
        let mut xml_parts = BTreeMap::new();
        let mut names = Vec::new();
        let mut inflated = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|e| OpcError::NotAnArchive(e.to_string()))?;
            if entry.is_dir() {
                continue;
            }
            if entry.encrypted() {
                return Err(OpcError::Encrypted);
            }
            let raw_name = entry
                .name()
                .map_err(|e| OpcError::NotAnArchive(e.to_string()))?
                .into_owned();
            let Some(name) = PartName::new(&raw_name) else {
                continue;
            };
            if is_xml_name(&name) {
                let mut data = Vec::new();
                let cap = MAX_PART.get() + 1;
                entry
                    .by_ref()
                    .take(cap)
                    .read_to_end(&mut data)
                    .map_err(|e| OpcError::NotAnArchive(e.to_string()))?;
                let size = u64::try_from(data.len()).unwrap_or(u64::MAX);
                inflated = inflated.saturating_add(size);
                if size > MAX_PART.get() {
                    return Err(OpcError::TooLarge(MAX_PART));
                }
                if inflated > MAX_PACKAGE.get() {
                    return Err(OpcError::TooLarge(MAX_PACKAGE));
                }
                xml_parts.insert(name.key(), (name.clone(), data));
            }
            names.push(name);
        }
        Ok(Self { xml_parts, names })
    }

    /// The bytes of the XML part `name`, or `None` when the package holds no
    /// such XML part.
    #[must_use]
    pub fn part(&self, name: &PartName) -> Option<&[u8]> {
        self.xml_parts
            .get(&name.key())
            .map(|(_, data)| data.as_slice())
    }

    /// Every part the archive lists, XML or not, in archive order.
    #[must_use]
    pub fn part_names(&self) -> &[PartName] {
        &self.names
    }

    /// The relationships of `source`, or of the package itself for `None`.
    /// A part with no relationship part has none.
    ///
    /// # Errors
    ///
    /// [`OpcError::Malformed`] for a relationship part that does not read.
    pub fn relationships(&self, source: Option<&PartName>) -> Result<Vec<Relationship>, OpcError> {
        let rels = source.map_or(PartName("_rels/.rels".to_owned()), PartName::rels);
        let Some(bytes) = self.part(&rels) else {
            return Ok(Vec::new());
        };
        let root = xml::parse_tree(bytes).map_err(|e| OpcError::Malformed(rels.clone(), e))?;
        Ok(root
            .children("Relationship")
            .filter_map(|r| {
                let target = r.attr("Target")?;
                let target = if r.attr("TargetMode") == Some("External") {
                    Target::External(target.to_owned())
                } else {
                    Target::Part(PartName::resolve(source, target)?)
                };
                Some(Relationship {
                    id: r.attr("Id")?.to_owned(),
                    kind: r.attr("Type")?.to_owned(),
                    target,
                })
            })
            .collect())
    }

    /// The part the package-level relationship of `kind` points to.
    ///
    /// # Errors
    ///
    /// [`OpcError::MissingPart`] when no relationship of `kind` points into
    /// the package, and [`OpcError::Malformed`] for a relationship part that
    /// does not read.
    pub fn main_part(&self, kind: RelKind) -> Result<PartName, OpcError> {
        self.relationships(None)?
            .into_iter()
            .find_map(|r| match r.target {
                Target::Part(name) if kind.matches(&r.kind) => Some(name),
                _ => None,
            })
            .ok_or_else(|| OpcError::MissingPart(PartName("_rels/.rels".to_owned())))
    }
}

/// Whether the reader inflates the part: an XML part or a relationship part.
fn is_xml_name(name: &PartName) -> bool {
    let key = name.key();
    key.ends_with(".xml") || key.ends_with(".rels")
}

/// A package being written: its parts, their content types and their
/// relationships. The archive is written by [`PackageWriter::finish`] in the
/// order the parts were added, with no timestamp, so one model writes one
/// sequence of bytes.
#[derive(Debug, Default)]
pub struct PackageWriter {
    parts: Vec<(PartName, &'static str, Vec<u8>)>,
    rels: BTreeMap<Option<PartName>, Vec<(String, RelKind, String)>>,
}

impl PackageWriter {
    /// An empty package.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the part `name` of `content_type`. A second part of one name
    /// replaces the first.
    pub fn add_part(&mut self, name: PartName, content_type: &'static str, bytes: Vec<u8>) {
        self.parts.retain(|(n, _, _)| n.key() != name.key());
        self.parts.push((name, content_type, bytes));
    }

    /// Relate `source`, or the package for `None`, to the part `target` by
    /// `kind`, and answer the id the source names it by.
    pub fn relate(
        &mut self,
        source: Option<&PartName>,
        kind: RelKind,
        target: &PartName,
    ) -> String {
        let list = self.rels.entry(source.cloned()).or_default();
        let id = format!("rId{}", list.len() + 1);
        let href = relative_target(source, target);
        list.push((id.clone(), kind, href));
        id
    }

    /// The bytes of the archive.
    ///
    /// # Errors
    ///
    /// [`OpcError::NotAnArchive`] when the zip writer refuses an entry.
    pub fn finish(self) -> Result<Vec<u8>, OpcError> {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let mut write = |name: &str, bytes: &[u8]| -> Result<(), OpcError> {
            zip.start_file(name, options)
                .map_err(|e| OpcError::NotAnArchive(e.to_string()))?;
            zip.write_all(bytes)
                .map_err(|e| OpcError::NotAnArchive(e.to_string()))
        };
        write("[Content_Types].xml", self.content_types().as_bytes())?;
        for (source, list) in &self.rels {
            let rels = source
                .as_ref()
                .map_or(PartName("_rels/.rels".to_owned()), PartName::rels);
            write(rels.as_str(), relationships_xml(list).as_bytes())?;
        }
        for (name, _, bytes) in &self.parts {
            write(name.as_str(), bytes)?;
        }
        zip.finish()
            .map(Cursor::into_inner)
            .map_err(|e| OpcError::NotAnArchive(e.to_string()))
    }

    /// The `[Content_Types].xml` part: the two defaults every package uses,
    /// then one override per part.
    fn content_types(&self) -> String {
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
             <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>",
        );
        for (name, content_type, _) in &self.parts {
            out.push_str(&format!(
                "<Override PartName=\"/{}\" ContentType=\"{content_type}\"/>",
                xml::escape(name.as_str())
            ));
        }
        out.push_str("</Types>");
        out
    }
}

/// A relationship part listing `list`.
fn relationships_xml(list: &[(String, RelKind, String)]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target) in list {
        out.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{}\" Target=\"{}\"/>",
            kind.uri(),
            xml::escape(target)
        ));
    }
    out.push_str("</Relationships>");
    out
}

/// The target text of `target` as `source` names it: relative to the
/// source's folder when the target sits below it, absolute otherwise.
fn relative_target(source: Option<&PartName>, target: &PartName) -> String {
    let folder = source.map(PartName::folder).unwrap_or_default();
    if folder.is_empty() {
        return target.as_str().to_owned();
    }
    target
        .as_str()
        .strip_prefix(folder)
        .and_then(|rest| rest.strip_prefix('/'))
        .map_or_else(|| format!("/{}", target.as_str()), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> PartName {
        PartName::new(text).unwrap_or_else(|| panic!("{text} is a part name"))
    }

    #[test]
    fn a_target_resolves_from_the_folder_of_its_source() {
        let source = name("xl/workbook.xml");
        assert_eq!(
            PartName::resolve(Some(&source), "worksheets/sheet1.xml"),
            Some(name("xl/worksheets/sheet1.xml"))
        );
        assert_eq!(
            PartName::resolve(
                Some(&name("xl/worksheets/sheet1.xml")),
                "../drawings/d1.xml"
            ),
            Some(name("xl/drawings/d1.xml"))
        );
        assert_eq!(
            PartName::resolve(Some(&source), "/xl/styles.xml"),
            Some(name("xl/styles.xml"))
        );
        assert_eq!(PartName::resolve(None, "../../etc/passwd"), None);
        assert_eq!(PartName::new("a/../b"), None);
        assert_eq!(
            PartName::new("\\xl\\workbook.xml"),
            Some(name("xl/workbook.xml"))
        );
    }

    #[test]
    fn a_written_package_reads_back_its_parts_and_relationships() {
        let mut writer = PackageWriter::new();
        let main = name("xl/workbook.xml");
        let sheet = name("xl/worksheets/sheet1.xml");
        writer.add_part(
            main.clone(),
            "application/main+xml",
            b"<workbook/>".to_vec(),
        );
        writer.add_part(
            sheet.clone(),
            "application/sheet+xml",
            b"<worksheet/>".to_vec(),
        );
        writer.relate(None, RelKind::OFFICE_DOCUMENT, &main);
        let id = writer.relate(Some(&main), RelKind::OFFICE_DOCUMENT, &sheet);
        let Ok(bytes) = writer.finish() else {
            panic!("the package writes");
        };
        let Ok(package) = Package::read(&Payload::new(bytes)) else {
            panic!("the package reads");
        };
        assert_eq!(
            package.main_part(RelKind::OFFICE_DOCUMENT),
            Ok(main.clone())
        );
        assert_eq!(
            package.part(&name("XL/Workbook.xml")),
            Some(&b"<workbook/>"[..])
        );
        let Ok(rels) = package.relationships(Some(&main)) else {
            panic!("the relationships read");
        };
        assert_eq!(
            rels.first().map(|r| (&r.id, &r.target)),
            Some((&id, &Target::Part(sheet)))
        );
    }

    #[test]
    fn bytes_that_are_no_archive_are_refused() {
        assert!(matches!(
            Package::read(&Payload::new(b"not a zip".to_vec())),
            Err(OpcError::NotAnArchive(_))
        ));
        assert!(matches!(
            Package::read(&Payload::new(Vec::new())),
            Err(OpcError::NotAnArchive(_))
        ));
    }

    #[test]
    fn a_part_that_inflates_past_its_cap_is_refused() {
        // A deflated run of zeros inflates about a thousand times: 300 KiB of
        // archive stands for 300 MiB of part.
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        assert!(zip.start_file("xl/bomb.xml", options).is_ok());
        let block = vec![0u8; 1 << 20];
        for _ in 0..257 {
            assert!(zip.write_all(&block).is_ok());
        }
        let Ok(bytes) = zip.finish().map(Cursor::into_inner) else {
            panic!("the archive writes");
        };
        assert_eq!(
            Package::read(&Payload::new(bytes)).err(),
            Some(OpcError::TooLarge(MAX_PART))
        );
    }
}
