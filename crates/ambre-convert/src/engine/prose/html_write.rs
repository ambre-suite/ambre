//! The intermediate form written as a standalone HTML document
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The file stands alone: its stylesheet sits in the head, and it names no
//! address but the links of the text (`PROSE-IO-063`). It is inert. The writer
//! emits no script, no event handler and no address a browser loads. A
//! content security policy in the head refuses all three, should one slip
//! through (`PROSE-IO-064`). Every text and attribute value is escaped, and a
//! link a host may not open is written as its text alone (`PROSE-IO-067`).
//!
//! The writer reads the form alone (`PROSE-IO-045`). It writes the threads
//! and the suggestions the form carries. The caller drops them first unless
//! the user asked for them (`PROSE-IO-022`).

use std::fmt::Write as _;

use ambre_core::{Slant, Strikethrough, ThreadState, Underline, Weight};
use ambre_prose_form::clip::safe_family;

use super::html_read::{canonical_color, pasted_size};
use ambre_prose_form::link::{openable, within_href_max};
use ambre_prose_form::{
    Banding, FormReport, InlineCode, IrBlock, IrComment, IrImage, IrImageAlign, IrListStyle,
    IrLook, IrOrderedStyle, IrRun, IrSuggestion, IrTableRow, Merging, OwnStyle, Paging, ProseIr,
    Subscript, Superscript, UnsupportedFeature,
};

/// The stylesheet every file carries in its head: the file needs no other
/// (`PROSE-IO-063`).
const STYLESHEET: &str = "body{font-family:sans-serif;line-height:1.5;max-width:46em;\
margin:2em auto;padding:0 1em}\
table{border-collapse:collapse}td{border:1px solid #999;padding:.25em .5em;vertical-align:top}\
pre{background:#f4f4f4;padding:.5em;overflow:auto}\
ul.todo{list-style:none;padding-left:1.2em}\
hr.page-break{border:0;break-after:page;page-break-after:always}\
section.footnotes,section.comments{border-top:1px solid #999;margin-top:2em}";

/// A policy that refuses every script, every handler and every address a
/// browser would load. The file needs none of them, so a writer bug that
/// let one through still runs nothing (`PROSE-IO-064`).
const POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'";

/// Write `ir` as an HTML document, and the report of what it lost.
#[must_use]
pub fn write(ir: &ProseIr) -> (String, FormReport) {
    let mut writer = Writer {
        out: String::new(),
        report: FormReport::default(),
        suggestions: &ir.suggestions,
    };
    // The format has no page and no element for a band (`PROSE-IO-019`).
    if ir.meta.paged == Paging::Paged {
        writer.report.unsupported(UnsupportedFeature::PageFormat);
    }
    if ir.meta.bands == Banding::Banded {
        writer
            .report
            .unsupported(UnsupportedFeature::HeaderAndFooter);
    }
    writer
        .out
        .push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    let _ = writeln!(
        writer.out,
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{POLICY}\">"
    );
    let _ = writeln!(
        writer.out,
        "<title>{}</title>\n<style>{STYLESHEET}</style>\n</head>\n<body>",
        escape(ir.meta.title.as_deref().unwrap_or("Document"))
    );
    let headings = headings(&ir.blocks);
    writer.blocks(&ir.blocks, &headings);
    if !ir.footnotes.is_empty() {
        writer
            .out
            .push_str("<section class=\"footnotes\" aria-label=\"Footnotes\">\n<ol>\n");
        for note in &ir.footnotes {
            let _ = write!(writer.out, "<li id=\"fn-{}\">", note.ordinal);
            writer.runs(&note.runs);
            writer.out.push_str("</li>\n");
        }
        writer.out.push_str("</ol>\n</section>\n");
    }
    writer.comments(&ir.comments);
    writer.out.push_str("</body>\n</html>\n");
    (writer.out, writer.report)
}

struct Writer<'a> {
    out: String,
    report: FormReport,
    suggestions: &'a [IrSuggestion],
}

/// The text of each heading at the top of the document: a table of contents
/// lists them.
fn headings(blocks: &[IrBlock]) -> Vec<String> {
    blocks
        .iter()
        .filter_map(|block| match block {
            IrBlock::Heading { runs, .. } => Some(runs.iter().map(|r| r.text.as_str()).collect()),
            _ => None,
        })
        .collect()
}

