//! The Open Packaging Conventions layer: a ZIP archive of parts, their
//! content types and the relationships between them (ISO/IEC 29500-2).
//!
//! The DOCX, xlsx and PPTX codecs share this one crate. It knows no part of
//! WordprocessingML, SpreadsheetML or PresentationML: a codec names its own
//! relationship types as [`RelKind`] constants. It opens a package, reads a
//! part, lists the relationships of a part, and writes a package back.
//!
//! Every bound a hostile archive could pass is checked while the bytes
//! inflate, never read off a header the archive wrote. The three bounds are
//! one set for the three formats, under the client memory budget
//! (`ambre-spec-budgets.md section 6`, the OOXML package row).
//!
//! The crate wraps `zip` alone, and `deny.toml` keeps that crate here. A
//! codec reads the XML of its own parts with its own parser.
//!
//! The OpenDocument package shares the archive and its bounds. Its index
//! differs: a `mimetype` entry and a manifest replace the content types and
//! the relationships. The [`odf`] module reads and writes it.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use ambre_core::{Count, CountKind, Octets, Payload, PayloadKind};

pub mod odf;
mod xml;

/// The parts of one package, counted.
pub enum PackageParts {}

impl CountKind for PackageParts {
    type Repr = u32;
    const NAME: &'static str = "PackagePartCount";
}

/// The most parts a package may hold. Word writes some twenty for a plain
/// document, a workbook of a hundred sheets with their drawings a few
/// hundred, and each picture adds one.
pub const MAX_PARTS: Count<PackageParts> = Count::new(4096);

/// The most bytes one part may inflate to: 32 MiB. The largest real Word
/// body measured, 12 MB, and a sheet of 100 000 cells read under it.
pub const MAX_PART_SIZE: Count<Octets> = Count::new(32 * 1024 * 1024);

/// The most bytes the parts a reader reads may inflate to, together: 64 MiB,
/// about 67 MB. The client memory budget is 100 MB, and the model a codec
/// builds from the parts needs room beside them. A package past this bound is
/// no typical document (`ambre-spec-budgets.md section 6`).
pub const MAX_INFLATED: Count<Octets> = Count::new(64 * 1024 * 1024);

