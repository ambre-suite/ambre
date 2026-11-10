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
#[cfg(feature = "markup")]
mod prose;
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
        Conversion::ReadMarkdown { text } => ambre_prose_form::encode(&prose::md_read::read(&text))
            .map(Converted::Form)
            .map_err(|e| e.to_string()),
        #[cfg(feature = "markup")]
        Conversion::WriteMarkdown { form, flavor } => {
            let ir: ambre_prose_form::ProseIr =
                ambre_prose_form::decode(&form).map_err(|e| e.to_string())?;
            let (text, report) = prose::md_write::write(&ir, flavor);
            let report = ambre_prose_form::encode(&report).map_err(|e| e.to_string())?;
            Ok(Converted::Written {
                file: Payload::new(text.into_bytes()),
                report,
            })
        }
        #[cfg(feature = "markup")]
        Conversion::ReadHtml { html } => Ok(Converted::Pasted(prose::html_read::read(
            &html::read(&html),
        ))),
        #[cfg(feature = "images")]
        Conversion::ReadSvg { data } => svg::read(&data).map(Converted::Svg),
        #[cfg(feature = "images")]
        Conversion::DecodeWebp { data } => webp::decode(&data).map(Converted::Pixels),
        #[cfg(feature = "ooxml")]
        Conversion::ReadDocx { data, zone } => {
            let zone = ambre_prose_form::zone::Zone::resolve(&zone).map_err(|e| e.to_string())?;
            let decoded = ambre_docx::read(data.as_bytes()).map_err(|e| e.to_string())?;
            let read = ambre_prose_form::word_read::read_document(
                &decoded.document,
                &decoded.report,
                &zone,
            );
            ambre_prose_form::encode(&read)
                .map(Converted::Form)
                .map_err(|e| e.to_string())
        }
        #[cfg(feature = "ooxml")]
        Conversion::WriteDocx { form, zone } => {
            let zone = ambre_prose_form::zone::Zone::resolve(&zone).map_err(|e| e.to_string())?;
            let ir: ambre_prose_form::ProseIr =
                ambre_prose_form::decode(&form).map_err(|e| e.to_string())?;
            let (document, report) = ambre_prose_form::word_write::write(&ir, &zone);
            let file = ambre_docx::write(&document).map_err(|e| e.to_string())?;
            let report = ambre_prose_form::encode(&report).map_err(|e| e.to_string())?;
            Ok(Converted::Written {
                file: file.into_role(),
                report,
            })
        }
        #[cfg(feature = "ooxml")]
        Conversion::ReadXlsx { data } => {
            let mut read = ambre_xlsx::read(&data).map_err(|e| e.to_string())?;
            let book = ambre_xlsx::book::book_of(&read.workbook, &mut read.report);
            crate::encode_workbook(&(book, read.report))
                .map(Converted::Workbook)
                .map_err(|e| e.to_string())
        }
        #[cfg(feature = "ooxml")]
        Conversion::WriteXlsx { workbook } => {
            let book: ambre_xlsx::book::Book =
                crate::decode_workbook(&workbook).map_err(|e| e.to_string())?;
            let mut report = ambre_xlsx::Report::default();
            let model = ambre_xlsx::book::workbook_of(&book, &mut report);
            let written = ambre_xlsx::write(&model).map_err(|e| e.to_string())?;
            for entry in written.report.entries() {
                report.record_count(entry.construct, entry.outcome, entry.count);
            }
            let report = crate::encode_workbook(&report).map_err(|e| e.to_string())?;
            Ok(Converted::WrittenWorkbook {
                file: written.bytes,
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

    /// The form crosses into the module and back as bytes. It is written as
    /// a Word package, then read back through the same module as the form it
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
        let Ok(Converted::Written { file, report }) = round_trip(&Conversion::WriteDocx {
            form,
            zone: ambre_prose_form::TimeZone::UTC,
        }) else {
            panic!("a write answers a file");
        };
        let report: FormReport = ambre_prose_form::decode(&report).expect("the report decodes");
        assert!(report.entries().is_empty());
        let Ok(Converted::Form(bytes)) = round_trip(&Conversion::ReadDocx {
            data: file,
            zone: ambre_prose_form::TimeZone::UTC,
        }) else {
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
                zone: ambre_prose_form::TimeZone::UTC,
            }),
            Err(ConvertError::Refused(
                "the file is not a ZIP package".to_owned()
            ))
        );
    }

    /// Trame's form of a workbook crosses into the module as bytes and
    /// leaves as an xlsx file. It reads back through the same module as the
    /// form it was. A Trame formula takes its Excel text on the way.
    #[test]
    fn a_book_written_by_the_module_reads_back_through_it() {
        use ambre_xlsx::book::{Book, BookCell, BookContent, BookSheet};
        use ambre_xlsx::model::{Value, Visibility};
        let at = |t: &str| ambre_xlsx::cellref::CellRef::parse(t).unwrap();
        let shown = BookSheet {
            name: "Shown".to_owned(),
            visibility: Visibility::Visible,
            cells: Vec::new(),
            merges: Vec::new(),
            column_widths: Vec::new(),
            row_heights: Vec::new(),
        };
        let book = Book {
            sheets: vec![
                shown,
                BookSheet {
                    name: "Data: 2026".to_owned(),
                    visibility: Visibility::Hidden,
                    cells: vec![
                        BookCell {
                            at: at("A1"),
                            content: BookContent::Literal("2.5".to_owned()),
                            style: ambre_xlsx::model::StyleId::DEFAULT,
                        },
                        BookCell {
                            at: at("A2"),
                            content: BookContent::Formula {
                                trame: ambre_core::Validated::parse("=IFS(A1>1,A1)".to_owned())
                                    .ok(),
                                excel: None,
                                cached: Value::Number(2.5),
                            },
                            style: ambre_xlsx::model::StyleId::DEFAULT,
                        },
                    ],
                    merges: Vec::new(),
                    column_widths: Vec::new(),
                    row_heights: Vec::new(),
                },
            ],
            styles: vec![Payload::new(Vec::new())],
        };
        let Ok(Converted::WrittenWorkbook { file, report }) = round_trip(&Conversion::WriteXlsx {
            workbook: crate::encode_workbook(&book).unwrap(),
        }) else {
            panic!("a write answers a file");
        };
        let report: ambre_xlsx::Report = crate::decode_workbook(&report).unwrap();
        assert_eq!(
            report.count(
                ambre_xlsx::report::XlsxConstruct::Sheet,
                ambre_xlsx::report::Outcome::Approximated
            ),
            ambre_core::Count::new(1),
            "the name Excel refuses is reported"
        );
        let Ok(Converted::Workbook(bytes)) = round_trip(&Conversion::ReadXlsx { data: file })
        else {
            panic!("a read answers a workbook");
        };
        let (back, _): (Book, ambre_xlsx::Report) = crate::decode_workbook(&bytes).unwrap();
        let sheet = &back.sheets[1];
        assert_eq!(sheet.name, "Data_ 2026");
        assert_eq!(sheet.visibility, Visibility::Hidden);
        assert_eq!(
            sheet.cells[0].content,
            BookContent::Literal("2.5".to_owned())
        );
        assert_eq!(
            sheet.cells[1].content,
            BookContent::Formula {
                trame: ambre_core::Validated::parse("=IFS(A1>1,A1)".to_owned()).ok(),
                excel: ambre_core::Validated::parse("_xlfn.IFS(A1>1,A1)".to_owned()).ok(),
                cached: Value::Number(2.5),
            }
        );
    }
}
