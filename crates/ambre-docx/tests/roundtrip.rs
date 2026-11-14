//! The round trip over generated models: what the writer writes, the reader
//! reads back, ids numbered again (`PROSE-IO-073`).
//!
//! The oracle is the model itself: a document written then read equals the
//! document as `renumbered` numbers it, and nothing else.

#![cfg(not(target_arch = "wasm32"))]
#![expect(
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_core::Flag as _;
use ambre_core::{Bounded, FontSize, LinkUrl, Payload, PrintLength, Pt, Rgb};
use ambre_docx::model::{
    Alignment, Block, BreakKind, Comment, CommentState, GridSpan, HeadingDepth, Image, ImageFormat,
    Inline, Link, LinkTarget, ListDepth, ListKind, ListMembership, Margins, Note, NoteKind,
    NumberFormat, Orientation, PageSetup, Paragraph, ParagraphRole, Revision, RevisionKind,
    RowRole, Run, RunFormat, Table, TableCell, TableRow, VerticalMerge, VerticalPosition,
    WordDocument, WordId,
};
use ambre_docx::{read, write};
use proptest::collection::vec;
use proptest::option;
use proptest::prelude::*;

/// Text a person types, XML's special characters and two scripts included.
/// A tab and a line end are inlines of their own, so no run holds one.
fn words() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 &<>\"'.,\u{e9}\u{4e2d}\u{1F600}]{1,12}"
}

fn author() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["Alice Martin", "Bob", "Zo\u{e9} <z@x>", ""]).prop_map(str::to_owned)
}

/// A date in whole seconds, as Word writes one: a wall time alone, or the
/// true instant beside it.
fn date() -> impl Strategy<Value = Option<ambre_docx::model::WordDate>> {
    let second = || (0_u64..4_000_000_000).prop_map(|s| ambre_core::Instant::new(s * 1000));
    option::of(prop_oneof![
        second().prop_map(ambre_docx::model::WordDate::Floating),
        (second(), second())
            .prop_map(|(utc, wall)| ambre_docx::model::WordDate::Zoned { utc, wall }),
    ])
}

/// A length a file spells in twips.
fn twips() -> impl Strategy<Value = PrintLength<Pt>> {
    (0_i32..20_000).prop_map(|t| PrintLength::new(t as f32 / 20.0))
}

fn color() -> impl Strategy<Value = Option<Rgb>> {
    option::of((0_u32..0x100_0000).prop_map(Rgb::from_bits))
}

fn format() -> impl Strategy<Value = RunFormat> {
    (
        (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()),
        prop::sample::select(vec![
            VerticalPosition::Baseline,
            VerticalPosition::Superscript,
            VerticalPosition::Subscript,
        ]),
        option::of((2_u16..400).prop_map(|h| FontSize::new(f32::from(h) / 2.0).expect("a size"))),
        color(),
        color(),
        option::of(prop::sample::select(vec![
            "Arial",
            "Courier New",
            "Noto Serif",
        ])),
    )
        .prop_map(
            |((b, i, u, s), position, size, color, fill, font)| RunFormat {
                bold: ambre_core::Weight::from_bool(b),
                italic: ambre_core::Slant::from_bool(i),
                underline: ambre_core::Underline::from_bool(u),
                strike: ambre_core::Strikethrough::from_bool(s),
                position,
                size,
                color,
                fill,
                font: font.map(str::to_owned),
            },
        )
}

fn run() -> impl Strategy<Value = Run> {
    (
        words(),
        format(),
        option::of((
            prop::sample::select(vec![RevisionKind::Insertion, RevisionKind::Deletion]),
            0_u32..50,
            author(),
            date(),
        )),
    )
        .prop_map(|(text, format, revision)| Run {
            text,
            format,
            revision: revision.map(|(kind, id, author, date)| Revision {
                kind,
                id: WordId::new(id),
                author,
                date,
            }),
        })
}

fn image() -> impl Strategy<Value = Image> {
    (
        "[a-z ]{0,10}",
        1_u16..600,
        1_u16..600,
        prop::sample::select(vec![ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Gif]),
        vec(any::<u8>(), 0..64),
    )
        .prop_map(|(description, w, h, format, bytes)| Image {
            description: ambre_core::Validated::new(description),
            width: PrintLength::new(f32::from(w)),
            height: PrintLength::new(f32::from(h)),
            format,
            bytes: Payload::new(bytes),
        })
}

fn inline() -> impl Strategy<Value = Inline> {
    prop_oneof![
        6 => run().prop_map(Inline::Run),
        1 => prop::sample::select(vec![BreakKind::Line, BreakKind::Page, BreakKind::Column])
            .prop_map(Inline::Break),
        1 => Just(Inline::Tab),
        1 => (
            prop_oneof![
                prop::sample::select(vec!["https://example.org/a?b=1&c=2", "mailto:x@y.z"])
                    .prop_map(|url| LinkTarget::External(LinkUrl::new(url.to_owned()))),
                "[A-Za-z_]{1,8}".prop_map(LinkTarget::Anchor),
            ],
            vec(run(), 1..3),
        )
            .prop_map(|(target, runs)| Inline::Link(Link { target, runs })),
        1 => image().prop_map(Inline::Image),
    ]
}

