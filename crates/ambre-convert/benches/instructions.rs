//! The module calls whose time sets the answer deadline of each converter
//! family, held by a count of instructions (`ambre-spec-budgets.md section 8`).
//!
//! The converter thread fails a job that runs past the deadline of its family
//! (`ARCH-108`, `ARCH-127`). Each operation below runs one module call of its
//! family through `Converter::convert`, as the converter thread calls it. The
//! call encodes the request, runs the engine and decodes the answer. A time
//! measured on a shared CI runner moves by tens of percent between two runs,
//! so `scripts/instructions.py` holds the count instead.
//!
//! Each operation runs once, inside a function that is never inlined and whose
//! name is unique, so the setup costs nothing in the count. A `_largest`
//! operation runs the largest input its family admits, and a `_dense` one a
//! document of common size. Run with no operation named, as `cargo bench`
//! does, every dense operation runs once: a smoke test.

#![expect(clippy::panic, reason = "a bench aborts on a fixture it cannot run")]

use std::hint::black_box;

use ambre_convert::jpeg::{JpegDecode, JpegTurn};
use ambre_convert::{Conversion, Converted, Converter};
use ambre_core::{ConverterFamily, Payload, Px, Size};
use ambre_docx::model::{Block, Inline, Paragraph, Run, WordDocument};

/// The engine of `family`, linked in place on the host.
fn engine(family: ConverterFamily) -> Converter {
    match Converter::load(family, &Payload::new(Vec::new())) {
        Ok(converter) => converter,
        Err(e) => panic!("the host links the {family} engine: {e}"),
    }
}

/// One module call of the images family, counted alone by `callgrind`.
#[inline(never)]
fn ambre_deadline_images(converter: &Converter, conversion: &Conversion) -> Converted {
    match converter.convert(black_box(conversion)) {
        Ok(converted) => converted,
        Err(e) => panic!("the images module answers: {e}"),
    }
}

/// One Word module call of the Office family, counted alone by `callgrind`.
#[inline(never)]
fn ambre_deadline_word(converter: &Converter, conversion: &Conversion) -> Converted {
    match converter.convert(black_box(conversion)) {
        Ok(converted) => converted,
        Err(e) => panic!("the Office module answers a Word call: {e}"),
    }
}

/// One spreadsheet module call of the Office family, counted alone by
/// `callgrind`. Its body differs from the Word one: two identical functions
/// merge into one symbol, and `callgrind` then counts nothing in one of them.
#[inline(never)]
fn ambre_deadline_sheet(converter: &Converter, conversion: &Conversion) -> Converted {
    let answer = converter.convert(black_box(conversion));
    match black_box(answer) {
        Ok(converted) => converted,
        Err(e) => panic!("the Office module answers a sheet call: {e}"),
    }
}

/// The largest edge of a picture source Prose admits (`MAX_SOURCE_EDGE`).
const SOURCE_EDGE: u32 = 8192;

/// The longer side a Prose picture is decoded down to (`CACHE_EDGE`).
const CACHE_EDGE: u32 = 2048;

/// A JPEG of `width` by `height` pixels, decoded upright down to the cache
/// edge: the request Prose sends for a picture it shows. Its samples change
/// at each pixel, as a photograph's do.
fn jpeg_decode(width: u32, height: u32) -> Conversion {
    let photo = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])
    });
    let mut bytes = Vec::new();
    if let Err(e) = image::codecs::jpeg::JpegEncoder::new(&mut bytes).encode_image(&photo) {
        panic!("a JPEG encodes: {e}");
    }
    let longer = width.max(height);
    let scaled = |side: u32| (u64::from(side) * u64::from(CACHE_EDGE) / u64::from(longer)) as u32;
    Conversion::DecodeJpeg(JpegDecode {
        data: Payload::new(bytes),
        bound: Size::new(Px::new(SOURCE_EDGE), Px::new(SOURCE_EDGE)),
        size: Size::new(Px::new(scaled(width)), Px::new(scaled(height))),
        turn: JpegTurn::Upright,
    })
}

/// A photograph of 4032 by 3024 pixels, as a phone takes it.
fn images_dense() {
    let conversion = jpeg_decode(4032, 3024);
    let converter = engine(ConverterFamily::Images);
    black_box(ambre_deadline_images(&converter, &conversion));
}

/// A JPEG at the source bound on both edges, the largest decode admitted.
fn images_largest() {
    let conversion = jpeg_decode(SOURCE_EDGE, SOURCE_EDGE);
    let converter = engine(ConverterFamily::Images);
    black_box(ambre_deadline_images(&converter, &conversion));
}

