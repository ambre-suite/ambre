//! The codec on files it did not write. Word files come from the test sets
//! of pandoc and of the Open XML SDK. LibreOffice writes one file from a
//! committed source. Each other test assembles a package from XML parts.
//! `fixtures/THIRD_PARTY.md` names the source of each copied file.

#![expect(
    clippy::panic,
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use std::fmt::Write as _;

use ambre_core::Payload;
use ambre_docx::model::{
    Block, CommentState, Inline, LinkTarget, ListKind, NoteKind, NumberFormat, Paragraph,
    ParagraphRole, Revision, RevisionKind, Run, VerticalPosition, WordDate, WordDocument,
};
use ambre_docx::opc::{OpcError, PackageWriter, PartName, RelKind, RelTarget};
use ambre_docx::{DocxError, Outcome, WordConstruct, WordReport, XmlError, read, rel, write};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{DIR}/{name}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Every file a test reads, from the three sources.
const FIXTURES: [&str; 17] = [
    "pandoc/block_quotes.docx",
    "pandoc/codeblock.docx",
    "pandoc/comments.docx",
    "pandoc/headers.docx",
    "pandoc/image.docx",
    "pandoc/inline_formatting.docx",
    "pandoc/links.docx",
    "pandoc/lists.docx",
    "pandoc/notes.docx",
    "pandoc/tables.docx",
    "pandoc/track_changes_deletion.docx",
    "pandoc/track_changes_insertion.docx",
    "pandoc/track_changes_move.docx",
    "pandoc/track_changes_scrubbed_metadata.docx",
    "open-xml-sdk/Comment004.docx",
    "open-xml-sdk/strict-numbering.docx",
    "libreoffice/features.docx",
];

fn paragraphs(blocks: &[Block]) -> Vec<&Paragraph> {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => out.push(paragraph),
            Block::Table(table) => {
                for cell in table.rows.iter().flat_map(|row| &row.cells) {
                    out.extend(paragraphs(&cell.blocks));
                }
            }
        }
    }
    out
}

fn runs(paragraph: &Paragraph) -> Vec<&Run> {
    paragraph
        .inlines
        .iter()
        .flat_map(|inline| match inline {
            Inline::Run(run) => vec![run],
            Inline::Link(link) => link.runs.iter().collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn text(paragraph: &Paragraph) -> String {
    runs(paragraph)
        .iter()
        .map(|run| run.text.as_str())
        .collect()
}

fn revisions(document: &WordDocument) -> Vec<(Revision, String)> {
    paragraphs(&document.body)
        .into_iter()
        .flat_map(runs)
        .filter_map(|run| Some((run.revision.clone()?, run.text.clone())))
        .collect()
}

/// The canonical dump of a document: one line per block and per note,
/// comment and band, with the text of each run. A change to what the reader
/// reads changes the dump, and the `.expected` file beside the fixture
/// holds the dump a person accepted.
fn dump(document: &WordDocument) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "title {:?}", document.title);
    if let Some(page) = &document.page {
        let _ = writeln!(
            out,
            "page {}x{} {:?} margins {} {} {} {}",
            page.width.get(),
            page.height.get(),
            page.orientation,
            page.margins.top.get(),
            page.margins.right.get(),
            page.margins.bottom.get(),
            page.margins.left.get()
        );
    }
    dump_blocks(&mut out, &document.body, 0);
    for (label, blocks) in [("header", &document.header), ("footer", &document.footer)] {
        if let Some(blocks) = blocks {
            let _ = writeln!(out, "{label}");
            dump_blocks(&mut out, blocks, 1);
        }
    }
    for (label, notes) in [
        ("footnote", &document.footnotes),
        ("endnote", &document.endnotes),
    ] {
        for note in notes {
            let _ = writeln!(out, "{label} {}", note.id.get());
            dump_blocks(&mut out, &note.blocks, 1);
        }
    }
    for comment in &document.comments {
        let _ = writeln!(
            out,
            "comment {} by {:?} at {:?} parent {:?} {:?}",
            comment.id.get(),
            comment.author,
            comment.date,
            comment.parent.map(ambre_docx::model::WordId::get),
            comment.state
        );
        for paragraph in &comment.paragraphs {
            dump_paragraph(&mut out, paragraph, 1);
        }
    }
    out
}

fn dump_blocks(out: &mut String, blocks: &[Block], depth: usize) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => dump_paragraph(out, paragraph, depth),
            Block::Table(table) => {
                let columns: Vec<_> = table.columns.iter().map(|c| c.get()).collect();
                let _ = writeln!(out, "{}table {columns:?}", "  ".repeat(depth));
                for row in &table.rows {
                    let _ = writeln!(out, "{}row {:?}", "  ".repeat(depth + 1), row.role);
                    for cell in &row.cells {
                        let _ = writeln!(
                            out,
                            "{}cell span {} {:?} {:?}",
                            "  ".repeat(depth + 2),
                            cell.span.get(),
                            cell.merge,
                            cell.fill
                        );
                        dump_blocks(out, &cell.blocks, depth + 3);
                    }
                }
            }
        }
    }
}

