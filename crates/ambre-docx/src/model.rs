//! The model of a Word document, as the format holds it
//! (`ambre-spec-prose-import-export.md section 3`).
//!
//! The model names the constructs of WordprocessingML that the codec reads
//! and writes, and nothing of Prose. A Word comment anchors on two markers
//! that may sit in two paragraphs, so the model keeps the two markers. A
//! tracked change wraps runs, so a run carries its revision. The mapping to
//! an application's form lives with that application.
//!
//! Every text a person wrote is content. So `Debug` prints its length alone,
//! through `Redacted`, and a log carries none of it (`INTEROP-004`).

use std::fmt;
use std::marker::PhantomData;

use ambre_core::{
    Authored, Bounded, Bounds, FontSize, Instant, LinkUrl, Payload, PayloadKind, PrintLength, Pt,
    Redacted, Refusal, Rgb, Saturates, Slant, Strikethrough, Underline, Weight,
};
use serde::{Deserialize, Serialize};

/// The date of a comment or a tracked change. Word writes `w:date` as the
/// author's wall time with a `Z` it does not mean, so that text alone names
/// no instant. Word since 2023 writes the true instant beside it, in
/// `w16du:dateUtc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WordDate {
    /// `w:date` alone: a wall time with no zone known, held as if it were
    /// UTC. A reader turns it into an instant with a zone of its choosing.
    Floating(Instant<Authored>),
    /// The true instant of `w16du:dateUtc`, and the wall time `w:date` spells.
    Zoned {
        utc: Instant<Authored>,
        wall: Instant<Authored>,
    },
}

/// A whole Word document: its body, its notes, its comments and its page.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WordDocument {
    /// The title of `docProps/core.xml`.
    pub title: Option<String>,
    /// The page of the first section. A later section is approximated as the
    /// first one, and the reader reports it.
    pub page: Option<PageSetup>,
    pub body: Vec<Block>,
    /// The default header of the first section.
    pub header: Option<Vec<Block>>,
    /// The default footer of the first section.
    pub footer: Option<Vec<Block>>,
    pub footnotes: Vec<Note>,
    pub endnotes: Vec<Note>,
    /// Every comment, a reply included. A reply names its parent.
    pub comments: Vec<Comment>,
}

impl fmt::Debug for WordDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WordDocument")
            .field("title", &self.title.as_ref().map(Redacted::new))
            .field("page", &self.page)
            .field("body", &self.body)
            .field("header", &self.header)
            .field("footer", &self.footer)
            .field("footnotes", &self.footnotes)
            .field("endnotes", &self.endnotes)
            .field("comments", &self.comments)
            .finish()
    }
}

/// The size, the margins and the orientation of a page.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageSetup {
    pub width: PrintLength<Pt>,
    pub height: PrintLength<Pt>,
    pub margins: Margins,
    pub orientation: Orientation,
}

/// The four margins of a page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    pub top: PrintLength<Pt>,
    pub bottom: PrintLength<Pt>,
    pub left: PrintLength<Pt>,
    pub right: PrintLength<Pt>,
}

/// Which way a page turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// One block of a body, a cell, a note, a band or a comment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

/// One paragraph: its role, its list, its alignment and its content.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    pub role: ParagraphRole,
    pub list: Option<ListMembership>,
    pub align: Option<Alignment>,
    pub inlines: Vec<Inline>,
}

/// What a paragraph is, as its style says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParagraphRole {
    #[default]
    Body,
    /// A heading, at the outline level its style gives.
    Heading(Bounded<HeadingDepth>),
    /// The title of the document.
    Title,
    /// A quotation.
    Quote,
    /// Preformatted source code.
    Code,
}

impl ParagraphRole {
    /// The role a style's name gives, in lower case: a heading of its
    /// level, the title, a quotation or source code. Word names its built-in
    /// styles in English whatever the language of its interface, and
    /// LibreOffice names its own the same way in both formats. The Word and
    /// the OpenDocument readers share this one list, so a name cannot read
    /// as two roles.
    #[must_use]
    pub fn from_style_name(name: &str) -> Option<Self> {
        let compact: String = name.chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(level) = compact.strip_prefix("heading") {
            let level: u8 = level.parse().ok()?;
            return Bounded::<HeadingDepth>::new(level).map(Self::Heading);
        }
        match compact.as_str() {
            "title" => Some(Self::Title),
            "quote" | "intensequote" | "quotation" | "quotations" | "blockquotation"
            | "blocktext" => Some(Self::Quote),
            "sourcecode" | "code" | "htmlpreformatted" | "preformattedtext" | "macrotext" => {
                Some(Self::Code)
            }
            _ => None,
        }
    }
}

/// The outline level of a heading: 1 to 9, as Word numbers its built-in
/// heading styles.
pub enum HeadingDepth {}

