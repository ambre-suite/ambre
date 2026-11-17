// ambre-convert CSV memory test (tier `host`, `TRAME-IO-091`).
// ============================================================================
// The ceiling of the `Trame CSV import file` row of
// `ambre-spec-budgets.md section 6`. It reads the most bytes the heap held at
// once while the tables engine reads a file at the import bound. The read
// runs as the converter thread runs it: the request encoded, the engine
// call, the answer decoded. The file itself counts.
//
// The test is alone in its binary, so no other test shares the allocator it
// counts. A counting allocator reads the heap itself: what the C allocator
// keeps of the freed memory, and the pages it never touched, do not count.
// ============================================================================
#![cfg(all(not(target_arch = "wasm32"), feature = "tables"))]

use std::alloc::System;

use ambre_convert::{
    CSV_MAX_SIZE, Conversion, Converted, Converter, CsvDelimiter, CsvRoom, FirstRecord,
};
use ambre_core::index::Len;
use ambre_core::{ConverterFamily, Payload};
use cap::Cap;

/// The allocator of this binary: the system one, with a count of the bytes
/// it holds, their peak, and no limit.
#[global_allocator]
static HEAP: Cap<System> = Cap::new(System, usize::MAX);

/// The ceiling of the peak, in bytes: 100 MB.
const CEILING: usize = 100_000_000;

/// The room of an import at the first cell of Trame's grid: its 100 000
/// rows and 1 000 columns.
const GRID: CsvRoom = CsvRoom {
    first: FirstRecord::Kept,
    records: Len::new(100_000),
    fields: Len::new(1_000),
};

/// A file of `line` over and over, at the import bound at most.
fn file_of(line: &str) -> Vec<u8> {
    let bound = usize::try_from(CSV_MAX_SIZE.get()).unwrap_or(usize::MAX);
    line.repeat(bound / line.len()).into_bytes()
}

/// The peak of the heap over the read of a file of `line` at the bound, past
/// what the heap held before the file existed. The rows the read kept come
/// back beside it. `None` when the answer holds no records.
fn peak_of_read(converter: &Converter, line: &str) -> Option<(usize, u32)> {
    let before = HEAP.allocated();
    let conversion = Conversion::ReadCsv {
        data: Payload::new(file_of(line)),
        delimiter: CsvDelimiter::COMMA,
        room: GRID,
    };
    let answer = converter.convert(&conversion);
    let Ok(Converted::Records { records, .. }) = answer else {
        return None;
    };
    let rows = records.len().get();
    drop(records);
    drop(conversion);
    Some((HEAP.max_allocated().saturating_sub(before), rows))
}

/// `TRAME-IO-091`: the read of a file at the import bound peaks under
/// `CEILING`. The file of one-byte lines holds the most records: 4 194 304.
/// The file of ten short fields a line keeps the most text. The peak never
/// falls, so the shape that peaks lower runs first, and the second reading
/// holds both.
#[test]
fn a_csv_read_at_the_bound_peaks_under_the_ceiling() {
    let converter = Converter::load(ConverterFamily::Tables, &Payload::new(Vec::new()))
        .expect("the host links the tables engine");
    let (peak, rows) = peak_of_read(&converter, "a\n").expect("the read answers records");
    eprintln!("one-byte lines: {rows} rows kept, the heap peaked at {peak} bytes");
    assert_eq!(
        rows,
        GRID.records.get(),
        "the read keeps the rows of the grid"
    );
    let common = "2026-11-17,Paris,North,1234.50,17,ok,alpha,beta,7,x\n";
    let (peak, rows) = peak_of_read(&converter, common).expect("the read answers records");
    eprintln!("ten short fields a line: {rows} rows kept, the heap peaked at {peak} bytes");
    assert_eq!(
        rows,
        GRID.records.get(),
        "the read keeps the rows of the grid"
    );
    assert!(
        peak < CEILING,
        "a CSV read at the bound peaked at {peak} bytes"
    );
}
