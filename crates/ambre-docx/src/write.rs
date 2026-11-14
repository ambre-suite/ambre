//! The writer: a [`WordDocument`] into a `.docx` package.
//!
//! The writer writes every construct the model holds, so it loses nothing
//! and reports nothing. An approximation happens in the mapping from an
//! application's form, which names it. The writer numbers every id again
//! first (`WordDocument::renumbered`). So the same document writes the same
//! file, and a reader reads the renumbered document back.

use std::collections::BTreeMap;

use ambre_core::{Becomes, FontSize, Payload, PrintLength, Pt, Rgb};

use crate::DocxError;
use crate::model::{
    Alignment, Block, BreakKind, Comment, CommentState, Image, Inline, Link, LinkTarget, ListKind,
    ListMembership, Note, NoteKind, NumberFormat, Orientation, PageSetup, Paragraph, ParagraphRole,
    RevisionKind, RowRole, Run, RunFormat, Table, VerticalMerge, VerticalPosition, WordDocument,
};
use crate::model::{ImageBytes, ListInstance, WordDate, WordId};
use crate::opc::{
    self, PackageBytes, PackageWriter, PartBytes, PartName, RelId, RelKind, RelTarget,
};
use crate::rel;
use crate::time;
use crate::xml::XmlWriter;

const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
const NS_W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const NS_W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const NS_W16DU: &str = "http://schemas.microsoft.com/office/word/2023/wordml/word16du";

const TYPE_DOCUMENT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const TYPE_STYLES: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
const TYPE_SETTINGS: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";
const TYPE_NUMBERING: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
const TYPE_FOOTNOTES: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";
const TYPE_ENDNOTES: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml";
const TYPE_COMMENTS: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
const TYPE_COMMENTS_EXTENDED: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
const TYPE_HEADER: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
const TYPE_FOOTER: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
const TYPE_CORE: &str = "application/vnd.openxmlformats-package.core-properties+xml";

// A picture the model holds becomes a part of the package as it is.
impl Becomes<PartBytes> for ImageBytes {}

// A part read becomes a picture of the model as it is.
impl Becomes<ImageBytes> for PartBytes {}

/// The EMUs in one point.
const EMU_PER_PT: f32 = 12_700.0;

/// The twips in one point.
const TWIPS_PER_PT: f32 = 20.0;

/// The `.docx` package of `document`.
///
/// # Errors
///
/// [`DocxError::Package`] when the ZIP writer fails.
pub fn write(document: &WordDocument) -> Result<Payload<PackageBytes>, DocxError> {
    let document = document.renumbered();
    let mut package = PackageWriter::new();
    let main = part("word/document.xml");
    package.relate(
        &opc::root(),
        RelKind::OFFICE_DOCUMENT,
        RelTarget::Part(main.clone()),
    );
    let core = part("docProps/core.xml");
    package.relate(
        &opc::root(),
        RelKind::CORE_PROPERTIES,
        RelTarget::Part(core.clone()),
    );
    package.part(core, Some(TYPE_CORE), core_properties(&document));

    let mut context = Context {
        package,
        source: main.clone(),
        images: 0,
        paragraphs: 0,
        lists: BTreeMap::new(),
    };
    context.relate_part(rel::STYLES, "word/styles.xml", TYPE_STYLES, styles());
    context.relate_part(
        rel::SETTINGS,
        "word/settings.xml",
        TYPE_SETTINGS,
        settings(&document),
    );

    let header = document
        .header
        .as_ref()
        .map(|blocks| context.band(rel::HEADER, "header1.xml", TYPE_HEADER, blocks));
    let footer = document
        .footer
        .as_ref()
        .map(|blocks| context.band(rel::FOOTER, "footer1.xml", TYPE_FOOTER, blocks));
    if !document.footnotes.is_empty() {
        let bytes = context.notes(NoteKind::Footnote, &document.footnotes);
        context.relate_part(rel::FOOTNOTES, "word/footnotes.xml", TYPE_FOOTNOTES, bytes);
    }
    if !document.endnotes.is_empty() {
        let bytes = context.notes(NoteKind::Endnote, &document.endnotes);
        context.relate_part(rel::ENDNOTES, "word/endnotes.xml", TYPE_ENDNOTES, bytes);
    }
    if !document.comments.is_empty() {
        let (comments, extended) = context.comments(&document.comments);
        context.relate_part(rel::COMMENTS, "word/comments.xml", TYPE_COMMENTS, comments);
        context.relate_part(
            rel::COMMENTS_EXTENDED,
            "word/commentsExtended.xml",
            TYPE_COMMENTS_EXTENDED,
            extended,
        );
    }

    let mut xml = XmlWriter::new();
    xml.open("w:document", &namespaces());
    xml.open("w:body", &[]);
    context.blocks(&mut xml, &document.body);
    section(
        &mut xml,
        document.page.as_ref(),
        header.as_ref(),
        footer.as_ref(),
    );
    xml.close("w:body");
    xml.close("w:document");
    let body = xml.finish();

    if !context.lists.is_empty() {
        let bytes = numbering(&context.lists);
        context.relate_part(rel::NUMBERING, "word/numbering.xml", TYPE_NUMBERING, bytes);
    }
    let mut package = context.package;
    package.part(main, Some(TYPE_DOCUMENT), body);
    Ok(package.finish()?)
}

