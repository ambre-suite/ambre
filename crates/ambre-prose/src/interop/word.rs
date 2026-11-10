//! A Word import and export, through the OOXML converter module
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The module reads a package into the intermediate form, and writes the form
//! back as a package: the Word codec and the Word mapping both run there, so
//! the Main instance links neither (`ambre-spec-budgets.md section 5`). This
//! half hands the bytes over and carries the report of the mapping onto
//! Prose's own. It emits no command and sees no document (`PROSE-IO-009`).

use ambre_bus::interop::{Direction, FileFormat};
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::{Count, FileBytes, Octets, Payload};
use ambre_prose_form::{FormReport, ZoneOffset};

use super::{ProseIr, Warnings};
use crate::message::{ConversionReport, InteropError};

/// The largest file a Word import hands the module. It is the bound of what
/// the package layer inflates (`ambre_opc::MAX_INFLATED`), which a stored
/// package reaches with no compression at all.
pub const MAX_SIZE: Count<Octets> = Count::<Octets>::new(64 * 1024 * 1024);

/// Read the bytes of `file` as a Word package, through the OOXML module
/// `converter` holds.
///
/// # Errors
///
/// [`InteropError::TooLarge`] past [`MAX_SIZE`], and
/// [`InteropError::ParseFailed`] for a package the reader refuses, in its
/// own words. A module that stops or answers out of turn fails as
/// [`InteropError::Stopped`].
pub fn read(
    file: &Payload<FileBytes>,
    zone: ZoneOffset,
    converter: &Converter,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    let size = file.size();
    if size > MAX_SIZE {
        return Err(InteropError::TooLarge(size));
    }
    match converter.convert(&Conversion::ReadDocx {
        data: file.clone(),
        zone,
    }) {
        Ok(Converted::Form(bytes)) => {
            let (ir, report): (ProseIr, FormReport) = ambre_prose_form::decode(&bytes)
                .map_err(|e| InteropError::Stopped(e.to_string()))?;
            let mut warnings = Warnings::new(Direction::Import, FileFormat::Docx);
            warnings.form(&report);
            Ok((ir, warnings.report()))
        }
        Ok(_) => Err(InteropError::Stopped(ConvertError::Malformed.to_string())),
        Err(ConvertError::Refused(reason)) => Err(InteropError::ParseFailed(reason)),
        Err(e) => Err(InteropError::Stopped(e.to_string())),
    }
}

/// The bytes of `ir` as a Word package, through the OOXML module, and the
/// report of what Word could not hold (`PROSE-IO-014`, `PROSE-IO-006`).
///
/// # Errors
///
/// [`InteropError::Stopped`] when the module stops, answers out of turn, or
/// refuses the form.
pub fn write(
    ir: &ProseIr,
    zone: ZoneOffset,
    converter: &Converter,
) -> Result<(Payload<FileBytes>, ConversionReport), InteropError> {
    let stopped = |e: String| InteropError::Stopped(e);
    let form = ambre_prose_form::encode(ir).map_err(|e| stopped(e.to_string()))?;
    match converter.convert(&Conversion::WriteDocx { form, zone }) {
        Ok(Converted::Written { file, report }) => {
            let report: FormReport =
                ambre_prose_form::decode(&report).map_err(|e| stopped(e.to_string()))?;
            let mut warnings = Warnings::new(Direction::Export, FileFormat::Docx);
            warnings.form(&report);
            Ok((file, warnings.report()))
        }
        Ok(_) => Err(stopped(ConvertError::Malformed.to_string())),
        Err(e) => Err(stopped(e.to_string())),
    }
}
