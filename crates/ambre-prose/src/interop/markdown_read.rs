//! The Markdown reader: text to the intermediate form
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The markup converter module parses the text into its events
//! (`ambre_convert::markup::MdEvent`), and this reader maps them onto the
//! form. It reads GFM, which CommonMark is a subset of, so one reader serves
//! both flavours. It emits no command and sees no document (`PROSE-IO-009`). An
//! element it has no block or mark for is dropped with a warning, its text
//! kept where it has any (`PROSE-IO-005`). A file that is not text, or is too
//! large, fails as a typed error and yields no form at all (`PROSE-IO-010`).

use crate::message::{CodeLang, ConversionReport, DroppedElement, HeadingLevel, InteropError};
use ambre_convert::markup::{MdCodeBlock, MdEvent, MdHeading, MdList, MdTag, MdTagEnd};
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::Validated;

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

/// Read the bytes of `file` as Markdown, through the markup module
/// `converter` holds.
///
/// # Errors
///
/// [`InteropError::TooLarge`] past [`MAX_SIZE`], and
/// [`InteropError::ParseFailed`] for bytes that are not UTF-8. Every string of
/// text is some Markdown document, so the parser refuses nothing. A module
/// that stops or answers out of turn fails as [`InteropError::Stopped`].
pub fn read(
    file: &ambre_core::Payload<ambre_core::FileBytes>,
    converter: &Converter,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    let size = file.size();
    if size > MAX_SIZE {
        return Err(InteropError::TooLarge(size));
    }
    let text = std::str::from_utf8(file.as_bytes())
        .map_err(|e| InteropError::ParseFailed(format!("the file is not UTF-8 text: {e}")))?;
    let answer = converter.convert(&Conversion::ReadMarkdown {
        text: text.to_owned(),
    });
    match answer {
        Ok(Converted::Markdown(events)) => Ok(read_events(events)),
        Ok(_) => Err(InteropError::Stopped(ConvertError::Malformed.to_string())),
        Err(e) => Err(InteropError::Stopped(e.to_string())),
    }
}