fn part(name: &str) -> PartName {
    // Every name below is a literal of this module, so none climbs out.
    PartName::new(name).unwrap_or_else(opc::root)
}

/// The namespaces every part of WordprocessingML declares on its root.
fn namespaces() -> [(&'static str, &'static str); 10] {
    [
        ("xmlns:w", NS_W),
        ("xmlns:r", NS_R),
        ("xmlns:wp", NS_WP),
        ("xmlns:a", NS_A),
        ("xmlns:pic", NS_PIC),
        ("xmlns:w14", NS_W14),
        ("xmlns:w15", NS_W15),
        ("xmlns:w16du", NS_W16DU),
        ("xmlns:mc", NS_MC),
        ("mc:Ignorable", "w14 w15 w16du"),
    ]
}

/// The writer's state across the parts of one package.
struct Context {
    package: PackageWriter,
    /// The part whose relationships a hyperlink or a picture joins.
    source: PartName,
    images: u32,
    /// The paragraph ids handed out, for `w14:paraId`.
    paragraphs: u32,
    /// The kind of each level of each list, as its paragraphs first gave it.
    lists: BTreeMap<WordId<ListInstance>, [Option<ListKind>; 9]>,
}

impl Context {
    fn relate_part(
        &mut self,
        kind: RelKind,
        name: &str,
        content_type: &str,
        bytes: Payload<PartBytes>,
    ) {
        let name = part(name);
        let main = part("word/document.xml");
        self.package
            .relate(&main, kind, RelTarget::Part(name.clone()));
        self.package.part(name, Some(content_type), bytes);
    }

    fn band(&mut self, kind: RelKind, file: &str, content_type: &str, blocks: &[Block]) -> RelId {
        let name = part(&format!("word/{file}"));
        let outer = std::mem::replace(&mut self.source, name.clone());
        let mut xml = XmlWriter::new();
        let root = if kind == rel::HEADER {
            "w:hdr"
        } else {
            "w:ftr"
        };
        xml.open(root, &namespaces());
        self.blocks(&mut xml, blocks);
        xml.close(root);
        self.source = outer;
        let main = part("word/document.xml");
        let id = self
            .package
            .relate(&main, kind, RelTarget::Part(name.clone()));
        self.package.part(name, Some(content_type), xml.finish());
        id
    }

    fn notes(&mut self, kind: NoteKind, notes: &[Note]) -> Payload<PartBytes> {
        let (root, element, file, reference) = match kind {
            NoteKind::Footnote => (
                "w:footnotes",
                "w:footnote",
                "word/footnotes.xml",
                "w:footnoteRef",
            ),
            NoteKind::Endnote => (
                "w:endnotes",
                "w:endnote",
                "word/endnotes.xml",
                "w:endnoteRef",
            ),
        };
        let outer = std::mem::replace(&mut self.source, part(file));
        let mut xml = XmlWriter::new();
        xml.open(root, &namespaces());
        // The two separator notes Word draws above the notes, which
        // `settings.xml` names.
        for (id, separator) in [("-1", "separator"), ("0", "continuationSeparator")] {
            xml.open(element, &[("w:type", separator), ("w:id", id)]);
            xml.open("w:p", &[]);
            xml.open("w:r", &[]);
            xml.empty(&format!("w:{separator}"), &[]);
            xml.close("w:r");
            xml.close("w:p");
            xml.close(element);
        }
        let style = match kind {
            NoteKind::Footnote => ("FootnoteReference", "FootnoteText"),
            NoteKind::Endnote => ("EndnoteReference", "EndnoteText"),
        };
        for note in notes {
            let id = note.id.get().to_string();
            xml.open(element, &[("w:id", &id)]);
            // The number of the note opens its first paragraph, as Word
            // writes it; the reader skips it.
            let mark = |xml: &mut XmlWriter| {
                xml.open("w:r", &[]);
                xml.open("w:rPr", &[]);
                xml.empty("w:rStyle", &[("w:val", style.0)]);
                xml.close("w:rPr");
                xml.empty(reference, &[]);
                xml.close("w:r");
            };
            self.blocks_with_mark(&mut xml, &note.blocks, Some(style.1), &mark);
            xml.close(element);
        }
        xml.close(root);
        self.source = outer;
        xml.finish()
    }

