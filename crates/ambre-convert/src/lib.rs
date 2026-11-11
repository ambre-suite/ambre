//! The converter modules: every file format Ambre reads or writes, one wasm
//! instance per format family. The Logic thread loads a family's module on
//! the first conversion of that family (`ambre-spec-architecture.md section
//! 2`, the converter modules).
//!
//! The crate has three halves, and each build takes the ones it needs:
//!
//! | Half | Built in | What it holds |
//! | :--- | :--- | :--- |
//! | The request types, [`Conversion`] and [`Converted`], and the trees in [`markup`] and [`images`] | every build | the one interface: bytes in, records, trees or bytes out |
//! | The engine, one feature per family | one family in its module build, every family on the host | the third-party readers and writers of that family |
//! | The loader, [`Converter`] | every build | a module instantiated on wasm, the engine called in place on the host |
//!
//! A module parses, and the application maps. A reader in a module answers a
//! format-level tree, never an application's model. So a module links no
//! application crate, and holds no CRDT and no document. The application maps
//! the tree onto its model in the Main instance.
//!
//! A request and its answer cross as postcard bytes, on the host as in the
//! browser. The host tier therefore runs the same encoding the browser does.
//! A module imports no capability. The loader answers every import with a
//! trap, so a converter never reaches a key, the relay or the page
//! (`ARCH-079`).

use ambre_core::{
    ConverterFamily, Count, CountKind, FileBytes, Payload, PayloadKind, Refused, Refuses,
};
use ambre_draw::list::PrintDocument;
use ambre_prose_form::FormBytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod images;
pub mod markup;

#[cfg(feature = "engine")]
pub mod engine;

#[cfg(all(target_arch = "wasm32", feature = "engine"))]
mod exports;

mod loader;
pub use loader::Converter;

/// One conversion the Logic thread asks of a module. Each conversion names
/// its family, and only that family's module serves it ([`Conversion::family`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Conversion {
    /// Read the records of a CSV file (`trame-import-export`).
    ReadCsv {
        data: Payload<FileBytes>,
        delimiter: CsvDelimiter,
    },
    /// Write records as a CSV file (`trame-import-export`).
    WriteCsv {
        records: Vec<Vec<String>>,
        delimiter: CsvDelimiter,
    },
    /// Write a recorded document as a PDF file (`toile-pdf-export`).
    WritePdf { document: PrintDocument },
    /// Read Markdown text into Prose's intermediate form, through the
    /// Markdown mapping (`prose-import-export`). Answered by
    /// [`Converted::Form`].
    ReadMarkdown { text: String },
    /// Write Prose's intermediate form, the bytes of an
    /// `ambre_prose_form::ProseIr`, as Markdown text in `flavor`
    /// (`prose-import-export`). Answered by [`Converted::Written`].
    WriteMarkdown {
        form: Payload<FormBytes>,
        flavor: ambre_prose_form::MarkdownFlavor,
    },
    /// Read an HTML document into the blocks a paste writes
    /// (`prose-clipboard`). Answered by [`Converted::Pasted`].
    ReadHtml { html: String },
    /// Read an SVG file into its drawing (`toile-import-export`).
    ReadSvg { data: Payload<FileBytes> },
    /// Decode a WebP picture into its pixels, which the Main instance cannot
    /// decode itself (`toile-import-export`). Answered by
    /// [`Converted::Pixels`].
    DecodeWebp { data: Payload<FileBytes> },
    /// Read a Word package into Prose's intermediate form, through the Word
    /// mapping (`prose-import-export`). Answered by [`Converted::Form`].
    ReadDocx {
        data: Payload<FileBytes>,
        /// The session's zone, which a date Word wrote as a wall time alone
        /// is read in.
        zone: ambre_prose_form::TimeZone,
    },
    /// Write Prose's intermediate form, the bytes of an
    /// `ambre_prose_form::ProseIr`, as a Word package (`prose-import-export`).
    /// Answered by [`Converted::Written`].
    WriteDocx {
        form: Payload<FormBytes>,
        /// The session's zone, which each date's wall time is written in.
        zone: ambre_prose_form::TimeZone,
    },
    /// Read an Excel workbook into Trame's form, an `ambre_xlsx::book::Book`
    /// (`trame-import-export`). Answered by [`Converted::Workbook`]. An Excel date is a serial: a
    /// wall time with no zone. The import writes the serial as it is, so the
    /// read takes no zone.
    ReadXlsx { data: Payload<FileBytes> },
    /// Write the bytes of an `ambre_xlsx::book::Book` as an xlsx package
    /// (`trame-import-export`). Answered by [`Converted::WrittenWorkbook`].
    WriteXlsx { workbook: Payload<WorkbookBytes> },
    /// Write the bytes of an `ambre_pptx::model::Deck` as a PowerPoint
    /// package (`toile-pptx-export`). Answered by [`Converted::File`]. Toile
    /// maps the board onto the deck and builds the whole report, so the
    /// writer carries every item and adds none.
    WritePptx { deck: Payload<DeckBytes> },
}