/// Why a package does not open, or does not write. No variant quotes a part
/// name: a name is the user's content (`INTEROP-004`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OpcError {
    /// The bytes are an OLE compound file: a legacy binary document, or an
    /// OOXML package sealed by a password, which no codec can open.
    #[error("the file is a legacy or password-protected document")]
    CompoundFile,
    #[error("the file is not a ZIP package")]
    NotAZip,
    #[error("the package holds more than {} parts", MAX_PARTS.get())]
    TooManyParts,
    /// A part, or all the parts read together, inflate past their bound.
    #[error("the package inflates past {} bytes", .0.get())]
    TooLarge(Count<Octets>),
    /// A part is stored encrypted or with a compression method the layer
    /// does not link.
    #[error("a part is stored in a way the reader cannot read")]
    UnreadablePart,
    #[error("a required part is missing: {0}")]
    MissingPart(&'static str),
    /// The content types part or a relationships part is not well-formed
    /// XML, or declares a document type.
    #[error("a package part does not read: {0}")]
    Malformed(&'static str),
    /// The writer could not write the archive.
    #[error("the package could not be written")]
    WriteFailed,
    /// A relationship hangs from a part, or reaches a part, the package does
    /// not hold: a reader would follow it to nothing.
    #[error("a relationship names a part the package does not hold")]
    DanglingRelationship,
    /// An OpenDocument package of another type than the codec reads: a text
    /// opened as a spreadsheet.
    #[error("the package holds another kind of document")]
    OtherDocument,
}

/// The name of a part inside a package, absolute and without its leading
/// slash: `word/document.xml`.
///
/// A name compares in ASCII lower case, as the conventions compare it. It
/// never climbs out of the package: a target that would is refused.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartName(String);

impl PartName {
    /// The part `name` names, with or without its leading slash, or `None`
    /// for an empty name or one that climbs out of the package. A backslash
    /// reads as a slash, as some writers spell a folder.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        Self::resolve_from("", name)
    }

    /// The name as the archive spells it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The folder the part sits in, with its trailing slash, or the empty
    /// text at the root.
    fn folder(&self) -> &str {
        self.0
            .rfind('/')
            .map_or("", |at| self.0.get(..=at).unwrap_or(""))
    }

    /// The extension of the part's name, after its last dot.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let file = self.0.rsplit('/').next().unwrap_or(&self.0);
        file.rsplit_once('.').map(|(_, extension)| extension)
    }

    /// The name of the relationships part of this part:
    /// `word/_rels/document.xml.rels`, or `_rels/.rels` for [`root`].
    #[must_use]
    pub fn relationships(&self) -> Self {
        let folder = self.folder();
        let file = self.0.get(folder.len()..).unwrap_or_default();
        Self(format!("{folder}_rels/{file}.rels"))
    }

    /// The part a relationship target names, read against this part's folder.
    #[must_use]
    pub fn resolve(&self, target: &str) -> Option<Self> {
        Self::resolve_from(self.folder(), target)
    }

    fn resolve_from(folder: &str, target: &str) -> Option<Self> {
        let target = target.replace('\\', "/");
        let (base, rest) = match target.strip_prefix('/') {
            Some(rest) => ("", rest),
            None => (folder, target.as_str()),
        };
        let mut segments: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
        for segment in rest.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    segments.pop()?;
                }
                segment => segments.push(segment),
            }
        }
        (!segments.is_empty()).then(|| Self(segments.join("/")))
    }

    /// The name in ASCII lower case, the form two names compare in.
    fn key(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

/// The package root, whose relationships name the main part.
#[must_use]
pub fn root() -> PartName {
    PartName(String::new())
}

/// A relationship type, as the URI the transitional namespace gives it. A
/// codec declares the types it follows as constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelKind(&'static str);

impl RelKind {
    /// The main part of a package: a document, a workbook or a presentation.
    pub const OFFICE_DOCUMENT: Self =
        Self("http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument");
    /// The core properties: the title, the author, the dates.
    pub const CORE_PROPERTIES: Self = Self(
        "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
    );
    /// The extended properties: the application that wrote the package.
    pub const EXTENDED_PROPERTIES: Self = Self(
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties",
    );

    /// The relationship type `uri` names.
    #[must_use]
    pub const fn new(uri: &'static str) -> Self {
        Self(uri)
    }

    /// The URI the writer gives the type.
    #[must_use]
    pub const fn uri(self) -> &'static str {
        self.0
    }

    /// The last segment of the URI. The transitional and the strict
    /// namespaces end alike, so it names the type under both.
    fn last_segment(self) -> &'static str {
        last_segment(self.0)
    }
}

/// The last segment of a relationship type URI.
fn last_segment(uri: &str) -> &str {
    uri.rsplit('/').next().unwrap_or_default()
}

/// A relationship type as the package spells it. It may name a type no
/// codec follows, so it stays text, and [`RelType::is`] matches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelType(String);

impl RelType {
    /// Whether the type is `kind`, under the transitional or the strict
    /// namespace.
    #[must_use]
    pub fn is(&self, kind: RelKind) -> bool {
        last_segment(&self.0) == kind.last_segment()
    }

    /// The URI as the package spells it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The id a part gives one of its relationships, `rId7`: the key an
/// attribute such as `r:embed` names it by.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelId(String);

impl RelId {
    /// The id the file spells as `id`.
    #[must_use]
    pub fn new(id: &str) -> Self {
        Self(id.to_owned())
    }

    /// The id as the file spells it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a relationship points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelTarget {
    /// A part of the package.
    Part(PartName),
    /// An address outside the package. The layer fetches nothing.
    External(String),
}

/// One relationship of a part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub id: RelId,
    pub kind: RelType,
    pub target: RelTarget,
}

impl Relationship {
    /// Whether the relationship is of `kind`.
    #[must_use]
    pub fn is(&self, kind: RelKind) -> bool {
        self.kind.is(kind)
    }
}

/// An open package: the archive, the content types, and the bytes read so
/// far against [`MAX_INFLATED`]. A part inflates only when a reader asks for
/// it, so a picture a reader drops costs nothing.
pub struct Package<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
    /// Each part's index in the archive, by its lower-case name.
    index: BTreeMap<String, usize>,
    defaults: BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
    inflated: Count<Octets>,
}

/// The eight bytes an OLE compound file opens on.
const COMPOUND_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// The name of the content types part, which every package carries.
const CONTENT_TYPES: &str = "[Content_Types].xml";

