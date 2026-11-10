//! The converters themselves, one Cargo feature per family. A module build
//! turns one family on, the host turns every family on, and the Main
//! instance none (`ARCH-078`).
//!
//! [`run`] is the one entry. It reads a [`Conversion`] from postcard bytes and
//! writes an [`Answer`] back. The modules and the host thus share one path
//! from the bytes in to the bytes out. A conversion of a family this build
//! does not carry answers a refusal that names the family.

#[cfg(not(feature = "tables"))]
use ambre_core::Payload;
#[cfg(feature = "tables")]
use ambre_core::{Count, FileBytes, Payload};

#[cfg(any(
    feature = "tables",
    feature = "pdf",
    feature = "markup",
    feature = "images",
    feature = "ooxml"
))]
use crate::Converted;
#[cfg(feature = "tables")]
use crate::CsvDelimiter;
use crate::{Answer, Conversion, EncodedAnswer, EncodedRequest};

#[cfg(feature = "markup")]
mod html;
#[cfg(feature = "markup")]
mod html_sink;
#[cfg(feature = "markup")]
mod markdown;
#[cfg(feature = "images")]
mod svg;
#[cfg(feature = "images")]
mod webp;

/// Run the conversion `request` encodes, and encode its answer.
///
/// Never fails. A request that does not decode answers a refusal. An answer
/// postcard cannot write leaves the bytes empty, which the loader reads as
/// malformed.
pub fn run(request: &Payload<EncodedRequest>) -> Payload<EncodedAnswer> {
    let answer: Answer = match postcard::from_bytes::<Conversion>(request.as_bytes()) {
        Ok(conversion) => convert(conversion),
        Err(e) => Err(format!("the request could not be read: {e}")),
    };
    Payload::new(postcard::to_allocvec(&answer).unwrap_or_default())
}

/// Run one decoded conversion.
///
/// # Errors
///
/// The converter's refusal, in its own words. A reader cannot read its file,
/// a writer cannot assemble its file, or this build lacks the family.
pub fn convert(conversion: Conversion) -> Answer {
    match conversion {
        #[cfg(feature = "tables")]
        Conversion::ReadCsv { data, delimiter } => Ok(read_csv(&data, delimiter)),
        #[cfg(feature = "tables")]
        Conversion::WriteCsv { records, delimiter } => write_csv(&records, delimiter),
        #[cfg(feature = "pdf")]
        Conversion::WritePdf { document } => ambre_pdf::write_document(&document)
            .map(Converted::File)
            .map_err(|e| e.to_string()),
        #[cfg(feature = "markup")]
        Conversion::ReadMarkdown { text } => Ok(Converted::Markdown(markdown::read(&text))),
        #[cfg(feature = "markup")]
        Conversion::ReadHtml { html } => Ok(Converted::Html(html::read(&html))),
        #[cfg(feature = "images")]
        Conversion::ReadSvg { data } => svg::read(&data).map(Converted::Svg),
        #[cfg(feature = "images")]
        Conversion::DecodeWebp { data } => webp::decode(&data).map(Converted::Pixels),
        #[cfg(feature = "ooxml")]
        Conversion::ReadDocx { data } => {
            let decoded = ambre_docx::read(data.as_bytes()).map_err(|e| e.to_string())?;
            let read =
                ambre_prose_form::word_read::read_document(&decoded.document, &decoded.report);
            ambre_prose_form::encode(&read)
                .map(Converted::Form)
                .map_err(|e| e.to_string())
        }
        #[cfg(feature = "ooxml")]
        Conversion::WriteDocx { form } => {
            let ir: ambre_prose_form::ProseIr =
                ambre_prose_form::decode(&form).map_err(|e| e.to_string())?;
            let (document, report) = ambre_prose_form::word_write::write(&ir);
            let file = ambre_docx::write(&document).map_err(|e| e.to_string())?;
            let report = ambre_prose_form::encode(&report).map_err(|e| e.to_string())?;
            Ok(Converted::Written {
                file: file.into_role(),
                report,
            })
        }
        #[cfg(not(all(
            feature = "tables",
            feature = "pdf",
            feature = "markup",
            feature = "images",
            feature = "ooxml"
        )))]
        other => Err(format!(
            "this module does not carry the {} family",
            other.family()
        )),
    }
}

#[cfg(feature = "tables")]
/// The records of a CSV file. Records may hold different field counts, as
/// real exports do. An unreadable record is counted and skipped.
fn read_csv(data: &Payload<FileBytes>, delimiter: CsvDelimiter) -> Converted {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter.byte())
        // The caller skips a header itself, so every record reads alike.
        .has_headers(false)
        .flexible(true)
        .from_reader(data.as_bytes());
    let mut unreadable = 0usize;
    let records = reader
        .records()
        .filter_map(|rec| rec.inspect_err(|_| unreadable += 1).ok())
        .map(|rec| rec.iter().map(str::to_owned).collect())
        .collect();
    Converted::Records {
        records,
        unreadable: Count::of(unreadable),
    }
}

