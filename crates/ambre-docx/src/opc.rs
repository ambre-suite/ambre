//! The Open Packaging Conventions layer: a ZIP archive of parts, their
//! content types and the relationships between them (ISO/IEC 29500-2).
//!
//! The layer knows no part of WordprocessingML. DOCX, xlsx and PPTX share
//! it, so its surface stays small. It opens a package, reads a part, lists
//! the relationships of a part, and writes a package back. Every bound a hostile
//! archive could exceed is checked while the bytes inflate, never read off
//! a header the archive wrote.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use ambre_core::{Becomes, Count, Octets, Payload, PayloadKind};

pub use crate::xml::XmlError;
use crate::xml::{self, Ns, XmlWriter};

/// The most parts a package may hold. Word writes some twenty for a plain
/// document, and each picture adds one.
pub const MAX_PARTS: usize = 4096;

/// The most bytes one part may inflate to: 32 MiB.
pub const MAX_PART_SIZE: u64 = 32 * 1024 * 1024;

/// The most bytes the parts a reader reads may inflate to, together: 96 MiB.
/// The client memory budget is 100 MB for a typical document, and a
/// package past this bound is no typical document
/// (`ambre-spec-budgets.md section 1`).
pub const MAX_INFLATED: u64 = 96 * 1024 * 1024;

/// Why a package does not open. No variant quotes a part name: a name is
/// the user's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OpcError {
    /// The bytes are an OLE compound file: a legacy binary document, or an
    /// OOXML package sealed by a password, which the codec cannot open.
    #[error("the file is a legacy or password-protected document")]
    CompoundFile,
    #[error("the file is not a ZIP package")]
    NotAZip,
    #[error("the package holds more than {MAX_PARTS} parts")]
    TooManyParts,
    /// A part, or all the parts read together, inflate past their bound.
    #[error("the package inflates past {0} bytes")]
    TooLarge(Count<Octets>),
    /// A part is stored encrypted or with a compression method the codec
    /// does not link.
    #[error("a part is stored in a way the reader cannot read")]
    UnreadablePart,
    #[error("a required part is missing: {0}")]
    MissingPart(&'static str),
    #[error(transparent)]
    Xml(#[from] XmlError),
    /// The writer could not write the archive.
    #[error("the package could not be written")]
    WriteFailed,
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
    /// for an empty name or one that climbs out of the package.
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
        self.0.rfind('/').map_or("", |at| &self.0[..=at])
    }

    /// The extension of the part's name, after its last dot.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let file = self.0.rsplit('/').next().unwrap_or(&self.0);
        file.rsplit_once('.').map(|(_, extension)| extension)
    }

    /// The name of the relationships part of this part:
    /// `word/_rels/document.xml.rels`.
    #[must_use]
    pub fn relationships(&self) -> Self {
        let file = &self.0[self.folder().len()..];
        Self(format!("{}_rels/{file}.rels", self.folder()))
    }

    /// The part a relationship target names, read against this part's folder.
    #[must_use]
    pub fn resolve(&self, target: &str) -> Option<Self> {
        Self::resolve_from(self.folder(), target)
    }

    fn resolve_from(folder: &str, target: &str) -> Option<Self> {
        let (base, rest) = match target.strip_prefix('/') {
            Some(rest) => ("", rest),
            None => (folder, target),
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

    fn key(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

/// The package root, whose relationships name the main part.
#[must_use]
pub fn root() -> PartName {
    PartName(String::new())
}

/// What a relationship says its target is. The last segment of the type URI
/// names it, so the transitional and the strict URIs read as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelKind {
    OfficeDocument,
    CoreProperties,
    Styles,
    Numbering,
    Settings,
    Footnotes,
    Endnotes,
    Comments,
    CommentsExtended,
    Header,
    Footer,
    Hyperlink,
    Image,
    /// A macro project. The reader names it and never reads it.
    VbaProject,
    /// Another kind, which no reader follows.
    Other,
}

impl RelKind {
    fn of(uri: &str) -> Self {
        match uri.rsplit('/').next().unwrap_or_default() {
            "officeDocument" => Self::OfficeDocument,
            "core-properties" | "metadata/core-properties" => Self::CoreProperties,
            "styles" => Self::Styles,
            "numbering" => Self::Numbering,
            "settings" => Self::Settings,
            "footnotes" => Self::Footnotes,
            "endnotes" => Self::Endnotes,
            "comments" => Self::Comments,
            "commentsExtended" => Self::CommentsExtended,
            "header" => Self::Header,
            "footer" => Self::Footer,
            "hyperlink" => Self::Hyperlink,
            "image" => Self::Image,
            "vbaProject" => Self::VbaProject,
            _ => Self::Other,
        }
    }

    /// The transitional URI the writer gives the kind.
    const fn uri(self) -> &'static str {
        match self {
            Self::OfficeDocument => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
            }
            Self::CoreProperties => {
                "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties"
            }
            Self::Styles => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles"
            }
            Self::Numbering => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering"
            }
            Self::Settings => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings"
            }
            Self::Footnotes => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes"
            }
            Self::Endnotes => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes"
            }
            Self::Comments => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments"
            }
            Self::CommentsExtended => {
                "http://schemas.microsoft.com/office/2011/relationships/commentsExtended"
            }
            Self::Header => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header"
            }
            Self::Footer => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer"
            }
            Self::Hyperlink => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
            }
            Self::Image => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
            }
            Self::VbaProject => "http://schemas.microsoft.com/office/2006/relationships/vbaProject",
            Self::Other => {
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/other"
            }
        }
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
    pub kind: RelKind,
    pub target: RelTarget,
}

