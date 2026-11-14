//! The trees the markup module answers: the events of a Markdown text and the
//! tree of an HTML document (`ambre-spec-architecture.md section 2`, the
//! converter modules).
//!
//! Both are format-level. They carry what the parser read, in its own terms,
//! and no construct of an application. Prose maps them onto its intermediate
//! form and its paste blocks in the Main instance. Both are flat: a Markdown
//! text is a list of events and an HTML tree is an arena. So a hostile file
//! never makes postcard recurse on the Logic thread.

use ambre_core::index::{Axis, IdxVec, Pos};
use serde::{Deserialize, Serialize};

/// One event of a Markdown text, as `pulldown-cmark` emits it with GFM tables,
/// strikethrough, task lists and footnotes on. The text an event carries is
/// content, so `Debug` prints its length alone.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MdEvent {
    /// A container or a span opens.
    Start(MdTag),
    /// The container or span opened last closes.
    End(MdTagEnd),
    /// Text.
    Text(String),
    /// A code span.
    Code(String),
    /// A line break inside a paragraph that renders as a space.
    SoftBreak,
    /// A line break inside a paragraph that renders as a break.
    HardBreak,
    /// A thematic break.
    Rule,
    /// The box of a task list item, checked or not.
    TaskListMarker(bool),
    /// A reference to a footnote.
    FootnoteReference,
    /// Raw HTML, inline or as a block line. Its text is never read.
    Html,
    /// A math span or block, with its source text.
    Math(String),
}

impl std::fmt::Debug for MdEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ambre_core::Redacted;
        match self {
            Self::Start(tag) => f.debug_tuple("Start").field(tag).finish(),
            Self::End(tag) => f.debug_tuple("End").field(tag).finish(),
            Self::Text(text) => f.debug_tuple("Text").field(&Redacted::new(text)).finish(),
            Self::Code(text) => f.debug_tuple("Code").field(&Redacted::new(text)).finish(),
            Self::SoftBreak => f.write_str("SoftBreak"),
            Self::HardBreak => f.write_str("HardBreak"),
            Self::Rule => f.write_str("Rule"),
            Self::TaskListMarker(done) => f.debug_tuple("TaskListMarker").field(done).finish(),
            Self::FootnoteReference => f.write_str("FootnoteReference"),
            Self::Html => f.write_str("Html"),
            Self::Math(text) => f.debug_tuple("Math").field(&Redacted::new(text)).finish(),
        }
    }
}

/// A container or a span that opens. A link carries its destination, which
/// is content, so `Debug` prints its length alone.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MdTag {
    Paragraph,
    Heading(MdHeading),
    CodeBlock(MdCodeBlock),
    List(MdList),
    Item,
    Table,
    TableHead,
    TableRow,
    TableCell,
    BlockQuote,
    FootnoteDefinition,
    HtmlBlock,
    Emphasis,
    Strong,
    Strikethrough,
    Link(String),
    Image,
    /// A container this vocabulary does not name: a definition list, a
    /// metadata block, a superscript.
    Other,
}

impl std::fmt::Debug for MdTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Paragraph => f.write_str("Paragraph"),
            Self::Heading(level) => f.debug_tuple("Heading").field(level).finish(),
            Self::CodeBlock(kind) => f.debug_tuple("CodeBlock").field(kind).finish(),
            Self::List(kind) => f.debug_tuple("List").field(kind).finish(),
            Self::Item => f.write_str("Item"),
            Self::Table => f.write_str("Table"),
            Self::TableHead => f.write_str("TableHead"),
            Self::TableRow => f.write_str("TableRow"),
            Self::TableCell => f.write_str("TableCell"),
            Self::BlockQuote => f.write_str("BlockQuote"),
            Self::FootnoteDefinition => f.write_str("FootnoteDefinition"),
            Self::HtmlBlock => f.write_str("HtmlBlock"),
            Self::Emphasis => f.write_str("Emphasis"),
            Self::Strong => f.write_str("Strong"),
            Self::Strikethrough => f.write_str("Strikethrough"),
            Self::Link(dest) => f
                .debug_tuple("Link")
                .field(&ambre_core::Redacted::new(dest))
                .finish(),
            Self::Image => f.write_str("Image"),
            Self::Other => f.write_str("Other"),
        }
    }
}

