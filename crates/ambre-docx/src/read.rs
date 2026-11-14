//! The reader: a `.docx` package into a [`WordDocument`] and its report.
//!
//! The reader walks each part once, as a tree. It follows the relationships
//! of the main part and of each part it reads, and no address outside the
//! package. It never reads the bytes of a macro project. The report names
//! each construct the model has no form for, approximated or dropped. The
//! reader keeps the text under an element it does not recognise.

use std::collections::{BTreeMap, BTreeSet};

use ambre_core::{
    Bounded, FontSize, LinkUrl, PrintLength, Pt, Rgb, Slant, Strikethrough, Underline, Weight,
};

use crate::DocxError;
use crate::construct::WordConstruct;
use crate::model::{
    Alignment, Block, BreakKind, Comment, CommentMarker, CommentState, GridSpan, HeadingDepth,
    Image, ImageFormat, Inline, Link, LinkTarget, ListDepth, ListKind, ListMembership, Margins,
    Note, NoteKind, NumberFormat, Orientation, PageSetup, Paragraph, ParagraphRole, Revision,
    RevisionKind, RowRole, Run, RunFormat, Table, TableCell, TableRow, VerticalMerge,
    VerticalPosition, WordDocument, WordId,
};
use crate::opc::{self, Package, PartName, RelId, RelKind, RelTarget, Relationship};
use crate::rel;
use crate::report::{Outcome, WordReport};
use crate::time;
use crate::xml::{self, Element, Ns};
use ambre_core::Flag as _;

/// The EMUs in one point: a DrawingML length is in English Metric Units.
const EMU_PER_PT: f32 = 12_700.0;

/// The twips in one point: a WordprocessingML length is in twentieths of a
/// point.
const TWIPS_PER_PT: f32 = 20.0;

/// A document read, and what the reading carried and lost.
#[derive(Debug)]
pub struct Decoded {
    pub document: WordDocument,
    pub report: WordReport,
}

/// The document a `.docx`, `.docm` or `.dotx` package holds.
///
/// # Errors
///
/// A [`DocxError`] when the bytes are no Word package or a part the
/// document needs does not read. A package past a bound of [`opc`] fails
/// too.
pub fn read(bytes: &[u8]) -> Result<Decoded, DocxError> {
    let mut package = Package::open(bytes)?;
    let main = package
        .related(&opc::root(), RelKind::OFFICE_DOCUMENT)?
        .ok_or(DocxError::NoMainPart)?;
    let content_type = package.content_type(&main).unwrap_or_default();
    if !is_word_main(content_type) {
        return Err(DocxError::NotWord);
    }
    let mut reader = Reader {
        report: WordReport::default(),
        styles: BTreeMap::new(),
        numbering: BTreeMap::new(),
        comments_started: BTreeSet::new(),
        fields: Vec::new(),
        pending_starts: Vec::new(),
    };
    reader.note_macros(&mut package)?;
    let main_rels = package.relationships(&main)?;
    if let Some(part) = find(&main_rels, rel::STYLES) {
        let root = parse_part(&mut package, &part)?;
        reader.styles = read_styles(&root);
    }
    if let Some(part) = find(&main_rels, rel::NUMBERING) {
        let root = parse_part(&mut package, &part)?;
        reader.numbering = read_numbering(&root);
    }

    let mut document = WordDocument::default();
    let root = parse_part(&mut package, &main)?;
    let body = root
        .child(Ns::W, "body")
        .ok_or(DocxError::MissingPart("word/document.xml body"))?;
    let context = PartContext::new(&main_rels);
    document.body = reader.blocks(&mut package, &context, body)?;
    if let Some(section) = body.child(Ns::W, "sectPr") {
        document.page = page_setup(section);
        for reference in section.elements() {
            let band = match reference.name() {
                "headerReference" => rel::HEADER,
                "footerReference" => rel::FOOTER,
                _ => continue,
            };
            if !matches!(reference.w_attr("type"), None | Some("default")) {
                reader
                    .report
                    .record(WordConstruct::PageBand, Outcome::Dropped);
                continue;
            }
            let Some(part) = reference
                .attr(Ns::R, "id")
                .and_then(|id| context.part(id, band))
            else {
                continue;
            };
            let blocks = reader.part_blocks(&mut package, &part)?;
            match band {
                rel::HEADER => document.header = Some(blocks),
                _ => document.footer = Some(blocks),
            }
        }
    }
    if let Some(part) = find(&main_rels, rel::FOOTNOTES) {
        document.footnotes = reader.notes(&mut package, &part, NoteKind::Footnote)?;
    }
    if let Some(part) = find(&main_rels, rel::ENDNOTES) {
        document.endnotes = reader.notes(&mut package, &part, NoteKind::Endnote)?;
    }
    if let Some(part) = find(&main_rels, rel::COMMENTS) {
        let extended = match find(&main_rels, rel::COMMENTS_EXTENDED) {
            Some(part) => Some(parse_part(&mut package, &part)?),
            None => None,
        };
        document.comments = reader.comments(&mut package, &part, extended.as_ref())?;
    }
    if let Some(part) = package.related(&opc::root(), RelKind::CORE_PROPERTIES)? {
        let root = parse_part(&mut package, &part)?;
        document.title = root
            .child(Ns::DublinCore, "title")
            .map(Element::text)
            .filter(|title| !title.is_empty());
    }
    Ok(Decoded {
        document,
        report: reader.report,
    })
}

/// Whether a content type names the main part of a Word document, a
/// template or a document that carries macros.
fn is_word_main(content_type: &str) -> bool {
    [
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
        "application/vnd.ms-word.document.macroEnabled.main+xml",
        "application/vnd.ms-word.template.macroEnabledTemplate.main+xml",
    ]
    .contains(&content_type)
}

fn find(rels: &[Relationship], kind: RelKind) -> Option<PartName> {
    rels.iter().find_map(|rel| match &rel.target {
        RelTarget::Part(name) if rel.is(kind) => Some(name.clone()),
        _ => None,
    })
}

fn parse_part(package: &mut Package<'_>, part: &PartName) -> Result<Element, DocxError> {
    let bytes = package
        .part(part)?
        .ok_or(DocxError::MissingPart("a related part"))?;
    Ok(xml::parse(&bytes)?)
}

