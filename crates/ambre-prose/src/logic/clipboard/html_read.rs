//! HTML into the blocks a paste writes (`features/ambre-spec-prose-clipboard.md`).
//!
//! The tree builder closes implied tags as a browser does. The walk then keeps
//! what an allowlist names: the block elements, the inline elements that are
//! formatting, and a few style declarations. Everything else is dropped, a
//! script and a remote reference included, so nothing a page carries runs or
//! loads. The walk holds its own stack, so a deep tree never recurses.

use ambre_core::{Rgb, Rgba};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use html5ever::{Attribute, LocalName, ParseOpts, ns, parse_document};

use ambre_bus::app::prose::{CodeLang, HeadingLevel, OrderedStyle};

use super::sink::{Arena, Data, NodeId, Nodes};
use super::{ClipBlock, ClipKind, ClipRun, ClipValue};
use crate::mark::{self, MarkRoot, MarkValue};
use ambre_bus::app::prose::MarkKey;

/// Elements whose whole subtree carries nothing a paste keeps.
const SKIPPED: [&str; 14] = [
    "head", "script", "style", "template", "noscript", "iframe", "object", "embed", "title",
    "select", "textarea", "button", "canvas", "video",
];

/// Elements that close the block before them and open one of their own.
const BLOCKS: [&str; 29] = [
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "pre",
    "blockquote",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "address",
    "dt",
    "dd",
    "table",
    "tr",
    "td",
    "th",
    "caption",
    "center",
];

/// The block elements an empty instance of which still makes a paragraph.
const KEPT_EMPTY: [&str; 11] = [
    "p", "h1", "h2", "h3", "h4", "h5", "h6", "li", "pre", "td", "th",
];

/// Read `html` into blocks. A document with no text reads as no block, and
/// so does one nested deeper than [`DEPTH_MAX`].
pub(super) fn read(html: &str) -> Vec<ClipBlock> {
    if too_deep(html) {
        return Vec::new();
    }
    let nodes = parse_document(Arena::new(), ParseOpts::default()).one(html);
    let mut reader = Reader::default();
    reader.walk(&nodes);
    reader.flush(EmptyBlock::Drop);
    reader.blocks
}

/// The deepest nesting a paste reads.
///
/// The tree builder walks the open elements to check an element's scope. A
/// tree `n` deep thus costs `n` squared: 350 KB of nested lists takes seconds
/// on the Logic thread. A list nested nine levels in Google Docs opens about
/// forty elements, so a cap four times that keeps every real fragment. Chrome
/// caps its own tree at 512, which the budget of a paste cannot afford.
const DEPTH_MAX: usize = 128;

/// The elements the tree builder closes on its own, and the void ones: none
/// of them deepens the tree.
const UNCOUNTED: [&str; 31] = [
    "p", "li", "dt", "dd", "tr", "td", "th", "thead", "tbody", "tfoot", "option", "optgroup", "rb",
    "rt", "rp", "colgroup", "caption", "area", "base", "br", "col", "embed", "hr", "img", "input",
    "link", "meta", "param", "source", "track", "wbr",
];

/// Counts the nesting of `html` in one pass of the tokenizer, which is
/// linear, before the tree builder sees it.
#[derive(Default)]
struct Depth {
    /// The counted elements open, never more than [`DEPTH_MAX`] of them.
    open: RefCell<Vec<LocalName>>,
    exceeded: Cell<bool>,
}

impl TokenSink for Depth {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let Token::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };
        let name = &*tag.name;
        let mut open = self.open.borrow_mut();
        match tag.kind {
            TagKind::StartTag => {
                let raw = match name {
                    "script" => Some(RawKind::ScriptData),
                    "style" | "xmp" | "iframe" | "noembed" | "noframes" => Some(RawKind::Rawtext),
                    "title" | "textarea" => Some(RawKind::Rcdata),
                    _ => None,
                };
                // `/>` closes an element inside SVG and MathML alone: an HTML
                // `<div/>` opens a `div`.
                let foreign = open.iter().any(|n| &**n == "svg" || &**n == "math");
                let closed = tag.self_closing && foreign;
                if raw.is_none() && !closed && !UNCOUNTED.contains(&name) {
                    if open.len() >= DEPTH_MAX {
                        self.exceeded.set(true);
                    } else {
                        open.push(tag.name.clone());
                    }
                }
                if let Some(kind) = raw {
                    return TokenSinkResult::RawData(kind);
                }
            }
            TagKind::EndTag => {
                if let Some(at) = open.iter().rposition(|n| *n == tag.name) {
                    open.truncate(at);
                }
            }
        }
        TokenSinkResult::Continue
    }
}