/// The list element a run of items sits in. Consecutive items of one kind
/// share one list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Bullet,
    Ordered(IrOrderedStyle),
    Todo,
}

impl ListKind {
    const fn of(style: IrListStyle) -> Self {
        match style {
            IrListStyle::Bullet => Self::Bullet,
            IrListStyle::Ordered(style) => Self::Ordered(style),
            IrListStyle::Todo(_) => Self::Todo,
        }
    }

    const fn open(self) -> &'static str {
        match self {
            Self::Bullet => "<ul>\n",
            Self::Todo => "<ul class=\"todo\">\n",
            Self::Ordered(IrOrderedStyle::Decimal) => "<ol>\n",
            Self::Ordered(IrOrderedStyle::LowerAlpha) => "<ol type=\"a\">\n",
            Self::Ordered(IrOrderedStyle::UpperAlpha) => "<ol type=\"A\">\n",
            Self::Ordered(IrOrderedStyle::LowerRoman) => "<ol type=\"i\">\n",
            Self::Ordered(IrOrderedStyle::UpperRoman) => "<ol type=\"I\">\n",
        }
    }

    const fn close(self) -> &'static str {
        match self {
            Self::Bullet | Self::Todo => "</ul>\n",
            Self::Ordered(_) => "</ol>\n",
        }
    }
}

