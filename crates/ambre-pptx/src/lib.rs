//! The PowerPoint codec: a deck model, and a writer and a reader of it. The
//! writer turns a deck into a `.pptx` file
//! (`3-Toile/features/ambre-spec-toile-pptx-export.md`). The reader turns a
//! file back into a deck (`3-Toile/features/ambre-spec-toile-pptx-import.md`).
//!
//! The crate has two halves, and each build takes the ones it needs:
//!
//! | Half | Built in | What it holds |
//! | :--- | :--- | :--- |
//! | The model, [`model`] and [`report`] | every build | the slides, their items and their notes, in points, and what a reader lost |
//! | The codec, `codec` feature | the converter module and the host | the package layer [`opc`], the writer [`write`] and the reader [`read`] |
//!
//! An application maps its document onto the model, and reports what the
//! model could not hold. The writer is a pure function: bytes out, no state,
//! no clock, no network. The same deck gives the same bytes.

pub mod model;
pub mod report;

/// The package layer the three OOXML codecs share.
#[cfg(feature = "codec")]
pub use ambre_opc as opc;
#[cfg(feature = "codec")]
mod pml;
#[cfg(feature = "codec")]
mod read;
#[cfg(feature = "codec")]
mod xml;

#[cfg(feature = "codec")]
pub use pml::write;
#[cfg(feature = "codec")]
pub use read::read;

/// Why the writer produced no file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PptxError {
    /// The deck holds no slide, or the file names none.
    #[error("the deck holds no slide")]
    NoSlide,
    /// A picture named as PNG holds bytes that are no PNG file.
    #[error("a picture is no PNG file")]
    NotPng,
    /// A picture named as JPEG holds bytes that are no JPEG file.
    #[error("a picture is no JPEG file")]
    NotJpeg,
    /// The file is no presentation: its package names no presentation
    /// part, or a part it names is missing.
    #[error("the file holds no presentation")]
    NoPresentation,
    /// A part of the file is no well-formed XML.
    #[error("a part of the file does not read")]
    Unreadable,
    /// The deck, its masters flattened onto every slide, passes a bound of
    /// the reader.
    #[error("the deck is larger than the reader accepts")]
    TooLarge,
    /// An XML part could not be written, in the writer's words.
    #[error("a part could not be written: {0}")]
    Xml(String),
    /// A part name the writer builds breaks the grammar of a package.
    #[error("a part name is malformed")]
    PartName,
    /// The package layer refused, in its own words.
    #[cfg(feature = "codec")]
    #[error(transparent)]
    Package(#[from] opc::OpcError),
}

#[cfg(test)]
#[cfg(feature = "codec")]
mod read_back;
#[cfg(test)]
#[cfg(feature = "codec")]
mod tests;