    fn comments(&mut self, comments: &[Comment]) -> (Payload<PartBytes>, Payload<PartBytes>) {
        let outer = std::mem::replace(&mut self.source, part("word/comments.xml"));
        let mut xml = XmlWriter::new();
        xml.open("w:comments", &namespaces());
        // The paragraph id of each comment's last paragraph, which the
        // extended part keys a reply's parent and a thread's state by.
        let mut last_paragraph = BTreeMap::new();
        for comment in comments {
            let id = comment.id.get().to_string();
            let dates = date_attrs(comment.date);
            let initials = initials(&comment.author);
            let mut attrs = vec![
                ("w:id", id.as_str()),
                ("w:author", comment.author.as_str()),
                ("w:initials", initials.as_str()),
            ];
            attrs.extend(dates.iter().map(|(k, v)| (*k, v.as_str())));
            xml.open("w:comment", &attrs);
            let paragraphs: Vec<Block> = if comment.paragraphs.is_empty() {
                vec![Block::Paragraph(Paragraph::default())]
            } else {
                comment
                    .paragraphs
                    .iter()
                    .cloned()
                    .map(Block::Paragraph)
                    .collect()
            };
            let mark = |xml: &mut XmlWriter| {
                xml.open("w:r", &[]);
                xml.open("w:rPr", &[]);
                xml.empty("w:rStyle", &[("w:val", "CommentReference")]);
                xml.close("w:rPr");
                xml.empty("w:annotationRef", &[]);
                xml.close("w:r");
            };
            self.blocks_with_mark(&mut xml, &paragraphs, Some("CommentText"), &mark);
            last_paragraph.insert(comment.id, self.paragraphs);
            xml.close("w:comment");
        }
        xml.close("w:comments");
        self.source = outer;

        let mut extended = XmlWriter::new();
        extended.open("w15:commentsEx", &namespaces());
        for comment in comments {
            let Some(paragraph) = last_paragraph.get(&comment.id) else {
                continue;
            };
            let paragraph = paragraph_id(*paragraph);
            let parent = comment
                .parent
                .and_then(|parent| last_paragraph.get(&parent))
                .map(|p| paragraph_id(*p));
            let done = match comment.state {
                CommentState::Open => "0",
                CommentState::Resolved => "1",
            };
            let mut attrs = vec![("w15:paraId", paragraph.as_str())];
            if let Some(parent) = &parent {
                attrs.push(("w15:paraIdParent", parent));
            }
            attrs.push(("w15:done", done));
            extended.empty("w15:commentEx", &attrs);
        }
        extended.close("w15:commentsEx");
        (xml.finish(), extended.finish())
    }

    fn blocks(&mut self, xml: &mut XmlWriter, blocks: &[Block]) {
        self.blocks_with_mark(xml, blocks, None, &|_: &mut XmlWriter| {});
    }