impl Writer<'_> {
    fn blocks(&mut self, blocks: &[IrBlock], headings: &[String]) {
        let mut list: Option<ListKind> = None;
        for block in blocks {
            let kind = match block {
                IrBlock::ListItem { style, .. } => Some(ListKind::of(*style)),
                _ => None,
            };
            if list != kind {
                if let Some(open) = list {
                    self.out.push_str(open.close());
                }
                if let Some(kind) = kind {
                    self.out.push_str(kind.open());
                }
                list = kind;
            }
            self.block(block, headings);
        }
        if let Some(open) = list {
            self.out.push_str(open.close());
        }
    }

    fn block(&mut self, block: &IrBlock, headings: &[String]) {
        match block {
            IrBlock::Paragraph { styled, runs } => {
                if *styled == OwnStyle::Styled {
                    self.report
                        .unsupported(UnsupportedFeature::ParagraphFormatting);
                }
                self.element("p", runs);
            }
            IrBlock::Heading { level, runs } => {
                if !(1..=6).contains(&level.get()) {
                    self.report.approximated(UnsupportedFeature::HeadingLevel);
                }
                let tag = format!("h{}", level.clamped().get());
                self.element(&tag, runs);
            }
            IrBlock::ListItem {
                style,
                runs,
                children,
            } => {
                self.out.push_str("<li>");
                if let IrListStyle::Todo(mark) = style {
                    self.out.push_str(if mark.is_done() {
                        "<input type=\"checkbox\" disabled checked> "
                    } else {
                        "<input type=\"checkbox\" disabled> "
                    });
                }
                self.runs(runs);
                if !children.is_empty() {
                    self.out.push('\n');
                    self.blocks(children, headings);
                }
                self.out.push_str("</li>\n");
            }
            IrBlock::Table { merged, rows } => {
                if *merged == Merging::Merged {
                    self.report.approximated(UnsupportedFeature::MergedCells);
                }
                self.table(rows, headings);
            }
            IrBlock::CodeBlock { lang, text } => {
                self.out.push_str("<pre><code");
                if let Some(lang) = lang {
                    let _ = write!(self.out, " class=\"language-{}\"", escape(lang.as_str()));
                }
                let _ = writeln!(self.out, ">{}</code></pre>", escape_pre(text));
            }
            IrBlock::Image(image) => self.image(image),
            IrBlock::PageBreak => self.out.push_str("<hr class=\"page-break\">\n"),
            IrBlock::HRule => self.out.push_str("<hr>\n"),
            IrBlock::Toc => {
                // A list of the headings as they stand: it does not follow
                // a later edit.
                self.report
                    .approximated(UnsupportedFeature::TableOfContents);
                self.out
                    .push_str("<nav aria-label=\"Table of contents\">\n<ol>\n");
                for heading in headings {
                    let _ = writeln!(self.out, "<li>{}</li>", escape(heading));
                }
                self.out.push_str("</ol>\n</nav>\n");
            }
        }
    }

    fn element(&mut self, tag: &str, runs: &[IrRun]) {
        let _ = write!(self.out, "<{tag}>");
        self.runs(runs);
        let _ = writeln!(self.out, "</{tag}>");
    }

    /// A picture as a figure: its bytes inline as a `data:` address, so the
    /// file names nothing outside itself and stays inert (`PROSE-IO-064`).
    /// A picture whose bytes the export could not get is its alternative
    /// text in brackets, and a line of the report (`PROSE-IO-008`).
    fn image(&mut self, image: &IrImage) {
        match image.align {
            IrImageAlign::Center => self.out.push_str("<figure style=\"text-align:center\">"),
            IrImageAlign::Left | IrImageAlign::Right => {
                self.report.approximated(UnsupportedFeature::PictureWrap);
                self.out.push_str("<figure>");
            }
            IrImageAlign::Block => self.out.push_str("<figure>"),
        }
        match &image.bytes {
            Some(bytes) => {
                let _ = write!(
                    self.out,
                    "<img src=\"data:{};base64,{}\" alt=\"{}\"",
                    bytes.format.media_type(),
                    base64(&bytes.data),
                    escape(image.alt.as_str())
                );
                // A CSS pixel is three quarters of a point.
                let px = |pt: f32| (pt * 96.0 / 72.0).round();
                if image.width.get() > 0.0 && image.height.get() > 0.0 {
                    let _ = write!(
                        self.out,
                        " width=\"{}\" height=\"{}\"",
                        px(image.width.get()),
                        px(image.height.get())
                    );
                }
                self.out.push('>');
            }
            None => {
                self.report.approximated(UnsupportedFeature::PictureMissing);
                let _ = write!(self.out, "<span>[{}]</span>", escape(image.alt.as_str()));
            }
        }
        if !image.caption.as_str().is_empty() {
            let _ = write!(
                self.out,
                "<figcaption>{}</figcaption>",
                escape(image.caption.as_str())
            );
        }
        self.out.push_str("</figure>\n");
    }

    fn table(&mut self, rows: &[IrTableRow], headings: &[String]) {
        self.out.push_str("<table>\n<tbody>\n");
        for row in rows {
            self.out.push_str("<tr>");
            for cell in &row.cells {
                if cell.styled == OwnStyle::Styled {
                    self.report.unsupported(UnsupportedFeature::CellFormatting);
                }
                self.out.push_str("<td>\n");
                self.blocks(&cell.blocks, headings);
                self.out.push_str("</td>");
            }
            self.out.push_str("</tr>\n");
        }
        self.out.push_str("</tbody>\n</table>\n");
    }

    /// The runs of one block, each in the elements its marks stand for. Each
    /// run opens and closes its own elements, so the nesting is always well
    /// formed.
    fn runs(&mut self, runs: &[IrRun]) {
        for run in runs {
            if run.cross_ref.as_bool() {
                self.report.unsupported(UnsupportedFeature::CrossReference);
            }
            let mut close: Vec<&'static str> = Vec::new();
            if let Some(url) = run
                .link
                .as_ref()
                .filter(|url| openable(url) && within_href_max(url.size()))
            {
                let _ = write!(self.out, "<a href=\"{}\">", escape(url.as_str()));
                close.push("</a>");
            }
            if let Some(change) = run.insertion.or(run.deletion) {
                let (open, end) = match run.insertion {
                    Some(_) => ("<ins", "</ins>"),
                    None => ("<del", "</del>"),
                };
                let _ = write!(self.out, "{open} data-suggestion=\"{}\"", change.get());
                let author = self
                    .suggestions
                    .get(change.to_usize())
                    .and_then(|s| s.author.as_ref());
                if let Some(author) = author {
                    let _ = write!(self.out, " data-author=\"{}\"", escape(author.as_str()));
                }
                self.out.push('>');
                close.push(end);
            }
            if !run.comments.is_empty() {
                let ids: Vec<String> = run.comments.iter().map(|c| c.get().to_string()).collect();
                let _ = write!(self.out, "<span data-comment=\"{}\">", ids.join(" "));
                close.push("</span>");
            }
            let flags: [(bool, &'static str, &'static str); 7] = [
                (run.bold == Weight::Bold, "<strong>", "</strong>"),
                (run.italic == Slant::Italic, "<em>", "</em>"),
                (run.underline == Underline::Underlined, "<u>", "</u>"),
                (run.strikethrough == Strikethrough::Struck, "<s>", "</s>"),
                (run.superscript == Superscript::Raised, "<sup>", "</sup>"),
                (run.subscript == Subscript::Lowered, "<sub>", "</sub>"),
                (run.code == InlineCode::Code, "<code>", "</code>"),
            ];
            for (on, open, end) in flags {
                if on {
                    self.out.push_str(open);
                    close.push(end);
                }
            }
            if run.styled == OwnStyle::Styled {
                self.report.unsupported(UnsupportedFeature::Styling);
            }
            if let Some(style) = inline_style(&run.look) {
                let _ = write!(self.out, "<span style=\"{}\">", escape(&style));
                close.push("</span>");
            }
            self.out.push_str(&escape(&run.text));
            if let Some(ordinal) = run.footnote_ref {
                let _ = write!(
                    self.out,
                    "<sup id=\"fnref-{ordinal}\"><a href=\"#fn-{ordinal}\">{ordinal}</a></sup>"
                );
            }
            for end in close.into_iter().rev() {
                self.out.push_str(end);
            }
        }
    }

    /// The threads, after the text, each body with the name the document
    /// stamped on it. The form carries them only when the user asked for
    /// them (`PROSE-IO-022`).
    fn comments(&mut self, comments: &[IrComment]) {
        if comments.is_empty() {
            return;
        }
        self.out
            .push_str("<section class=\"comments\" aria-label=\"Comments\">\n");
        for (at, thread) in comments.iter().enumerate() {
            let _ = write!(self.out, "<article data-comment=\"{at}\"");
            if thread.resolved == ThreadState::Resolved {
                self.out.push_str(" data-resolved=\"true\"");
            }
            self.out.push_str(">\n");
            for reply in &thread.replies {
                self.out.push_str("<p>");
                if let Some(author) = &reply.author {
                    let _ = write!(self.out, "<strong>{}</strong>: ", escape(author.as_str()));
                }
                let _ = writeln!(self.out, "{}</p>", escape(&reply.text));
            }
            self.out.push_str("</article>\n");
        }
        self.out.push_str("</section>\n");
    }
}

