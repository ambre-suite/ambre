//! The converter modules: every file format Ambre reads or writes, one wasm
//! instance per format family. The Logic thread loads a family's module on
//! the first conversion of that family (`ambre-spec-architecture.md section
//! 2`, the converter modules).
//!
//! The crate has three halves, and each build takes the ones it needs:
//!
//! | Half | Built in | What it holds |
//! | :--- | :--- | :--- |
//! | The request types, [`Conversion`] and [`Converted`], and the trees in [`markup`], [`images`] and [`boards`] | every build | the one interface: bytes in, records, trees or bytes out |
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

use ambre_client_core::FileBytes;
use ambre_core::NarrowCount;
use ambre_core::{ConverterFamily, Count, Octets, Payload, PayloadKind, Refused, Refuses};
use ambre_draw::list::PrintDocument;
use ambre_prose_form::FormBytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod boards;
pub mod images;
pub mod jpeg;
pub mod markup;
pub mod raster;
pub mod spelling;

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
    /// Read an HTML document into the items a paste writes
    /// (`prose-clipboard`). Answered by [`Converted::Pasted`].
    ReadHtml { html: String },
    /// Read a whole HTML document through the walk a paste runs, onto
    /// Prose's intermediate form (`prose-import-export`).
    /// Answered by [`Converted::Form`].
    ReadHtmlDocument { html: String },
    /// Write Prose's intermediate form as a standalone HTML document
    /// (`prose-import-export`). Answered by [`Converted::Written`].
    WriteHtml { form: Payload<FormBytes> },
    /// Read an SVG file into its drawing (`toile-import-export`).
    ReadSvg { data: Payload<FileBytes> },
    /// Read a Figma or a Miro export into its board, the format told apart by
    /// the file's own keys (`toile-import-export`). Answered by
    /// [`Converted::Board`].
    ReadBoard { data: Payload<FileBytes> },
    /// Decode a WebP picture into its pixels, which the Main instance cannot
    /// decode itself (`toile-import-export`). Answered by
    /// [`Converted::Pixels`].
    DecodeWebp { data: Payload<FileBytes> },
    /// Decode a JPEG picture into its samples, scaled and turned as asked,
    /// which the Main instance cannot decode itself. The converter thread
    /// runs it, never the Logic thread (`ARCH-097`). Answered by
    /// [`Converted::Pixels`].
    DecodeJpeg(jpeg::JpegDecode),
    /// Decode a PNG picture into its samples, scaled and turned as asked.
    /// The converter thread runs it, never the Logic thread (`ARCH-097`).
    /// Answered by [`Converted::Pixels`].
    DecodePng(raster::PngDecode),
    /// Decode a GIF picture into the frames of its animation, each scaled
    /// as asked. The converter thread runs it, never the Logic thread
    /// (`ARCH-097`). Answered by [`Converted::Frames`].
    DecodeGif(raster::GifDecode),
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
    /// Read an OpenDocument text into Prose's intermediate form, through
    /// the Word mapping (`prose-import-export`). Answered by
    /// [`Converted::Form`].
    ReadOdt {
        data: Payload<FileBytes>,
        /// The session's zone, which a date with no zone is read in.
        zone: ambre_prose_form::TimeZone,
    },
    /// Write Prose's intermediate form, the bytes of an
    /// `ambre_prose_form::ProseIr`, as an OpenDocument text
    /// (`prose-import-export`). Answered by [`Converted::Written`].
    WriteOdt {
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
    /// Read an OpenDocument spreadsheet into Trame's form, the same
    /// `ambre_xlsx::book::Book` an Excel workbook of the same content reads
    /// into (`trame-import-export`). Answered by [`Converted::Workbook`]. A
    /// date reads as the serial of its wall time, as an Excel date does, so
    /// the read takes no zone.
    ReadOds { data: Payload<FileBytes> },
    /// Write the bytes of an `ambre_xlsx::book::Book` as an OpenDocument
    /// spreadsheet (`trame-import-export`). Answered by
    /// [`Converted::WrittenWorkbook`].
    WriteOds { workbook: Payload<WorkbookBytes> },
    /// Write the bytes of an `ambre_pptx::model::Deck` as a PowerPoint
    /// package (`toile-pptx-export`). Answered by [`Converted::File`]. Toile
    /// maps the board onto the deck and builds the whole report, so the
    /// writer carries every item and adds none.
    WritePptx { deck: Payload<DeckBytes> },
    /// Read a PowerPoint package into a deck, its masters and layouts
    /// flattened onto each slide (`toile-pptx-import`). Answered by
    /// [`Converted::Deck`]. Toile maps the deck onto the board.
    ReadPptx {
        data: Payload<FileBytes>,
        /// The largest picture a thumbnail is decoded from, each edge
        /// apart: the bound of the board's import.
        bound: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
    },
    /// Read an OpenDocument presentation into the deck the PowerPoint
    /// reader answers, its master pages flattened onto each slide
    /// (`toile-pptx-import`). Answered by [`Converted::Deck`]: Toile maps
    /// both through one map.
    ReadOdp {
        data: Payload<FileBytes>,
        /// The largest picture a thumbnail is decoded from, each edge
        /// apart: the bound of the board's import.
        bound: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
    },
    /// Rasterize the one page of a recorded document, scaled to fill `size`,
    /// and write it as a PNG file (`toile-import-export`). The converter
    /// thread runs it, never the Logic thread (`toile-import-export`). Answered by
    /// [`Converted::File`]. An edge past [`RASTER_EDGE`] or a document of
    /// other than one page is refused.
    WritePng {
        document: PrintDocument,
        size: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
    },
    /// Look the words up in one dictionary (`prose-spell-check`). The
    /// converter thread runs it, never the Logic thread. Answered by
    /// [`Converted::Misspelled`].
    CheckSpelling(spelling::SpellCheck),
    /// Suggest at most five corrections of one word (`prose-spell-check`).
    /// Answered by [`Converted::Suggestions`].
    SuggestSpelling(spelling::SpellSuggest),
}