/// The words of every paragraph of the Word documents below.
const WORDS: &str = "The rain had stopped before the train reached the coast, and nobody spoke. ";

/// A Word document of `pages` pages of some 400 words, a heading every ten
/// pages. Each paragraph holds a plain run, a bold run and a plain run.
fn word_document(pages: u32) -> Vec<u8> {
    let mut document = WordDocument::default();
    for page in 0..pages {
        if page.is_multiple_of(10) {
            document.body.push(Block::Paragraph(Paragraph {
                role: ambre_docx::model::ParagraphRole::Heading(
                    ambre_core::Bounded::new(1).unwrap_or_else(|| panic!("a level")),
                ),
                inlines: vec![Inline::Run(Run::plain("Chapter"))],
                ..Paragraph::default()
            }));
        }
        for _ in 0..4 {
            let mut bold = Run::plain(WORDS);
            bold.format.bold = ambre_core::Weight::Bold;
            document.body.push(Block::Paragraph(Paragraph {
                inlines: vec![
                    Inline::Run(Run::plain(WORDS.repeat(3))),
                    Inline::Run(bold),
                    Inline::Run(Run::plain(WORDS.repeat(3))),
                ],
                ..Paragraph::default()
            }));
        }
    }
    match ambre_docx::write(&document) {
        Ok(bytes) => bytes.as_ref().to_vec(),
        Err(e) => panic!("the document writes: {e}"),
    }
}

/// The pages of the dense Word document: a novel of 300 pages.
const DENSE_PAGES: u32 = 300;

/// The pages of the largest Word document: its body inflates to about
/// 31 MiB, under the 32 MiB a part may inflate to (`ambre_opc::MAX_PART_SIZE`).
const LARGEST_PAGES: u32 = 11_000;

/// The read of a Word document of `pages` pages into Prose's form.
fn docx_read(pages: u32) -> Conversion {
    Conversion::ReadDocx {
        data: Payload::new(word_document(pages)),
        zone: ambre_prose_form::TimeZone::UTC,
    }
}

/// The write of the form a Word document of `pages` pages reads into.
fn docx_write(pages: u32) -> Conversion {
    let converter = engine(ConverterFamily::Office);
    let Ok(Converted::Form(form)) = converter.convert(&docx_read(pages)) else {
        panic!("the document reads into a form");
    };
    let (ir, _): (ambre_prose_form::ProseIr, ambre_prose_form::FormReport) =
        match ambre_prose_form::decode(&form) {
            Ok(read) => read,
            Err(e) => panic!("the form decodes: {e}"),
        };
    let form = match ambre_prose_form::encode(&ir) {
        Ok(form) => form,
        Err(e) => panic!("the form encodes: {e}"),
    };
    Conversion::WriteDocx {
        form,
        zone: ambre_prose_form::TimeZone::UTC,
    }
}

fn word_read_dense() {
    let conversion = docx_read(DENSE_PAGES);
    black_box(ambre_deadline_word(
        &engine(ConverterFamily::Office),
        &conversion,
    ));
}

fn word_read_largest() {
    let conversion = docx_read(LARGEST_PAGES);
    black_box(ambre_deadline_word(
        &engine(ConverterFamily::Office),
        &conversion,
    ));
}

fn word_write_dense() {
    let conversion = docx_write(DENSE_PAGES);
    black_box(ambre_deadline_word(
        &engine(ConverterFamily::Office),
        &conversion,
    ));
}

fn word_write_largest() {
    let conversion = docx_write(LARGEST_PAGES);
    black_box(ambre_deadline_word(
        &engine(ConverterFamily::Office),
        &conversion,
    ));
}