/// The relationships of the part being read, by id.
struct PartContext {
    rels: BTreeMap<RelId, Relationship>,
}

impl PartContext {
    fn new(rels: &[Relationship]) -> Self {
        Self {
            rels: rels
                .iter()
                .map(|rel| (rel.id.clone(), rel.clone()))
                .collect(),
        }
    }

    fn rel(&self, id: &str) -> Option<&Relationship> {
        self.rels.get(&RelId::new(id))
    }

    fn part(&self, id: &str, kind: RelKind) -> Option<PartName> {
        match self.rel(id) {
            Some(Relationship {
                kind: found,
                target: RelTarget::Part(name),
                ..
            }) if found.is(kind) => Some(name.clone()),
            _ => None,
        }
    }
}

/// Where a reader stands in a computed field.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldPhase {
    /// Between the field's start and its separator: the instruction.
    Instruction,
    /// Between the separator and the end: the result the field last showed.
    Result,
}

struct Reader {
    report: WordReport,
    styles: BTreeMap<String, StyleDef>,
    numbering: BTreeMap<String, [Option<ListLevel>; 9]>,
    /// The comments whose anchor has started, so a reference with no anchor
    /// reads as an anchor on its own position.
    comments_started: BTreeSet<WordId<CommentMarker>>,
    /// The open fields, the innermost last. A field may span paragraphs.
    fields: Vec<FieldPhase>,
    /// Comment starts met between blocks, for the next paragraph.
    pending_starts: Vec<Inline>,
}

/// One level of a list definition, and whether the model carries its
/// numbering format.
#[derive(Clone, Copy)]
struct ListLevel {
    /// What the level shows, or `None` for a level that shows nothing, as
    /// the outline numbering LibreOffice gives its headings.
    kind: Option<ListKind>,
    exact: Exactness,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Exactness {
    Exact,
    Approximated,
}

impl Reader {
    fn note_macros(&mut self, package: &mut Package<'_>) -> Result<(), DocxError> {
        let named = package.part_names().iter().any(|name| {
            name.as_str()
                .to_ascii_lowercase()
                .ends_with("vbaproject.bin")
        });
        let related = package
            .relationships(&opc::root())?
            .iter()
            .any(|rel| rel.is(rel::VBA_PROJECT));
        if named || related {
            self.report.record(WordConstruct::Macro, Outcome::Dropped);
        }
        Ok(())
    }

    /// The blocks of a header or a footer part.
    fn part_blocks(
        &mut self,
        package: &mut Package<'_>,
        part: &PartName,
    ) -> Result<Vec<Block>, DocxError> {
        let rels = package.relationships(part)?;
        let root = parse_part(package, part)?;
        let context = PartContext::new(&rels);
        self.fields.clear();
        self.blocks(package, &context, &root)
    }

    fn notes(
        &mut self,
        package: &mut Package<'_>,
        part: &PartName,
        kind: NoteKind,
    ) -> Result<Vec<Note>, DocxError> {
        let rels = package.relationships(part)?;
        let root = parse_part(package, part)?;
        let context = PartContext::new(&rels);
        let mut notes = Vec::new();
        for note in root.elements() {
            if !matches!(note.name(), "footnote" | "endnote") {
                continue;
            }
            // A separator note draws the rule above the notes, and no call
            // names it.
            if note.w_attr("type").is_some_and(|kind| kind != "normal") {
                continue;
            }
            let Some(id) = note.w_attr("id").and_then(decimal_id) else {
                continue;
            };
            self.fields.clear();
            let blocks = self.blocks(package, &context, note)?;
            self.report.record(
                match kind {
                    NoteKind::Footnote => WordConstruct::Footnote,
                    NoteKind::Endnote => WordConstruct::Endnote,
                },
                Outcome::Carried,
            );
            notes.push(Note {
                id: WordId::new(id),
                blocks,
            });
        }
        Ok(notes)
    }

    fn comments(
        &mut self,
        package: &mut Package<'_>,
        part: &PartName,
        extended: Option<&Element>,
    ) -> Result<Vec<Comment>, DocxError> {
        // Word 2013 keeps a reply's parent and a thread's state apart, keyed
        // by the paragraph id of each comment's last paragraph.
        let mut by_paragraph: BTreeMap<String, (Option<String>, CommentState)> = BTreeMap::new();
        for entry in extended.into_iter().flat_map(Element::elements) {
            if !entry.is(Ns::W15, "commentEx") {
                continue;
            }
            let Some(paragraph) = entry.attr(Ns::W15, "paraId") else {
                continue;
            };
            let state = match entry.attr(Ns::W15, "done") {
                Some("1" | "true") => CommentState::Resolved,
                _ => CommentState::Open,
            };
            let parent = entry.attr(Ns::W15, "paraIdParent").map(str::to_owned);
            by_paragraph.insert(paragraph.to_owned(), (parent, state));
        }

        let rels = package.relationships(part)?;
        let root = parse_part(package, part)?;
        let context = PartContext::new(&rels);
        let mut comments = Vec::new();
        let mut last_paragraph: BTreeMap<String, WordId<CommentMarker>> = BTreeMap::new();
        let mut parents: Vec<Option<String>> = Vec::new();
        for comment in root.elements().filter(|e| e.is(Ns::W, "comment")) {
            let Some(id) = comment.w_attr("id").and_then(decimal_id) else {
                continue;
            };
            let id = WordId::new(id);
            self.fields.clear();
            let blocks = self.blocks(package, &context, comment)?;
            let paragraphs = self.flatten(blocks);
            let paragraph_id = comment
                .elements()
                .filter(|e| e.is(Ns::W, "p"))
                .last()
                .and_then(|p| p.attr(Ns::W14, "paraId"))
                .map(str::to_owned);
            let (parent, state) = paragraph_id
                .as_ref()
                .and_then(|key| by_paragraph.get(key).cloned())
                .unwrap_or_default();
            if let Some(key) = paragraph_id {
                last_paragraph.insert(key, id);
            }
            parents.push(parent);
            self.report.record(WordConstruct::Comment, Outcome::Carried);
            comments.push(Comment {
                id,
                author: comment.w_attr("author").unwrap_or_default().to_owned(),
                date: time::word_date(comment.w_attr("date"), comment.attr(Ns::W16du, "dateUtc")),
                paragraphs,
                parent: None,
                state,
            });
        }
        for (comment, parent) in comments.iter_mut().zip(parents) {
            comment.parent = parent.and_then(|key| last_paragraph.get(&key).copied());
        }
        Ok(comments)
    }

