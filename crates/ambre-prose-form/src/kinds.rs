//! The kinds the intermediate form names, which Prose re-exports at their
//! old paths. They live here so that a converter module, which links no
//! application crate, can read and write the form
//! (`4-Prose/features/ambre-spec-prose-import-export.md section 3`).

use std::fmt;

use ambre_core::{Refused, Refuses, Rule};
use serde::{Deserialize, Serialize};

/// The number a footnote is called by, one for the first call in document
/// order. It is counted, never stored (`PROSE-FEAT-013`).
pub enum FootnoteCall {}

impl ambre_core::OrdinalKind for FootnoteCall {
    const NAME: &'static str = "FootnoteOrdinal";
}

/// Whether a to-do item's checkbox is ticked.
///
/// A clipboard payload carries it, so the variant that was `false` comes
/// first and a payload written before reads the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TodoMark {
    /// Not ticked: the item is still to do.
    #[default]
    Open,
    /// Ticked: the item is done.
    Done,
}

impl TodoMark {
    /// [`TodoMark::Done`] when the stored or read checkbox is ticked.
    #[must_use]
    pub fn done_if(ticked: bool) -> Self {
        if ticked { Self::Done } else { Self::Open }
    }

    /// The checkbox as the document stores it.
    #[must_use]
    pub fn is_done(self) -> bool {
        self == Self::Done
    }
}

/// One of the six heading levels the format declares
/// (`features/ambre-spec-prose-headings.md`).
///
/// A command carries one of these, so a level outside one to six cannot reach
/// a block through a command. A level read back from a document may still lie
/// outside, and [`HeadingLevel::clamp`] brings it to the nearer end. The type
/// serializes as the bare number, and a decoded number outside one to six is
/// refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum HeadingLevel {
    H1 = 1,
    H2 = 2,
    H3 = 3,
    H4 = 4,
    H5 = 5,
    H6 = 6,
}

impl HeadingLevel {
    /// The six levels, shallowest first.
    pub const ALL: [Self; 6] = [Self::H1, Self::H2, Self::H3, Self::H4, Self::H5, Self::H6];

    /// The deepest level.
    pub const MAX: Self = Self::H6;

    /// The level `level` names, or `None` outside one to six.
    #[must_use]
    pub const fn new(level: u8) -> Option<Self> {
        Some(match level {
            1 => Self::H1,
            2 => Self::H2,
            3 => Self::H3,
            4 => Self::H4,
            5 => Self::H5,
            6 => Self::H6,
            _ => return None,
        })
    }

    /// `level` brought inside one to six: zero reads as a level one, and
    /// anything past six as a level six.
    #[must_use]
    pub const fn clamp(level: u8) -> Self {
        match Self::new(level) {
            Some(level) => level,
            None if level == 0 => Self::H1,
            None => Self::H6,
        }
    }

    /// The level as a number, one to six.
    #[must_use]
    pub const fn get(self) -> u8 {
        self as u8
    }
}

/// A decoded heading level outside one to six: the marker of its
/// [`Refused<HeadingLevelCode>`], which echoes the level.
pub enum HeadingLevelCode {}

impl Refuses for HeadingLevelCode {
    type Raw = u8;
    const NAME: &'static str = "InvalidHeadingLevel";

    fn message(raw: &Self::Raw, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a heading level is one to six, not {raw}")
    }
}

impl TryFrom<u8> for HeadingLevel {
    type Error = Refused<HeadingLevelCode>;

    fn try_from(level: u8) -> Result<Self, Self::Error> {
        Self::new(level).ok_or(Refused::new(level))
    }
}

impl From<HeadingLevel> for u8 {
    fn from(level: HeadingLevel) -> Self {
        level.get()
    }
}

impl std::fmt::Display for HeadingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get())
    }
}

/// The rule of the language a code block is marked as. A
/// `Validated<CodeLang>` holds one. A tag is never empty: a block with no
/// language reads as plain and carries `None`. The user typed the tag after
/// a fence, so it opts in to no log: `Debug` prints its length alone.
pub enum CodeLang {}