/// The longest edge a PNG export rasterizes. Past it, the caller scales the
/// page down, and the module refuses the request. At this edge the module
/// holds 64 MiB of pixels, the bound past which the converter thread loads
/// it again (`ambre-spec-budgets.md section 6`).
pub const RASTER_EDGE: ambre_core::Px<ambre_core::Image> = ambre_core::Px::new(4096);

impl Conversion {
    /// The family whose module serves this conversion.
    #[must_use]
    pub const fn family(&self) -> ConverterFamily {
        match self {
            Self::ReadCsv { .. } | Self::WriteCsv { .. } => ConverterFamily::Tables,
            Self::WritePdf { .. } | Self::WritePng { .. } => ConverterFamily::Pdf,
            Self::ReadMarkdown { .. }
            | Self::WriteMarkdown { .. }
            | Self::ReadHtml { .. }
            | Self::ReadHtmlDocument { .. }
            | Self::WriteHtml { .. } => ConverterFamily::Markup,
            Self::ReadBoard { .. } => ConverterFamily::Boards,
            Self::ReadSvg { .. }
            | Self::DecodeWebp { .. }
            | Self::DecodeJpeg(_)
            | Self::DecodePng(_)
            | Self::DecodeGif(_) => ConverterFamily::Images,
            Self::ReadDocx { .. }
            | Self::WriteDocx { .. }
            | Self::ReadOdt { .. }
            | Self::WriteOdt { .. }
            | Self::ReadXlsx { .. }
            | Self::WriteXlsx { .. }
            | Self::ReadOds { .. }
            | Self::WriteOds { .. }
            | Self::WritePptx { .. }
            | Self::ReadPptx { .. }
            | Self::ReadOdp { .. } => ConverterFamily::Office,
            Self::CheckSpelling(check) => check.dictionary.family(),
            Self::SuggestSpelling(suggest) => suggest.dictionary.family(),
        }
    }
}