fn role() -> impl Strategy<Value = ParagraphRole> {
    prop_oneof![
        4 => Just(ParagraphRole::Body),
        2 => (1_u8..=9).prop_map(|l| ParagraphRole::Heading(
            Bounded::<HeadingDepth>::new(l).expect("a level")
        )),
        1 => Just(ParagraphRole::Title),
        1 => Just(ParagraphRole::Quote),
        1 => Just(ParagraphRole::Code),
    ]
}

/// The kind a list shows at a level. A list keeps one kind per level, as a
/// Word list definition does.
fn list_kind(list: u32, level: u8) -> ListKind {
    match (list + u32::from(level)) % 6 {
        0 => ListKind::Bullet,
        1 => ListKind::Numbered(NumberFormat::Decimal),
        2 => ListKind::Numbered(NumberFormat::LowerLetter),
        3 => ListKind::Numbered(NumberFormat::UpperLetter),
        4 => ListKind::Numbered(NumberFormat::LowerRoman),
        _ => ListKind::Numbered(NumberFormat::UpperRoman),
    }
}

fn paragraph() -> impl Strategy<Value = Paragraph> {
    (
        role(),
        option::of((1_u32..5, 0_u8..=8)),
        option::of(prop::sample::select(vec![
            Alignment::Start,
            Alignment::Center,
            Alignment::End,
            Alignment::Justified,
        ])),
        vec(inline(), 0..5),
    )
        .prop_map(|(role, list, align, inlines)| Paragraph {
            role,
            list: list.map(|(list, level)| ListMembership {
                list: WordId::new(list),
                level: Bounded::<ListDepth>::new(level).expect("a level"),
                kind: list_kind(list, level),
            }),
            align,
            inlines,
        })
}

fn table(cell: BoxedStrategy<Vec<Block>>) -> impl Strategy<Value = Table> {
    (
        vec(twips(), 0..4),
        vec(
            (
                prop::sample::select(vec![RowRole::Body, RowRole::Header]),
                vec(
                    (
                        1_u8..4,
                        prop::sample::select(vec![
                            VerticalMerge::None,
                            VerticalMerge::Restart,
                            VerticalMerge::Continue,
                        ]),
                        color(),
                        cell,
                    ),
                    1..3,
                ),
            ),
            1..3,
        ),
    )
        .prop_map(|(columns, rows)| Table {
            columns,
            rows: rows
                .into_iter()
                .map(|(role, cells)| TableRow {
                    role,
                    cells: cells
                        .into_iter()
                        .map(|(span, merge, fill, blocks)| TableCell {
                            span: Bounded::<GridSpan>::new(span).expect("a span"),
                            merge,
                            fill,
                            blocks,
                        })
                        .collect(),
                })
                .collect(),
        })
}

/// Blocks a cell, a note or a band holds: a paragraph last, as Word
/// requires of a cell.
fn cell_blocks() -> BoxedStrategy<Vec<Block>> {
    (vec(paragraph(), 0..2), paragraph())
        .prop_map(|(mut first, last)| {
            first.push(last);
            first.into_iter().map(Block::Paragraph).collect()
        })
        .boxed()
}

fn blocks() -> impl Strategy<Value = Vec<Block>> {
    vec(
        prop_oneof![
            4 => paragraph().prop_map(Block::Paragraph),
            1 => table(cell_blocks()).prop_map(Block::Table),
        ],
        1..6,
    )
}

fn page() -> impl Strategy<Value = Option<PageSetup>> {
    option::of(
        (
            twips(),
            twips(),
            (twips(), twips(), twips(), twips()),
            prop::sample::select(vec![Orientation::Portrait, Orientation::Landscape]),
        )
            .prop_map(|(width, height, (top, bottom, left, right), orientation)| {
                PageSetup {
                    width,
                    height,
                    margins: Margins {
                        top,
                        bottom,
                        left,
                        right,
                    },
                    orientation,
                }
            }),
    )
}

fn comment_paragraph() -> impl Strategy<Value = Paragraph> {
    paragraph()
}