fn dump_paragraph(out: &mut String, paragraph: &Paragraph, depth: usize) {
    let _ = write!(
        out,
        "{}p {:?} {:?} {:?} |",
        "  ".repeat(depth),
        paragraph.role,
        paragraph
            .list
            .map(|l| (l.list.get(), l.level.get(), l.kind)),
        paragraph.align
    );
    for inline in &paragraph.inlines {
        match inline {
            Inline::Run(run) => dump_run(out, run),
            Inline::Link(link) => {
                let target = match &link.target {
                    LinkTarget::External(url) => url.as_str().to_owned(),
                    LinkTarget::Anchor(name) => format!("#{name}"),
                };
                let _ = write!(out, " <link {target}>");
                for run in &link.runs {
                    dump_run(out, run);
                }
                out.push_str(" </link>");
            }
            Inline::Image(image) => {
                let _ = write!(
                    out,
                    " <image {:?} {}x{} {:?} {} bytes>",
                    image.description,
                    image.width.get(),
                    image.height.get(),
                    image.format,
                    image.bytes.as_ref().len()
                );
            }
            other => {
                let _ = write!(out, " {other:?}");
            }
        }
    }
    out.push('\n');
}

fn dump_run(out: &mut String, run: &Run) {
    let format = &run.format;
    let mut marks = String::new();
    for (on, mark) in [
        (format.bold.as_bool(), 'b'),
        (format.italic.as_bool(), 'i'),
        (format.underline.as_bool(), 'u'),
        (format.strike.as_bool(), 's'),
        (format.position == VerticalPosition::Superscript, '^'),
        (format.position == VerticalPosition::Subscript, '_'),
    ] {
        if on {
            marks.push(mark);
        }
    }
    let _ = write!(out, " [{:?}", run.text);
    if !marks.is_empty() {
        let _ = write!(out, " {marks}");
    }
    if let Some(size) = format.size {
        let _ = write!(out, " {}pt", size.points());
    }
    if let Some(color) = format.color {
        let _ = write!(out, " {color}");
    }
    if let Some(fill) = format.fill {
        let _ = write!(out, " on {fill}");
    }
    if let Some(font) = &format.font {
        let _ = write!(out, " {font:?}");
    }
    if let Some(revision) = &run.revision {
        let _ = write!(
            out,
            " {:?} {} by {:?} at {:?}",
            revision.kind,
            revision.id.get(),
            revision.author,
            revision.date
        );
    }
    out.push(']');
}

