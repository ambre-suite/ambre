//! The package layer of an Office Open XML file: the zip, the content types
//! and the relationships (ECMA-376 Part 2, Open Packaging Conventions).
//!
//! It knows parts and relationships, and nothing of a slide. The Word, the
//! spreadsheet and the PowerPoint codecs each write their parts through it,
//! so its interface stays small:
//!
//! | Half | Entry | What it does |
//! | :--- | :--- | :--- |
//! | Write | [`PackageWriter`] | add a part, relate two parts, finish the zip |
//! | Read | [`Package`] | open a zip under a size bound, read a part, its type and its relationships |
//!
//! The writer is deterministic: the same parts and relationships give the
//! same bytes. The zip carries a fixed time, and the parts keep the order
//! they were added in.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use ambre_core::{Count, FileBytes, Octets, Payload, Rule, Validated};
use quick_xml::Reader;
use quick_xml::events::Event;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::xml::Xml;

/// The part that names the content type of every other part.
const CONTENT_TYPES: &str = "[Content_Types].xml";

/// The namespace of the content types part.
const NS_CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

/// The namespace of a relationships part.
const NS_RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// The content type of a relationships part.
const RELATIONSHIPS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";

/// The most parts a package may hold before the reader refuses it.
const MAX_PARTS: usize = 10_000;

/// The most bytes the parts of a package may unpack to before the reader
/// refuses it. A zip of a few kilobytes can claim gigabytes, so the reader
/// counts what it inflates, never what the zip claims.
pub const MAX_UNPACKED: Count<Octets> = Count::<Octets>::new(512 * 1024 * 1024);

/// Why a package could not be written or read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpcError {
    /// The zip layer refused, in its own words.
    #[error("the zip archive failed: {0}")]
    Zip(String),
    /// An XML part of the package layer does not parse, in the parser's words.
    #[error("a package part does not parse: {0}")]
    Xml(String),
    /// A name, a type or a relationship breaks the grammar of a package.
    #[error("the package is malformed: {0}")]
    Malformed(String),
    /// The parts unpack past [`MAX_UNPACKED`], or the package holds more parts
    /// than the reader takes.
    #[error("the package unpacks past {0:?}")]
    TooLarge(Count<Octets>),
}

impl From<zip::result::ZipError> for OpcError {
    fn from(e: zip::result::ZipError) -> Self {
        Self::Zip(e.to_string())
    }
}

impl From<std::io::Error> for OpcError {
    fn from(e: std::io::Error) -> Self {
        Self::Zip(e.to_string())
    }
}

/// The rule of a part name: an absolute path of non-empty segments, none of
/// them `.` or `..`, in printable ASCII with no space.
pub enum PartName {}

impl Rule for PartName {
    type Error = OpcError;
    const LOGGED_AS: Option<&'static str> = Some("PartName");

    fn admit(text: String) -> Result<String, OpcError> {
        let refuse = || OpcError::Malformed(format!("`{text}` is no part name"));
        let rest = text.strip_prefix('/').ok_or_else(refuse)?;
        let segments_hold = rest
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..");
        let chars_hold = text.bytes().all(|b| b.is_ascii_graphic());
        if segments_hold && chars_hold {
            Ok(text)
        } else {
            Err(refuse())
        }
    }
}

/// The rule of a content type: `type/subtype`, printable ASCII, no space.
pub enum ContentType {}

impl Rule for ContentType {
    type Error = OpcError;
    const LOGGED_AS: Option<&'static str> = Some("ContentType");

    fn admit(text: String) -> Result<String, OpcError> {
        let holds = text
            .split_once('/')
            .is_some_and(|(t, s)| !t.is_empty() && !s.is_empty() && !s.contains('/'))
            && text.bytes().all(|b| b.is_ascii_graphic());
        if holds {
            Ok(text)
        } else {
            Err(OpcError::Malformed(format!("`{text}` is no content type")))
        }
    }
}

/// The rule of a relationship type: a URI, printable ASCII, no space.
pub enum RelationshipType {}