impl Bounds for HeadingDepth {
    type Repr = u8;
    const MIN: u8 = 1;
    const MAX: u8 = 9;
    const NAME: &'static str = "HeadingDepth";
    const REFUSAL: Refusal = Refusal::new("heading level ", " is outside 1 to 9");
}

/// The paragraph's place in a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListMembership {
    /// The list the paragraph belongs to. Two paragraphs of one list count
    /// on together, and a new list starts its count again.
    pub list: WordId<ListInstance>,
    /// The nesting, 0 for the outermost.
    pub level: Bounded<ListDepth>,
    pub kind: ListKind,
}

/// The nesting of a list paragraph: 0 to 8, the nine levels of a Word list.
pub enum ListDepth {}

impl Bounds for ListDepth {
    type Repr = u8;
    const MIN: u8 = 0;
    const MAX: u8 = 8;
    const NAME: &'static str = "ListDepth";
    const REFUSAL: Refusal = Refusal::new("list level ", " is outside 0 to 8");
}

// A level past the ninth reads as the ninth, as Word clamps it.
impl Saturates for ListDepth {}

/// What a list paragraph shows before its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListKind {
    Bullet,
    Numbered(NumberFormat),
}

/// How a numbered list counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumberFormat {
    Decimal,
    LowerLetter,
    UpperLetter,
    LowerRoman,
    UpperRoman,
}

/// Where the lines of a paragraph sit between its margins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alignment {
    Start,
    Center,
    End,
    Justified,
}

/// A table: its column widths and its rows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Table {
    /// The width of each grid column. Empty when the file gives none.
    pub columns: Vec<PrintLength<Pt>>,
    pub rows: Vec<TableRow>,
}

/// One row of a table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TableRow {
    pub role: RowRole,
    pub cells: Vec<TableCell>,
}

/// Whether a row repeats as the table's header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowRole {
    #[default]
    Body,
    Header,
}

/// One cell of a row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    /// The grid columns the cell spans, 1 for a plain cell.
    pub span: Bounded<GridSpan>,
    pub merge: VerticalMerge,
    pub fill: Option<Rgb>,
    pub blocks: Vec<Block>,
}

impl Default for TableCell {
    fn default() -> Self {
        Self {
            // The lowest span is one column, a plain cell.
            span: Bounded::<GridSpan>::MIN,
            merge: VerticalMerge::default(),
            fill: None,
            blocks: Vec::new(),
        }
    }
}

/// The grid columns one cell spans: 1 to 63, the columns of a Word table.
pub enum GridSpan {}

impl Bounds for GridSpan {
    type Repr = u8;
    const MIN: u8 = 1;
    const MAX: u8 = 63;
    const NAME: &'static str = "GridSpan";
    const REFUSAL: Refusal = Refusal::new("a cell span of ", " is outside 1 to 63");
}

// A span past the grid reads as the widest one, as Word clamps it.
impl Saturates for GridSpan {}

/// How a cell joins the cell above it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerticalMerge {
    /// The cell stands alone.
    #[default]
    None,
    /// The cell starts a merge the cells below continue.
    Restart,
    /// The cell continues the merge of the cell above.
    Continue,
}

/// One piece of a paragraph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Inline {
    Run(Run),
    Break(BreakKind),
    Tab,
    /// The call of a footnote or an endnote.
    NoteCall(NoteKind, WordId<NoteMarker>),
    /// The start of a comment's anchor. The anchor may end in another
    /// paragraph.
    CommentStart(WordId<CommentMarker>),
    /// The end of a comment's anchor.
    CommentEnd(WordId<CommentMarker>),
    Link(Link),
    Image(Image),
}

/// What a break ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakKind {
    Line,
    Page,
    Column,
}

/// Which kind of note a call names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteKind {
    Footnote,
    Endnote,
}

/// A run of text with one format and one revision.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    pub format: RunFormat,
    /// The tracked change the run belongs to.
    pub revision: Option<Revision>,
}

impl Run {
    /// A run of plain text, settled and with no format.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

impl fmt::Debug for Run {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Run")
            .field("text", &Redacted::new(&self.text))
            .field("format", &self.format)
            .field("revision", &self.revision)
            .finish()
    }
}

/// The look of a run.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunFormat {
    pub bold: Weight,
    pub italic: Slant,
    pub underline: Underline,
    pub strike: Strikethrough,
    pub position: VerticalPosition,
    pub size: Option<FontSize>,
    pub color: Option<Rgb>,
    /// The fill behind the text: a highlight or a shading.
    pub fill: Option<Rgb>,
    /// The family the run names, as the file spells it.
    pub font: Option<String>,
}

impl fmt::Debug for RunFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunFormat")
            .field("bold", &self.bold)
            .field("italic", &self.italic)
            .field("underline", &self.underline)
            .field("strike", &self.strike)
            .field("position", &self.position)
            .field("size", &self.size)
            .field("color", &self.color)
            .field("fill", &self.fill)
            .field("font", &self.font.as_ref().map(Redacted::new))
            .finish()
    }
}

