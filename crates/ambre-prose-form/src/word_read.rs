//! The Word reader's half of the mapping: the Word model into Prose's
//! intermediate form (`4-Prose/features/ambre-spec-prose-import-export.md
//! section 3`). It runs in the Office converter module.
//!
//! The codec reads the package into the model and a report
//! (`ambre_docx::read`). This half maps the model onto the form, carries the
//! report entry by entry, and reports what the form holds no place for. It
//! emits no command and sees no document (`PROSE-IO-009`). It runs nothing
//! the file carries: the model holds no macro, and the module never read its
//! bytes (`PROSE-IO-024`, `PROSE-IO-071`).
//!
//! A tracked change becomes a suggestion and a comment becomes a thread,
//! each on the runs it covers, never plain text (`PROSE-IO-060`).

use std::collections::{BTreeMap, BTreeSet};

use ambre_core::Bounded;
use ambre_core::index::Pos;
use ambre_core::{
    Authored, Instant, Ordinal, Slant, Strikethrough, ThreadState, Underline, Validated, Weight,
};
use ambre_docx::WordConstruct;
use ambre_docx::model::WordDate;
use ambre_docx::model::{
    Alignment, Block, BreakKind, CommentMarker, CommentState, Inline, LinkTarget, ListDepth,
    ListKind, NoteKind, NoteMarker, NumberFormat, Paragraph, ParagraphRole, RevisionKind,
    RevisionMarker, Run, Table, VerticalPosition, WordId,
};
use ambre_docx::{WordDocument, WordReport};

use crate::zone::Zone;
use crate::{
    Banding, FootnoteCall, FormConstruct, FormOutcome, FormReport, IrBlock, IrChange, IrComment,
    IrFootnote, IrImage, IrImageAlign, IrImageBytes, IrListStyle, IrMeta, IrOrderedStyle, IrReply,
    IrRun, IrSuggestion, IrTableCell, IrTableRow, IrThread, Merging, OwnStyle, Paging, ProseIr,
    StoredLevel, Subscript, SuggestionKind, Superscript, UnsupportedFeature,
};
use ambre_core::Flag as _;
use ambre_core::{Count, NarrowCount};

/// The comment threads open at one place of the walk, counted.
enum OpenThreads {}

impl NarrowCount for OpenThreads {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("OpenThreadCount");
}

/// The most threads open at once. Each run carries the threads open over
/// it. A file that opens many threads and closes none would copy them all
/// into every run (`W304`). Word nests a handful. A start past the bound
/// leaves its thread without an anchor, and the report names it.
const MAX_OPEN_THREADS: Count<OpenThreads> = Count::new(16);

/// Map a Word model onto the form, and report what the reader and the
/// mapping lost.
#[must_use]
///
/// A date Word wrote as a wall time alone is read in `zone`, the zone of
/// the importing session. It is reported approximated, since Word may have
/// written it in another (`PROSE-IO-084`).
pub fn read_document(
    document: &WordDocument,
    report: &WordReport,
    zone: &Zone,
) -> (ProseIr, FormReport) {
    let mut warnings = FormReport::default();
    for entry in report.entries() {
        let outcome = match entry.outcome {
            ambre_docx::Outcome::Carried => FormOutcome::Carried,
            ambre_docx::Outcome::Approximated => FormOutcome::Approximated,
            ambre_docx::Outcome::Dropped => FormOutcome::Dropped,
        };
        warnings.record(
            FormConstruct::Word(entry.construct),
            outcome,
            ambre_core::Count::new(entry.count.get()),
        );
    }
    let mut reader = Reader {
        zone,
        warnings,
        threads: BTreeMap::new(),
        suggestions: BTreeMap::new(),
        notes: BTreeMap::new(),
        ir_suggestions: Vec::new(),
        open: Vec::new(),
    };
    reader.number_notes(document);
    let comments = reader.threads(document);
    let blocks = reader.blocks(&document.body);
    let footnotes = reader.footnotes(document);
    let ir = ProseIr {
        meta: IrMeta {
            title: document.title.clone(),
            paged: if document.page.is_some() {
                Paging::Paged
            } else {
                Paging::Flow
            },
            bands: if document.header.is_some() || document.footer.is_some() {
                Banding::Banded
            } else {
                Banding::Bare
            },
        },
        blocks,
        footnotes,
        comments,
        suggestions: std::mem::take(&mut reader.ir_suggestions),
    };
    (ir, reader.warnings)
}

