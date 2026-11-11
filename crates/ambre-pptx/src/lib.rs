//! The PowerPoint codec: a deck model, and the writer that turns it into a
//! `.pptx` file (`3-Toile/features/ambre-spec-toile-pptx-export.md`).
//!
//! The crate has two halves, and each build takes the ones it needs:
//!
//! | Half | Built in | What it holds |
//! | :--- | :--- | :--- |
//! | The model, [`model`] | every build | the slides, their items and their notes, in points |
//! | The codec, `codec` feature | the converter module and the host | the package layer [`opc`] and the writer [`write`] |
//!
//! An application maps its document onto the model, and reports what the
//! model could not hold. The writer is a pure function: bytes out, no state,
//! no clock, no network. The same deck gives the same bytes.

pub mod model;

/// The package layer the three OOXML codecs share.
#[cfg(feature = "codec")]
pub use ambre_opc as opc;
#[cfg(feature = "codec")]
mod pml;
#[cfg(feature = "codec")]
mod xml;

#[cfg(feature = "codec")]
pub use pml::write;

/// Why the writer produced no file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PptxError {
    /// The deck holds no slide.
    #[error("the deck holds no slide")]
    NoSlide,
    /// A picture holds bytes that are no PNG file.
    #[error("a picture is no PNG file")]
    NotPng,
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
