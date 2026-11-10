//! The constructs of a Word file the reader names in its report
//! (`ambre-spec-interop.md section 3`). The words mirror the shared
//! vocabulary, which a codec cannot name (`crate::report`).
//!
//! A construct is named once per outcome with its count (`INTEROP-003`). The
//! reader names what it carried of the constructs an application tracks, a
//! comment or a tracked change, and every construct it approximated or
//! dropped. The mapping to an application's form names its own losses in
//! its own words.

use serde::{Deserialize, Serialize};

/// A construct of WordprocessingML, as the user reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WordConstruct {
    Comment,
    /// A tracked insertion or deletion.
    TrackedChange,
    /// A tracked move, read as a deletion and an insertion.
    TrackedMove,
    /// A tracked change of format: the reader keeps the format now in force.
    FormatChange,
    /// A tracked insertion or deletion of a paragraph mark.
    ParagraphMarkChange,
    Footnote,
    Endnote,
    Table,
    Picture,
    /// A picture laid out around the text, read as one in the line.
    FloatingPicture,
    /// A picture whose encoding the codec does not carry, such as EMF.
    PictureFormat,
    /// A picture linked from outside the package, which the reader fetches
    /// none of.
    LinkedPicture,
    /// A shape, a chart, a diagram or a text box.
    Drawing,
    /// An embedded object, such as a spreadsheet or a package.
    EmbeddedObject,
    Equation,
    /// A computed field, read as the result it last showed.
    Field,
    /// A content control, read as its content.
    ContentControl,
    /// A section past the first, read as part of the first.
    Section,
    /// A header or a footer for a first or an even page.
    PageBand,
    Bookmark,
    /// A numbering format the model has no form for, read as decimal.
    ListFormat,
    /// Foreign content a part imports as is, such as HTML.
    ImportedChunk,
    /// A macro project. The reader never reads its bytes.
    Macro,
    /// An element the reader does not recognise. Its text is kept.
    UnknownElement,
}

impl WordConstruct {
    /// The words the user reads for one of it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::TrackedChange => "tracked change",
            Self::TrackedMove => "tracked move",
            Self::FormatChange => "tracked format change",
            Self::ParagraphMarkChange => "tracked paragraph mark",
            Self::Footnote => "footnote",
            Self::Endnote => "endnote",
            Self::Table => "table",
            Self::Picture => "picture",
            Self::FloatingPicture => "floating picture",
            Self::PictureFormat => "picture in an unsupported format",
            Self::LinkedPicture => "linked picture",
            Self::Drawing => "drawing",
            Self::EmbeddedObject => "embedded object",
            Self::Equation => "equation",
            Self::Field => "field",
            Self::ContentControl => "content control",
            Self::Section => "section",
            Self::PageBand => "first or even page header or footer",
            Self::Bookmark => "bookmark",
            Self::ListFormat => "list numbering format",
            Self::ImportedChunk => "imported content",
            Self::Macro => "macro",
            Self::UnknownElement => "unknown element",
        }
    }

    /// The words the user reads for several of it.
    #[must_use]
    pub const fn plural(self) -> &'static str {
        match self {
            Self::Comment => "comments",
            Self::TrackedChange => "tracked changes",
            Self::TrackedMove => "tracked moves",
            Self::FormatChange => "tracked format changes",
            Self::ParagraphMarkChange => "tracked paragraph marks",
            Self::Footnote => "footnotes",
            Self::Endnote => "endnotes",
            Self::Table => "tables",
            Self::Picture => "pictures",
            Self::FloatingPicture => "floating pictures",
            Self::PictureFormat => "pictures in an unsupported format",
            Self::LinkedPicture => "linked pictures",
            Self::Drawing => "drawings",
            Self::EmbeddedObject => "embedded objects",
            Self::Equation => "equations",
            Self::Field => "fields",
            Self::ContentControl => "content controls",
            Self::Section => "sections",
            Self::PageBand => "first or even page headers or footers",
            Self::Bookmark => "bookmarks",
            Self::ListFormat => "list numbering formats",
            Self::ImportedChunk => "imported contents",
            Self::Macro => "macros",
            Self::UnknownElement => "unknown elements",
        }
    }
}