/// The note a call names: its ordinal, and whether Word calls it an endnote.
#[derive(Clone, Copy)]
struct NoteRef {
    ordinal: Ordinal<FootnoteCall>,
    kind: NoteKind,
}

struct Reader<'a> {
    zone: &'a Zone,
    warnings: FormReport,
    /// The thread each comment marker names: a reply's marker names its
    /// thread's.
    threads: BTreeMap<WordId<CommentMarker>, Pos<IrThread>>,
    /// The suggestion each tracked change becomes, by its Word id.
    suggestions: BTreeMap<WordId<RevisionMarker>, Pos<IrChange>>,
    /// The ordinal of each note, the footnotes first, then the endnotes.
    notes: BTreeMap<(NoteKindKey, WordId<NoteMarker>), NoteRef>,
    ir_suggestions: Vec<IrSuggestion>,
    /// The threads open at the point the walk stands. A Word comment may
    /// span paragraphs, so the set outlives one paragraph.
    open: Vec<Pos<IrThread>>,
}

/// The first ancestor of `id`, each comment on the way remembered in
/// `known`. A parent the document does not hold ends the climb. A loop of
/// parents ends at the first comment met twice, so each comment climbs once.
fn climb(
    parents: &BTreeMap<WordId<CommentMarker>, Option<WordId<CommentMarker>>>,
    known: &mut BTreeMap<WordId<CommentMarker>, WordId<CommentMarker>>,
    id: WordId<CommentMarker>,
) -> WordId<CommentMarker> {
    let mut path: Vec<WordId<CommentMarker>> = Vec::new();
    let mut seen: BTreeSet<WordId<CommentMarker>> = BTreeSet::new();
    let mut at = id;
    let root = loop {
        if let Some(root) = known.get(&at) {
            break *root;
        }
        if !seen.insert(at) {
            break at;
        }
        path.push(at);
        match parents.get(&at).copied().flatten() {
            Some(parent) if parents.contains_key(&parent) => at = parent,
            _ => break at,
        }
    };
    for step in path {
        known.insert(step, root);
    }
    root
}

/// A note kind as a map key: the model's kind derives no order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NoteKindKey {
    Footnote,
    Endnote,
}

const fn note_key(kind: NoteKind) -> NoteKindKey {
    match kind {
        NoteKind::Footnote => NoteKindKey::Footnote,
        NoteKind::Endnote => NoteKindKey::Endnote,
    }
}

