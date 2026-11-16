//! An HTML import and export, through the markup converter module
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The module reads a file through the walk a rich paste runs. An import and
//! a paste of one page thus land the same blocks (`PROSE-IO-025`). The walk
//! keeps an allowlist and drops the rest: a script, a stylesheet, a frame and
//! a picture never run or load (`PROSE-IO-016`, `PROSE-IO-062`,
//! `PROSE-IO-023`). This half bounds the file, checks it is text, hands the
//! bytes over, and carries the report onto Prose's own. It emits no command
//! and sees no document (`PROSE-IO-009`).

use ambre_bus::interop::{Direction, FileFormat};
use ambre_client_core::FileBytes;
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::{Count, Octets, Payload};
use ambre_prose_form::FormReport;

use super::{ProseIr, Warnings};
use crate::message::{ConversionReport, InteropError};

/// The largest file an HTML import reads. A saved page holds its markup and
/// its inline styles, so the bound is the Markdown one.
pub const MAX_SIZE: Count<Octets> = super::markdown::MAX_SIZE;

/// Read the bytes of `file` as an HTML document, through the markup module
/// `converter` holds.
///
/// # Errors
///
/// [`InteropError::TooLarge`] past [`MAX_SIZE`], and
/// [`InteropError::ParseFailed`] for bytes that are not UTF-8. The tree
/// builder reads any text, so the module refuses only a document nested past
/// its bound. That refusal, and a module that stops or answers out of turn,
/// fail as [`InteropError::Stopped`].
pub fn read(
    file: &Payload<FileBytes>,
    converter: &Converter,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    let size = file.len();
    if size > MAX_SIZE {
        return Err(InteropError::TooLarge(size));
    }
    let text = std::str::from_utf8(file.as_bytes())
        .map_err(|_| InteropError::ParseFailed(ambre_core::FileRefusal::NotText))?;
    let answer = converter.convert(&Conversion::ReadHtmlDocument {
        html: text.to_owned(),
    });
    match answer {
        Ok(Converted::Form(bytes)) => {
            let (ir, report): (ProseIr, FormReport) =
                ambre_prose_form::decode(&bytes).map_err(|e| super::form_unreadable(&e))?;
            let mut warnings = Warnings::new(Direction::Import, FileFormat::Html);
            warnings.form(&report);
            Ok((ir, warnings.report()))
        }
        Ok(_) => Err(super::stopped(&ConvertError::Malformed)),
        Err(e) => Err(super::stopped(&e)),
    }
}

/// `ir` as a standalone HTML document, through the markup module, and the
/// report of what the format could not hold (`PROSE-IO-019`). The caller
/// drops the review first unless the user asked for it (`PROSE-IO-022`).
///
/// # Errors
///
/// [`InteropError::Stopped`] when the module stops, answers out of turn, or
/// refuses the form.
pub fn write(
    ir: &ProseIr,
    converter: &Converter,
) -> Result<(Payload<FileBytes>, ConversionReport), InteropError> {
    let form = ambre_prose_form::encode(ir).map_err(|e| super::form_unreadable(&e))?;
    match converter.convert(&Conversion::WriteHtml { form }) {
        Ok(Converted::Written { file, report }) => {
            let report: FormReport =
                ambre_prose_form::decode(&report).map_err(|e| super::form_unreadable(&e))?;
            let mut warnings = Warnings::new(Direction::Export, FileFormat::Html);
            warnings.form(&report);
            Ok((file, warnings.report()))
        }
        Ok(_) => Err(super::stopped(&ConvertError::Malformed)),
        Err(e) => Err(super::stopped(&e)),
    }
}