    /// The paragraphs of `blocks`, a table's read cell by cell. A comment
    /// holds paragraphs alone.
    fn flatten(&mut self, blocks: Vec<Block>) -> Vec<Paragraph> {
        let mut out = Vec::new();
        let mut stack: Vec<Block> = blocks.into_iter().rev().collect();
        while let Some(block) = stack.pop() {
            match block {
                Block::Paragraph(paragraph) => out.push(paragraph),
                Block::Table(table) => {
                    self.report
                        .record(WordConstruct::Table, Outcome::Approximated);
                    let cells = table
                        .rows
                        .into_iter()
                        .flat_map(|row| row.cells)
                        .flat_map(|cell| cell.blocks);
                    let mut inner: Vec<Block> = cells.collect();
                    inner.reverse();
                    stack.extend(inner);
                }
            }
        }
        out
    }

    /// The blocks under `parent`: a body, a cell, a note, a band or a
    /// comment.
    fn blocks(
        &mut self,
        package: &mut Package<'_>,
        context: &PartContext,
        parent: &Element,
    ) -> Result<Vec<Block>, DocxError> {
        let mut out = Vec::new();
        self.blocks_into(package, context, parent, &mut out)?;
        Ok(out)
    }

    fn blocks_into(
        &mut self,
        package: &mut Package<'_>,
        context: &PartContext,
        parent: &Element,
        out: &mut Vec<Block>,
    ) -> Result<(), DocxError> {
        for element in parent.elements() {
            if element.ns != Ns::W {
                if element.is(Ns::Mc, "AlternateContent") {
                    if let Some(branch) = alternate(element) {
                        self.blocks_into(package, context, branch, out)?;
                    }
                } else if element.ns == Ns::Math {
                    self.report
                        .record(WordConstruct::Equation, Outcome::Dropped);
                }
                continue;
            }
            match element.name() {
                "p" => {
                    let paragraph = self.paragraph(package, context, element)?;
                    out.push(Block::Paragraph(paragraph));
                }
                "tbl" => {
                    let table = self.table(package, context, element)?;
                    out.push(Block::Table(table));
                }
                "sdt" => {
                    self.report
                        .record(WordConstruct::ContentControl, Outcome::Approximated);
                    if let Some(content) = element.child(Ns::W, "sdtContent") {
                        self.blocks_into(package, context, content, out)?;
                    }
                }
                "customXml" => self.blocks_into(package, context, element, out)?,
                "altChunk" => self
                    .report
                    .record(WordConstruct::ImportedChunk, Outcome::Dropped),
                "bookmarkStart" => self.bookmark(element),
                "commentRangeStart" => {
                    if let Some(id) = element.w_attr("id").and_then(decimal_id) {
                        let id = WordId::new(id);
                        self.comments_started.insert(id);
                        self.pending_starts.push(Inline::CommentStart(id));
                    }
                }
                "commentRangeEnd" => {
                    if let Some(id) = element.w_attr("id").and_then(decimal_id) {
                        let end = Inline::CommentEnd(WordId::new(id));
                        match out.last_mut() {
                            Some(Block::Paragraph(paragraph)) => paragraph.inlines.push(end),
                            _ => self.pending_starts.push(end),
                        }
                    }
                }
                "sectPr" | "bookmarkEnd" | "proofErr" | "permStart" | "permEnd"
                | "moveFromRangeStart" | "moveFromRangeEnd" | "moveToRangeStart"
                | "moveToRangeEnd" | "tblPr" | "tblGrid" | "tcPr" | "trPr" | "footnoteRef" => {}
                _ => {
                    self.report
                        .record(WordConstruct::UnknownElement, Outcome::Dropped);
                    self.blocks_into(package, context, element, out)?;
                }
            }
        }
        Ok(())
    }

    fn bookmark(&mut self, element: &Element) {
        // Word writes hidden bookmarks, `_GoBack` and `_Toc...`, which no
        // user named.
        if !element
            .w_attr("name")
            .is_some_and(|name| name.starts_with('_'))
        {
            self.report
                .record(WordConstruct::Bookmark, Outcome::Dropped);
        }
    }

    fn paragraph(
        &mut self,
        package: &mut Package<'_>,
        context: &PartContext,
        element: &Element,
    ) -> Result<Paragraph, DocxError> {
        let mut paragraph = Paragraph {
            inlines: std::mem::take(&mut self.pending_starts),
            ..Paragraph::default()
        };
        let properties = element.child(Ns::W, "pPr");
        let style = properties
            .and_then(|p| p.child(Ns::W, "pStyle"))
            .and_then(|s| s.w_attr("val"));
        paragraph.role = self.role(style, properties);
        let numbering = properties
            .and_then(|p| p.child(Ns::W, "numPr"))
            .map(|n| {
                (
                    n.child(Ns::W, "numId")
                        .and_then(|e| e.w_attr("val"))
                        .map(str::to_owned),
                    n.child(Ns::W, "ilvl")
                        .and_then(|e| e.w_attr("val"))
                        .map(str::to_owned),
                )
            })
            .or_else(|| self.style_numbering(style));
        paragraph.list = numbering.and_then(|(id, level)| self.list(id, level));
        paragraph.align = properties
            .and_then(|p| p.child(Ns::W, "jc"))
            .and_then(|jc| jc.w_attr("val"))
            .and_then(alignment)
            .or_else(|| self.style_alignment(style));
        if let Some(properties) = properties {
            if properties.child(Ns::W, "sectPr").is_some() {
                self.report
                    .record(WordConstruct::Section, Outcome::Approximated);
            }
            if properties.child(Ns::W, "pPrChange").is_some() {
                self.report
                    .record(WordConstruct::FormatChange, Outcome::Dropped);
            }
            let mark = properties.child(Ns::W, "rPr");
            if mark
                .is_some_and(|m| m.child(Ns::W, "ins").is_some() || m.child(Ns::W, "del").is_some())
            {
                self.report
                    .record(WordConstruct::ParagraphMarkChange, Outcome::Approximated);
            }
        }
        let mut state = InlineState { revision: None };
        self.inlines(
            package,
            context,
            element,
            &mut state,
            &mut paragraph.inlines,
        )?;
        Ok(paragraph)
    }

