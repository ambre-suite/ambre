//! The codec on files it did not write. LibreOffice writes `libreoffice/`
//! from a committed flat source. pandoc writes `pandoc/` from committed
//! Markdown sources, and LibreOffice saves each as Word. Each `.odt` sits
//! beside its Word twin. `fixtures/AUTHORED.md` names each file.

#![expect(
    clippy::panic,
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use std::fmt::Write as _;

use ambre_docx::model::{Block, Inline, Run, WordDocument};
use ambre_prose_form::zone::Zone;
use ambre_prose_form::{FormReport, IrBlock, IrRun, ProseIr};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// Each source, written as an `.odt` and as a `.docx` by one tool.
const PAIRS: [&str; 3] = ["libreoffice/features", "pandoc/inline", "pandoc/structure"];

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{DIR}/{name}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The form an OpenDocument file lands as, and its report.
fn odt_form(bytes: &[u8]) -> (ProseIr, FormReport) {
    let decoded = ambre_odt::read(bytes).expect("the package reads");
    ambre_prose_form::word_read::read_document(&decoded.document, &decoded.report, &Zone::utc())
}

/// The form a Word file lands as, and its report.
///
/// A Word note body opens on the separator the writer draws after the note
/// mark: LibreOffice writes a tab there, and the source holds none. The
/// Word reader keeps it as text, so the comparison drops it, and nothing
/// else.
fn docx_form(bytes: &[u8]) -> (ProseIr, FormReport) {
    let decoded = ambre_docx::read(bytes).expect("the package reads");
    let (mut ir, report) = ambre_prose_form::word_read::read_document(
        &decoded.document,
        &decoded.report,
        &Zone::utc(),
    );
    for note in &mut ir.footnotes {
        if let Some(first) = note.runs.first_mut()
            && let Some(rest) = first.text.strip_prefix(['\t', ' '])
        {
            first.text = rest.to_owned();
        }
    }
    (ir, report)
}

/// The form with each two neighbour runs of the same marks joined: where a
/// tool cuts its runs is no data the user reads.
fn joined(mut ir: ProseIr) -> ProseIr {
    join_blocks(&mut ir.blocks);
    for note in &mut ir.footnotes {
        note.runs = join(std::mem::take(&mut note.runs));
    }
    ir
}

fn join_blocks(blocks: &mut [IrBlock]) {
    for block in blocks {
        match block {
            IrBlock::Paragraph { runs, .. } | IrBlock::Heading { runs, .. } => {
                *runs = join(std::mem::take(runs));
            }
            IrBlock::ListItem { runs, children, .. } => {
                *runs = join(std::mem::take(runs));
                join_blocks(children);
            }
            IrBlock::Table { rows, .. } => {
                for cell in rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                    join_blocks(&mut cell.blocks);
                }
            }
            _ => {}
        }
    }
}

fn join(runs: Vec<IrRun>) -> Vec<IrRun> {
    let mut out: Vec<IrRun> = Vec::new();
    for run in runs {
        if let Some(last) = out.last_mut() {
            let mut a = last.clone();
            a.text.clear();
            let mut b = run.clone();
            b.text.clear();
            if a == b {
                last.text.push_str(&run.text);
                continue;
            }
        }
        if !run.text.is_empty() || run.footnote_ref.is_some() {
            out.push(run);
        }
    }
    out
}

/// The entries of a report, in the order the user reads them.
fn entries(report: &FormReport) -> Vec<String> {
    report
        .entries()
        .iter()
        .map(|e| format!("{:?} {:?} {}", e.construct, e.outcome, e.count.get()))
        .collect()
}

/// A dump of the form that prints the text, for a diff a person reads.
fn dump(ir: &ProseIr) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "meta {:?} {:?} {:?}",
        ir.meta.title, ir.meta.paged, ir.meta.bands
    )
    .ok();
    dump_blocks(&ir.blocks, 0, &mut out);
    for note in &ir.footnotes {
        writeln!(out, "note {:?} {}", note.ordinal, runs(&note.runs)).ok();
    }
    for comment in &ir.comments {
        writeln!(out, "thread {:?}", comment.resolved).ok();
        for reply in &comment.replies {
            writeln!(
                out,
                "  reply {:?} {:?} {:?}",
                reply.author, reply.created_at, reply.text
            )
            .ok();
        }
    }
    for suggestion in &ir.suggestions {
        writeln!(
            out,
            "suggestion {:?} {:?} {:?}",
            suggestion.kind, suggestion.author, suggestion.date
        )
        .ok();
    }
    out
}

