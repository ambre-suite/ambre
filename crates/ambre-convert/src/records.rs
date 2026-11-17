//! The records a CSV read keeps, laid flat (`trame-import-export`).
//!
//! A list of records of owned fields costs one allocation per field and one
//! per record. A file of one-byte lines at the import bound then held its
//! input many times over. Here the text of every kept field lies end to end
//! in one string, and two lists of offsets cut it. So a read allocates a
//! constant number of buffers, whatever the count of records, and the
//! answer decodes in one pass, on the converter thread.

use ambre_core::index::{Axis, IdxVec, Len, MeasuresAs, Pos, Span};
use ambre_core::{Count, NarrowCount};
use serde::{Deserialize, Serialize};

/// The bytes of the text a CSV read kept, every field end to end.
#[derive(PartialEq, Eq)]
pub enum KeptByte {}

impl Axis for KeptByte {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("KeptByte");
}

/// The fields a CSV read kept, every record end to end.
#[derive(PartialEq, Eq)]
pub enum KeptField {}

impl Axis for KeptField {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("KeptField");
}

/// The records of a CSV file, in order: the rows the read lands on a grid.
#[derive(PartialEq, Eq)]
pub enum CsvRow {}

impl Axis for CsvRow {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CsvRow");
}

/// A record's run of kept fields counts its columns.
impl MeasuresAs<CsvColumn> for KeptField {}

/// The fields of one CSV record, in order: the columns the read lands on a
/// grid.
#[derive(PartialEq, Eq)]
pub enum CsvColumn {}

impl Axis for CsvColumn {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CsvColumn");
}

/// The records a read left out past the room of its request.
pub enum CutRecords {}

impl NarrowCount for CutRecords {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CutRecordCount");
}

/// The fields a read left out past the room of a record.
pub enum CutFields {}

impl NarrowCount for CutFields {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CutFieldCount");
}

/// What a CSV read does with the first record it can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirstRecord {
    /// The first record is data, and the read keeps it.
    Kept,
    /// The first record is a header, and the read drops it before any room
    /// counts.
    Dropped,
}

/// The records and the fields a CSV read keeps: the room a grid has from the
/// cell the import starts at. The read counts what it leaves out, and
/// allocates nothing for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvRoom {
    pub first: FirstRecord,
    pub records: Len<CsvRow>,
    pub fields: Len<CsvColumn>,
}

/// What a CSV read left out past its room, counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvCut {
    /// The readable records past the last row of the room.
    pub records: Count<CutRecords>,
    /// The fields of kept records past the last column of the room.
    pub fields: Count<CutFields>,
}

/// The records a CSV read kept, in order, each one a list of fields.
///
/// Built by [`CsvRecords::push`], or decoded through a check that every
/// offset cuts the text where a field may end. So a reader never meets a
/// field outside the text, nor one that splits a character.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "FlatRecords")]
pub struct CsvRecords {
    /// The text of every field, end to end.
    text: String,
    /// Where each field ends in `text`, in order.
    field_ends: IdxVec<KeptField, Pos<KeptByte>>,
    /// Where each record ends in `field_ends`, in order.
    record_ends: IdxVec<CsvRow, Pos<KeptField>>,
}

/// The decoded form of [`CsvRecords`], before its offsets are checked.
#[derive(Deserialize)]
struct FlatRecords {
    text: String,
    field_ends: IdxVec<KeptField, Pos<KeptByte>>,
    record_ends: IdxVec<CsvRow, Pos<KeptField>>,
}

/// Why a [`CsvRecords`] cannot hold some records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordsRefusal {
    /// A field ends before the field before it, past the text, or inside a
    /// character.
    FieldEnd,
    /// A record ends before the record before it, or past the fields.
    RecordEnd,
    /// The text spans more bytes than an offset holds.
    TooLong,
}

impl std::fmt::Display for RecordsRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FieldEnd => "a CSV field ends outside the text",
            Self::RecordEnd => "a CSV record ends outside the fields",
            Self::TooLong => "the CSV text spans more than an offset holds",
        })
    }
}

impl TryFrom<FlatRecords> for CsvRecords {
    type Error = RecordsRefusal;

    fn try_from(flat: FlatRecords) -> Result<Self, Self::Error> {
        let text_end =
            Pos::<KeptByte>::from_index(flat.text.len()).ok_or(RecordsRefusal::TooLong)?;
        let mut start = Pos::<KeptByte>::ZERO;
        for &end in flat.field_ends.iter() {
            let boundary = end
                .to_index()
                .is_some_and(|at| flat.text.is_char_boundary(at));
            if end < start || end > text_end || !boundary {
                return Err(RecordsRefusal::FieldEnd);
            }
            start = end;
        }
        let mut first = Pos::<KeptField>::ZERO;
        let last = flat.field_ends.len().as_end();
        for &end in flat.record_ends.iter() {
            if end < first || end > last {
                return Err(RecordsRefusal::RecordEnd);
            }
            first = end;
        }
        Ok(Self {
            text: flat.text,
            field_ends: flat.field_ends,
            record_ends: flat.record_ends,
        })
    }
}

