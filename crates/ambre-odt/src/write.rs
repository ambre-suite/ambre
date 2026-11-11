//! The writer: the Word model into the parts of an OpenDocument text
//! (`4-Prose/features/ambre-spec-prose-import-export.md section 3`).
//!
//! The writer reads nothing but the model it is handed, and writes the same
//! parts for the same model: each automatic style is named in the order the
//! walk first needs it. A tracked change becomes a changed region, a comment
//! an annotation with its end mark, and a note a note in the line.

use ambre_core::{
    Authored, Instant, Payload, PrintLength, Pt, Rgb, Slant, Strikethrough, Underline, Weight,
};
use ambre_docx::model::{
    Alignment, Block, BreakKind, CommentMarker, CommentState, Image, Inline, LinkTarget, ListKind,
    NoteKind, NumberFormat, Orientation, Paragraph, ParagraphRole, Revision, RevisionKind, RowRole,
    Run, RunFormat, Table, VerticalMerge, VerticalPosition, WordDate, WordDocument, WordId,
};

use ambre_opc::PartBytes;

use crate::xml::XmlWriter;

/// The namespaces every part declares on its root.
const NAMESPACES: [(&str, &str); 11] = [
    (
        "xmlns:office",
        "urn:oasis:names:tc:opendocument:xmlns:office:1.0",
    ),
    (
        "xmlns:style",
        "urn:oasis:names:tc:opendocument:xmlns:style:1.0",
    ),
    (
        "xmlns:text",
        "urn:oasis:names:tc:opendocument:xmlns:text:1.0",
    ),
    (
        "xmlns:table",
        "urn:oasis:names:tc:opendocument:xmlns:table:1.0",
    ),
    (
        "xmlns:draw",
        "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0",
    ),
    (
        "xmlns:fo",
        "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0",
    ),
    ("xmlns:xlink", "http://www.w3.org/1999/xlink"),
    ("xmlns:dc", "http://purl.org/dc/elements/1.1/"),
    (
        "xmlns:meta",
        "urn:oasis:names:tc:opendocument:xmlns:meta:1.0",
    ),
    (
        "xmlns:svg",
        "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0",
    ),
    (
        "xmlns:loext",
        "urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0",
    ),
];