    fn role(&self, style: Option<&str>, properties: Option<&Element>) -> ParagraphRole {
        let direct = properties
            .and_then(|p| p.child(Ns::W, "outlineLvl"))
            .and_then(|o| o.w_attr("val"))
            .and_then(|v| v.parse::<u8>().ok());
        if let Some(level) = direct.and_then(heading_from_outline) {
            return level;
        }
        let mut seen = 0;
        let mut next = style.map(str::to_owned);
        while let Some(id) = next.take() {
            seen += 1;
            if seen > 16 {
                break;
            }
            let def = self.styles.get(&id);
            let name = def.map_or_else(|| id.to_ascii_lowercase(), |def| def.name.clone());
            if let Some(role) = ParagraphRole::from_style_name(&name) {
                return role;
            }
            if let Some(level) = def
                .and_then(|def| def.outline)
                .and_then(heading_from_outline)
            {
                return level;
            }
            next = def.and_then(|def| def.based_on.clone());
        }
        ParagraphRole::Body
    }

    fn style_chain(&self, style: Option<&str>) -> Vec<&StyleDef> {
        let mut chain = Vec::new();
        let mut next = style;
        while let Some(def) = next.and_then(|id| self.styles.get(id)) {
            if chain.len() > 16 {
                break;
            }
            chain.push(def);
            next = def.based_on.as_deref();
        }
        chain
    }

    fn style_numbering(&self, style: Option<&str>) -> Option<(Option<String>, Option<String>)> {
        self.style_chain(style)
            .into_iter()
            .find_map(|def| def.numbering.clone())
    }

    fn style_alignment(&self, style: Option<&str>) -> Option<Alignment> {
        self.style_chain(style)
            .into_iter()
            .find_map(|def| def.align)
    }

    fn list(&mut self, id: Option<String>, level: Option<String>) -> Option<ListMembership> {
        let id = id?;
        let number: u32 = id.parse().ok()?;
        // `numId` 0 removes the numbering a style gives.
        if number == 0 {
            return None;
        }
        let depth = level.and_then(|l| l.parse::<i64>().ok()).unwrap_or(0);
        let level = Bounded::<ListDepth>::saturating_from(depth);
        let defined = self
            .numbering
            .get(&id)
            .and_then(|levels| levels.get(usize::from(level.get())).copied().flatten());
        let kind = match defined {
            // A level that shows nothing makes no list.
            Some(ListLevel { kind: None, .. }) => return None,
            Some(ListLevel {
                kind: Some(kind),
                exact,
            }) => {
                if exact == Exactness::Approximated {
                    self.report
                        .record(WordConstruct::ListFormat, Outcome::Approximated);
                }
                kind
            }
            None => {
                self.report
                    .record(WordConstruct::ListFormat, Outcome::Approximated);
                ListKind::Bullet
            }
        };
        Some(ListMembership {
            list: WordId::new(number),
            level,
            kind,
        })
    }

    fn table(
        &mut self,
        package: &mut Package<'_>,
        context: &PartContext,
        element: &Element,
    ) -> Result<Table, DocxError> {
        self.report.record(WordConstruct::Table, Outcome::Carried);
        let columns = element
            .child(Ns::W, "tblGrid")
            .into_iter()
            .flat_map(Element::elements)
            .filter(|e| e.is(Ns::W, "gridCol"))
            .map(|col| twips(col.w_attr("w")).unwrap_or(PrintLength::ZERO))
            .collect();
        let mut rows = Vec::new();
        for row in unwrap_content(element, "tr") {
            let role = match row
                .child(Ns::W, "trPr")
                .and_then(|p| p.child(Ns::W, "tblHeader"))
            {
                Some(header) if on_off(header.w_attr("val")) => RowRole::Header,
                _ => RowRole::Body,
            };
            let mut cells = Vec::new();
            for cell in unwrap_content(row, "tc") {
                let properties = cell.child(Ns::W, "tcPr");
                let span = properties
                    .and_then(|p| p.child(Ns::W, "gridSpan"))
                    .and_then(|s| s.w_attr("val"))
                    .and_then(|v| v.parse::<i64>().ok())
                    .map_or(
                        Bounded::<GridSpan>::MIN,
                        Bounded::<GridSpan>::saturating_from,
                    );
                let merge = match properties.and_then(|p| p.child(Ns::W, "vMerge")) {
                    None => VerticalMerge::None,
                    Some(merge) if merge.w_attr("val") == Some("restart") => VerticalMerge::Restart,
                    Some(_) => VerticalMerge::Continue,
                };
                let fill = properties
                    .and_then(|p| p.child(Ns::W, "shd"))
                    .and_then(|s| s.w_attr("fill"))
                    .and_then(hex_color);
                let blocks = self.blocks(package, context, cell)?;
                cells.push(TableCell {
                    span,
                    merge,
                    fill,
                    blocks,
                });
            }
            rows.push(TableRow { role, cells });
        }
        Ok(Table { columns, rows })
    }