fn document() -> impl Strategy<Value = WordDocument> {
    (
        option::of("[A-Za-z][A-Za-z ]{0,10}"),
        page(),
        blocks(),
        option::of(cell_blocks()),
        option::of(cell_blocks()),
        vec((1_u32..1000, cell_blocks()), 0..3),
        vec((1_u32..1000, cell_blocks()), 0..3),
        vec(
            (
                author(),
                date(),
                vec(comment_paragraph(), 1..3),
                any::<bool>(),
                any::<bool>(),
            ),
            0..4,
        ),
    )
        .prop_map(
            |(title, page, mut body, header, footer, footnotes, endnotes, comments)| {
                let footnotes = notes(footnotes);
                let endnotes = notes(endnotes);
                let comments: Vec<Comment> = comments
                    .into_iter()
                    .enumerate()
                    .map(
                        |(at, (author, date, paragraphs, reply, resolved))| Comment {
                            id: WordId::new(u32::try_from(at).expect("small") * 3 + 2),
                            author,
                            date,
                            paragraphs,
                            parent: (reply && at > 0).then(|| WordId::new(2)),
                            state: if resolved {
                                CommentState::Resolved
                            } else {
                                CommentState::Open
                            },
                        },
                    )
                    .collect();
                // Each note gets its call and each comment its anchor, in a
                // body paragraph.
                let mut calls: Vec<(Inline, Option<Inline>)> = footnotes
                    .iter()
                    .map(|n| (Inline::NoteCall(NoteKind::Footnote, n.id), None))
                    .chain(
                        endnotes
                            .iter()
                            .map(|n| (Inline::NoteCall(NoteKind::Endnote, n.id), None)),
                    )
                    .chain(
                        comments
                            .iter()
                            .map(|c| (Inline::CommentStart(c.id), Some(Inline::CommentEnd(c.id)))),
                    )
                    .collect();
                let mut at = 0;
                for block in &mut body {
                    if let Block::Paragraph(paragraph) = block {
                        for (open, close) in calls.drain(..) {
                            paragraph
                                .inlines
                                .insert(at.min(paragraph.inlines.len()), open);
                            if let Some(close) = close {
                                paragraph.inlines.push(close);
                            }
                            at += 1;
                        }
                    }
                }
                if !calls.is_empty() {
                    let mut paragraph = Paragraph::default();
                    for (open, close) in calls {
                        paragraph.inlines.push(open);
                        paragraph.inlines.extend(close);
                    }
                    body.push(Block::Paragraph(paragraph));
                }
                WordDocument {
                    title,
                    page,
                    body,
                    header,
                    footer,
                    footnotes,
                    endnotes,
                    comments,
                }
            },
        )
}

/// Notes with distinct ids.
fn notes(notes: Vec<(u32, Vec<Block>)>) -> Vec<Note> {
    let mut seen = std::collections::BTreeSet::new();
    notes
        .into_iter()
        .filter(|(id, _)| seen.insert(*id))
        .map(|(id, blocks)| Note {
            id: WordId::new(id),
            blocks,
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// `PROSE-IO-073`: a document written then read is the document, ids
    /// numbered again.
    #[test]
    fn a_written_document_reads_back_whole(document in document()) {
        let bytes = write(&document).expect("writes");
        let decoded = read(bytes.as_ref()).expect("reads");
        prop_assert_eq!(decoded.document, document.renumbered());
        // The reader names a loss of nothing the writer writes.
        let carried_alone = decoded
            .report
            .entries()
            .iter()
            .all(|entry| entry.outcome == ambre_docx::Outcome::Carried);
        prop_assert!(carried_alone);
    }

    /// Numbering twice changes nothing, so the writer's output is fixed by
    /// the document alone.
    #[test]
    fn renumbering_is_idempotent(document in document()) {
        let once = document.renumbered();
        prop_assert_eq!(once.renumbered(), once.clone());
        prop_assert_eq!(write(&document).expect("writes"), write(&once).expect("writes"));
    }
}

/// A novel of 300 pages: 300 chapters' worth of paragraphs of some 400
/// words a page, with a heading every ten pages.
fn novel() -> WordDocument {
    let sentence = "The rain had stopped before the train reached the coast, and nobody spoke. ";
    let mut body = Vec::new();
    for page in 0..300_u32 {
        if page % 10 == 0 {
            body.push(Block::Paragraph(Paragraph {
                role: ParagraphRole::Heading(Bounded::<HeadingDepth>::MIN),
                inlines: vec![Inline::Run(Run::plain(format!(
                    "Chapter {}",
                    page / 10 + 1
                )))],
                ..Paragraph::default()
            }));
        }
        for _ in 0..4 {
            body.push(Block::Paragraph(Paragraph {
                inlines: vec![Inline::Run(Run::plain(sentence.repeat(7)))],
                ..Paragraph::default()
            }));
        }
    }
    WordDocument {
        body,
        ..WordDocument::default()
    }
}

/// A 300-page novel imports whole, under every bound of the package layer.
#[test]
fn a_three_hundred_page_novel_reads_whole() {
    let document = novel();
    let bytes = write(&document).expect("writes");
    let started = std::time::Instant::now();
    let decoded = read(bytes.as_ref()).expect("reads under the bounds");
    let elapsed = started.elapsed();
    assert_eq!(decoded.document, document.renumbered());
    // A debug build on the host; the bench measures the release figure.
    assert!(elapsed.as_secs() < 30, "{elapsed:?}");
}
