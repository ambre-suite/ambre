//! A Word import and export, through the Office converter module
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The module reads a package into the intermediate form, and writes the form
//! back as a package. The Word codec and the Word mapping both run there, so
//! the Main instance links neither (`ambre-spec-budgets.md section 5`). This
//! half hands the bytes over and carries the report of the mapping onto
//! Prose's own. It emits no command and sees no document (`PROSE-IO-009`).

use ambre_bus::interop::{Direction, FileFormat};
use ambre_client_core::FileBytes;
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::{Count, Octets, Payload};
use ambre_prose_form::{FormReport, TimeZone};

use super::{ProseIr, Warnings};
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

/// Read the bytes of `file` as a Word package, through the Office module
/// `converter` holds.
///
/// # Errors
///
/// [`InteropError::TooLarge`] past [`MAX_SIZE`], and
/// [`InteropError::ParseFailed`] for a package the reader refuses, with its
/// kind. A module that stops or answers out of turn fails as
/// [`InteropError::Stopped`].
pub fn read(
    file: &Payload<FileBytes>,
    zone: &TimeZone,
    converter: &Converter,
    package: TextPackage,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    let size = file.len();
    if size > MAX_SIZE {
        return Err(InteropError::TooLarge(size));
    }
    let (data, zone) = (file.clone(), zone.clone());
    let conversion = match package {
        TextPackage::Word => Conversion::ReadDocx { data, zone },
        TextPackage::OpenDocument => Conversion::ReadOdt { data, zone },
    };
    match converter.convert(&conversion) {
        Ok(Converted::Form(bytes)) => {
            let (ir, report): (ProseIr, FormReport) =
                ambre_prose_form::decode(&bytes).map_err(|e| super::form_unreadable(&e))?;
            let mut warnings = Warnings::new(Direction::Import, package.file_format());
            warnings.form(&report);
            Ok((ir, warnings.report()))
        }
        Ok(_) => Err(super::stopped(&ConvertError::Malformed)),
        Err(ConvertError::Refused(reason)) => Err(InteropError::ParseFailed(reason)),
        Err(e) => Err(super::stopped(&e)),
    }
}

/// The bytes of `ir` as a Word package, through the Office module, and the
/// report of what Word could not hold (`PROSE-IO-014`, `PROSE-IO-006`).
///
/// # Errors
///
/// [`InteropError::Stopped`] when the module stops, answers out of turn, or
/// refuses the form.
pub fn write(
    ir: &ProseIr,
    zone: &TimeZone,
    converter: &Converter,
    package: TextPackage,
) -> Result<(Payload<FileBytes>, ConversionReport), InteropError> {
    let form = ambre_prose_form::encode(ir).map_err(|e| super::form_unreadable(&e))?;
    let zone = zone.clone();
    let conversion = match package {
        TextPackage::Word => Conversion::WriteDocx { form, zone },
        TextPackage::OpenDocument => Conversion::WriteOdt { form, zone },
    };
    match converter.convert(&conversion) {
        Ok(Converted::Written { file, report }) => {
            let report: FormReport =
                ambre_prose_form::decode(&report).map_err(|e| super::form_unreadable(&e))?;
            let mut warnings = Warnings::new(Direction::Export, package.file_format());
            warnings.form(&report);
            Ok((file, warnings.report()))
        }
        Ok(_) => Err(super::stopped(&ConvertError::Malformed)),
        Err(e) => Err(super::stopped(&e)),
    }
}