/// The parts of a text, ready for the package: the three XML parts, then
/// each picture with its path and its media type.
pub struct WrittenParts {
    pub content: Payload<PartBytes>,
    pub styles: Payload<PartBytes>,
    pub meta: Payload<PartBytes>,
    pub pictures: Vec<(String, &'static str, Payload<PartBytes>)>,
}

/// The parts of `document`. Its ids are numbered again first, so two models
/// that differ in their ids alone write the same parts.
#[must_use]
pub fn write_parts(document: &WordDocument) -> WrittenParts {
    let document = document.renumbered();
    let mut body = Writer::new(&document, "P", "T");
    body.blocks(&document.body);
    body.close_insertion();
    let mut bands = Writer::new(&document, "MP", "MT");
    bands.pictures = std::mem::take(&mut body.pictures);
    bands.next_note = body.next_note;
    let header = document.header.as_ref().map(|blocks| {
        bands.blocks(blocks);
        bands.close_insertion();
        std::mem::take(&mut bands.out)
    });
    let footer = document.footer.as_ref().map(|blocks| {
        bands.blocks(blocks);
        bands.close_insertion();
        std::mem::take(&mut bands.out)
    });

    let mut content = XmlWriter::new();
    content.open("office:document-content", &root_attrs());
    content.open("office:automatic-styles", &[]);
    content.raw(&body.styles.write());
    content.close("office:automatic-styles");
    content.open("office:body", &[]);
    content.open("office:text", &[]);
    if !body.regions.is_empty() {
        content.open("text:tracked-changes", &[]);
        content.raw(&body.regions);
        content.close("text:tracked-changes");
    }
    content.raw(&body.out);
    content.close("office:text");
    content.close("office:body");
    content.close("office:document-content");

    let mut styles = XmlWriter::new();
    styles.open("office:document-styles", &root_attrs());
    styles.raw(NAMED_STYLES);
    styles.open("office:automatic-styles", &[]);
    page_layout(&mut styles, &document);
    styles.raw(&bands.styles.write());
    styles.close("office:automatic-styles");
    styles.open("office:master-styles", &[]);
    styles.open(
        "style:master-page",
        &[
            ("style:name", "Standard"),
            ("style:page-layout-name", "pm1"),
        ],
    );
    for (tag, band) in [("style:header", header), ("style:footer", footer)] {
        if let Some(band) = band {
            styles.open(tag, &[]);
            styles.raw(&band);
            styles.close(tag);
        }
    }
    styles.close("style:master-page");
    styles.close("office:master-styles");
    styles.close("office:document-styles");

    let mut meta = XmlWriter::new();
    meta.open("office:document-meta", &root_attrs());
    meta.open("office:meta", &[]);
    if let Some(title) = &document.title {
        meta.leaf("dc:title", &[], title);
    }
    meta.close("office:meta");
    meta.close("office:document-meta");

    WrittenParts {
        content: content.finish(),
        styles: styles.finish(),
        meta: meta.finish(),
        pictures: bands.pictures,
    }
}

/// The named style of each heading level, 1 to 9.
const HEADINGS: [&str; 9] = [
    "Heading_20_1",
    "Heading_20_2",
    "Heading_20_3",
    "Heading_20_4",
    "Heading_20_5",
    "Heading_20_6",
    "Heading_20_7",
    "Heading_20_8",
    "Heading_20_9",
];

fn root_attrs() -> Vec<(&'static str, &'static str)> {
    let mut attrs = NAMESPACES.to_vec();
    attrs.push(("office:version", "1.3"));
    attrs
}

/// The named styles of every text: the body, the nine headings and the
/// three roles the reader reads by name.
const NAMED_STYLES: &str = concat!(
    "<office:styles>",
    "<style:style style:name=\"Standard\" style:family=\"paragraph\" style:class=\"text\"/>",
    "<style:style style:name=\"Heading\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"text\">",
    "<style:text-properties fo:font-weight=\"bold\"/></style:style>",
    "<style:style style:name=\"Heading_20_1\" style:display-name=\"Heading 1\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"1\" style:class=\"text\"><style:text-properties fo:font-size=\"130%\"/></style:style>",
    "<style:style style:name=\"Heading_20_2\" style:display-name=\"Heading 2\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"2\" style:class=\"text\"><style:text-properties fo:font-size=\"115%\"/></style:style>",
    "<style:style style:name=\"Heading_20_3\" style:display-name=\"Heading 3\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"3\" style:class=\"text\"/>",
    "<style:style style:name=\"Heading_20_4\" style:display-name=\"Heading 4\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"4\" style:class=\"text\"/>",
    "<style:style style:name=\"Heading_20_5\" style:display-name=\"Heading 5\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"5\" style:class=\"text\"/>",
    "<style:style style:name=\"Heading_20_6\" style:display-name=\"Heading 6\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"6\" style:class=\"text\"/>",
    "<style:style style:name=\"Heading_20_7\" style:display-name=\"Heading 7\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"7\" style:class=\"text\"/>",
    "<style:style style:name=\"Heading_20_8\" style:display-name=\"Heading 8\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"8\" style:class=\"text\"/>",
    "<style:style style:name=\"Heading_20_9\" style:display-name=\"Heading 9\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"9\" style:class=\"text\"/>",
    "<style:style style:name=\"Title\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:class=\"chapter\"><style:text-properties fo:font-size=\"200%\"/></style:style>",
    "<style:style style:name=\"Quotations\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"html\"><style:paragraph-properties fo:margin-left=\"0.4in\" fo:margin-right=\"0.4in\"/></style:style>",
    "<style:style style:name=\"Preformatted_20_Text\" style:display-name=\"Preformatted Text\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"html\"><style:text-properties style:font-name=\"Liberation Mono\" fo:font-family=\"'Liberation Mono'\" style:font-family-generic=\"modern\" style:font-pitch=\"fixed\"/></style:style>",
    "</office:styles>",
);

/// The automatic styles one part needs, each named in the order the walk
/// first asks for it.
struct Styles {
    paragraph_prefix: &'static str,
    text_prefix: &'static str,
    paragraphs: Vec<ParagraphStyle>,
    texts: Vec<RunFormat>,
    lists: Vec<Vec<Option<ListKind>>>,
    tables: Vec<(Vec<PrintLength<Pt>>, Vec<Rgb>)>,
}

#[derive(Clone, PartialEq)]
struct ParagraphStyle {
    parent: &'static str,
    align: Option<Alignment>,
    before: Option<BreakKind>,
}

impl Styles {
    fn paragraph(&mut self, style: ParagraphStyle) -> String {
        let at = match self.paragraphs.iter().position(|held| *held == style) {
            Some(at) => at,
            None => {
                self.paragraphs.push(style);
                self.paragraphs.len() - 1
            }
        };
        format!("{}{}", self.paragraph_prefix, at + 1)
    }

    fn text(&mut self, format: &RunFormat) -> Option<String> {
        if *format == RunFormat::default() {
            return None;
        }
        let at = match self.texts.iter().position(|held| held == format) {
            Some(at) => at,
            None => {
                self.texts.push(format.clone());
                self.texts.len() - 1
            }
        };
        Some(format!("{}{}", self.text_prefix, at + 1))
    }

