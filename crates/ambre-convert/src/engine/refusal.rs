//! Each codec's error, mapped to the refusal that crosses back to the Logic
//! thread. A codec error names a part, a size or a parser's words. The answer
//! names the kind alone, so no text of the file leaves the module
//! (`INTEROP-004`, `BUS-046`).

use ambre_core::FileRefusal;

/// The refusal a package error reads as.
#[cfg(feature = "office")]
pub(super) const fn opc(error: ambre_xlsx::opc::OpcError) -> FileRefusal {
    use ambre_xlsx::opc::OpcError;
    match error {
        OpcError::CompoundFile => FileRefusal::Protected,
        OpcError::NotAZip => FileRefusal::NotAPackage,
        OpcError::TooManyParts | OpcError::TooLarge(_) => FileRefusal::PastBound,
        OpcError::UnreadablePart | OpcError::Malformed(_) => FileRefusal::Malformed,
        OpcError::MissingPart(_) | OpcError::DanglingRelationship => FileRefusal::PartMissing,
        OpcError::WriteFailed => FileRefusal::Unwritable,
        OpcError::OtherDocument => FileRefusal::OtherDocument,
    }
}

/// The refusal a Word error reads as.
#[cfg(feature = "office")]
pub(super) const fn docx(error: ambre_docx::DocxError) -> FileRefusal {
    use ambre_docx::DocxError;
    match error {
        DocxError::Package(error) => opc(error),
        DocxError::Xml(ambre_docx::XmlError::TooLarge) => FileRefusal::PastBound,
        DocxError::Xml(_) => FileRefusal::Malformed,
        DocxError::NoMainPart | DocxError::MissingPart(_) => FileRefusal::PartMissing,
        DocxError::NotWord => FileRefusal::OtherDocument,
    }
}

/// The refusal an OpenDocument text error reads as.
#[cfg(feature = "office")]
pub(super) const fn odt(error: ambre_odt::OdtError) -> FileRefusal {
    use ambre_odt::OdtError;
    match error {
        OdtError::Package(error) => opc(error),
        OdtError::Xml(ambre_odt::xml::XmlError::TooLarge) | OdtError::TooLarge => {
            FileRefusal::PastBound
        }
        OdtError::Xml(_) => FileRefusal::Malformed,
        OdtError::NotText => FileRefusal::OtherDocument,
        OdtError::MissingPart(_) => FileRefusal::PartMissing,
    }
}

/// The refusal a spreadsheet error reads as.
#[cfg(feature = "office")]
pub(super) fn xlsx(error: &ambre_xlsx::XlsxError) -> FileRefusal {
    use ambre_xlsx::XlsxError;
    match error {
        XlsxError::Package(error) => opc(*error),
        XlsxError::CompoundFile => FileRefusal::Protected,
        XlsxError::NotAWorkbook => FileRefusal::OtherDocument,
        XlsxError::MissingPart(_) => FileRefusal::PartMissing,
        XlsxError::Malformed(..) | XlsxError::DuplicateSheetName(_) => FileRefusal::Malformed,
        XlsxError::NoSheet => FileRefusal::Empty,
        XlsxError::TooManyCells(_) | XlsxError::TooManySheets(_) | XlsxError::TooMuchText(_) => {
            FileRefusal::PastBound
        }
    }
}

/// The refusal a PowerPoint writer error reads as.
#[cfg(feature = "office")]
pub(super) fn pptx(error: &ambre_pptx::PptxError) -> FileRefusal {
    use ambre_pptx::PptxError;
    match error {
        PptxError::NoSlide => FileRefusal::Empty,
        PptxError::NotPng | PptxError::NotJpeg | PptxError::Xml(_) | PptxError::PartName => {
            FileRefusal::Unwritable
        }
        PptxError::NoPresentation => FileRefusal::OtherDocument,
        PptxError::Unreadable => FileRefusal::Malformed,
        PptxError::TooLarge => FileRefusal::PastBound,
        PptxError::Package(error) => opc(*error),
    }
}

/// The refusal a PDF writer error reads as.
#[cfg(feature = "pdf")]
pub(super) fn pdf(error: &ambre_pdf::PdfWriteError) -> FileRefusal {
    match error {
        ambre_pdf::PdfWriteError::Empty => FileRefusal::Empty,
        ambre_pdf::PdfWriteError::Serialize(_) => FileRefusal::Unwritable,
    }
}

/// The refusal a JPEG decoder refusal reads as.
#[cfg(feature = "images")]
pub(super) const fn jpeg(refusal: crate::jpeg::JpegRefusal) -> FileRefusal {
    use crate::jpeg::JpegRefusal;
    match refusal {
        JpegRefusal::NotJpeg => FileRefusal::OtherDocument,
        JpegRefusal::PastBound { .. } => FileRefusal::PastBound,
        JpegRefusal::SizeUnavailable | JpegRefusal::FrameMismatch | JpegRefusal::Undecodable => {
            FileRefusal::Malformed
        }
    }
}

/// The kind of a PNG or a GIF refusal, as the converter states it.
#[cfg(feature = "images")]
pub(super) const fn raster(refusal: crate::raster::RasterRefusal) -> FileRefusal {
    use crate::raster::RasterRefusal;
    match refusal {
        RasterRefusal::OtherFormat => FileRefusal::OtherDocument,
        RasterRefusal::PastBound { .. } => FileRefusal::PastBound,
        RasterRefusal::SizeUnavailable
        | RasterRefusal::FrameMismatch
        | RasterRefusal::Undecodable => FileRefusal::Malformed,
    }
}
