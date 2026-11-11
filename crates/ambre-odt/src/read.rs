//! The reader: the parts of an OpenDocument text into the Word model and its
//! report (`4-Prose/features/ambre-spec-prose-import-export.md section 3`).
//!
//! The reader walks `content.xml` once, as a tree, with the styles of
//! `content.xml` and `styles.xml` resolved beside it. It names each construct
//! the model has no form for in the words the Word reader uses, so an
//! OpenDocument file and the same file saved from Word meet the same mapping
//! and the same report. It keeps the text under an element it does not
//! recognise. It never reads a macro, and fetches no address a link or a
//! picture names.

use std::collections::BTreeMap;

use ambre_core::{
    Authored, Bounded, FontSize, Instant, LinkUrl, Payload, PrintLength, Pt, Rgb, Slant,
    Strikethrough, Underline, Weight,
};
use ambre_docx::WordConstruct;
use ambre_docx::model::{
    Alignment, Block, BreakKind, Comment, CommentMarker, CommentState, GridSpan, HeadingDepth,
    Image, ImageBytes, ImageFormat, Inline, Link, LinkTarget, ListDepth, ListInstance, ListKind,
    ListMembership, Margins, Note, NoteKind, NumberFormat, Orientation, PageSetup, Paragraph,
    ParagraphRole, Revision, RevisionKind, RevisionMarker, RowRole, Run, RunFormat, Table,
    TableCell, TableRow, VerticalMerge, VerticalPosition, WordDate, WordDocument, WordId,
};
use ambre_docx::report::{Outcome, WordReport};

use crate::OdtError;
use crate::xml::{Element, Node, Ns};

/// The most grid columns or rows one repeat attribute adds. A hostile file
/// may ask for millions, and a text table holds far fewer.
const MAX_REPEAT: u32 = 1024;

/// The most a run of `text:s` writes, for the same reason.
const MAX_SPACES: u32 = 1024;

/// The parsed parts a reader reads. `content` is the root of `content.xml`,
/// and the other two are absent when the package carries none.
pub struct Parts<'a> {
    pub content: &'a Element,
    pub styles: Option<&'a Element>,
    pub meta: Option<&'a Element>,
}

/// Where the reader finds the bytes of a picture the text embeds.
pub trait Pictures {
    /// The bytes of the part at `path`, or `None` when the package holds
    /// none.
    ///
    /// # Errors
    ///
    /// The package's error, a part past its bound included.
    fn picture(&mut self, path: &str) -> Result<Option<Payload<ImageBytes>>, OdtError>;
}

/// The document the parts hold, and what the reading carried and lost.
///
/// # Errors
///
/// [`OdtError::NotText`] for parts that hold no text document.
pub fn read_parts(
    parts: &Parts<'_>,
    pictures: &mut dyn Pictures,
    macros: bool,
) -> Result<(WordDocument, WordReport), OdtError> {
    let text = parts
        .content
        .child(Ns::Office, "body")
        .and_then(|body| body.child(Ns::Office, "text"))
        .ok_or(OdtError::NotText)?;
    let mut reader = Reader::default();
    if macros {
        reader.report.record(WordConstruct::Macro, Outcome::Dropped);
    }
    for root in [parts.styles, Some(parts.content)].into_iter().flatten() {
        reader.read_styles(root);
    }
    if let Some(changes) = text.child(Ns::Text, "tracked-changes") {
        reader.read_changes(changes);
    }
    let mut document = WordDocument {
        body: reader.blocks(text, &mut Scope::default(), pictures)?,
        ..WordDocument::default()
    };
    if let Some(styles) = parts.styles {
        reader.master_page(styles, &mut document, pictures)?;
    }
    document.title = parts
        .meta
        .and_then(|meta| meta.child(Ns::Office, "meta"))
        .and_then(|meta| meta.child(Ns::DublinCore, "title"))
        .map(Element::text)
        .filter(|title| !title.is_empty());
    document.footnotes = std::mem::take(&mut reader.footnotes);
    document.endnotes = std::mem::take(&mut reader.endnotes);
    document.comments = std::mem::take(&mut reader.comments);
    Ok((document, reader.report))
}

/// A style family of ODF: a name is unique within its family alone.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    Paragraph,
    Text,
    TableColumn,
    TableCell,
    Other,
}

impl Family {
    fn of(name: Option<&str>) -> Self {
        match name {
            Some("paragraph") => Self::Paragraph,
            Some("text") => Self::Text,
            Some("table-column") => Self::TableColumn,
            Some("table-cell") => Self::TableCell,
            _ => Self::Other,
        }
    }
}

/// One style, as the reader resolves it.
#[derive(Default)]
struct StyleDef {
    /// The display name, decoded and in lower case: the role a named style
    /// gives.
    name: String,
    parent: Option<String>,
    /// Whether the style sits among the automatic styles: a paragraph's own
    /// format, not a style the user named.
    automatic: bool,
    outline: Option<u8>,
    align: Option<Alignment>,
    list_style: Option<String>,
    break_before: Option<BreakKind>,
    break_after: Option<BreakKind>,
    run: RunProps,
    column_width: Option<PrintLength<Pt>>,
    fill: Option<Rgb>,
}

/// The format of a run as one style sets it. `None` leaves the style below
/// in force.
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
    small_caps: Option<bool>,
    caps: Option<bool>,
    hidden: Option<bool>,
    double_strike: Option<bool>,
}