impl Rule for RelationshipType {
    type Error = OpcError;
    const LOGGED_AS: Option<&'static str> = Some("RelationshipType");

    fn admit(text: String) -> Result<String, OpcError> {
        if !text.is_empty() && text.bytes().all(|b| b.is_ascii_graphic()) {
            Ok(text)
        } else {
            Err(OpcError::Malformed(format!(
                "`{text}` is no relationship type"
            )))
        }
    }
}

/// The rule of a relationship id: an XML name of ASCII letters and digits,
/// opening on a letter.
pub enum RelationshipId {}

impl Rule for RelationshipId {
    type Error = OpcError;
    const LOGGED_AS: Option<&'static str> = Some("RelationshipId");

    fn admit(text: String) -> Result<String, OpcError> {
        let opens = text.bytes().next().is_some_and(|b| b.is_ascii_alphabetic());
        if opens && text.bytes().all(|b| b.is_ascii_alphanumeric()) {
            Ok(text)
        } else {
            Err(OpcError::Malformed(format!(
                "`{text}` is no relationship id"
            )))
        }
    }
}

/// What a set of relationships hangs from: the package itself, or one part.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Package,
    Part(Validated<PartName>),
}

impl Source {
    /// The name of the part that holds the relationships of the source:
    /// `/_rels/.rels` for the package, `/dir/_rels/name.rels` for a part.
    ///
    /// # Errors
    ///
    /// [`OpcError::Malformed`] never in practice: a valid part name gives a
    /// valid relationships part name.
    pub fn relationships_part(&self) -> Result<Validated<PartName>, OpcError> {
        match self {
            Self::Package => Validated::<PartName>::parse("/_rels/.rels"),
            Self::Part(name) => {
                let (dir, file) = split_dir(name.as_str());
                Validated::<PartName>::parse(format!("{dir}_rels/{file}.rels"))
            }
        }
    }

    /// The directory a relative target of the source starts from, with its
    /// closing slash.
    fn base(&self) -> &str {
        match self {
            Self::Package => "/",
            Self::Part(name) => split_dir(name.as_str()).0,
        }
    }
}

/// One relationship: its id, what it means, and the part it reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub id: Validated<RelationshipId>,
    pub kind: Validated<RelationshipType>,
    pub target: Validated<PartName>,
}

/// A part the writer holds until it finishes the zip.
struct HeldPart {
    name: Validated<PartName>,
    content_type: Validated<ContentType>,
    data: Vec<u8>,
}

/// The writer of a package: parts, then relationships between them, then the
/// zip.
#[derive(Default)]
pub struct PackageWriter {
    parts: Vec<HeldPart>,
    relationships: BTreeMap<Source, Vec<Relationship>>,
}

impl PackageWriter {
    /// An empty package.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the part `name` of type `content_type`. A part added twice keeps
    /// the last bytes, at the place of the first.
    pub fn add_part(
        &mut self,
        name: Validated<PartName>,
        content_type: Validated<ContentType>,
        data: Vec<u8>,
    ) {
        match self.parts.iter_mut().find(|p| p.name == name) {
            Some(held) => {
                held.content_type = content_type;
                held.data = data;
            }
            None => self.parts.push(HeldPart {
                name,
                content_type,
                data,
            }),
        }
    }

    /// Relate `source` to the part `target` with the meaning `kind`, and
    /// answer the id the source cites it by. Ids count up from `rId1` per
    /// source, in the order of the calls.
    ///
    /// # Errors
    ///
    /// [`OpcError::Malformed`] never in practice: the id the writer mints
    /// always keeps the grammar.
    pub fn relate(
        &mut self,
        source: &Source,
        kind: Validated<RelationshipType>,
        target: &Validated<PartName>,
    ) -> Result<Validated<RelationshipId>, OpcError> {
        let list = self.relationships.entry(source.clone()).or_default();
        let id = Validated::<RelationshipId>::parse(format!("rId{}", list.len() + 1))?;
        list.push(Relationship {
            id: id.clone(),
            kind,
            target: target.clone(),
        });
        Ok(id)
    }