/// A workbook of one sheet of `cells` cells in ten columns, as the sheet of
/// the xlsx import row. Half the cells are numbers, a quarter texts, a
/// quarter formulas.
fn workbook(cells: u32) -> Vec<u8> {
    use ambre_core::index::Pos;
    use ambre_xlsx::cellref::CellRef;
    use ambre_xlsx::formula::Formula;
    use ambre_xlsx::model::{Cell, CellStyle, Content, Sheet, SheetName, StyleId, Value};
    const COLS: u32 = 10;
    let name = ambre_core::Validated::<SheetName>::parse("Data".to_owned())
        .unwrap_or_else(|e| panic!("a sheet name: {e}"));
    let mut sheet = Sheet::named(name);
    sheet.cells = (0..cells)
        .map(|i| {
            let (row, col) = (i / COLS, i % COLS);
            let content = match col % 4 {
                0 | 1 => Content::Value(Value::Number(f64::from(i) * 1.5)),
                2 => Content::Value(Value::Text(format!("item {i}"))),
                _ => Content::Formula {
                    formula: Formula::from_ambre(&format!("=A{}*2+B{}", row + 1, row + 1))
                        .unwrap_or_else(|e| panic!("a formula: {e:?}")),
                    cached: Value::Number(f64::from(i)),
                },
            };
            Cell {
                at: CellRef {
                    row: Pos::new(row),
                    col: Pos::new(col),
                },
                content,
                style: StyleId::DEFAULT,
            }
        })
        .collect();
    let book = ambre_xlsx::Workbook {
        sheets: vec![sheet],
        styles: vec![CellStyle::default()],
    };
    match ambre_xlsx::write(&book) {
        Ok(written) => written.bytes.as_bytes().to_vec(),
        Err(e) => panic!("the workbook writes: {e}"),
    }
}

/// The cells of the dense workbook: the sheet of the xlsx import row.
const DENSE_CELLS: u32 = 100_000;

/// The cells of the largest workbook: its sheet inflates to about 31 MiB,
/// under the 32 MiB a part may inflate to.
const LARGEST_CELLS: u32 = 840_000;

/// The read of a workbook of `cells` cells into Trame's form.
fn xlsx_read(cells: u32) -> Conversion {
    Conversion::ReadXlsx {
        data: Payload::new(workbook(cells)),
    }
}

/// The write of Trame's form of a workbook of `cells` cells.
fn xlsx_write(cells: u32) -> Conversion {
    let converter = engine(ConverterFamily::Office);
    let Ok(Converted::Workbook(read)) = converter.convert(&xlsx_read(cells)) else {
        panic!("the workbook reads into a form");
    };
    let (book, _): (ambre_xlsx::book::Book, ambre_xlsx::Report) =
        match ambre_convert::decode_workbook(&read) {
            Ok(read) => read,
            Err(e) => panic!("the form decodes: {e}"),
        };
    match ambre_convert::encode_workbook(&book) {
        Ok(workbook) => Conversion::WriteXlsx { workbook },
        Err(e) => panic!("the form encodes: {e}"),
    }
}

fn sheet_read_dense() {
    let conversion = xlsx_read(DENSE_CELLS);
    black_box(ambre_deadline_sheet(
        &engine(ConverterFamily::Office),
        &conversion,
    ));
}

fn sheet_read_largest() {
    let conversion = xlsx_read(LARGEST_CELLS);
    black_box(ambre_deadline_sheet(
        &engine(ConverterFamily::Office),
        &conversion,
    ));
}

fn sheet_write_dense() {
    let conversion = xlsx_write(DENSE_CELLS);
    black_box(ambre_deadline_sheet(
        &engine(ConverterFamily::Office),
        &conversion,
    ));
}

fn sheet_write_largest() {
    let conversion = xlsx_write(LARGEST_CELLS);
    black_box(ambre_deadline_sheet(
        &engine(ConverterFamily::Office),
        &conversion,
    ));
}

/// One module call of the markup family, counted alone by `callgrind`.
#[inline(never)]
fn ambre_deadline_markup(converter: &Converter, conversion: &Conversion) -> Converted {
    match converter.convert(black_box(conversion)) {
        Ok(converted) => converted,
        Err(e) => panic!("the markup module answers: {e}"),
    }
}

/// The largest file a Prose Markdown or HTML import reads: 3.5 MiB
/// (`ambre_prose::interop::markdown::MAX_SIZE`).
const MARKUP_BOUND: usize = 7 * 512 * 1024;

/// A text of `bytes` bytes at most, `paragraph` over and over: a novel as
/// a word processor exports it.
fn repeated(paragraph: &str, bytes: usize) -> String {
    paragraph.repeat(bytes / paragraph.len())
}

/// One paragraph of the Markdown novel, a bold run in it.
const MARKDOWN_PARAGRAPH: &str = "The river ran **past the mill** and under the old bridge, \
    where the children of the village met each evening to watch the boats come in.\n\n";

/// One paragraph of the HTML novel, a bold run in it.
const HTML_PARAGRAPH: &str = "<p>The river ran <b>past the mill</b> and under the old \
    bridge, where the children of the village met each evening to watch the boats.</p>\n";

fn markdown_largest() {
    let conversion = Conversion::ReadMarkdown {
        text: repeated(MARKDOWN_PARAGRAPH, MARKUP_BOUND),
    };
    black_box(ambre_deadline_markup(
        &engine(ConverterFamily::Markup),
        &conversion,
    ));
}