    /// Write `blocks`, the first paragraph opening on `mark` and each
    /// paragraph of the default role taking `style`.
    fn blocks_with_mark(
        &mut self,
        xml: &mut XmlWriter,
        blocks: &[Block],
        style: Option<&str>,
        mark: &dyn Fn(&mut XmlWriter),
    ) {
        let mut first = true;
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => {
                    let opening: &dyn Fn(&mut XmlWriter) = if first { mark } else { &|_| {} };
                    self.paragraph(xml, paragraph, style, opening);
                }
                Block::Table(table) => self.table(xml, table),
            }
            first = false;
        }
    }

    fn paragraph(
        &mut self,
        xml: &mut XmlWriter,
        paragraph: &Paragraph,
        default_style: Option<&str>,
        mark: &dyn Fn(&mut XmlWriter),
    ) {
        self.paragraphs += 1;
        let id = paragraph_id(self.paragraphs);
        xml.open("w:p", &[("w14:paraId", &id), ("w14:textId", &id)]);
        let style = match paragraph.role {
            ParagraphRole::Body => default_style.map(str::to_owned),
            ParagraphRole::Heading(level) => Some(format!("Heading{}", level.get())),
            ParagraphRole::Title => Some("Title".to_owned()),
            ParagraphRole::Quote => Some("Quote".to_owned()),
            ParagraphRole::Code => Some("SourceCode".to_owned()),
        };
        if style.is_some() || paragraph.list.is_some() || paragraph.align.is_some() {
            xml.open("w:pPr", &[]);
            if let Some(style) = &style {
                xml.empty("w:pStyle", &[("w:val", style)]);
            }
            if let Some(list) = &paragraph.list {
                self.list(xml, list);
            }
            if let Some(align) = paragraph.align {
                let value = match align {
                    Alignment::Start => "left",
                    Alignment::Center => "center",
                    Alignment::End => "right",
                    Alignment::Justified => "both",
                };
                xml.empty("w:jc", &[("w:val", value)]);
            }
            xml.close("w:pPr");
        }
        mark(xml);
        for inline in &paragraph.inlines {
            self.inline(xml, inline);
        }
        xml.close("w:p");
    }

    fn list(&mut self, xml: &mut XmlWriter, list: &ListMembership) {
        let levels = self.lists.entry(list.list).or_default();
        if let Some(slot) = levels.get_mut(usize::from(list.level.get())) {
            slot.get_or_insert(list.kind);
        }
        xml.open("w:numPr", &[]);
        xml.empty("w:ilvl", &[("w:val", &list.level.get().to_string())]);
        xml.empty("w:numId", &[("w:val", &list.list.get().to_string())]);
        xml.close("w:numPr");
    }

    fn table(&mut self, xml: &mut XmlWriter, table: &Table) {
        xml.open("w:tbl", &[]);
        xml.open("w:tblPr", &[]);
        xml.empty("w:tblStyle", &[("w:val", "TableGrid")]);
        xml.empty("w:tblW", &[("w:w", "0"), ("w:type", "auto")]);
        xml.close("w:tblPr");
        xml.open("w:tblGrid", &[]);
        for column in &table.columns {
            xml.empty("w:gridCol", &[("w:w", &twips(*column))]);
        }
        xml.close("w:tblGrid");
        for row in &table.rows {
            xml.open("w:tr", &[]);
            if row.role == RowRole::Header {
                xml.open("w:trPr", &[]);
                xml.empty("w:tblHeader", &[]);
                xml.close("w:trPr");
            }
            // A row holds at least one cell, as Word requires.
            let empty = [crate::model::TableCell::default()];
            let cells = if row.cells.is_empty() {
                &empty[..]
            } else {
                &row.cells[..]
            };
            for cell in cells {
                xml.open("w:tc", &[]);
                let span = cell.span.get();
                if span > 1 || cell.merge != VerticalMerge::None || cell.fill.is_some() {
                    xml.open("w:tcPr", &[]);
                    if span > 1 {
                        xml.empty("w:gridSpan", &[("w:val", &span.to_string())]);
                    }
                    match cell.merge {
                        VerticalMerge::None => {}
                        VerticalMerge::Restart => xml.empty("w:vMerge", &[("w:val", "restart")]),
                        VerticalMerge::Continue => xml.empty("w:vMerge", &[]),
                    }
                    if let Some(fill) = cell.fill {
                        xml.empty(
                            "w:shd",
                            &[
                                ("w:val", "clear"),
                                ("w:color", "auto"),
                                ("w:fill", &hex(fill)),
                            ],
                        );
                    }
                    xml.close("w:tcPr");
                }
                self.blocks(xml, &cell.blocks);
                // A cell ends on a paragraph, as Word requires.
                if !matches!(cell.blocks.last(), Some(Block::Paragraph(_))) {
                    self.paragraph(xml, &Paragraph::default(), None, &|_| {});
                }
                xml.close("w:tc");
            }
            xml.close("w:tr");
        }
        xml.close("w:tbl");
    }

    fn inline(&mut self, xml: &mut XmlWriter, inline: &Inline) {
        match inline {
            Inline::Run(run) => write_run(xml, run),
            Inline::Break(kind) => {
                xml.open("w:r", &[]);
                match kind {
                    BreakKind::Line => xml.empty("w:br", &[]),
                    BreakKind::Page => xml.empty("w:br", &[("w:type", "page")]),
                    BreakKind::Column => xml.empty("w:br", &[("w:type", "column")]),
                }
                xml.close("w:r");
            }
            Inline::Tab => {
                xml.open("w:r", &[]);
                xml.empty("w:tab", &[]);
                xml.close("w:r");
            }
            Inline::NoteCall(kind, id) => {
                let (style, element) = match kind {
                    NoteKind::Footnote => ("FootnoteReference", "w:footnoteReference"),
                    NoteKind::Endnote => ("EndnoteReference", "w:endnoteReference"),
                };
                xml.open("w:r", &[]);
                xml.open("w:rPr", &[]);
                xml.empty("w:rStyle", &[("w:val", style)]);
                xml.close("w:rPr");
                xml.empty(element, &[("w:id", &id.get().to_string())]);
                xml.close("w:r");
            }
            Inline::CommentStart(id) => {
                xml.empty("w:commentRangeStart", &[("w:id", &id.get().to_string())]);
            }
            Inline::CommentEnd(id) => {
                let id = id.get().to_string();
                xml.empty("w:commentRangeEnd", &[("w:id", &id)]);
                // The mark Word draws in the margin follows the anchor's end.
                xml.open("w:r", &[]);
                xml.open("w:rPr", &[]);
                xml.empty("w:rStyle", &[("w:val", "CommentReference")]);
                xml.close("w:rPr");
                xml.empty("w:commentReference", &[("w:id", &id)]);
                xml.close("w:r");
            }
            Inline::Link(link) => self.link(xml, link),
            Inline::Image(image) => self.image(xml, image),
        }
    }

    fn link(&mut self, xml: &mut XmlWriter, link: &Link) {
        match &link.target {
            LinkTarget::External(url) => {
                let id = self.package.relate(
                    &self.source,
                    rel::HYPERLINK,
                    RelTarget::External(url.as_str().to_owned()),
                );
                xml.open("w:hyperlink", &[("r:id", id.as_str())]);
            }
            LinkTarget::Anchor(name) => xml.open("w:hyperlink", &[("w:anchor", name)]),
        }
        for run in &link.runs {
            write_run(xml, run);
        }
        xml.close("w:hyperlink");
    }

    fn image(&mut self, xml: &mut XmlWriter, image: &Image) {
        self.images += 1;
        let number = self.images.to_string();
        let name = part(&format!(
            "word/media/image{number}.{}",
            image.format.extension()
        ));
        self.package
            .default_type(image.format.extension(), image.format.media_type());
        self.package
            .part(name.clone(), None, image.bytes.clone().into_role());
        let id = self
            .package
            .relate(&self.source, rel::IMAGE, RelTarget::Part(name));
        let cx = emu(image.width);
        let cy = emu(image.height);
        let label = format!("Picture {number}");
        xml.open("w:r", &[]);
        xml.open("w:drawing", &[]);
        xml.open(
            "wp:inline",
            &[
                ("distT", "0"),
                ("distB", "0"),
                ("distL", "0"),
                ("distR", "0"),
            ],
        );
        xml.empty("wp:extent", &[("cx", &cx), ("cy", &cy)]);
        xml.empty(
            "wp:docPr",
            &[
                ("id", &number),
                ("name", &label),
                ("descr", image.description.as_str()),
            ],
        );
        xml.open("a:graphic", &[]);
        xml.open(
            "a:graphicData",
            &[(
                "uri",
                "http://schemas.openxmlformats.org/drawingml/2006/picture",
            )],
        );
        xml.open("pic:pic", &[]);
        xml.open("pic:nvPicPr", &[]);
        xml.empty(
            "pic:cNvPr",
            &[
                ("id", &number),
                ("name", &label),
                ("descr", image.description.as_str()),
            ],
        );
        xml.empty("pic:cNvPicPr", &[]);
        xml.close("pic:nvPicPr");
        xml.open("pic:blipFill", &[]);
        xml.empty("a:blip", &[("r:embed", id.as_str())]);
        xml.open("a:stretch", &[]);
        xml.empty("a:fillRect", &[]);
        xml.close("a:stretch");
        xml.close("pic:blipFill");
        xml.open("pic:spPr", &[]);
        xml.open("a:xfrm", &[]);
        xml.empty("a:off", &[("x", "0"), ("y", "0")]);
        xml.empty("a:ext", &[("cx", &cx), ("cy", &cy)]);
        xml.close("a:xfrm");
        xml.open("a:prstGeom", &[("prst", "rect")]);
        xml.empty("a:avLst", &[]);
        xml.close("a:prstGeom");
        xml.close("pic:spPr");
        xml.close("pic:pic");
        xml.close("a:graphicData");
        xml.close("a:graphic");
        xml.close("wp:inline");
        xml.close("w:drawing");
        xml.close("w:r");
    }
}