    /// The inlines under `parent`, a paragraph or a container inside one.
    fn inlines(
        &mut self,
        package: &mut Package<'_>,
        context: &PartContext,
        parent: &Element,
        state: &mut InlineState,
        out: &mut Vec<Inline>,
    ) -> Result<(), DocxError> {
        for element in parent.elements() {
            if element.ns != Ns::W {
                if element.is(Ns::Mc, "AlternateContent") {
                    if let Some(branch) = alternate(element) {
                        self.inlines(package, context, branch, state, out)?;
                    }
                } else if element.ns == Ns::Math {
                    self.report
                        .record(WordConstruct::Equation, Outcome::Dropped);
                }
                continue;
            }
            match element.name() {
                "r" => self.run(package, context, element, state, out)?,
                "ins" | "del" | "moveTo" | "moveFrom" => {
                    let kind = match element.name() {
                        "ins" | "moveTo" => RevisionKind::Insertion,
                        _ => RevisionKind::Deletion,
                    };
                    let moved = element.name().starts_with("move");
                    self.report.record(
                        if moved {
                            WordConstruct::TrackedMove
                        } else {
                            WordConstruct::TrackedChange
                        },
                        if moved {
                            Outcome::Approximated
                        } else {
                            Outcome::Carried
                        },
                    );
                    let revision = Revision {
                        kind,
                        id: WordId::new(element.w_attr("id").and_then(decimal_id).unwrap_or(0)),
                        author: element.w_attr("author").unwrap_or_default().to_owned(),
                        date: time::word_date(
                            element.w_attr("date"),
                            element.attr(Ns::W16du, "dateUtc"),
                        ),
                    };
                    let outer = state.revision.replace(revision);
                    self.inlines(package, context, element, state, out)?;
                    state.revision = outer;
                }
                "hyperlink" => {
                    let mut inner = Vec::new();
                    self.inlines(package, context, element, state, &mut inner)?;
                    let target = element
                        .attr(Ns::R, "id")
                        .and_then(|id| context.rel(id))
                        .and_then(|rel| match &rel.target {
                            RelTarget::External(address) => {
                                Some(LinkTarget::External(LinkUrl::new(address.clone())))
                            }
                            RelTarget::Part(_) => None,
                        })
                        .or_else(|| {
                            element
                                .w_attr("anchor")
                                .map(|anchor| LinkTarget::Anchor(anchor.to_owned()))
                        });
                    match target {
                        Some(target) => {
                            let (runs, rest): (Vec<Inline>, Vec<Inline>) = inner
                                .into_iter()
                                .partition(|inline| matches!(inline, Inline::Run(_)));
                            let runs = runs
                                .into_iter()
                                .filter_map(|inline| match inline {
                                    Inline::Run(run) => Some(run),
                                    _ => None,
                                })
                                .collect::<Vec<_>>();
                            if !runs.is_empty() {
                                out.push(Inline::Link(Link { target, runs }));
                            }
                            out.extend(rest);
                        }
                        None => out.extend(inner),
                    }
                }
                "fldSimple" => {
                    self.report
                        .record(WordConstruct::Field, Outcome::Approximated);
                    self.inlines(package, context, element, state, out)?;
                }
                "sdt" => {
                    self.report
                        .record(WordConstruct::ContentControl, Outcome::Approximated);
                    if let Some(content) = element.child(Ns::W, "sdtContent") {
                        self.inlines(package, context, content, state, out)?;
                    }
                }
                "smartTag" | "customXml" | "dir" | "bdo" => {
                    self.inlines(package, context, element, state, out)?;
                }
                "commentRangeStart" => {
                    if let Some(id) = element.w_attr("id").and_then(decimal_id) {
                        let id = WordId::new(id);
                        self.comments_started.insert(id);
                        out.push(Inline::CommentStart(id));
                    }
                }
                "commentRangeEnd" => {
                    if let Some(id) = element.w_attr("id").and_then(decimal_id) {
                        out.push(Inline::CommentEnd(WordId::new(id)));
                    }
                }
                "bookmarkStart" => self.bookmark(element),
                "pPr"
                | "bookmarkEnd"
                | "proofErr"
                | "permStart"
                | "permEnd"
                | "moveFromRangeStart"
                | "moveFromRangeEnd"
                | "moveToRangeStart"
                | "moveToRangeEnd"
                | "customXmlInsRangeStart"
                | "customXmlInsRangeEnd"
                | "customXmlDelRangeStart"
                | "customXmlDelRangeEnd" => {}
                _ => {
                    self.report
                        .record(WordConstruct::UnknownElement, Outcome::Dropped);
                    self.inlines(package, context, element, state, out)?;
                }
            }
        }
        Ok(())
    }

    fn run(
        &mut self,
        package: &mut Package<'_>,
        context: &PartContext,
        element: &Element,
        state: &InlineState,
        out: &mut Vec<Inline>,
    ) -> Result<(), DocxError> {
        let properties = element.child(Ns::W, "rPr");
        if properties.is_some_and(|p| p.child(Ns::W, "rPrChange").is_some()) {
            self.report
                .record(WordConstruct::FormatChange, Outcome::Dropped);
        }
        let format = self.run_format(properties);
        let mut text = String::new();
        let flush = |text: &mut String, out: &mut Vec<Inline>| {
            if !text.is_empty() {
                out.push(Inline::Run(Run {
                    text: std::mem::take(text),
                    format: format.clone(),
                    revision: state.revision.clone(),
                }));
            }
        };
        for child in element.elements() {
            if child.ns != Ns::W {
                if child.is(Ns::Mc, "AlternateContent") {
                    // A run's alternate content is a drawing or a symbol.
                    self.report.record(WordConstruct::Drawing, Outcome::Dropped);
                }
                continue;
            }
            if child.name() == "fldChar" {
                match child.w_attr("fldCharType") {
                    Some("begin") => {
                        self.report
                            .record(WordConstruct::Field, Outcome::Approximated);
                        self.fields.push(FieldPhase::Instruction);
                    }
                    Some("separate") => {
                        if let Some(top) = self.fields.last_mut() {
                            *top = FieldPhase::Result;
                        }
                    }
                    Some("end") => {
                        self.fields.pop();
                    }
                    _ => {}
                }
                continue;
            }
            // The instruction of a field is no text the user reads.
            if self.fields.contains(&FieldPhase::Instruction) {
                continue;
            }
            match child.name() {
                "t" | "delText" => text.push_str(&child.text()),
                "tab" | "ptab" => {
                    flush(&mut text, out);
                    out.push(Inline::Tab);
                }
                "br" => {
                    flush(&mut text, out);
                    out.push(Inline::Break(match child.w_attr("type") {
                        Some("page") => BreakKind::Page,
                        Some("column") => BreakKind::Column,
                        _ => BreakKind::Line,
                    }));
                }
                "cr" => {
                    flush(&mut text, out);
                    out.push(Inline::Break(BreakKind::Line));
                }
                "noBreakHyphen" => text.push('\u{2011}'),
                "softHyphen" => text.push('\u{AD}'),
                "footnoteReference" | "endnoteReference" => {
                    flush(&mut text, out);
                    let kind = if child.name() == "footnoteReference" {
                        NoteKind::Footnote
                    } else {
                        NoteKind::Endnote
                    };
                    if let Some(id) = child.w_attr("id").and_then(decimal_id) {
                        out.push(Inline::NoteCall(kind, WordId::new(id)));
                    }
                }
                "commentReference" => {
                    if let Some(id) = child.w_attr("id").and_then(decimal_id) {
                        let id = WordId::new(id);
                        if self.comments_started.insert(id) {
                            flush(&mut text, out);
                            out.push(Inline::CommentStart(id));
                            out.push(Inline::CommentEnd(id));
                        }
                    }
                }
                "drawing" => {
                    flush(&mut text, out);
                    if let Some(image) = self.drawing(package, context, child)? {
                        out.push(Inline::Image(image));
                    }
                }
                "pict" => self.report.record(WordConstruct::Drawing, Outcome::Dropped),
                "object" => self
                    .report
                    .record(WordConstruct::EmbeddedObject, Outcome::Dropped),
                "sym" => self
                    .report
                    .record(WordConstruct::UnknownElement, Outcome::Dropped),
                "rPr"
                | "instrText"
                | "delInstrText"
                | "lastRenderedPageBreak"
                | "footnoteRef"
                | "endnoteRef"
                | "annotationRef"
                | "separator"
                | "continuationSeparator" => {}
                _ => {
                    self.report
                        .record(WordConstruct::UnknownElement, Outcome::Dropped);
                    text.push_str(&child.text());
                }
            }
        }
        flush(&mut text, out);
        Ok(())
    }

