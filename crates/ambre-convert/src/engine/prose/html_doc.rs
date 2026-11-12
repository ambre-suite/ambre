//! A whole HTML document onto the intermediate form, for an import
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The blocks are the ones the walk of a paste reads, through the one reader
//! of `html_read` (`PROSE-IO-025`). This mapping only lifts them into the
//! form. A list item nests under the item its list sits in. Each mark takes
//! the field the form holds for it. The walk already dropped every
//! script, remote reference and handler, and named each in its report
//! (`PROSE-IO-016`, `PROSE-IO-062`).

use ambre_core::{
    Count, FontFamily, FontSize, LinkUrl, Rgba, Slant, Strikethrough, Underline, Weight,
};
use ambre_prose_form::clip::{ClipKind, ClipRun, ClipValue};
use ambre_prose_form::{
    DroppedElement, FormReport, InlineCode, IrBlock, IrListStyle, IrOrderedStyle, IrRun,
    OrderedStyle, OwnStyle, ProseIr, StoredLevel, Subscript, Superscript,
};

use super::html_read::{DocumentRead, ListNesting, parse_hex};
use super::md_read::MAX_LIST_DEPTH;

/// The form of a document the walk read, and the report of the walk and of
/// this mapping.
pub fn read(read: DocumentRead) -> (ProseIr, FormReport) {
    let DocumentRead {
        blocks,
        nesting,
        mut losses,
    } = read;
    let mut out: Vec<IrBlock> = Vec::new();
    for (block, depth) in blocks.into_iter().zip(nesting) {
        let runs: Vec<IrRun> = block.runs.iter().map(ir_run).collect();
        let style = match block.kind {
            ClipKind::Paragraph => {
                out.push(IrBlock::Paragraph {
                    styled: OwnStyle::Plain,
                    runs,
                });
                continue;
            }
            ClipKind::Heading(level) => {
                out.push(IrBlock::Heading {
                    level: StoredLevel::of(level),
                    runs,
                });
                continue;
            }
            ClipKind::Code(lang) => {
                out.push(IrBlock::CodeBlock {
                    lang,
                    text: block.runs.iter().map(|run| run.text.as_str()).collect(),
                });
                continue;
            }
            ClipKind::Bullet => IrListStyle::Bullet,
            ClipKind::Ordered(style) => IrListStyle::Ordered(ordered(style)),
            ClipKind::Todo(mark) => IrListStyle::Todo(mark),
        };
        let item = IrBlock::ListItem {
            style,
            runs,
            children: Vec::new(),
        };
        nest(&mut out, depth, item, &mut losses);
    }
    (
        ProseIr {
            blocks: out,
            ..ProseIr::default()
        },
        losses,
    )
}

/// Put `item` under the last item of the list it nests in. A list inside an
/// item sits one level under it. An item one level past the item before it
/// at most nests, and one past [`MAX_LIST_DEPTH`] lands at that depth,
/// reported (`PROSE-IO-005`).
fn nest(out: &mut Vec<IrBlock>, depth: Count<ListNesting>, item: IrBlock, losses: &mut FormReport) {
    // A Word item is a paragraph outside any list: it reads at the top.
    let cap = Count::<ListNesting>::recount(MAX_LIST_DEPTH);
    let depth = if depth > cap {
        losses.dropped(DroppedElement::DeepList);
        cap
    } else {
        depth
    };
    push_at(out, depth.minus(ONE), item);
}

/// One level of list nesting.
const ONE: Count<ListNesting> = Count::<ListNesting>::new(1);

/// Push `item` `level` levels down the last items of `siblings`. Where the
/// items stop sooner, it lands as deep as they go. The level is capped, so
/// the recursion is bounded.
fn push_at(siblings: &mut Vec<IrBlock>, level: Count<ListNesting>, item: IrBlock) {
    if !level.is_zero()
        && let Some(IrBlock::ListItem { children, .. }) = siblings.last_mut()
    {
        push_at(children, level.minus(ONE), item);
        return;
    }
    siblings.push(item);
}

fn ordered(style: OrderedStyle) -> IrOrderedStyle {
    match style {
        OrderedStyle::Decimal => IrOrderedStyle::Decimal,
        OrderedStyle::LowerAlpha => IrOrderedStyle::LowerAlpha,
        OrderedStyle::UpperAlpha => IrOrderedStyle::UpperAlpha,
        OrderedStyle::LowerRoman => IrOrderedStyle::LowerRoman,
        OrderedStyle::UpperRoman => IrOrderedStyle::UpperRoman,
    }
}

/// The run of the form a pasted run stands for. The walk spells each value
/// as the document stores it. A value that does not read back is one the
/// walk never writes, so it is left out.
fn ir_run(run: &ClipRun) -> IrRun {
    let mut out = IrRun::plain(run.text.clone());
    for (key, value) in &run.marks {
        let text = match value {
            ClipValue::Text(text) => text.as_str(),
            ClipValue::Bool(_) | ClipValue::Int(_) | ClipValue::Float(_) => "",
        };
        match key.as_str() {
            "b" => out.bold = Weight::Bold,
            "i" => out.italic = Slant::Italic,
            "u" => out.underline = Underline::Underlined,
            "s" => out.strikethrough = Strikethrough::Struck,
            "sup" => out.superscript = Superscript::Raised,
            "sub" => out.subscript = Subscript::Lowered,
            "c" => out.code = InlineCode::Code,
            "a" => out.link = Some(LinkUrl::new(text.to_owned())),
            "fg" => out.look.color = color(text),
            "bg" => out.look.background = color(text),
            "fs" => out.look.font_size = text.parse::<f32>().ok().and_then(FontSize::new),
            "ff" => out.look.font_family = Some(FontFamily::from_label(text)),
            _ => {}
        }
    }
    out
}

