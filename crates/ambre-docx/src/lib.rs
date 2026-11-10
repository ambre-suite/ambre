//! The Word codec (`ambre-spec-prose-import-export.md section 3`).
//!
//! Two pure functions over a model of the format. [`read`] turns the bytes
//! of a `.docx` package into a [`WordDocument`] and a report of what it
//! approximated or dropped. [`write`] turns a [`WordDocument`] back into
//! bytes. Neither reads a clock, a file or the network, and neither knows an
//! application: the mapping to Prose's form lives with Prose.
//!
//! The package layer is `ambre-opc`, which the spreadsheet and presentation
//! codecs share, re-exported as [`opc`]. The relationship types Word follows
//! are [`rel`].
//!
//! The `codec` feature carries the reader, the writer and their ZIP and XML
//! dependencies. Without it the crate is the model alone.

pub mod construct;
pub mod model;
#[cfg(feature = "codec")]
pub use ambre_opc as opc;
#[cfg(feature = "codec")]
mod read;
#[cfg(feature = "codec")]
pub mod rel;
pub mod report;
#[cfg(feature = "codec")]
mod time;
#[cfg(feature = "codec")]
mod write;
#[cfg(feature = "codec")]
mod xml;

use std::collections::BTreeMap;

pub use construct::WordConstruct;
pub use model::WordDocument;
#[cfg(feature = "codec")]
pub use read::{Decoded, read};
pub use report::{Outcome, WordReport};
#[cfg(feature = "codec")]
pub use write::write;
#[cfg(feature = "codec")]
pub use xml::XmlError;

use model::{
    Block, CommentMarker, Inline, ListInstance, NoteKind, NoteMarker, Paragraph, RevisionMarker,
    Run, WordId,
};

/// Why a package produced no document. No variant quotes the file: its
/// bytes and its part names are the user's content (`INTEROP-004`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DocxError {
    #[cfg(feature = "codec")]
    #[error(transparent)]
    Package(#[from] opc::OpcError),
    #[cfg(feature = "codec")]
    #[error(transparent)]
    Xml(#[from] XmlError),
    #[error("the package names no main document")]
    NoMainPart,
    #[error("the package holds no Word document")]
    NotWord,
    #[error("a required part is missing: {0}")]
    MissingPart(&'static str),
}

impl WordDocument {
    /// The same document with every id numbered again, in the order the
    /// writer meets it. Comments count from 0, and tracked changes follow
    /// them. Notes of each kind count from 1, and lists from 1.
    ///
    /// A comment and a tracked change never share an id, as Word requires
    /// of its annotations. A note id stays clear of the two separators
    /// Word numbers -1 and 0. Two runs of one tracked change keep one id.
    /// A marker naming an id the document does not hold keeps a fresh id of
    /// its own, so it names nothing still.
    #[must_use]
    pub fn renumbered(&self) -> Self {
        let mut document = self.clone();
        let mut comments = Renumbering::<CommentMarker>::from(WordId::new(0));
        for comment in &document.comments {
            comments.map(comment.id);
        }
        let mut ids = Ids {
            phase: Phase::Register,
            comments,
            revisions: Renumbering::from(WordId::new(0)),
            footnotes: Renumbering::from(WordId::new(1)),
            endnotes: Renumbering::from(WordId::new(1)),
            lists: Renumbering::from(WordId::new(1)),
        };
        for note in &document.footnotes {
            ids.footnotes.map(note.id);
        }
        for note in &document.endnotes {
            ids.endnotes.map(note.id);
        }
        // The first walk registers every comment marker, a stray one
        // included, so the tracked changes the second walk numbers start
        // past the last comment id.
        ids.walk(&mut document);
        ids.phase = Phase::Apply;
        ids.revisions = Renumbering::from(ids.comments.next.shared_with());
        ids.walk(&mut document);
        for comment in &mut document.comments {
            comment.id = ids.comments.map(comment.id);
            comment.parent = comment.parent.map(|parent| ids.comments.map(parent));
        }
        for note in &mut document.footnotes {
            note.id = ids.footnotes.map(note.id);
        }
        for note in &mut document.endnotes {
            note.id = ids.endnotes.map(note.id);
        }
        document
    }
}

/// The new id of each old id of one kind, handed out in order.
struct Renumbering<K> {
    map: BTreeMap<WordId<K>, WordId<K>>,
    next: WordId<K>,
}

impl<K> Renumbering<K> {
    fn from(first: WordId<K>) -> Self {
        Self {
            map: BTreeMap::new(),
            next: first,
        }
    }

    fn map(&mut self, old: WordId<K>) -> WordId<K> {
        *self.map.entry(old).or_insert_with(|| {
            let id = self.next;
            self.next = WordId::new(id.get().saturating_add(1));
            id
        })
    }
}

/// Whether a walk only registers the comment markers, or numbers again.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Register,
    Apply,
}

struct Ids {
    phase: Phase,
    comments: Renumbering<CommentMarker>,
    revisions: Renumbering<RevisionMarker>,
    footnotes: Renumbering<NoteMarker>,
    endnotes: Renumbering<NoteMarker>,
    lists: Renumbering<ListInstance>,
}

impl Ids {
    fn walk(&mut self, document: &mut WordDocument) {
        self.blocks(&mut document.body);
        for blocks in [&mut document.header, &mut document.footer]
            .into_iter()
            .flatten()
        {
            self.blocks(blocks);
        }
        for note in document.footnotes.iter_mut().chain(&mut document.endnotes) {
            self.blocks(&mut note.blocks);
        }
        for paragraph in document
            .comments
            .iter_mut()
            .flat_map(|comment| comment.paragraphs.iter_mut())
        {
            self.paragraph(paragraph);
        }
    }

    fn blocks(&mut self, blocks: &mut [Block]) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => self.paragraph(paragraph),
                Block::Table(table) => {
                    for cell in table.rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                        self.blocks(&mut cell.blocks);
                    }
                }
            }
        }
    }

    fn paragraph(&mut self, paragraph: &mut Paragraph) {
        if self.phase == Phase::Register {
            for inline in &paragraph.inlines {
                if let Inline::CommentStart(id) | Inline::CommentEnd(id) = inline {
                    self.comments.map(*id);
                }
            }
            return;
        }
        if let Some(list) = &mut paragraph.list {
            list.list = self.lists.map(list.list);
        }
        for inline in &mut paragraph.inlines {
            match inline {
                Inline::Run(run) => self.run(run),
                Inline::Link(link) => link.runs.iter_mut().for_each(|run| self.run(run)),
                Inline::NoteCall(NoteKind::Footnote, id) => *id = self.footnotes.map(*id),
                Inline::NoteCall(NoteKind::Endnote, id) => *id = self.endnotes.map(*id),
                Inline::CommentStart(id) | Inline::CommentEnd(id) => *id = self.comments.map(*id),
                Inline::Break(_) | Inline::Tab | Inline::Image(_) => {}
            }
        }
    }

    fn run(&mut self, run: &mut Run) {
        if let Some(revision) = &mut run.revision {
            revision.id = self.revisions.map(revision.id);
        }
    }
}