fn html_largest() {
    let conversion = Conversion::ReadHtmlDocument {
        html: format!(
            "<html><body>{}</body></html>",
            repeated(HTML_PARAGRAPH, MARKUP_BOUND - 64)
        ),
    };
    black_box(ambre_deadline_markup(
        &engine(ConverterFamily::Markup),
        &conversion,
    ));
}

/// One module call of the tables family, counted alone by `callgrind`. Its
/// body differs from the others, so no two of them merge into one symbol.
#[inline(never)]
fn ambre_deadline_tables(converter: &Converter, conversion: &Conversion) -> Converted {
    let answer = converter.convert(black_box(conversion));
    match black_box(answer) {
        Ok(converted) => converted,
        Err(e) => panic!("the tables module answers: {e}"),
    }
}

/// The room of an import at the first cell of Trame's grid: its 100 000
/// rows and 1 000 columns. A read parses the records past it, and keeps
/// none of them.
const GRID: ambre_convert::CsvRoom = ambre_convert::CsvRoom {
    first: ambre_convert::FirstRecord::Kept,
    records: ambre_core::index::Len::new(100_000),
    fields: ambre_core::index::Len::new(1_000),
};

/// A CSV file of `bytes` bytes at most, `line` over and over.
fn csv_file(line: &str, bytes: usize) -> Vec<u8> {
    line.repeat(bytes / line.len()).into_bytes()
}

/// Read a CSV file of `line` over and over, `bytes` long at most.
fn csv_read(line: &str, bytes: usize) {
    let conversion = Conversion::ReadCsv {
        data: Payload::new(csv_file(line, bytes)),
        delimiter: ambre_convert::CsvDelimiter::COMMA,
        room: GRID,
    };
    black_box(ambre_deadline_tables(
        &engine(ConverterFamily::Tables),
        &conversion,
    ));
}

/// One line of a common export: a date, a place, a figure and short labels.
const CSV_LINE: &str = "2026-11-17,Paris,North,1234.50,17,ok,alpha,beta,7,x\n";

/// The lines of the CSV import of `ambre-spec-budgets.md section 1`.
const DENSE_LINES: usize = 10_000;

fn csv_read_dense() {
    csv_read(CSV_LINE, CSV_LINE.len() * DENSE_LINES);
}

/// A file at the bound of one short field per line, the dearest shape. Each
/// record costs its parse for two bytes of input, and the records of the
/// grid a copy of their field too. On 2026-11-17, ten short fields a line
/// counted a sixth of it, twenty one-byte fields two fifths, and twenty
/// empty fields a quarter. Those figures predate the room of the read.
fn csv_read_largest() {
    let bound = match usize::try_from(ambre_convert::CSV_MAX_SIZE.get()) {
        Ok(bound) => bound,
        Err(e) => panic!("the CSV bound fits the address space: {e}"),
    };
    csv_read("a\n", bound);
}

/// Whether `cargo bench` runs the operation when no name is given.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Smoke {
    /// The operation is dense: the smoke run takes it.
    Runs,
    /// The operation takes the largest input: only a name runs it.
    Named,
}

/// Each operation, by the name `scripts/instructions.py` passes.
const OPERATIONS: [(&str, fn(), Smoke); 14] = [
    ("images_dense", images_dense, Smoke::Runs),
    ("images_largest", images_largest, Smoke::Named),
    ("word_read_dense", word_read_dense, Smoke::Runs),
    ("word_read_largest", word_read_largest, Smoke::Named),
    ("word_write_dense", word_write_dense, Smoke::Runs),
    ("word_write_largest", word_write_largest, Smoke::Named),
    ("sheet_read_dense", sheet_read_dense, Smoke::Runs),
    ("sheet_read_largest", sheet_read_largest, Smoke::Named),
    ("sheet_write_dense", sheet_write_dense, Smoke::Runs),
    ("sheet_write_largest", sheet_write_largest, Smoke::Named),
    ("markdown_largest", markdown_largest, Smoke::Named),
    ("html_largest", html_largest, Smoke::Named),
    ("csv_read_dense", csv_read_dense, Smoke::Runs),
    ("csv_read_largest", csv_read_largest, Smoke::Named),
];

fn main() {
    // `cargo bench` passes flags such as `--bench`; only a bare word names an
    // operation.
    let named: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    for (name, run, smoke) in OPERATIONS {
        let picked = if named.is_empty() {
            smoke == Smoke::Runs
        } else {
            named.iter().any(|n| n == name)
        };
        if picked {
            run();
        }
    }
}
