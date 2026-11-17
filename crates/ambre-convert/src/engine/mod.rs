//! The converters themselves, one Cargo feature per family. A module build
//! turns one family on, the host turns every family on, and the Main
//! instance none (`ARCH-078`).
//!
//! [`run`] is the one entry. It reads a [`Conversion`] from postcard bytes and
//! writes an [`Answer`] back. The modules and the host thus share one path
//! from the bytes in to the bytes out. A conversion of a family this build
//! does not carry answers a refusal that names the family.

#[cfg(feature = "tables")]
use ambre_client_core::FileBytes;
#[cfg(not(feature = "tables"))]
use ambre_core::Payload;
#[cfg(feature = "tables")]
use ambre_core::{Count, Payload};

#[cfg(any(
    feature = "tables",
    feature = "pdf",
    feature = "markup",
    feature = "images",
    feature = "office",
    feature = "boards",
    feature = "spell_en",
    feature = "spell_fr"
))]
use crate::Converted;
use crate::{Answer, Conversion, EncodedAnswer, EncodedRequest};
#[cfg(feature = "tables")]
use crate::{CsvCut, CsvDelimiter, CsvRecords, CsvRoom, FirstRecord};
use ambre_core::FileRefusal;

#[cfg(feature = "boards")]
mod board;
#[cfg(feature = "markup")]
mod html;
#[cfg(feature = "markup")]
mod html_sink;
#[cfg(feature = "boards")]
mod json;
#[cfg(feature = "markup")]
mod markdown;
#[cfg(any(feature = "images", feature = "office"))]
mod picture;
#[cfg(feature = "pdf")]
mod png;
#[cfg(feature = "markup")]
mod prose;
#[cfg(any(feature = "office", feature = "pdf", feature = "images"))]
mod refusal;
#[cfg(any(feature = "spell_en", feature = "spell_fr"))]
mod spell;
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
        Err(_) => Err(FileRefusal::Request),
    };
    Payload::new(postcard::to_allocvec(&answer).unwrap_or_default())
}