#[cfg(feature = "tables")]
/// The bytes of `records` written as CSV, quoted where a field needs it.
fn write_csv(records: &[Vec<String>], delimiter: CsvDelimiter) -> Answer {
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter.byte())
        .flexible(true)
        .from_writer(Vec::new());
    for record in records {
        writer.write_record(record).map_err(|e| e.to_string())?;
    }
    writer
        .into_inner()
        .map(|bytes| Converted::File(Payload::new(bytes)))
        .map_err(|e| e.to_string())
}

#[cfg(all(test, feature = "tables", feature = "pdf"))]
mod tests {
    use super::*;
    use crate::{decode_answer, encode_request};

    fn round_trip(conversion: &Conversion) -> Result<Converted, crate::ConvertError> {
        decode_answer(&run(&encode_request(conversion)?))
    }

    #[test]
    fn a_csv_file_reads_back_as_its_records() {
        let answer = round_trip(&Conversion::ReadCsv {
            data: Payload::new(b"a;\"b;c\"\n1;2;3\n".to_vec()),
            delimiter: CsvDelimiter::SEMICOLON,
        });
        assert_eq!(
            answer,
            Ok(Converted::Records {
                records: vec![
                    vec!["a".to_owned(), "b;c".to_owned()],
                    vec!["1".to_owned(), "2".to_owned(), "3".to_owned()],
                ],
                unreadable: Count::ZERO,
            })
        );
    }

    #[test]
    fn written_records_read_back_unchanged() {
        let records = vec![
            vec!["x,y".to_owned(), "\"q\"".to_owned()],
            vec!["1".to_owned()],
        ];
        let Ok(Converted::File(bytes)) = round_trip(&Conversion::WriteCsv {
            records: records.clone(),
            delimiter: CsvDelimiter::COMMA,
        }) else {
            panic!("a write answers a file");
        };
        let back = round_trip(&Conversion::ReadCsv {
            data: bytes,
            delimiter: CsvDelimiter::COMMA,
        });
        assert_eq!(
            back,
            Ok(Converted::Records {
                records,
                unreadable: Count::ZERO,
            })
        );
    }

    #[test]
    fn invalid_utf8_counts_as_unreadable() {
        let answer = round_trip(&Conversion::ReadCsv {
            data: Payload::new(b"ok\n\xff\xfe\nfine\n".to_vec()),
            delimiter: CsvDelimiter::COMMA,
        });
        let Ok(Converted::Records {
            records,
            unreadable,
        }) = answer
        else {
            panic!("a read answers records");
        };
        assert_eq!(unreadable.get(), 1);
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn an_empty_document_is_refused_in_the_writers_words() {
        let answer = round_trip(&Conversion::WritePdf {
            document: ambre_draw::list::PrintDocument::default(),
        });
        assert_eq!(
            answer,
            Err(crate::ConvertError::Refused(
                "no pages to export".to_owned()
            ))
        );
    }

    #[test]
    fn bytes_that_are_no_request_answer_a_refusal() {
        assert!(matches!(
            decode_answer(&run(&Payload::new(vec![0xff; 4]))),
            Err(crate::ConvertError::Refused(_))
        ));
    }
}

#[cfg(all(test, feature = "ooxml"))]
mod ooxml_tests {
    use super::*;
    use crate::{ConvertError, decode_answer, encode_request};
    use ambre_prose_form::{FormReport, IrBlock, IrRun, OwnStyle, ProseIr};

    fn round_trip(conversion: &Conversion) -> Result<Converted, ConvertError> {
        decode_answer(&run(&encode_request(conversion)?))
    }

    /// The form crosses into the module and back as bytes: written as a
    /// Word package, then read back through the same module as the form it
    /// was, with no loss reported.
    #[test]
    fn a_form_written_by_the_module_reads_back_through_it() {
        let ir = ProseIr {
            blocks: vec![IrBlock::Paragraph {
                styled: OwnStyle::Plain,
                runs: vec![IrRun::plain("hello")],
            }],
            ..ProseIr::default()
        };
        let form = ambre_prose_form::encode(&ir).expect("the form encodes");
        let Ok(Converted::Written { file, report }) = round_trip(&Conversion::WriteDocx { form })
        else {
            panic!("a write answers a file");
        };
        let report: FormReport = ambre_prose_form::decode(&report).expect("the report decodes");
        assert!(report.entries().is_empty());
        let Ok(Converted::Form(bytes)) = round_trip(&Conversion::ReadDocx { data: file }) else {
            panic!("a read answers a form");
        };
        let (back, _): (ProseIr, FormReport) =
            ambre_prose_form::decode(&bytes).expect("the form decodes");
        assert_eq!(back, ir);
    }

    /// A file that is no package answers the reader's refusal in its words.
    #[test]
    fn noise_is_refused_in_the_readers_words() {
        assert_eq!(
            round_trip(&Conversion::ReadDocx {
                data: Payload::new(b"not a package".to_vec()),
            }),
            Err(ConvertError::Refused(
                "the file is not a ZIP package".to_owned()
            ))
        );
    }
}