/// A read of a text the caller holds: the borrowed twin of the three text
/// reads of [`Conversion`]. It encodes to the same postcard bytes as the
/// owned variant, so the module reads it as that variant. The caller thus
/// copies no text to build its request (`ARCH-126`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRead<'a> {
    /// As [`Conversion::ReadMarkdown`].
    Markdown(&'a str),
    /// As [`Conversion::ReadHtml`].
    HtmlPaste(&'a str),
    /// As [`Conversion::ReadHtmlDocument`].
    HtmlDocument(&'a str),
}

impl TextRead<'_> {
    /// The family whose module serves this read: the markup module.
    #[must_use]
    pub const fn family(&self) -> ConverterFamily {
        ConverterFamily::Markup
    }
}

/// The variant index and the names of each owned twin, as the derive of
/// [`Conversion`] writes them. A test holds the bytes of both equal, so an
/// edit of the order of [`Conversion`] fails it.
impl Serialize for TextRead<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStructVariant;
        let (index, variant, field, text) = match *self {
            Self::Markdown(text) => (3, "ReadMarkdown", "text", text),
            Self::HtmlPaste(text) => (5, "ReadHtml", "html", text),
            Self::HtmlDocument(text) => (6, "ReadHtmlDocument", "html", text),
        };
        let mut state = serializer.serialize_struct_variant("Conversion", index, variant, 1)?;
        state.serialize_field(field, text)?;
        state.end()
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
    /// The items the HTML half of a paste reads into: blocks in the shape
    /// this application's own copy writes, and the tables between them.
    Pasted(Vec<ambre_prose_form::clip::ClipItem>),
    /// The drawing of an SVG file.
    Svg(images::SvgTree),
    /// The bytes of the board of a Figma or a Miro export, a
    /// [`boards::BoardTree`]. Only the images module and Toile decode them,
    /// so no other module links the serde code of the board.
    Board(Payload<boards::BoardBytes>),
    /// The pixels of a decoded picture.
    Pixels(images::Pixels),
    /// The frames of a decoded animation, in order.
    Frames(images::Frames),
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
    /// or an OpenDocument file read into, and what the reader carried and
    /// lost.
    Workbook(Payload<WorkbookBytes>),
    /// The bytes of a written xlsx or OpenDocument file, and the bytes of the
    /// `Report` of what it carried and approximated.
    WrittenWorkbook {
        file: Payload<FileBytes>,
        report: Payload<WorkbookBytes>,
    },
    /// The bytes of an `ambre_pptx::model::ImportedDeck`: the deck a
    /// presentation read into, and what the reader lost. `thumbs` holds one
    /// entry per picture of the deck, in slide order then item order. An
    /// entry is the thumbnail of a JPEG picture. It is `None` for a PNG
    /// picture, and for a JPEG the module could not decode.
    Deck {
        deck: Payload<DeckBytes>,
        thumbs: Vec<Option<images::Pixels>>,
    },
    /// The words of a check the dictionary does not know, by their position
    /// in the list asked, in order.
    Misspelled(Vec<ambre_core::index::Pos<spelling::CheckedWord>>),
    /// The corrections of a word, best first, at most five.
    Suggestions(Vec<ambre_core::Validated<spelling::SpellWord>>),
}

/// The role of Trame's form of a workbook, or of a report, as it crosses a
/// module boundary:
/// postcard bytes. Only the Office module and Trame decode them, so no other
/// module links their serde code. `Payload<WorkbookBytes>`.
pub enum WorkbookBytes {}

impl PayloadKind for WorkbookBytes {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("WorkbookBytes");
}

