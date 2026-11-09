//! The converters themselves, behind the `engine` feature: the module build
//! and the host link them, the Main instance never does (`ARCH-078`).
//!
//! [`run`] is the one entry. It reads a [`Conversion`] from postcard bytes and
//! writes an [`Answer`] back. The module and the host thus share one path from
//! the bytes in to the bytes out.

use ambre_core::{Count, FileBytes, Payload};

use crate::{Answer, Conversion, Converted, CsvDelimiter, EncodedAnswer, EncodedRequest};

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
/// The converter's refusal, in its own words: a writer that cannot assemble
/// its file.
pub fn convert(conversion: Conversion) -> Answer {
    match conversion {
        Conversion::ReadCsv { data, delimiter } => Ok(read_csv(&data, delimiter)),
        Conversion::WriteCsv { records, delimiter } => write_csv(&records, delimiter),
        Conversion::WritePdf { document } => ambre_pdf::write_document(&document)
            .map(Converted::File)
            .map_err(|e| e.to_string()),
    }
}

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

#[cfg(test)]
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