    fn write(&self) -> String {
        let mut xml = XmlWriter::fragment();
        for (at, style) in self.paragraphs.iter().enumerate() {
            let name = format!("{}{}", self.paragraph_prefix, at + 1);
            xml.open(
                "style:style",
                &[
                    ("style:name", &name),
                    ("style:family", "paragraph"),
                    ("style:parent-style-name", style.parent),
                ],
            );
            let mut props: Vec<(&str, &str)> = Vec::new();
            if let Some(align) = style.align {
                props.push(("fo:text-align", align_value(align)));
            }
            match style.before {
                Some(BreakKind::Page) => props.push(("fo:break-before", "page")),
                Some(BreakKind::Column) => props.push(("fo:break-before", "column")),
                Some(BreakKind::Line) | None => {}
            }
            if !props.is_empty() {
                xml.empty("style:paragraph-properties", &props);
            }
            xml.close("style:style");
        }
        for (at, format) in self.texts.iter().enumerate() {
            let name = format!("{}{}", self.text_prefix, at + 1);
            xml.open(
                "style:style",
                &[("style:name", &name), ("style:family", "text")],
            );
            text_properties(&mut xml, format);
            xml.close("style:style");
        }
        for (at, levels) in self.lists.iter().enumerate() {
            let name = format!("L{}", at + 1);
            xml.open("text:list-style", &[("style:name", &name)]);
            for (level, kind) in levels.iter().enumerate() {
                let number = (level + 1).to_string();
                match kind {
                    Some(ListKind::Numbered(format)) => xml.empty(
                        "text:list-level-style-number",
                        &[
                            ("text:level", &number),
                            ("style:num-format", number_format(*format)),
                            ("style:num-suffix", "."),
                        ],
                    ),
                    Some(ListKind::Bullet) | None => xml.empty(
                        "text:list-level-style-bullet",
                        &[("text:level", &number), ("text:bullet-char", "\u{2022}")],
                    ),
                }
            }
            xml.close("text:list-style");
        }
        for (at, (columns, fills)) in self.tables.iter().enumerate() {
            let table = format!("Table{}", at + 1);
            for (column, width) in columns.iter().enumerate() {
                let name = format!("{table}.C{}", column + 1);
                let width = length(*width);
                xml.open(
                    "style:style",
                    &[("style:name", &name), ("style:family", "table-column")],
                );
                xml.empty(
                    "style:table-column-properties",
                    &[("style:column-width", &width)],
                );
                xml.close("style:style");
            }
            for (fill_at, fill) in fills.iter().enumerate() {
                let name = format!("{table}.F{}", fill_at + 1);
                let color = hex(*fill);
                xml.open(
                    "style:style",
                    &[("style:name", &name), ("style:family", "table-cell")],
                );
                xml.empty(
                    "style:table-cell-properties",
                    &[("fo:background-color", &color)],
                );
                xml.close("style:style");
            }
        }
        xml.into_string()
    }
}

fn text_properties(xml: &mut XmlWriter, format: &RunFormat) {
    let size = format.size.map(|size| format!("{}pt", size.points()));
    let color = format.color.map(hex);
    let fill = format.fill.map(hex);
    let font = format.font.as_ref().map(|font| format!("'{font}'"));
    let mut props: Vec<(&str, &str)> = Vec::new();
    if format.bold == Weight::Bold {
        props.push(("fo:font-weight", "bold"));
    }
    if format.italic == Slant::Italic {
        props.push(("fo:font-style", "italic"));
    }
    if format.underline == Underline::Underlined {
        props.push(("style:text-underline-style", "solid"));
        props.push(("style:text-underline-width", "auto"));
        props.push(("style:text-underline-color", "font-color"));
    }
    if format.strike == Strikethrough::Struck {
        props.push(("style:text-line-through-style", "solid"));
    }
    match format.position {
        VerticalPosition::Superscript => props.push(("style:text-position", "super 58%")),
        VerticalPosition::Subscript => props.push(("style:text-position", "sub 58%")),
        VerticalPosition::Baseline => {}
    }
    if let Some(size) = &size {
        props.push(("fo:font-size", size));
    }
    if let Some(color) = &color {
        props.push(("fo:color", color));
    }
    if let Some(fill) = &fill {
        props.push(("fo:background-color", fill));
    }
    if let Some(font) = &font {
        props.push(("fo:font-family", font));
    }
    xml.empty("style:text-properties", &props);
}

/// The walk of one part: its blocks into XML, its automatic styles beside.
struct Writer<'a> {
    document: &'a WordDocument,
    out: String,
    /// The changed regions, written into `text:tracked-changes`.
    regions: String,
    styles: Styles,
    /// The insertion whose region is open: its Word id and its region name.
    insertion: Option<(WordId<ambre_docx::model::RevisionMarker>, String)>,
    next_region: usize,
    /// The notes written so far, for their citations.
    next_note: (usize, usize),
    pictures: Vec<(String, &'static str, Payload<PartBytes>)>,
    /// Each list written, with the Word list it holds and its `xml:id`.
    lists: Vec<(WordId<ambre_docx::model::ListInstance>, String)>,
}

impl<'a> Writer<'a> {
    fn new(
        document: &'a WordDocument,
        paragraph_prefix: &'static str,
        text_prefix: &'static str,
    ) -> Self {
        Self {
            document,
            out: String::new(),
            regions: String::new(),
            styles: Styles {
                paragraph_prefix,
                text_prefix,
                paragraphs: Vec::new(),
                texts: Vec::new(),
                lists: Vec::new(),
                tables: Vec::new(),
            },
            insertion: None,
            next_region: 0,
            next_note: (0, 0),
            pictures: Vec::new(),
            lists: Vec::new(),
        }
    }

