//! HTML into the blocks a paste writes (`features/ambre-spec-prose-clipboard.md`).
//!
//! It runs in the markup module, beside the tree builder, so the Main
//! instance links neither the walk nor the style parser
//! (`ambre-spec-budgets.md section 5`). It answers the blocks Prose's own copy
//! writes, and Prose checks each mark of them again on the paste. A value
//! this walk keeps already passes that check, so the two never disagree.
//!
//! The tree builder closes implied tags as a browser does. The walk then keeps
//! what an allowlist names: the block elements, the inline elements that are
//! formatting, and a few style declarations. Everything else is dropped, a
//! script and a remote reference included, so nothing a page carries runs or
//! loads. The walk holds its own stack, so a deep tree never recurses.

use ambre_core::Validated;
use ambre_core::index::Pos;
use ambre_core::{FontSize, Rgb, Rgba};
use std::collections::BTreeMap;

use crate::markup::{HtmlAttr, HtmlData, HtmlNode, HtmlTree, attr};

use ambre_prose_form::clip::{ClipBlock, ClipKind, ClipRun, ClipValue, safe_family, safe_link};
use ambre_prose_form::{CodeLang, HeadingLevel, MarkKey, OrderedStyle, TodoMark};

/// The marks a paste sets, each spelled as the key Prose's registry declares
/// it (`ambre-spec-prose-schema.md section 3`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum MarkRoot {
    Bold,
    Italic,
    Underline,
    Strike,
    Superscript,
    Subscript,
    Code,
    Link,
    Ink,
    Highlight,
    FontSize,
    FontFamily,
}

impl MarkRoot {
    /// The key the document stores the mark under.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Bold => "b",
            Self::Italic => "i",
            Self::Underline => "u",
            Self::Strike => "s",
            Self::Superscript => "sup",
            Self::Subscript => "sub",
            Self::Code => "c",
            Self::Link => "a",
            Self::Ink => "fg",
            Self::Highlight => "bg",
            Self::FontSize => "fs",
            Self::FontFamily => "ff",
        }
    }

    fn key(self) -> MarkKey {
        MarkKey::from(self.as_str())
    }
}

/// The value of one mark, in the spelling the document stores.
type MarkValue = String;

/// The largest font size a Prose document carries: 1638 points, the cap of
/// the word-processing formats (`CANVAS-088`).
const FONT_SIZE_MAX: FontSize = FontSize::from_whole_points(1638);

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

/// Read the tree of an HTML document into blocks. The markup module builds
/// the tree (`ambre_convert::markup::HtmlTree`). A document with no text reads
/// as no block, and so does one the module found nested past its bound.
pub fn read(tree: &HtmlTree) -> Vec<ClipBlock> {
    let mut reader = Reader::default();
    reader.walk(tree);
    reader.flush(EmptyBlock::Drop);
    reader.blocks
}

enum Step {
    Enter(Pos<HtmlNode>),
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
    /// What its close does when it holds no text.
    empty: EmptyBlock,
    /// Whether a block was written inside it, so its own close adds none.
    touched: Touched,
}

/// Whether a block was written inside an open element.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Touched {
    Untouched,
    Written,
}

/// What an element opened that its close undoes, beside its marks.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Opened {
    /// Nothing but marks.
    #[default]
    Inline,
    /// A list.
    List,
    /// A block.
    Block,
    /// A preformatted block.
    Pre,
}

/// What leaving an element undoes.
#[derive(Default)]
struct Frame {
    restore: Vec<(MarkRoot, Option<MarkValue>)>,
    opened: Opened,
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

/// The style properties a paste reads. Any other is kept as `Other` and
/// read by nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CssProperty {
    Display,
    MsoList,
    FontWeight,
    FontStyle,
    /// `text-decoration` or `text-decoration-line`.
    TextDecoration,
    VerticalAlign,
    Color,
    /// `background-color` or `background`.
    Background,
    FontSize,
    FontFamily,
    Other,
}