fn too_deep(html: &str) -> bool {
    let tokenizer = Tokenizer::new(Depth::default(), TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.exceeded.get()
}

enum Step {
    Enter(NodeId),
    Leave,
}

/// Whose style an element carries: the author's for this text, or a page's
/// theme on a block that holds blocks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StyleOrigin {
    Author,
    Theme,
}

/// What a flush does with a block that holds no text.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EmptyBlock {
    Drop,
    Keep,
}

/// Whether the walk is inside preformatted text, where code is the block's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    Normal,
    Pre,
}

/// A block element the walk is inside.
struct Open {
    kind: ClipKind,
    keep_empty: bool,
    /// Whether a block was written inside it, so its own close adds none.
    touched: bool,
}

/// What leaving an element undoes.
#[derive(Default)]
struct Frame {
    restore: Vec<(MarkRoot, Option<MarkValue>)>,
    block: bool,
    list: bool,
    pre: bool,
}

#[derive(Default)]
struct Reader {
    blocks: Vec<ClipBlock>,
    runs: Vec<ClipRun>,
    open: Vec<Open>,
    lists: Vec<ClipKind>,
    marks: BTreeMap<MarkKey, MarkValue>,
    /// `marks` in the shape a run carries, rebuilt when they change.
    current: Vec<(MarkKey, ClipValue)>,
    frames: Vec<Frame>,
    pre: usize,
}

fn attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name.ns == ns!() && &*a.name.local == name)
        .map(|a| &*a.value)
}

/// The declarations of a style attribute, each name lower-cased.
fn declarations(style: &str) -> impl Iterator<Item = (String, &str)> {
    style.split(';').filter_map(|declaration| {
        let (name, value) = declaration.split_once(':')?;
        let value = value.trim();
        let value = value.strip_suffix("!important").unwrap_or(value).trim();
        Some((name.trim().to_ascii_lowercase(), value))
    })
}

fn hidden(style: &str) -> bool {
    declarations(style).any(|(name, value)| {
        let value = value.to_ascii_lowercase();
        (name == "display" && value == "none") || (name == "mso-list" && value == "ignore")
    })
}

impl Reader {
    fn walk(&mut self, nodes: &Nodes) {
        let mut steps = vec![Step::Enter(NodeId::DOCUMENT)];
        while let Some(step) = steps.pop() {
            let id = match step {
                Step::Leave => {
                    self.leave();
                    continue;
                }
                Step::Enter(id) => id,
            };
            let Some(node) = nodes.get(id) else {
                continue;
            };
            match &node.data {
                Data::Text(text) => self.text(text),
                Data::Ignored => {}
                Data::Document => steps.extend(node.children.iter().rev().map(|c| Step::Enter(*c))),
                Data::Element { name, attrs } => {
                    // An SVG or a MathML subtree carries no text a paste keeps.
                    if name.ns != ns!(html) {
                        continue;
                    }
                    let tag = &*name.local;
                    let style = attr(attrs, "style").unwrap_or("");
                    if SKIPPED.contains(&tag) || hidden(style) {
                        continue;
                    }
                    // A block that holds blocks is a page's theme, not formatting an
                    // author chose for this text: its style sets no mark.
                    let themed = BLOCKS.contains(&tag)
                        && node.children.iter().any(|child| {
                            nodes.get(*child).is_some_and(|child| {
                                matches!(&child.data, Data::Element { name, .. }
                                    if BLOCKS.contains(&&*name.local))
                            })
                        });
                    let origin = if themed {
                        StyleOrigin::Theme
                    } else {
                        StyleOrigin::Author
                    };
                    let frame = self.enter(tag, attrs, style, origin);
                    self.frames.push(frame);
                    steps.push(Step::Leave);
                    steps.extend(node.children.iter().rev().map(|c| Step::Enter(*c)));
                }
            }
        }
    }

    fn kind(&self) -> ClipKind {
        self.open
            .last()
            .map_or(ClipKind::Paragraph, |open| open.kind.clone())
    }

    /// The kind a block element opens: its own, or that of the list item or
    /// heading around it. A paragraph inside a list item takes the item's.
    fn block_kind(&self, tag: &str, attrs: &[Attribute], style: &str) -> ClipKind {
        let heading = tag
            .strip_prefix('h')
            .and_then(|level| level.parse::<u8>().ok())
            .and_then(HeadingLevel::new);
        if let Some(level) = heading {
            return ClipKind::Heading(level);
        }
        match tag {
            "li" => {
                if attr(attrs, "role") == Some("checkbox") {
                    return ClipKind::Todo(crate::events::TodoMark::done_if(
                        attr(attrs, "aria-checked") == Some("true"),
                    ));
                }
                self.lists.last().cloned().unwrap_or(ClipKind::Bullet)
            }
            "pre" => ClipKind::Code(language(attrs)),
            _ => {
                // Word writes a list item as a paragraph carrying `mso-list`.
                let word_item = attr(attrs, "class")
                    .is_some_and(|c| c.starts_with("MsoListParagraph"))
                    || declarations(style).any(|(name, _)| name == "mso-list");
                if word_item {
                    return ClipKind::Bullet;
                }
                match self.kind() {
                    kind @ (ClipKind::Bullet | ClipKind::Ordered(_) | ClipKind::Todo(_)) => kind,
                    _ => ClipKind::Paragraph,
                }
            }
        }
    }