    fn emit(&mut self, xml: XmlWriter) {
        self.out.push_str(&xml.into_string());
    }

    fn blocks(&mut self, blocks: &[Block]) {
        let mut at = 0;
        while let Some(block) = blocks.get(at) {
            match block {
                Block::Paragraph(paragraph) if paragraph.list.is_some() => {
                    let end = blocks
                        .get(at..)
                        .unwrap_or_default()
                        .iter()
                        .position(|b| match b {
                            Block::Paragraph(p) => {
                                p.list.map(|l| l.list) != paragraph.list.map(|l| l.list)
                            }
                            Block::Table(_) => true,
                        })
                        .map_or(blocks.len(), |n| at + n);
                    self.list(blocks.get(at..end).unwrap_or_default());
                    at = end;
                    continue;
                }
                Block::Paragraph(paragraph) => self.paragraph(paragraph),
                Block::Table(table) => self.table(table),
            }
            at += 1;
        }
    }

    /// One run of paragraphs of one list, nested by their levels.
    fn list(&mut self, items: &[Block]) {
        let paragraphs: Vec<&Paragraph> = items
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(p) => Some(p),
                Block::Table(_) => None,
            })
            .collect();
        let Some(first) = paragraphs.first().and_then(|p| p.list) else {
            return;
        };
        let mut levels: Vec<Option<ListKind>> = vec![None; 9];
        for membership in paragraphs.iter().filter_map(|p| p.list) {
            if let Some(slot) = levels.get_mut(usize::from(membership.level.get())) {
                slot.get_or_insert(membership.kind);
            }
        }
        // Each list carries an id, so a later list of the same Word list
        // continues it by name, whatever came between.
        self.styles.lists.push(levels);
        let style = format!("L{}", self.styles.lists.len());
        let xml_id = format!("list{}", self.styles.lists.len());
        let continued = self
            .lists
            .iter()
            .rev()
            .find(|(list, _)| *list == first.list)
            .map(|(_, name)| name.clone());
        self.lists.push((first.list, xml_id.clone()));
        let mut depth = 0_usize;
        for paragraph in paragraphs {
            let target = paragraph.list.map_or(1, |l| usize::from(l.level.get()) + 1);
            let mut xml = XmlWriter::fragment();
            if depth == 0 {
                let mut attrs = vec![
                    ("text:style-name", style.as_str()),
                    ("xml:id", xml_id.as_str()),
                ];
                if let Some(name) = &continued {
                    attrs.push(("text:continue-list", name));
                }
                xml.open("text:list", &attrs);
                xml.open("text:list-item", &[]);
                depth = 1;
            } else if target <= depth {
                while depth > target {
                    xml.close("text:list-item");
                    xml.close("text:list");
                    depth -= 1;
                }
                xml.close("text:list-item");
                xml.open("text:list-item", &[]);
            }
            while depth < target {
                xml.open("text:list", &[]);
                xml.open("text:list-item", &[]);
                depth += 1;
            }
            self.emit(xml);
            self.paragraph(paragraph);
        }
        let mut xml = XmlWriter::fragment();
        while depth > 0 {
            xml.close("text:list-item");
            xml.close("text:list");
            depth -= 1;
        }
        self.emit(xml);
    }

    /// One paragraph. A page or a column break inside it splits it, and the
    /// paragraph after the break starts on the new page.
    fn paragraph(&mut self, paragraph: &Paragraph) {
        let parent = match paragraph.role {
            ParagraphRole::Body => "Standard",
            ParagraphRole::Heading(depth) => HEADINGS
                .get(usize::from(depth.get()).saturating_sub(1))
                .copied()
                .unwrap_or("Heading"),
            ParagraphRole::Title => "Title",
            ParagraphRole::Quote => "Quotations",
            ParagraphRole::Code => "Preformatted_20_Text",
        };
        let mut pieces: Vec<(Option<BreakKind>, &[Inline])> = Vec::new();
        let mut before = None;
        let mut start = 0;
        for (at, inline) in paragraph.inlines.iter().enumerate() {
            if let Inline::Break(kind @ (BreakKind::Page | BreakKind::Column)) = inline {
                pieces.push((before, paragraph.inlines.get(start..at).unwrap_or_default()));
                before = Some(*kind);
                start = at + 1;
            }
        }
        pieces.push((before, paragraph.inlines.get(start..).unwrap_or_default()));
        // A break that opens the paragraph moves onto it, with no empty
        // paragraph before it.
        if pieces.len() > 1 && pieces.first().is_some_and(|(_, p)| p.is_empty()) {
            pieces.remove(0);
        }
        for (before, inlines) in pieces {
            let style = self.styles.paragraph(ParagraphStyle {
                parent,
                align: paragraph.align,
                before,
            });
            let mut xml = XmlWriter::fragment();
            match paragraph.role {
                ParagraphRole::Heading(depth) => {
                    let level = depth.get().to_string();
                    xml.open(
                        "text:h",
                        &[("text:style-name", &style), ("text:outline-level", &level)],
                    );
                }
                _ => xml.open("text:p", &[("text:style-name", &style)]),
            }
            self.emit(xml);
            let mut text = TextState { space: true };
            self.inlines(inlines, &mut text);
            let mut xml = XmlWriter::fragment();
            xml.close(if matches!(paragraph.role, ParagraphRole::Heading(_)) {
                "text:h"
            } else {
                "text:p"
            });
            self.emit(xml);
        }
    }

    fn inlines(&mut self, inlines: &[Inline], text: &mut TextState) {
        let mut at = 0;
        while let Some(inline) = inlines.get(at) {
            match inline {
                Inline::Run(run) => {
                    if run
                        .revision
                        .as_ref()
                        .is_some_and(|r| r.kind == RevisionKind::Deletion)
                    {
                        let end = inlines
                            .get(at..)
                            .unwrap_or_default()
                            .iter()
                            .position(|i| !same_deletion(i, run.revision.as_ref()))
                            .map_or(inlines.len(), |n| at + n);
                        self.deletion(inlines.get(at..end).unwrap_or_default());
                        at = end;
                        continue;
                    }
                    self.revision_mark(run.revision.as_ref());
                    self.run(run, text);
                }
                Inline::Link(link) => {
                    let href = match &link.target {
                        LinkTarget::External(url) => url.as_str().to_owned(),
                        LinkTarget::Anchor(name) => format!("#{name}"),
                    };
                    if let Some(first) = link.runs.first() {
                        self.revision_mark(first.revision.as_ref());
                    }
                    let mut xml = XmlWriter::fragment();
                    xml.open("text:a", &[("xlink:type", "simple"), ("xlink:href", &href)]);
                    self.emit(xml);
                    for run in &link.runs {
                        self.run(run, text);
                    }
                    let mut xml = XmlWriter::fragment();
                    xml.close("text:a");
                    self.emit(xml);
                }
                Inline::Break(BreakKind::Line) => {
                    let mut xml = XmlWriter::fragment();
                    xml.empty("text:line-break", &[]);
                    self.emit(xml);
                    text.space = true;
                }
                Inline::Break(BreakKind::Page | BreakKind::Column) => {}
                Inline::Tab => {
                    let mut xml = XmlWriter::fragment();
                    xml.empty("text:tab", &[]);
                    self.emit(xml);
                    text.space = false;
                }
                Inline::NoteCall(kind, id) => self.note(*kind, *id),
                Inline::CommentStart(id) => self.annotation(*id),
                Inline::CommentEnd(id) => {
                    let name = comment_name(*id);
                    let mut xml = XmlWriter::fragment();
                    xml.empty("office:annotation-end", &[("office:name", &name)]);
                    self.emit(xml);
                }
                Inline::Image(image) => self.image(image),
            }
            at += 1;
        }
    }

    fn run(&mut self, run: &Run, text: &mut TextState) {
        let style = self.styles.text(&run.format);
        let mut xml = XmlWriter::fragment();
        if let Some(style) = &style {
            xml.open("text:span", &[("text:style-name", style)]);
        }
        write_text(&mut xml, &run.text, text);
        if style.is_some() {
            xml.close("text:span");
        }
        self.emit(xml);
    }

    /// Open the insertion region `revision` names, or close the one open
    /// when the run belongs to none.
    fn revision_mark(&mut self, revision: Option<&Revision>) {
        let open = self.insertion.as_ref().map(|(id, _)| *id);
        let wanted = revision
            .filter(|r| r.kind == RevisionKind::Insertion)
            .map(|r| r.id);
        if open == wanted {
            return;
        }
        self.close_insertion();
        let Some(revision) = revision.filter(|r| r.kind == RevisionKind::Insertion) else {
            return;
        };
        let name = self.region(revision, &[]);
        let mut xml = XmlWriter::fragment();
        xml.empty("text:change-start", &[("text:change-id", &name)]);
        self.emit(xml);
        self.insertion = Some((revision.id, name));
    }

    fn close_insertion(&mut self) {
        if let Some((_, name)) = self.insertion.take() {
            let mut xml = XmlWriter::fragment();
            xml.empty("text:change-end", &[("text:change-id", &name)]);
            self.emit(xml);
        }
    }

    /// A deletion: its runs go into its region, and a mark stays in the
    /// line.
    fn deletion(&mut self, runs: &[Inline]) {
        let Some(Inline::Run(first)) = runs.first() else {
            return;
        };
        let Some(revision) = first.revision.clone() else {
            return;
        };
        let name = self.region(&revision, runs);
        let mut xml = XmlWriter::fragment();
        xml.empty("text:change", &[("text:change-id", &name)]);
        self.emit(xml);
    }

    /// Write a changed region for `revision`, holding `removed` when it is a
    /// deletion, and answer its name.
    fn region(&mut self, revision: &Revision, removed: &[Inline]) -> String {
        self.next_region += 1;
        let name = format!("ct{}", self.next_region);
        let kind = match revision.kind {
            RevisionKind::Insertion => "text:insertion",
            RevisionKind::Deletion => "text:deletion",
        };
        // The removed runs are written as a paragraph of their own, in the
        // body's buffer, then moved into the region.
        let outer = std::mem::take(&mut self.out);
        if revision.kind == RevisionKind::Deletion {
            let mut xml = XmlWriter::fragment();
            xml.open("text:p", &[]);
            self.emit(xml);
            let mut text = TextState { space: true };
            for inline in removed {
                if let Inline::Run(run) = inline {
                    self.run(run, &mut text);
                }
            }
            let mut xml = XmlWriter::fragment();
            xml.close("text:p");
            self.emit(xml);
        }
        let removed = std::mem::replace(&mut self.out, outer);
        let mut xml = XmlWriter::fragment();
        xml.open("text:changed-region", &[("text:id", &name)]);
        xml.open(kind, &[]);
        change_info(&mut xml, &revision.author, revision.date);
        xml.raw(&removed);
        xml.close(kind);
        xml.close("text:changed-region");
        self.regions.push_str(&xml.into_string());
        name
    }

    fn note(&mut self, kind: NoteKind, id: WordId<ambre_docx::model::NoteMarker>) {
        let (notes, class) = match kind {
            NoteKind::Footnote => (&self.document.footnotes, "footnote"),
            NoteKind::Endnote => (&self.document.endnotes, "endnote"),
        };
        let Some(note) = notes.iter().find(|note| note.id == id) else {
            return;
        };
        let count = match kind {
            NoteKind::Footnote => {
                self.next_note.0 += 1;
                self.next_note.0
            }
            NoteKind::Endnote => {
                self.next_note.1 += 1;
                self.next_note.1
            }
        };
        let name = format!("{}{count}", if class == "footnote" { "ftn" } else { "edn" });
        let citation = count.to_string();
        let mut xml = XmlWriter::fragment();
        xml.open(
            "text:note",
            &[("text:id", &name), ("text:note-class", class)],
        );
        xml.leaf("text:note-citation", &[], &citation);
        xml.open("text:note-body", &[]);
        self.emit(xml);
        // A note is a story of its own: the region open in the line stays
        // outside it.
        let outer = self.insertion.take();
        let blocks = note.blocks.clone();
        self.blocks(&blocks);
        self.close_insertion();
        self.insertion = outer;
        let mut xml = XmlWriter::fragment();
        xml.close("text:note-body");
        xml.close("text:note");
        self.emit(xml);
    }

    fn annotation(&mut self, id: WordId<CommentMarker>) {
        let document = self.document;
        let comments = document
            .comments
            .iter()
            .filter(|c| c.id == id || c.parent == Some(id));
        // A comment's body is a story of its own, as a note's is.
        let outer = self.insertion.take();
        for comment in comments {
            let name = comment_name(comment.id);
            let parent = comment.parent.map(comment_name);
            let resolved = if comment.state == CommentState::Resolved {
                "true"
            } else {
                "false"
            };
            let mut attrs = vec![("office:name", name.as_str()), ("loext:resolved", resolved)];
            if let Some(parent) = &parent {
                attrs.push(("loext:parent-name", parent));
            }
            let mut xml = XmlWriter::fragment();
            xml.open("office:annotation", &attrs);
            xml.leaf("dc:creator", &[], &comment.author);
            if let Some(date) = comment.date {
                xml.leaf("dc:date", &[], &date_text(date));
            }
            self.emit(xml);
            for paragraph in &comment.paragraphs {
                self.paragraph(paragraph);
            }
            self.close_insertion();
            let mut xml = XmlWriter::fragment();
            xml.close("office:annotation");
            self.emit(xml);
        }
        self.insertion = outer;
    }

    fn image(&mut self, image: &Image) {
        let at = self.pictures.len() + 1;
        let path = format!("Pictures/image{at}.{}", image.format.extension());
        let name = format!("Image{at}");
        let width = length(image.width);
        let height = length(image.height);
        let mut xml = XmlWriter::fragment();
        xml.open(
            "draw:frame",
            &[
                ("draw:name", &name),
                ("text:anchor-type", "as-char"),
                ("svg:width", &width),
                ("svg:height", &height),
            ],
        );
        xml.empty(
            "draw:image",
            &[
                ("xlink:href", &path),
                ("xlink:type", "simple"),
                ("xlink:show", "embed"),
                ("xlink:actuate", "onLoad"),
            ],
        );
        if !image.description.is_empty() {
            xml.leaf("svg:desc", &[], &image.description);
        }
        xml.close("draw:frame");
        self.emit(xml);
        self.pictures.push((
            path,
            image.format.media_type(),
            Payload::new(image.bytes.as_bytes().to_vec()),
        ));
    }

    fn table(&mut self, table: &Table) {
        self.close_insertion();
        let mut fills: Vec<Rgb> = Vec::new();
        for fill in table
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .filter_map(|c| c.fill)
        {
            if !fills.contains(&fill) {
                fills.push(fill);
            }
        }
        self.styles
            .tables
            .push((table.columns.clone(), fills.clone()));
        let name = format!("Table{}", self.styles.tables.len());
        let mut xml = XmlWriter::fragment();
        xml.open("table:table", &[("table:name", &name)]);
        if table.columns.is_empty() {
            let count = table
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(|c| usize::from(c.span.get()))
                        .sum::<usize>()
                })
                .max()
                .unwrap_or(1)
                .max(1)
                .to_string();
            xml.empty(
                "table:table-column",
                &[("table:number-columns-repeated", &count)],
            );
        }
        for column in 0..table.columns.len() {
            let style = format!("{name}.C{}", column + 1);
            xml.empty("table:table-column", &[("table:style-name", &style)]);
        }
        self.emit(xml);
        // The header rows lead the table, as ODF requires. A header row
        // below a body row is written as a body row.
        let headers = table
            .rows
            .iter()
            .take_while(|row| row.role == RowRole::Header)
            .count();
        for (at, row) in table.rows.iter().enumerate() {
            if at == 0 && headers > 0 {
                let mut xml = XmlWriter::fragment();
                xml.open("table:table-header-rows", &[]);
                self.emit(xml);
            }
            let mut xml = XmlWriter::fragment();
            xml.open("table:table-row", &[]);
            self.emit(xml);
            let mut column = 0_usize;
            for cell in &row.cells {
                let span = usize::from(cell.span.get());
                let fill = cell
                    .fill
                    .and_then(|f| fills.iter().position(|held| *held == f));
                let mut xml = XmlWriter::fragment();
                if cell.merge == VerticalMerge::Continue {
                    for _ in 0..span {
                        xml.empty("table:covered-table-cell", &[]);
                    }
                    self.emit(xml);
                    column += span;
                    continue;
                }
                let span_text = span.to_string();
                let rows = 1 + merged_below(table, at, column);
                let rows_text = rows.to_string();
                let fill_name = fill.map(|f| format!("{name}.F{}", f + 1));
                let mut attrs: Vec<(&str, &str)> = vec![("office:value-type", "string")];
                if let Some(fill_name) = &fill_name {
                    attrs.push(("table:style-name", fill_name));
                }
                if span > 1 {
                    attrs.push(("table:number-columns-spanned", &span_text));
                }
                if rows > 1 {
                    attrs.push(("table:number-rows-spanned", &rows_text));
                }
                xml.open("table:table-cell", &attrs);
                self.emit(xml);
                self.blocks(&cell.blocks);
                self.close_insertion();
                let mut xml = XmlWriter::fragment();
                xml.close("table:table-cell");
                for _ in 1..span {
                    xml.empty("table:covered-table-cell", &[]);
                }
                self.emit(xml);
                column += span;
            }
            let mut xml = XmlWriter::fragment();
            xml.close("table:table-row");
            if at + 1 == headers {
                xml.close("table:table-header-rows");
            }
            self.emit(xml);
        }
        let mut xml = XmlWriter::fragment();
        xml.close("table:table");
        self.emit(xml);
    }
}