/// The report as the sentence the user reads, in the words of the shared
/// report for an import (`ambre_bus::interop::ConversionReport::sentence`).
/// It names what arrived first, then each loss, worst last, three constructs
/// each. The codec names no bus type, so the dump spells the sentence here.
fn sentence(report: &WordReport) -> Option<String> {
    let noun = |construct: WordConstruct, count: u32| {
        if count == 1 {
            construct.label()
        } else {
            construct.plural()
        }
    };
    let carried: Vec<String> = report
        .entries()
        .iter()
        .filter(|e| e.outcome == Outcome::Carried)
        .map(|e| format!("{} {}", e.count.get(), noun(e.construct, e.count.get())))
        .collect();
    let head = carried.split_last().map(|(last, rest)| {
        if rest.is_empty() {
            format!("imported {last}")
        } else {
            format!("imported {} and {last}", rest.join(", "))
        }
    });
    let losses = [
        (Outcome::Approximated, "approximated"),
        (Outcome::Dropped, "dropped"),
    ]
    .into_iter()
    .filter_map(|(outcome, verb)| {
        let met: Vec<_> = report
            .entries()
            .iter()
            .filter(|e| e.outcome == outcome)
            .collect();
        if met.is_empty() {
            return None;
        }
        let named: Vec<String> = met
            .iter()
            .take(3)
            .map(|e| match e.count.get() {
                1 => e.construct.label().to_owned(),
                n => format!("{} ({n})", e.construct.label()),
            })
            .collect();
        let rest = met.len().saturating_sub(named.len());
        let list = if rest == 0 {
            named.join(", ")
        } else {
            format!("{}, and {rest} more", named.join(", "))
        };
        Some(format!("{verb}: {list}"))
    });
    let mut parts: Vec<String> = head.into_iter().chain(losses).collect();
    let first = parts.first_mut()?;
    if let Some(initial) = first.get(..1) {
        let capital = initial.to_ascii_uppercase();
        first.replace_range(..1, &capital);
    }
    Some(parts.join("; "))
}

/// `PROSE-IO-074`: each fixture reads as the dump a person accepted. Set
/// `AMBRE_ACCEPT_DOCX=1` to write a missing or changed dump; a reviewer then
/// reads the diff of the `.expected` file.
#[test]
fn every_fixture_reads_as_its_expected_dump() {
    let accept = std::env::var_os("AMBRE_ACCEPT_DOCX").is_some();
    let mut stale = Vec::new();
    for name in FIXTURES {
        let decoded = read(&fixture(name)).expect(name);
        let actual = format!(
            "{}report {:?}\n",
            dump(&decoded.document),
            sentence(&decoded.report)
        );
        let path = format!("{DIR}/{}.expected", name.trim_end_matches(".docx"));
        let expected = std::fs::read_to_string(&path).unwrap_or_default();
        if expected != actual {
            if accept {
                std::fs::write(&path, &actual).expect("writes the dump");
            } else {
                stale.push(name);
            }
        }
    }
    assert!(stale.is_empty(), "dumps differ: {stale:?}");
}

/// `PROSE-IO-070`: a tracked insertion and a tracked deletion read as
/// revisions with their author and their date, never as plain text.
#[test]
fn a_word_tracked_change_reads_as_a_revision_with_its_author() {
    let inserted = revisions(
        &read(&fixture("pandoc/track_changes_insertion.docx"))
            .expect("reads")
            .document,
    );
    let [(revision, text)] = inserted.as_slice() else {
        panic!("one inserted run, got {}", inserted.len());
    };
    assert_eq!(revision.kind, RevisionKind::Insertion);
    assert_eq!(revision.author, "eng-dept");
    assert!(revision.date.is_some());
    assert_eq!(text, "two exciting ");

    let deleted = read(&fixture("pandoc/track_changes_deletion.docx")).expect("reads");
    let revisions = revisions(&deleted.document);
    assert!(
        revisions
            .iter()
            .any(|(r, t)| r.kind == RevisionKind::Deletion && t.contains("excessively"))
    );
    assert_eq!(
        deleted
            .report
            .count(WordConstruct::TrackedChange, Outcome::Carried)
            .get(),
        1
    );
}

/// `PROSE-IO-074`: a tracked move is named as approximated: it reads as a
/// deletion and an insertion.
#[test]
fn a_tracked_move_is_named_as_approximated() {
    let decoded = read(&fixture("pandoc/track_changes_move.docx")).expect("reads");
    let kinds: Vec<_> = revisions(&decoded.document)
        .into_iter()
        .map(|(r, _)| r.kind)
        .collect();
    assert!(kinds.contains(&RevisionKind::Insertion) && kinds.contains(&RevisionKind::Deletion));
    assert_eq!(
        decoded
            .report
            .count(WordConstruct::TrackedMove, Outcome::Approximated)
            .get(),
        2
    );
}

