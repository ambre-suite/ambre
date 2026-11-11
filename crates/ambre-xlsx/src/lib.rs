//! The spreadsheet codecs: an Excel workbook or an OpenDocument
//! spreadsheet read into a plain [`Workbook`] and written back
//! (`2-Trame/features/ambre-spec-trame-import-export.md`).
//!
//! The crate holds pure functions over bytes. It links no browser binding and
//! no thread protocol, so the converter module and the host tests call the
//! same two entries: [`read`] and [`write`]. Each one answers what it did as
//! a [`Report`] in the words of the interop vocabulary.
//!
//! The model speaks Trame's style vocabulary, and the reader brings the file
//! to it. The reader approximates or drops each construct the model cannot
//! hold, and counts it. The formula keeps both dialects, as `formula` states.

pub mod book;
pub mod cellref;
pub mod formula;
pub mod model;
/// The package layer the office codecs share: the OOXML package, and the
/// OpenDocument package in its `odf` module.
#[cfg(feature = "codec")]
pub use ambre_opc as opc;
#[cfg(feature = "codec")]
mod ods;
#[cfg(feature = "codec")]
mod read;
pub mod report;
#[cfg(feature = "codec")]
mod styles;
#[cfg(feature = "codec")]
mod write;
#[cfg(feature = "codec")]
mod xml;

use ambre_core::{Cells, Count};
#[cfg(feature = "codec")]
use ambre_core::{FileBytes, Payload};

pub use model::Workbook;
pub use report::Report;

/// The most cells a read keeps across a workbook: about a million, Excel's
/// row count. A cell of the model costs about 80 bytes, so the cap holds a
/// read under the 100 MB of the client memory budget.
pub const MAX_CELLS: Count<Cells> = Count::new(1 << 20);

#[cfg(feature = "codec")]
/// What a read produced: the workbook, and what it carried and lost.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub workbook: Workbook,
    pub report: Report,
}

#[cfg(feature = "codec")]
/// What a write produced: the file, and what it carried and approximated.
#[derive(Debug, Clone, PartialEq)]
pub struct Exported {
    pub bytes: Payload<FileBytes>,
    pub report: Report,
}

#[cfg(feature = "codec")]
/// Why a read or a write produced nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XlsxError {
    /// The package layer refused the bytes.
    Package(opc::OpcError),
    /// An OLE compound file: a legacy `.xls`, or a workbook under a password.
    CompoundFile,
    /// The package holds no workbook part.
    NotAWorkbook,
    /// A part the workbook names is absent.
    MissingPart(opc::PartName),
    /// A part did not read as XML.
    Malformed(opc::PartName, xml::XmlError),
    /// The workbook holds no worksheet, or the model to write holds no sheet.
    NoSheet,
    /// The workbook holds more cells than a read keeps.
    TooManyCells(Count<Cells>),
    /// Two sheets of the model to write share one name, case ignored.
    DuplicateSheetName(ambre_core::Validated<model::SheetName>),
}

#[cfg(feature = "codec")]
impl std::fmt::Display for XlsxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Package(e) => e.fmt(f),
            Self::CompoundFile => {
                f.write_str("the file is a legacy or password-protected Excel file")
            }
            Self::NotAWorkbook => f.write_str("the file holds no workbook"),
            Self::MissingPart(name) => write!(f, "the workbook lacks its part {}", name.as_str()),
            Self::Malformed(name, e) => {
                write!(f, "the part {} did not read: {}", name.as_str(), e.reason())
            }
            Self::NoSheet => f.write_str("the workbook holds no sheet"),
            Self::TooManyCells(cap) => {
                write!(f, "the workbook holds more than {} cells", cap.get())
            }
            Self::DuplicateSheetName(_) => f.write_str("two sheets share one name"),
        }
    }
}

#[cfg(feature = "codec")]
impl std::error::Error for XlsxError {}

#[cfg(feature = "codec")]
/// Read the workbook `bytes` hold.
///
/// # Errors
///
/// [`XlsxError`] when the bytes hold no workbook a reader can read. That is
/// no zip archive, a compound file, or a package without a workbook or a
/// worksheet. A malformed part and a workbook past [`MAX_CELLS`] fail too.
pub fn read(bytes: &Payload<FileBytes>) -> Result<Imported, XlsxError> {
    read::read(bytes)
}

#[cfg(feature = "codec")]
/// Write `workbook` as an xlsx file.
///
/// # Errors
///
/// [`XlsxError::NoSheet`] for a workbook without a sheet, and
/// [`XlsxError::DuplicateSheetName`] for two sheets of one name.
pub fn write(workbook: &Workbook) -> Result<Exported, XlsxError> {
    write::write(workbook)
}

#[cfg(feature = "codec")]
/// Read the OpenDocument spreadsheet `bytes` hold, into the workbook and the
/// report an xlsx file of the same content reads into.
///
/// # Errors
///
/// [`XlsxError`] when the bytes hold no spreadsheet a reader can read. That
/// is no zip archive, a compound file, another kind of document, or a
/// package without its content part or a table. A malformed part and a
/// spreadsheet past [`MAX_CELLS`] fail too.
pub fn read_ods(bytes: &Payload<FileBytes>) -> Result<Imported, XlsxError> {
    ods::read(bytes)
}

#[cfg(feature = "codec")]
/// Write `workbook` as an OpenDocument spreadsheet.
///
/// # Errors
///
/// [`XlsxError::NoSheet`] for a workbook without a sheet, and
/// [`XlsxError::DuplicateSheetName`] for two sheets of one name.
pub fn write_ods(workbook: &Workbook) -> Result<Exported, XlsxError> {
    ods::write(workbook)
}