/// The empty text, which names no language: a block without one is plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a code language is never empty")]
pub struct EmptyCodeLang;

impl Rule for CodeLang {
    type Error = EmptyCodeLang;

    fn admit(text: String) -> Result<String, EmptyCodeLang> {
        if text.is_empty() {
            Err(EmptyCodeLang)
        } else {
            Ok(text)
        }
    }
}

/// Which edit a suggestion proposes (`features/ambre-spec-prose-track-changes.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionKind {
    /// Text a reviewer typed: accepting keeps it, rejecting removes it.
    Insertion,
    /// Text a reviewer deleted: accepting removes it, rejecting keeps it.
    Deletion,
}

/// A heading level exactly as the document holds it.
///
/// Raw on purpose. An import is the only way a level outside one to six reaches
/// a document, and it stays as it came (`PROSE-FEAT-091`). A reader that works
/// only inside the range, the numbering first (`PROSE-FEAT-029`), takes
/// [`StoredLevel::clamped`]; an export and a diagnostic read the number itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StoredLevel(u8);

impl StoredLevel {
    /// The level a document holds, read from its attribute.
    #[must_use]
    pub const fn from_stored(level: u8) -> Self {
        Self(level)
    }

    /// The level a document holds for `level`, which is always one to six.
    #[must_use]
    pub const fn of(level: HeadingLevel) -> Self {
        Self(level.get())
    }

    /// The level as the document holds it, inside one to six or not.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The level brought inside one to six, to the nearer end.
    #[must_use]
    pub const fn clamped(self) -> HeadingLevel {
        HeadingLevel::clamp(self.0)
    }
}

impl std::fmt::Display for StoredLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

ambre_core::define_flag! {
    /// Whether a run is set as inline code.
    pub enum InlineCode {
        /// The run is set as prose.
        Prose,
        /// The run is set as code, in the monospace family.
        Code,
    }

    /// Whether a run carries the raised mark.
    pub enum Superscript {
        /// The run carries no raised mark.
        Level,
        /// The run carries the raised mark.
        Raised,
    }

    /// Whether a run carries the lowered mark.
    pub enum Subscript {
        /// The run carries no lowered mark.
        Level,
        /// The run carries the lowered mark.
        Lowered,
    }
}

/// A construct a format has no syntax for, the subject of Prose's
/// `ProseConstruct::Feature`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnsupportedFeature {
    PageFormat,
    HeaderAndFooter,
    ParagraphFormatting,
    HeadingLevel,
    MergedCells,
    Image,
    PageBreak,
    TableOfContents,
    TodoItem,
    ListNumbering,
    CellFormatting,
    Table,
    ColumnWidths,
    HeadingInCell,
    ListInCell,
    BlockInCell,
    ParagraphsInCell,
    Footnote,
    Underline,
    Superscript,
    Subscript,
    Strikethrough,
    Styling,
    Comment,
    Suggestion,
    CrossReference,
    /// A block other than a list item nested in a list item: Word lists are
    /// flat paragraphs, so it follows the item.
    BlockInListItem,
    /// A list nested past the nine levels of a Word list.
    ListDepth,
    /// The language of a code block.
    CodeLanguage,
    HorizontalRule,
    /// Inline code: Word has no such run, so a writer gives it a monospace
    /// font.
    InlineCode,
    /// An endnote, read as a footnote.
    Endnote,
    /// A picture whose bytes the export could not get: the file holds a
    /// placeholder in its place (`PROSE-IO-008`).
    PictureMissing,
    /// A picture's caption, written as the paragraph under it.
    PictureCaption,
    /// A picture aligned left or right, written in the line.
    PictureWrap,
}