/// An open package: the archive, the content types, and the bytes read so
/// far against [`MAX_INFLATED`].
pub struct Package<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
    /// Each part's index in the archive, by its lower-case name.
    index: BTreeMap<String, usize>,
    defaults: BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
    inflated: u64,
}

/// The eight bytes an OLE compound file opens on.
const COMPOUND_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

impl<'a> Package<'a> {
    /// The package `bytes` hold.
    ///
    /// # Errors
    ///
    /// [`OpcError::CompoundFile`] for a legacy or sealed document,
    /// [`OpcError::NotAZip`], [`OpcError::TooManyParts`], and any error of
    /// reading `[Content_Types].xml`, which every package carries.
    pub fn open(bytes: &'a [u8]) -> Result<Self, OpcError> {
        if bytes.starts_with(&COMPOUND_FILE) {
            return Err(OpcError::CompoundFile);
        }
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| OpcError::NotAZip)?;
        if archive.len() > MAX_PARTS {
            return Err(OpcError::TooManyParts);
        }
        // `file_names` walks the archive in the order of its index.
        let index = archive
            .file_names()
            .zip(0..)
            .filter_map(|(name, at)| Some((name.ok()?.to_ascii_lowercase(), at)))
            .collect::<BTreeMap<_, _>>();
        let mut package = Self {
            archive,
            index,
            defaults: BTreeMap::new(),
            overrides: BTreeMap::new(),
            inflated: 0,
        };
        let types = PartName("[Content_Types].xml".to_owned());
        let bytes = package
            .part(&types)?
            .ok_or(OpcError::MissingPart("[Content_Types].xml"))?;
        let root = xml::parse(&bytes)?;
        for entry in root.elements() {
            let content_type = entry.attr(Ns::None, "ContentType").unwrap_or_default();
            if entry.is(Ns::ContentTypes, "Default") {
                if let Some(extension) = entry.attr(Ns::None, "Extension") {
                    package
                        .defaults
                        .insert(extension.to_ascii_lowercase(), content_type.to_owned());
                }
            } else if entry.is(Ns::ContentTypes, "Override")
                && let Some(name) = entry.attr(Ns::None, "PartName").and_then(PartName::new)
            {
                package
                    .overrides
                    .insert(name.key(), content_type.to_owned());
            }
        }
        Ok(package)
    }

    /// Whether the package holds the part `name`.
    #[must_use]
    pub fn contains(&self, name: &PartName) -> bool {
        self.index.contains_key(&name.key())
    }

    /// The names of every part, as the archive spells them.
    /// A name that is not UTF-8 is left out: no part the codec reads has one.
    pub fn part_names(&self) -> impl Iterator<Item = std::borrow::Cow<'_, str>> {
        self.archive.file_names().filter_map(Result::ok)
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
    /// method the codec does not link.
    pub fn part(&mut self, name: &PartName) -> Result<Option<Payload<PartBytes>>, OpcError> {
        let Some(&at) = self.index.get(&name.key()) else {
            return Ok(None);
        };
        let file = self
            .archive
            .by_index(at)
            .map_err(|_| OpcError::UnreadablePart)?;
        let left = MAX_INFLATED.saturating_sub(self.inflated);
        let cap = MAX_PART_SIZE.min(left);
        let mut bytes = Vec::new();
        // One byte past the cap tells a part at the cap from one past it,
        // whatever size the archive's header claims.
        file.take(cap.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| OpcError::UnreadablePart)?;
        let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if read > cap {
            let bound = if cap == MAX_PART_SIZE {
                MAX_PART_SIZE
            } else {
                MAX_INFLATED
            };
            return Err(OpcError::TooLarge(Count::new(bound)));
        }
        self.inflated = self.inflated.saturating_add(read);
        Ok(Some(Payload::new(bytes)))
    }

    /// The relationships of the part `source`, or of the package for
    /// [`root`]. A part with no relationships part has none.
    ///
    /// # Errors
    ///
    /// Any error of reading the relationships part.
    pub fn relationships(&mut self, source: &PartName) -> Result<Vec<Relationship>, OpcError> {
        let Some(bytes) = self.part(&source.relationships())? else {
            return Ok(Vec::new());
        };
        let root = xml::parse(&bytes)?;
        Ok(root
            .elements()
            .filter(|element| element.is(Ns::PackageRelationships, "Relationship"))
            .filter_map(|element| {
                let id = RelId::new(element.attr(Ns::None, "Id")?);
                let kind = RelKind::of(element.attr(Ns::None, "Type").unwrap_or_default());
                let target = element.attr(Ns::None, "Target")?;
                let target = if element.attr(Ns::None, "TargetMode") == Some("External") {
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
                RelTarget::Part(name) if rel.kind == kind => Some(name),
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

// A part read becomes a picture of the model as it is.
impl Becomes<crate::model::ImageBytes> for PartBytes {}

/// The bytes of a written package. `Payload<PackageBytes>`.
pub enum PackageBytes {}

impl PayloadKind for PackageBytes {
    const NAME: &'static str = "PackageBytes";
}

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
    /// one is given.
    pub fn part(&mut self, name: PartName, content_type: Option<&str>, bytes: Payload<PartBytes>) {
        if let Some(content_type) = content_type {
            self.overrides.push((name.clone(), content_type.to_owned()));
        }
        self.parts.push((name, bytes));
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
    /// [`OpcError::WriteFailed`] when the ZIP writer fails.
    pub fn finish(self) -> Result<Payload<PackageBytes>, OpcError> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), OpcError> {
            zip.start_file(name, options)
                .map_err(|_| OpcError::WriteFailed)?;
            zip.write_all(bytes).map_err(|_| OpcError::WriteFailed)
        };
        put("[Content_Types].xml", self.content_types().as_bytes())?;
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
        let mut xml = XmlWriter::new();
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

/// The relationships part of `source`. A target part is written relative to
/// the folder of `source`, as Word writes it.
fn relationships_part(
    source: &PartName,
    rels: &[(RelId, RelKind, RelTarget)],
) -> Payload<PartBytes> {
    let mut xml = XmlWriter::new();
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
                let relative = name
                    .as_str()
                    .strip_prefix(source.folder())
                    .unwrap_or(name.as_str());
                xml.empty(
                    "Relationship",
                    &[
                        ("Id", id.as_str()),
                        ("Type", kind.uri()),
                        ("Target", relative),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> PartName {
        PartName::new(text).expect("a valid name")
    }

    #[test]
    fn a_target_resolves_against_its_source_folder() {
        let document = name("word/document.xml");
        assert_eq!(
            document.resolve("media/a.png"),
            Some(name("word/media/a.png"))
        );
        assert_eq!(
            document.resolve("../docProps/core.xml"),
            Some(name("docProps/core.xml"))
        );
        assert_eq!(
            document.resolve("/word/styles.xml"),
            Some(name("word/styles.xml"))
        );
        assert_eq!(document.resolve("../../../etc/passwd"), None);
        assert_eq!(root().resolve("word/document.xml"), Some(document.clone()));
        assert_eq!(
            document.relationships(),
            name("word/_rels/document.xml.rels")
        );
        assert_eq!(root().relationships(), name("_rels/.rels"));
    }

    #[test]
    fn a_written_package_reads_back_with_its_types_and_relationships() {
        let mut writer = PackageWriter::new();
        let main = name("word/document.xml");
        writer.part(
            main.clone(),
            Some("application/main+xml"),
            Payload::new(b"<a/>".to_vec()),
        );
        writer.default_type("png", "image/png");
        writer.part(name("word/media/i.png"), None, Payload::new(vec![1, 2, 3]));
        writer.relate(
            &root(),
            RelKind::OfficeDocument,
            RelTarget::Part(main.clone()),
        );
        let image = writer.relate(
            &main,
            RelKind::Image,
            RelTarget::Part(name("word/media/i.png")),
        );
        writer.relate(
            &main,
            RelKind::Hyperlink,
            RelTarget::External("https://example.org/?a=1&b=2".to_owned()),
        );
        let bytes = writer.finish().expect("writes");

        let mut package = Package::open(bytes.as_ref()).expect("opens");
        assert_eq!(
            package
                .related(&root(), RelKind::OfficeDocument)
                .expect("rels"),
            Some(main.clone())
        );
        assert_eq!(package.content_type(&main), Some("application/main+xml"));
        assert_eq!(
            package.content_type(&name("WORD/media/i.PNG")),
            Some("image/png")
        );
        let rels = package.relationships(&main).expect("rels");
        assert_eq!(rels[0].id, image);
        assert_eq!(
            rels[1].target,
            RelTarget::External("https://example.org/?a=1&b=2".to_owned())
        );
        assert_eq!(
            package.part(&name("word/media/i.png")).expect("reads"),
            Some(Payload::new(vec![1, 2, 3]))
        );
    }

    #[test]
    fn the_same_parts_write_the_same_bytes() {
        let write = || {
            let mut writer = PackageWriter::new();
            writer.part(name("a.xml"), None, Payload::new(b"<a/>".to_vec()));
            writer.finish().expect("writes")
        };
        assert_eq!(write(), write());
    }

    #[test]
    fn a_compound_file_and_noise_are_refused() {
        let mut compound = COMPOUND_FILE.to_vec();
        compound.extend_from_slice(&[0; 64]);
        assert_eq!(Package::open(&compound).err(), Some(OpcError::CompoundFile));
        assert_eq!(Package::open(b"not a zip").err(), Some(OpcError::NotAZip));
    }

    #[test]
    fn a_part_that_inflates_past_its_bound_is_refused_while_it_inflates() {
        let mut writer = PackageWriter::new();
        // Zeros deflate a thousandfold, so the archive stays small.
        let size = usize::try_from(MAX_PART_SIZE).expect("fits") + 1;
        writer.part(name("big.xml"), None, Payload::new(vec![0; size]));
        let bytes = writer.finish().expect("writes");
        assert!(bytes.as_ref().len() < 1024 * 1024);
        let mut package = Package::open(bytes.as_ref()).expect("opens");
        assert_eq!(
            package.part(&name("big.xml")).err(),
            Some(OpcError::TooLarge(Count::new(MAX_PART_SIZE)))
        );
    }
}