fn runs(runs: &[IrRun]) -> String {
    runs.iter()
        .map(|r| {
            let mut marks = r.clone();
            marks.text.clear();
            format!(
                "[{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} c{:?} i{:?} d{:?} n{:?}]",
                r.text,
                r.bold,
                r.italic,
                r.underline,
                r.strikethrough,
                r.superscript,
                r.subscript,
                r.link,
                r.styled,
                r.comments,
                r.insertion,
                r.deletion,
                r.footnote_ref
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn dump_blocks(blocks: &[IrBlock], depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    for block in blocks {
        match block {
            IrBlock::Paragraph { styled, runs: r } => {
                writeln!(out, "{pad}p {styled:?} {}", runs(r)).ok();
            }
            IrBlock::Heading { level, runs: r } => {
                writeln!(out, "{pad}h {level:?} {}", runs(r)).ok();
            }
            IrBlock::ListItem {
                style,
                runs: r,
                children,
            } => {
                writeln!(out, "{pad}li {style:?} {}", runs(r)).ok();
                dump_blocks(children, depth + 1, out);
            }
            IrBlock::Table { merged, rows } => {
                writeln!(out, "{pad}table {merged:?}").ok();
                for row in rows {
                    writeln!(out, "{pad} row").ok();
                    for cell in &row.cells {
                        writeln!(out, "{pad}  cell {:?}", cell.styled).ok();
                        dump_blocks(&cell.blocks, depth + 3, out);
                    }
                }
            }
            other => {
                writeln!(out, "{pad}{other:?}").ok();
            }
        }
    }
}

#[test]
fn an_odt_import_lands_as_the_word_import_of_the_same_source() {
    for pair in PAIRS {
        let (odt, odt_report) = odt_form(&fixture(&format!("{pair}.odt")));
        let (docx, docx_report) = docx_form(&fixture(&format!("{pair}.docx")));
        let (odt, docx) = (joined(odt), joined(docx));
        assert_eq!(dump(&odt), dump(&docx), "{pair}: the form differs");
        assert_eq!(odt, docx, "{pair}: the form differs past the dump");
        assert_eq!(
            entries(&odt_report),
            entries(&docx_report),
            "{pair}: the report differs"
        );
    }
}

/// Every `.odt` file a test reads.
const ODT: [&str; 3] = [
    "libreoffice/features.odt",
    "pandoc/inline.odt",
    "pandoc/structure.odt",
];

/// `PROSE-IO-090`: each input file reads, writes and reads back as the
/// same document, its ids numbered again.
#[test]
fn every_odt_fixture_survives_a_round_trip() {
    for name in ODT {
        let document = ambre_odt::read(&fixture(name)).expect(name).document;
        let bytes = ambre_odt::write(&document).expect(name);
        let again = ambre_odt::read(bytes.as_bytes()).expect(name);
        // The ids are no part of the file: a region and an annotation go
        // by name. So both sides are numbered again.
        assert_eq!(again.document.renumbered(), document.renumbered(), "{name}");
        assert!(again.report.is_lossless(), "{name}: {:?}", again.report);
    }
}

/// `PROSE-IO-091`: the same document writes the same bytes.
#[test]
fn a_document_writes_the_same_bytes_twice() {
    for name in ODT {
        let document = ambre_odt::read(&fixture(name)).expect(name).document;
        assert_eq!(
            ambre_odt::write(&document).expect(name),
            ambre_odt::write(&document).expect(name),
            "{name}"
        );
    }
}

/// `PROSE-IO-089`: a form exported as an OpenDocument text lands, imported
/// again, as the same form exported as Word, with the same report.
#[test]
fn an_odt_export_reads_back_as_the_word_export_of_the_same_form() {
    for pair in PAIRS {
        let (ir, _) = docx_form(&fixture(&format!("{pair}.docx")));
        let (document, _) = ambre_prose_form::word_write::write(&ir, &Zone::utc());
        let odt = ambre_odt::write(&document).expect(pair);
        let docx = ambre_docx::write(&document).expect(pair);
        let (odt, odt_report) = odt_form(odt.as_bytes());
        let (docx, docx_report) = docx_form(docx.as_bytes());
        let (odt, docx) = (joined(odt), joined(docx));
        assert_eq!(dump(&odt), dump(&docx), "{pair}: the form differs");
        assert_eq!(odt, docx, "{pair}: the form differs past the dump");
        assert_eq!(
            entries(&odt_report),
            entries(&docx_report),
            "{pair}: the report differs"
        );
    }
}

/// The model with each two neighbour runs of one format and one revision
/// joined. An OpenDocument span holds text, not a run, so the reader joins
/// what the writer cut.
fn joined_model(document: &WordDocument) -> WordDocument {
    let mut document = document.renumbered();
    let mut blocks: Vec<&mut Vec<Block>> = vec![&mut document.body];
    blocks.extend(document.header.iter_mut());
    blocks.extend(document.footer.iter_mut());
    blocks.extend(
        document
            .footnotes
            .iter_mut()
            .chain(document.endnotes.iter_mut())
            .map(|note| &mut note.blocks),
    );
    for list in blocks {
        join_model_blocks(list);
    }
    for comment in &mut document.comments {
        for paragraph in &mut comment.paragraphs {
            join_inlines(&mut paragraph.inlines);
        }
    }
    document
}

fn join_model_blocks(blocks: &mut [Block]) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => join_inlines(&mut paragraph.inlines),
            Block::Table(table) => {
                for cell in table.rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                    join_model_blocks(&mut cell.blocks);
                }
            }
        }
    }
}