/// How many rows below `row` continue the merge the cell at grid column
/// `column` opens.
fn merged_below(table: &Table, row: usize, column: usize) -> usize {
    let mut count = 0;
    for below in table.rows.iter().skip(row + 1) {
        let mut at = 0_usize;
        let continues = below.cells.iter().any(|cell| {
            let here = at;
            at += usize::from(cell.span.get());
            here == column && cell.merge == VerticalMerge::Continue
        });
        if !continues {
            break;
        }
        count += 1;
    }
    count
}

fn same_deletion(inline: &Inline, revision: Option<&Revision>) -> bool {
    match inline {
        Inline::Run(run) => run.revision.as_ref() == revision,
        _ => false,
    }
}

/// Where the text of a paragraph stands: ODF collapses white space, so a
/// space after a space, or at the start, is written as `text:s`.
struct TextState {
    space: bool,
}

fn write_text(xml: &mut XmlWriter, text: &str, state: &mut TextState) {
    let mut plain = String::new();
    let mut spaces = 0_usize;
    let flush_spaces = |xml: &mut XmlWriter, plain: &mut String, spaces: &mut usize| {
        if *spaces > 0 {
            xml.text(plain);
            plain.clear();
            let count = spaces.to_string();
            if *spaces == 1 {
                xml.empty("text:s", &[]);
            } else {
                xml.empty("text:s", &[("text:c", &count)]);
            }
            *spaces = 0;
        }
    };
    for ch in text.chars() {
        match ch {
            ' ' if !state.space && spaces == 0 => {
                plain.push(' ');
                state.space = true;
            }
            ' ' => spaces += 1,
            '\t' => {
                flush_spaces(xml, &mut plain, &mut spaces);
                xml.text(&plain);
                plain.clear();
                xml.empty("text:tab", &[]);
                state.space = false;
            }
            '\n' => {
                flush_spaces(xml, &mut plain, &mut spaces);
                xml.text(&plain);
                plain.clear();
                xml.empty("text:line-break", &[]);
                state.space = true;
            }
            ch => {
                flush_spaces(xml, &mut plain, &mut spaces);
                plain.push(ch);
                state.space = false;
            }
        }
    }
    flush_spaces(xml, &mut plain, &mut spaces);
    xml.text(&plain);
}