/// Run one decoded conversion.
///
/// # Errors
///
/// The converter's refusal, as its kind. A reader cannot read its file,
/// a writer cannot assemble its file, or this build lacks the family.
pub fn convert(conversion: Conversion) -> Answer {
    match conversion {
        #[cfg(feature = "tables")]
        Conversion::ReadCsv {
            data,
            delimiter,
            room,
        } => Ok(read_csv(&data, delimiter, room)),
        #[cfg(feature = "tables")]
        Conversion::WriteCsv { records, delimiter } => write_csv(&records, delimiter),
        #[cfg(feature = "pdf")]
        Conversion::WritePdf { document } => ambre_pdf::write_document(&document)
            .map(Converted::File)
            .map_err(|e| refusal::pdf(&e)),
        #[cfg(feature = "pdf")]
        Conversion::WritePng { document, size } => png::write(&document, size).map(Converted::File),
        #[cfg(feature = "markup")]
        Conversion::ReadMarkdown { text } => ambre_prose_form::encode(&prose::md_read::read(&text))
            .map(Converted::Form)
            .map_err(|_| FileRefusal::Unwritable),
        #[cfg(feature = "markup")]
        Conversion::WriteMarkdown { form, flavor } => {
            let ir: ambre_prose_form::ProseIr =
                ambre_prose_form::decode(&form).map_err(|_| FileRefusal::Request)?;
            let (text, report) = prose::md_write::write(&ir, flavor);
            let report = ambre_prose_form::encode(&report).map_err(|_| FileRefusal::Unwritable)?;
            Ok(Converted::Written {
                file: Payload::new(text.into_bytes()),
                report,
            })
        }
        #[cfg(feature = "markup")]
        Conversion::ReadHtml { html } => Ok(Converted::Pasted(prose::html_read::read(
            &html::read(&html),
        ))),
        // The same walk as a paste, so an import and a paste of one page read
        // the same blocks. A file nested past the bound is refused, where a
        // paste reads as nothing.
        #[cfg(feature = "markup")]
        Conversion::ReadHtmlDocument { html } => {
            let tree = html::read_bounded(&html).ok_or(FileRefusal::PastBound)?;
            let read = prose::html_doc::read(prose::html_read::read_document(&tree));
            ambre_prose_form::encode(&read)
                .map(Converted::Form)
                .map_err(|_| FileRefusal::Unwritable)
        }
        #[cfg(feature = "markup")]
        Conversion::WriteHtml { form } => {
            let ir: ambre_prose_form::ProseIr =
                ambre_prose_form::decode(&form).map_err(|_| FileRefusal::Request)?;
            let (text, report) = prose::html_write::write(&ir);
            let report = ambre_prose_form::encode(&report).map_err(|_| FileRefusal::Unwritable)?;
            Ok(Converted::Written {
                file: Payload::new(text.into_bytes()),
                report,
            })
        }
        #[cfg(feature = "images")]
        Conversion::ReadSvg { data } => svg::read(&data).map(Converted::Svg),
        #[cfg(feature = "boards")]
        Conversion::ReadBoard { data } => board::read(&data).and_then(|tree| {
            crate::boards::encode_board(&tree)
                .map(Converted::Board)
                .map_err(|_| FileRefusal::Unwritable)
        }),
        #[cfg(feature = "images")]
        Conversion::DecodeWebp { data } => webp::decode(&data).map(Converted::Pixels),
        #[cfg(feature = "images")]
        Conversion::DecodeJpeg(request) => picture::jpeg::decode(&request)
            .map(Converted::Pixels)
            .map_err(refusal::jpeg),
        #[cfg(feature = "images")]
        Conversion::DecodePng(request) => picture::raster::decode_png(&request)
            .map(Converted::Pixels)
            .map_err(refusal::raster),
        #[cfg(feature = "images")]
        Conversion::DecodeGif(request) => picture::raster::decode_gif(&request)
            .map(Converted::Frames)
            .map_err(refusal::raster),
        // The Word and the OpenDocument readers answer one model and one
        // report, and both reach Prose through the one Word mapping.
        #[cfg(feature = "office")]
        Conversion::ReadDocx { data, zone } => {
            let decoded = ambre_docx::read(data.as_bytes()).map_err(refusal::docx)?;
            read_text(&decoded.document, &decoded.report, &zone)
        }
        #[cfg(feature = "office")]
        Conversion::ReadOdt { data, zone } => {
            let decoded = ambre_odt::read(data.as_bytes()).map_err(refusal::odt)?;
            read_text(&decoded.document, &decoded.report, &zone)
        }
        #[cfg(feature = "office")]
        Conversion::WriteDocx { form, zone } => write_text(&form, &zone, |document| {
            ambre_docx::write(document)
                .map(Payload::into_role)
                .map_err(refusal::docx)
        }),
        #[cfg(feature = "office")]
        Conversion::WriteOdt { form, zone } => write_text(&form, &zone, |document| {
            ambre_odt::write(document).map_err(refusal::odt)
        }),
        #[cfg(feature = "office")]
        Conversion::ReadXlsx { data } => read_workbook(ambre_xlsx::read(&data)),
        #[cfg(feature = "office")]
        Conversion::WriteXlsx { workbook } => write_workbook(&workbook, ambre_xlsx::write),
        // The OpenDocument reader answers the model and the report the xlsx
        // reader answers, and both reach Trame through one map.
        #[cfg(feature = "office")]
        Conversion::ReadOds { data } => read_workbook(ambre_xlsx::read_ods(&data)),
        #[cfg(feature = "office")]
        Conversion::WriteOds { workbook } => write_workbook(&workbook, ambre_xlsx::write_ods),
        #[cfg(feature = "office")]
        Conversion::ReadPptx { data, bound } => read_deck(
            ambre_pptx::read(data.as_bytes()).map_err(|e| refusal::pptx(&e))?,
            bound,
        ),
        // The OpenDocument reader answers the deck and the report the
        // PowerPoint reader answers, and both reach Toile through one map.
        #[cfg(feature = "office")]
        Conversion::ReadOdp { data, bound } => read_deck(
            ambre_pptx::read_odp(data.as_bytes()).map_err(|e| refusal::pptx(&e))?,
            bound,
        ),
        #[cfg(feature = "office")]
        Conversion::WritePptx { deck } => {
            let deck = crate::decode_deck(&deck).map_err(|_| FileRefusal::Request)?;
            ambre_pptx::write(&deck)
                .map(Converted::File)
                .map_err(|e| refusal::pptx(&e))
        }
        #[cfg(any(feature = "spell_en", feature = "spell_fr"))]
        Conversion::CheckSpelling(check) => spell::check(&check).map(Converted::Misspelled),
        #[cfg(any(feature = "spell_en", feature = "spell_fr"))]
        Conversion::SuggestSpelling(suggest) => {
            spell::suggest(&suggest).map(Converted::Suggestions)
        }
        #[cfg(not(all(
            feature = "tables",
            feature = "pdf",
            feature = "markup",
            feature = "images",
            feature = "office",
            feature = "boards",
            feature = "spell_en",
            feature = "spell_fr"
        )))]
        _ => Err(FileRefusal::NotCarried),
    }
}

