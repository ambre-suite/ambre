//! The Markdown reader: text to the intermediate form
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! It reads GFM, which CommonMark is a subset of, so one reader serves both
//! flavours. It emits no command and sees no document (`PROSE-IO-009`). An
//! element it has no block or mark for is dropped with a warning, its text
//! kept where it has any (`PROSE-IO-005`). A file that is not text, or is too
//! large, fails as a typed error and yields no form at all (`PROSE-IO-010`).

use crate::message::{CodeLang, ConversionReport, DroppedElement, HeadingLevel, InteropError};
use ambre_core::Validated;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use ambre_core::{Count, Slant, Strikethrough, Weight};

use super::{
    IrBlock, IrListStyle, IrOrderedStyle, IrRun, IrTableCell, IrTableRow, Merging, OwnStyle,
    ProseIr, Warnings,
};
use crate::doc::StoredLevel;
use crate::mark::InlineCode;

/// The largest file a Markdown import reads.
pub const MAX_SIZE: ambre_core::Count<ambre_core::Octets> =
    ambre_core::Count::<ambre_core::Octets>::new(16 * 1024 * 1024);

/// The deepest list the form keeps. A deeper item lands at this depth, so a
/// hostile file cannot grow the recursion of the passes that follow.
pub const MAX_LIST_DEPTH: Count<ListDepth> = Count::<ListDepth>::new(16);

pub use crate::numbering::ListDepth;

/// Read the bytes of `file` as Markdown.
///
/// # Errors
///
/// [`InteropError::TooLarge`] past [`MAX_SIZE`], and
/// [`InteropError::ParseFailed`] for bytes that are not UTF-8. Every string of
/// text is some Markdown document, so nothing else fails.
pub fn read(
    file: &ambre_core::Payload<ambre_core::FileBytes>,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    let size = file.size();
    if size > MAX_SIZE {
        return Err(InteropError::TooLarge(size));
    }
    let text = std::str::from_utf8(file.as_bytes())
        .map_err(|e| InteropError::ParseFailed(format!("the file is not UTF-8 text: {e}")))?;
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut reader = Reader::default();
    for event in Parser::new_ext(text, options) {
        reader.event(event);
    }
    reader.flush();
    Ok((
        ProseIr {
            blocks: reader.root,
            ..ProseIr::default()
        },
        reader.warnings.report(),
    ))
}

/// A block that holds other blocks while the reader is inside it.
enum Frame {
    /// A list, with the style each of its items takes.
    List {
        style: IrListStyle,
    },
    Item {
        style: IrListStyle,
        runs: Option<Vec<IrRun>>,
        children: Vec<IrBlock>,
        /// Items nested past [`MAX_LIST_DEPTH`], which land after this one.
        trailing: Vec<IrBlock>,
    },
    Table {
        rows: Vec<IrTableRow>,
    },
    Row {
        cells: Vec<IrTableCell>,
    },
    /// A container whose own markup is dropped and whose blocks land in the
    /// parent: a quote.
    Transparent,
    /// A container dropped with everything in it: a footnote body, raw HTML.
    Dropped,
}

/// The block whose text the inline events fill.
enum Leaf {
    Paragraph,
    Heading(HeadingLevel),
    Cell,
    Code {
        lang: Option<Validated<CodeLang>>,
        text: String,
    },
}

struct Reader {
    root: Vec<IrBlock>,
    frames: Vec<Frame>,
    leaf: Option<Leaf>,
    runs: Vec<IrRun>,
    bold: u32,
    italic: u32,
    strike: u32,
    links: Vec<ambre_core::LinkUrl>,
    /// Inside an image: its alt text is dropped with it.
    image: u32,
    warnings: Warnings,
}

impl Default for Reader {
    fn default() -> Self {
        Self {
            root: Vec::new(),
            frames: Vec::new(),
            leaf: None,
            runs: Vec::new(),
            bold: 0,
            italic: 0,
            strike: 0,
            links: Vec::new(),
            image: 0,
            warnings: Warnings::new(
                ambre_bus::interop::Direction::Import,
                ambre_bus::interop::FileFormat::Markdown,
            ),
        }
    }
}

/// Whether a run of text is inline code or plain text.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Inline {
    Plain,
    Code,
}

impl Reader {
    fn dropping(&self) -> bool {
        self.frames.iter().any(|f| matches!(f, Frame::Dropped))
    }