/// `PROSE-IO-070`: comments read with their author, their body and their
/// anchor, one across two paragraphs and one inside another included.
#[test]
fn a_word_comment_reads_with_its_author_body_and_anchor() {
    let document = read(&fixture("pandoc/comments.docx"))
        .expect("reads")
        .document;
    assert_eq!(document.comments.len(), 5);
    assert!(
        document
            .comments
            .iter()
            .all(|c| c.author == "Jesse Rosenthal" && c.date.is_some())
    );
    assert_eq!(
        text(&document.comments[0].paragraphs[0]),
        "I left a comment."
    );
    assert_eq!(document.comments[2].paragraphs.len(), 3);
    let body = paragraphs(&document.body);
    let across = document.comments[1].id;
    assert!(body[1].inlines.contains(&Inline::CommentStart(across)));
    assert!(body[2].inlines.contains(&Inline::CommentEnd(across)));

    let other = read(&fixture("open-xml-sdk/Comment004.docx"))
        .expect("reads")
        .document;
    let [comment] = other.comments.as_slice() else {
        panic!("one comment");
    };
    assert_eq!(text(&comment.paragraphs[0]), "Test comment");
    assert!(
        paragraphs(&other.body)[0]
            .inlines
            .contains(&Inline::CommentStart(comment.id))
    );
}

/// `PROSE-IO-070`: LibreOffice's comment and tracked changes read as Word's.
/// The fixture's source, `libreoffice/features.fodt`, states each construct.
#[test]
fn a_libreoffice_file_reads_every_construct_it_states() {
    let decoded = read(&fixture("libreoffice/features.docx")).expect("reads");
    let document = &decoded.document;
    assert_eq!(document.title.as_deref(), Some("Ambre fixture"));
    let body = paragraphs(&document.body);
    assert!(matches!(body[0].role, ParagraphRole::Heading(level) if level.get() == 1));
    let styled = runs(body[1]);
    assert!(
        styled
            .iter()
            .any(|r| r.text == "bold" && r.format.bold.as_bool())
    );
    let link = body[1]
        .inlines
        .iter()
        .find_map(|i| match i {
            Inline::Link(link) => Some(link),
            _ => None,
        })
        .expect("a link");
    assert!(
        matches!(&link.target, LinkTarget::External(url) if url.as_str() == "https://example.org/page")
    );
    // The link style draws the link and stays off its runs.
    assert!(!link.runs[0].format.underline.as_bool());
    let [comment] = document.comments.as_slice() else {
        panic!("one comment");
    };
    assert_eq!(comment.author, "Carol Petit");
    let authors: Vec<_> = revisions(document)
        .into_iter()
        .map(|(r, t)| (r.kind, r.author, t))
        .collect();
    assert!(authors.contains(&(
        RevisionKind::Insertion,
        "Alice Martin".to_owned(),
        "inserted words".to_owned()
    )));
    assert!(authors.contains(&(
        RevisionKind::Deletion,
        "Bob Durand".to_owned(),
        "removed words".to_owned()
    )));
}

/// A package in the shape Word writes a reply and a resolved thread. Each
/// comment's last paragraph carries a `w14:paraId`. `commentsExtended.xml`
/// keys the parent and the state by it.
fn thread_package() -> Vec<u8> {
    const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
                     xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" \
                     xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\"";
    let body = "<w:p><w:commentRangeStart w:id=\"0\"/><w:commentRangeStart w:id=\"1\"/>\
                <w:r><w:t>anchored</w:t></w:r><w:commentRangeEnd w:id=\"0\"/>\
                <w:commentRangeEnd w:id=\"1\"/></w:p>";
    let comments = format!(
        "<w:comments {W}>\
         <w:comment w:id=\"0\" w:author=\"Ann\" w:date=\"2024-01-02T03:04:05Z\">\
         <w:p w14:paraId=\"1A2B3C4D\"><w:r><w:t>Why?</w:t></w:r></w:p></w:comment>\
         <w:comment w:id=\"1\" w:author=\"Ben\" w:date=\"2024-01-02T04:00:00Z\">\
         <w:p w14:paraId=\"0F00BA44\"><w:r><w:t>Because.</w:t></w:r></w:p></w:comment>\
         </w:comments>"
    );
    let extended = format!(
        "<w15:commentsEx {W}>\
         <w15:commentEx w15:paraId=\"1A2B3C4D\" w15:done=\"1\"/>\
         <w15:commentEx w15:paraId=\"0F00BA44\" w15:paraIdParent=\"1A2B3C4D\" w15:done=\"0\"/>\
         </w15:commentsEx>"
    );
    package_with(body, |writer| {
        let main = part("word/document.xml");
        for (kind, name, bytes) in [
            (rel::COMMENTS, "word/comments.xml", comments),
            (
                rel::COMMENTS_EXTENDED,
                "word/commentsExtended.xml",
                extended,
            ),
        ] {
            writer.relate(&main, kind, RelTarget::Part(part(name)));
            writer.part(part(name), None, Payload::new(bytes.into_bytes()));
        }
    })
}