    /// The zip of every part, its content types and its relationships.
    ///
    /// # Errors
    ///
    /// [`OpcError::Zip`] when the zip layer refuses, [`OpcError::Malformed`]
    /// when a relationship reaches a part the package does not hold.
    pub fn finish(self) -> Result<Payload<FileBytes>, OpcError> {
        for (source, list) in &self.relationships {
            if let Source::Part(name) = source
                && !self.holds(name)
            {
                return Err(OpcError::Malformed(format!(
                    "relationships hang from `{}`, which the package does not hold",
                    name.as_str()
                )));
            }
            if let Some(missing) = list.iter().find(|r| !self.holds(&r.target)) {
                return Err(OpcError::Malformed(format!(
                    "a relationship reaches `{}`, which the package does not hold",
                    missing.target.as_str()
                )));
            }
        }
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let deflated = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(DateTime::default());
        // A picture is compressed already: deflating it again costs time and
        // saves nothing.
        let stored = deflated.compression_method(CompressionMethod::Stored);
        zip.start_file(CONTENT_TYPES, deflated)?;
        zip.write_all(&self.content_types()?)?;
        for (source, list) in &self.relationships {
            let name = source.relationships_part()?;
            zip.start_file(entry_name(&name), deflated)?;
            zip.write_all(&relationships_xml(source, list)?)?;
        }
        for part in &self.parts {
            let options = if part.content_type.as_str().starts_with("image/") {
                stored
            } else {
                deflated
            };
            zip.start_file(entry_name(&part.name), options)?;
            zip.write_all(&part.data)?;
        }
        let bytes = zip.finish()?.into_inner();
        Ok(Payload::new(bytes))
    }

    fn holds(&self, name: &Validated<PartName>) -> bool {
        self.parts.iter().any(|p| &p.name == name)
    }

    /// The content types part: a default for the relationships parts, and an
    /// override for every part the writer holds.
    fn content_types(&self) -> Result<Vec<u8>, OpcError> {
        let mut xml = Xml::part();
        xml.open("Types", &[("xmlns", NS_CONTENT_TYPES)]);
        xml.leaf(
            "Default",
            &[("Extension", "rels"), ("ContentType", RELATIONSHIPS_TYPE)],
        );
        xml.leaf(
            "Default",
            &[("Extension", "xml"), ("ContentType", "application/xml")],
        );
        for part in &self.parts {
            xml.leaf(
                "Override",
                &[
                    ("PartName", part.name.as_str()),
                    ("ContentType", part.content_type.as_str()),
                ],
            );
        }
        xml.close("Types");
        xml.finish().map_err(|e| OpcError::Xml(e.to_string()))
    }
}

/// The relationships part of `source`, each target written relative to it.
fn relationships_xml(source: &Source, list: &[Relationship]) -> Result<Vec<u8>, OpcError> {
    let mut xml = Xml::part();
    xml.open("Relationships", &[("xmlns", NS_RELATIONSHIPS)]);
    for rel in list {
        let target = relative_target(source.base(), rel.target.as_str());
        xml.leaf(
            "Relationship",
            &[
                ("Id", rel.id.as_str()),
                ("Type", rel.kind.as_str()),
                ("Target", &target),
            ],
        );
    }
    xml.close("Relationships");
    xml.finish().map_err(|e| OpcError::Xml(e.to_string()))
}

/// The name of the zip entry of a part: the part name without its leading
/// slash.
fn entry_name(name: &Validated<PartName>) -> &str {
    name.as_str().trim_start_matches('/')
}

/// The directory of a part name with its closing slash, and the file name.
fn split_dir(name: &str) -> (&str, &str) {
    match name.rfind('/') {
        Some(at) => name.split_at(at + 1),
        None => ("/", name),
    }
}