    fn event(&mut self, event: Event<'_>) {
        if self.dropping() {
            match event {
                Event::Start(Tag::FootnoteDefinition(_) | Tag::HtmlBlock) => {
                    self.frames.push(Frame::Dropped);
                }
                Event::End(TagEnd::FootnoteDefinition | TagEnd::HtmlBlock) => {
                    self.frames.pop();
                }
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text, Inline::Plain),
            Event::Code(text) => self.text(&text, Inline::Code),
            Event::SoftBreak | Event::HardBreak => {
                if matches!(self.leaf, Some(Leaf::Code { .. })) {
                    self.text("\n", Inline::Plain);
                } else {
                    self.text(" ", Inline::Plain);
                }
            }
            Event::Rule => {
                self.flush();
                self.place(IrBlock::HRule);
            }
            Event::TaskListMarker(done) => {
                if let Some(Frame::Item { style, .. }) = self.frames.last_mut() {
                    *style = IrListStyle::Todo(crate::events::TodoMark::done_if(done));
                }
            }
            Event::FootnoteReference(_) => self.warnings.dropped(DroppedElement::Footnote),
            Event::Html(_) | Event::InlineHtml(_) => self.warnings.dropped(DroppedElement::Html),
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                self.warnings.dropped(DroppedElement::Math);
                self.text(&text, Inline::Plain);
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                self.leaf = Some(Leaf::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.leaf = Some(Leaf::Heading(heading_level(level)));
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .and_then(|tag| Validated::<CodeLang>::parse(tag).ok()),
                    CodeBlockKind::Indented => None,
                };
                self.leaf = Some(Leaf::Code {
                    lang,
                    text: String::new(),
                });
            }
            Tag::List(start) => {
                self.flush();
                self.frames.push(Frame::List {
                    style: match start {
                        Some(_) => IrListStyle::Ordered(IrOrderedStyle::Decimal),
                        None => IrListStyle::Bullet,
                    },
                });
            }
            Tag::Item => {
                self.flush();
                let style = match self.frames.last() {
                    Some(Frame::List { style }) => *style,
                    _ => IrListStyle::Bullet,
                };
                self.frames.push(Frame::Item {
                    style,
                    runs: None,
                    children: Vec::new(),
                    trailing: Vec::new(),
                });
            }
            Tag::Table(_) => {
                self.flush();
                self.frames.push(Frame::Table { rows: Vec::new() });
            }
            Tag::TableHead | Tag::TableRow => self.frames.push(Frame::Row { cells: Vec::new() }),
            Tag::TableCell => self.leaf = Some(Leaf::Cell),
            Tag::BlockQuote(_) => {
                self.flush();
                self.warnings.dropped(DroppedElement::BlockQuote);
                self.frames.push(Frame::Transparent);
            }
            Tag::FootnoteDefinition(_) => {
                self.flush();
                self.warnings.dropped(DroppedElement::Footnote);
                self.frames.push(Frame::Dropped);
            }
            Tag::HtmlBlock => {
                self.flush();
                self.warnings.dropped(DroppedElement::Html);
                self.frames.push(Frame::Dropped);
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self
                .links
                .push(ambre_core::LinkUrl::new(dest_url.to_string())),
            Tag::Image { .. } => {
                self.warnings.dropped(DroppedElement::Image);
                self.image += 1;
            }
            _ => {
                self.flush();
                self.warnings.dropped(DroppedElement::UnknownBlock);
                self.frames.push(Frame::Transparent);
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                let runs = std::mem::take(&mut self.runs);
                self.leaf = None;
                // A paragraph whose only content was dropped, an image alone,
                // leaves nothing behind.
                if !runs.is_empty() {
                    self.place_runs(runs);
                }
            }
            TagEnd::Heading(_) => {
                let runs = std::mem::take(&mut self.runs);
                if let Some(Leaf::Heading(level)) = self.leaf.take() {
                    self.place(IrBlock::Heading {
                        level: StoredLevel::of(level),
                        runs,
                    });
                }
            }
            TagEnd::CodeBlock => {
                if let Some(Leaf::Code { lang, mut text }) = self.leaf.take() {
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    self.place(IrBlock::CodeBlock { lang, text });
                }
            }
            TagEnd::Item => {
                self.flush();
                if let Some(Frame::Item {
                    style,
                    runs,
                    children,
                    trailing,
                }) = self.frames.pop()
                {
                    self.place(IrBlock::ListItem {
                        style,
                        runs: runs.unwrap_or_default(),
                        children,
                    });
                    for item in trailing {
                        self.place(item);
                    }
                }
            }
            TagEnd::TableCell => {
                self.leaf = None;
                let runs = std::mem::take(&mut self.runs);
                if let Some(Frame::Row { cells }) = self.frames.last_mut() {
                    cells.push(IrTableCell {
                        styled: OwnStyle::Plain,
                        blocks: vec![IrBlock::Paragraph {
                            styled: OwnStyle::Plain,
                            runs,
                        }],
                    });
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(Frame::Row { cells }) = self.frames.pop()
                    && let Some(Frame::Table { rows }) = self.frames.last_mut()
                {
                    rows.push(IrTableRow { cells });
                }
            }
            TagEnd::Table => {
                if let Some(Frame::Table { rows }) = self.frames.pop() {
                    self.place(IrBlock::Table {
                        merged: Merging::Unmerged,
                        rows,
                    });
                }
            }
            TagEnd::List(_) | TagEnd::BlockQuote(_) => {
                self.flush();
                self.frames.pop();
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::Image => self.image = self.image.saturating_sub(1),
            _ => {
                self.flush();
                self.frames.pop();
            }
        }
    }

    fn text(&mut self, text: &str, inline: Inline) {
        if self.image > 0 {
            return;
        }
        if let Some(Leaf::Code { text: body, .. }) = &mut self.leaf {
            body.push_str(text);
            return;
        }
        let run = IrRun {
            text: text.to_owned(),
            bold: Weight::from_bool(self.bold > 0),
            italic: Slant::from_bool(self.italic > 0),
            strikethrough: Strikethrough::from_bool(self.strike > 0),
            code: InlineCode::from_bool(inline == Inline::Code),
            link: self.links.last().cloned(),
            ..IrRun::default()
        };
        match self.runs.last_mut() {
            Some(last) if same_marks(last, &run) => last.text.push_str(&run.text),
            _ => self.runs.push(run),
        }
    }

    /// Land the text a tight list item holds outside any paragraph.
    fn flush(&mut self) {
        if self.leaf.is_none() && !self.runs.is_empty() {
            let runs = std::mem::take(&mut self.runs);
            self.place_runs(runs);
        }
    }

    /// Runs outside a heading: an item's own text if it has none yet, a
    /// paragraph otherwise.
    fn place_runs(&mut self, runs: Vec<IrRun>) {
        if let Some(Frame::Item {
            runs: own,
            children,
            ..
        }) = self.frames.last_mut()
            && own.is_none()
            && children.is_empty()
        {
            *own = Some(runs);
            return;
        }
        self.place(IrBlock::Paragraph {
            styled: OwnStyle::Plain,
            runs,
        });
    }

    /// Put a finished block in the innermost frame that holds blocks. An
    /// item that would nest past [`MAX_LIST_DEPTH`] waits to land after the
    /// deepest item allowed, as its sibling.
    fn place(&mut self, block: IrBlock) {
        let depth = Count::<ListDepth>::of(
            self.frames
                .iter()
                .filter(|f| matches!(f, Frame::Item { .. }))
                .count(),
        );
        let is_item = matches!(block, IrBlock::ListItem { .. });
        for frame in self.frames.iter_mut().rev() {
            if let Frame::Item {
                runs,
                children,
                trailing,
                ..
            } = frame
            {
                if is_item && depth >= MAX_LIST_DEPTH {
                    self.warnings.dropped(DroppedElement::DeepList);
                    trailing.push(block);
                    return;
                }
                if runs.is_none() {
                    *runs = Some(Vec::new());
                }
                children.push(block);
                return;
            }
            if matches!(frame, Frame::Table { .. } | Frame::Row { .. }) {
                break;
            }
        }
        self.root.push(block);
    }
}

fn same_marks(a: &IrRun, b: &IrRun) -> bool {
    a.bold == b.bold
        && a.italic == b.italic
        && a.strikethrough == b.strikethrough
        && a.code == b.code
        && a.link == b.link
}

fn heading_level(level: pulldown_cmark::HeadingLevel) -> HeadingLevel {
    match level {
        pulldown_cmark::HeadingLevel::H1 => HeadingLevel::H1,
        pulldown_cmark::HeadingLevel::H2 => HeadingLevel::H2,
        pulldown_cmark::HeadingLevel::H3 => HeadingLevel::H3,
        pulldown_cmark::HeadingLevel::H4 => HeadingLevel::H4,
        pulldown_cmark::HeadingLevel::H5 => HeadingLevel::H5,
        pulldown_cmark::HeadingLevel::H6 => HeadingLevel::H6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interop::markdown_write;
    use crate::message::{MarkdownFlavor, ProseConstruct};

    /// [`read`] over a file of `bytes`.
    fn read_bytes(bytes: &[u8]) -> Result<(ProseIr, ConversionReport), InteropError> {
        read(&ambre_core::Payload::<ambre_core::FileBytes>::new(
            bytes.to_vec(),
        ))
    }

    fn blocks(markdown: &str) -> Vec<IrBlock> {
        read_bytes(markdown.as_bytes()).expect("reads").0.blocks
    }

    fn para(runs: Vec<IrRun>) -> IrBlock {
        IrBlock::Paragraph {
            styled: OwnStyle::Plain,
            runs,
        }
    }

    #[test]
    fn blocks_and_marks_read_into_the_form() {
        let read = blocks(
            "# Title\n\nSome **bold** and [a link](https://x.org).\n\n```rust\nlet a = 1;\n```\n",
        );
        assert_eq!(
            read,
            vec![
                IrBlock::Heading {
                    level: StoredLevel::of(HeadingLevel::H1),
                    runs: vec![IrRun::plain("Title")]
                },
                para(vec![
                    IrRun::plain("Some "),
                    IrRun {
                        bold: Weight::Bold,
                        ..IrRun::plain("bold")
                    },
                    IrRun::plain(" and "),
                    IrRun {
                        link: Some("https://x.org".into()),
                        ..IrRun::plain("a link")
                    },
                    IrRun::plain("."),
                ]),
                IrBlock::CodeBlock {
                    lang: Validated::<CodeLang>::parse("rust").ok(),
                    text: "let a = 1;".to_owned()
                },
            ]
        );
    }

    #[test]
    fn a_nested_list_nests_and_a_task_is_a_todo() {
        let read = blocks("1. one\n   - inner\n2. two\n\n- [x] done\n");
        let item = |style, text: &str, children| IrBlock::ListItem {
            style,
            runs: vec![IrRun::plain(text)],
            children,
        };
        let decimal = IrListStyle::Ordered(IrOrderedStyle::Decimal);
        assert_eq!(
            read,
            vec![
                item(
                    decimal,
                    "one",
                    vec![item(IrListStyle::Bullet, "inner", Vec::new())]
                ),
                item(decimal, "two", Vec::new()),
                item(
                    IrListStyle::Todo(crate::events::TodoMark::Done),
                    "done",
                    Vec::new()
                ),
            ]
        );
    }

    #[test]
    fn a_pipe_table_reads_as_rows_of_cells() {
        let read = blocks("| a | b |\n|---|---|\n| 1 | 2 |\n");
        let IrBlock::Table { rows, .. } = &read[0] else {
            panic!("{read:?}");
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].cells[1].blocks, vec![para(vec![IrRun::plain("2")])]);
    }

    /// `PROSE-IO-005`: what the form has no place for is named, and the
    /// quote's text is kept.
    #[test]
    fn an_unknown_element_is_reported_and_its_text_kept() {
        let (ir, report) =
            read_bytes(b"> quoted\n\n<div>raw</div>\n\n![alt](x.png)\n").expect("reads");
        assert_eq!(ir.blocks, vec![para(vec![IrRun::plain("quoted")])]);
        let dropped: Vec<_> = report
            .entries()
            .iter()
            .filter_map(|entry| match entry.construct {
                ProseConstruct::Element(element) => Some(element.label()),
                _ => None,
            })
            .collect();
        assert_eq!(dropped, ["block quote", "HTML", "image"]);
    }

    /// `PROSE-IO-010`: bytes that are not text fail typed, and nothing is
    /// read.
    #[test]
    fn bytes_that_are_not_text_fail_typed() {
        assert!(matches!(
            read_bytes(&[0xff, 0xfe, 0x00]),
            Err(InteropError::ParseFailed(_))
        ));
        assert!(matches!(
            read_bytes(&vec![b'a'; MAX_SIZE.saturating_len().to_usize() + 1]),
            Err(InteropError::TooLarge(_))
        ));
    }

    /// A list nested past the cap lands at the cap, and says so.
    #[test]
    fn a_hostile_nesting_is_capped() {
        let mut markdown = String::new();
        for depth in 0..200 {
            markdown.push_str(&"  ".repeat(depth));
            markdown.push_str("- x\n");
        }
        let (ir, report) = read_bytes(markdown.as_bytes()).expect("reads");
        fn depth(blocks: &[IrBlock]) -> usize {
            blocks
                .iter()
                .map(|b| match b {
                    IrBlock::ListItem { children, .. } => 1 + depth(children),
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
        }
        assert!(
            Count::<ListDepth>::of(depth(&ir.blocks)) <= MAX_LIST_DEPTH,
            "{}",
            depth(&ir.blocks)
        );
        assert!(!report.is_lossless());
    }

    /// Written then read, the form comes back: the reader and the writer
    /// agree on every construct both carry.
    #[test]
    fn what_the_writer_writes_the_reader_reads_back() {
        let source = "## Plan\n\nA *b* **c** ~~d~~ `e` [f](https://g.org)\n\n1. one\n   - two\n\n- [ ] three\n\n| h | i |\n| --- | --- |\n| j | k |\n\n```\ncode\n```\n\n---\n";
        let (ir, _) = read_bytes(source.as_bytes()).expect("reads");
        let (written, _) = markdown_write::write(&ir, MarkdownFlavor::Gfm);
        assert_eq!(written, source);
    }
}