/// `PROSE-IO-070`: a reply names its parent and a thread keeps its state,
/// read from `commentsExtended.xml`.
#[test]
fn a_reply_and_a_resolved_thread_read_from_the_extended_part() {
    let document = read(&thread_package()).expect("reads").document;
    let [first, reply] = document.comments.as_slice() else {
        panic!("two comments");
    };
    assert_eq!(first.state, CommentState::Resolved);
    assert_eq!(first.parent, None);
    assert_eq!(reply.parent, Some(first.id));
    assert_eq!(reply.state, CommentState::Open);
}

/// `PROSE-IO-073`: a reply and a resolved thread survive the round trip.
#[test]
fn a_reply_and_a_resolved_thread_survive_the_round_trip() {
    let document = read(&thread_package()).expect("reads").document;
    let again = read(write(&document).expect("writes").as_ref())
        .expect("reads")
        .document;
    assert_eq!(again, document.renumbered());
    assert_eq!(again.comments[1].parent, Some(again.comments[0].id));
}

#[test]
fn word_lists_read_their_levels_and_formats() {
    let document = read(&fixture("pandoc/lists.docx")).expect("reads").document;
    let items: Vec<_> = paragraphs(&document.body)
        .iter()
        .filter_map(|p| p.list)
        .map(|item| (item.level.get(), item.kind))
        .collect();
    assert!(items.contains(&(0, ListKind::Numbered(NumberFormat::Decimal))));
    assert!(items.contains(&(1, ListKind::Numbered(NumberFormat::LowerLetter))));
    assert!(items.contains(&(2, ListKind::Bullet)));
    // A list that starts again is another list.
    let lists: std::collections::BTreeSet<_> = paragraphs(&document.body)
        .iter()
        .filter_map(|p| p.list.map(|l| l.list))
        .collect();
    assert!(lists.len() > 1);
}

/// A part in the strict namespaces of ISO/IEC 29500 reads as a transitional
/// one.
#[test]
fn a_strict_package_reads_as_a_transitional_one() {
    let document = read(&fixture("open-xml-sdk/strict-numbering.docx"))
        .expect("reads")
        .document;
    assert!(paragraphs(&document.body).iter().any(|p| p.list.is_some()));
}

#[test]
fn word_notes_read_with_their_bodies() {
    let document = read(&fixture("pandoc/notes.docx")).expect("reads").document;
    let calls: Vec<_> = paragraphs(&document.body)
        .iter()
        .flat_map(|p| &p.inlines)
        .filter_map(|i| match i {
            Inline::NoteCall(kind, id) => Some((*kind, *id)),
            _ => None,
        })
        .collect();
    assert!(calls.contains(&(NoteKind::Footnote, document.footnotes[0].id)));
    assert!(calls.contains(&(NoteKind::Endnote, document.endnotes[0].id)));
    assert_eq!(
        text(paragraphs(&document.footnotes[0].blocks)[0]),
        " My note."
    );
}

