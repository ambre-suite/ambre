//! The Word writer's half of the mapping: Prose's intermediate form into the
//! Word model (`4-Prose/features/ambre-spec-prose-import-export.md section
//! 3`). It runs in the OOXML converter module.
//!
//! The OOXML converter module turns the model into a package
//! (`ambre_docx::write`). This half sees no document and no package: it maps
//! one form onto one model, and reports each construct the model holds no
//! form for. A construct it writes as the nearest thing Word holds is
//! approximated (`PROSE-IO-006`), and one it cannot write at all is dropped
//! (`PROSE-IO-004`).
//!
//! A comment thread becomes a Word comment and one reply per later body. A
//! suggestion becomes a tracked change on each run it covers
//! (`PROSE-IO-060`).

use ambre_core::index::Pos;
use ambre_core::{
    Bounded, Count, PrintLength, Pt, Slant, Strikethrough, ThreadState, Underline, Weight,
};
use ambre_docx::WordDocument;
use ambre_docx::model::{
    Block, BreakKind, Comment, CommentMarker, CommentState, HeadingDepth, Inline, Link, LinkTarget,
    ListDepth, ListInstance, ListKind, ListMembership, Margins, Note, NoteKind, NumberFormat,
    Orientation, PageSetup, Paragraph, ParagraphRole, Revision, RevisionKind, Run, RunFormat,
    Table, TableCell, TableRow, VerticalPosition, WordDate, WordId,
};

use crate::{
    Banding, FormReport, InlineCode, IrBlock, IrChange, IrListStyle, IrOrderedStyle, IrRun,
    IrThread, OwnStyle, Paging, ProseIr, Subscript, SuggestionKind, Superscript,
    UnsupportedFeature, ZoneOffset,
};

/// The monospace family a run of inline code takes in Word, which has no
/// such run. Word ships it on every platform it runs on.
const CODE_FONT: &str = "Courier New";

/// The page a paged document takes in Word: A4 portrait, 25.4 mm margins.
/// The form flags the page and carries none of its values, so the writer
/// reports it approximated.
const A4: PageSetup = PageSetup {
    width: PrintLength::<Pt>::new(595.3),
    height: PrintLength::<Pt>::new(841.9),
    margins: Margins {
        top: PrintLength::<Pt>::new(72.0),
        bottom: PrintLength::<Pt>::new(72.0),
        left: PrintLength::<Pt>::new(72.0),
        right: PrintLength::<Pt>::new(72.0),
    },
    orientation: Orientation::Portrait,
};

/// How deep a list item nests, its outermost item at zero.
pub enum ItemDepth {}

impl ambre_core::CountKind for ItemDepth {
    type Repr = u32;
    const NAME: &'static str = "ItemDepth";
}

/// The Word model of `ir`, and the report of what Word could not hold.
#[must_use]
///
/// Each date is written as its wall time in `zone`, the session's zone, with
/// its UTC instant beside it (`PROSE-IO-085`).
pub fn write(ir: &ProseIr, zone: ZoneOffset) -> (WordDocument, FormReport) {
    let mut writer = Writer {
        zone,
        ir,
        warnings: FormReport::default(),
        next_list: WordId::new(1),
    };
    let mut document = WordDocument {
        title: ir.meta.title.clone(),
        ..WordDocument::default()
    };
    if ir.meta.paged == Paging::Paged {
        document.page = Some(A4);
        writer.warnings.approximated(UnsupportedFeature::PageFormat);
    }
    if ir.meta.bands == Banding::Banded {
        writer
            .warnings
            .unsupported(UnsupportedFeature::HeaderAndFooter);
    }
    document.body = writer.blocks(&ir.blocks);
    document.footnotes = ir
        .footnotes
        .iter()
        .map(|note| Note {
            id: WordId::new(note.ordinal.get()),
            blocks: vec![Block::Paragraph(Paragraph {
                inlines: writer.inlines(&note.runs),
                ..Paragraph::default()
            })],
        })
        .collect();
    document.comments = writer.comments();
    (document, writer.warnings)
}

struct Writer<'a> {
    zone: ZoneOffset,
    ir: &'a ProseIr,
    warnings: FormReport,
    /// The id the next list takes. A list runs over consecutive items, and
    /// the next run of items starts its count again.
    next_list: WordId<ListInstance>,
}