    fn enter(&mut self, tag: &str, attrs: &[Attribute], style: &str, origin: StyleOrigin) -> Frame {
        let mut frame = Frame::default();
        match tag {
            "br" => {
                if self.pre > 0 {
                    self.push_str("\n");
                } else {
                    // A break outside any block ends a line, and makes no
                    // empty paragraph of its own.
                    self.flush(if self.open.is_empty() {
                        EmptyBlock::Drop
                    } else {
                        EmptyBlock::Keep
                    });
                }
            }
            "hr" => {
                self.flush(EmptyBlock::Drop);
            }
            "ul" | "ol" => {
                self.flush(EmptyBlock::Drop);
                self.lists.push(if tag == "ol" {
                    ClipKind::Ordered(ordered_style(attr(attrs, "type")))
                } else {
                    ClipKind::Bullet
                });
                frame.list = true;
            }
            "input" => {
                if attr(attrs, "type").is_some_and(|t| t.eq_ignore_ascii_case("checkbox"))
                    && let Some(open) = self.open.last_mut()
                    && matches!(
                        open.kind,
                        ClipKind::Bullet | ClipKind::Ordered(_) | ClipKind::Todo(_)
                    )
                {
                    open.kind = ClipKind::Todo(crate::events::TodoMark::done_if(
                        attr(attrs, "checked").is_some(),
                    ));
                }
            }
            "code" if self.pre > 0 => {
                if let Some(Open {
                    kind: ClipKind::Code(lang),
                    ..
                }) = self.open.last_mut()
                    && lang.is_none()
                {
                    *lang = language(attrs);
                }
            }
            _ if BLOCKS.contains(&tag) => {
                let kind = self.block_kind(tag, attrs, style);
                self.flush(EmptyBlock::Drop);
                self.open.push(Open {
                    kind,
                    keep_empty: KEPT_EMPTY.contains(&tag),
                    touched: false,
                });
                frame.block = true;
                if tag == "pre" {
                    self.pre += 1;
                    frame.pre = true;
                }
            }
            _ => {}
        }
        let flow = if self.pre > 0 {
            Flow::Pre
        } else {
            Flow::Normal
        };
        let mut changes = tag_marks(tag, attrs, flow);
        if origin == StyleOrigin::Author {
            changes.extend(style_marks(style));
        }
        for change in changes {
            let (key, previous) = match change {
                Change::Set(key, value) => (key, self.marks.insert(key.key(), value)),
                Change::Clear(key) => (key, self.marks.remove(key.as_str())),
            };
            frame.restore.push((key, previous));
        }
        if !frame.restore.is_empty() {
            self.rebuild();
        }
        frame
    }