impl UnsupportedFeature {
    /// The words the user reads for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PageFormat => "page format",
            Self::HeaderAndFooter => "header and footer",
            Self::ParagraphFormatting => "paragraph formatting",
            Self::HeadingLevel => "heading level outside 1 to 6",
            Self::MergedCells => "merged cells",
            Self::Image => "image",
            Self::PageBreak => "page break",
            Self::TableOfContents => "table of contents",
            Self::TodoItem => "to-do item",
            Self::ListNumbering => "list numbering style",
            Self::CellFormatting => "cell formatting",
            Self::Table => "table",
            Self::ColumnWidths => "column widths",
            Self::HeadingInCell => "heading in a table cell",
            Self::ListInCell => "list in a table cell",
            Self::BlockInCell => "block in a table cell",
            Self::ParagraphsInCell => "several paragraphs in a table cell",
            Self::Footnote => "footnote",
            Self::Underline => "underline",
            Self::Superscript => "superscript",
            Self::Subscript => "subscript",
            Self::Strikethrough => "strikethrough",
            Self::Styling => "colour, size or font",
            Self::Comment => "comment",
            Self::Suggestion => "suggestion",
            Self::CrossReference => "cross reference",
            Self::BlockInListItem => "block nested in a list item",
            Self::ListDepth => "list nesting deeper than nine levels",
            Self::CodeLanguage => "code block language",
            Self::HorizontalRule => "horizontal rule",
            Self::InlineCode => "inline code",
            Self::Endnote => "endnote",
            Self::PictureMissing => "picture not available",
            Self::PictureCaption => "picture caption",
            Self::PictureWrap => "picture with text wrapped beside it",
        }
    }
}

impl fmt::Display for UnsupportedFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The session's time zone, by its IANA name, such as `Europe/Paris`. A Word
/// date with no zone is a wall time. A reader turns it into an instant under
/// the zone's rules on that date, summer time included. A writer spells the
/// wall time of an instant under them. The session injects it as it
/// injects its identity, and no reader or writer guesses it from the machine.
///
/// The name alone crosses to the Office module, which holds the rules
/// (`word::Zone`): the Main instance links no time-zone database. A place
/// is personal, so `Debug` prints the length alone.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TimeZone(std::borrow::Cow<'static, str>);

/// A name no IANA zone could have: the marker of its [`Refused<TimeZoneName>`].
pub enum TimeZoneName {}

impl Refuses for TimeZoneName {
    type Raw = String;
    const NAME: &'static str = "InvalidTimeZone";

    fn message(raw: &Self::Raw, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a time-zone name is 1 to {} letters, digits, '/', '_', '+' or '-', not {} bytes of other text",
            TimeZone::NAME_MAX,
            raw.len()
        )
    }
}

impl TimeZone {
    /// UTC itself.
    pub const UTC: Self = Self(std::borrow::Cow::Borrowed("UTC"));

    /// The longest name a zone may carry, in bytes. The longest IANA name is
    /// under 32.
    pub const NAME_MAX: usize = 64;

    /// The zone the IANA name `name` spells. Whether the name exists is the
    /// module's to say, since the module holds the database.
    ///
    /// # Errors
    ///
    /// [`Refused<TimeZoneName>`] for an empty or overlong name, or one with a
    /// character no IANA name uses.
    pub fn named(name: impl Into<String>) -> Result<Self, Refused<TimeZoneName>> {
        let name = name.into();
        let plain = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '-'));
        if name.is_empty() || name.len() > Self::NAME_MAX || !plain {
            return Err(Refused::new(name));
        }
        Ok(Self(std::borrow::Cow::Owned(name)))
    }

    /// The IANA name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl Default for TimeZone {
    fn default() -> Self {
        Self::UTC
    }
}

impl fmt::Debug for TimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TimeZone")
            .field(&ambre_core::Redacted::new(self.name()))
            .finish()
    }
}

impl TryFrom<String> for TimeZone {
    type Error = Refused<TimeZoneName>;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Self::named(name)
    }
}