/// The bytes of `value`, an Excel model or report.
///
/// # Errors
///
/// [`ConvertError::Refused`] when postcard cannot write the value.
pub fn encode_workbook<T: Serialize>(value: &T) -> Result<Payload<WorkbookBytes>, ConvertError> {
    postcard::to_allocvec(value)
        .map(Payload::new)
        .map_err(|_| ConvertError::Refused(ambre_core::FileRefusal::Unwritable))
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

/// The role of a PowerPoint deck as it crosses into the Office module and
/// back: postcard bytes. Only that module and Toile decode them, so no
/// other module links the serde code of the deck. `Payload<DeckBytes>`.
pub enum DeckBytes {}

impl PayloadKind for DeckBytes {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DeckBytes");
}

/// The bytes of `deck`.
///
/// # Errors
///
/// [`ConvertError::Refused`] when postcard cannot write the deck.
pub fn encode_deck(deck: &ambre_pptx::model::Deck) -> Result<Payload<DeckBytes>, ConvertError> {
    postcard::to_allocvec(deck)
        .map(Payload::new)
        .map_err(|_| ConvertError::Refused(ambre_core::FileRefusal::Unwritable))
}

/// The deck `bytes` hold.
///
/// # Errors
///
/// [`ConvertError::Malformed`] for bytes that decode to no deck.
pub fn decode_deck(bytes: &Payload<DeckBytes>) -> Result<ambre_pptx::model::Deck, ConvertError> {
    postcard::from_bytes(bytes.as_bytes()).map_err(|_| ConvertError::Malformed)
}

/// The deck a presentation reader answered, and its report.
///
/// # Errors
///
/// [`ConvertError::Malformed`] for bytes that decode to no such deck.
pub fn decode_imported_deck(
    bytes: &Payload<DeckBytes>,
) -> Result<ambre_pptx::model::ImportedDeck, ConvertError> {
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

impl NarrowCount for UnreadableRecords {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("UnreadableRecordCount");
}

/// What a file conversion on the converter thread does to the document that
/// asked for it (`ARCH-122`, `ARCH-123`). The Logic thread reads it to land a
/// replay before a read writes the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileWork {
    /// The read of an import: its answer writes the document.
    Read,
    /// The write of an export: its answer hands over a file.
    Write,
}

/// What an application keeps of one file conversion until its answer lands:
/// what the answer needs and the module call does not carry.
pub trait FileJob {
    /// Whether the answer writes the document or hands over a file.
    fn work(&self) -> FileWork;
}

/// The file conversions of one document on the converter thread
/// (`ARCH-122`, `ARCH-123`, `ARCH-129`). Those in flight wait by their ticket.
/// Those the Logic dispatcher has not drained wait in the outbox. Every
/// application holds this one type, so a format family that moves off the
/// Logic thread adds a job variant and no new mechanism. `R` is the marker of
/// the ticket, which the bus mints.
pub struct FileJobs<R: ambre_core::SeqKind, J> {
    in_flight: ambre_core::hash::HashMap<ambre_core::Seq<R>, J>,
    outbox: Vec<(ambre_core::Seq<R>, Conversion)>,
}

impl<R: ambre_core::SeqKind, J> Default for FileJobs<R, J> {
    fn default() -> Self {
        Self {
            in_flight: ambre_core::hash::HashMap::default(),
            outbox: Vec::new(),
        }
    }
}

impl<R: ambre_core::SeqKind, J: FileJob> FileJobs<R, J> {
    /// Owe the converter thread `conversion` under `ticket`. `job` lands its
    /// answer.
    pub fn ask(&mut self, ticket: ambre_core::Seq<R>, conversion: Conversion, job: J) {
        self.in_flight.insert(ticket, job);
        self.outbox.push((ticket, conversion));
    }

    /// The conversions owed and not yet sent, drained. The Logic dispatcher
    /// sends each one and never waits for it.
    pub fn take_requests(&mut self) -> Vec<(ambre_core::Seq<R>, Conversion)> {
        std::mem::take(&mut self.outbox)
    }

    /// What the conversion under `ticket` does, or `None` when the ticket is
    /// not in flight here.
    #[must_use]
    pub fn work(&self, ticket: ambre_core::Seq<R>) -> Option<FileWork> {
        self.in_flight.get(&ticket).map(FileJob::work)
    }

    /// The job under `ticket`, removed, once its answer arrived. `None` when
    /// the ticket is not in flight here.
    pub fn land(&mut self, ticket: ambre_core::Seq<R>) -> Option<J> {
        self.in_flight.remove(&ticket)
    }
}

/// Why a conversion produced nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    /// The converter read the request and refused it, for this reason.
    #[error("{0}")]
    Refused(ambre_core::FileRefusal),
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

impl ConvertError {
    /// The stop as the bus carries it. The loader's words stay here: the
    /// caller logs them where it meets the error (`BUS-046`).
    #[must_use]
    pub const fn stop_cause(&self) -> ambre_core::StopCause {
        use ambre_core::StopCause;
        match self {
            Self::Refused(refusal) => StopCause::FileRefused(*refusal),
            Self::Stopped(_) => StopCause::ConverterStopped,
            Self::Malformed => StopCause::ConverterAnswered,
            Self::Absent => StopCause::ConverterUnavailable,
            Self::OtherFamily { .. } => StopCause::OtherFamily,
        }
    }
}

/// Why a job of the converter thread produced nothing, as its owner reads it.
/// The Logic thread logs the converter's words where it makes the failure,
/// and the owner's view makes the words a person reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobFailure {
    /// The module could not be loaded, or the thread is gone: asking again
    /// later may succeed.
    Unavailable,
    /// The converter read the file and refused it: asking again cannot help.
    Refused,
}