impl<'a> Package<'a> {
    /// The package `bytes` hold.
    ///
    /// # Errors
    ///
    /// [`OpcError::CompoundFile`] for a legacy or sealed document,
    /// [`OpcError::NotAZip`], [`OpcError::TooManyParts`], and any error of
    /// reading `[Content_Types].xml`.
    pub fn open(bytes: &'a [u8]) -> Result<Self, OpcError> {
        let mut package = Self::open_archive(bytes)?;
        let types = PartName(CONTENT_TYPES.to_owned());
        let bytes = package
            .part(&types)?
            .ok_or(OpcError::MissingPart(CONTENT_TYPES))?;
        let entries =
            xml::elements(bytes.as_bytes()).map_err(|_| OpcError::Malformed(CONTENT_TYPES))?;
        for entry in entries {
            let content_type = entry.attr("ContentType").unwrap_or_default();
            match entry.name.as_str() {
                "Default" => {
                    if let Some(extension) = entry.attr("Extension") {
                        package
                            .defaults
                            .insert(extension.to_ascii_lowercase(), content_type.to_owned());
                    }
                }
                "Override" => {
                    if let Some(name) = entry.attr("PartName").and_then(PartName::new) {
                        package
                            .overrides
                            .insert(name.key(), content_type.to_owned());
                    }
                }
                _ => {}
            }
        }
        Ok(package)
    }

    /// The archive `bytes` hold, under the same bounds, with no content
    /// types read: the OpenDocument package indexes its parts otherwise.
    fn open_archive(bytes: &'a [u8]) -> Result<Self, OpcError> {
        if bytes.starts_with(&COMPOUND_FILE) {
            return Err(OpcError::CompoundFile);
        }
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| OpcError::NotAZip)?;
        if Count::<PackageParts>::of(archive.len()) > MAX_PARTS {
            return Err(OpcError::TooManyParts);
        }
        // `file_names` walks the archive in the order of its index.
        let index = archive
            .file_names()
            .zip(0..)
            .filter_map(|(name, at)| Some((name.ok()?.to_ascii_lowercase(), at)))
            .collect::<BTreeMap<_, _>>();
        Ok(Self {
            archive,
            index,
            defaults: BTreeMap::new(),
            overrides: BTreeMap::new(),
            inflated: Count::ZERO,
        })
    }

    /// Whether the package holds the part `name`.
    #[must_use]
    pub fn contains(&self, name: &PartName) -> bool {
        self.index.contains_key(&name.key())
    }

    /// The name of every part, in archive order. A folder entry and a name
    /// that climbs out of the package are left out. So is a name that is not
    /// UTF-8: no part a codec reads has one.
    #[must_use]
    pub fn part_names(&self) -> Vec<PartName> {
        self.archive
            .file_names()
            .filter_map(Result::ok)
            .filter(|name| !name.ends_with('/'))
            .filter_map(|name| PartName::new(&name))
            .collect()
    }

    /// The content type of the part `name`: its override, or the default of
    /// its extension.
    #[must_use]
    pub fn content_type(&self, name: &PartName) -> Option<&str> {
        self.overrides
            .get(&name.key())
            .or_else(|| {
                name.extension()
                    .and_then(|extension| self.defaults.get(&extension.to_ascii_lowercase()))
            })
            .map(String::as_str)
    }

    /// The bytes of the part `name`, or `None` when the package holds none.
    ///
    /// # Errors
    ///
    /// [`OpcError::TooLarge`] when the part inflates past [`MAX_PART_SIZE`].
    /// The same error when the parts read so far pass [`MAX_INFLATED`].
    /// [`OpcError::UnreadablePart`] for a part stored encrypted or with a
    /// method the layer does not link.
    pub fn part(&mut self, name: &PartName) -> Result<Option<Payload<PartBytes>>, OpcError> {
        let Some(&at) = self.index.get(&name.key()) else {
            return Ok(None);
        };
        let file = self
            .archive
            .by_index(at)
            .map_err(|_| OpcError::UnreadablePart)?;
        if file.encrypted() {
            return Err(OpcError::UnreadablePart);
        }
        let left = MAX_INFLATED.minus(self.inflated);
        let cap = if MAX_PART_SIZE < left {
            MAX_PART_SIZE
        } else {
            left
        };
        let mut bytes = Vec::new();
        // One byte past the cap tells a part at the cap from one past it,
        // whatever size the archive's header claims.
        file.take(cap.get().saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| OpcError::UnreadablePart)?;
        let read = Payload::<PartBytes>::new(bytes);
        if read.len() > cap {
            let bound = if cap == MAX_PART_SIZE {
                MAX_PART_SIZE
            } else {
                MAX_INFLATED
            };
            return Err(OpcError::TooLarge(bound));
        }
        self.inflated = self.inflated.plus(read.len());
        Ok(Some(read))
    }

    /// The relationships of the part `source`, or of the package for
    /// [`root`]. A part with no relationships part has none.
    ///
    /// # Errors
    ///
    /// Any error of reading the relationships part, and
    /// [`OpcError::Malformed`] for one that is not well-formed.
    pub fn relationships(&mut self, source: &PartName) -> Result<Vec<Relationship>, OpcError> {
        let Some(bytes) = self.part(&source.relationships())? else {
            return Ok(Vec::new());
        };
        let entries = xml::elements(bytes.as_bytes())
            .map_err(|_| OpcError::Malformed("a relationships part"))?;
        Ok(entries
            .into_iter()
            .filter(|entry| entry.name == "Relationship")
            .filter_map(|entry| {
                let id = RelId::new(entry.attr("Id")?);
                let kind = RelType(entry.attr("Type").unwrap_or_default().to_owned());
                let target = entry.attr("Target")?;
                let target = if entry.attr("TargetMode") == Some("External") {
                    RelTarget::External(target.to_owned())
                } else {
                    RelTarget::Part(source.resolve(target)?)
                };
                Some(Relationship { id, kind, target })
            })
            .collect())
    }

    /// The part the first relationship of `kind` from `source` names.
    ///
    /// # Errors
    ///
    /// Any error of reading the relationships part.
    pub fn related(
        &mut self,
        source: &PartName,
        kind: RelKind,
    ) -> Result<Option<PartName>, OpcError> {
        Ok(self
            .relationships(source)?
            .into_iter()
            .find_map(|rel| match rel.target {
                RelTarget::Part(name) if rel.is(kind) => Some(name),
                _ => None,
            }))
    }
}

