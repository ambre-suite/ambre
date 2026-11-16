//! A Markdown import and export, through the markup converter module
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The module parses the text and maps it onto the intermediate form, and
//! writes the form back as text. The parser and the Markdown mapping both run
//! there, so the Main instance links neither (`ambre-spec-budgets.md section
//! 5`). This half bounds the file and checks it is text. It builds the
//! request the converter thread runs, and carries the report of the mapping
//! onto Prose's own. It emits no command and sees no document (`PROSE-IO-009`).
//! A file that is not text, or is too large, fails as a typed error and
//! yields no form at all (`PROSE-IO-010`).

use ambre_bus::interop::FileFormat;
#[cfg(test)]
use ambre_convert::Converter;
use ambre_convert::{Conversion, ConvertError, Converted};
use ambre_client_core::FileBytes;
use ambre_core::{Count, Octets, Payload};

use super::ProseIr;
use crate::message::{ConversionReport, InteropError, MarkdownFlavor};

/// The largest file a Markdown import reads: 3.5 MiB.
///
/// An import holds about 17 times its file at its peak, most of it the
/// document it writes. The markup module keeps about 7 times the file in its
/// own memory after the read. This is the largest round size whose import
/// stays inside the global memory target with both (the `Prose import,
/// transient` row of `ambre-spec-budgets.md section 6`). A file past it fails
/// as [`InteropError::TooLarge`] (`PROSE-IO-102`).
pub const MAX_SIZE: Count<Octets> = Count::<Octets>::new(7 * 512 * 1024);

/// The request that reads the bytes of `file` as Markdown, for the markup
/// module on the converter thread (`ARCH-097`).
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
    Ok(Conversion::ReadMarkdown {
        text: text.to_owned(),
    })
}

/// The form the module answered a [`read_request`], with the report of its
/// mapping.
///
/// # Errors
///
/// Every string of text is some Markdown document, so the parser refuses
/// nothing. A module that stops or answers out of turn fails as
/// [`InteropError::Stopped`].
pub fn read_answer(
    answer: Result<Converted, ConvertError>,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    super::read_form(answer, FileFormat::Markdown, super::refusal_stops)
}

/// The request that writes `ir` as Markdown in `flavor`, for the markup
/// module on the converter thread.
///
/// # Errors
///
/// [`InteropError::Stopped`] when the form does not encode.
pub fn write_request(ir: &ProseIr, flavor: MarkdownFlavor) -> Result<Conversion, InteropError> {
    let form = ambre_prose_form::encode(ir).map_err(|e| super::form_unreadable(&e))?;
    Ok(Conversion::WriteMarkdown { form, flavor })
}

/// The text the module answered a [`write_request`], and the report of what
/// the flavour could not hold (`PROSE-IO-018`).
///
/// # Errors
///
/// [`InteropError::Stopped`] when the module stops, answers out of turn, or
/// refuses the form.
pub fn write_answer(
    answer: Result<Converted, ConvertError>,
) -> Result<(Payload<FileBytes>, ConversionReport), InteropError> {
    super::written_file(answer, FileFormat::Markdown)
}

/// [`read_request`] and [`read_answer`] through `converter` in place, as a
/// test runs the module.
#[cfg(test)]
pub(crate) fn read(
    file: &Payload<FileBytes>,
    converter: &Converter,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    read_answer(converter.convert(&read_request(file)?))
}

/// [`write_request`] and [`write_answer`] through `converter` in place.
#[cfg(test)]
pub(crate) fn write(
    ir: &ProseIr,
    flavor: MarkdownFlavor,
    converter: &Converter,
) -> Result<(Payload<FileBytes>, ConversionReport), InteropError> {
    write_answer(converter.convert(&write_request(ir, flavor)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_bytes(bytes: &[u8]) -> Result<(ProseIr, ConversionReport), InteropError> {
        read(
            &Payload::<FileBytes>::new(bytes.to_vec()),
            &crate::logic::conversion::host_converter(),
        )
    }

    /// `PROSE-IO-010`: bytes that are not text fail typed, and nothing is
    /// read.
    #[test]
    fn bytes_that_are_not_text_fail_typed() {
        assert!(matches!(
            read_bytes(&[0xff, 0xfe, 0x00]),
            Err(InteropError::ParseFailed(_))
        ));
    }

    /// `PROSE-IO-102`: a file one byte past the bound fails as `TooLarge`
    /// before the module reads it, and a file at the bound is read.
    #[test]
    fn a_file_past_the_memory_bound_fails_typed() {
        let bound = MAX_SIZE.saturating_len().to_usize();
        let past = read_bytes(&vec![b'a'; bound + 1]);
        assert!(
            matches!(past, Err(InteropError::TooLarge(size)) if size > MAX_SIZE),
            "{past:?}"
        );
        assert!(read_bytes(&vec![b'a'; bound]).is_ok());
    }

    /// A text crosses into the module and back as the form, and the form
    /// back out as the same text.
    #[test]
    fn a_text_reads_and_writes_back_through_the_module() {
        let (ir, report) = read_bytes(b"# Title\n\nSome **bold**.\n").expect("reads");
        assert!(report.entries().is_empty(), "{report:?}");
        let (file, _) = write(
            &ir,
            MarkdownFlavor::Gfm,
            &crate::logic::conversion::host_converter(),
        )
        .expect("writes");
        assert_eq!(file.as_bytes(), b"# Title\n\nSome **bold**.\n");
    }
}