#[cfg(feature = "office")]
/// The bytes of the deck a presentation reader answered, and the thumbnail
/// of each JPEG picture it holds, each decoded within `bound`. The Main
/// instance links no JPEG decoder, so it decodes here (`ARCH-078`).
fn read_deck(
    read: ambre_pptx::model::ImportedDeck,
    bound: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
) -> Answer {
    use ambre_pptx::model::{Item, PictureFormat};
    let thumbs = read
        .deck
        .slides
        .iter()
        .flat_map(|slide| &slide.items)
        .filter_map(|item| match item {
            Item::Picture(picture) => Some(picture),
            _ => None,
        })
        .map(|picture| {
            if picture.format != PictureFormat::Jpeg {
                return None;
            }
            let data: Payload<ambre_client_core::FileBytes> = picture.bytes.clone().into_role();
            let stored = crate::jpeg::stored_size(&data)?;
            picture::jpeg::decode(&crate::jpeg::JpegDecode {
                data,
                bound,
                size: crate::images::thumb_size(stored),
                turn: crate::jpeg::JpegTurn::Stored,
            })
            .ok()
        })
        .collect();
    let deck = postcard::to_allocvec(&read)
        .map(Payload::new)
        .map_err(|_| FileRefusal::Unwritable)?;
    Ok(Converted::Deck { deck, thumbs })
}

#[cfg(feature = "office")]
/// Trame's form of the workbook a spreadsheet reader answered, and its
/// report.
fn read_workbook(read: Result<ambre_xlsx::Imported, ambre_xlsx::XlsxError>) -> Answer {
    let mut read = read.map_err(|e| refusal::xlsx(&e))?;
    let book = ambre_xlsx::book::book_of(&read.workbook, &mut read.report);
    crate::encode_workbook(&(book, read.report))
        .map(Converted::Workbook)
        .map_err(|_| FileRefusal::Unwritable)
}

#[cfg(feature = "office")]
/// The file `write` makes of the workbook `workbook` holds, and the report of
/// the map and the writer together.
fn write_workbook(
    workbook: &Payload<crate::WorkbookBytes>,
    write: fn(&ambre_xlsx::Workbook) -> Result<ambre_xlsx::Exported, ambre_xlsx::XlsxError>,
) -> Answer {
    let book: ambre_xlsx::book::Book =
        crate::decode_workbook(workbook).map_err(|_| FileRefusal::Request)?;
    let mut report = ambre_xlsx::Report::default();
    let model = ambre_xlsx::book::workbook_of(&book, &mut report);
    let written = write(&model).map_err(|e| refusal::xlsx(&e))?;
    for entry in written.report.entries() {
        report.record_count(entry.construct, entry.outcome, entry.count);
    }
    let report = crate::encode_workbook(&report).map_err(|_| FileRefusal::Unwritable)?;
    Ok(Converted::WrittenWorkbook {
        file: written.bytes,
        report,
    })
}