impl RunProps {
    fn read(properties: &Element) -> Self {
        let fo = |name: &str| properties.attr(Ns::Fo, name);
        let style = |name: &str| properties.attr(Ns::Style, name);
        let line = |name: &str| style(name).map(|value| value != "none");
        let through = line("text-line-through-style")
            .or_else(|| style("text-line-through-type").map(|value| value != "none"));
        Self {
            bold: fo("font-weight").map(|w| Weight::from_bool(is_bold(w))),
            italic: fo("font-style").map(|s| Slant::from_bool(s != "normal")),
            underline: line("text-underline-style").map(Underline::from_bool),
            strike: through.map(Strikethrough::from_bool),
            double_strike: style("text-line-through-type").map(|value| value == "double"),
            position: style("text-position").map(position),
            size: fo("font-size")
                .and_then(points)
                .and_then(|pt| FontSize::new(pt.get())),
            color: fo("color").and_then(hex_color),
            fill: fo("background-color").and_then(hex_color),
            font: style("font-name")
                .or_else(|| fo("font-family"))
                .map(|name| name.trim_matches(['\'', '"']).to_owned())
                .filter(|name| !name.is_empty()),
            small_caps: fo("font-variant").map(|v| v == "small-caps"),
            caps: fo("text-transform").map(|v| v == "uppercase"),
            hidden: properties.attr(Ns::Text, "display").map(|v| v == "none"),
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
        self.hidden = top.hidden.or(self.hidden);
        self.double_strike = top.double_strike.or(self.double_strike);
        if top.font.is_some() {
            self.font.clone_from(&top.font);
        }
    }

    fn format(&self) -> RunFormat {
        RunFormat {
            bold: self.bold.unwrap_or_default(),
            italic: self.italic.unwrap_or_default(),
            underline: self.underline.unwrap_or_default(),
            strike: self.strike.unwrap_or_default(),
            position: self.position.unwrap_or_default(),
            size: self.size,
            color: self.color,
            fill: self.fill,
            font: self.font.clone(),
        }
    }
}

/// One level of a list style: what its paragraphs show before their text.
#[derive(Clone, Copy)]
struct ListLevel {
    /// `None` for a level that shows nothing, which makes no list.
    kind: Option<ListKind>,
    exact: bool,
}

/// A tracked change of `text:tracked-changes`.
struct Change {
    kind: ChangeKind,
    id: WordId<RevisionMarker>,
    author: String,
    date: Option<WordDate>,
    /// The paragraphs a deletion removed.
    removed: Vec<Element>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    Insertion,
    Deletion,
    Format,
}

/// Where the walk stands in the nesting of lists.
#[derive(Clone, Default)]
struct Scope {
    /// The list style in force and the nesting depth, 1 inside a list.
    list: Option<(Option<String>, u8, WordId<ListInstance>)>,
}

#[derive(Default)]
struct Reader {
    report: WordReport,
    styles: BTreeMap<(Family, String), StyleDef>,
    lists: BTreeMap<String, Vec<ListLevel>>,
    changes: BTreeMap<String, Change>,
    /// The tracked insertion the walk stands inside, between its start and
    /// its end marks. It may span paragraphs.
    insertion: Option<Revision>,
    comment_ids: BTreeMap<String, WordId<CommentMarker>>,
    comments: Vec<Comment>,
    footnotes: Vec<Note>,
    endnotes: Vec<Note>,
    next_list: u32,
    last_list: Option<WordId<ListInstance>>,
    /// The list each `xml:id` names, for a list that continues it.
    list_ids: BTreeMap<String, WordId<ListInstance>>,
}

impl Reader {
    fn read_styles(&mut self, root: &Element) {
        for group in root.elements() {
            let automatic = group.is(Ns::Office, "automatic-styles");
            if !automatic && !group.is(Ns::Office, "styles") {
                continue;
            }
            for style in group.elements() {
                if style.is(Ns::Text, "list-style") {
                    if let Some(name) = style.attr(Ns::Style, "name") {
                        self.lists.insert(name.to_owned(), list_levels(style));
                    }
                    continue;
                }
                if !style.is(Ns::Style, "style") {
                    continue;
                }
                let Some(name) = style.attr(Ns::Style, "name") else {
                    continue;
                };
                let family = Family::of(style.attr(Ns::Style, "family"));
                let display = style.attr(Ns::Style, "display-name").unwrap_or(name);
                let paragraph = style.child(Ns::Style, "paragraph-properties");
                let pick = |name: &str| paragraph.and_then(|p| p.attr(Ns::Fo, name));
                let def = StyleDef {
                    name: decode_name(display).to_lowercase(),
                    parent: style
                        .attr(Ns::Style, "parent-style-name")
                        .map(str::to_owned),
                    automatic,
                    outline: style
                        .attr(Ns::Style, "default-outline-level")
                        .and_then(|level| level.parse().ok()),
                    align: pick("text-align").and_then(alignment),
                    list_style: style
                        .attr(Ns::Style, "list-style-name")
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned),
                    break_before: pick("break-before").and_then(break_kind),
                    break_after: pick("break-after").and_then(break_kind),
                    run: style
                        .child(Ns::Style, "text-properties")
                        .map(RunProps::read)
                        .unwrap_or_default(),
                    column_width: style
                        .child(Ns::Style, "table-column-properties")
                        .and_then(|p| p.attr(Ns::Style, "column-width"))
                        .and_then(points),
                    fill: style
                        .child(Ns::Style, "table-cell-properties")
                        .and_then(|p| p.attr(Ns::Fo, "background-color"))
                        .and_then(hex_color),
                };
                self.styles.insert((family, name.to_owned()), def);
            }
        }
    }

    /// The style `name` of `family`, then each style it derives from. A
    /// loop stops at the first style met twice.
    fn chain(&self, family: Family, name: Option<&str>) -> Vec<&StyleDef> {
        let mut out: Vec<&StyleDef> = Vec::new();
        let mut next = name.map(str::to_owned);
        while let Some(name) = next.take() {
            let Some(def) = self.styles.get(&(family, name)) else {
                break;
            };
            if out.iter().any(|seen| std::ptr::eq(*seen, def)) {
                break;
            }
            next.clone_from(&def.parent);
            out.push(def);
        }
        out
    }

    fn read_changes(&mut self, changes: &Element) {
        for region in changes.elements() {
            let Some(name) = region
                .attr(Ns::Text, "id")
                .or_else(|| region.attr(Ns::None, "id"))
            else {
                continue;
            };
            let Some(body) = region.elements().next() else {
                continue;
            };
            let kind = match body.name() {
                "insertion" => ChangeKind::Insertion,
                "deletion" => ChangeKind::Deletion,
                _ => ChangeKind::Format,
            };
            let info = body.child(Ns::Office, "change-info");
            let id = WordId::new(u32::try_from(self.changes.len()).unwrap_or(u32::MAX));
            let removed = body
                .elements()
                .filter(|e| !e.is(Ns::Office, "change-info"))
                .cloned()
                .collect();
            self.changes.insert(
                name.to_owned(),
                Change {
                    kind,
                    id,
                    author: info
                        .and_then(|i| i.child(Ns::DublinCore, "creator"))
                        .map(Element::text)
                        .unwrap_or_default(),
                    date: info
                        .and_then(|i| i.child(Ns::DublinCore, "date"))
                        .and_then(|d| odf_date(&d.text())),
                    removed,
                },
            );
        }
    }

    fn revision(&self, change: &Change) -> Revision {
        Revision {
            kind: match change.kind {
                ChangeKind::Deletion => RevisionKind::Deletion,
                ChangeKind::Insertion | ChangeKind::Format => RevisionKind::Insertion,
            },
            id: change.id,
            author: change.author.clone(),
            date: change.date,
        }
    }