fn join_inlines(inlines: &mut Vec<Inline>) {
    let mut out: Vec<Inline> = Vec::new();
    for mut inline in inlines.drain(..) {
        if let Inline::Link(link) = &mut inline {
            link.runs = join_model_runs(std::mem::take(&mut link.runs));
        }
        if let (Some(Inline::Run(last)), Inline::Run(run)) = (out.last_mut(), &inline)
            && last.format == run.format
            && last.revision == run.revision
        {
            last.text.push_str(&run.text);
            continue;
        }
        if matches!(&inline, Inline::Run(run) if run.text.is_empty()) {
            continue;
        }
        out.push(inline);
    }
    *inlines = out;
}

fn join_model_runs(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for run in runs {
        if let Some(last) = out.last_mut()
            && last.format == run.format
            && last.revision == run.revision
        {
            last.text.push_str(&run.text);
            continue;
        }
        out.push(run);
    }
    out
}

/// `PROSE-IO-089`: every Word file of the Word codec's test set, written as
/// an OpenDocument text, reads back as the same document. The writer is
/// held on files it did not read itself.
#[test]
fn every_word_fixture_written_as_odt_reads_back() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-docx/tests/fixtures");
    let mut read = 0;
    for folder in ["libreoffice", "open-xml-sdk", "pandoc"] {
        let entries = std::fs::read_dir(format!("{dir}/{folder}")).expect("the folder lists");
        for entry in entries {
            let path = entry.expect("an entry").path();
            if path.extension().is_none_or(|e| e != "docx") {
                continue;
            }
            let name = path.display().to_string();
            let bytes = std::fs::read(&path).expect("the file reads");
            let document = ambre_docx::read(&bytes).expect(&name).document;
            let written = ambre_odt::write(&document).expect(&name);
            let again = ambre_odt::read(written.as_bytes()).expect(&name).document;
            assert_eq!(joined_model(&again), joined_model(&document), "{name}");
            read += 1;
        }
    }
    assert!(read >= 17, "the Word test set lists {read} files");
}

/// A package holding `content` as its content part, and the entries
/// `extra` names, each with its bytes.
fn package(content: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write as _;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = stored.compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).expect("an entry");
    zip.write_all(b"application/vnd.oasis.opendocument.text")
        .expect("written");
    zip.start_file("content.xml", deflated).expect("an entry");
    zip.write_all(content.as_bytes()).expect("written");
    for (name, bytes) in extra {
        zip.start_file(*name, deflated).expect("an entry");
        zip.write_all(bytes).expect("written");
    }
    zip.finish().expect("the archive").into_inner()
}

const TEXT: &str = concat!(
    r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
    r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">"#,
    r#"<office:body><office:text><text:p>hi</text:p></office:text></office:body>"#,
    r#"</office:document-content>"#
);

/// `PROSE-IO-092`: a part that inflates past its bound fails typed while it
/// inflates, before it holds the memory it asks for.
#[test]
fn a_zip_bomb_fails_typed() {
    let padding = format!("<!-- {} -->", " ".repeat(33 * 1024 * 1024));
    let bomb = package(&format!("{TEXT}{padding}"), &[]);
    assert!(bomb.len() < 1024 * 1024, "the bomb is small on disk");
    assert_eq!(
        ambre_odt::read(&bomb).map(|_| ()),
        Err(ambre_odt::OdtError::Package(ambre_opc::OpcError::TooLarge(
            ambre_opc::MAX_PART_SIZE
        )))
    );
}

