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

/// A package holding `content` as its content part, and the parts `extra`
/// names, each with its bytes.
fn package(content: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    use ambre_opc::PartName;
    use ambre_opc::odf::{OdfType, OdfWriter};
    let name = |name: &str| PartName::new(name).expect("a part name");
    let mut writer = OdfWriter::new(OdfType::TEXT);
    writer.part(
        name("content.xml"),
        OdfType::XML,
        ambre_core::Payload::new(content.as_bytes().to_vec()),
    );
    for (part, bytes) in extra {
        writer.part(
            name(part),
            OdfType::XML,
            ambre_core::Payload::new(bytes.to_vec()),
        );
    }
    writer.finish().expect("the archive").into_bytes()
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

/// A content part whose text body is `body`, under the namespaces the
/// hostile files below use.
fn body(body: &str) -> String {
    format!(
        concat!(
            r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
            r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" "#,
            r#"xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0">"#,
            r#"<office:body><office:text>{}</office:text></office:body>"#,
            r#"</office:document-content>"#
        ),
        body
    )
}

/// The text of every run of `blocks`, cells included, in order.
fn all_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => {
                for inline in &paragraph.inlines {
                    if let Inline::Run(run) = inline {
                        out.push_str(&run.text);
                    }
                }
            }
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        out.push_str(&all_text(&cell.blocks));
                    }
                }
            }
        }
    }
    out
}

/// A deletion region whose removed paragraph holds `inner`.
fn deletion(inner: &str) -> String {
    format!(
        concat!(
            r#"<text:tracked-changes><text:changed-region text:id="x"><text:deletion>"#,
            r#"<office:change-info/><text:p>{}</text:p>"#,
            r#"</text:deletion></text:changed-region></text:tracked-changes>"#
        ),
        inner
    )
}

/// `W300`: a tracked deletion whose removed paragraph names the deletion
/// itself ends: the mark expands once, and the read answers.
#[test]
fn a_deletion_that_names_itself_reads_once() {
    let content = body(&format!(
        r#"{}<text:p><text:change text:change-id="x"/></text:p>"#,
        deletion(r#"gone<text:change text:change-id="x"/>"#)
    ));
    let decoded = ambre_odt::read(&package(&content, &[])).expect("the text reads");
    assert_eq!(all_text(&decoded.document.body), "gone");
}

/// `W301`: a deleted region is copied at its first mark alone, whatever the
/// count of marks that name it.
#[test]
fn a_deleted_region_is_copied_once_whatever_its_marks() {
    let marks = r#"<text:change text:change-id="x"/>"#.repeat(10_000);
    let content = body(&format!(
        "{}<text:p>{marks}</text:p>",
        deletion(&"d".repeat(1024 * 1024))
    ));
    let decoded = ambre_odt::read(&package(&content, &[])).expect("the text reads");
    assert_eq!(all_text(&decoded.document.body).len(), 1024 * 1024);
}

/// `W302`: repeated rows of repeated cells nest a table in the cell. They
/// pass the walk bound of a document and fail typed, before they hold the
/// cells they ask for.
#[test]
fn nested_repeats_of_a_table_fail_typed() {
    let mut cell = "<text:p>a</text:p>".to_owned();
    for _ in 0..3 {
        cell = format!(
            concat!(
                r#"<table:table><table:table-row table:number-rows-repeated="1024">"#,
                r#"<table:table-cell table:number-columns-repeated="1024">{}</table:table-cell>"#,
                r#"</table:table-row></table:table>"#
            ),
            cell
        );
    }
    let content = body(&cell);
    assert!(content.len() < 2048, "the hostile body is small");
    assert_eq!(
        ambre_odt::read(&package(&content, &[])).map(|_| ()),
        Err(ambre_odt::OdtError::TooLarge)
    );
}

/// `W302`: space marks each under the repeat bound write past the text bound
/// of a document together, and fail typed.
#[test]
fn space_marks_past_the_text_bound_fail_typed() {
    let marks = r#"<text:s text:c="1024"/>"#.repeat(70_000);
    let content = body(&format!("<text:p>{marks}</text:p>"));
    assert_eq!(
        ambre_odt::read(&package(&content, &[])).map(|_| ()),
        Err(ambre_odt::OdtError::TooLarge)
    );
}

/// `W72`: a content part of empty elements past the node bound of a tree
/// fails typed, though it reads under the bound of a part.
#[test]
fn a_part_past_the_node_bound_fails_typed() {
    let past = usize::try_from(ambre_opc::MAX_TREE_NODES.get()).expect("fits");
    let content = body(&"<text:soft-page-break/>".repeat(past));
    assert_eq!(
        ambre_odt::read(&package(&content, &[])).map(|_| ()),
        Err(ambre_odt::OdtError::Xml(ambre_odt::xml::XmlError::TooLarge))
    );
}

/// `P66`: ten thousand paragraph styles form one chain of parents, and each
/// paragraph names its last style. A chain stops at its bound, so no
/// paragraph walks every style, and the text reads in time.
#[test]
fn a_long_chain_of_styles_reads_within_its_bound() {
    let count = 10_000;
    let styles: String = (0..count)
        .map(|n| {
            let parent = if n == 0 {
                String::new()
            } else {
                format!(r#" style:parent-style-name="s{}""#, n - 1)
            };
            format!(r#"<style:style style:name="s{n}" style:family="paragraph"{parent}/>"#)
        })
        .collect();
    let paragraphs =
        format!(r#"<text:p text:style-name="s{}">x</text:p>"#, count - 1).repeat(count);
    let content = format!(
        concat!(
            r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
            r#"xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" "#,
            r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">"#,
            r#"<office:automatic-styles>{}</office:automatic-styles>"#,
            r#"<office:body><office:text>{}</office:text></office:body>"#,
            r#"</office:document-content>"#
        ),
        styles, paragraphs
    );
    let decoded = ambre_odt::read(&package(&content, &[])).expect("the text reads");
    assert_eq!(all_text(&decoded.document.body).len(), count);
}