    fn leave(&mut self) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        if !frame.restore.is_empty() {
            for (key, previous) in frame.restore.into_iter().rev() {
                match previous {
                    Some(value) => self.marks.insert(key.key(), value),
                    None => self.marks.remove(key.as_str()),
                };
            }
            self.rebuild();
        }
        if frame.list {
            self.lists.pop();
        }
        if frame.pre {
            self.pre = self.pre.saturating_sub(1);
        }
        if frame.block {
            let keep = if self.open.last().is_some_and(|o| o.keep_empty && !o.touched) {
                EmptyBlock::Keep
            } else {
                EmptyBlock::Drop
            };
            let wrote = self.flush(keep);
            let touched = self.open.pop().is_some_and(|o| o.touched) || wrote;
            if touched && let Some(parent) = self.open.last_mut() {
                parent.touched = true;
            }
        }
    }

    fn rebuild(&mut self) {
        self.current = self
            .marks
            .iter()
            .map(|(key, value)| (key.clone(), ClipValue::Text(value.as_str().to_owned())))
            .collect();
    }

    fn push_str(&mut self, text: &str) {
        match self.runs.last_mut() {
            Some(run) if run.marks == self.current => run.text.push_str(text),
            _ => self.runs.push(ClipRun {
                text: text.to_owned(),
                marks: self.current.clone(),
            }),
        }
    }

    fn ends_blank(&self) -> bool {
        self.runs
            .iter()
            .rev()
            .find_map(|run| run.text.chars().next_back())
            .is_none_or(|last| last == ' ')
    }

    /// Text outside a `pre` collapses its white space as a browser lays it
    /// out; inside one it is kept. Control characters never land.
    fn text(&mut self, text: &str) {
        let mut buffer = String::new();
        let mut blank = self.ends_blank();
        for c in text.chars() {
            if self.pre > 0 {
                if c == '\n' || !c.is_control() {
                    buffer.push(c);
                }
            } else if c.is_ascii_whitespace() {
                if !blank {
                    buffer.push(' ');
                    blank = true;
                }
            } else if !c.is_control() {
                buffer.push(c);
                blank = false;
            }
        }
        if !buffer.is_empty() {
            self.push_str(&buffer);
        }
    }

    /// Close the block being written. A block with no text is kept only when
    /// `empty` says an element asked for it. Answers whether one landed.
    /// Trailing spaces go, the no-break one Word writes into an empty
    /// paragraph included.
    fn flush(&mut self, empty: EmptyBlock) -> bool {
        let code = matches!(self.kind(), ClipKind::Code(_));
        while let Some(run) = self.runs.last_mut() {
            let trimmed = if code {
                run.text.trim_end_matches('\n')
            } else {
                run.text.trim_end_matches([' ', '\u{a0}'])
            };
            run.text.truncate(trimmed.len());
            if !run.text.is_empty() {
                break;
            }
            self.runs.pop();
        }
        if self.runs.is_empty() && empty == EmptyBlock::Drop {
            return false;
        }
        let runs = std::mem::take(&mut self.runs);
        self.blocks.push(ClipBlock {
            kind: self.kind(),
            runs,
        });
        if let Some(open) = self.open.last_mut() {
            open.touched = true;
        }
        true
    }
}

/// The language a code block names through a `language-` or `lang-` class.
fn language(attrs: &[Attribute]) -> Option<CodeLang> {
    attr(attrs, "class")
        .into_iter()
        .flat_map(str::split_ascii_whitespace)
        .find_map(|class| {
            class
                .strip_prefix("language-")
                .or_else(|| class.strip_prefix("lang-"))
        })
        .and_then(|lang| {
            CodeLang::new(
                lang.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '#'))
                    .take(32)
                    .collect::<String>(),
            )
        })
}

fn ordered_style(kind: Option<&str>) -> OrderedStyle {
    match kind {
        Some("a") => OrderedStyle::LowerAlpha,
        Some("A") => OrderedStyle::UpperAlpha,
        Some("i") => OrderedStyle::LowerRoman,
        Some("I") => OrderedStyle::UpperRoman,
        _ => OrderedStyle::Decimal,
    }
}

/// What an element or a declaration does to one mark.
enum Change {
    /// Set the mark to a value the paste keeps.
    Set(MarkRoot, MarkValue),
    /// Clear the mark.
    Clear(MarkRoot),
}

/// The change that sets `key` to `raw`, or `None` for a value the paste
/// does not keep (`PROSE-UI-220`).
fn set(key: MarkRoot, raw: &str) -> Option<Change> {
    super::pasted(key, Some(raw)).map(|value| Change::Set(key, value))
}

/// The change that turns on the mark whose presence is the formatting.
fn on(key: MarkRoot) -> Option<Change> {
    set(key, "")
}

/// The marks an element's name stands for.
fn tag_marks(tag: &str, attrs: &[Attribute], flow: Flow) -> Vec<Change> {
    let change = match tag {
        "b" | "strong" => on(MarkRoot::Bold),
        "i" | "em" | "cite" | "dfn" | "var" => on(MarkRoot::Italic),
        "u" | "ins" => on(MarkRoot::Underline),
        "s" | "strike" | "del" => on(MarkRoot::Strike),
        "sup" => on(MarkRoot::Superscript),
        "sub" => on(MarkRoot::Subscript),
        "code" | "kbd" | "samp" | "tt" if flow == Flow::Normal => on(MarkRoot::Code),
        "a" => attr(attrs, "href")
            .map(str::trim)
            .and_then(|href| set(MarkRoot::Link, href)),
        _ => None,
    };
    change.into_iter().collect()
}