/// The path from the directory `base` to the part `target`, as a
/// relationship writes it: `../slideLayouts/slideLayout1.xml` from
/// `/ppt/slides/`.
fn relative_target(base: &str, target: &str) -> String {
    let from: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    let (to_dirs, file) = match to.split_last() {
        Some((file, dirs)) => (dirs, *file),
        None => (&[][..], ""),
    };
    let shared = from
        .iter()
        .zip(to_dirs.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut path: Vec<&str> = from.iter().skip(shared).map(|_| "..").collect();
    path.extend(to_dirs.iter().skip(shared).copied());
    path.push(file);
    path.join("/")
}

/// The part a relationship target names, resolved against the directory
/// `base`: `..` climbs one directory, and a target that opens on `/` is
/// absolute.
fn resolve_target(base: &str, target: &str) -> Result<Validated<PartName>, OpcError> {
    let mut path: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        base.split('/').filter(|s| !s.is_empty()).collect()
    };
    for segment in target.split('/').filter(|s| !s.is_empty() && *s != ".") {
        if segment == ".." {
            if path.pop().is_none() {
                return Err(OpcError::Malformed(format!(
                    "the target `{target}` climbs out of the package"
                )));
            }
        } else {
            path.push(segment);
        }
    }
    Validated::<PartName>::parse(format!("/{}", path.join("/")))
}

/// A package read whole into memory: every part, under the size bound.
#[derive(Debug)]
pub struct Package {
    parts: BTreeMap<Validated<PartName>, Payload<FileBytes>>,
    content_types: BTreeMap<Validated<PartName>, Validated<ContentType>>,
}

impl Package {
    /// Read the package `bytes` holds.
    ///
    /// # Errors
    ///
    /// | Error | Cause |
    /// | :--- | :--- |
    /// | [`OpcError::Zip`] | bytes that are no zip |
    /// | [`OpcError::TooLarge`] | parts past [`MAX_UNPACKED`], or past the part count the reader takes |
    /// | [`OpcError::Xml`] | a content types part that does not parse |
    /// | [`OpcError::Malformed`] | an entry name that is no part name |
    pub fn open(bytes: &Payload<FileBytes>) -> Result<Self, OpcError> {
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_bytes()))?;
        if archive.len() > MAX_PARTS {
            return Err(OpcError::TooLarge(MAX_UNPACKED));
        }
        let mut left = MAX_UNPACKED.get();
        let mut parts = BTreeMap::new();
        for index in 0..archive.len() {
            let entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let name = Validated::<PartName>::parse(format!("/{}", entry.name()))?;
            let mut data = Vec::new();
            // One byte past what is left tells a part at the bound from one
            // past it.
            let read = entry.take(left.saturating_add(1)).read_to_end(&mut data)?;
            let read = u64::try_from(read).unwrap_or(u64::MAX);
            if read > left {
                return Err(OpcError::TooLarge(MAX_UNPACKED));
            }
            left -= read;
            parts.insert(name, Payload::new(data));
        }
        let types_name = Validated::<PartName>::parse(format!("/{CONTENT_TYPES}"))?;
        let types = parts
            .get(&types_name)
            .ok_or_else(|| OpcError::Malformed("the package holds no content types".into()))?;
        let content_types = read_content_types(types.as_bytes(), parts.keys())?;
        Ok(Self {
            parts,
            content_types,
        })
    }

    /// The bytes of the part `name`, or `None` when the package lacks it.
    #[must_use]
    pub fn part(&self, name: &Validated<PartName>) -> Option<&Payload<FileBytes>> {
        self.parts.get(name)
    }

    /// The content type of the part `name`, by its override or by the default
    /// of its extension.
    #[must_use]
    pub fn content_type(&self, name: &Validated<PartName>) -> Option<&Validated<ContentType>> {
        self.content_types.get(name)
    }

    /// Every part name, in order.
    pub fn part_names(&self) -> impl Iterator<Item = &Validated<PartName>> {
        self.parts.keys()
    }

    /// The relationships of `source`, each target resolved to a part name.
    /// A source with no relationships part has none.
    ///
    /// # Errors
    ///
    /// [`OpcError::Xml`] for a relationships part that does not parse.
    /// [`OpcError::Malformed`] for an id, a type or a target that breaks the
    /// grammar of a package.
    pub fn relationships(&self, source: &Source) -> Result<Vec<Relationship>, OpcError> {
        let Some(part) = self.parts.get(&source.relationships_part()?) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for attrs in elements(part.as_bytes(), b"Relationship")? {
            // An external target is no part of the package: a hyperlink.
            if attrs.get("TargetMode").is_some_and(|m| m == "External") {
                continue;
            }
            let field = |key: &str| {
                attrs.get(key).cloned().ok_or_else(|| {
                    OpcError::Malformed(format!("a relationship carries no `{key}`"))
                })
            };
            out.push(Relationship {
                id: Validated::<RelationshipId>::parse(field("Id")?)?,
                kind: Validated::<RelationshipType>::parse(field("Type")?)?,
                target: resolve_target(source.base(), &field("Target")?)?,
            });
        }
        Ok(out)
    }
}