/// The answer as it crosses back: the converted value, or the converter's
/// refusal as its kind.
pub type Answer = Result<Converted, ambre_core::FileRefusal>;

/// The role of a request as it crosses into the module: the postcard bytes of
/// a [`Conversion`]. `Payload<EncodedRequest>`.
pub enum EncodedRequest {}

impl PayloadKind for EncodedRequest {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("EncodedRequest");
}

/// The role of an answer as it crosses back: the postcard bytes of an
/// [`Answer`]. `Payload<EncodedAnswer>`.
pub enum EncodedAnswer {}

impl PayloadKind for EncodedAnswer {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("EncodedAnswer");
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
        .map_err(|_| ConvertError::Refused(ambre_core::FileRefusal::Request))
}

/// The request of a borrowed text read, as the module reads it: the bytes
/// of its owned twin in [`Conversion`].
///
/// # Errors
///
/// As [`encode_request`].
pub fn encode_text_read(read: &TextRead<'_>) -> Result<Payload<EncodedRequest>, ConvertError> {
    postcard::to_allocvec(read)
        .map(Payload::new)
        .map_err(|_| ConvertError::Refused(ambre_core::FileRefusal::Request))
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
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("InvalidCsvDelimiter");

    fn message(raw: &Self::Raw, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "byte {raw:#04x} cannot separate CSV fields")
    }
}

/// The largest CSV file an import reads: 8 MiB. The Trame import refuses a
/// file past it before any work.
///
/// A sheet of ten columns fills it with about 840 000 cells. The Office
/// module reads an Excel sheet of as many cells under its part bound. It
/// also holds the 100 000 rows of the grid at ten columns of short text. The
/// bound sets the dearest call of the tables module, so the tables answer
/// deadline is measured on a file at it (`ARCH-128`,
/// `ambre-spec-budgets.md section 8`).
pub const CSV_MAX_SIZE: Count<Octets> = Count::<Octets>::new(8 * 1024 * 1024);

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
    fn a_borrowed_text_read_encodes_as_its_owned_twin() {
        let text = "# Title\n\nA paragraph with <b>markup</b>.";
        let pairs = [
            (
                TextRead::Markdown(text),
                Conversion::ReadMarkdown {
                    text: text.to_owned(),
                },
            ),
            (
                TextRead::HtmlPaste(text),
                Conversion::ReadHtml {
                    html: text.to_owned(),
                },
            ),
            (
                TextRead::HtmlDocument(text),
                Conversion::ReadHtmlDocument {
                    html: text.to_owned(),
                },
            ),
        ];
        for (read, owned) in pairs {
            let borrowed = encode_text_read(&read).expect("a text read encodes");
            assert_eq!(
                borrowed.as_bytes(),
                encode_request(&owned)
                    .expect("a conversion encodes")
                    .as_bytes()
            );
            assert_eq!(read.family(), owned.family());
            assert_eq!(
                postcard::from_bytes::<Conversion>(borrowed.as_bytes()).ok(),
                Some(owned)
            );
        }
    }

    #[test]
    fn bytes_that_are_no_answer_are_malformed() {
        assert_eq!(
            decode_answer(&Payload::new(vec![0xff, 0xff])),
            Err(ConvertError::Malformed)
        );
    }
}