#[test]
fn word_formatting_headings_tables_and_pictures_read() {
    let inline = read(&fixture("pandoc/inline_formatting.docx"))
        .expect("reads")
        .document;
    let all: Vec<_> = paragraphs(&inline.body)
        .into_iter()
        .flat_map(runs)
        .collect();
    assert!(
        all.iter()
            .any(|r| r.text == "italics" && r.format.italic.as_bool())
    );
    assert!(
        all.iter()
            .any(|r| r.text == "strikethrough" && r.format.strike.as_bool())
    );
    assert!(all.iter().any(|r| r.format.underline.as_bool()));
    assert!(
        all.iter()
            .any(|r| r.format.position == VerticalPosition::Superscript)
    );
    assert!(
        all.iter()
            .any(|r| r.format.position == VerticalPosition::Subscript)
    );

    let headers = read(&fixture("pandoc/headers.docx"))
        .expect("reads")
        .document;
    let levels: Vec<_> = paragraphs(&headers.body)
        .iter()
        .filter_map(|p| match p.role {
            ParagraphRole::Heading(level) => Some(level.get()),
            _ => None,
        })
        .collect();
    assert_eq!(levels[..6], [1, 2, 3, 4, 5, 6]);

    let tables = read(&fixture("pandoc/tables.docx"))
        .expect("reads")
        .document;
    let Some(Block::Table(table)) = tables.body.iter().find(|b| matches!(b, Block::Table(_)))
    else {
        panic!("a table");
    };
    assert_eq!(table.rows.len(), 4);
    assert!(table.rows.iter().all(|row| row.cells.len() == 4));

    let picture = read(&fixture("pandoc/image.docx")).expect("reads").document;
    let image = paragraphs(&picture.body)
        .iter()
        .flat_map(|p| &p.inlines)
        .find_map(|i| match i {
            Inline::Image(image) => Some(image.clone()),
            _ => None,
        })
        .expect("a picture");
    assert!(!image.bytes.as_ref().is_empty());
    assert!(image.width.get() > 0.0 && !image.description.is_empty());

    let code = read(&fixture("pandoc/codeblock.docx"))
        .expect("reads")
        .document;
    assert!(
        paragraphs(&code.body)
            .iter()
            .any(|p| p.role == ParagraphRole::Code)
    );
    let quotes = read(&fixture("pandoc/block_quotes.docx"))
        .expect("reads")
        .document;
    assert_eq!(
        paragraphs(&quotes.body)
            .iter()
            .filter(|p| p.role == ParagraphRole::Quote)
            .count(),
        2
    );
}

/// `PROSE-IO-073`: every structure a file carries survives the codec's own
/// round trip, ids numbered again.
#[test]
fn every_fixture_survives_a_round_trip() {
    for name in FIXTURES {
        let document = read(&fixture(name)).expect(name).document;
        let bytes = write(&document).expect(name);
        let again = read(bytes.as_ref()).expect(name).document;
        assert_eq!(again, document.renumbered(), "{name}");
    }
}

/// `PROSE-IO-076`: the same document writes the same bytes.
#[test]
fn a_document_writes_the_same_bytes_twice() {
    for name in FIXTURES {
        let document = read(&fixture(name)).expect(name).document;
        assert_eq!(
            write(&document).expect(name),
            write(&document).expect(name),
            "{name}"
        );
    }
}

fn part(name: &str) -> PartName {
    PartName::new(name).expect("a valid name")
}

/// A package holding a minimal document, and the parts `extra` adds.
fn package(extra: impl FnOnce(&mut PackageWriter)) -> Vec<u8> {
    package_with("<w:p><w:r><w:t>hi</w:t></w:r></w:p>", extra)
}

/// A package whose body holds `body`, and the parts `extra` adds.
fn package_with(body: &str, extra: impl FnOnce(&mut PackageWriter)) -> Vec<u8> {
    package_of(
        &format!(
            "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
             xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
             xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
             xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
             xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\" \
             xmlns:w16du=\"http://schemas.microsoft.com/office/word/2023/wordml/word16du\">\
             <w:body>{body}</w:body></w:document>"
        ),
        extra,
    )
}

/// A package whose main part holds `document`, and the parts `extra` adds.
fn package_of(document: &str, extra: impl FnOnce(&mut PackageWriter)) -> Vec<u8> {
    let mut writer = PackageWriter::new();
    let main = part("word/document.xml");
    writer.relate(
        &ambre_docx::opc::root(),
        RelKind::OFFICE_DOCUMENT,
        RelTarget::Part(main.clone()),
    );
    writer.part(
        main,
        Some("application/vnd.ms-word.document.macroEnabled.main+xml"),
        Payload::new(document.as_bytes().to_vec()),
    );
    extra(&mut writer);
    writer.finish().expect("writes").as_ref().to_vec()
}

