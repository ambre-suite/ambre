//! A Markdown import and export, through the markup converter module
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The module parses the text and maps it onto the intermediate form, and
//! writes the form back as text. The parser and the Markdown mapping both run
//! there, so the Main instance links neither (`ambre-spec-budgets.md section
//! 5`). This half bounds the file, checks it is text, hands the bytes over,
//! and carries the report of the mapping onto Prose's own. It emits no
//! command and sees no document (`PROSE-IO-009`). A file that is not text, or
//! is too large, fails as a typed error and yields no form at all
//! (`PROSE-IO-010`).

use ambre_bus::interop::{Direction, FileFormat};
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::{Count, FileBytes, Octets, Payload};
use ambre_prose_form::FormReport;

use super::{ProseIr, Warnings};
use crate::message::{ConversionReport, InteropError, MarkdownFlavor};

/// The largest file a Markdown import reads: 2 MiB.
///
/// An import holds about 45 times its file until it ends. This is the largest
/// file whose import stays inside the global memory target (the `Prose
/// import, transient` row of `ambre-spec-budgets.md section 6`). A
/// file past it fails as [`InteropError::TooLarge`] (`PROSE-IO-102`).
pub const MAX_SIZE: Count<Octets> = Count::<Octets>::new(2 * 1024 * 1024);

/// Read the bytes of `file` as Markdown, through the markup module
/// `converter` holds.
///
/// # Errors
///
/// [`InteropError::TooLarge`] past [`MAX_SIZE`], and
/// [`InteropError::ParseFailed`] for bytes that are not UTF-8. Every string of
/// text is some Markdown document, so the parser refuses nothing. A module
/// that stops or answers out of turn fails as [`InteropError::Stopped`].
pub fn read(
    file: &Payload<FileBytes>,
    converter: &Converter,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    let size = file.size();
    if size > MAX_SIZE {
        return Err(InteropError::TooLarge(size));
    }
    let text = std::str::from_utf8(file.as_bytes())
        .map_err(|e| InteropError::ParseFailed(format!("the file is not UTF-8 text: {e}")))?;
    let answer = converter.convert(&Conversion::ReadMarkdown {
        text: text.to_owned(),
    });
    match answer {
        Ok(Converted::Form(bytes)) => {
            let (ir, report): (ProseIr, FormReport) = ambre_prose_form::decode(&bytes)
                .map_err(|e| InteropError::Stopped(e.to_string()))?;
            let mut warnings = Warnings::new(Direction::Import, FileFormat::Markdown);
            warnings.form(&report);
            Ok((ir, warnings.report()))
        }
        Ok(_) => Err(InteropError::Stopped(ConvertError::Malformed.to_string())),
        Err(e) => Err(InteropError::Stopped(e.to_string())),
    }
}

/// The text of `ir` as Markdown in `flavor`, through the markup module, and
/// the report of what the flavour could not hold (`PROSE-IO-018`).
///
/// # Errors
///
/// [`InteropError::Stopped`] when the module stops, answers out of turn, or
/// refuses the form.
pub fn write(
    ir: &ProseIr,
    flavor: MarkdownFlavor,
    converter: &Converter,
) -> Result<(Payload<FileBytes>, ConversionReport), InteropError> {
    let stopped = |e: String| InteropError::Stopped(e);
    let form = ambre_prose_form::encode(ir).map_err(|e| stopped(e.to_string()))?;
    match converter.convert(&Conversion::WriteMarkdown { form, flavor }) {
        Ok(Converted::Written { file, report }) => {
            let report: FormReport =
                ambre_prose_form::decode(&report).map_err(|e| stopped(e.to_string()))?;
            let mut warnings = Warnings::new(Direction::Export, FileFormat::Markdown);
            warnings.form(&report);
            Ok((file, warnings.report()))
        }
        Ok(_) => Err(stopped(ConvertError::Malformed.to_string())),
        Err(e) => Err(stopped(e.to_string())),
    }
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