/// The date attributes of an annotation. `w:date` holds the wall time, with
/// the `Z` Word expects. `w16du:dateUtc` holds the true instant, when the
/// model knows it.
fn date_attrs(date: Option<WordDate>) -> Vec<(&'static str, String)> {
    match date {
        None => Vec::new(),
        Some(WordDate::Floating(wall)) => vec![("w:date", time::format(wall))],
        Some(WordDate::Zoned { utc, wall }) => vec![
            ("w:date", time::format(wall)),
            ("w16du:dateUtc", time::format(utc)),
        ],
    }
}

/// One run, inside the tracked change it belongs to.
fn write_run(xml: &mut XmlWriter, run: &Run) {
    let wrapper = run.revision.as_ref().map(|revision| {
        let tag = match revision.kind {
            RevisionKind::Insertion => "w:ins",
            RevisionKind::Deletion => "w:del",
        };
        let id = revision.id.get().to_string();
        let mut attrs = vec![("w:id", id), ("w:author", revision.author.clone())];
        attrs.extend(date_attrs(revision.date));
        (tag, attrs)
    });
    if let Some((tag, attrs)) = &wrapper {
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        xml.open(tag, &attrs);
    }
    xml.open("w:r", &[]);
    run_properties(xml, &run.format);
    let deleted = run
        .revision
        .as_ref()
        .is_some_and(|revision| revision.kind == RevisionKind::Deletion);
    let tag = if deleted { "w:delText" } else { "w:t" };
    xml.leaf(tag, &[("xml:space", "preserve")], &run.text);
    xml.close("w:r");
    if let Some((tag, _)) = wrapper {
        xml.close(tag);
    }
}