/// The content type of every part: its override, else the default of its
/// extension. A part neither names has none.
fn read_content_types<'a>(
    xml: &[u8],
    names: impl Iterator<Item = &'a Validated<PartName>>,
) -> Result<BTreeMap<Validated<PartName>, Validated<ContentType>>, OpcError> {
    let mut defaults = BTreeMap::new();
    for attrs in elements(xml, b"Default")? {
        if let (Some(ext), Some(ct)) = (attrs.get("Extension"), attrs.get("ContentType")) {
            defaults.insert(
                ext.to_ascii_lowercase(),
                Validated::<ContentType>::parse(ct.clone())?,
            );
        }
    }
    let mut overrides = BTreeMap::new();
    for attrs in elements(xml, b"Override")? {
        if let (Some(name), Some(ct)) = (attrs.get("PartName"), attrs.get("ContentType")) {
            overrides.insert(
                Validated::<PartName>::parse(name.clone())?,
                Validated::<ContentType>::parse(ct.clone())?,
            );
        }
    }
    let mut out = BTreeMap::new();
    for name in names {
        let by_extension = || {
            let (_, file) = split_dir(name.as_str());
            let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
            defaults.get(&ext).cloned()
        };
        if let Some(ct) = overrides.get(name).cloned().or_else(by_extension) {
            out.insert(name.clone(), ct);
        }
    }
    Ok(out)
}