/// `PROSE-IO-072`: a part declaring an external entity is refused whole;
/// the reader never resolves the entity.
#[test]
fn an_external_entity_is_refused() {
    let bytes = package_of(
        "<?xml version=\"1.0\"?><!DOCTYPE d [<!ENTITY x SYSTEM \"file:///etc/passwd\">]>\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
         <w:body><w:p><w:r><w:t>&x;</w:t></w:r></w:p></w:body></w:document>",
        |_| {},
    );
    assert_eq!(
        read(&bytes).err(),
        Some(DocxError::Xml(XmlError::DocumentType))
    );
}

/// `PROSE-IO-072`: a table nested past any editor fails typed, and the
/// stack holds.
#[test]
fn a_hostile_nesting_fails_typed() {
    let depth = 2000;
    let body = format!(
        "{}<w:p/>{}",
        "<w:tbl><w:tr><w:tc>".repeat(depth),
        "</w:tc></w:tr></w:tbl>".repeat(depth)
    );
    assert_eq!(
        read(&package_with(&body, |_| {})).err(),
        Some(DocxError::Xml(XmlError::TooDeep))
    );
}

/// `PROSE-IO-072`: every prefix of a real file fails typed or reads.
#[test]
fn a_truncated_file_fails_typed() {
    let bytes = fixture("pandoc/comments.docx");
    for len in (0..bytes.len()).step_by(97) {
        let _ = read(&bytes[..len]);
    }
    assert!(matches!(
        read(&bytes[..bytes.len() / 2]),
        Err(DocxError::Package(_))
    ));
}

/// `PROSE-IO-072`: flipped bytes anywhere in a real file never panic.
#[test]
fn a_corrupted_file_never_panics() {
    let bytes = fixture("libreoffice/features.docx");
    for at in (0..bytes.len()).step_by(29) {
        let mut corrupted = bytes.clone();
        corrupted[at] ^= 0xA5;
        let _ = read(&corrupted);
    }
}

/// `PROSE-IO-071`: a macro project is named in the report and never read.
/// The project here inflates past the bound of a part, so a reader that
/// read it would fail.
#[test]
fn a_macro_project_is_named_and_never_read() {
    let bytes = package(|writer| {
        let size = usize::try_from(ambre_docx::opc::MAX_PART_SIZE.get()).expect("fits") + 1;
        let project = part("word/vbaProject.bin");
        writer.relate(
            &part("word/document.xml"),
            rel::VBA_PROJECT,
            RelTarget::Part(project.clone()),
        );
        writer.part(
            project,
            Some("application/vnd.ms-office.vbaProject"),
            Payload::new(vec![0; size]),
        );
    });
    let decoded = read(&bytes).expect("reads the text");
    assert_eq!(text(paragraphs(&decoded.document.body)[0]), "hi");
    assert_eq!(
        decoded
            .report
            .count(WordConstruct::Macro, Outcome::Dropped)
            .get(),
        1
    );
}

/// `PROSE-IO-072`: a part that inflates past its bound fails typed.
#[test]
fn a_zip_bomb_fails_typed() {
    let bytes = package(|writer| {
        let size = usize::try_from(ambre_docx::opc::MAX_PART_SIZE.get()).expect("fits") + 1;
        writer.relate(
            &part("word/document.xml"),
            rel::STYLES,
            RelTarget::Part(part("word/styles.xml")),
        );
        writer.part(
            part("word/styles.xml"),
            None,
            Payload::new(vec![b' '; size]),
        );
    });
    assert!(bytes.len() < 1024 * 1024);
    assert!(matches!(
        read(&bytes),
        Err(DocxError::Package(OpcError::TooLarge(_)))
    ));
}

#[test]
fn a_package_of_another_format_is_refused() {
    let mut writer = PackageWriter::new();
    let main = part("xl/workbook.xml");
    writer.relate(
        &ambre_docx::opc::root(),
        RelKind::OFFICE_DOCUMENT,
        RelTarget::Part(main.clone()),
    );
    writer.part(
        main,
        Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"),
        Payload::new(b"<workbook/>".to_vec()),
    );
    let bytes = writer.finish().expect("writes");
    assert_eq!(read(bytes.as_ref()).err(), Some(DocxError::NotWord));
}