/// The Word comment id of a thread: its place in the form.
fn thread_id(at: Pos<IrThread>) -> WordId<CommentMarker> {
    WordId::new(at.get())
}

impl Writer<'_> {
    /// The date Word reads: the wall time in the session's zone, and the
    /// UTC instant beside it.
    fn word_date(&self, utc: ambre_core::Instant<ambre_core::Authored>) -> WordDate {
        WordDate::Zoned {
            utc,
            wall: self.zone.wall_of(utc),
        }
    }

    /// The id of a new list: the next one, and the count moves on.
    fn mint_list(&mut self) -> WordId<ListInstance> {
        let id = self.next_list;
        self.next_list = WordId::new(id.get().saturating_add(1));
        id
    }

    fn blocks(&mut self, blocks: &[IrBlock]) -> Vec<Block> {
        let mut out = Vec::new();
        let mut list: Option<WordId<ListInstance>> = None;
        for block in blocks {
            if let IrBlock::ListItem { .. } = block {
                let id = *list.get_or_insert_with(|| self.mint_list());
                self.list_item(block, id, Count::ZERO, &mut out);
            } else {
                list = None;
                self.block(block, &mut out);
            }
        }
        out
    }

    /// One item and the items nested in it, each a paragraph of the list one
    /// level deeper. A block other than an item follows the item, outside
    /// the list: Word nests no block in a paragraph.
    fn list_item(
        &mut self,
        block: &IrBlock,
        list: WordId<ListInstance>,
        depth: Count<ItemDepth>,
        out: &mut Vec<Block>,
    ) {
        let IrBlock::ListItem {
            style,
            runs,
            children,
        } = block
        else {
            self.warnings
                .approximated(UnsupportedFeature::BlockInListItem);
            self.block(block, out);
            return;
        };
        let level = Bounded::<ListDepth>::saturating_from(i64::from(depth.get()));
        if u32::from(level.get()) < depth.get() {
            self.warnings.approximated(UnsupportedFeature::ListDepth);
        }
        let kind = match style {
            IrListStyle::Bullet => ListKind::Bullet,
            IrListStyle::Todo(_) => {
                self.warnings.approximated(UnsupportedFeature::TodoItem);
                ListKind::Bullet
            }
            IrListStyle::Ordered(ordered) => ListKind::Numbered(match ordered {
                IrOrderedStyle::Decimal => NumberFormat::Decimal,
                IrOrderedStyle::LowerAlpha => NumberFormat::LowerLetter,
                IrOrderedStyle::UpperAlpha => NumberFormat::UpperLetter,
                IrOrderedStyle::LowerRoman => NumberFormat::LowerRoman,
                IrOrderedStyle::UpperRoman => NumberFormat::UpperRoman,
            }),
        };
        out.push(Block::Paragraph(Paragraph {
            list: Some(ListMembership { list, level, kind }),
            inlines: self.inlines(runs),
            ..Paragraph::default()
        }));
        for child in children {
            self.list_item(child, list, depth.incremented(), out);
        }
    }

    fn block(&mut self, block: &IrBlock, out: &mut Vec<Block>) {
        match block {
            IrBlock::Paragraph { styled, runs } => {
                if *styled == OwnStyle::Styled {
                    self.warnings
                        .unsupported(UnsupportedFeature::ParagraphFormatting);
                }
                out.push(Block::Paragraph(Paragraph {
                    inlines: self.inlines(runs),
                    ..Paragraph::default()
                }));
            }
            IrBlock::Heading { level, runs } => {
                let depth = Bounded::<HeadingDepth>::new(level.get()).unwrap_or_else(|| {
                    self.warnings.approximated(UnsupportedFeature::HeadingLevel);
                    if level.get() == 0 {
                        Bounded::<HeadingDepth>::MIN
                    } else {
                        Bounded::<HeadingDepth>::MAX
                    }
                });
                out.push(Block::Paragraph(Paragraph {
                    role: ParagraphRole::Heading(depth),
                    inlines: self.inlines(runs),
                    ..Paragraph::default()
                }));
            }
            IrBlock::ListItem { .. } => {
                let list = self.mint_list();
                self.list_item(block, list, Count::ZERO, out);
            }
            IrBlock::Table { merged, rows } => {
                if merged.as_bool() {
                    self.warnings.approximated(UnsupportedFeature::MergedCells);
                }
                let rows = rows
                    .iter()
                    .map(|row| TableRow {
                        cells: row
                            .cells
                            .iter()
                            .map(|cell| {
                                if cell.styled == OwnStyle::Styled {
                                    self.warnings
                                        .unsupported(UnsupportedFeature::CellFormatting);
                                }
                                let mut blocks = self.blocks(&cell.blocks);
                                // Word requires a paragraph in every cell.
                                if blocks.is_empty() {
                                    blocks.push(Block::Paragraph(Paragraph::default()));
                                }
                                TableCell {
                                    blocks,
                                    ..TableCell::default()
                                }
                            })
                            .collect(),
                        ..TableRow::default()
                    })
                    .collect();
                out.push(Block::Table(Table {
                    columns: Vec::new(),
                    rows,
                }));
            }
            IrBlock::CodeBlock { lang, text } => {
                if lang.is_some() {
                    self.warnings.unsupported(UnsupportedFeature::CodeLanguage);
                }
                let mut inlines = Vec::new();
                for (at, line) in text.split('\n').enumerate() {
                    if at > 0 {
                        inlines.push(Inline::Break(BreakKind::Line));
                    }
                    if !line.is_empty() {
                        inlines.push(Inline::Run(Run::plain(line)));
                    }
                }
                out.push(Block::Paragraph(Paragraph {
                    role: ParagraphRole::Code,
                    inlines,
                    ..Paragraph::default()
                }));
            }
            IrBlock::Image { .. } => self.warnings.unsupported(UnsupportedFeature::Image),
            IrBlock::PageBreak => out.push(Block::Paragraph(Paragraph {
                inlines: vec![Inline::Break(BreakKind::Page)],
                ..Paragraph::default()
            })),
            IrBlock::HRule => self
                .warnings
                .unsupported(UnsupportedFeature::HorizontalRule),
            IrBlock::Toc => self
                .warnings
                .unsupported(UnsupportedFeature::TableOfContents),
        }
    }

    /// The inlines of one paragraph. A thread opens where its first run
    /// starts and closes where its last run ends, so a comment anchors on
    /// the runs that name it.
    fn inlines(&mut self, runs: &[IrRun]) -> Vec<Inline> {
        let mut out = Vec::new();
        let mut open: Vec<Pos<IrThread>> = Vec::new();
        for run in runs {
            open.retain(|thread| {
                let keeps = run.comments.contains(thread);
                if !keeps {
                    out.push(Inline::CommentEnd(thread_id(*thread)));
                }
                keeps
            });
            for thread in &run.comments {
                if !open.contains(thread) && self.ir.comments.get(thread.to_usize()).is_some() {
                    out.push(Inline::CommentStart(thread_id(*thread)));
                    open.push(*thread);
                }
            }
            if !run.text.is_empty() {
                let word = self.run(run);
                out.push(match &run.link {
                    Some(url) => Inline::Link(Link {
                        target: LinkTarget::External(url.clone()),
                        runs: vec![word],
                    }),
                    None => Inline::Run(word),
                });
            }
            if let Some(ordinal) = run.footnote_ref {
                out.push(Inline::NoteCall(
                    NoteKind::Footnote,
                    WordId::new(ordinal.get()),
                ));
            }
        }
        out.extend(
            open.into_iter()
                .map(|thread| Inline::CommentEnd(thread_id(thread))),
        );
        out
    }

    fn run(&mut self, run: &IrRun) -> Run {
        let position = match (run.superscript, run.subscript) {
            (Superscript::Raised, Subscript::Lowered) => {
                self.warnings.approximated(UnsupportedFeature::Subscript);
                VerticalPosition::Superscript
            }
            (Superscript::Raised, Subscript::Level) => VerticalPosition::Superscript,
            (Superscript::Level, Subscript::Lowered) => VerticalPosition::Subscript,
            (Superscript::Level, Subscript::Level) => VerticalPosition::Baseline,
        };
        let font = (run.code == InlineCode::Code).then(|| {
            self.warnings.approximated(UnsupportedFeature::InlineCode);
            CODE_FONT.to_owned()
        });
        if run.styled == OwnStyle::Styled {
            self.warnings.unsupported(UnsupportedFeature::Styling);
        }
        if run.cross_ref.as_bool() {
            self.warnings
                .unsupported(UnsupportedFeature::CrossReference);
        }
        Run {
            text: run.text.clone(),
            format: RunFormat {
                bold: if run.bold == Weight::Bold {
                    Weight::Bold
                } else {
                    Weight::Regular
                },
                italic: if run.italic == Slant::Italic {
                    Slant::Italic
                } else {
                    Slant::Upright
                },
                underline: if run.underline == Underline::Underlined {
                    Underline::Underlined
                } else {
                    Underline::Plain
                },
                strike: if run.strikethrough == Strikethrough::Struck {
                    Strikethrough::Struck
                } else {
                    Strikethrough::Plain
                },
                position,
                font,
                ..RunFormat::default()
            },
            revision: self.revision(run),
        }
    }

    /// The tracked change a run carries. A Word run carries one, so a run
    /// both inserted and deleted keeps its deletion, which decides whether
    /// the text survives an acceptance.
    fn revision(&mut self, run: &IrRun) -> Option<Revision> {
        let change = match (run.insertion, run.deletion) {
            (Some(_), Some(deletion)) => {
                self.warnings.approximated(UnsupportedFeature::Suggestion);
                deletion
            }
            (Some(change), None) | (None, Some(change)) => change,
            (None, None) => return None,
        };
        let suggestion = self.ir.suggestions.get(change.to_usize())?;
        Some(Revision {
            kind: match suggestion.kind {
                SuggestionKind::Insertion => RevisionKind::Insertion,
                SuggestionKind::Deletion => RevisionKind::Deletion,
            },
            id: WordId::new(Pos::<IrChange>::get(change)),
            author: author_name(suggestion.author.as_ref()),
            date: suggestion.date.map(|utc| self.word_date(utc)),
        })
    }

    /// One Word comment per body of each thread: the opening body takes the
    /// thread's id, and each reply names it as its parent.
    fn comments(&self) -> Vec<Comment> {
        let mut out = Vec::new();
        let mut next = u32::try_from(self.ir.comments.len()).unwrap_or(u32::MAX);
        for (at, thread) in self.ir.comments.iter().enumerate() {
            let root = thread_id(Pos::<IrThread>::from_usize(at));
            let state = match thread.resolved {
                ThreadState::Open => CommentState::Open,
                ThreadState::Resolved => CommentState::Resolved,
            };
            for (index, reply) in thread.replies.iter().enumerate() {
                let (id, parent) = if index == 0 {
                    (root, None)
                } else {
                    let id = WordId::new(next);
                    next = next.saturating_add(1);
                    (id, Some(root))
                };
                out.push(Comment {
                    id,
                    author: author_name(reply.author.as_ref()),
                    date: reply.created_at.map(|utc| self.word_date(utc)),
                    paragraphs: reply
                        .text
                        .split('\n')
                        .map(|line| Paragraph {
                            inlines: if line.is_empty() {
                                Vec::new()
                            } else {
                                vec![Inline::Run(Run::plain(line))]
                            },
                            ..Paragraph::default()
                        })
                        .collect(),
                    parent,
                    state,
                });
            }
        }
        out
    }
}