fn run_properties(xml: &mut XmlWriter, format: &RunFormat) {
    if *format == RunFormat::default() {
        return;
    }
    xml.open("w:rPr", &[]);
    if let Some(font) = &format.font {
        xml.empty(
            "w:rFonts",
            &[
                ("w:ascii", font),
                ("w:hAnsi", font),
                ("w:cs", font),
                ("w:eastAsia", font),
            ],
        );
    }
    if format.bold.as_bool() {
        xml.empty("w:b", &[]);
    }
    if format.italic.as_bool() {
        xml.empty("w:i", &[]);
    }
    if format.strike.as_bool() {
        xml.empty("w:strike", &[]);
    }
    if let Some(color) = format.color {
        xml.empty("w:color", &[("w:val", &hex(color))]);
    }
    if let Some(size) = format.size {
        xml.empty("w:sz", &[("w:val", &half_points(size))]);
    }
    if format.underline.as_bool() {
        xml.empty("w:u", &[("w:val", "single")]);
    }
    if let Some(fill) = format.fill {
        xml.empty(
            "w:shd",
            &[
                ("w:val", "clear"),
                ("w:color", "auto"),
                ("w:fill", &hex(fill)),
            ],
        );
    }
    match format.position {
        VerticalPosition::Baseline => {}
        VerticalPosition::Superscript => xml.empty("w:vertAlign", &[("w:val", "superscript")]),
        VerticalPosition::Subscript => xml.empty("w:vertAlign", &[("w:val", "subscript")]),
    }
    xml.close("w:rPr");
}

/// The final section: its bands, its page size and its margins.
fn section(
    xml: &mut XmlWriter,
    page: Option<&PageSetup>,
    header: Option<&RelId>,
    footer: Option<&RelId>,
) {
    if page.is_none() && header.is_none() && footer.is_none() {
        return;
    }
    xml.open("w:sectPr", &[]);
    if let Some(id) = header {
        xml.empty(
            "w:headerReference",
            &[("w:type", "default"), ("r:id", id.as_str())],
        );
    }
    if let Some(id) = footer {
        xml.empty(
            "w:footerReference",
            &[("w:type", "default"), ("r:id", id.as_str())],
        );
    }
    if let Some(page) = page {
        let mut size = vec![("w:w", twips(page.width)), ("w:h", twips(page.height))];
        if page.orientation == Orientation::Landscape {
            size.push(("w:orient", "landscape".to_owned()));
        }
        let size: Vec<(&str, &str)> = size.iter().map(|(k, v)| (*k, v.as_str())).collect();
        xml.empty("w:pgSz", &size);
        let margins = [
            ("w:top", twips(page.margins.top)),
            ("w:right", twips(page.margins.right)),
            ("w:bottom", twips(page.margins.bottom)),
            ("w:left", twips(page.margins.left)),
            ("w:header", "708".to_owned()),
            ("w:footer", "708".to_owned()),
            ("w:gutter", "0".to_owned()),
        ];
        let margins: Vec<(&str, &str)> = margins.iter().map(|(k, v)| (*k, v.as_str())).collect();
        xml.empty("w:pgMar", &margins);
    }
    xml.close("w:sectPr");
}

fn core_properties(document: &WordDocument) -> Payload<PartBytes> {
    let mut xml = XmlWriter::new();
    xml.open(
        "cp:coreProperties",
        &[
            (
                "xmlns:cp",
                "http://schemas.openxmlformats.org/package/2006/metadata/core-properties",
            ),
            ("xmlns:dc", "http://purl.org/dc/elements/1.1/"),
        ],
    );
    if let Some(title) = &document.title {
        xml.leaf("dc:title", &[], title);
    }
    xml.close("cp:coreProperties");
    xml.finish()
}