impl CssProperty {
    /// The property a declaration names, its case ignored.
    fn of(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "display" => Self::Display,
            "mso-list" => Self::MsoList,
            "font-weight" => Self::FontWeight,
            "font-style" => Self::FontStyle,
            "text-decoration" | "text-decoration-line" => Self::TextDecoration,
            "vertical-align" => Self::VerticalAlign,
            "color" => Self::Color,
            "background-color" | "background" => Self::Background,
            "font-size" => Self::FontSize,
            "font-family" => Self::FontFamily,
            _ => Self::Other,
        }
    }
}

/// The declarations of an element's style attribute, parsed once: each
/// property with its value, `!important` and the spaces around it dropped.
struct Declarations<'a>(Vec<(CssProperty, &'a str)>);

impl<'a> Declarations<'a> {
    /// The declarations `style` holds, in their order.
    fn of(style: &'a str) -> Self {
        Self(
            style
                .split(';')
                .filter_map(|declaration| {
                    let (name, value) = declaration.split_once(':')?;
                    let value = value.trim();
                    let value = value.strip_suffix("!important").unwrap_or(value).trim();
                    Some((CssProperty::of(name), value))
                })
                .collect(),
        )
    }

    /// Whether the style hides the element: `display: none`, or Word's
    /// `mso-list: Ignore` on a list marker it draws as text.
    fn hidden(&self) -> bool {
        self.0.iter().any(|(property, value)| {
            let value = value.to_ascii_lowercase();
            (*property == CssProperty::Display && value == "none")
                || (*property == CssProperty::MsoList && value == "ignore")
        })
    }

    /// Whether the style names `property` at all.
    fn names(&self, property: CssProperty) -> bool {
        self.0.iter().any(|(named, _)| *named == property)
    }
}