    fn drawing(
        &mut self,
        package: &mut Package<'_>,
        context: &PartContext,
        drawing: &Element,
    ) -> Result<Option<Image>, DocxError> {
        let Some(frame) = drawing
            .child(Ns::Wp, "inline")
            .or_else(|| drawing.child(Ns::Wp, "anchor"))
        else {
            self.report.record(WordConstruct::Drawing, Outcome::Dropped);
            return Ok(None);
        };
        let picture = frame
            .child(Ns::A, "graphic")
            .and_then(|g| g.child(Ns::A, "graphicData"))
            .and_then(|d| d.child(Ns::Pic, "pic"));
        let Some(picture) = picture else {
            self.report.record(WordConstruct::Drawing, Outcome::Dropped);
            return Ok(None);
        };
        let blip = picture
            .child(Ns::Pic, "blipFill")
            .and_then(|f| f.child(Ns::A, "blip"));
        let Some(part) = blip
            .and_then(|b| b.attr(Ns::R, "embed"))
            .and_then(|id| context.part(id, rel::IMAGE))
        else {
            let linked = blip.is_some_and(|b| b.attr(Ns::R, "link").is_some());
            self.report.record(
                if linked {
                    WordConstruct::LinkedPicture
                } else {
                    WordConstruct::Drawing
                },
                Outcome::Dropped,
            );
            return Ok(None);
        };
        let format = package
            .content_type(&part)
            .and_then(|t| t.strip_prefix("image/"))
            .and_then(|t| match t {
                "svg+xml" => Some(ImageFormat::Svg),
                other => ImageFormat::from_extension(other),
            })
            .or_else(|| part.extension().and_then(ImageFormat::from_extension));
        let Some(format) = format else {
            self.report
                .record(WordConstruct::PictureFormat, Outcome::Dropped);
            return Ok(None);
        };
        let Some(bytes) = package.part(&part)? else {
            self.report.record(WordConstruct::Drawing, Outcome::Dropped);
            return Ok(None);
        };
        if frame.name() == "anchor" {
            self.report
                .record(WordConstruct::FloatingPicture, Outcome::Approximated);
        }
        let extent = frame.child(Ns::Wp, "extent");
        let length = |name: &str| {
            extent
                .and_then(|e| e.attr(Ns::None, name))
                .and_then(|v| v.parse::<i64>().ok())
                .map_or(PrintLength::ZERO, emu)
        };
        let properties = frame.child(Ns::Wp, "docPr");
        let description = properties
            .and_then(|p| {
                p.attr(Ns::None, "descr")
                    .or_else(|| p.attr(Ns::None, "title"))
            })
            .unwrap_or_default()
            .to_owned();
        self.report.record(WordConstruct::Picture, Outcome::Carried);
        Ok(Some(Image {
            description: ambre_core::Validated::new(description),
            width: length("cx"),
            height: length("cy"),
            format,
            bytes: bytes.into_role(),
        }))
    }

    fn run_format(&mut self, properties: Option<&Element>) -> RunFormat {
        let mut props = RunProps::default();
        let style = properties
            .and_then(|p| p.child(Ns::W, "rStyle"))
            .and_then(|s| s.w_attr("val"));
        let chain = self.style_chain(style);
        // The base of the chain applies first, so a style overrides the
        // style it is based on.
        for def in chain.iter().rev() {
            if !def.structural {
                props.overlay(&def.run);
            }
        }
        if let Some(properties) = properties {
            props.overlay(&RunProps::read(properties));
        }
        let effects = [
            (props.small_caps, WordConstruct::SmallCaps, Outcome::Dropped),
            (props.caps, WordConstruct::AllCaps, Outcome::Dropped),
            (
                props.vanish,
                WordConstruct::HiddenText,
                Outcome::Approximated,
            ),
            (
                props.double_strike,
                WordConstruct::DoubleStrike,
                Outcome::Approximated,
            ),
        ];
        for (on, construct, outcome) in effects {
            if on == Some(true) {
                self.report.record(construct, outcome);
            }
        }
        props.into_format()
    }
}

/// The revision the runs under a container belong to.
struct InlineState {
    revision: Option<Revision>,
}

/// The children of `parent` named `name`, through the content controls and
/// the custom XML a table wraps its rows and its cells in.
fn unwrap_content<'e>(parent: &'e Element, name: &str) -> Vec<&'e Element> {
    let mut out = Vec::new();
    let mut stack: Vec<&Element> = parent.elements().collect();
    stack.reverse();
    while let Some(element) = stack.pop() {
        if element.is(Ns::W, name) {
            out.push(element);
        } else if element.is(Ns::W, "sdt") {
            if let Some(content) = element.child(Ns::W, "sdtContent") {
                stack.extend(content.elements().collect::<Vec<_>>().into_iter().rev());
            }
        } else if element.is(Ns::W, "customXml") {
            stack.extend(element.elements().collect::<Vec<_>>().into_iter().rev());
        }
    }
    out
}