fn change_info(xml: &mut XmlWriter, author: &str, date: Option<WordDate>) {
    xml.open("office:change-info", &[]);
    xml.leaf("dc:creator", &[], author);
    if let Some(date) = date {
        xml.leaf("dc:date", &[], &date_text(date));
    }
    xml.close("office:change-info");
}

/// A date as `xsd:dateTime`: a wall time with no zone, or the wall time and
/// its offset from the instant when the model knows both.
fn date_text(date: WordDate) -> String {
    let wall_of = |instant: Instant<Authored>| {
        let text = ambre_docx::time::format(instant);
        text.trim_end_matches('Z').to_owned()
    };
    match date {
        WordDate::Floating(wall) => wall_of(wall),
        WordDate::Zoned { utc, wall } => {
            let offset = i128::from(wall.get()) - i128::from(utc.get());
            let minutes = offset / 60_000;
            // An offset of whole minutes within a day names the instant; any
            // other is no zone a clock keeps, so the instant is written.
            if offset % 60_000 != 0 || minutes.abs() >= 24 * 60 {
                return ambre_docx::time::format(utc);
            }
            let sign = if minutes < 0 { '-' } else { '+' };
            let minutes = minutes.abs();
            format!(
                "{}{sign}{:02}:{:02}",
                wall_of(wall),
                minutes / 60,
                minutes % 60
            )
        }
    }
}