    /// The blocks under `parent`: a body, a cell, a note, a band or a
    /// section.
    fn blocks(
        &mut self,
        parent: &Element,
        scope: &mut Scope,
        pictures: &mut dyn Pictures,
    ) -> Result<Vec<Block>, OdtError> {
        let mut out = Vec::new();
        self.blocks_into(parent, scope, pictures, &mut out)?;
        Ok(out)
    }

    fn blocks_into(
        &mut self,
        parent: &Element,
        scope: &mut Scope,
        pictures: &mut dyn Pictures,
        out: &mut Vec<Block>,
    ) -> Result<(), OdtError> {
        for element in parent.elements() {
            match (element.ns, element.name()) {
                (Ns::Text, "p" | "h") => {
                    let paragraph = self.paragraph(element, scope, pictures)?;
                    out.push(Block::Paragraph(paragraph));
                }
                (Ns::Text, "list") => self.list(element, scope, pictures, out)?,
                (Ns::Text, "numbered-paragraph") => {
                    let depth = element
                        .attr(Ns::Text, "level")
                        .and_then(|level| level.parse::<u8>().ok())
                        .unwrap_or(1);
                    let id = self.new_list(false);
                    let mut inner = Scope {
                        list: Some((
                            element.attr(Ns::Text, "style-name").map(str::to_owned),
                            depth,
                            id,
                        )),
                    };
                    self.blocks_into(element, &mut inner, pictures, out)?;
                }
                (Ns::Table, "table") => {
                    let table = self.table(element, pictures)?;
                    out.push(Block::Table(table));
                }
                (Ns::Text, "section") => {
                    self.report
                        .record(WordConstruct::Section, Outcome::Approximated);
                    self.blocks_into(element, scope, pictures, out)?;
                }
                (
                    Ns::Text,
                    "table-of-content" | "illustration-index" | "table-index" | "object-index"
                    | "user-index" | "alphabetical-index" | "bibliography",
                ) => {
                    self.report
                        .record(WordConstruct::Field, Outcome::Approximated);
                    if let Some(body) = element.child(Ns::Text, "index-body") {
                        self.blocks_into(body, scope, pictures, out)?;
                    }
                }
                (Ns::Text, "index-title") => self.blocks_into(element, scope, pictures, out)?,
                (Ns::Text, "change") => {
                    // A deletion that removed whole paragraphs.
                    let removed = self.removed(element, pictures)?;
                    out.extend(removed.into_iter().map(Block::Paragraph));
                }
                (Ns::Text, "change-start") => self.start_change(element),
                (Ns::Text, "change-end") => self.insertion = None,
                (Ns::Draw, _) => {
                    let mut inlines = Vec::new();
                    self.drawing(element, pictures, &mut inlines)?;
                    if !inlines.is_empty() {
                        out.push(Block::Paragraph(Paragraph {
                            inlines,
                            ..Paragraph::default()
                        }));
                    }
                }
                (
                    Ns::Text,
                    "tracked-changes"
                    | "sequence-decls"
                    | "variable-decls"
                    | "user-field-decls"
                    | "dde-connection-decls"
                    | "soft-page-break"
                    | "bookmark-end"
                    | "alphabetical-index-auto-mark-file",
                )
                | (Ns::Office, "forms")
                | (Ns::Table, "calculation-settings" | "content-validations")
                | (Ns::Style, _) => {}
                (Ns::Text, "bookmark" | "bookmark-start") => self.bookmark(element),
                (Ns::Office, "scripts" | "event-listeners") => {
                    self.report.record(WordConstruct::Macro, Outcome::Dropped);
                }
                _ => {
                    self.report
                        .record(WordConstruct::UnknownElement, Outcome::Dropped);
                    self.blocks_into(element, scope, pictures, out)?;
                }
            }
        }
        Ok(())
    }

    fn new_list(&mut self, continued: bool) -> WordId<ListInstance> {
        if continued && let Some(id) = self.last_list {
            return id;
        }
        self.next_list = self.next_list.saturating_add(1);
        let id = WordId::new(self.next_list);
        self.last_list = Some(id);
        id
    }

    fn list(
        &mut self,
        element: &Element,
        scope: &Scope,
        pictures: &mut dyn Pictures,
        out: &mut Vec<Block>,
    ) -> Result<(), OdtError> {
        let own = element.attr(Ns::Text, "style-name").map(str::to_owned);
        let inner = match &scope.list {
            Some((style, depth, id)) => {
                (own.or_else(|| style.clone()), depth.saturating_add(1), *id)
            }
            None => {
                let id = match element.attr(Ns::Text, "continue-list") {
                    Some(name) => match self.list_ids.get(name).copied() {
                        Some(id) => {
                            self.last_list = Some(id);
                            id
                        }
                        None => self.new_list(true),
                    },
                    None => {
                        self.new_list(element.attr(Ns::Text, "continue-numbering") == Some("true"))
                    }
                };
                if let Some(name) = element.attr(Ns::Xml, "id") {
                    self.list_ids.insert(name.to_owned(), id);
                }
                (own, 1, id)
            }
        };
        let mut scope = Scope { list: Some(inner) };
        for item in element.elements() {
            if item.is(Ns::Text, "list-item") || item.is(Ns::Text, "list-header") {
                self.blocks_into(item, &mut scope, pictures, out)?;
            }
        }
        Ok(())
    }

    /// The paragraph's place in the list the walk stands in.
    fn membership(&mut self, scope: &Scope, style: Option<&str>) -> Option<ListMembership> {
        let (list_style, depth, id) = scope.list.as_ref()?;
        let list_style = list_style.clone().or_else(|| {
            self.chain(Family::Paragraph, style)
                .into_iter()
                .find_map(|def| def.list_style.clone())
        });
        let level = Bounded::<ListDepth>::saturating_from(i64::from(*depth) - 1);
        let defined = list_style
            .and_then(|name| self.lists.get(&name))
            .and_then(|levels| levels.get(usize::from(level.get())).copied());
        let kind = match defined {
            Some(ListLevel { kind: None, .. }) => return None,
            Some(ListLevel {
                kind: Some(kind),
                exact,
            }) => {
                if !exact {
                    self.report
                        .record(WordConstruct::ListFormat, Outcome::Approximated);
                }
                kind
            }
            None => ListKind::Bullet,
        };
        Some(ListMembership {
            list: *id,
            level,
            kind,
        })
    }