impl Conversion {
    /// The family whose module serves this conversion.
    #[must_use]
    pub const fn family(&self) -> ConverterFamily {
        match self {
            Self::ReadCsv { .. } | Self::WriteCsv { .. } => ConverterFamily::Tables,
            Self::WritePdf { .. } => ConverterFamily::Pdf,
            Self::ReadMarkdown { .. } | Self::WriteMarkdown { .. } | Self::ReadHtml { .. } => {
                ConverterFamily::Markup
            }
            Self::ReadSvg { .. } | Self::DecodeWebp { .. } => ConverterFamily::Images,
            Self::ReadDocx { .. }
            | Self::WriteDocx { .. }
            | Self::ReadXlsx { .. }
            | Self::WriteXlsx { .. }
            | Self::WritePptx { .. } => ConverterFamily::Ooxml,
        }
    }
}

/// What a conversion produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Converted {
    /// The records a CSV file holds, in order, each one a list of fields.
    Records {
        records: Vec<Vec<String>>,
        /// The records the reader could not read and skipped.
        unreadable: Count<UnreadableRecords>,
    },
    /// The bytes of a written file.
    File(Payload<FileBytes>),
    /// The blocks the HTML half of a paste reads into, in the shape this
    /// application's own copy writes.
    Pasted(Vec<ambre_prose_form::clip::ClipBlock>),
    /// The drawing of an SVG file.
    Svg(images::SvgTree),
    /// The pixels of a decoded picture.
    Pixels(images::Pixels),
    /// The bytes of a `(ProseIr, FormReport)`: the form a Word package or a
    /// Markdown text mapped onto, and what the reader and the mapping lost.
    Form(Payload<FormBytes>),
    /// The bytes of a written file, and the bytes of the `FormReport` of
    /// what its format could not hold.
    Written {
        file: Payload<FileBytes>,
        report: Payload<FormBytes>,
    },
    /// The bytes of a `(Book, Report)`: Trame's form of the workbook an xlsx
    /// file read into, and what the reader carried and lost.
    Workbook(Payload<WorkbookBytes>),
    /// The bytes of a written xlsx file, and the bytes of the `Report` of
    /// what it carried and approximated.
    WrittenWorkbook {
        file: Payload<FileBytes>,
        report: Payload<WorkbookBytes>,
    },
}

/// The role of Trame's form of a workbook, or of a report, as it crosses a
/// module boundary:
/// postcard bytes. Only the OOXML module and Trame decode them, so no other
/// module links their serde code. `Payload<WorkbookBytes>`.
pub enum WorkbookBytes {}

impl PayloadKind for WorkbookBytes {
    const NAME: &'static str = "WorkbookBytes";
}

/// The bytes of `value`, an Excel model or report.
///
/// # Errors
///
/// [`ConvertError::Refused`] when postcard cannot write the value.
pub fn encode_workbook<T: Serialize>(value: &T) -> Result<Payload<WorkbookBytes>, ConvertError> {
    postcard::to_allocvec(value)
        .map(Payload::new)
        .map_err(|e| ConvertError::Refused(e.to_string()))
}

/// The value `bytes` hold, an Excel model or report.
///
/// # Errors
///
/// [`ConvertError::Malformed`] for bytes that decode to no such value.
pub fn decode_workbook<T: serde::de::DeserializeOwned>(
    bytes: &Payload<WorkbookBytes>,
) -> Result<T, ConvertError> {
    postcard::from_bytes(bytes.as_bytes()).map_err(|_| ConvertError::Malformed)
}

/// The role of a PowerPoint deck as it crosses into the OOXML module:
/// postcard bytes. Only that module decodes them, so no other module links
/// the serde code of the deck. `Payload<DeckBytes>`.
pub enum DeckBytes {}

impl PayloadKind for DeckBytes {
    const NAME: &'static str = "DeckBytes";
}

/// The bytes of `deck`.
///
/// # Errors
///
/// [`ConvertError::Refused`] when postcard cannot write the deck.
pub fn encode_deck(deck: &ambre_pptx::model::Deck) -> Result<Payload<DeckBytes>, ConvertError> {
    postcard::to_allocvec(deck)
        .map(Payload::new)
        .map_err(|e| ConvertError::Refused(e.to_string()))
}

/// The deck `bytes` hold.
///
/// # Errors
///
/// [`ConvertError::Malformed`] for bytes that decode to no deck.
pub fn decode_deck(bytes: &Payload<DeckBytes>) -> Result<ambre_pptx::model::Deck, ConvertError> {
    postcard::from_bytes(bytes.as_bytes()).map_err(|_| ConvertError::Malformed)
}

/// Whether `bytes` open as a WebP file: a RIFF container of the `WEBP` form.
/// A reader asks before it chooses the images module for a picture.
#[must_use]
pub fn is_webp(file: &Payload<FileBytes>) -> bool {
    let bytes = file.as_bytes();
    bytes.get(..4) == Some(b"RIFF".as_slice()) && bytes.get(8..12) == Some(b"WEBP".as_slice())
}

