//! An HTML import and export, through the markup converter module
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The module reads a file through the walk a rich paste runs. An import and
//! a paste of one page thus land the same blocks (`PROSE-IO-025`). The walk
//! keeps an allowlist and drops the rest: a script, a stylesheet, a frame and
//! a picture never run or load (`PROSE-IO-016`, `PROSE-IO-062`,
//! `PROSE-IO-023`). This half bounds the file and checks it is text. It
//! builds the request the converter thread runs, and carries the report of
//! its answer onto Prose's own. It emits no command and sees no document
//! (`PROSE-IO-009`).

use ambre_bus::interop::FileFormat;
use ambre_convert::{Conversion, ConvertError, Converted};
use ambre_client_core::FileBytes;
use ambre_core::{Count, Octets, Payload};

use super::ProseIr;
use crate::message::{ConversionReport, InteropError};

/// The largest file an HTML import reads. A saved page holds its markup and
/// its inline styles, so the bound is the Markdown one.
pub const MAX_SIZE: Count<Octets> = super::markdown::MAX_SIZE;

/// The request that reads the bytes of `file` as an HTML document, for the
/// markup module on the converter thread (`ARCH-097`).
///
/// # Errors
///
/// [`InteropError::TooLarge`] past [`MAX_SIZE`], and
/// [`InteropError::ParseFailed`] for bytes that are not UTF-8. Either fails
/// before any request leaves.
pub fn read_request(file: &Payload<FileBytes>) -> Result<Conversion, InteropError> {
    let size = file.len();
    if size > MAX_SIZE {
        return Err(InteropError::TooLarge(size));
    }
    let text = std::str::from_utf8(file.as_bytes())
        .map_err(|_| InteropError::ParseFailed(ambre_core::FileRefusal::NotText))?;
    Ok(Conversion::ReadHtmlDocument {
        html: text.to_owned(),
    })
}

/// The form the module answered a [`read_request`], with the report of its
/// mapping.
///
/// # Errors
///
/// The tree builder reads any text, so the module refuses only a document
/// nested past its bound. That refusal, and a module that stops or answers
/// out of turn, fail as [`InteropError::Stopped`].
pub fn read_answer(
    answer: Result<Converted, ConvertError>,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    super::read_form(answer, FileFormat::Html, super::refusal_stops)
}

/// The request that writes `ir` as a standalone HTML document, for the
/// markup module on the converter thread. The caller drops the review first
/// unless the user asked for it (`PROSE-IO-022`).
///
/// # Errors
///
/// [`InteropError::Stopped`] when the form does not encode.
pub fn write_request(ir: &ProseIr) -> Result<Conversion, InteropError> {
    let form = ambre_prose_form::encode(ir).map_err(|e| super::form_unreadable(&e))?;
    Ok(Conversion::WriteHtml { form })
}

/// The document the module answered a [`write_request`], and the report of
/// what the format could not hold (`PROSE-IO-019`).
///
/// # Errors
///
/// [`InteropError::Stopped`] when the module stops, answers out of turn, or
/// refuses the form.
pub fn write_answer(
    answer: Result<Converted, ConvertError>,
) -> Result<(Payload<FileBytes>, ConversionReport), InteropError> {
    super::written_file(answer, FileFormat::Html)
}