fn settings(document: &WordDocument) -> Payload<PartBytes> {
    let mut xml = XmlWriter::new();
    xml.open("w:settings", &namespaces());
    xml.empty("w:defaultTabStop", &[("w:val", "708")]);
    if !document.footnotes.is_empty() {
        xml.open("w:footnotePr", &[]);
        xml.empty("w:footnote", &[("w:id", "-1")]);
        xml.empty("w:footnote", &[("w:id", "0")]);
        xml.close("w:footnotePr");
    }
    if !document.endnotes.is_empty() {
        xml.open("w:endnotePr", &[]);
        xml.empty("w:endnote", &[("w:id", "-1")]);
        xml.empty("w:endnote", &[("w:id", "0")]);
        xml.close("w:endnotePr");
    }
    xml.empty("w:compat", &[]);
    xml.close("w:settings");
    xml.finish()
}

/// The styles every written document carries: the roles of the model, the
/// marks of the notes and the comments, and the grid of a table.
fn styles() -> Payload<PartBytes> {
    let mut xml = XmlWriter::new();
    xml.open("w:styles", &namespaces());
    xml.open("w:docDefaults", &[]);
    xml.open("w:rPrDefault", &[]);
    xml.open("w:rPr", &[]);
    xml.empty("w:sz", &[("w:val", "22")]);
    xml.close("w:rPr");
    xml.close("w:rPrDefault");
    xml.open("w:pPrDefault", &[]);
    xml.open("w:pPr", &[]);
    xml.empty(
        "w:spacing",
        &[
            ("w:after", "160"),
            ("w:line", "259"),
            ("w:lineRule", "auto"),
        ],
    );
    xml.close("w:pPr");
    xml.close("w:pPrDefault");
    xml.close("w:docDefaults");

    let paragraph = |xml: &mut XmlWriter, id: &str, name: &str, body: &dyn Fn(&mut XmlWriter)| {
        let mut attrs = vec![("w:type", "paragraph"), ("w:styleId", id)];
        if id == "Normal" {
            attrs.push(("w:default", "1"));
        }
        xml.open("w:style", &attrs);
        xml.empty("w:name", &[("w:val", name)]);
        if id != "Normal" {
            xml.empty("w:basedOn", &[("w:val", "Normal")]);
        }
        xml.empty("w:qFormat", &[]);
        body(xml);
        xml.close("w:style");
    };
    paragraph(&mut xml, "Normal", "Normal", &|_| {});
    for level in 1..=9_u8 {
        let id = format!("Heading{level}");
        let name = format!("heading {level}");
        let size = (34 - 2 * u32::from(level.min(6))).to_string();
        let outline = (level - 1).to_string();
        paragraph(&mut xml, &id, &name, &|xml| {
            xml.open("w:pPr", &[]);
            xml.empty("w:keepNext", &[]);
            xml.empty("w:spacing", &[("w:before", "240"), ("w:after", "80")]);
            xml.empty("w:outlineLvl", &[("w:val", &outline)]);
            xml.close("w:pPr");
            xml.open("w:rPr", &[]);
            xml.empty("w:b", &[]);
            xml.empty("w:sz", &[("w:val", &size)]);
            xml.close("w:rPr");
        });
    }
    paragraph(&mut xml, "Title", "Title", &|xml| {
        xml.open("w:rPr", &[]);
        xml.empty("w:sz", &[("w:val", "56")]);
        xml.close("w:rPr");
    });
    paragraph(&mut xml, "Quote", "Quote", &|xml| {
        xml.open("w:pPr", &[]);
        xml.empty("w:ind", &[("w:left", "864"), ("w:right", "864")]);
        xml.close("w:pPr");
        xml.open("w:rPr", &[]);
        xml.empty("w:i", &[]);
        xml.close("w:rPr");
    });
    paragraph(&mut xml, "SourceCode", "Source Code", &|xml| {
        xml.open("w:pPr", &[]);
        xml.empty("w:spacing", &[("w:after", "0")]);
        xml.close("w:pPr");
        xml.open("w:rPr", &[]);
        xml.empty(
            "w:rFonts",
            &[("w:ascii", "Consolas"), ("w:hAnsi", "Consolas")],
        );
        xml.close("w:rPr");
    });
    for (id, name) in [
        ("FootnoteText", "footnote text"),
        ("EndnoteText", "endnote text"),
        ("CommentText", "annotation text"),
    ] {
        paragraph(&mut xml, id, name, &|xml| {
            xml.open("w:rPr", &[]);
            xml.empty("w:sz", &[("w:val", "20")]);
            xml.close("w:rPr");
        });
    }
    let character = |xml: &mut XmlWriter, id: &str, name: &str, body: &dyn Fn(&mut XmlWriter)| {
        xml.open("w:style", &[("w:type", "character"), ("w:styleId", id)]);
        xml.empty("w:name", &[("w:val", name)]);
        xml.open("w:rPr", &[]);
        body(xml);
        xml.close("w:rPr");
        xml.close("w:style");
    };
    let raised = |xml: &mut XmlWriter| xml.empty("w:vertAlign", &[("w:val", "superscript")]);
    character(&mut xml, "FootnoteReference", "footnote reference", &raised);
    character(&mut xml, "EndnoteReference", "endnote reference", &raised);
    character(
        &mut xml,
        "CommentReference",
        "annotation reference",
        &|xml| {
            xml.empty("w:sz", &[("w:val", "16")]);
        },
    );
    character(&mut xml, "Hyperlink", "Hyperlink", &|xml| {
        xml.empty("w:color", &[("w:val", "0563C1")]);
        xml.empty("w:u", &[("w:val", "single")]);
    });
    xml.open(
        "w:style",
        &[("w:type", "table"), ("w:styleId", "TableGrid")],
    );
    xml.empty("w:name", &[("w:val", "Table Grid")]);
    xml.open("w:tblPr", &[]);
    xml.open("w:tblBorders", &[]);
    for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        xml.empty(
            &format!("w:{side}"),
            &[
                ("w:val", "single"),
                ("w:sz", "4"),
                ("w:space", "0"),
                ("w:color", "auto"),
            ],
        );
    }
    xml.close("w:tblBorders");
    xml.close("w:tblPr");
    xml.close("w:style");
    xml.close("w:styles");
    xml.finish()
}