impl From<TimeZone> for String {
    fn from(zone: TimeZone) -> Self {
        zone.0.into_owned()
    }
}

/// An element a reader does not recognise, the subject of Prose's
/// `ProseConstruct::Element`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DroppedElement {
    Footnote,
    Html,
    Math,
    BlockQuote,
    Image,
    UnknownBlock,
    DeepList,
    TableOfContents,
    /// A picture that names an address outside the file: never fetched, so
    /// never resolved (`PROSE-IO-054`).
    UnresolvedImage,
    /// A script, never run (`PROSE-IO-016`).
    Script,
    /// A stylesheet or a font, never fetched or applied (`PROSE-IO-062`).
    Stylesheet,
    /// A frame, an object, an embed, a canvas or a media element.
    EmbeddedContent,
    /// A form control: a field, a button or a list box.
    FormControl,
}

impl DroppedElement {
    /// The words the user reads for it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Footnote => "footnote",
            Self::Html => "HTML",
            Self::Math => "math",
            Self::BlockQuote => "block quote",
            Self::Image => "image",
            Self::UnknownBlock => "unknown block",
            Self::DeepList => "list nesting deeper than 16",
            Self::TableOfContents => "table of contents",
            Self::UnresolvedImage => "image at an address outside the file, unresolved",
            Self::Script => "script",
            Self::Stylesheet => "stylesheet or font",
            Self::EmbeddedContent => "embedded content",
            Self::FormControl => "form control",
        }
    }
}

impl fmt::Display for DroppedElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The Markdown dialects a writer targets. GFM adds the table, the to-do item
/// and the footnote that CommonMark has no syntax for (`PROSE-IO-061`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkdownFlavor {
    Gfm,
    CommonMark,
}

/// How an ordered item's ordinal is spelled. A clipboard payload carries it
/// as its bare code, and a decoded code outside the five is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum OrderedStyle {
    Decimal = 0,
    LowerAlpha = 1,
    UpperAlpha = 2,
    LowerRoman = 3,
    UpperRoman = 4,
}

impl OrderedStyle {
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Decimal,
            1 => Self::LowerAlpha,
            2 => Self::UpperAlpha,
            3 => Self::LowerRoman,
            4 => Self::UpperRoman,
            _ => return None,
        })
    }
}

/// A decoded ordinal style outside the five codes: the marker of its
/// [`Refused<OrderedStyleCode>`], which echoes the code.
pub enum OrderedStyleCode {}

impl Refuses for OrderedStyleCode {
    type Raw = u8;
    const NAME: &'static str = "InvalidOrderedStyle";

    fn message(raw: &Self::Raw, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "an ordinal style is a code of zero to four, not {raw}")
    }
}

impl TryFrom<u8> for OrderedStyle {
    type Error = Refused<OrderedStyleCode>;

    fn try_from(code: u8) -> Result<Self, Self::Error> {
        Self::from_code(code).ok_or(Refused::new(code))
    }
}

impl From<OrderedStyle> for u8 {
    fn from(style: OrderedStyle) -> Self {
        style.code()
    }
}

/// The key an inline mark is stored under in a block's text
/// (`ambre-spec-prose-schema.md section 3`).
///
/// A key is a registered root, such as `b` or `fg`, or a root and an id
/// joined by a colon, such as `cm:<id>`. A document written by a newer client
/// may carry a key this build does not know, so the type holds any text. The
/// prose registry reads the root and rules on it (`MarkRoot::of`). The spelling is the one the
/// document stores. `Debug` prints the bare text, as the `String` it replaced
/// did.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MarkKey(String);

impl MarkKey {
    /// The key spelled as `text`.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The key as the document stores it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part before the first colon, which names the registry entry. A key
    /// with no colon is its own root.
    #[must_use]
    pub fn root(&self) -> &str {
        self.0.split(':').next().unwrap_or(&self.0)
    }
}

impl From<&str> for MarkKey {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl std::borrow::Borrow<str> for MarkKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for MarkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for MarkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