/// The bytes of one part of a package, as the archive stores them
/// inflated. `Payload<PartBytes>`.
pub enum PartBytes {}

impl PayloadKind for PartBytes {
    const NAME: &'static str = "PartBytes";
}

/// The bytes of a written package. `Payload<PackageBytes>`.
pub enum PackageBytes {}

impl PayloadKind for PackageBytes {
    const NAME: &'static str = "PackageBytes";
}

/// A written package is the file a codec hands back.
impl ambre_core::Becomes<ambre_core::FileBytes> for PackageBytes {}

/// A package being written: its parts, their content types and their
/// relationships. The archive it writes is the same for the same parts:
/// every entry carries the one date of a ZIP archive's epoch.
#[derive(Default)]
pub struct PackageWriter {
    parts: Vec<(PartName, Payload<PartBytes>)>,
    defaults: BTreeMap<String, String>,
    overrides: Vec<(PartName, String)>,
    relationships: BTreeMap<PartName, Vec<(RelId, RelKind, RelTarget)>>,
}

impl PackageWriter {
    /// A package with the two defaults every package uses: the
    /// relationships parts and plain XML.
    #[must_use]
    pub fn new() -> Self {
        let mut writer = Self::default();
        writer.default_type(
            "rels",
            "application/vnd.openxmlformats-package.relationships+xml",
        );
        writer.default_type("xml", "application/xml");
        writer
    }

    /// Give every part of `extension` the content type `content_type`.
    pub fn default_type(&mut self, extension: &str, content_type: &str) {
        self.defaults
            .insert(extension.to_owned(), content_type.to_owned());
    }

    /// Add the part `name` holding `bytes`, with its own content type when
    /// one is given. A part added twice keeps the last bytes and type, at the
    /// place of the first.
    pub fn part(&mut self, name: PartName, content_type: Option<&str>, bytes: Payload<PartBytes>) {
        self.overrides.retain(|(held, _)| held.key() != name.key());
        if let Some(content_type) = content_type {
            self.overrides.push((name.clone(), content_type.to_owned()));
        }
        match self
            .parts
            .iter_mut()
            .find(|(held, _)| held.key() == name.key())
        {
            Some((_, held)) => *held = bytes,
            None => self.parts.push((name, bytes)),
        }
    }