/// One definition per list, its nine levels each counting as the first
/// paragraph at that level gave, or as the list's kind beneath.
fn numbering(lists: &BTreeMap<WordId<ListInstance>, [Option<ListKind>; 9]>) -> Payload<PartBytes> {
    let mut xml = XmlWriter::new();
    xml.open("w:numbering", &namespaces());
    for (id, levels) in lists {
        let definition = id.get().to_string();
        xml.open("w:abstractNum", &[("w:abstractNumId", &definition)]);
        xml.empty("w:multiLevelType", &[("w:val", "hybridMultilevel")]);
        let family = levels
            .iter()
            .flatten()
            .next()
            .copied()
            .unwrap_or(ListKind::Bullet);
        for (level, kind) in (0_u32..).zip(levels.iter()) {
            let kind = kind.unwrap_or(match family {
                ListKind::Bullet => ListKind::Bullet,
                ListKind::Numbered(_) => ListKind::Numbered(NumberFormat::Decimal),
            });
            let (format, text) = match kind {
                ListKind::Bullet => ("bullet", "\u{2022}".to_owned()),
                ListKind::Numbered(format) => (
                    match format {
                        NumberFormat::Decimal => "decimal",
                        NumberFormat::LowerLetter => "lowerLetter",
                        NumberFormat::UpperLetter => "upperLetter",
                        NumberFormat::LowerRoman => "lowerRoman",
                        NumberFormat::UpperRoman => "upperRoman",
                    },
                    format!("%{}.", level + 1),
                ),
            };
            xml.open("w:lvl", &[("w:ilvl", &level.to_string())]);
            xml.empty("w:start", &[("w:val", "1")]);
            xml.empty("w:numFmt", &[("w:val", format)]);
            xml.empty("w:lvlText", &[("w:val", &text)]);
            xml.empty("w:lvlJc", &[("w:val", "left")]);
            xml.open("w:pPr", &[]);
            let indent = (720 * (level + 1)).to_string();
            xml.empty("w:ind", &[("w:left", &indent), ("w:hanging", "360")]);
            xml.close("w:pPr");
            xml.close("w:lvl");
        }
        xml.close("w:abstractNum");
    }
    for id in lists.keys() {
        let id = id.get().to_string();
        xml.open("w:num", &[("w:numId", &id)]);
        xml.empty("w:abstractNumId", &[("w:val", &id)]);
        xml.close("w:num");
    }
    xml.close("w:numbering");
    xml.finish()
}

/// The initials Word shows on a comment: the first letter of each word of
/// the author's name.
fn initials(author: &str) -> String {
    author
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .collect()
}

/// A paragraph id, `ST_LongHexNumber`, below `0x80000000` as Word requires.
fn paragraph_id(number: u32) -> String {
    format!("{:08X}", number & 0x7FFF_FFFF)
}

fn hex(color: Rgb) -> String {
    format!("{:06X}", color.bits())
}

fn twips(length: PrintLength<Pt>) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a page length in twips fits an i64"
    )]
    let twips = (length.get() * TWIPS_PER_PT).round() as i64;
    twips.to_string()
}

fn emu(length: PrintLength<Pt>) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a picture's EMU count fits an i64"
    )]
    let emu = (length.get() * EMU_PER_PT).round() as i64;
    emu.max(0).to_string()
}

fn half_points(size: FontSize) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a font size in half points fits"
    )]
    let half = (size.points() * 2.0).round() as i64;
    half.to_string()
}