/// `PROSE-IO-075`: a picture linked from outside the package is reported,
/// and its address is never fetched.
#[test]
fn a_linked_picture_is_reported_and_never_fetched() {
    let body = r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="12700" cy="12700"/><wp:docPr id="1" name="p"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:link="rId1"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r><w:hyperlink r:id="rId1"><w:r><w:t>away</w:t></w:r></w:hyperlink></w:p>"#;
    let bytes = package_with(body, |writer| {
        // The first relationship of the main part takes the id `rId1`.
        writer.relate(
            &part("word/document.xml"),
            rel::IMAGE,
            RelTarget::External("https://tracker.example/pixel.png".to_owned()),
        );
    });
    let decoded = read(&bytes).expect("reads");
    assert_eq!(
        decoded
            .report
            .count(WordConstruct::LinkedPicture, Outcome::Dropped)
            .get(),
        1
    );
    let inlines = &paragraphs(&decoded.document.body)[0].inlines;
    assert!(!inlines.iter().any(|i| matches!(i, Inline::Image(_))));
    // The link keeps the address as text, for the user to open or not.
    assert!(inlines.iter().any(|i| matches!(
        i,
        Inline::Link(link) if matches!(&link.target, LinkTarget::External(url) if url.as_str() == "https://tracker.example/pixel.png")
    )));
}

/// `PROSE-IO-077`: an element the reader does not recognise is reported,
/// and the text under it is kept.
#[test]
fn an_unknown_element_is_reported_and_its_text_kept() {
    let body = "<w:p><w:r><w:t>a</w:t></w:r><w:futureThing><w:r><w:t>kept</w:t></w:r></w:futureThing></w:p>";
    let decoded = read(&package_with(body, |_| {})).expect("reads");
    assert_eq!(text(paragraphs(&decoded.document.body)[0]), "akept");
    assert_eq!(
        decoded
            .report
            .count(WordConstruct::UnknownElement, Outcome::Dropped)
            .get(),
        1
    );
}

/// `PROSE-IO-086`: a run effect the model has no form for is named in the
/// report, never dropped in silence: small capitals and all capitals are
/// dropped, hidden text and a double strikethrough approximated.
#[test]
fn a_run_effect_the_model_lacks_is_named_in_the_report() {
    let run = |effect: &str, text: &str| {
        format!("<w:r><w:rPr><w:{effect}/></w:rPr><w:t>{text}</w:t></w:r>")
    };
    let body = format!(
        "<w:p>{}{}{}{}</w:p>",
        run("smallCaps", "a"),
        run("caps", "b"),
        run("vanish", "c"),
        run("dstrike", "d")
    );
    let decoded = read(&package_with(&body, |_| {})).expect("reads");
    assert_eq!(text(paragraphs(&decoded.document.body)[0]), "abcd");
    let count = |construct, outcome| decoded.report.count(construct, outcome).get();
    assert_eq!(count(WordConstruct::SmallCaps, Outcome::Dropped), 1);
    assert_eq!(count(WordConstruct::AllCaps, Outcome::Dropped), 1);
    assert_eq!(count(WordConstruct::HiddenText, Outcome::Approximated), 1);
    assert_eq!(count(WordConstruct::DoubleStrike, Outcome::Approximated), 1);
}

/// `PROSE-IO-084`: a tracked change that carries `w16du:dateUtc` reads its
/// true instant beside the wall time of `w:date`. One with `w:date` alone
/// reads a wall time with no zone.
#[test]
fn a_date_reads_its_utc_instant_when_the_part_gives_one() {
    let body = "<w:p><w:ins w:id=\"1\" w:author=\"A\" w:date=\"2016-05-09T16:13:00Z\" \
                w16du:dateUtc=\"2016-05-09T14:13:00Z\"><w:r><w:t>new</w:t></w:r></w:ins>\
                <w:del w:id=\"2\" w:author=\"B\" w:date=\"2016-05-09T16:13:00Z\">\
                <w:r><w:delText>old</w:delText></w:r></w:del></w:p>";
    let decoded = read(&package_with(body, |_| {})).expect("reads");
    let dates: Vec<_> = revisions(&decoded.document)
        .into_iter()
        .map(|(revision, _)| revision.date)
        .collect();
    let wall = ambre_core::Instant::new(1_462_810_380_000);
    assert_eq!(
        dates,
        [
            Some(WordDate::Zoned {
                utc: ambre_core::Instant::new(1_462_803_180_000),
                wall,
            }),
            Some(WordDate::Floating(wall)),
        ]
    );
}