impl Reader {
    fn walk(&mut self, nodes: &HtmlTree) {
        let mut steps = vec![Step::Enter(HtmlTree::DOCUMENT)];
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
                HtmlData::Text(text) => self.text(text),
                // An SVG or a MathML subtree carries no text a paste keeps:
                // the module hands it on as an ignored node.
                HtmlData::Ignored => {}
                HtmlData::Document => {
                    steps.extend(node.children.iter().rev().map(|c| Step::Enter(*c)));
                }
                HtmlData::Element { name, attrs } => {
                    let tag = name.as_str();
                    let style = Declarations::of(attr(attrs, "style").unwrap_or(""));
                    if SKIPPED.contains(&tag) || style.hidden() {
                        continue;
                    }
                    // A block that holds blocks is a page's theme, not formatting an
                    // author chose for this text: its style sets no mark.
                    let themed = BLOCKS.contains(&tag)
                        && node.children.iter().any(|child| {
                            nodes.get(*child).is_some_and(|child| {
                                matches!(&child.data, HtmlData::Element { name, .. }
                                    if BLOCKS.contains(&name.as_str()))
                            })
                        });
                    let origin = if themed {
                        StyleOrigin::Theme
                    } else {
                        StyleOrigin::Author
                    };
                    let frame = self.enter(tag, attrs, &style, origin);
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
    fn block_kind(&self, tag: &str, attrs: &[HtmlAttr], style: &Declarations<'_>) -> ClipKind {
        let heading = match tag {
            "h1" => Some(HeadingLevel::H1),
            "h2" => Some(HeadingLevel::H2),
            "h3" => Some(HeadingLevel::H3),
            "h4" => Some(HeadingLevel::H4),
            "h5" => Some(HeadingLevel::H5),
            "h6" => Some(HeadingLevel::H6),
            _ => None,
        };
        if let Some(level) = heading {
            return ClipKind::Heading(level);
        }
        match tag {
            "li" => {
                if attr(attrs, "role") == Some("checkbox") {
                    return ClipKind::Todo(TodoMark::done_if(
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
                    || style.names(CssProperty::MsoList);
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

    fn enter(
        &mut self,
        tag: &str,
        attrs: &[HtmlAttr],
        style: &Declarations<'_>,
        origin: StyleOrigin,
    ) -> Frame {
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
                frame.opened = Opened::List;
            }
            "input" => {
                if attr(attrs, "type").is_some_and(|t| t.eq_ignore_ascii_case("checkbox"))
                    && let Some(open) = self.open.last_mut()
                    && matches!(
                        open.kind,
                        ClipKind::Bullet | ClipKind::Ordered(_) | ClipKind::Todo(_)
                    )
                {
                    open.kind = ClipKind::Todo(TodoMark::done_if(attr(attrs, "checked").is_some()));
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
                    empty: if KEPT_EMPTY.contains(&tag) {
                        EmptyBlock::Keep
                    } else {
                        EmptyBlock::Drop
                    },
                    touched: Touched::Untouched,
                });
                frame.opened = Opened::Block;
                if tag == "pre" {
                    self.pre += 1;
                    frame.opened = Opened::Pre;
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
        if frame.opened == Opened::List {
            self.lists.pop();
        }
        if frame.opened == Opened::Pre {
            self.pre = self.pre.saturating_sub(1);
        }
        if matches!(frame.opened, Opened::Block | Opened::Pre) {
            let keep = match self.open.last() {
                Some(open) if open.touched == Touched::Untouched => open.empty,
                _ => EmptyBlock::Drop,
            };
            let wrote = self.flush(keep);
            let touched = self
                .open
                .pop()
                .is_some_and(|o| o.touched == Touched::Written)
                || wrote;
            if touched && let Some(parent) = self.open.last_mut() {
                parent.touched = Touched::Written;
            }
        }
    }

    fn rebuild(&mut self) {
        self.current = self
            .marks
            .iter()
            .map(|(key, value)| (key.clone(), ClipValue::Text(value.clone())))
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
            open.touched = Touched::Written;
        }
        true
    }
}

/// The language a code block names through a `language-` or `lang-` class.
fn language(attrs: &[HtmlAttr]) -> Option<Validated<CodeLang>> {
    attr(attrs, "class")
        .into_iter()
        .flat_map(str::split_ascii_whitespace)
        .find_map(|class| {
            class
                .strip_prefix("language-")
                .or_else(|| class.strip_prefix("lang-"))
        })
        .and_then(|lang| {
            Validated::<CodeLang>::parse(
                lang.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '#'))
                    .take(32)
                    .collect::<String>(),
            )
            .ok()
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
/// does not keep (`PROSE-UI-220`). The walk spells a colour and a size
/// itself, in the one spelling the document stores. So only a flag, a link
/// and a family name are checked here, by the rules Prose's paste applies.
fn set(key: MarkRoot, raw: &str) -> Option<Change> {
    let kept = match key {
        MarkRoot::Link => safe_link(raw),
        MarkRoot::FontFamily => safe_family(raw),
        MarkRoot::Bold
        | MarkRoot::Italic
        | MarkRoot::Underline
        | MarkRoot::Strike
        | MarkRoot::Superscript
        | MarkRoot::Subscript
        | MarkRoot::Code => raw.is_empty(),
        MarkRoot::Ink | MarkRoot::Highlight | MarkRoot::FontSize => true,
    };
    kept.then(|| Change::Set(key, raw.to_owned()))
}

/// The change that turns on the mark whose presence is the formatting.
fn on(key: MarkRoot) -> Option<Change> {
    set(key, "")
}

/// The marks an element's name stands for.
fn tag_marks(tag: &str, attrs: &[HtmlAttr], flow: Flow) -> Vec<Change> {
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
fn style_marks(style: &Declarations<'_>) -> Vec<Change> {
    let mut changes = Vec::new();
    for &(property, value) in &style.0 {
        let lower = value.to_ascii_lowercase();
        match property {
            CssProperty::FontWeight => {
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
            CssProperty::FontStyle => match lower.as_str() {
                "italic" | "oblique" => changes.extend(on(MarkRoot::Italic)),
                "normal" => changes.push(Change::Clear(MarkRoot::Italic)),
                _ => {}
            },
            CssProperty::TextDecoration => {
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
            CssProperty::VerticalAlign => match lower.as_str() {
                "super" => changes.extend(on(MarkRoot::Superscript)),
                "sub" => changes.extend(on(MarkRoot::Subscript)),
                "baseline" => changes.extend([
                    Change::Clear(MarkRoot::Superscript),
                    Change::Clear(MarkRoot::Subscript),
                ]),
                _ => {}
            },
            CssProperty::Color => {
                if let Some(color) = css_color(&lower) {
                    changes.extend(if ink(color) {
                        Some(Change::Clear(MarkRoot::Ink))
                    } else {
                        set(MarkRoot::Ink, &canonical_color(color))
                    });
                }
            }
            CssProperty::Background => {
                if lower == "transparent" {
                    changes.push(Change::Clear(MarkRoot::Highlight));
                } else if let Some(color) = css_color(&lower) {
                    let white = color == Rgba::from_bits(0xffff_ffff);
                    changes.extend(if white {
                        Some(Change::Clear(MarkRoot::Highlight))
                    } else {
                        set(MarkRoot::Highlight, &canonical_color(color))
                    });
                }
            }
            CssProperty::FontSize => {
                if let Some(points) = font_size(&lower).and_then(pasted_size) {
                    changes.extend(set(MarkRoot::FontSize, &points));
                }
            }
            CssProperty::FontFamily => {
                if let Some(family) = font_family(value) {
                    changes.push(Change::Set(MarkRoot::FontFamily, family));
                }
            }
            CssProperty::Display | CssProperty::MsoList | CssProperty::Other => {}
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
fn font_size(value: &str) -> Option<FontSize> {
    let points = if let Some(pt) = value.strip_suffix("pt") {
        pt.trim().parse::<f32>().ok()?
    } else {
        ambre_core::Px::<ambre_core::Doc>::new(
            value.strip_suffix("px")?.trim().parse::<f32>().ok()?,
        )
        .to_pt()
        .get()
    };
    FontSize::new((points * 2.0).round() / 2.0)
}

/// The spelling a pasted size lands in, as Prose's paste reads a size. One
/// past the cap lands at the cap (`CANVAS-088`), and one under a point lands
/// at none.
fn pasted_size(size: FontSize) -> Option<String> {
    let points = size.points().min(FONT_SIZE_MAX.points());
    if points < 1.0 {
        return None;
    }
    // A pasted size is a whole number of half points. So it is spelled as
    // the shortest decimal that reads back as it, as Prose's grammar spells
    // a size. The module thus links no float formatter.
    let halves = halves(points);
    Some(if halves.is_multiple_of(2) {
        (halves / 2).to_string()
    } else {
        format!("{}.5", halves / 2)
    })
}

/// A size of whole half points, as the count of its half points.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the size is a whole number of half points in one to 1638 points"
)]
fn halves(points: f32) -> u32 {
    (points * 2.0).round() as u32
}

/// The one spelling a colour is stored as: six digits when opaque, eight
/// otherwise, in lower case, as Prose's mark grammar spells it.
fn canonical_color(rgba: Rgba) -> String {
    match rgba.channels() {
        [_, _, _, 0xFF] => format!("#{:06x}", rgba.rgb().bits()),
        _ => format!("#{:08x}", rgba.bits()),
    }
}

/// A colour of six or eight hexadecimal digits, the six-digit one opaque.
fn parse_hex(digits: &str) -> Option<Rgba> {
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bits = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(Rgb::from_bits(bits).to_rgba(ambre_core::Alpha::OPAQUE)),
        8 => Some(Rgba::from_bits(bits)),
        _ => None,
    }
}

/// The first family a font stack names, when it names one rather than a
/// generic family or a platform alias.
fn font_family(value: &str) -> Option<MarkValue> {
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
    if !usable {
        return None;
    }
    safe_family(first).then(|| first.to_owned())
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
        return parse_hex(&long);
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