/// The attributes of every element of the local name `local` in `xml`, in
/// document order, unescaped.
fn elements(xml: &[u8], local: &[u8]) -> Result<Vec<BTreeMap<String, String>>, OpcError> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == local => {
                let mut attrs = BTreeMap::new();
                for attr in e.attributes() {
                    let attr = attr.map_err(|err| OpcError::Xml(err.to_string()))?;
                    let key = String::from_utf8_lossy(attr.key.local_name().as_ref()).into_owned();
                    let value = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|err| OpcError::Xml(err.to_string()))?;
                    attrs.insert(key, value.into_owned());
                }
                out.push(attrs);
            }
            Ok(Event::Eof) => return Ok(out),
            Ok(_) => {}
            Err(err) => return Err(OpcError::Xml(err.to_string())),
        }
        buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> Validated<PartName> {
        Validated::<PartName>::parse(text).expect("a part name")
    }

    fn xml_type() -> Validated<ContentType> {
        Validated::<ContentType>::parse("application/xml").expect("a content type")
    }

    fn kind() -> Validated<RelationshipType> {
        Validated::<RelationshipType>::parse("http://example.com/rel").expect("a relationship type")
    }

    #[test]
    fn a_part_name_is_an_absolute_path_of_real_segments() {
        assert!(Validated::<PartName>::parse("/ppt/slides/slide1.xml").is_ok());
        for bad in [
            "ppt/a.xml",
            "/ppt//a.xml",
            "/ppt/../a.xml",
            "/ppt/a b.xml",
            "/",
            "/a/",
        ] {
            assert!(
                Validated::<PartName>::parse(bad).is_err(),
                "{bad} is no part name"
            );
        }
    }

    #[test]
    fn a_target_is_written_relative_to_its_source() {
        assert_eq!(
            relative_target("/ppt/slides/", "/ppt/slideLayouts/slideLayout1.xml"),
            "../slideLayouts/slideLayout1.xml"
        );
        assert_eq!(
            relative_target("/ppt/", "/ppt/slides/slide1.xml"),
            "slides/slide1.xml"
        );
        assert_eq!(
            relative_target("/", "/ppt/presentation.xml"),
            "ppt/presentation.xml"
        );
    }

    #[test]
    fn a_relative_target_resolves_back_to_its_part() {
        for (base, target) in [
            ("/ppt/slides/", "/ppt/slideLayouts/slideLayout1.xml"),
            ("/ppt/", "/ppt/slides/slide1.xml"),
            ("/", "/docProps/core.xml"),
            ("/ppt/notesSlides/", "/ppt/media/image1.png"),
        ] {
            let written = relative_target(base, target);
            assert_eq!(
                resolve_target(base, &written).expect("resolves").as_str(),
                target
            );
        }
        assert!(resolve_target("/", "../outside.xml").is_err());
    }

    #[test]
    fn a_written_package_reads_back_with_its_types_and_relationships() {
        let mut w = PackageWriter::new();
        let doc = name("/doc/main.xml");
        let other = name("/doc/sub/other.xml");
        w.add_part(doc.clone(), xml_type(), b"<a/>".to_vec());
        w.add_part(other.clone(), xml_type(), b"<b/>".to_vec());
        let first = w.relate(&Source::Package, kind(), &doc).expect("relates");
        let second = w
            .relate(&Source::Part(doc.clone()), kind(), &other)
            .expect("relates");
        assert_eq!(first.as_str(), "rId1");
        assert_eq!(second.as_str(), "rId1", "ids count per source");
        let bytes = w.finish().expect("finishes");

        let back = Package::open(&bytes).expect("opens");
        assert_eq!(back.part(&other).map(Payload::as_bytes), Some(&b"<b/>"[..]));
        assert_eq!(back.content_type(&doc), Some(&xml_type()));
        let rels = back
            .relationships(&Source::Part(doc))
            .expect("relationships read");
        assert_eq!(rels.len(), 1);
        assert_eq!(rels.first().map(|r| r.target.clone()), Some(other));
    }

    #[test]
    fn a_relationship_to_a_missing_part_is_refused() {
        let mut w = PackageWriter::new();
        let doc = name("/doc/main.xml");
        w.add_part(doc, xml_type(), Vec::new());
        w.relate(&Source::Package, kind(), &name("/doc/gone.xml"))
            .expect("relates");
        assert!(matches!(w.finish(), Err(OpcError::Malformed(_))));
    }

    #[test]
    fn the_same_parts_write_the_same_bytes() {
        let write = || {
            let mut w = PackageWriter::new();
            let doc = name("/doc/main.xml");
            w.add_part(doc.clone(), xml_type(), b"<a/>".to_vec());
            w.relate(&Source::Package, kind(), &doc).expect("relates");
            w.finish().expect("finishes")
        };
        assert_eq!(write(), write());
    }

    #[test]
    fn bytes_that_are_no_zip_are_refused() {
        let junk = Payload::<FileBytes>::new(b"not a zip at all".to_vec());
        assert!(matches!(Package::open(&junk), Err(OpcError::Zip(_))));
    }

    #[test]
    fn a_package_without_content_types_is_refused() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("a.xml", SimpleFileOptions::default())
            .expect("starts");
        zip.write_all(b"<a/>").expect("writes");
        let bytes = Payload::new(zip.finish().expect("finishes").into_inner());
        assert!(matches!(Package::open(&bytes), Err(OpcError::Malformed(_))));
    }
}