    fn paragraph(
        &mut self,
        element: &Element,
        scope: &Scope,
        pictures: &mut dyn Pictures,
    ) -> Result<Paragraph, OdtError> {
        let style = element.attr(Ns::Text, "style-name");
        let chain = self.chain(Family::Paragraph, style);
        let role = if element.name() == "h" {
            let level = element
                .attr(Ns::Text, "outline-level")
                .and_then(|level| level.parse::<u8>().ok())
                .unwrap_or(1);
            ParagraphRole::Heading(
                Bounded::<HeadingDepth>::new(level.clamp(1, 9))
                    .unwrap_or(Bounded::<HeadingDepth>::MIN),
            )
        } else if chain.is_empty() {
            // A style the file names and never defines reads by its name,
            // as LibreOffice reads a built-in one.
            style
                .map(|name| decode_name(name).to_lowercase())
                .and_then(|name| ParagraphRole::from_style_name(&name))
                .unwrap_or_default()
        } else {
            chain
                .iter()
                .find_map(|def| {
                    ParagraphRole::from_style_name(&def.name).or_else(|| {
                        def.outline
                            .and_then(Bounded::<HeadingDepth>::new)
                            .map(ParagraphRole::Heading)
                    })
                })
                .unwrap_or_default()
        };
        let align = chain.iter().find_map(|def| def.align);
        let before = chain.iter().find_map(|def| def.break_before);
        let after = chain.iter().find_map(|def| def.break_after);
        // A paragraph's own automatic style is its direct format: its runs
        // take it, as a Word run takes the format set on it. A named style
        // stays off the run, as the Word reader keeps it.
        let base = chain
            .first()
            .filter(|def| def.automatic)
            .map(|def| def.run.clone())
            .unwrap_or_default();
        let list = self.membership(scope, style);
        let mut inlines = Vec::new();
        if let Some(kind) = before {
            inlines.push(Inline::Break(kind));
        }
        let mut state = InlineState {
            format: base,
            link: None,
            space: true,
        };
        self.inlines(element, &mut state, pictures, &mut inlines)?;
        if let Some(kind) = after {
            inlines.push(Inline::Break(kind));
        }
        Ok(Paragraph {
            role,
            list,
            align,
            inlines,
        })
    }

    /// The inlines under `parent`, a paragraph or a container inside one.
    fn inlines(
        &mut self,
        parent: &Element,
        state: &mut InlineState,
        pictures: &mut dyn Pictures,
        out: &mut Vec<Inline>,
    ) -> Result<(), OdtError> {
        for node in &parent.children {
            let element = match node {
                Node::Text(text) => {
                    let text = state.collapse(text);
                    self.push_text(&text, state, out);
                    continue;
                }
                Node::Element(element) => element,
            };
            match (element.ns, element.name()) {
                (Ns::Text, "span") => {
                    let outer = state.format.clone();
                    let style = element.attr(Ns::Text, "style-name");
                    let mut props = RunProps::default();
                    for def in self.chain(Family::Text, style).iter().rev() {
                        props.overlay(&def.run);
                    }
                    state.format.overlay(&props);
                    self.inlines(element, state, pictures, out)?;
                    state.format = outer;
                }
                (Ns::Text, "a") => {
                    let target =
                        element
                            .attr(Ns::XLink, "href")
                            .map(|href| match href.strip_prefix('#') {
                                Some(anchor) => LinkTarget::Anchor(anchor.to_owned()),
                                None => LinkTarget::External(LinkUrl::new(href.to_owned())),
                            });
                    let outer = state.link.take();
                    state.link = target.map(|target| (target, Vec::new()));
                    self.inlines(element, state, pictures, out)?;
                    let link = std::mem::replace(&mut state.link, outer);
                    if let Some((target, runs)) = link
                        && !runs.is_empty()
                    {
                        out.push(Inline::Link(Link { target, runs }));
                    }
                }
                (Ns::Text, "s") => {
                    let count = element
                        .attr(Ns::Text, "c")
                        .and_then(|c| c.parse::<u32>().ok())
                        .unwrap_or(1)
                        .min(MAX_SPACES);
                    let spaces: String = " ".repeat(usize::try_from(count).unwrap_or(1));
                    self.push_text(&spaces, state, out);
                    state.space = false;
                }
                (Ns::Text, "tab") => {
                    self.flush_link(state, out);
                    out.push(Inline::Tab);
                    state.space = false;
                }
                (Ns::Text, "line-break") => {
                    self.flush_link(state, out);
                    out.push(Inline::Break(BreakKind::Line));
                    state.space = true;
                }
                (Ns::Text, "note") => {
                    self.flush_link(state, out);
                    self.note(element, pictures, out)?;
                }
                (Ns::Office, "annotation") => {
                    self.flush_link(state, out);
                    self.annotation(element, pictures, out)?;
                }
                (Ns::Office, "annotation-end") => {
                    if let Some(id) = element
                        .attr(Ns::Office, "name")
                        .and_then(|name| self.comment_ids.get(name))
                    {
                        self.flush_link(state, out);
                        out.push(Inline::CommentEnd(*id));
                    }
                }
                (Ns::Text, "change-start") => self.start_change(element),
                (Ns::Text, "change-end") => self.insertion = None,
                (Ns::Text, "change") => {
                    self.flush_link(state, out);
                    for paragraph in self.removed(element, pictures)? {
                        out.extend(paragraph.inlines);
                    }
                }
                (Ns::Text, "bookmark" | "bookmark-start") => self.bookmark(element),
                (
                    Ns::Text,
                    "bookmark-end"
                    | "soft-page-break"
                    | "reference-mark"
                    | "reference-mark-start"
                    | "reference-mark-end",
                ) => {}
                (Ns::Text, "meta") => self.inlines(element, state, pictures, out)?,
                (
                    Ns::Text,
                    "toc-mark"
                    | "toc-mark-start"
                    | "toc-mark-end"
                    | "alphabetical-index-mark"
                    | "alphabetical-index-mark-start"
                    | "alphabetical-index-mark-end"
                    | "user-index-mark"
                    | "user-index-mark-start"
                    | "user-index-mark-end",
                ) => {
                    self.report
                        .record(WordConstruct::Field, Outcome::Approximated);
                }
                (Ns::Text, "hidden-text" | "hidden-paragraph") => {
                    self.report
                        .record(WordConstruct::HiddenText, Outcome::Approximated);
                    self.inlines(element, state, pictures, out)?;
                }
                (Ns::Text, name) if is_field(name) => {
                    self.report
                        .record(WordConstruct::Field, Outcome::Approximated);
                    self.inlines(element, state, pictures, out)?;
                }
                (Ns::Draw, _) => {
                    self.flush_link(state, out);
                    self.drawing(element, pictures, out)?;
                }
                (Ns::Office, "scripts" | "event-listeners") => {
                    self.report.record(WordConstruct::Macro, Outcome::Dropped);
                }
                _ => {
                    self.report
                        .record(WordConstruct::UnknownElement, Outcome::Dropped);
                    self.inlines(element, state, pictures, out)?;
                }
            }
        }
        self.flush_link(state, out);
        Ok(())
    }