/// The marks a style attribute's declarations stand for. Black ink and a
/// white or transparent background are the page's defaults. The author did
/// not choose them, so they clear the mark rather than set it.
fn style_marks(style: &str) -> Vec<Change> {
    let mut changes = Vec::new();
    for (name, value) in declarations(style) {
        let lower = value.to_ascii_lowercase();
        match name.as_str() {
            "font-weight" => {
                let bold = match lower.as_str() {
                    "bold" | "bolder" => Some(true),
                    "normal" | "lighter" => Some(false),
                    weight => weight.parse::<u16>().ok().map(|w| w >= 600),
                };
                if let Some(bold) = bold {
                    changes.extend(if bold {
                        on(MarkRoot::Bold)
                    } else {
                        Some(Change::Clear(MarkRoot::Bold))
                    });
                }
            }
            "font-style" => match lower.as_str() {
                "italic" | "oblique" => changes.extend(on(MarkRoot::Italic)),
                "normal" => changes.push(Change::Clear(MarkRoot::Italic)),
                _ => {}
            },
            "text-decoration" | "text-decoration-line" => {
                if lower.contains("none") {
                    changes.extend([
                        Change::Clear(MarkRoot::Underline),
                        Change::Clear(MarkRoot::Strike),
                    ]);
                }
                if lower.contains("underline") {
                    changes.extend(on(MarkRoot::Underline));
                }
                if lower.contains("line-through") {
                    changes.extend(on(MarkRoot::Strike));
                }
            }
            "vertical-align" => match lower.as_str() {
                "super" => changes.extend(on(MarkRoot::Superscript)),
                "sub" => changes.extend(on(MarkRoot::Subscript)),
                "baseline" => changes.extend([
                    Change::Clear(MarkRoot::Superscript),
                    Change::Clear(MarkRoot::Subscript),
                ]),
                _ => {}
            },
            "color" => {
                if let Some(color) = css_color(&lower) {
                    changes.extend(if ink(color) {
                        Some(Change::Clear(MarkRoot::Ink))
                    } else {
                        set(MarkRoot::Ink, &mark::canonical_color(color))
                    });
                }
            }
            "background-color" | "background" => {
                if lower == "transparent" {
                    changes.push(Change::Clear(MarkRoot::Highlight));
                } else if let Some(color) = css_color(&lower) {
                    let white = color == Rgba::from_bits(0xffff_ffff);
                    changes.extend(if white {
                        Some(Change::Clear(MarkRoot::Highlight))
                    } else {
                        set(MarkRoot::Highlight, &mark::canonical_color(color))
                    });
                }
            }
            "font-size" => {
                if let Some(points) =
                    font_size(&lower).and_then(|size| mark::canonical_number(size.points()))
                {
                    changes.extend(set(MarkRoot::FontSize, &points));
                }
            }
            "font-family" => {
                if let Some(family) = font_family(value) {
                    changes.extend(set(MarkRoot::FontFamily, &family));
                }
            }
            _ => {}
        }
    }
    changes
}

/// Whether a colour reads as the page's body ink: black, or a grey too dark
/// to tell from it. Sites set `#202122` or `#333` as their text colour.
fn ink(color: Rgba) -> bool {
    let [r, g, b, _] = color.channels();
    let (low, high) = (r.min(g).min(b), r.max(g).max(b));
    high <= 0x40 && high - low <= 0x10
}

/// A size in points, from points or pixels. A relative size depends on a
/// page the paste does not carry.
fn font_size(value: &str) -> Option<ambre_core::FontSize> {
    let points = if let Some(pt) = value.strip_suffix("pt") {
        pt.trim().parse::<f32>().ok()?
    } else {
        ambre_core::DocPx::new(value.strip_suffix("px")?.trim().parse::<f32>().ok()?)
            .to_pt()
            .get()
    };
    ambre_core::FontSize::new((points * 2.0).round() / 2.0)
}

/// The first family a font stack names, when it names one rather than a
/// generic family or a platform alias.
fn font_family(value: &str) -> Option<String> {
    let first = value
        .split(',')
        .next()?
        .trim()
        .trim_matches(['"', '\''])
        .trim();
    let generic = [
        "serif",
        "sans-serif",
        "monospace",
        "cursive",
        "fantasy",
        "system-ui",
        "inherit",
        "initial",
        "unset",
    ];
    let usable = !first.starts_with('-') && !generic.contains(&first.to_ascii_lowercase().as_str());
    usable.then(|| first.to_owned())
}