/// The records of a CSV file the reader skipped.
pub enum UnreadableRecords {}

impl CountKind for UnreadableRecords {
    type Repr = u32;
    const NAME: &'static str = "UnreadableRecordCount";
}

/// Why a conversion produced nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    /// The converter read the request and refused it, in its own words.
    #[error("{0}")]
    Refused(String),
    /// The module trapped or could not be called. The words are the
    /// browser's.
    #[error("the converter stopped: {0}")]
    Stopped(String),
    /// The module answered bytes that decode to no answer.
    #[error("the converter answered bytes no request reads")]
    Malformed,
    /// No converter is linked into this build and none was loaded.
    #[error("this build carries no converter")]
    Absent,
    /// The conversion belongs to another family than the module loaded.
    #[error("the {held} module does not serve a {asked} conversion")]
    OtherFamily {
        held: ConverterFamily,
        asked: ConverterFamily,
    },
}

/// The answer as it crosses back: the converted value, or the converter's
/// refusal in its own words.
pub type Answer = Result<Converted, String>;

/// The role of a request as it crosses into the module: the postcard bytes of
/// a [`Conversion`]. `Payload<EncodedRequest>`.
pub enum EncodedRequest {}

impl PayloadKind for EncodedRequest {
    const NAME: &'static str = "EncodedRequest";
}

/// The role of an answer as it crosses back: the postcard bytes of an
/// [`Answer`]. `Payload<EncodedAnswer>`.
pub enum EncodedAnswer {}

impl PayloadKind for EncodedAnswer {
    const NAME: &'static str = "EncodedAnswer";
}

/// The request as the module reads it.
///
/// # Errors
///
/// [`ConvertError::Refused`] when postcard cannot write the request, which a
/// request built from these types never triggers.
pub fn encode_request(conversion: &Conversion) -> Result<Payload<EncodedRequest>, ConvertError> {
    postcard::to_allocvec(conversion)
        .map(Payload::new)
        .map_err(|e| ConvertError::Refused(e.to_string()))
}

/// The answer the module wrote, read back.
///
/// # Errors
///
/// [`ConvertError::Malformed`] for bytes that decode to no answer, and
/// [`ConvertError::Refused`] for the converter's own refusal.
pub fn decode_answer(bytes: &Payload<EncodedAnswer>) -> Result<Converted, ConvertError> {
    let answer: Answer =
        postcard::from_bytes(bytes.as_bytes()).map_err(|_| ConvertError::Malformed)?;
    answer.map_err(ConvertError::Refused)
}

/// A byte refused as a CSV field separator: the double quote, CR, LF, or a
/// byte outside ASCII. The marker of its [`Refused<DelimiterByte>`], which
/// echoes the byte.
pub enum DelimiterByte {}

impl Refuses for DelimiterByte {
    type Raw = u8;
    const NAME: &'static str = "InvalidCsvDelimiter";

    fn message(raw: &Self::Raw, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "byte {raw:#04x} cannot separate CSV fields")
    }
}

/// The byte that separates the fields of a CSV record (`trame-import-export`).
/// It is an ASCII byte other than the double quote, which opens a quoted
/// field, and CR and LF, which end a record. A byte outside ASCII would cut a
/// UTF-8 character in two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CsvDelimiter(u8);

impl CsvDelimiter {
    pub const COMMA: Self = Self(b',');
    pub const SEMICOLON: Self = Self(b';');
    pub const TAB: Self = Self(b'\t');

    /// The separator `byte` is.
    ///
    /// # Errors
    ///
    /// [`Refused<DelimiterByte>`] for the double quote, CR, LF and any byte
    /// outside ASCII.
    pub const fn new(byte: u8) -> Result<Self, Refused<DelimiterByte>> {
        match byte {
            b'"' | b'\r' | b'\n' | 0x80.. => Err(Refused::new(byte)),
            _ => Ok(Self(byte)),
        }
    }

    /// The byte a CSV reader or writer is configured with.
    pub const fn byte(self) -> u8 {
        self.0
    }
}

impl Serialize for CsvDelimiter {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.0)
    }
}

/// Read through [`CsvDelimiter::new`], so a request never carries a separator
/// the reader would refuse.
impl<'de> Deserialize<'de> for CsvDelimiter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let byte = u8::deserialize(deserializer)?;
        Self::new(byte).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_separator_does_not_decode() {
        let bytes = postcard::to_allocvec(&b'"').expect("a byte encodes");
        assert!(postcard::from_bytes::<CsvDelimiter>(&bytes).is_err());
        let bytes = postcard::to_allocvec(&b';').expect("a byte encodes");
        assert_eq!(
            postcard::from_bytes::<CsvDelimiter>(&bytes).ok(),
            Some(CsvDelimiter::SEMICOLON)
        );
    }

    #[test]
    fn bytes_that_are_no_answer_are_malformed() {
        assert_eq!(
            decode_answer(&Payload::new(vec![0xff, 0xff])),
            Err(ConvertError::Malformed)
        );
    }
}