/// The branch of a markup compatibility block the reader takes: the
/// fallback, which needs no extension, or else the first choice.
fn alternate(element: &Element) -> Option<&Element> {
    element
        .child(Ns::Mc, "Fallback")
        .or_else(|| element.child(Ns::Mc, "Choice"))
}

/// A decimal id the file spells. A negative id names a separator note,
/// which no call names.
fn decimal_id(text: &str) -> Option<u32> {
    text.trim().parse().ok()
}

fn heading_from_outline(level: u8) -> Option<ParagraphRole> {
    // Outline level 9 is body text.
    Bounded::<HeadingDepth>::new(level.checked_add(1)?).map(ParagraphRole::Heading)
}

fn alignment(value: &str) -> Option<Alignment> {
    match value {
        "left" | "start" => Some(Alignment::Start),
        "center" => Some(Alignment::Center),
        "right" | "end" => Some(Alignment::End),
        "both" | "distribute" | "mediumKashida" | "highKashida" | "lowKashida"
        | "thaiDistribute" => Some(Alignment::Justified),
        _ => None,
    }
}

/// A WordprocessingML on-off value: absent, `1`, `true` and `on` are on.
fn on_off(value: Option<&str>) -> bool {
    !matches!(value, Some("0" | "false" | "off" | "none"))
}

fn twips(value: Option<&str>) -> Option<PrintLength<Pt>> {
    let twips: i64 = value?.trim().parse().ok()?;
    // A twip count fits an f32 exactly up to 2^24, past any page.
    #[expect(clippy::cast_precision_loss, reason = "a twip count of a page fits")]
    Some(PrintLength::new(twips as f32 / TWIPS_PER_PT))
}

fn emu(value: i64) -> PrintLength<Pt> {
    #[expect(clippy::cast_precision_loss, reason = "a picture's EMU count fits")]
    PrintLength::new(value as f32 / EMU_PER_PT)
}

fn hex_color(value: &str) -> Option<Rgb> {
    if value.len() != 6 {
        return None;
    }
    u32::from_str_radix(value, 16).ok().map(Rgb::from_bits)
}

/// The colour of a named highlight, `ST_HighlightColor`.
fn highlight(value: &str) -> Option<Rgb> {
    let bits = match value {
        "black" => 0x00_0000,
        "blue" => 0x00_00FF,
        "cyan" => 0x00_FFFF,
        "green" => 0x00_FF00,
        "magenta" => 0xFF_00FF,
        "red" => 0xFF_0000,
        "yellow" => 0xFF_FF00,
        "white" => 0xFF_FFFF,
        "darkBlue" => 0x00_0080,
        "darkCyan" => 0x00_8080,
        "darkGreen" => 0x00_8000,
        "darkMagenta" => 0x80_0080,
        "darkRed" => 0x80_0000,
        "darkYellow" => 0x80_8000,
        "darkGray" => 0x80_8080,
        "lightGray" => 0xC0_C0C0,
        _ => return None,
    };
    Some(Rgb::from_bits(bits))
}

fn page_setup(section: &Element) -> Option<PageSetup> {
    let size = section.child(Ns::W, "pgSz")?;
    let margins = section.child(Ns::W, "pgMar");
    let margin = |name: &str| {
        margins
            .and_then(|m| twips(m.w_attr(name)))
            .unwrap_or(PrintLength::ZERO)
    };
    Some(PageSetup {
        width: twips(size.w_attr("w")).unwrap_or(PrintLength::ZERO),
        height: twips(size.w_attr("h")).unwrap_or(PrintLength::ZERO),
        margins: Margins {
            top: margin("top"),
            bottom: margin("bottom"),
            left: margin("left"),
            right: margin("right"),
        },
        orientation: match size.w_attr("orient") {
            Some("landscape") => Orientation::Landscape,
            _ => Orientation::Portrait,
        },
    })
}

/// The run properties one style or one run sets: a property it leaves
/// unset lets the level beneath show.
#[derive(Clone, Default)]
struct RunProps {
    bold: Option<Weight>,
    italic: Option<Slant>,
    underline: Option<Underline>,
    strike: Option<Strikethrough>,
    position: Option<VerticalPosition>,
    size: Option<FontSize>,
    color: Option<Rgb>,
    fill: Option<Rgb>,
    font: Option<String>,
    /// The run effects the model has no form for: the reader names each in
    /// its report rather than drop it in silence.
    small_caps: Option<bool>,
    caps: Option<bool>,
    vanish: Option<bool>,
    double_strike: Option<bool>,
}

impl RunProps {
    fn read(properties: &Element) -> Self {
        let toggle = |name: &str| {
            properties
                .child(Ns::W, name)
                .map(|e| on_off(e.w_attr("val")))
        };
        let highlight_fill = properties
            .child(Ns::W, "highlight")
            .and_then(|h| h.w_attr("val"))
            .and_then(highlight);
        let shading_fill = properties
            .child(Ns::W, "shd")
            .and_then(|s| s.w_attr("fill"))
            .and_then(hex_color);
        Self {
            bold: toggle("b").map(Weight::from_bool),
            italic: toggle("i").map(Slant::from_bool),
            underline: properties
                .child(Ns::W, "u")
                .map(|u| Underline::from_bool(u.w_attr("val") != Some("none"))),
            small_caps: toggle("smallCaps"),
            caps: toggle("caps"),
            vanish: toggle("vanish"),
            double_strike: toggle("dstrike"),
            strike: match (toggle("strike"), toggle("dstrike")) {
                (None, None) => None,
                (a, b) => Some(Strikethrough::from_bool(
                    a.unwrap_or(false) || b.unwrap_or(false),
                )),
            },
            position: properties
                .child(Ns::W, "vertAlign")
                .and_then(|v| v.w_attr("val"))
                .map(|v| match v {
                    "superscript" => VerticalPosition::Superscript,
                    "subscript" => VerticalPosition::Subscript,
                    _ => VerticalPosition::Baseline,
                }),
            size: properties
                .child(Ns::W, "sz")
                .and_then(|s| s.w_attr("val"))
                .and_then(|v| v.parse::<u16>().ok())
                .and_then(|half_points| FontSize::new(f32::from(half_points) / 2.0)),
            color: properties
                .child(Ns::W, "color")
                .and_then(|c| c.w_attr("val"))
                .and_then(hex_color),
            fill: highlight_fill.or(shading_fill),
            font: properties
                .child(Ns::W, "rFonts")
                .and_then(|f| f.w_attr("ascii").or_else(|| f.w_attr("hAnsi")))
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        }
    }