/// A colour of six or eight hexadecimal digits after a hash: the two
/// spellings the walk writes.
fn color(text: &str) -> Option<Rgba> {
    parse_hex(text.strip_prefix('#')?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_prose_form::{FormConstruct, FormOutcome, HeadingLevel, TodoMark};

    fn read_html(html: &str) -> (ProseIr, FormReport) {
        read(super::super::html_read::read_document(
            &crate::engine::html::read(html),
        ))
    }

    fn dropped(report: &FormReport, element: DroppedElement) -> u32 {
        report
            .count(FormConstruct::Element(element), FormOutcome::Dropped)
            .get()
    }

    fn text_of(ir: &ProseIr) -> String {
        fn walk(blocks: &[IrBlock], out: &mut String) {
            for block in blocks {
                match block {
                    IrBlock::Paragraph { runs, .. } | IrBlock::Heading { runs, .. } => {
                        out.extend(runs.iter().map(|r| r.text.as_str()));
                    }
                    IrBlock::ListItem { runs, children, .. } => {
                        out.extend(runs.iter().map(|r| r.text.as_str()));
                        walk(children, out);
                    }
                    IrBlock::CodeBlock { text, .. } => out.push_str(text),
                    _ => {}
                }
                out.push('|');
            }
        }
        let mut out = String::new();
        walk(&ir.blocks, &mut out);
        out
    }

    /// `PROSE-IO-016`, `PROSE-IO-062`: a script, a handler, a stylesheet, a
    /// font and a frame reach no block, and the report names each kind. The
    /// text around them lands.
    #[test]
    fn a_script_a_stylesheet_and_a_frame_are_named_and_never_kept() {
        let (ir, report) = read_html(concat!(
            "<html><head>",
            "<link rel=\"stylesheet\" href=\"https://tracker.example/a.css\">",
            "<link rel=\"preload\" as=\"font\" href=\"https://tracker.example/f.woff2\">",
            "<style>@import url(https://tracker.example/b.css);",
            "@font-face{src:url(https://tracker.example/c.woff)}</style>",
            "<script>fetch('https://tracker.example/')</script></head>",
            "<body onload=\"steal()\"><p onclick=\"steal()\">kept</p>",
            "<script>document.write('injected')</script>",
            "<iframe src=\"https://tracker.example/frame\">fallback</iframe>",
            "<object data=\"https://tracker.example/o\"></object>",
            "<p>also kept</p></body></html>"
        ));
        assert_eq!(text_of(&ir), "kept|also kept|");
        assert_eq!(dropped(&report, DroppedElement::Script), 4);
        assert_eq!(dropped(&report, DroppedElement::Stylesheet), 3);
        assert_eq!(dropped(&report, DroppedElement::EmbeddedContent), 2);
    }

    /// `PROSE-IO-023`, `PROSE-IO-054`: a picture that names an address is
    /// reported unresolved, one carried in the file as data is reported
    /// dropped, and neither lands. The reader has no way to fetch either:
    /// the module declares no network crate.
    #[test]
    fn a_remote_picture_is_reported_unresolved() {
        let (ir, report) = read_html(concat!(
            "<p>a<img src=\"https://tracker.example/pixel.gif\" alt=\"x\">b</p>",
            "<img src=\"/relative.png\"><img src=\"data:image/png;base64,AAAA\">"
        ));
        assert_eq!(text_of(&ir), "ab|");
        assert_eq!(dropped(&report, DroppedElement::UnresolvedImage), 2);
        assert_eq!(dropped(&report, DroppedElement::Image), 1);
    }

    /// `PROSE-IO-005`, `PROSE-IO-098`: an element the walk does not
    /// recognise is named, and its text kept. A table reads as one paragraph
    /// per cell, approximated. A form control and a rule are named dropped.
    #[test]
    fn an_unknown_element_is_reported_and_its_text_kept() {
        let (ir, report) = read_html(concat!(
            "<p><blink>odd</blink> text</p><table><tr><td>a</td><td>b</td></tr></table>",
            "<hr><button>press</button><textarea>typed</textarea>"
        ));
        assert_eq!(text_of(&ir), "odd text|a|b|");
        assert_eq!(dropped(&report, DroppedElement::Html), 1);
        assert_eq!(dropped(&report, DroppedElement::FormControl), 2);
        assert_eq!(
            report
                .count(
                    FormConstruct::Feature(ambre_prose_form::UnsupportedFeature::HorizontalRule),
                    FormOutcome::Dropped
                )
                .get(),
            1
        );
        assert_eq!(
            report
                .count(
                    FormConstruct::Feature(ambre_prose_form::UnsupportedFeature::Table),
                    FormOutcome::Approximated
                )
                .get(),
            1
        );
    }

    /// A list inside an item nests under it, and a to-do item keeps its
    /// mark.
    #[test]
    fn a_nested_list_nests_under_its_item() {
        let (ir, _) = read_html(concat!(
            "<h2>T</h2><ul><li>one<ol><li>two</li></ol></li>",
            "<li><input type=\"checkbox\" checked> done</li></ul>"
        ));
        let [
            IrBlock::Heading { level, .. },
            IrBlock::ListItem {
                style: IrListStyle::Bullet,
                children,
                ..
            },
            IrBlock::ListItem { style: todo, .. },
        ] = ir.blocks.as_slice()
        else {
            panic!("{:?}", ir.blocks);
        };
        assert_eq!(level.clamped(), HeadingLevel::H2);
        assert!(matches!(
            children.as_slice(),
            [IrBlock::ListItem {
                style: IrListStyle::Ordered(IrOrderedStyle::Decimal),
                ..
            }]
        ));
        assert_eq!(*todo, IrListStyle::Todo(TodoMark::Done));
    }
}