    /// Append `text` as a run in the format, the revision and the link in
    /// force. A run of the same format and revision grows instead.
    fn push_text(&mut self, text: &str, state: &mut InlineState, out: &mut Vec<Inline>) {
        if text.is_empty() {
            return;
        }
        self.effects(&state.format);
        let run = Run {
            text: text.to_owned(),
            format: state.format.format(),
            revision: self.insertion.clone(),
        };
        let runs = match &mut state.link {
            Some((_, runs)) => runs,
            None => {
                if let Some(Inline::Run(last)) = out.last_mut()
                    && last.format == run.format
                    && last.revision == run.revision
                {
                    last.text.push_str(&run.text);
                    return;
                }
                out.push(Inline::Run(run));
                return;
            }
        };
        match runs.last_mut() {
            Some(last) if last.format == run.format && last.revision == run.revision => {
                last.text.push_str(&run.text);
            }
            _ => runs.push(run),
        }
    }

    /// Name each run effect the model has no form for, as the Word reader
    /// names it.
    fn effects(&mut self, props: &RunProps) {
        let effects = [
            (props.small_caps, WordConstruct::SmallCaps, Outcome::Dropped),
            (props.caps, WordConstruct::AllCaps, Outcome::Dropped),
            (
                props.hidden,
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
    }

    /// Close the link the walk stands in, if any, so a tab, a note or a
    /// comment mark lands after the runs it follows. Text after it in the
    /// same link opens a link of the same target.
    fn flush_link(&self, state: &mut InlineState, out: &mut Vec<Inline>) {
        if let Some((target, runs)) = &mut state.link
            && !runs.is_empty()
        {
            out.push(Inline::Link(Link {
                target: target.clone(),
                runs: std::mem::take(runs),
            }));
        }
    }

    fn start_change(&mut self, element: &Element) {
        let Some(change) = element
            .attr(Ns::Text, "change-id")
            .and_then(|name| self.changes.get(name))
        else {
            return;
        };
        match change.kind {
            ChangeKind::Format => {
                self.report
                    .record(WordConstruct::FormatChange, Outcome::Dropped);
            }
            ChangeKind::Insertion | ChangeKind::Deletion => {
                self.report
                    .record(WordConstruct::TrackedChange, Outcome::Carried);
                self.insertion = Some(self.revision(change));
            }
        }
    }

    /// The paragraphs a deletion mark removed, each run marked deleted.
    fn removed(
        &mut self,
        element: &Element,
        pictures: &mut dyn Pictures,
    ) -> Result<Vec<Paragraph>, OdtError> {
        let Some(change) = element
            .attr(Ns::Text, "change-id")
            .and_then(|name| self.changes.get(name))
        else {
            return Ok(Vec::new());
        };
        if change.kind != ChangeKind::Deletion {
            return Ok(Vec::new());
        }
        let revision = self.revision(change);
        let removed = change.removed.clone();
        self.report
            .record(WordConstruct::TrackedChange, Outcome::Carried);
        if removed.len() > 1 {
            self.report
                .record(WordConstruct::ParagraphMarkChange, Outcome::Approximated);
        }
        let outer = self.insertion.replace(revision);
        let mut out = Vec::new();
        for block in &removed {
            if block.is(Ns::Text, "p") || block.is(Ns::Text, "h") {
                out.push(self.paragraph(block, &Scope::default(), pictures)?);
            } else {
                self.report
                    .record(WordConstruct::UnknownElement, Outcome::Dropped);
            }
        }
        self.insertion = outer;
        Ok(out)
    }

    fn bookmark(&mut self, element: &Element) {
        // LibreOffice and Word write hidden bookmarks whose names start
        // with an underscore, which no user named.
        if !element
            .attr(Ns::Text, "name")
            .is_some_and(|name| name.starts_with('_'))
        {
            self.report
                .record(WordConstruct::Bookmark, Outcome::Dropped);
        }
    }

    fn note(
        &mut self,
        element: &Element,
        pictures: &mut dyn Pictures,
        out: &mut Vec<Inline>,
    ) -> Result<(), OdtError> {
        let kind = match element.attr(Ns::Text, "note-class") {
            Some("endnote") => NoteKind::Endnote,
            _ => NoteKind::Footnote,
        };
        // A note's body is its own story: an insertion open around the call
        // does not cover it.
        let outer = self.insertion.take();
        let blocks = match element.child(Ns::Text, "note-body") {
            Some(body) => self.blocks(body, &mut Scope::default(), pictures)?,
            None => Vec::new(),
        };
        self.insertion = outer;
        let (notes, construct) = match kind {
            NoteKind::Footnote => (&mut self.footnotes, WordConstruct::Footnote),
            NoteKind::Endnote => (&mut self.endnotes, WordConstruct::Endnote),
        };
        let id = WordId::new(
            u32::try_from(notes.len())
                .unwrap_or(u32::MAX)
                .saturating_add(1),
        );
        notes.push(Note { id, blocks });
        self.report.record(construct, Outcome::Carried);
        out.push(Inline::NoteCall(kind, id));
        Ok(())
    }

    fn annotation(
        &mut self,
        element: &Element,
        pictures: &mut dyn Pictures,
        out: &mut Vec<Inline>,
    ) -> Result<(), OdtError> {
        let id = WordId::new(u32::try_from(self.comments.len()).unwrap_or(u32::MAX));
        let name = element.attr(Ns::Office, "name");
        if let Some(name) = name {
            self.comment_ids.insert(name.to_owned(), id);
        }
        let parent = element
            .attr(Ns::LoExt, "parent-name")
            .or_else(|| element.attr(Ns::Office, "parent-name"))
            .and_then(|parent| self.comment_ids.get(parent).copied());
        // A comment's body is its own story, as a note's is.
        let outer = self.insertion.take();
        let mut paragraphs = Vec::new();
        for p in element
            .elements()
            .filter(|e| e.is(Ns::Text, "p") || e.is(Ns::Text, "h"))
        {
            paragraphs.push(self.paragraph(p, &Scope::default(), pictures)?);
        }
        self.insertion = outer;
        let resolved = element.attr(Ns::LoExt, "resolved") == Some("true");
        self.comments.push(Comment {
            id,
            author: element
                .child(Ns::DublinCore, "creator")
                .map(Element::text)
                .unwrap_or_default(),
            date: element
                .child(Ns::DublinCore, "date")
                .and_then(|d| odf_date(&d.text())),
            paragraphs,
            parent,
            state: if resolved {
                CommentState::Resolved
            } else {
                CommentState::Open
            },
        });
        self.report.record(WordConstruct::Comment, Outcome::Carried);
        // A reply anchors on its thread's range, so it marks nothing.
        if parent.is_some() {
            return Ok(());
        }
        out.push(Inline::CommentStart(id));
        // A comment with no end mark covers no text.
        if name.is_none() {
            out.push(Inline::CommentEnd(id));
        }
        Ok(())
    }

    /// A frame or a shape: a picture in the text, or a construct the model
    /// has no form for.
    fn drawing(
        &mut self,
        element: &Element,
        pictures: &mut dyn Pictures,
        out: &mut Vec<Inline>,
    ) -> Result<(), OdtError> {
        match element.name() {
            "frame" => {}
            "a" => {
                for inner in element.elements() {
                    self.drawing(inner, pictures, out)?;
                }
                return Ok(());
            }
            _ => {
                self.report.record(WordConstruct::Drawing, Outcome::Dropped);
                return Ok(());
            }
        }
        let Some(content) = element.elements().find(|e| e.ns == Ns::Draw) else {
            self.report.record(WordConstruct::Drawing, Outcome::Dropped);
            return Ok(());
        };
        match content.name() {
            "image" => {}
            "object" | "object-ole" | "plugin" | "applet" | "floating-frame" => {
                self.report
                    .record(WordConstruct::EmbeddedObject, Outcome::Dropped);
                return Ok(());
            }
            _ => {
                self.report.record(WordConstruct::Drawing, Outcome::Dropped);
                return Ok(());
            }
        }
        let href = content.attr(Ns::XLink, "href").unwrap_or_default();
        let inside = !href.is_empty()
            && !href.contains("://")
            && !href.starts_with('/')
            && !href.starts_with("..");
        if !inside {
            let construct = if content.child(Ns::Office, "binary-data").is_some() {
                WordConstruct::PictureFormat
            } else {
                WordConstruct::LinkedPicture
            };
            self.report.record(construct, Outcome::Dropped);
            return Ok(());
        }
        let path = href.trim_start_matches("./");
        let format = path
            .rsplit_once('.')
            .and_then(|(_, extension)| ImageFormat::from_extension(extension));
        let Some(format) = format else {
            self.report
                .record(WordConstruct::PictureFormat, Outcome::Dropped);
            return Ok(());
        };
        let Some(bytes) = pictures.picture(path)? else {
            self.report.record(WordConstruct::Drawing, Outcome::Dropped);
            return Ok(());
        };
        if element.attr(Ns::Text, "anchor-type") != Some("as-char") {
            self.report
                .record(WordConstruct::FloatingPicture, Outcome::Approximated);
        }
        let length = |name: &str| {
            element
                .attr(Ns::Svg, name)
                .and_then(points)
                .unwrap_or(PrintLength::ZERO)
        };
        let description = element
            .child(Ns::Svg, "desc")
            .or_else(|| element.child(Ns::Svg, "title"))
            .map(Element::text)
            .unwrap_or_default();
        self.report.record(WordConstruct::Picture, Outcome::Carried);
        out.push(Inline::Image(Image {
            description,
            width: length("width"),
            height: length("height"),
            format,
            bytes,
        }));
        Ok(())
    }

    fn table(&mut self, element: &Element, pictures: &mut dyn Pictures) -> Result<Table, OdtError> {
        self.report.record(WordConstruct::Table, Outcome::Carried);
        let mut table = Table::default();
        let mut grid = Grid::default();
        self.table_parts(element, RowRole::Body, &mut table, &mut grid, pictures)?;
        Ok(table)
    }

    fn table_parts(
        &mut self,
        parent: &Element,
        role: RowRole,
        table: &mut Table,
        grid: &mut Grid,
        pictures: &mut dyn Pictures,
    ) -> Result<(), OdtError> {
        for part in parent.elements() {
            match (part.ns, part.name()) {
                (Ns::Table, "table-column") => {
                    let width = self
                        .chain(Family::TableColumn, part.attr(Ns::Table, "style-name"))
                        .into_iter()
                        .find_map(|def| def.column_width)
                        .unwrap_or(PrintLength::ZERO);
                    for _ in 0..repeat(part.attr(Ns::Table, "number-columns-repeated")) {
                        if table.columns.len() < usize::try_from(MAX_REPEAT).unwrap_or(0) {
                            table.columns.push(width);
                        }
                    }
                }
                (Ns::Table, "table-columns" | "table-column-group" | "table-header-columns") => {
                    self.table_parts(part, role, table, grid, pictures)?;
                }
                (Ns::Table, "table-header-rows") => {
                    self.table_parts(part, RowRole::Header, table, grid, pictures)?;
                }
                (Ns::Table, "table-rows" | "table-row-group") => {
                    self.table_parts(part, role, table, grid, pictures)?;
                }
                (Ns::Table, "table-row") => {
                    let times = repeat(part.attr(Ns::Table, "number-rows-repeated"));
                    for _ in 0..times {
                        if table.rows.len() >= usize::try_from(MAX_REPEAT).unwrap_or(0) {
                            break;
                        }
                        let row = self.row(part, role, grid, pictures)?;
                        table.rows.push(row);
                    }
                }
                (Ns::Table, "title" | "desc") | (Ns::Text, "soft-page-break") => {}
                _ => {
                    self.report
                        .record(WordConstruct::UnknownElement, Outcome::Dropped);
                }
            }
        }
        Ok(())
    }

    fn row(
        &mut self,
        element: &Element,
        role: RowRole,
        grid: &mut Grid,
        pictures: &mut dyn Pictures,
    ) -> Result<TableRow, OdtError> {
        let mut row = TableRow {
            role,
            cells: Vec::new(),
        };
        // The grid column the walk stands at, and the columns left of a
        // horizontal span.
        let mut column = 0_usize;
        let mut skip = 0_u32;
        for cell in element.elements() {
            let covered = cell.is(Ns::Table, "covered-table-cell");
            if !covered && !cell.is(Ns::Table, "table-cell") {
                continue;
            }
            for _ in 0..repeat(cell.attr(Ns::Table, "number-columns-repeated")) {
                if covered {
                    if skip > 0 {
                        skip -= 1;
                        column += 1;
                        continue;
                    }
                    // A cell below a cell that spans rows continues it.
                    match grid.below.get_mut(&column) {
                        Some((left, span)) if *left > 0 => {
                            *left -= 1;
                            let span = *span;
                            row.cells.push(TableCell {
                                span: Bounded::<GridSpan>::saturating_from(i64::from(span)),
                                merge: VerticalMerge::Continue,
                                ..TableCell::default()
                            });
                            skip = span.saturating_sub(1);
                        }
                        _ => {}
                    }
                    column += 1;
                    continue;
                }
                let span = repeat(cell.attr(Ns::Table, "number-columns-spanned"));
                let rows = repeat(cell.attr(Ns::Table, "number-rows-spanned"));
                let merge = if rows > 1 {
                    grid.below.insert(column, (rows - 1, span));
                    VerticalMerge::Restart
                } else {
                    VerticalMerge::None
                };
                let fill = self
                    .chain(Family::TableCell, cell.attr(Ns::Table, "style-name"))
                    .into_iter()
                    .find_map(|def| def.fill);
                let blocks = self.blocks(cell, &mut Scope::default(), pictures)?;
                row.cells.push(TableCell {
                    span: Bounded::<GridSpan>::saturating_from(i64::from(span)),
                    merge,
                    fill,
                    blocks,
                });
                skip = span.saturating_sub(1);
                column += 1;
                if row.cells.len() >= usize::try_from(MAX_REPEAT).unwrap_or(0) {
                    return Ok(row);
                }
            }
        }
        Ok(row)
    }

    /// The page, the header and the footer of the first master page.
    fn master_page(
        &mut self,
        styles: &Element,
        document: &mut WordDocument,
        pictures: &mut dyn Pictures,
    ) -> Result<(), OdtError> {
        let master = styles
            .child(Ns::Office, "master-styles")
            .and_then(|m| m.child(Ns::Style, "master-page"));
        let Some(master) = master else {
            return Ok(());
        };
        let layout = master.attr(Ns::Style, "page-layout-name").and_then(|name| {
            styles
                .child(Ns::Office, "automatic-styles")
                .into_iter()
                .chain(styles.child(Ns::Office, "styles"))
                .flat_map(Element::elements)
                .find(|e| e.is(Ns::Style, "page-layout") && e.attr(Ns::Style, "name") == Some(name))
        });
        document.page = layout
            .and_then(|l| l.child(Ns::Style, "page-layout-properties"))
            .and_then(page_setup);
        for band in master.elements() {
            let shown = band.attr(Ns::Style, "display") != Some("false");
            match band.name() {
                "header" | "footer" if shown => {
                    let blocks = self.blocks(band, &mut Scope::default(), pictures)?;
                    if band.name() == "header" {
                        document.header = Some(blocks);
                    } else {
                        document.footer = Some(blocks);
                    }
                }
                "header-left" | "footer-left" | "header-first" | "footer-first" if shown => {
                    self.report
                        .record(WordConstruct::PageBand, Outcome::Dropped);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// The vertical merges a table holds open: by grid column, the rows the
/// merge still covers and the columns it spans.
#[derive(Default)]
struct Grid {
    below: BTreeMap<usize, (u32, u32)>,
}

/// The state of the walk through one paragraph.
struct InlineState {
    format: RunProps,
    /// The link the walk stands in, and its runs so far.
    link: Option<(LinkTarget, Vec<Run>)>,
    /// Whether the last character met was white space: ODF collapses a run
    /// of it into one space, and drops it at the start of a paragraph.
    space: bool,
}

impl InlineState {
    fn collapse(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            if matches!(ch, ' ' | '\t' | '\n' | '\r') {
                if !self.space {
                    out.push(' ');
                    self.space = true;
                }
            } else {
                out.push(ch);
                self.space = false;
            }
        }
        out
    }
}

/// The levels of one list style, the first at 0.
fn list_levels(style: &Element) -> Vec<ListLevel> {
    let mut levels: Vec<ListLevel> = Vec::new();
    for level in style.elements() {
        let Some(at) = level
            .attr(Ns::Text, "level")
            .and_then(|l| l.parse::<usize>().ok())
            .filter(|l| (1..=10).contains(l))
        else {
            continue;
        };
        let defined = match level.name() {
            "list-level-style-bullet" | "list-level-style-image" => ListLevel {
                kind: Some(ListKind::Bullet),
                exact: true,
            },
            "list-level-style-number" => number_level(level.attr(Ns::Style, "num-format")),
            _ => continue,
        };
        if levels.len() < at {
            levels.resize(
                at,
                ListLevel {
                    kind: Some(ListKind::Bullet),
                    exact: true,
                },
            );
        }
        if let Some(slot) = levels.get_mut(at - 1) {
            *slot = defined;
        }
    }
    levels
}

fn number_level(format: Option<&str>) -> ListLevel {
    let exact = |format| ListLevel {
        kind: Some(ListKind::Numbered(format)),
        exact: true,
    };
    match format.unwrap_or_default() {
        "1" => exact(NumberFormat::Decimal),
        "a" => exact(NumberFormat::LowerLetter),
        "A" => exact(NumberFormat::UpperLetter),
        "i" => exact(NumberFormat::LowerRoman),
        "I" => exact(NumberFormat::UpperRoman),
        "" => ListLevel {
            kind: None,
            exact: true,
        },
        _ => ListLevel {
            kind: Some(ListKind::Numbered(NumberFormat::Decimal)),
            exact: false,
        },
    }
}

/// Whether a text element of this name is a computed field: its content is
/// the result it last showed.
fn is_field(name: &str) -> bool {
    matches!(
        name,
        "date"
            | "time"
            | "page-number"
            | "page-count"
            | "page-continuation"
            | "sender-firstname"
            | "sender-lastname"
            | "sender-initials"
            | "sender-title"
            | "sender-position"
            | "sender-email"
            | "sender-phone-private"
            | "sender-fax"
            | "sender-company"
            | "sender-phone-work"
            | "sender-street"
            | "sender-city"
            | "sender-postal-code"
            | "sender-country"
            | "sender-state-or-province"
            | "author-name"
            | "author-initials"
            | "chapter"
            | "file-name"
            | "template-name"
            | "sheet-name"
            | "variable-set"
            | "variable-get"
            | "variable-input"
            | "user-field-get"
            | "user-field-input"
            | "sequence"
            | "expression"
            | "text-input"
            | "initial-creator"
            | "creation-date"
            | "creation-time"
            | "description"
            | "user-defined"
            | "print-time"
            | "print-date"
            | "printed-by"
            | "title"
            | "subject"
            | "keywords"
            | "editing-cycles"
            | "editing-duration"
            | "modification-time"
            | "modification-date"
            | "creator"
            | "page-count-range"
            | "word-count"
            | "character-count"
            | "paragraph-count"
            | "table-count"
            | "image-count"
            | "object-count"
            | "bookmark-ref"
            | "reference-ref"
            | "sequence-ref"
            | "note-ref"
            | "script"
            | "execute-macro"
            | "dde-connection"
            | "measure"
            | "placeholder"
            | "conditional-text"
            | "page-variable-set"
            | "page-variable-get"
            | "database-display"
            | "database-next"
            | "database-row-select"
            | "database-row-number"
            | "database-name"
            | "drop-down"
            | "template"
    )
}

/// A display name of ODF with each `_xx_` escape read as its character: a
/// name may spell a space as `_20_`.
fn decode_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(at) = rest.find('_') {
        let (before, tail) = rest.split_at(at);
        out.push_str(before);
        let decoded = tail
            .get(1..3)
            .filter(|_| tail.get(3..4) == Some("_"))
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .map(char::from);
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = tail.get(4..).unwrap_or_default();
            }
            None => {
                out.push('_');
                rest = tail.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

fn alignment(value: &str) -> Option<Alignment> {
    match value {
        "start" | "left" => Some(Alignment::Start),
        "center" => Some(Alignment::Center),
        "end" | "right" => Some(Alignment::End),
        "justify" => Some(Alignment::Justified),
        _ => None,
    }
}

fn break_kind(value: &str) -> Option<BreakKind> {
    match value {
        "page" => Some(BreakKind::Page),
        "column" => Some(BreakKind::Column),
        _ => None,
    }
}

/// Whether a `fo:font-weight` is bold: `bold`, or a weight of 600 and up.
fn is_bold(value: &str) -> bool {
    value == "bold" || value.parse::<u16>().is_ok_and(|weight| weight >= 600)
}

/// The position a `style:text-position` gives: `super`, `sub`, or a signed
/// percentage of the font's height.
fn position(value: &str) -> VerticalPosition {
    let first = value.split_whitespace().next().unwrap_or_default();
    match first {
        "super" => VerticalPosition::Superscript,
        "sub" => VerticalPosition::Subscript,
        other => {
            let number = other.trim_end_matches('%').parse::<f32>().unwrap_or(0.0);
            if number > 0.0 {
                VerticalPosition::Superscript
            } else if number < 0.0 {
                VerticalPosition::Subscript
            } else {
                VerticalPosition::Baseline
            }
        }
    }
}

/// A length of ODF in points: a number and a unit of `pt`, `in`, `cm`,
/// `mm`, `pc` or `px`.
pub(crate) fn points(value: &str) -> Option<PrintLength<Pt>> {
    let value = value.trim();
    let at = value
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(at);
    let number: f32 = number.trim().parse().ok()?;
    let per = match unit {
        "pt" => 1.0,
        "in" => 72.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        "pc" => 12.0,
        "px" => 0.75,
        _ => return None,
    };
    Some(PrintLength::new(number * per)).filter(|length| length.get().is_finite())
}

fn hex_color(value: &str) -> Option<Rgb> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok().map(Rgb::from_bits)
}

/// How many times a repeat or span attribute asks for, at least 1 and at
/// most [`MAX_REPEAT`].
fn repeat(value: Option<&str>) -> u32 {
    value
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, MAX_REPEAT)
}

/// The size, the margins and the orientation of a page layout.
fn page_setup(properties: &Element) -> Option<PageSetup> {
    let length = |name: &str| properties.attr(Ns::Fo, name).and_then(points);
    Some(PageSetup {
        width: length("page-width")?,
        height: length("page-height")?,
        margins: Margins {
            top: length("margin-top").unwrap_or(PrintLength::ZERO),
            bottom: length("margin-bottom").unwrap_or(PrintLength::ZERO),
            left: length("margin-left").unwrap_or(PrintLength::ZERO),
            right: length("margin-right").unwrap_or(PrintLength::ZERO),
        },
        orientation: match properties.attr(Ns::Style, "print-orientation") {
            Some("landscape") => Orientation::Landscape,
            _ => Orientation::Portrait,
        },
    })
}

/// The date of an annotation or a change, `xsd:dateTime`. A date with no
/// zone is a wall time, as LibreOffice writes it. A zone names the instant.
fn odf_date(text: &str) -> Option<WordDate> {
    let text = text.trim();
    let clock = text.split_once('T').map_or("", |(_, clock)| clock);
    let zone_at = clock.find(['Z', '+', '-']);
    let wall_text = match zone_at {
        Some(at) => text.get(..text.len() - clock.len() + at)?,
        None => text,
    };
    let wall: Instant<Authored> = ambre_docx::time::parse(wall_text)?;
    if zone_at.is_none() {
        return Some(WordDate::Floating(wall));
    }
    let utc = ambre_docx::time::parse(text)?;
    Some(WordDate::Zoned { utc, wall })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_escaped_name_reads_as_its_characters() {
        assert_eq!(decode_name("Heading_20_1"), "Heading 1");
        assert_eq!(decode_name("a_b"), "a_b");
        assert_eq!(decode_name("x_zz_"), "x_zz_");
    }

    #[test]
    fn a_length_reads_in_points_whatever_its_unit() {
        let close = |text: &str, pt: f32| {
            let got = points(text).expect("a length").get();
            assert!((got - pt).abs() < 0.01, "{text}: {got}");
        };
        close("1in", 72.0);
        close("2.54cm", 72.0);
        close("12pt", 12.0);
        assert!(points("12furlongs").is_none());
    }

    #[test]
    fn a_date_with_no_zone_is_a_wall_time_and_a_zone_names_the_instant() {
        assert!(matches!(
            odf_date("2024-03-05T10:20:00"),
            Some(WordDate::Floating(_))
        ));
        let Some(WordDate::Zoned { utc, wall }) = odf_date("2024-03-05T10:20:00+01:00") else {
            panic!("a zoned date");
        };
        assert_eq!(wall.get() - utc.get(), 3_600_000);
    }
}