/// Map the events of a Markdown text onto the form, and report what it lost.
#[must_use]
pub fn read_events(events: Vec<MdEvent>) -> (ProseIr, ConversionReport) {
    let mut reader = Reader::default();
    for event in events {
        reader.event(event);
    }
    reader.flush();
    (
        ProseIr {
            blocks: reader.root,
            ..ProseIr::default()
        },
        reader.warnings.report(),
    )
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

    fn event(&mut self, event: MdEvent) {
        if self.dropping() {
            match event {
                MdEvent::Start(MdTag::FootnoteDefinition | MdTag::HtmlBlock) => {
                    self.frames.push(Frame::Dropped);
                }
                MdEvent::End(MdTagEnd::FootnoteDefinition | MdTagEnd::HtmlBlock) => {
                    self.frames.pop();
                }
                _ => {}
            }
            return;
        }
        match event {
            MdEvent::Start(tag) => self.start(tag),
            MdEvent::End(tag) => self.end(tag),
            MdEvent::Text(text) => self.text(&text, Inline::Plain),
            MdEvent::Code(text) => self.text(&text, Inline::Code),
            MdEvent::SoftBreak | MdEvent::HardBreak => {
                if matches!(self.leaf, Some(Leaf::Code { .. })) {
                    self.text("\n", Inline::Plain);
                } else {
                    self.text(" ", Inline::Plain);
                }
            }
            MdEvent::Rule => {
                self.flush();
                self.place(IrBlock::HRule);
            }
            MdEvent::TaskListMarker(done) => {
                if let Some(Frame::Item { style, .. }) = self.frames.last_mut() {
                    *style = IrListStyle::Todo(crate::events::TodoMark::done_if(done));
                }
            }
            MdEvent::FootnoteReference => self.warnings.dropped(DroppedElement::Footnote),
            MdEvent::Html => self.warnings.dropped(DroppedElement::Html),
            MdEvent::Math(text) => {
                self.warnings.dropped(DroppedElement::Math);
                self.text(&text, Inline::Plain);
            }
        }
    }

    fn start(&mut self, tag: MdTag) {
        match tag {
            MdTag::Paragraph => {
                self.flush();
                self.leaf = Some(Leaf::Paragraph);
            }
            MdTag::Heading(level) => {
                self.flush();
                self.leaf = Some(Leaf::Heading(heading_level(level)));
            }
            MdTag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    MdCodeBlock::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .and_then(|tag| Validated::<CodeLang>::parse(tag).ok()),
                    MdCodeBlock::Indented => None,
                };
                self.leaf = Some(Leaf::Code {
                    lang,
                    text: String::new(),
                });
            }
            MdTag::List(kind) => {
                self.flush();
                self.frames.push(Frame::List {
                    style: match kind {
                        MdList::Ordered => IrListStyle::Ordered(IrOrderedStyle::Decimal),
                        MdList::Bullet => IrListStyle::Bullet,
                    },
                });
            }
            MdTag::Item => {
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
            MdTag::Table => {
                self.flush();
                self.frames.push(Frame::Table { rows: Vec::new() });
            }
            MdTag::TableHead | MdTag::TableRow => {
                self.frames.push(Frame::Row { cells: Vec::new() });
            }
            MdTag::TableCell => self.leaf = Some(Leaf::Cell),
            MdTag::BlockQuote => {
                self.flush();
                self.warnings.dropped(DroppedElement::BlockQuote);
                self.frames.push(Frame::Transparent);
            }
            MdTag::FootnoteDefinition => {
                self.flush();
                self.warnings.dropped(DroppedElement::Footnote);
                self.frames.push(Frame::Dropped);
            }
            MdTag::HtmlBlock => {
                self.flush();
                self.warnings.dropped(DroppedElement::Html);
                self.frames.push(Frame::Dropped);
            }
            MdTag::Emphasis => self.italic += 1,
            MdTag::Strong => self.bold += 1,
            MdTag::Strikethrough => self.strike += 1,
            MdTag::Link(dest) => self.links.push(ambre_core::LinkUrl::new(dest)),
            MdTag::Image => {
                self.warnings.dropped(DroppedElement::Image);
                self.image += 1;
            }
            MdTag::Other => {
                self.flush();
                self.warnings.dropped(DroppedElement::UnknownBlock);
                self.frames.push(Frame::Transparent);
            }
        }
    }

    fn end(&mut self, tag: MdTagEnd) {
        match tag {
            MdTagEnd::Paragraph => {
                let runs = std::mem::take(&mut self.runs);
                self.leaf = None;
                // A paragraph whose only content was dropped, an image alone,
                // leaves nothing behind.
                if !runs.is_empty() {
                    self.place_runs(runs);
                }
            }
            MdTagEnd::Heading => {
                let runs = std::mem::take(&mut self.runs);
                if let Some(Leaf::Heading(level)) = self.leaf.take() {
                    self.place(IrBlock::Heading {
                        level: StoredLevel::of(level),
                        runs,
                    });
                }
            }
            MdTagEnd::CodeBlock => {
                if let Some(Leaf::Code { lang, mut text }) = self.leaf.take() {
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    self.place(IrBlock::CodeBlock { lang, text });
                }
            }
            MdTagEnd::Item => {
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
            MdTagEnd::TableCell => {
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
            MdTagEnd::TableHead | MdTagEnd::TableRow => {
                if let Some(Frame::Row { cells }) = self.frames.pop()
                    && let Some(Frame::Table { rows }) = self.frames.last_mut()
                {
                    rows.push(IrTableRow { cells });
                }
            }
            MdTagEnd::Table => {
                if let Some(Frame::Table { rows }) = self.frames.pop() {
                    self.place(IrBlock::Table {
                        merged: Merging::Unmerged,
                        rows,
                    });
                }
            }
            MdTagEnd::List | MdTagEnd::BlockQuote => {
                self.flush();
                self.frames.pop();
            }
            MdTagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            MdTagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            MdTagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            MdTagEnd::Link => {
                self.links.pop();
            }
            MdTagEnd::Image => self.image = self.image.saturating_sub(1),
            MdTagEnd::FootnoteDefinition | MdTagEnd::HtmlBlock | MdTagEnd::Other => {
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

const fn heading_level(level: MdHeading) -> HeadingLevel {
    match level {
        MdHeading::H1 => HeadingLevel::H1,
        MdHeading::H2 => HeadingLevel::H2,
        MdHeading::H3 => HeadingLevel::H3,
        MdHeading::H4 => HeadingLevel::H4,
        MdHeading::H5 => HeadingLevel::H5,
        MdHeading::H6 => HeadingLevel::H6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interop::markdown_write;
    use crate::message::{MarkdownFlavor, ProseConstruct};

    /// [`read`] over a file of `bytes`, through the markup engine the host
    /// links in place.
    fn read_bytes(bytes: &[u8]) -> Result<(ProseIr, ConversionReport), InteropError> {
        let converter = Converter::load(
            ambre_core::ConverterFamily::Markup,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the engine");
        read(
            &ambre_core::Payload::<ambre_core::FileBytes>::new(bytes.to_vec()),
            &converter,
        )
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