/// The inline style a run's look writes, or `None` for a run with none. Each
/// value is spelled from its type, as a paste spells it. The one text of the
/// document a declaration holds is a family name that passes the paste's
/// check. The size is spelled in half points, so the module links no float
/// formatter.
fn inline_style(look: &IrLook) -> Option<String> {
    let mut style = String::new();
    let mut declare = |property: &str, value: &str| {
        if !style.is_empty() {
            style.push(';');
        }
        style.push_str(property);
        style.push(':');
        style.push_str(value);
    };
    if let Some(color) = look.color {
        declare("color", &canonical_color(color));
    }
    if let Some(color) = look.background {
        declare("background-color", &canonical_color(color));
    }
    if let Some(points) = look.font_size.and_then(pasted_size) {
        declare("font-size", &(points + "pt"));
    }
    if let Some(family) = look
        .font_family
        .as_ref()
        .map(ambre_core::FontFamily::label)
        .filter(|name| safe_family(name) && !name.contains(['\\', '\'']))
    {
        declare("font-family", &["'", family, "'"].concat());
    }
    (!style.is_empty()).then_some(style)
}

/// The standard base64 alphabet (RFC 4648 section 4).
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `bytes` as padded standard base64, the spelling a `data:` address takes.
/// Twenty lines here cost the markup module less than a crate.
fn base64(picture: &ambre_core::Payload<ambre_prose_form::ImageBytes>) -> String {
    let bytes = picture.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (at, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            let sextet = usize::try_from((n >> shift) & 63).unwrap_or(0);
            if at <= chunk.len() {
                out.push(char::from(BASE64.get(sextet).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `text` with every character that could open markup or end an attribute
/// value escaped, and every control character dropped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The text of a code block, escaped as [`escape`] does, its line breaks kept.
fn escape_pre(text: &str) -> String {
    text.split('\n').map(escape).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::html;
    use crate::markup::{HtmlData, HtmlTree};
    use ambre_core::{FontFamily, FontSize, LinkUrl, Rgba};
    use ambre_prose_form::{
        FormConstruct, FormOutcome, IrFootnote, IrLook, IrMeta, StoredLevel, TodoMark,
    };
    use proptest::prelude::*;

    fn para(runs: Vec<IrRun>) -> IrBlock {
        IrBlock::Paragraph {
            styled: OwnStyle::Plain,
            runs,
        }
    }

    /// Every element name and attribute of the tree a browser builds from
    /// `text`, each as `name` or `name@attribute=value`.
    fn elements(text: &str) -> Vec<String> {
        let tree: HtmlTree = html::read(text);
        let mut out = Vec::new();
        let mut stack = vec![HtmlTree::DOCUMENT];
        while let Some(id) = stack.pop() {
            let Some(node) = tree.get(id) else { continue };
            if let HtmlData::Element { name, attrs } = &node.data {
                out.push(name.clone());
                out.extend(
                    attrs
                        .iter()
                        .map(|a| format!("{name}@{}={}", a.name, a.value)),
                );
            }
            stack.extend(node.children.iter().copied());
        }
        out
    }

    /// Whether the tree of `text` holds nothing a browser runs or loads. It
    /// holds no script, no handler and no frame. Its one address is a link
    /// a reader follows, or an anchor inside the file.
    fn inert(text: &str) -> Result<(), String> {
        for element in elements(text) {
            let bad = match element.split_once('@') {
                None => ["script", "iframe", "object", "embed", "link", "img", "base"]
                    .contains(&element.as_str()),
                Some((tag, attr)) => {
                    let (name, value) = attr.split_once('=').unwrap_or((attr, ""));
                    name.starts_with("on")
                        || ["src", "srcset", "action", "formaction", "data"].contains(&name)
                        || (name == "href"
                            && !(tag == "a"
                                && (value.starts_with("https://")
                                    || value.starts_with("http://")
                                    || value.starts_with("mailto:")
                                    || value.starts_with("#fn-"))))
                        || (name == "style" && value.contains("url("))
                }
            };
            if bad {
                return Err(element);
            }
        }
        Ok(())
    }

    /// `PROSE-IO-063`: the file names its stylesheet in its head and needs
    /// no other file: no link, no address a browser loads.
    #[test]
    fn the_file_stands_alone_with_its_stylesheet_in_the_head() {
        let ir = ProseIr {
            blocks: vec![para(vec![IrRun::plain("hello")])],
            ..ProseIr::default()
        };
        let (text, report) = write(&ir);
        let head = text.split("</head>").next().unwrap_or_default();
        assert!(head.contains("<style>body{"), "{head}");
        assert!(head.contains("Content-Security-Policy"), "{head}");
        assert!(!text.contains("url("), "{text}");
        assert_eq!(inert(&text), Ok(()));
        assert!(report.entries().is_empty(), "{report:?}");
        assert!(elements(&text).contains(&"style".to_owned()));
    }

    /// `PROSE-IO-019`: the header and the footer have no element, so the
    /// writer reports them, and the page format beside them.
    #[test]
    fn the_bands_and_the_page_are_reported() {
        let ir = ProseIr {
            meta: IrMeta {
                paged: Paging::Paged,
                bands: Banding::Banded,
                ..IrMeta::default()
            },
            ..ProseIr::default()
        };
        let (_, report) = write(&ir);
        for feature in [
            UnsupportedFeature::HeaderAndFooter,
            UnsupportedFeature::PageFormat,
        ] {
            assert_eq!(
                report
                    .count(FormConstruct::Feature(feature), FormOutcome::Dropped)
                    .get(),
                1,
                "{feature:?}"
            );
        }
    }

    /// A link a host may not open is written as its text alone, and every
    /// text that looks like markup is escaped.
    #[test]
    fn a_hostile_text_and_link_are_written_inert() {
        let run = |text: &str, link: &str| IrRun {
            link: Some(LinkUrl::new(link.to_owned())),
            ..IrRun::plain(text)
        };
        let ir = ProseIr {
            meta: IrMeta {
                title: Some("</title><script>alert(1)</script>".to_owned()),
                ..IrMeta::default()
            },
            blocks: vec![
                para(vec![
                    run("<script>alert(1)</script>", "javascript:alert(1)"),
                    run(
                        "x\" onmouseover=\"alert(1)",
                        "https://example.com/\"onclick=\"a",
                    ),
                    run("data", "data:text/html,<script>alert(1)</script>"),
                ]),
                IrBlock::CodeBlock {
                    lang: None,
                    text: "</code></pre><img src=x onerror=alert(1)>".to_owned(),
                },
            ],
            ..ProseIr::default()
        };
        let (text, _) = write(&ir);
        assert_eq!(inert(&text), Ok(()), "{text}");
        assert!(!text.contains("javascript:"), "{text}");
        assert!(!text.contains("<script"), "{text}");
    }

    fn hostile_text() -> impl Strategy<Value = String> {
        prop_oneof![
            any::<String>(),
            "[<>\"'&=/ a-z:;()]{0,40}",
            Just("<script>alert(1)</script>".to_owned()),
            Just("\" onload=\"x".to_owned()),
            Just("javascript:alert(1)".to_owned()),
        ]
    }

    fn hostile_run() -> impl Strategy<Value = IrRun> {
        (
            hostile_text(),
            proptest::option::of(hostile_text()),
            proptest::option::of(hostile_text()),
            any::<u32>(),
            any::<bool>(),
        )
            .prop_map(|(text, link, family, bits, bold)| IrRun {
                link: link.map(LinkUrl::new),
                bold: if bold { Weight::Bold } else { Weight::Regular },
                look: IrLook {
                    color: Some(Rgba::from_bits(bits)),
                    background: None,
                    font_size: FontSize::new(12.0),
                    font_family: family.map(|f| FontFamily::from_label(&f)),
                },
                ..IrRun::plain(text)
            })
    }

    proptest! {
        /// `PROSE-IO-064`: the file holds no script, no handler and no
        /// address a browser loads. That holds for any text, link, family,
        /// title or code the document holds.
        #[test]
        fn the_file_is_inert_whatever_the_document_holds(
            runs in proptest::collection::vec(hostile_run(), 0..6),
            title in proptest::option::of(hostile_text()),
            code in hostile_text(),
            note in hostile_run(),
        ) {
            let ir = ProseIr {
                meta: IrMeta { title, ..IrMeta::default() },
                blocks: vec![
                    para(runs.clone()),
                    IrBlock::ListItem {
                        style: IrListStyle::Todo(TodoMark::Done),
                        runs,
                        children: Vec::new(),
                    },
                    IrBlock::CodeBlock { lang: None, text: code },
                ],
                footnotes: vec![IrFootnote {
                    ordinal: ambre_core::Ordinal::FIRST,
                    runs: vec![note],
                }],
                ..ProseIr::default()
            };
            let (text, _) = write(&ir);
            prop_assert_eq!(inert(&text), Ok(()));
        }
    }

    /// A form the writer writes reads back through the import walk as the
    /// same form: the blocks, the nesting, the marks and the look.
    #[test]
    fn a_written_form_reads_back_as_itself() {
        let marked = IrRun {
            bold: Weight::Bold,
            italic: Slant::Italic,
            underline: Underline::Underlined,
            strikethrough: Strikethrough::Struck,
            link: Some(LinkUrl::new("https://example.com/?a=1&b=2".to_owned())),
            look: IrLook {
                color: Some(Rgba::from_bits(0xff00_00ff)),
                background: Some(Rgba::from_bits(0xffff_00ff)),
                font_size: FontSize::new(13.5),
                font_family: Some(FontFamily::from_label("Georgia")),
            },
            ..IrRun::plain("rich")
        };
        let item = |style, text: &str, children| IrBlock::ListItem {
            style,
            runs: vec![IrRun::plain(text)],
            children,
        };
        let ir = ProseIr {
            blocks: vec![
                IrBlock::Heading {
                    level: StoredLevel::from_stored(2),
                    runs: vec![IrRun::plain("Title")],
                },
                para(vec![
                    IrRun::plain("plain "),
                    marked,
                    IrRun {
                        superscript: Superscript::Raised,
                        ..IrRun::plain("2")
                    },
                ]),
                item(
                    IrListStyle::Bullet,
                    "one",
                    vec![item(
                        IrListStyle::Ordered(IrOrderedStyle::LowerRoman),
                        "two",
                        Vec::new(),
                    )],
                ),
                item(IrListStyle::Todo(TodoMark::Done), "done", Vec::new()),
                IrBlock::CodeBlock {
                    lang: ambre_core::Validated::parse("rust".to_owned()).ok(),
                    text: "fn a() {}\n    b();".to_owned(),
                },
            ],
            ..ProseIr::default()
        };
        let (text, report) = write(&ir);
        assert!(report.entries().is_empty(), "{report:?}");
        let (back, _) = super::super::html_doc::read(super::super::html_read::read_document(
            &html::read(&text),
        ));
        assert_eq!(back.blocks, ir.blocks, "{text}");
    }
}