/// The form a text document's model maps onto, and the report of the
/// reader and of the mapping, as postcard bytes.
#[cfg(feature = "office")]
fn read_text(
    document: &ambre_docx::WordDocument,
    report: &ambre_docx::WordReport,
    zone: &ambre_prose_form::TimeZone,
) -> Answer {
    let zone = ambre_prose_form::zone::Zone::resolve(zone).map_err(|_| FileRefusal::UnknownZone)?;
    let read = ambre_prose_form::word_read::read_document(document, report, &zone);
    ambre_prose_form::encode(&read)
        .map(Converted::Form)
        .map_err(|_| FileRefusal::Unwritable)
}

/// The file `encode` writes from the model the form maps onto, and the
/// report of the mapping.
#[cfg(feature = "office")]
fn write_text(
    form: &Payload<crate::FormBytes>,
    zone: &ambre_prose_form::TimeZone,
    encode: impl FnOnce(
        &ambre_docx::WordDocument,
    ) -> Result<Payload<ambre_client_core::FileBytes>, FileRefusal>,
) -> Answer {
    let zone = ambre_prose_form::zone::Zone::resolve(zone).map_err(|_| FileRefusal::UnknownZone)?;
    let ir: ambre_prose_form::ProseIr =
        ambre_prose_form::decode(form).map_err(|_| FileRefusal::Request)?;
    let (document, report) = ambre_prose_form::word_write::write(&ir, &zone);
    let file = encode(&document)?;
    let report = ambre_prose_form::encode(&report).map_err(|_| FileRefusal::Unwritable)?;
    Ok(Converted::Written { file, report })
}

#[cfg(feature = "tables")]
/// The records of a CSV file that fit `room`. Records may hold different
/// field counts, as real exports do. An unreadable record is counted and
/// skipped.
///
/// One record buffer serves every record, and the kept fields land in one
/// flat text. So the read allocates nothing per record, and a record past
/// the room costs its parse alone.
fn read_csv(data: &Payload<FileBytes>, delimiter: CsvDelimiter, room: CsvRoom) -> Converted {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter.byte())
        // The room drops a header itself, so every record reads alike.
        .has_headers(false)
        .flexible(true)
        .from_reader(data.as_bytes());
    let mut record = csv::StringRecord::new();
    let mut records = CsvRecords::default();
    let mut first = room.first;
    let mut unreadable = 0usize;
    let mut cut_records = 0usize;
    let mut cut_fields = 0usize;
    let room_fields = room.fields.to_usize();
    loop {
        let before = reader.position().byte();
        match reader.read_record(&mut record) {
            Ok(false) => break,
            Ok(true) => {}
            Err(_) => {
                unreadable += 1;
                // A reader over a slice always moves past a bad record. One
                // that did not would answer the same error for ever.
                if reader.position().byte() == before {
                    break;
                }
                continue;
            }
        }
        if first == FirstRecord::Dropped {
            first = FirstRecord::Kept;
            continue;
        }
        if records.len() >= room.records {
            cut_records += 1;
            continue;
        }
        let kept = record.len().min(room_fields);
        cut_fields += record.len() - kept;
        if records.push(record.iter().take(kept)).is_err() {
            cut_records += 1;
        }
    }
    Converted::Records {
        records,
        unreadable: Count::of(unreadable),
        cut: CsvCut {
            records: Count::of(cut_records),
            fields: Count::of(cut_fields),
        },
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
        writer
            .write_record(record)
            .map_err(|_| FileRefusal::Unwritable)?;
    }
    writer
        .into_inner()
        .map(|bytes| Converted::File(Payload::new(bytes)))
        .map_err(|_| FileRefusal::Unwritable)
}

#[cfg(all(test, feature = "tables", feature = "pdf"))]
mod tests {
    use super::*;
    use crate::{decode_answer, encode_request};

    fn round_trip(conversion: &Conversion) -> Result<Converted, crate::ConvertError> {
        decode_answer(&run(&encode_request(conversion)?))
    }

    /// Room for every record and every field of a small file.
    const ALL: CsvRoom = CsvRoom {
        first: FirstRecord::Kept,
        records: ambre_core::index::Len::new(100),
        fields: ambre_core::index::Len::new(100),
    };