/// A colour read from hexadecimal, `rgb()`, `rgba()`, or one of the sixteen
/// basic names.
fn css_color(value: &str) -> Option<Rgba> {
    if let Some(hex) = value.strip_prefix('#') {
        let long = match hex.len() {
            3 => hex.chars().flat_map(|c| [c, c]).collect(),
            6 | 8 => hex.to_owned(),
            _ => return None,
        };
        return mark::parse_color(&format!("#{long}")).ok();
    }
    if let Some(args) = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<&str> = args
            .split([',', ' ', '/'])
            .filter(|part| !part.is_empty())
            .collect();
        let channel = |part: &str| -> Option<u8> {
            let value = match part.strip_suffix('%') {
                Some(percent) => percent.parse::<f32>().ok()? * 2.55,
                None => part.parse::<f32>().ok()?,
            };
            Some(value.round().clamp(0.0, 255.0) as u8)
        };
        let alpha = |part: &str| -> Option<u8> {
            let value = match part.strip_suffix('%') {
                Some(percent) => percent.parse::<f32>().ok()? / 100.0,
                None => part.parse::<f32>().ok()?,
            };
            Some((value.clamp(0.0, 1.0) * 255.0).round() as u8)
        };
        let [r, g, b] = [parts.first()?, parts.get(1)?, parts.get(2)?].map(|p| channel(p));
        let a = parts.get(3).map_or(Some(255), |p| alpha(p))?;
        let rgba = Rgba::from_channels(r?, g?, b?, a);
        if a == 0 {
            return None;
        }
        return Some(rgba);
    }
    let named = match value {
        "black" => 0x00_0000,
        "white" => 0xff_ffff,
        "red" => 0xff_0000,
        "green" => 0x00_8000,
        "blue" => 0x00_00ff,
        "yellow" => 0xff_ff00,
        "gray" | "grey" => 0x80_8080,
        "silver" => 0xc0_c0c0,
        "maroon" => 0x80_0000,
        "olive" => 0x80_8000,
        "lime" => 0x00_ff00,
        "aqua" | "cyan" => 0x00_ffff,
        "teal" => 0x00_8080,
        "navy" => 0x00_0080,
        "fuchsia" | "magenta" => 0xff_00ff,
        "purple" => 0x80_0080,
        "orange" => 0xff_a500,
        _ => return None,
    };
    Some(Rgb::from_bits(named).to_rgba(ambre_core::Alpha::OPAQUE))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    fn texts(blocks: &[ClipBlock]) -> Vec<String> {
        blocks
            .iter()
            .map(|block| block.runs.iter().map(|run| run.text.as_str()).collect())
            .collect()
    }

    fn keys(run: &ClipRun) -> Vec<&str> {
        run.marks.iter().map(|(key, _)| key.as_str()).collect()
    }

    /// `PROSE-UI-219`: Google Docs wraps its fragment in a bold element that
    /// its own style cancels, and marks each span by style.
    #[test]
    fn a_google_docs_fragment_keeps_its_blocks_and_marks() {
        let html = concat!(
            "<meta charset=\"utf-8\"><b style=\"font-weight:normal;\" id=\"docs-internal-guid-1\">",
            "<h1 dir=\"ltr\"><span style=\"font-size:20pt;font-weight:400\">Title</span></h1>",
            "<p dir=\"ltr\"><span style=\"font-weight:700;color:#000000\">Bold</span>",
            "<span style=\"font-weight:400;font-style:italic\"> plain</span></p>",
            "<ul><li dir=\"ltr\"><p dir=\"ltr\"><span>item</span></p></li></ul></b><br>",
        );
        let blocks = read(html);
        assert_eq!(texts(&blocks), ["Title", "Bold plain", "item"]);
        assert_eq!(blocks[0].kind, ClipKind::Heading(HeadingLevel::H1));
        assert_eq!(keys(&blocks[0].runs[0]), ["fs"]);
        assert_eq!(keys(&blocks[1].runs[0]), ["b"], "black ink is no colour");
        assert_eq!(keys(&blocks[1].runs[1]), ["i"]);
        assert_eq!(blocks[2].kind, ClipKind::Bullet);
    }

    /// `PROSE-UI-219`: Word writes a list item as a paragraph, and its bullet
    /// in a span it hides from other readers. An empty paragraph is a
    /// no-break space.
    #[test]
    fn a_word_fragment_keeps_its_list_and_drops_its_bullet_glyph() {
        let html = concat!(
            "<p class=MsoListParagraphCxSpFirst style='mso-list:l0 level1 lfo1'>",
            "<![if !supportLists]><span style='mso-list:Ignore'>&#183;",
            "<span style='font:7.0pt \"Times New Roman\"'>&nbsp;&nbsp; </span></span>",
            "<![endif]>One<o:p></o:p></p>",
            "<p class=MsoNormal><o:p>&nbsp;</o:p></p>",
            "<p class=MsoNormal><i>two</i><o:p></o:p></p>",
        );
        let blocks = read(html);
        assert_eq!(texts(&blocks), ["One", "", "two"]);
        assert_eq!(blocks[0].kind, ClipKind::Bullet);
        assert_eq!(blocks[2].kind, ClipKind::Paragraph);
        assert_eq!(keys(&blocks[2].runs[0]), ["i"]);
    }

    /// `PROSE-UI-220`: a script, a style sheet, a handler, a picture and a
    /// link that runs code land as nothing; the text around them stays.
    #[test]
    fn hostile_markup_leaves_only_its_text() {
        let html = concat!(
            "<script>alert(1)</script><style>p{color:red}</style>",
            "<p onclick=\"alert(2)\">safe <a href=\"javascript:alert(3)\">link</a>",
            "<img src=\"http://example.com/x.png\" onerror=\"alert(4)\">",
            "<a href=\"https://example.com\">kept</a></p>",
            "<template><p>hidden</p></template><div style=\"display:none\">gone</div>",
            "<svg><text>drawn</text></svg>",
        );
        let blocks = read(html);
        assert_eq!(texts(&blocks), ["safe linkkept"]);
        let marks: Vec<Vec<&str>> = blocks[0].runs.iter().map(keys).collect();
        assert_eq!(marks, [vec![], vec!["a"]]);
    }

    /// `PROSE-UI-221`: a tree nested past the cap reads as no block, however
    /// the nesting is spelled, and one under it reads whole.
    #[test]
    fn a_tree_nested_past_the_cap_reads_as_nothing() {
        let deep = |open: &str, close: &str, depth: usize| {
            format!("{}deep{}", open.repeat(depth), close.repeat(depth))
        };
        for html in [
            deep("<div>", "</div>", 50_000),
            deep("<ul><li>", "</li></ul>", 50_000),
            deep("<div></span>", "", 50_000),
            deep("<div/>", "", 50_000),
        ] {
            assert!(read(&html).is_empty(), "{}", &html[..24]);
        }
        let shallow = deep("<div><span>", "</span></div>", DEPTH_MAX / 2 - 1);
        assert_eq!(texts(&read(&shallow)), ["deep"]);
        let svg = format!("<svg>{}</svg><p>after</p>", "<path/>".repeat(50_000));
        assert_eq!(texts(&read(&svg)), ["after"]);
    }

    /// `PROSE-UI-219`: white space collapses as a browser lays it out, and a
    /// `pre` keeps its lines in one code block.
    #[test]
    fn white_space_collapses_outside_a_pre_and_stays_inside_one() {
        let html = concat!(
            "<p>  one\n   two  </p>",
            "<pre class=\"language-rust\">fn a() {}\n    b();\n</pre>",
            "<ol type=\"i\"><li>x</li></ol>",
        );
        let blocks = read(html);
        assert_eq!(texts(&blocks), ["one two", "fn a() {}\n    b();", "x"]);
        assert_eq!(blocks[1].kind, ClipKind::Code(CodeLang::new("rust")));
        assert_eq!(blocks[2].kind, ClipKind::Ordered(OrderedStyle::LowerRoman));
    }

    /// `PROSE-UI-219`: a colour spelled in any form lands in the one spelling
    /// the document stores. A mark value the registry refuses is dropped.
    #[test]
    fn colours_and_sizes_land_canonical() {
        let html = concat!(
            "<p><span style=\"color:rgb(255, 0, 0);background-color:#0F0\">a</span>",
            "<span style=\"font-size:16px;font-family:'Georgia', serif\">b</span>",
            "<span style=\"font-size:9000pt;color:bogus\">c</span></p>",
        );
        let blocks = read(html);
        let run = |i: usize| &blocks[0].runs[i].marks;
        let text = |v: &str| ClipValue::Text(v.to_owned());
        assert_eq!(
            run(0),
            &vec![
                (MarkKey::from("bg"), text("#00ff00")),
                (MarkKey::from("fg"), text("#ff0000"))
            ]
        );
        assert_eq!(
            run(1),
            &vec![
                (MarkKey::from("ff"), text("Georgia")),
                (MarkKey::from("fs"), text("12"))
            ]
        );
        assert!(run(2).is_empty());
    }

    fn kinds(blocks: &[ClipBlock]) -> Vec<ClipKind> {
        blocks.iter().map(|block| block.kind.clone()).collect()
    }

    /// The runs of one block, each with its mark keys.
    fn marked(block: &ClipBlock) -> Vec<(&str, Vec<&str>)> {
        block
            .runs
            .iter()
            .map(|run| (run.text.as_str(), keys(run)))
            .collect()
    }

    /// `PROSE-UI-219`: a whole Word clipboard document, head, conditional
    /// comments and fragment markers included, reads as its fragment.
    #[test]
    fn a_word_clipboard_document_reads_as_its_fragment() {
        let blocks = read(include_str!("fixtures/word.html"));
        assert_eq!(
            texts(&blocks),
            [
                "Rapport annuel",
                "Le chiffre d\u{2019}affaires progresse de 12\u{a0}%, et la marge recule l\u{e9}g\u{e8}rement.",
                "Ventes",
                "Achats",
                "",
                "Voir le d\u{e9}tail.",
            ]
        );
        assert_eq!(
            kinds(&blocks),
            [
                ClipKind::Heading(HeadingLevel::H1),
                ClipKind::Paragraph,
                ClipKind::Bullet,
                ClipKind::Bullet,
                ClipKind::Paragraph,
                ClipKind::Paragraph,
            ]
        );
        let body: Vec<Vec<&str>> = blocks[1].runs.iter().map(keys).collect();
        assert_eq!(
            body,
            [
                vec![],
                vec!["b"],
                vec![],
                vec!["fg"],
                vec![],
                vec!["i"],
                vec![]
            ]
        );
        assert_eq!(keys(&blocks[3].runs[0]), ["u"]);
        assert_eq!(keys(&blocks[5].runs[1]), ["a"]);
    }

    /// `PROSE-UI-219`: a Google Docs fragment keeps its ordered list, its
    /// highlight and its link, and its trailing break adds no block.
    #[test]
    fn a_google_docs_clipboard_fragment_keeps_list_highlight_and_link() {
        let blocks = read(include_str!("fixtures/google-docs.html"));
        assert_eq!(
            texts(&blocks),
            [
                "Plan de la semaine",
                "Important : relire le contrat avant jeudi",
                "Appeler le client",
                "Envoyer le devis",
                "x = 1",
            ]
        );
        assert_eq!(blocks[0].kind, ClipKind::Heading(HeadingLevel::H2));
        assert_eq!(blocks[2].kind, blocks[3].kind);
        assert!(matches!(blocks[2].kind, ClipKind::Ordered(_)));
        let body = marked(&blocks[1]);
        assert_eq!(body[0], ("Important", vec!["b", "ff", "fs"]));
        assert_eq!(body[1].1, ["bg", "ff", "fs"]);
        assert_eq!(body[3].1, ["a", "ff", "fg", "fs", "u"]);
        assert_eq!(keys(&blocks[2].runs[0]), ["ff", "fs", "i"]);
        assert_eq!(keys(&blocks[3].runs[0]), ["ff", "fs", "s"]);
    }

    /// `PROSE-UI-224`: a web page's theme, the style its container carries,
    /// sets no mark, and its body ink is no colour. Inline formatting, links,
    /// lists and preformatted code stay; a picture, a script-only fallback and a
    /// button leave nothing.
    #[test]
    fn a_web_page_keeps_its_formatting_and_drops_its_theme() {
        let blocks = read(include_str!("fixtures/web-page.html"));
        assert_eq!(
            texts(&blocks)[..4],
            [
                "Histoire",
                "Le langage Rust est n\u{e9} chez Mozilla en 2006[1]. Il vise la s\u{fb}ret\u{e9} m\u{e9}moire sans ramasse-miettes.",
                "Le logo",
                "cargo, le gestionnaire de paquets\u{a0};",
            ]
        );
        assert_eq!(blocks[0].kind, ClipKind::Heading(HeadingLevel::H2));
        assert_eq!(keys(&blocks[0].runs[0]), ["ff"]);
        let body = marked(&blocks[1]);
        assert_eq!(body[0], ("Le langage ", vec![]));
        assert_eq!(body[1], ("Rust", vec!["b"]));
        assert_eq!(body[3], ("Mozilla", vec!["a", "fg"]));
        assert_eq!(body[5], ("[1]", vec!["sup"]));
        assert_eq!(keys(&blocks[3].runs[0]), ["bg", "c"]);
        assert_eq!(keys(&blocks[4].runs[0]), ["a"]);
        let last = blocks
            .last()
            .map(|block| (block.kind.clone(), texts(std::slice::from_ref(block))));
        assert_eq!(
            last,
            Some((
                ClipKind::Code(None),
                vec!["fn main() {\n    println!(\"<ok>\");\n}".to_owned()]
            ))
        );
        assert!(
            !texts(&blocks)
                .iter()
                .any(|t| t.contains("Partager") || t.contains("modifier"))
        );
    }

    /// `PROSE-UI-219`: a PDF viewer's text layer reads as one paragraph per
    /// line, as its plain text does, and its computed sizes set no mark.
    #[test]
    fn a_pdf_text_layer_reads_one_paragraph_per_line() {
        let blocks = read(include_str!("fixtures/pdfjs.html"));
        assert_eq!(
            texts(&blocks),
            [
                "Les r\u{e9}sultats du trimestre montrent une",
                "hausse nette des ventes.",
                "Conclusion",
            ]
        );
        assert!(
            blocks
                .iter()
                .all(|block| block.runs.iter().all(|run| run.marks.is_empty()))
        );
    }
}