    fn overlay(&mut self, top: &Self) {
        self.bold = top.bold.or(self.bold);
        self.italic = top.italic.or(self.italic);
        self.underline = top.underline.or(self.underline);
        self.strike = top.strike.or(self.strike);
        self.position = top.position.or(self.position);
        self.size = top.size.or(self.size);
        self.color = top.color.or(self.color);
        self.fill = top.fill.or(self.fill);
        self.small_caps = top.small_caps.or(self.small_caps);
        self.caps = top.caps.or(self.caps);
        self.vanish = top.vanish.or(self.vanish);
        self.double_strike = top.double_strike.or(self.double_strike);
        if top.font.is_some() {
            self.font.clone_from(&top.font);
        }
    }

    fn into_format(self) -> RunFormat {
        RunFormat {
            bold: self.bold.unwrap_or_default(),
            italic: self.italic.unwrap_or_default(),
            underline: self.underline.unwrap_or_default(),
            strike: self.strike.unwrap_or_default(),
            position: self.position.unwrap_or_default(),
            size: self.size,
            color: self.color,
            fill: self.fill,
            font: self.font,
        }
    }
}

/// One style of `styles.xml`, as the reader resolves it.
struct StyleDef {
    /// The name, in lower case.
    name: String,
    based_on: Option<String>,
    /// The outline level, 0 for a first-level heading.
    outline: Option<u8>,
    /// The `numId` and the `ilvl` the style gives its paragraphs.
    numbering: Option<(Option<String>, Option<String>)>,
    align: Option<Alignment>,
    run: RunProps,
    /// A character style that draws a structure the model already holds: a
    /// link, a note call, a comment mark. Its look stays off the run.
    structural: bool,
}

fn read_styles(root: &Element) -> BTreeMap<String, StyleDef> {
    root.elements()
        .filter(|e| e.is(Ns::W, "style"))
        .filter_map(|style| {
            let id = style.w_attr("styleId")?.to_owned();
            let name = style
                .child(Ns::W, "name")
                .and_then(|n| n.w_attr("val"))
                .unwrap_or(&id)
                .to_ascii_lowercase();
            let paragraph = style.child(Ns::W, "pPr");
            let structural = matches!(
                name.as_str(),
                "hyperlink"
                    | "followedhyperlink"
                    | "footnote reference"
                    | "endnote reference"
                    | "annotation reference"
                    | "comment reference"
                    | "internet link"
                    | "footnote anchor"
                    | "endnote anchor"
            );
            let def = StyleDef {
                based_on: style
                    .child(Ns::W, "basedOn")
                    .and_then(|b| b.w_attr("val"))
                    .map(str::to_owned),
                outline: paragraph
                    .and_then(|p| p.child(Ns::W, "outlineLvl"))
                    .and_then(|o| o.w_attr("val"))
                    .and_then(|v| v.parse().ok()),
                numbering: paragraph.and_then(|p| p.child(Ns::W, "numPr")).map(|n| {
                    (
                        n.child(Ns::W, "numId")
                            .and_then(|e| e.w_attr("val"))
                            .map(str::to_owned),
                        n.child(Ns::W, "ilvl")
                            .and_then(|e| e.w_attr("val"))
                            .map(str::to_owned),
                    )
                }),
                align: paragraph
                    .and_then(|p| p.child(Ns::W, "jc"))
                    .and_then(|jc| jc.w_attr("val"))
                    .and_then(alignment),
                run: style
                    .child(Ns::W, "rPr")
                    .map(RunProps::read)
                    .unwrap_or_default(),
                structural,
                name,
            };
            Some((id, def))
        })
        .collect()
}

fn read_numbering(root: &Element) -> BTreeMap<String, [Option<ListLevel>; 9]> {
    let abstracts: BTreeMap<&str, [Option<ListLevel>; 9]> = root
        .elements()
        .filter(|e| e.is(Ns::W, "abstractNum"))
        .filter_map(|definition| {
            let id = definition.w_attr("abstractNumId")?;
            let mut levels = [None; 9];
            for level in definition.elements().filter(|e| e.is(Ns::W, "lvl")) {
                let Some(slot) = level
                    .w_attr("ilvl")
                    .and_then(|l| l.parse::<usize>().ok())
                    .and_then(|at| levels.get_mut(at))
                else {
                    continue;
                };
                let format = level
                    .child(Ns::W, "numFmt")
                    .and_then(|f| f.w_attr("val"))
                    .unwrap_or("decimal");
                *slot = Some(list_level(format));
            }
            Some((id, levels))
        })
        .collect();
    root.elements()
        .filter(|e| e.is(Ns::W, "num"))
        .filter_map(|num| {
            let id = num.w_attr("numId")?.to_owned();
            let definition = num
                .child(Ns::W, "abstractNumId")
                .and_then(|a| a.w_attr("val"))?;
            Some((id, *abstracts.get(definition)?))
        })
        .collect()
}

fn list_level(format: &str) -> ListLevel {
    let exact = |kind| ListLevel {
        kind: Some(kind),
        exact: Exactness::Exact,
    };
    match format {
        "bullet" => exact(ListKind::Bullet),
        "decimal" => exact(ListKind::Numbered(NumberFormat::Decimal)),
        "lowerLetter" => exact(ListKind::Numbered(NumberFormat::LowerLetter)),
        "upperLetter" => exact(ListKind::Numbered(NumberFormat::UpperLetter)),
        "lowerRoman" => exact(ListKind::Numbered(NumberFormat::LowerRoman)),
        "upperRoman" => exact(ListKind::Numbered(NumberFormat::UpperRoman)),
        "none" => ListLevel {
            kind: None,
            exact: Exactness::Exact,
        },
        _ => ListLevel {
            kind: Some(ListKind::Numbered(NumberFormat::Decimal)),
            exact: Exactness::Approximated,
        },
    }
}