/// `PROSE-IO-092`: a document type declaration is refused, and a nesting
/// past the bound fails without recursion.
#[test]
fn an_external_entity_and_a_hostile_nesting_fail_typed() {
    let entity = format!(
        r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY e SYSTEM "file:///etc/passwd">]>{TEXT}"#
    );
    assert_eq!(
        ambre_odt::read(&package(&entity, &[])).map(|_| ()),
        Err(ambre_odt::OdtError::Xml(
            ambre_odt::xml::XmlError::DocumentType
        ))
    );
    let depth = 100_000;
    let nested = TEXT.replace(
        "<text:p>hi</text:p>",
        &format!(
            "{}{}",
            "<text:span>".repeat(depth),
            "</text:span>".repeat(depth)
        ),
    );
    assert_eq!(
        ambre_odt::read(&package(&nested, &[])).map(|_| ()),
        Err(ambre_odt::OdtError::Xml(ambre_odt::xml::XmlError::TooDeep))
    );
}

/// `PROSE-IO-092`: a truncated or a corrupted file fails typed, and never
/// panics.
#[test]
fn a_truncated_or_corrupted_file_never_panics() {
    let bytes = fixture("libreoffice/features.odt");
    for cut in (0..bytes.len()).step_by(97) {
        let _ = ambre_odt::read(&bytes[..cut]);
    }
    for at in (0..bytes.len()).step_by(89) {
        let mut flipped = bytes.clone();
        if let Some(byte) = flipped.get_mut(at) {
            *byte ^= 0x5A;
        }
        let _ = ambre_odt::read(&flipped);
    }
    assert_eq!(
        ambre_odt::read(b"not a zip").map(|_| ()),
        Err(ambre_odt::OdtError::Package(ambre_opc::OpcError::NotAZip))
    );
}

/// `PROSE-IO-093`: a macro library is named in the report and never read.
/// It inflates past the bound of a part, so a read of it would fail.
#[test]
fn a_macro_library_is_named_and_never_read() {
    let library = vec![b' '; 33 * 1024 * 1024];
    let bytes = package(TEXT, &[("Basic/Standard/Module1.xml", &library)]);
    let decoded = ambre_odt::read(&bytes).expect("the text reads");
    assert_eq!(
        decoded
            .report
            .count(
                ambre_docx::WordConstruct::Macro,
                ambre_docx::Outcome::Dropped
            )
            .get(),
        1
    );
}

/// `PROSE-IO-094`: a link keeps its address and a linked picture is named,
/// and neither is fetched: the codec declares no network crate.
#[test]
fn a_linked_picture_is_reported_and_never_fetched() {
    let content = TEXT
        .replace(
            r#"xmlns:text="#,
            r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:text="#,
        )
        .replace(
            "<text:p>hi</text:p>",
            r#"<text:p>hi<draw:frame text:anchor-type="as-char"><draw:image xlink:href="https://example.org/a.png"/></draw:frame></text:p>"#,
        );
    let decoded = ambre_odt::read(&package(&content, &[])).expect("the text reads");
    assert_eq!(
        decoded
            .report
            .count(
                ambre_docx::WordConstruct::LinkedPicture,
                ambre_docx::Outcome::Dropped
            )
            .get(),
        1
    );
}

/// `PROSE-IO-095`: an element the reader does not recognise is named in
/// the report, and its text kept.
#[test]
fn an_unknown_element_is_reported_and_its_text_kept() {
    let content = TEXT.replace(
        "<text:p>hi</text:p>",
        r#"<text:p>a <x:wrap xmlns:x="urn:example:unknown">kept words</x:wrap> b</text:p>"#,
    );
    let decoded = ambre_odt::read(&package(&content, &[])).expect("the text reads");
    let (ir, _) = ambre_prose_form::word_read::read_document(
        &decoded.document,
        &decoded.report,
        &Zone::utc(),
    );
    let text: String = match ir.blocks.first() {
        Some(IrBlock::Paragraph { runs, .. }) => runs.iter().map(|r| r.text.as_str()).collect(),
        other => panic!("a paragraph: {other:?}"),
    };
    assert_eq!(text, "a kept words b");
    assert_eq!(
        decoded
            .report
            .count(
                ambre_docx::WordConstruct::UnknownElement,
                ambre_docx::Outcome::Dropped
            )
            .get(),
        1
    );
}