/// Where a run sits against the baseline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerticalPosition {
    #[default]
    Baseline,
    Superscript,
    Subscript,
}

/// A tracked change: what it does, who made it and when.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub kind: RevisionKind,
    pub id: WordId<RevisionMarker>,
    /// The author's name, as the file spells it.
    pub author: String,
    pub date: Option<WordDate>,
}

impl fmt::Debug for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Revision")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .field("author", &Redacted::new(&self.author))
            .field("date", &self.date)
            .finish()
    }
}

/// Whether a tracked change adds its runs or removes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevisionKind {
    Insertion,
    Deletion,
}

/// A hyperlink: its target and the runs it shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub target: LinkTarget,
    pub runs: Vec<Run>,
}

/// Where a hyperlink leads.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkTarget {
    /// An address outside the document. The reader keeps it and fetches
    /// nothing.
    External(LinkUrl),
    /// A bookmark inside the document, by name.
    Anchor(String),
}

impl fmt::Debug for LinkTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::External(url) => f.debug_tuple("External").field(url).finish(),
            Self::Anchor(name) => f.debug_tuple("Anchor").field(&Redacted::new(name)).finish(),
        }
    }
}

/// A picture in the text, with its bytes.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// The alternative text.
    pub description: String,
    pub width: PrintLength<Pt>,
    pub height: PrintLength<Pt>,
    pub format: ImageFormat,
    pub bytes: Payload<ImageBytes>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("description", &Redacted::new(&self.description))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// The bytes of a picture part. `Payload<ImageBytes>`.
pub enum ImageBytes {}

impl PayloadKind for ImageBytes {
    const NAME: &'static str = "ImageBytes";
}

/// The encoding of a picture part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Tiff,
    Svg,
}

impl ImageFormat {
    /// The extension of the part's name.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::Svg => "svg",
        }
    }

    /// The media type `[Content_Types].xml` gives the extension.
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
            Self::Tiff => "image/tiff",
            Self::Svg => "image/svg+xml",
        }
    }

    /// The format an extension names, in any case.
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" | "jpe" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            "bmp" => Some(Self::Bmp),
            "tif" | "tiff" => Some(Self::Tiff),
            "svg" => Some(Self::Svg),
            _ => None,
        }
    }
}

/// A footnote or an endnote body. A call names it by its id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: WordId<NoteMarker>,
    pub blocks: Vec<Block>,
}

/// A comment: who wrote it, when, its body, its parent and its state.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: WordId<CommentMarker>,
    /// The author's name, as the file spells it.
    pub author: String,
    pub date: Option<WordDate>,
    pub paragraphs: Vec<Paragraph>,
    /// The comment this one answers. A comment with none opens a thread.
    pub parent: Option<WordId<CommentMarker>>,
    pub state: CommentState,
}

impl fmt::Debug for Comment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Comment")
            .field("id", &self.id)
            .field("author", &Redacted::new(&self.author))
            .field("date", &self.date)
            .field("paragraphs", &self.paragraphs)
            .field("parent", &self.parent)
            .field("state", &self.state)
            .finish()
    }
}

/// Whether a comment's thread awaits an answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentState {
    #[default]
    Open,
    Resolved,
}

/// A decimal id of WordprocessingML, `ST_DecimalNumber`: the `w:id` that
/// pairs a comment with its markers, a note with its call, and names a
/// tracked change. The marker says which of the four it pairs.
///
/// The file fixes the meaning and the width, so the value is no count and no
/// order: it only matches. The writer numbers every id again, in the order of
/// the model (`WordDocument::renumbered`).
pub struct WordId<K>(u32, PhantomData<fn() -> K>);

impl<K> WordId<K> {
    /// The id the file spells as `value`.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value, PhantomData)
    }

    /// The number the file spells.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The same number under the marker `J`. A file numbers its comments
    /// and its tracked changes in one space of ids. So the first id of one
    /// kind can follow the last of the other.
    #[must_use]
    pub const fn shared_with<J>(self) -> WordId<J> {
        WordId::new(self.0)
    }
}

impl<K> Clone for WordId<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for WordId<K> {}

impl<K> PartialEq for WordId<K> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<K> Eq for WordId<K> {}

impl<K> PartialOrd for WordId<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K> Ord for WordId<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<K> std::hash::Hash for WordId<K> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<K> fmt::Debug for WordId<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WordId({})", self.0)
    }
}

impl<K> Serialize for WordId<K> {
    fn serialize<S: serde::Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(out)
    }
}

impl<'de, K> Deserialize<'de> for WordId<K> {
    fn deserialize<D: serde::Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        u32::deserialize(input).map(Self::new)
    }
}

/// The marker of a comment's id.
pub enum CommentMarker {}

/// The marker of a footnote's or an endnote's id.
pub enum NoteMarker {}

/// The marker of a tracked change's id.
pub enum RevisionMarker {}

/// The marker of a list's id, the `w:numId` of its paragraphs.
pub enum ListInstance {}