    /// The fields of each record of `records`, owned.
    fn owned(records: &CsvRecords) -> Vec<Vec<String>> {
        records
            .iter()
            .map(|record| record.fields().map(str::to_owned).collect())
            .collect()
    }

    /// The records a read of `data` answers, owned, its unreadable count and
    /// its cut. `None` when the answer holds no records.
    fn read(
        data: &[u8],
        delimiter: CsvDelimiter,
        room: CsvRoom,
    ) -> Option<(Vec<Vec<String>>, u32, CsvCut)> {
        let answer = round_trip(&Conversion::ReadCsv {
            data: Payload::new(data.to_vec()),
            delimiter,
            room,
        });
        let Ok(Converted::Records {
            records,
            unreadable,
            cut,
        }) = answer
        else {
            return None;
        };
        Some((owned(&records), unreadable.get(), cut))
    }

    #[test]
    fn a_csv_file_reads_back_as_its_records() {
        let (records, unreadable, cut) = read(b"a;\"b;c\"\n1;2;3\n", CsvDelimiter::SEMICOLON, ALL)
            .expect("a read answers records");
        assert_eq!(records, vec![vec!["a", "b;c"], vec!["1", "2", "3"]]);
        assert_eq!((unreadable, cut), (0, CsvCut::default()));
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
        let (back, unreadable, _) =
            read(bytes.as_bytes(), CsvDelimiter::COMMA, ALL).expect("a read answers records");
        assert_eq!((back, unreadable), (records, 0));
    }

    #[test]
    fn invalid_utf8_counts_as_unreadable() {
        let (records, unreadable, _) = read(b"ok\n\xff\xfe\nfine\n", CsvDelimiter::COMMA, ALL)
            .expect("a read answers records");
        assert_eq!(unreadable, 1);
        assert_eq!(records.len(), 2);
    }

    /// The read drops the header and keeps its room. It
    /// counts the readable records past the last row, and the fields of
    /// kept records past the last column. A header's own fields count for
    /// nothing.
    #[test]
    fn a_read_keeps_its_room_and_counts_what_it_cuts() {
        let room = CsvRoom {
            first: FirstRecord::Dropped,
            records: ambre_core::index::Len::new(2),
            fields: ambre_core::index::Len::new(2),
        };
        let (records, unreadable, cut) = read(
            b"h1,h2,h3,h4\na,b,c\nd\n\xff\ne,f,g\nh\n",
            CsvDelimiter::COMMA,
            room,
        )
        .expect("a read answers records");
        assert_eq!(records, vec![vec!["a", "b"], vec!["d"]]);
        assert_eq!(unreadable, 1);
        assert_eq!((cut.records.get(), cut.fields.get()), (2, 1));
    }