    /// Whether the package holds the part `name` so far.
    #[must_use]
    pub fn holds(&self, name: &PartName) -> bool {
        self.parts.iter().any(|(held, _)| held.key() == name.key())
    }

    /// Add a relationship from `source` and answer its id, numbered from
    /// `rId1` in the order the relationships of `source` were added.
    pub fn relate(&mut self, source: &PartName, kind: RelKind, target: RelTarget) -> RelId {
        let rels = self.relationships.entry(source.clone()).or_default();
        let id = RelId(format!("rId{}", rels.len() + 1));
        rels.push((id.clone(), kind, target));
        id
    }

    /// The archive.
    ///
    /// # Errors
    ///
    /// [`OpcError::DanglingRelationship`] when a relationship hangs from or
    /// reaches a part the package does not hold, and
    /// [`OpcError::WriteFailed`] when the ZIP writer fails.
    pub fn finish(self) -> Result<Payload<PackageBytes>, OpcError> {
        let dangling = self.relationships.iter().any(|(source, rels)| {
            (*source != root() && !self.holds(source))
                || rels.iter().any(|(_, _, target)| match target {
                    RelTarget::Part(name) => !self.holds(name),
                    RelTarget::External(_) => false,
                })
        });
        if dangling {
            return Err(OpcError::DanglingRelationship);
        }
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), OpcError> {
            zip.start_file(name, options)
                .map_err(|_| OpcError::WriteFailed)?;
            zip.write_all(bytes).map_err(|_| OpcError::WriteFailed)
        };
        put(CONTENT_TYPES, self.content_types().as_bytes())?;
        for (source, rels) in &self.relationships {
            put(
                source.relationships().as_str(),
                relationships_part(source, rels).as_bytes(),
            )?;
        }
        for (name, bytes) in &self.parts {
            put(name.as_str(), bytes.as_bytes())?;
        }
        let cursor = zip.finish().map_err(|_| OpcError::WriteFailed)?;
        Ok(Payload::new(cursor.into_inner()))
    }

    fn content_types(&self) -> Payload<PartBytes> {
        let mut xml = xml::XmlWriter::new();
        xml.open(
            "Types",
            &[(
                "xmlns",
                "http://schemas.openxmlformats.org/package/2006/content-types",
            )],
        );
        for (extension, content_type) in &self.defaults {
            xml.empty(
                "Default",
                &[("Extension", extension), ("ContentType", content_type)],
            );
        }
        for (name, content_type) in &self.overrides {
            let part = format!("/{}", name.as_str());
            xml.empty(
                "Override",
                &[("PartName", &part), ("ContentType", content_type)],
            );
        }
        xml.close("Types");
        xml.finish()
    }
}

/// The relationships part of `source`, each target part written relative to
/// the folder of `source`, as Office writes it.
fn relationships_part(
    source: &PartName,
    rels: &[(RelId, RelKind, RelTarget)],
) -> Payload<PartBytes> {
    let mut xml = xml::XmlWriter::new();
    xml.open(
        "Relationships",
        &[(
            "xmlns",
            "http://schemas.openxmlformats.org/package/2006/relationships",
        )],
    );
    for (id, kind, target) in rels {
        match target {
            RelTarget::Part(name) => {
                let relative = relative_target(source, name);
                xml.empty(
                    "Relationship",
                    &[
                        ("Id", id.as_str()),
                        ("Type", kind.uri()),
                        ("Target", &relative),
                    ],
                );
            }
            RelTarget::External(address) => xml.empty(
                "Relationship",
                &[
                    ("Id", id.as_str()),
                    ("Type", kind.uri()),
                    ("Target", address),
                    ("TargetMode", "External"),
                ],
            ),
        }
    }
    xml.close("Relationships");
    xml.finish()
}

/// The path from the folder of `source` to the part `target`, as a
/// relationship writes it: `media/a.png` from `word/document.xml`, and
/// `../slideLayouts/slideLayout1.xml` from `ppt/slides/slide1.xml`.
fn relative_target(source: &PartName, target: &PartName) -> String {
    let from: Vec<&str> = source
        .folder()
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let to: Vec<&str> = target.as_str().split('/').collect();
    let shared = from
        .iter()
        .zip(&to)
        .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
        .count();
    let climbs = from.len().saturating_sub(shared);
    let mut path: Vec<&str> = std::iter::repeat_n("..", climbs).collect();
    path.extend(to.iter().skip(shared));
    path.join("/")
}

#[cfg(test)]
mod tests;
