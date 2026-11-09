//! Criterion benchmark for the Word codec, read and write.
//!
//! Target from `ambre-spec-budgets.md`, the "Prose DOCX import / export"
//! row: 100 pages read under 500 ms and written under 300 ms on the Logic
//! thread. The bench builds the document: 100 pages of 500 words. Each
//! page holds a list item, a table, a comment and a tracked change. Every
//! fifth page opens on a heading. So every path of the codec runs.

#![expect(clippy::panic, reason = "a bench aborts on a document it cannot run")]

use std::hint::black_box;

use ambre_core::{Bounded, Instant, Weight};
use ambre_docx::model::{
    Block, Comment, CommentState, Inline, ListDepth, ListKind, ListMembership, Paragraph,
    ParagraphRole, Revision, RevisionKind, Run, Table, TableCell, TableRow, WordDocument, WordId,
};
use criterion::{Criterion, criterion_group, criterion_main};

const PAGES: u32 = 100;
const WORDS: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do ";

/// One page: paragraphs and a list item of some 500 words, a small table,
/// a comment and an insertion. Every fifth page opens on a heading.
fn page(number: u32, document: &mut WordDocument) {
    let paragraph = |inlines| {
        Block::Paragraph(Paragraph {
            inlines,
            ..Paragraph::default()
        })
    };
    if number.is_multiple_of(5) {
        document.body.push(Block::Paragraph(Paragraph {
            role: ParagraphRole::Heading(Bounded::new(1).unwrap_or_else(|| panic!("a level"))),
            inlines: vec![Inline::Run(Run::plain("Chapter"))],
            ..Paragraph::default()
        }));
    }
    for at in 0..6 {
        let mut bold = Run::plain(WORDS);
        bold.format.bold = Weight::Bold;
        document.body.push(paragraph(vec![
            Inline::Run(Run::plain(WORDS.repeat(6))),
            Inline::Run(bold),
            Inline::Run(Run::plain(WORDS.repeat(1 + at % 2))),
        ]));
    }
    document.body.push(Block::Paragraph(Paragraph {
        list: Some(ListMembership {
            list: WordId::new(1),
            level: Bounded::<ListDepth>::MIN,
            kind: ListKind::Bullet,
        }),
        inlines: vec![Inline::Run(Run::plain(WORDS))],
        ..Paragraph::default()
    }));
    let comment = WordId::new(number);
    let mut inserted = Run::plain(WORDS);
    inserted.revision = Some(Revision {
        kind: RevisionKind::Insertion,
        id: WordId::new(number),
        author: "Alice".to_owned(),
        date: Some(Instant::new(1_700_000_000_000)),
    });
    document.body.push(paragraph(vec![
        Inline::CommentStart(comment),
        Inline::Run(Run::plain(WORDS)),
        Inline::CommentEnd(comment),
        Inline::Run(inserted),
    ]));
    document.comments.push(Comment {
        id: comment,
        author: "Bob".to_owned(),
        date: Some(Instant::new(1_700_000_000_000)),
        paragraphs: vec![Paragraph {
            inlines: vec![Inline::Run(Run::plain("Is this right?"))],
            ..Paragraph::default()
        }],
        parent: None,
        state: CommentState::Open,
    });
    let cell = || TableCell {
        blocks: vec![paragraph(vec![Inline::Run(Run::plain("cell"))])],
        ..TableCell::default()
    };
    document.body.push(Block::Table(Table {
        columns: Vec::new(),
        rows: vec![
            TableRow {
                cells: vec![cell(), cell(), cell()],
                ..TableRow::default()
            };
            2
        ],
    }));
}

fn document() -> WordDocument {
    let mut document = WordDocument::default();
    for number in 0..PAGES {
        page(number, &mut document);
    }
    document
}

/// A novel of 300 pages of some 400 words, a heading every ten pages: the
/// largest plain document the import must take.
fn novel() -> WordDocument {
    let sentence = "The rain had stopped before the train reached the coast, and nobody spoke. ";
    let mut document = WordDocument::default();
    for page in 0..300_u32 {
        if page.is_multiple_of(10) {
            document.body.push(Block::Paragraph(Paragraph {
                role: ParagraphRole::Heading(Bounded::new(1).unwrap_or_else(|| panic!("a level"))),
                inlines: vec![Inline::Run(Run::plain("Chapter"))],
                ..Paragraph::default()
            }));
        }
        for _ in 0..4 {
            document.body.push(Block::Paragraph(Paragraph {
                inlines: vec![Inline::Run(Run::plain(sentence.repeat(7)))],
                ..Paragraph::default()
            }));
        }
    }
    document
}

fn bench(c: &mut Criterion) {
    let document = document();
    let bytes = ambre_docx::write(&document).unwrap_or_else(|e| panic!("writes: {e}"));
    c.bench_function("docx write, 100 pages", |b| {
        b.iter(|| ambre_docx::write(black_box(&document)));
    });
    c.bench_function("docx read, 100 pages", |b| {
        b.iter(|| ambre_docx::read(black_box(bytes.as_ref())));
    });
    let novel = ambre_docx::write(&novel()).unwrap_or_else(|e| panic!("writes: {e}"));
    c.bench_function("docx read, 300-page novel", |b| {
        b.iter(|| ambre_docx::read(black_box(novel.as_ref())));
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
