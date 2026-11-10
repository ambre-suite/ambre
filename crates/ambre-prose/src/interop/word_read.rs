//! The Word reader's half in Prose: the Word model into the intermediate
//! form (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The OOXML converter module reads the package into the model and a report
//! (`ambre_docx::read`). This half maps the model onto the form, carries the
//! report entry by entry, and reports what the form holds no place for. It
//! emits no command and sees no document (`PROSE-IO-009`). It runs nothing
//! the file carries: the model holds no macro, and the module never read its
//! bytes (`PROSE-IO-024`, `PROSE-IO-071`).
//!
//! A tracked change becomes a suggestion and a comment becomes a thread,
//! each on the runs it covers, never plain text (`PROSE-IO-060`).

use std::collections::BTreeMap;

use ambre_bus::interop::{Direction, FileFormat};
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::Bounded;
use ambre_core::index::Pos;
use ambre_core::{
    Authored, Count, FileBytes, Instant, Octets, Ordinal, Payload, Slant, Strikethrough,
    ThreadState, Underline, Validated, Weight,
};
use ambre_docx::model::{
    Alignment, Block, BreakKind, CommentMarker, CommentState, Inline, LinkTarget, ListDepth,
    ListKind, NoteKind, NoteMarker, NumberFormat, Paragraph, ParagraphRole, RevisionKind,
    RevisionMarker, Run, Table, VerticalPosition, WordId,
};
use ambre_docx::{WordDocument, WordReport};

use super::{
    Banding, IrBlock, IrChange, IrComment, IrFootnote, IrListStyle, IrMeta, IrOrderedStyle,
    IrReply, IrRun, IrSuggestion, IrTableCell, IrTableRow, IrThread, Merging, OwnStyle, Paging,
    ProseIr, Warnings,
};
use crate::doc::StoredLevel;
use crate::events::SuggestionKind;
use crate::mark::{Subscript, Superscript};
use crate::message::{ConversionReport, FootnoteCall, InteropError, UnsupportedFeature};

/// The largest file a Word import hands the module. It is the bound of what
/// the package layer inflates (`ambre_opc::MAX_INFLATED`), which a stored
/// package reaches with no compression at all.
pub const MAX_SIZE: Count<Octets> = Count::<Octets>::new(64 * 1024 * 1024);

/// Read the bytes of `file` as a Word package, through the OOXML module
/// `converter` holds.
///
/// # Errors
///
/// [`InteropError::TooLarge`] past [`MAX_SIZE`], and
/// [`InteropError::ParseFailed`] for a package the reader refuses, in its
/// own words: a bound passed, a document type, a part missing. A module that
/// stops or answers out of turn fails as [`InteropError::Stopped`].
pub fn read(
    file: &Payload<FileBytes>,
    converter: &Converter,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    let size = file.size();
    if size > MAX_SIZE {
        return Err(InteropError::TooLarge(size));
    }
    match converter.convert(&Conversion::ReadDocx { data: file.clone() }) {
        Ok(Converted::Word(model)) => ambre_convert::decode_word(&model)
            .map(|(document, report)| read_document(&document, &report))
            .map_err(|e| InteropError::Stopped(e.to_string())),
        Ok(_) => Err(InteropError::Stopped(ConvertError::Malformed.to_string())),
        Err(ConvertError::Refused(reason)) => Err(InteropError::ParseFailed(reason)),
        Err(e) => Err(InteropError::Stopped(e.to_string())),
    }
}

/// Map a Word model onto the form, and report what the reader and the
/// mapping lost.
#[must_use]
pub fn read_document(document: &WordDocument, report: &WordReport) -> (ProseIr, ConversionReport) {
    let mut warnings = Warnings::new(Direction::Import, FileFormat::Docx);
    warnings.word(report);
    let mut reader = Reader {
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
    (ir, reader.warnings.report())
}

/// The note a call names: its ordinal, and whether Word calls it an endnote.
#[derive(Clone, Copy)]
struct NoteRef {
    ordinal: Ordinal<FootnoteCall>,
    kind: NoteKind,
}

struct Reader {
    warnings: Warnings,
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

impl Reader {
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
        let root_of = |mut id: WordId<CommentMarker>| {
            // A chain longer than the comments it could visit is a loop.
            for _ in 0..=parents.len() {
                match parents.get(&id).copied().flatten() {
                    Some(parent) if parents.contains_key(&parent) => id = parent,
                    _ => return id,
                }
            }
            id
        };
        let mut threads: Vec<IrComment> = Vec::new();
        let mut roots: BTreeMap<WordId<CommentMarker>, Pos<IrThread>> = BTreeMap::new();
        for comment in &document.comments {
            let root = root_of(comment.id);
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
                created_at: comment.date.unwrap_or(Instant::<Authored>::EPOCH),
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
        if !images.is_empty() {
            self.warnings.unsupported(UnsupportedFeature::Image);
        }
        Some((
            list.level,
            IrBlock::ListItem {
                style,
                runs,
                children: Vec::new(),
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
            out.push(block);
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
                        self.open.push(at);
                    }
                }
                Inline::CommentEnd(id) => {
                    if let Some(at) = self.threads.get(id).copied() {
                        self.open.retain(|open| *open != at);
                    }
                }
                Inline::Image(image) => images.push(IrBlock::Image {
                    alt: image.description.clone(),
                }),
            }
        }
        out
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
                self.ir_suggestions.push(IrSuggestion {
                    kind: match revision.kind {
                        RevisionKind::Insertion => SuggestionKind::Insertion,
                        RevisionKind::Deletion => SuggestionKind::Deletion,
                    },
                    author: display_name(&revision.author),
                    date: revision.date,
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