/// The name a Word annotation carries: the author's display name, or none.
fn author_name(name: Option<&ambre_core::Validated<ambre_core::DisplayName>>) -> String {
    name.map(|name| name.as_str().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::word_read::read_document;
    use crate::{
        FootnoteCall, FormConstruct as ProseConstruct, FormOutcome as Outcome, HeadingLevel,
        IrComment, IrFootnote, IrMeta, IrReply, IrSuggestion, IrTableCell, IrTableRow, Merging,
        Referencing, StoredLevel,
    };
    use ambre_core::{Authored, DisplayName, Instant, LinkUrl, Ordinal, Validated};

    fn name(text: &str) -> Option<Validated<DisplayName>> {
        Some(Validated::<DisplayName>::parse(text).expect("a display name"))
    }

    /// A whole second: Word dates carry no millisecond.
    fn at(seconds: u64) -> Instant<Authored> {
        Instant::<Authored>::new(seconds * 1000)
    }

    fn para(runs: Vec<IrRun>) -> IrBlock {
        IrBlock::Paragraph {
            styled: OwnStyle::Plain,
            runs,
        }
    }

    fn thread(at: usize) -> Pos<IrThread> {
        Pos::<IrThread>::from_usize(at)
    }

    fn change(at: usize) -> Pos<IrChange> {
        Pos::<IrChange>::from_usize(at)
    }

    /// Every structure the form holds that Word holds too, once each.
    fn representable() -> ProseIr {
        let item = |style, text: &str, children| IrBlock::ListItem {
            style,
            runs: vec![IrRun::plain(text)],
            children,
        };
        ProseIr {
            meta: IrMeta {
                title: Some("Report".to_owned()),
                paged: Paging::Paged,
                bands: Banding::Bare,
            },
            blocks: vec![
                IrBlock::Heading {
                    level: StoredLevel::of(HeadingLevel::H1),
                    runs: vec![IrRun::plain("Title")],
                },
                para(vec![
                    IrRun::plain("plain "),
                    IrRun {
                        bold: Weight::Bold,
                        italic: Slant::Italic,
                        ..IrRun::plain("strong")
                    },
                    IrRun {
                        underline: Underline::Underlined,
                        strikethrough: Strikethrough::Struck,
                        ..IrRun::plain("marked")
                    },
                    IrRun {
                        superscript: Superscript::Raised,
                        ..IrRun::plain("2")
                    },
                    IrRun {
                        subscript: Subscript::Lowered,
                        ..IrRun::plain("i")
                    },
                    IrRun {
                        link: Some(LinkUrl::new("https://example.org/a?b=1&c=2".to_owned())),
                        ..IrRun::plain("a link")
                    },
                    IrRun {
                        footnote_ref: Some(Ordinal::<FootnoteCall>::FIRST),
                        ..IrRun::plain("noted")
                    },
                ]),
                para(vec![
                    IrRun::plain("Hello "),
                    IrRun {
                        comments: vec![thread(0)],
                        ..IrRun::plain("world")
                    },
                    IrRun {
                        comments: vec![thread(0), thread(1)],
                        insertion: Some(change(0)),
                        ..IrRun::plain(" new")
                    },
                    IrRun {
                        deletion: Some(change(1)),
                        ..IrRun::plain(" old")
                    },
                ]),
                item(
                    IrListStyle::Bullet,
                    "one",
                    vec![item(
                        IrListStyle::Ordered(IrOrderedStyle::LowerRoman),
                        "one.i",
                        Vec::new(),
                    )],
                ),
                item(IrListStyle::Bullet, "two", Vec::new()),
                IrBlock::Table {
                    merged: Merging::Unmerged,
                    rows: vec![IrTableRow {
                        cells: vec![
                            IrTableCell {
                                styled: OwnStyle::Plain,
                                blocks: vec![para(vec![IrRun::plain("a")])],
                            },
                            IrTableCell {
                                styled: OwnStyle::Plain,
                                blocks: vec![para(vec![IrRun::plain("b")])],
                            },
                        ],
                    }],
                },
                IrBlock::CodeBlock {
                    lang: None,
                    text: "let x = 1;\nlet y = 2;".to_owned(),
                },
                IrBlock::PageBreak,
                IrBlock::Heading {
                    level: StoredLevel::from_stored(9),
                    runs: vec![IrRun::plain("Deep")],
                },
            ],
            footnotes: vec![IrFootnote {
                ordinal: Ordinal::<FootnoteCall>::FIRST,
                runs: vec![IrRun::plain("The note.")],
            }],
            comments: vec![
                IrComment {
                    resolved: ThreadState::Open,
                    replies: vec![
                        IrReply {
                            author: name("Ada"),
                            created_at: Some(at(1_700_000_000)),
                            text: "Check this.\nTwice.".to_owned(),
                        },
                        IrReply {
                            author: name("Bob"),
                            created_at: Some(at(1_700_000_060)),
                            text: "Done.".to_owned(),
                        },
                    ],
                },
                IrComment {
                    resolved: ThreadState::Resolved,
                    replies: vec![IrReply {
                        author: None,
                        created_at: Some(at(1_700_000_120)),
                        text: "Closed.".to_owned(),
                    }],
                },
            ],
            suggestions: vec![
                IrSuggestion {
                    kind: SuggestionKind::Insertion,
                    author: name("Bob"),
                    date: Some(at(1_700_000_180)),
                },
                IrSuggestion {
                    kind: SuggestionKind::Deletion,
                    author: name("Eve"),
                    date: None,
                },
            ],
        }
    }

    /// The form a Word package of `ir` reads back as, through the codec,
    /// and the report of the export.
    fn round_trip(ir: &ProseIr) -> (ProseIr, FormReport) {
        let (document, exported) = write(ir, ZoneOffset::UTC);
        let bytes = ambre_docx::write(&document).expect("the model writes");
        let decoded = ambre_docx::read(bytes.as_bytes()).expect("the package reads");
        let (back, _) = read_document(&decoded.document, &decoded.report, ZoneOffset::UTC);
        (back, exported)
    }

    /// `PROSE-IO-014`: every structure Word holds comes back as it left.
    /// That is the marks, the link, the nested list, the table, the code
    /// block, the break and the note. It is also the threads, the
    /// suggestions and their authors. The one loss is the page, which Word
    /// gets as A4, and the report names it.
    #[test]
    fn a_word_round_trip_keeps_every_structure_word_holds() {
        let ir = representable();
        let (back, report) = round_trip(&ir);
        assert_eq!(back, ir);
        assert_eq!(report.entries().len(), 1, "{report:?}");
        assert_eq!(
            report
                .count(
                    ProseConstruct::Feature(UnsupportedFeature::PageFormat),
                    Outcome::Approximated
                )
                .get(),
            1
        );
    }

    /// `PROSE-IO-014`: what Word cannot hold is named once each, with what
    /// the writer did with it.
    #[test]
    fn what_word_lacks_is_reported_once_each() {
        let ir = ProseIr {
            meta: IrMeta {
                title: None,
                paged: Paging::Flow,
                bands: Banding::Banded,
            },
            blocks: vec![
                IrBlock::Paragraph {
                    styled: OwnStyle::Styled,
                    runs: vec![IrRun {
                        styled: OwnStyle::Styled,
                        cross_ref: Referencing::Reference,
                        ..IrRun::plain("x")
                    }],
                },
                IrBlock::CodeBlock {
                    lang: Some(
                        Validated::<crate::CodeLang>::parse("rust".to_owned()).expect("a language"),
                    ),
                    text: "fn f() {}".to_owned(),
                },
                IrBlock::Image {
                    alt: "a cat".to_owned(),
                },
                IrBlock::HRule,
                IrBlock::Toc,
            ],
            ..ProseIr::default()
        };
        let (_, report) = write(&ir, ZoneOffset::UTC);
        for feature in [
            UnsupportedFeature::HeaderAndFooter,
            UnsupportedFeature::ParagraphFormatting,
            UnsupportedFeature::Styling,
            UnsupportedFeature::CrossReference,
            UnsupportedFeature::CodeLanguage,
            UnsupportedFeature::Image,
            UnsupportedFeature::HorizontalRule,
            UnsupportedFeature::TableOfContents,
        ] {
            assert_eq!(
                report
                    .count(ProseConstruct::Feature(feature), Outcome::Dropped)
                    .get(),
                1,
                "{feature:?} in {report:?}"
            );
        }
        assert_eq!(report.entries().len(), 8);
    }

    /// `PROSE-IO-006`: a construct the writer emits as the nearest thing Word
    /// holds is recorded as approximated. A to-do item is a bullet, inline
    /// code a monospace run, and merged cells single cells. An item nested
    /// past nine levels lands at the ninth.
    #[test]
    fn an_approximation_the_writer_emits_is_in_the_report() {
        let mut deep = IrBlock::ListItem {
            style: IrListStyle::Bullet,
            runs: vec![IrRun::plain("deepest")],
            children: Vec::new(),
        };
        for _ in 0..10 {
            deep = IrBlock::ListItem {
                style: IrListStyle::Bullet,
                runs: vec![IrRun::plain("level")],
                children: vec![deep],
            };
        }
        let ir = ProseIr {
            blocks: vec![
                IrBlock::ListItem {
                    style: IrListStyle::Todo(crate::TodoMark::Done),
                    runs: vec![IrRun {
                        code: InlineCode::Code,
                        ..IrRun::plain("ship")
                    }],
                    children: Vec::new(),
                },
                IrBlock::Table {
                    merged: Merging::Merged,
                    rows: vec![IrTableRow {
                        cells: vec![IrTableCell::default()],
                    }],
                },
                deep,
            ],
            ..ProseIr::default()
        };
        let (document, report) = write(&ir, ZoneOffset::UTC);
        let approximated = |feature| {
            report
                .count(ProseConstruct::Feature(feature), Outcome::Approximated)
                .get()
        };
        assert_eq!(approximated(UnsupportedFeature::TodoItem), 1);
        assert_eq!(approximated(UnsupportedFeature::InlineCode), 1);
        assert_eq!(approximated(UnsupportedFeature::MergedCells), 1);
        assert_eq!(approximated(UnsupportedFeature::ListDepth), 2);
        let Some(Block::Paragraph(todo)) = document.body.first() else {
            panic!("the to-do item leads the body");
        };
        assert_eq!(todo.list.map(|list| list.kind), Some(ListKind::Bullet));
        let Some(Inline::Run(run)) = todo.inlines.first() else {
            panic!("the item holds its run");
        };
        assert_eq!(run.format.font.as_deref(), Some(CODE_FONT));
    }

    /// Paris in summer: two hours east of UTC.
    fn paris() -> ZoneOffset {
        ZoneOffset::from_minutes_east(120).expect("an offset")
    }

    fn one_comment(date: Option<WordDate>) -> WordDocument {
        WordDocument {
            comments: vec![Comment {
                id: WordId::new(0),
                author: "Ada".to_owned(),
                date,
                paragraphs: Vec::new(),
                parent: None,
                state: CommentState::Open,
            }],
            ..WordDocument::default()
        }
    }

    /// `PROSE-IO-084`: a date Word wrote as a wall time alone reads in the
    /// session's zone, and the report names it approximated. A date that
    /// carries its UTC instant reads as that instant, and names nothing.
    #[test]
    fn a_wall_time_reads_in_the_session_zone_and_is_reported() {
        let wall = Instant::<Authored>::new(1_462_810_380_000);
        let (ir, report) = read_document(
            &one_comment(Some(WordDate::Floating(wall))),
            &ambre_docx::WordReport::default(),
            paris(),
        );
        assert_eq!(
            ir.comments[0].replies[0].created_at,
            Some(Instant::<Authored>::new(1_462_810_380_000 - 7_200_000))
        );
        let local = ProseConstruct::Word(ambre_docx::WordConstruct::LocalDate);
        assert_eq!(report.count(local, Outcome::Approximated).get(), 1);

        let utc = Instant::<Authored>::new(1_462_803_180_000);
        let (ir, report) = read_document(
            &one_comment(Some(WordDate::Zoned { utc, wall })),
            &ambre_docx::WordReport::default(),
            paris(),
        );
        assert_eq!(ir.comments[0].replies[0].created_at, Some(utc));
        assert!(report.entries().is_empty());
    }

    /// `PROSE-IO-085`: an export writes each date as its wall time in the
    /// session's zone, with the `Z` Word expects, and its UTC instant in
    /// `w16du:dateUtc`, the namespace declared.
    #[test]
    fn a_written_date_carries_its_wall_time_and_its_utc_instant() {
        let utc = Instant::<Authored>::new(1_462_803_180_000);
        let ir = ProseIr {
            blocks: vec![para(vec![IrRun {
                comments: vec![thread(0)],
                ..IrRun::plain("x")
            }])],
            comments: vec![IrComment {
                resolved: ThreadState::Open,
                replies: vec![IrReply {
                    author: name("Ada"),
                    created_at: Some(utc),
                    text: "When?".to_owned(),
                }],
            }],
            ..ProseIr::default()
        };
        let (document, _) = write(&ir, paris());
        assert_eq!(
            document.comments[0].date,
            Some(WordDate::Zoned {
                utc,
                wall: Instant::<Authored>::new(1_462_810_380_000),
            })
        );
        let bytes = ambre_docx::write(&document).expect("the model writes");
        let mut package =
            ambre_docx::opc::Package::open(bytes.as_bytes()).expect("the package opens");
        let part = ambre_docx::opc::PartName::new("word/comments.xml").expect("a name");
        let comments = package
            .part(&part)
            .expect("reads")
            .expect("a comments part");
        let text = String::from_utf8(comments.as_bytes().to_vec()).expect("UTF-8");
        assert!(text.contains("w:date=\"2016-05-09T16:13:00Z\""), "{text}");
        assert!(
            text.contains("w16du:dateUtc=\"2016-05-09T14:13:00Z\""),
            "{text}"
        );
        assert!(text.contains("xmlns:w16du="));
    }
}