impl Reader<'_> {
    fn number_notes(&mut self, document: &WordDocument) {
        let notes = document
            .footnotes
            .iter()
            .map(|note| (NoteKind::Footnote, note.id))
            .chain(
                document
                    .endnotes
                    .iter()
                    .map(|note| (NoteKind::Endnote, note.id)),
            );
        for (position, (kind, id)) in notes.enumerate() {
            self.notes.insert(
                (note_key(kind), id),
                NoteRef {
                    ordinal: Ordinal::<FootnoteCall>::from_position(position),
                    kind,
                },
            );
        }
    }

    /// The threads of the document: each comment with no parent opens one,
    /// and each reply joins the thread of its first ancestor. A parent the
    /// document does not hold, or a loop of parents, opens a thread too.
    fn threads(&mut self, document: &WordDocument) -> Vec<IrComment> {
        let parents: BTreeMap<_, _> = document
            .comments
            .iter()
            .map(|comment| (comment.id, comment.parent))
            .collect();
        // The root each comment met so far climbs to. A chain of replies is
        // climbed once, not once per reply (`W305`).
        let mut root_of: BTreeMap<WordId<CommentMarker>, WordId<CommentMarker>> = BTreeMap::new();
        let mut threads: Vec<IrComment> = Vec::new();
        let mut roots: BTreeMap<WordId<CommentMarker>, Pos<IrThread>> = BTreeMap::new();
        for comment in &document.comments {
            let root = climb(&parents, &mut root_of, comment.id);
            let at = *roots.entry(root).or_insert_with(|| {
                threads.push(IrComment {
                    resolved: ThreadState::Open,
                    replies: Vec::new(),
                });
                Pos::<IrThread>::from_usize(threads.len() - 1)
            });
            self.threads.insert(comment.id, at);
            let Some(thread) = threads.get_mut(at.to_usize()) else {
                continue;
            };
            if root == comment.id {
                thread.resolved = match comment.state {
                    CommentState::Open => ThreadState::Open,
                    CommentState::Resolved => ThreadState::Resolved,
                };
            }
            let reply = IrReply {
                author: display_name(&comment.author),
                created_at: self.date(comment.date),
                text: comment
                    .paragraphs
                    .iter()
                    .map(paragraph_text)
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            // The opening body leads its thread whatever order the file
            // lists the comments in.
            if root == comment.id {
                thread.replies.insert(0, reply);
            } else {
                thread.replies.push(reply);
            }
        }
        threads
    }

    fn blocks(&mut self, blocks: &[Block]) -> Vec<IrBlock> {
        let mut out = Vec::new();
        // The items of the list being read, each with its level.
        let mut items: Vec<(Bounded<ListDepth>, IrBlock)> = Vec::new();
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) if paragraph.list.is_some() => {
                    if let Some(item) = self.list_item(paragraph) {
                        items.push(item);
                    }
                }
                Block::Paragraph(paragraph) => {
                    out.extend(nest(std::mem::take(&mut items)));
                    self.paragraph(paragraph, &mut out);
                }
                Block::Table(table) => {
                    out.extend(nest(std::mem::take(&mut items)));
                    out.push(self.table(table));
                }
            }
        }
        out.extend(nest(items));
        out
    }

    fn list_item(&mut self, paragraph: &Paragraph) -> Option<(Bounded<ListDepth>, IrBlock)> {
        let list = paragraph.list?;
        let style = match list.kind {
            ListKind::Bullet => IrListStyle::Bullet,
            ListKind::Numbered(format) => IrListStyle::Ordered(match format {
                NumberFormat::Decimal => IrOrderedStyle::Decimal,
                NumberFormat::LowerLetter => IrOrderedStyle::LowerAlpha,
                NumberFormat::UpperLetter => IrOrderedStyle::UpperAlpha,
                NumberFormat::LowerRoman => IrOrderedStyle::LowerRoman,
                NumberFormat::UpperRoman => IrOrderedStyle::UpperRoman,
            }),
        };
        let mut images = Vec::new();
        let runs = self.runs(&paragraph.inlines, &mut images);
        // A picture in the item's line rides as its child. The tree takes a
        // picture at the root alone. So the builder lifts it beside the list
        // and reports the lift, rather than dropping it
        // (`PROSE-FEAT-053`).
        Some((
            list.level,
            IrBlock::ListItem {
                style,
                runs,
                children: images,
            },
        ))
    }

    /// One paragraph, as one block or more. A page break splits it, and a
    /// picture in its line follows it as a block of its own.
    fn paragraph(&mut self, paragraph: &Paragraph, out: &mut Vec<IrBlock>) {
        let pieces = paragraph
            .inlines
            .split(|inline| matches!(inline, Inline::Break(BreakKind::Page)));
        for (at, piece) in pieces.enumerate() {
            if at > 0 {
                out.push(IrBlock::PageBreak);
            }
            // A page break alone in its paragraph is the whole block.
            if piece.is_empty() && at > 0 {
                continue;
            }
            if piece.is_empty() && paragraph_has_page_break(paragraph) {
                continue;
            }
            let mut images = Vec::new();
            let block = match paragraph.role {
                ParagraphRole::Code => IrBlock::CodeBlock {
                    lang: None,
                    text: code_text(piece),
                },
                ParagraphRole::Heading(depth) => IrBlock::Heading {
                    level: StoredLevel::from_stored(depth.get()),
                    runs: self.runs(piece, &mut images),
                },
                ParagraphRole::Body | ParagraphRole::Title | ParagraphRole::Quote => {
                    let plain = paragraph.role == ParagraphRole::Body
                        && matches!(paragraph.align, None | Some(Alignment::Start));
                    IrBlock::Paragraph {
                        styled: OwnStyle::from_bool(!plain),
                        runs: self.runs(piece, &mut images),
                    }
                }
            };
            // A paragraph that held nothing but its pictures is those pictures,
            // so a round trip adds no empty paragraph before each one.
            let bare = matches!(&block, IrBlock::Paragraph { runs, .. } if runs.is_empty());
            if !(bare && !images.is_empty()) {
                out.push(block);
            }
            // A picture alone in a centred paragraph is a centred picture.
            if paragraph.align == Some(Alignment::Center) {
                for image in &mut images {
                    if let IrBlock::Image(image) = image {
                        image.align = IrImageAlign::Center;
                    }
                }
            }
            out.extend(images);
        }
    }

    fn table(&mut self, table: &Table) -> IrBlock {
        let merged = table.rows.iter().flat_map(|row| &row.cells).any(|cell| {
            cell.span.get() > 1 || cell.merge != ambre_docx::model::VerticalMerge::None
        });
        IrBlock::Table {
            merged: Merging::from_bool(merged),
            rows: table
                .rows
                .iter()
                .map(|row| IrTableRow {
                    cells: row
                        .cells
                        .iter()
                        .map(|cell| IrTableCell {
                            styled: OwnStyle::from_bool(cell.fill.is_some()),
                            blocks: self.blocks(&cell.blocks),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// The runs of `inlines`. A picture in the line is moved to `images`.
    fn runs(&mut self, inlines: &[Inline], images: &mut Vec<IrBlock>) -> Vec<IrRun> {
        let mut out: Vec<IrRun> = Vec::new();
        for inline in inlines {
            match inline {
                Inline::Run(run) => out.push(self.run(run, None)),
                Inline::Link(link) => {
                    let url = match &link.target {
                        LinkTarget::External(url) => Some(url.clone()),
                        LinkTarget::Anchor(_) => {
                            self.warnings
                                .unsupported(UnsupportedFeature::CrossReference);
                            None
                        }
                    };
                    for run in &link.runs {
                        out.push(self.run(run, url.clone()));
                    }
                }
                Inline::Tab => out.push(self.plain("\t")),
                Inline::Break(_) => out.push(self.plain("\n")),
                Inline::NoteCall(kind, id) => {
                    let Some(note) = self.notes.get(&(note_key(*kind), *id)).copied() else {
                        continue;
                    };
                    if note.kind == NoteKind::Endnote {
                        self.warnings.approximated(UnsupportedFeature::Endnote);
                    }
                    // The call follows the text it ends, as the form keeps it.
                    match out.last_mut().filter(|run| run.footnote_ref.is_none()) {
                        Some(run) => run.footnote_ref = Some(note.ordinal),
                        None => {
                            let mut run = self.plain("");
                            run.footnote_ref = Some(note.ordinal);
                            out.push(run);
                        }
                    }
                }
                Inline::CommentStart(id) => {
                    if let Some(at) = self.threads.get(id).copied()
                        && !self.open.contains(&at)
                    {
                        if Count::<OpenThreads>::of(self.open.len()) < MAX_OPEN_THREADS {
                            self.open.push(at);
                        } else {
                            self.warnings.approximated(UnsupportedFeature::Comment);
                        }
                    }
                }
                Inline::CommentEnd(id) => {
                    if let Some(at) = self.threads.get(id).copied() {
                        self.open.retain(|open| *open != at);
                    }
                }
                Inline::Image(image) => images.push(IrBlock::Image(Box::new(IrImage {
                    alt: image.description.clone(),
                    width: image.width,
                    height: image.height,
                    bytes: Some(IrImageBytes {
                        format: image.format,
                        data: image.bytes.clone(),
                    }),
                    ..IrImage::default()
                }))),
            }
        }
        out
    }

    /// The instant of a Word date: its UTC instant when the file gives one,
    /// else its wall time read in the session's zone, reported.
    fn date(&mut self, date: Option<WordDate>) -> Option<Instant<Authored>> {
        match date? {
            WordDate::Zoned { utc, .. } => Some(utc),
            WordDate::Floating(wall) => {
                self.warnings.record(
                    FormConstruct::Word(WordConstruct::LocalDate),
                    FormOutcome::Approximated,
                    crate::report::ONE,
                );
                self.zone.instant_of(wall)
            }
        }
    }

    /// A run of text under the threads open here, with no format.
    fn plain(&self, text: &str) -> IrRun {
        IrRun {
            comments: self.open.clone(),
            ..IrRun::plain(text)
        }
    }

    fn run(&mut self, run: &Run, link: Option<ambre_core::LinkUrl>) -> IrRun {
        let format = &run.format;
        let mut out = IrRun {
            bold: if format.bold == Weight::Bold {
                Weight::Bold
            } else {
                Weight::Regular
            },
            italic: if format.italic == Slant::Italic {
                Slant::Italic
            } else {
                Slant::Upright
            },
            underline: if format.underline == Underline::Underlined {
                Underline::Underlined
            } else {
                Underline::Plain
            },
            strikethrough: if format.strike == Strikethrough::Struck {
                Strikethrough::Struck
            } else {
                Strikethrough::Plain
            },
            superscript: if format.position == VerticalPosition::Superscript {
                Superscript::Raised
            } else {
                Superscript::Level
            },
            subscript: if format.position == VerticalPosition::Subscript {
                Subscript::Lowered
            } else {
                Subscript::Level
            },
            link,
            styled: OwnStyle::from_bool(
                format.size.is_some()
                    || format.color.is_some()
                    || format.fill.is_some()
                    || format.font.is_some(),
            ),
            ..self.plain(&run.text)
        };
        if let Some(revision) = &run.revision {
            let next = Pos::<IrChange>::from_usize(self.ir_suggestions.len());
            let at = *self.suggestions.entry(revision.id).or_insert(next);
            if at == next {
                let date = self.date(revision.date);
                self.ir_suggestions.push(IrSuggestion {
                    kind: match revision.kind {
                        RevisionKind::Insertion => SuggestionKind::Insertion,
                        RevisionKind::Deletion => SuggestionKind::Deletion,
                    },
                    author: display_name(&revision.author),
                    date,
                });
            }
            match revision.kind {
                RevisionKind::Insertion => out.insertion = Some(at),
                RevisionKind::Deletion => out.deletion = Some(at),
            }
        }
        out
    }

    /// The footnote bodies, the endnotes after them, each flattened into
    /// one run of paragraphs.
    fn footnotes(&mut self, document: &WordDocument) -> Vec<IrFootnote> {
        let notes = document
            .footnotes
            .iter()
            .map(|note| (NoteKind::Footnote, note))
            .chain(
                document
                    .endnotes
                    .iter()
                    .map(|note| (NoteKind::Endnote, note)),
            );
        let mut out = Vec::new();
        for (kind, note) in notes {
            let Some(reference) = self.notes.get(&(note_key(kind), note.id)).copied() else {
                continue;
            };
            let mut images = Vec::new();
            let mut runs = Vec::new();
            for block in &note.blocks {
                if let Block::Paragraph(paragraph) = block {
                    runs.extend(self.runs(&paragraph.inlines, &mut images));
                }
            }
            out.push(IrFootnote {
                ordinal: reference.ordinal,
                runs,
            });
        }
        out
    }
}

/// The items of one list, each nested under the last item of a lower level
/// before it. The levels are 0 to 8, so the nesting is nine deep at most.
fn nest(items: Vec<(Bounded<ListDepth>, IrBlock)>) -> Vec<IrBlock> {
    let mut out = Vec::new();
    let mut rest = items.into_iter().peekable();
    while let Some((level, mut item)) = rest.next() {
        let mut deeper = Vec::new();
        while let Some((next, _)) = rest.peek() {
            if *next <= level {
                break;
            }
            if let Some(child) = rest.next() {
                deeper.push(child);
            }
        }
        if let IrBlock::ListItem { children, .. } = &mut item {
            *children = nest(deeper);
        }
        out.push(item);
    }
    out
}

fn paragraph_has_page_break(paragraph: &Paragraph) -> bool {
    paragraph
        .inlines
        .iter()
        .any(|inline| matches!(inline, Inline::Break(BreakKind::Page)))
}

/// The text of a code paragraph: each line break a line feed.
fn code_text(inlines: &[Inline]) -> String {
    let mut text = String::new();
    for inline in inlines {
        match inline {
            Inline::Run(run) => text.push_str(&run.text),
            Inline::Link(link) => link.runs.iter().for_each(|run| text.push_str(&run.text)),
            Inline::Break(_) => text.push('\n'),
            Inline::Tab => text.push('\t'),
            _ => {}
        }
    }
    text
}

/// The text of one comment paragraph.
fn paragraph_text(paragraph: &Paragraph) -> String {
    code_text(&paragraph.inlines)
}

/// The name a Word annotation gives, as a display name. An empty name, or
/// one past the rule, is no name.
fn display_name(author: &str) -> Option<Validated<ambre_core::DisplayName>> {
    Validated::<ambre_core::DisplayName>::parse(author).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_docx::model::Comment;

    fn comment(id: u32, parent: Option<u32>) -> Comment {
        Comment {
            id: WordId::new(id),
            author: String::new(),
            date: None,
            paragraphs: Vec::new(),
            parent: parent.map(WordId::new),
            state: CommentState::Open,
        }
    }

    fn text(text: &str) -> Inline {
        Inline::Run(Run {
            text: text.to_owned(),
            ..Run::default()
        })
    }

    /// `W304`: thousands of threads are opened and never closed. Each run
    /// holds at most the bound of open threads, and the report names the
    /// starts past it.
    #[test]
    fn threads_left_open_are_bounded_per_run() {
        let threads: u32 = 2_000;
        let mut inlines: Vec<Inline> = (0..threads)
            .map(|id| Inline::CommentStart(WordId::new(id)))
            .collect();
        inlines.extend((0..2_000).map(|_| text("a")));
        let document = WordDocument {
            body: vec![Block::Paragraph(Paragraph {
                inlines,
                ..Paragraph::default()
            })],
            comments: (0..threads).map(|id| comment(id, None)).collect(),
            ..WordDocument::default()
        };
        let (ir, report) = read_document(&document, &WordReport::default(), &Zone::utc());
        let bound = usize::try_from(MAX_OPEN_THREADS.get()).expect("fits");
        let Some(IrBlock::Paragraph { runs, .. }) = ir.blocks.first() else {
            panic!("a paragraph");
        };
        assert!(!runs.is_empty());
        assert!(runs.iter().all(|run| run.comments.len() <= bound));
        assert_eq!(
            report
                .count(
                    FormConstruct::Feature(UnsupportedFeature::Comment),
                    FormOutcome::Approximated
                )
                .get(),
            threads - MAX_OPEN_THREADS.get()
        );
    }

    /// `W305`: a chain of replies, each the child of the one before, joins
    /// one thread, each comment climbed once.
    #[test]
    fn a_long_chain_of_replies_joins_one_thread() {
        let length: u32 = 100_000;
        let document = WordDocument {
            comments: (0..length)
                .map(|id| comment(id, id.checked_sub(1)))
                .collect(),
            ..WordDocument::default()
        };
        let (ir, _) = read_document(&document, &WordReport::default(), &Zone::utc());
        assert_eq!(ir.comments.len(), 1);
        assert_eq!(
            ir.comments.first().map(|thread| thread.replies.len()),
            Some(usize::try_from(length).expect("fits"))
        );
    }

    /// A loop of parents still ends, and opens one thread.
    #[test]
    fn a_loop_of_parents_opens_one_thread() {
        let document = WordDocument {
            comments: vec![
                comment(0, Some(2)),
                comment(1, Some(0)),
                comment(2, Some(1)),
            ],
            ..WordDocument::default()
        };
        let (ir, _) = read_document(&document, &WordReport::default(), &Zone::utc());
        assert_eq!(ir.comments.len(), 1);
    }
}
