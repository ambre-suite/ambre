//! A Word import and export, through the Office converter module
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The module reads a package into the intermediate form, and writes the form
//! back as a package. The Word codec and the Word mapping both run there, so
//! the Main instance links neither (`ambre-spec-budgets.md section 5`). This
//! half builds the request the converter thread runs, and carries the report
//! of the mapping onto Prose's own. It emits no command and sees no document
//! (`PROSE-IO-009`).

use ambre_bus::interop::FileFormat;
use ambre_convert::{Conversion, ConvertError, Converted};
use ambre_client_core::FileBytes;
use ambre_core::{Count, Octets, Payload};
use ambre_prose_form::TimeZone;

use super::ProseIr;
use crate::message::{ConversionReport, InteropError};

/// A package of text the Office module reads onto the Word model. An
/// OpenDocument text lands as its Word twin would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextPackage {
    Word,
    OpenDocument,
}

impl TextPackage {
    const fn file_format(self) -> FileFormat {
        match self {
            Self::Word => FileFormat::Docx,
            Self::OpenDocument => FileFormat::Odt,
        }
    }
}

/// The largest file a Word import hands the module. It is the bound of what
/// the package layer inflates (`ambre_opc::MAX_INFLATED`), which a stored
/// package reaches with no compression at all.
pub const MAX_SIZE: Count<Octets> = Count::<Octets>::new(64 * 1024 * 1024);

/// The request that reads the bytes of `file` as `package`, for the Office
/// module on the converter thread (`ARCH-097`). `zone` is the session's,
/// which a date with no zone is read in.
///
/// # Errors
///
/// [`InteropError::TooLarge`] past [`MAX_SIZE`], before any request leaves.
pub fn read_request(
    file: &Payload<FileBytes>,
    zone: &TimeZone,
    package: TextPackage,
) -> Result<Conversion, InteropError> {
    let size = file.len();
    if size > MAX_SIZE {
        return Err(InteropError::TooLarge(size));
    }
    let (data, zone) = (file.clone(), zone.clone());
    Ok(match package {
        TextPackage::Word => Conversion::ReadDocx { data, zone },
        TextPackage::OpenDocument => Conversion::ReadOdt { data, zone },
    })
}

/// The form the module answered a [`read_request`] of `package`, with the
/// report of its mapping.
///
/// # Errors
///
/// [`InteropError::ParseFailed`] for a package the reader refuses, with its
/// kind. A module that stops or answers out of turn fails as
/// [`InteropError::Stopped`].
pub fn read_answer(
    answer: Result<Converted, ConvertError>,
    package: TextPackage,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    super::read_form(answer, package.file_format(), InteropError::ParseFailed)
}

/// The request that writes `ir` as `package`, each date's wall time in
/// `zone`, for the Office module on the converter thread.
///
/// # Errors
///
/// [`InteropError::Stopped`] when the form does not encode.
pub fn write_request(
    ir: &ProseIr,
    zone: &TimeZone,
    package: TextPackage,
) -> Result<Conversion, InteropError> {
    let form = ambre_prose_form::encode(ir).map_err(|e| super::form_unreadable(&e))?;
    let zone = zone.clone();
    Ok(match package {
        TextPackage::Word => Conversion::WriteDocx { form, zone },
        TextPackage::OpenDocument => Conversion::WriteOdt { form, zone },
    })
}

/// The package the module answered a [`write_request`] of `package`, and the
/// report of what the format could not hold (`PROSE-IO-014`,
/// `PROSE-IO-006`).
///
/// # Errors
///
/// [`InteropError::Stopped`] when the module stops, answers out of turn, or
/// refuses the form.
pub fn write_answer(
    answer: Result<Converted, ConvertError>,
    package: TextPackage,
) -> Result<(Payload<FileBytes>, ConversionReport), InteropError> {
    super::written_file(answer, package.file_format())
}