fn comment_name(id: WordId<CommentMarker>) -> String {
    format!("__Annotation__{}", id.get())
}

fn align_value(align: Alignment) -> &'static str {
    match align {
        Alignment::Start => "start",
        Alignment::Center => "center",
        Alignment::End => "end",
        Alignment::Justified => "justify",
    }
}

fn number_format(format: NumberFormat) -> &'static str {
    match format {
        NumberFormat::Decimal => "1",
        NumberFormat::LowerLetter => "a",
        NumberFormat::UpperLetter => "A",
        NumberFormat::LowerRoman => "i",
        NumberFormat::UpperRoman => "I",
    }
}

fn hex(color: Rgb) -> String {
    format!("#{:06x}", color.bits() & 0x00FF_FFFF)
}

fn length(length: PrintLength<Pt>) -> String {
    format!("{}pt", length.get())
}

fn page_layout(xml: &mut XmlWriter, document: &WordDocument) {
    xml.open("style:page-layout", &[("style:name", "pm1")]);
    if let Some(page) = &document.page {
        let width = length(page.width);
        let height = length(page.height);
        let top = length(page.margins.top);
        let bottom = length(page.margins.bottom);
        let left = length(page.margins.left);
        let right = length(page.margins.right);
        let orientation = match page.orientation {
            Orientation::Portrait => "portrait",
            Orientation::Landscape => "landscape",
        };
        xml.empty(
            "style:page-layout-properties",
            &[
                ("fo:page-width", &width),
                ("fo:page-height", &height),
                ("style:print-orientation", orientation),
                ("fo:margin-top", &top),
                ("fo:margin-bottom", &bottom),
                ("fo:margin-left", &left),
                ("fo:margin-right", &right),
            ],
        );
    }
    xml.close("style:page-layout");
}