impl CsvRecords {
    /// The count of records.
    #[must_use]
    pub fn len(&self) -> Len<CsvRow> {
        self.record_ends.len()
    }

    /// Whether the read kept no record.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.record_ends.is_empty()
    }

    /// Add one record of `fields`.
    ///
    /// # Errors
    ///
    /// [`RecordsRefusal::TooLong`] when the text would pass what an offset
    /// holds. The record is then left out whole. The import bound keeps
    /// every file far under it.
    pub fn push<'a, I>(&mut self, fields: I) -> Result<(), RecordsRefusal>
    where
        I: IntoIterator<Item = &'a str>,
        I::IntoIter: Clone,
    {
        let fields = fields.into_iter();
        // The text ends past every field once the record lands, so one check
        // up front holds each offset of the record.
        let grown = fields
            .clone()
            .try_fold(self.text.len(), |len, field| len.checked_add(field.len()));
        if grown.and_then(Pos::<KeptByte>::from_index).is_none() {
            return Err(RecordsRefusal::TooLong);
        }
        for field in fields {
            self.text.push_str(field);
            self.field_ends
                .push(Pos::<KeptByte>::from_index(self.text.len()).unwrap_or(Pos::MAX));
        }
        self.record_ends.push(self.field_ends.len().as_end());
        Ok(())
    }

    /// Each record, in order.
    pub fn iter(&self) -> impl Iterator<Item = CsvRecord<'_>> + '_ {
        let starts =
            std::iter::once(Pos::<KeptField>::ZERO).chain(self.record_ends.iter().copied());
        starts
            .zip(self.record_ends.iter().copied())
            .map(move |(first, end)| CsvRecord {
                records: self,
                fields: Span::new(first, end),
            })
    }

    /// The text of the field at `at`. A decode checked every offset, so the
    /// empty text answers no field of a value this type holds.
    fn field(&self, at: Pos<KeptField>) -> &str {
        let end = self.field_ends.get(at).copied().unwrap_or_default();
        let start = at
            .prev()
            .and_then(|before| self.field_ends.get(before).copied())
            .unwrap_or_default();
        self.text
            .get(Span::new(start, end).to_usize_range())
            .unwrap_or_default()
    }
}

/// One record of [`CsvRecords`]: its fields, borrowed from the flat text.
#[derive(Clone, Copy)]
pub struct CsvRecord<'a> {
    records: &'a CsvRecords,
    fields: Span<KeptField>,
}

impl<'a> CsvRecord<'a> {
    /// The count of fields.
    #[must_use]
    pub fn len(&self) -> Len<CsvColumn> {
        self.fields.len().as_axis()
    }

    /// Whether the record holds no field.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Each field, in order.
    pub fn fields(self) -> impl Iterator<Item = &'a str> + 'a {
        let records = self.records;
        self.fields.into_iter().map(move |at| records.field(at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(text: &str, fields: &[u32], records: &[u32]) -> FlatRecords {
        FlatRecords {
            text: text.to_owned(),
            field_ends: fields.iter().map(|&end| Pos::new(end)).collect(),
            record_ends: records.iter().map(|&end| Pos::new(end)).collect(),
        }
    }

    fn read_back(records: &CsvRecords) -> Vec<Vec<&str>> {
        records.iter().map(|r| r.fields().collect()).collect()
    }

    #[test]
    fn pushed_records_read_back_in_order() {
        let mut records = CsvRecords::default();
        assert_eq!(records.push(["a", "", "bc"]), Ok(()));
        assert_eq!(records.push([]), Ok(()));
        assert_eq!(records.push(["\u{e9}t\u{e9}"]), Ok(()));
        assert_eq!(
            read_back(&records),
            vec![vec!["a", "", "bc"], vec![], vec!["\u{e9}t\u{e9}"]]
        );
        assert_eq!(records.len(), Len::new(3));
        let lens: Vec<_> = records.iter().map(|r| r.len().get()).collect();
        assert_eq!(lens, [3, 0, 1]);
    }

    #[test]
    fn records_survive_postcard() {
        let mut records = CsvRecords::default();
        assert_eq!(records.push(["x", "y"]), Ok(()));
        assert_eq!(records.push(["z"]), Ok(()));
        let bytes = postcard::to_allocvec(&records).expect("records encode");
        let back: CsvRecords = postcard::from_bytes(&bytes).expect("records decode");
        assert_eq!(back, records);
    }

    #[test]
    fn an_offset_outside_the_text_is_refused() {
        let cases = [
            (flat("ab", &[3], &[1]), RecordsRefusal::FieldEnd),
            (flat("ab", &[2, 1], &[2]), RecordsRefusal::FieldEnd),
            (flat("\u{e9}", &[1], &[1]), RecordsRefusal::FieldEnd),
            (flat("ab", &[1, 2], &[3]), RecordsRefusal::RecordEnd),
            (flat("ab", &[1, 2], &[2, 1]), RecordsRefusal::RecordEnd),
        ];
        for (flat, refusal) in cases {
            assert_eq!(CsvRecords::try_from(flat), Err(refusal));
        }
    }
}
