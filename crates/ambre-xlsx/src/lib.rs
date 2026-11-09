//! The xlsx codec: an Excel workbook read into a plain [`Workbook`] and
//! written back (`2-Trame/features/ambre-spec-trame-import-export.md`).
//!
//! The crate holds pure functions over bytes. It links no browser binding and
//! no thread protocol, so the converter module and the host tests call the
//! same two entries: [`read`] and [`write`]. Each one answers what it did as
//! a [`Report`] in the words of the interop vocabulary.
//!
//! The model speaks Trame's style vocabulary, and the reader brings the file
//! to it. The reader approximates or drops each construct the model cannot
//! hold, and counts it. The formula keeps both dialects, as `formula` states.

pub mod cellref;
pub mod formula;
pub mod model;
pub mod opc;
mod read;
pub mod report;
mod styles;
mod write;
mod xml;

use ambre_core::{Cells, Count, FileBytes, Payload};

pub use model::Workbook;
pub use report::Report;

/// The most cells a read keeps across a workbook: about a million, Excel's
/// row count. A cell of the model costs about 80 bytes, so the cap holds a
/// read under the 100 MB of the client memory budget.
pub const MAX_CELLS: Count<Cells> = Count::new(1 << 20);

/// What a read produced: the workbook, and what it carried and lost.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub workbook: Workbook,
    pub report: Report,
}

/// What a write produced: the file, and what it carried and approximated.
#[derive(Debug, Clone, PartialEq)]
pub struct Exported {
    pub bytes: Payload<FileBytes>,
    pub report: Report,
}

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

impl std::error::Error for XlsxError {}

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

/// Write `workbook` as an xlsx file.
///
/// # Errors
///
/// [`XlsxError::NoSheet`] for a workbook without a sheet, and
/// [`XlsxError::DuplicateSheetName`] for two sheets of one name.
pub fn write(workbook: &Workbook) -> Result<Exported, XlsxError> {
    write::write(workbook)
}