/// The container or span that closes, as [`MdTag`] names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MdTagEnd {
    Paragraph,
    Heading,
    CodeBlock,
    List,
    Item,
    Table,
    TableHead,
    TableRow,
    TableCell,
    BlockQuote,
    FootnoteDefinition,
    HtmlBlock,
    Emphasis,
    Strong,
    Strikethrough,
    Link,
    Image,
    Other,
}

/// The level of a Markdown heading, one to six `#`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MdHeading {
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
}

/// How a code block was written. The info string of a fence is content.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MdCodeBlock {
    /// Indented by four spaces: no info string.
    Indented,
    /// Fenced, with the info string after the fence.
    Fenced(String),
}

impl std::fmt::Debug for MdCodeBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Indented => f.write_str("Indented"),
            Self::Fenced(info) => f
                .debug_tuple("Fenced")
                .field(&ambre_core::Redacted::new(info))
                .finish(),
        }
    }
}

/// Whether a list numbers its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MdList {
    Bullet,
    Ordered,
}

/// The places of an HTML tree: a [`Pos<HtmlNode>`] names a node. Only the
/// markup module hands one out.
#[derive(PartialEq, Eq)]
pub enum HtmlNode {}

impl Axis for HtmlNode {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("HtmlNode");
}

/// The tree an HTML document builds, as a WHATWG tree builder closes it, held
/// in a flat arena. The document is node 0. A node names its children by
/// position, so a walk holds its own stack and never recurses.
///
/// The tree keeps the HTML elements alone. An SVG or a MathML element reads
/// as [`HtmlData::Ignored`], with no child: it carries no text a paste keeps.
/// A document nested past the module's depth bound reads as the document
/// alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlTree {
    nodes: IdxVec<HtmlNode, HtmlNodeData>,
}

impl HtmlTree {
    /// The document node, the root every other node hangs from.
    pub const DOCUMENT: Pos<HtmlNode> = Pos::ZERO;

    /// A tree of the document node alone.
    #[must_use]
    pub fn empty() -> Self {
        Self::from_nodes(IdxVec::from(vec![HtmlNodeData::leaf(HtmlData::Document)]))
    }

    /// The tree `nodes` hold, the document at position 0.
    #[must_use]
    pub const fn from_nodes(nodes: IdxVec<HtmlNode, HtmlNodeData>) -> Self {
        Self { nodes }
    }

    /// The node at `pos`, or `None` past the end of the arena.
    #[must_use]
    pub fn get(&self, pos: Pos<HtmlNode>) -> Option<&HtmlNodeData> {
        self.nodes.get(pos)
    }
}

/// One node of an HTML tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlNodeData {
    pub children: Vec<Pos<HtmlNode>>,
    pub data: HtmlData,
}

impl HtmlNodeData {
    /// A node with no child yet, holding `data`.
    #[must_use]
    pub const fn leaf(data: HtmlData) -> Self {
        Self {
            children: Vec::new(),
            data,
        }
    }
}

/// What a node of the tree holds. A comment, a doctype, a processing
/// instruction and a foreign element carry nothing a reader keeps. Text and
/// attribute values are content, so `Debug` prints their length alone.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HtmlData {
    Document,
    /// An element of the HTML namespace, its local name in lower case.
    Element {
        name: String,
        attrs: Vec<HtmlAttr>,
    },
    Text(String),
    Ignored,
}

impl std::fmt::Debug for HtmlData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Document => f.write_str("Document"),
            Self::Element { name, attrs } => f
                .debug_struct("Element")
                .field("name", name)
                .field("attrs", attrs)
                .finish(),
            Self::Text(text) => f
                .debug_tuple("Text")
                .field(&ambre_core::Redacted::new(text))
                .finish(),
            Self::Ignored => f.write_str("Ignored"),
        }
    }
}

/// An attribute in no namespace, as the tree builder read it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlAttr {
    pub name: String,
    pub value: String,
}

impl std::fmt::Debug for HtmlAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HtmlAttr")
            .field("name", &self.name)
            .field("value", &ambre_core::Redacted::new(&self.value))
            .finish()
    }
}

/// The value of the attribute `name` in `attrs`, if one is set.
#[must_use]
pub fn attr<'a>(attrs: &'a [HtmlAttr], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.value.as_str())
}