    #[test]
    fn an_empty_document_is_refused_as_empty() {
        let answer = round_trip(&Conversion::WritePdf {
            document: ambre_draw::list::PrintDocument::default(),
        });
        assert_eq!(
            answer,
            Err(crate::ConvertError::Refused(FileRefusal::Empty))
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

#[cfg(all(test, feature = "office"))]
mod office_tests {
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

    /// The form crosses into the module as an OpenDocument text too, and
    /// reads back through the same module as the form it was.
    #[test]
    fn a_form_written_as_an_odt_by_the_module_reads_back_through_it() {
        let ir = ProseIr {
            blocks: vec![IrBlock::Paragraph {
                styled: OwnStyle::Plain,
                runs: vec![IrRun::plain("hello")],
            }],
            ..ProseIr::default()
        };
        let form = ambre_prose_form::encode(&ir).expect("the form encodes");
        let Ok(Converted::Written { file, report }) = round_trip(&Conversion::WriteOdt {
            form,
            zone: ambre_prose_form::TimeZone::UTC,
        }) else {
            panic!("a write answers a file");
        };
        let report: FormReport = ambre_prose_form::decode(&report).expect("the report decodes");
        assert!(report.entries().is_empty());
        let Ok(Converted::Form(bytes)) = round_trip(&Conversion::ReadOdt {
            data: file,
            zone: ambre_prose_form::TimeZone::UTC,
        }) else {
            panic!("a read answers a form");
        };
        let (back, _): (ProseIr, FormReport) =
            ambre_prose_form::decode(&bytes).expect("the form decodes");
        assert_eq!(back, ir);
    }

    /// A file that is no package answers the reader's refusal as its kind.
    #[test]
    fn noise_is_refused_as_no_package() {
        assert_eq!(
            round_trip(&Conversion::ReadDocx {
                data: Payload::new(b"not a package".to_vec()),
                zone: ambre_prose_form::TimeZone::UTC,
            }),
            Err(ConvertError::Refused(FileRefusal::NotAPackage))
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
            frozen: ambre_xlsx::model::FrozenPane::NONE,
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
                    frozen: ambre_xlsx::model::FrozenPane::NONE,
                },
            ],
            styles: vec![Payload::new(Vec::new())],
        };
        let Ok(Converted::WrittenWorkbook { file, report }) = round_trip(&Conversion::WriteXlsx {
            workbook: crate::encode_workbook(&book).unwrap(),
        }) else {
            panic!("a write answers a file");
        };
        // The same book leaves as an OpenDocument spreadsheet, and reads back
        // as the same form the xlsx file reads back as.
        let Ok(Converted::WrittenWorkbook { file: ods, .. }) = round_trip(&Conversion::WriteOds {
            workbook: crate::encode_workbook(&book).unwrap(),
        }) else {
            panic!("a write answers a file");
        };
        let Ok(Converted::Workbook(from_ods)) = round_trip(&Conversion::ReadOds { data: ods })
        else {
            panic!("a read answers a workbook");
        };
        let Ok(Converted::Workbook(from_xlsx)) =
            round_trip(&Conversion::ReadXlsx { data: file.clone() })
        else {
            panic!("a read answers a workbook");
        };
        let from_ods: (Book, ambre_xlsx::Report) = crate::decode_workbook(&from_ods).unwrap();
        let from_xlsx: (Book, ambre_xlsx::Report) = crate::decode_workbook(&from_xlsx).unwrap();
        assert_eq!(from_ods.0, from_xlsx.0);
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

    /// An OpenDocument presentation crosses into the module. It comes back
    /// as the deck the PowerPoint reader answers: one slide per page, with
    /// its notes.
    #[test]
    fn an_odp_reads_through_the_module_as_a_deck() {
        use ambre_pptx::opc::PartName;
        use ambre_pptx::opc::odf::{OdfType, OdfWriter};
        let content = concat!(
            r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
            r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "#,
            r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" "#,
            r#"xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" "#,
            r#"xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0">"#,
            r#"<office:body><office:presentation><draw:page draw:name="Intro">"#,
            r#"<draw:frame svg:x="1cm" svg:y="1cm" svg:width="5cm" svg:height="2cm"><draw:text-box><text:p>Hello</text:p></draw:text-box></draw:frame>"#,
            r#"<presentation:notes><draw:frame presentation:class="notes"><draw:text-box><text:p>Say hi.</text:p></draw:text-box></draw:frame></presentation:notes>"#,
            r#"</draw:page></office:presentation></office:body></office:document-content>"#,
        );
        let mut w = OdfWriter::new(OdfType::PRESENTATION);
        w.part(
            PartName::new("content.xml").expect("a part name"),
            OdfType::XML,
            Payload::new(content.as_bytes().to_vec()),
        );
        let file = w.finish().expect("the package writes");
        let Ok(Converted::Deck { deck, thumbs }) = round_trip(&Conversion::ReadOdp {
            data: file.into_role(),
            bound: ambre_core::Size::new(
                ambre_core::Px::<ambre_core::Image>::new(4096),
                ambre_core::Px::<ambre_core::Image>::new(4096),
            ),
        }) else {
            panic!("a read answers a deck");
        };
        let read = crate::decode_imported_deck(&deck).expect("the deck decodes");
        assert_eq!(read.deck.slides.len(), 1);
        assert_eq!(read.deck.slides[0].name, "Intro");
        assert_eq!(read.deck.slides[0].notes, "Say hi.");
        assert_eq!(read.deck.slides[0].items.len(), 1);
        assert!(thumbs.is_empty());
    }
}
