//! The Markdown reader: text to the intermediate form
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The parser answers the events of the text (`crate::markup::MdEvent`).
//! This reader maps them onto Prose's intermediate form in the markup
//! module, so the Main instance links neither (`ambre-spec-budgets.md
//! section 5`). It reads GFM, which CommonMark is a subset of, so one reader serves
//! both flavours. It emits no command and sees no document (`PROSE-IO-009`). An
//! element it has no block or mark for is dropped with a warning, its text
//! kept where it has any (`PROSE-IO-005`).

use ambre_core::Validated;
use ambre_core::{Count, CountKind, Slant, Strikethrough, Weight};
use ambre_prose_form::{
    CodeLang, DroppedElement, FormReport, HeadingLevel, InlineCode, IrBlock, IrListStyle,
    IrOrderedStyle, IrRun, IrTableCell, IrTableRow, Merging, OwnStyle, ProseIr, StoredLevel,
    TodoMark,
};

use crate::markup::{MdCodeBlock, MdEvent, MdHeading, MdList, MdTag, MdTagEnd};
use ambre_core::Flag as _;

/// How deep a list item nests, its outermost item at one.
pub enum ListDepth {}

impl CountKind for ListDepth {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("MarkdownListDepth");
}

/// The deepest list the form keeps. A deeper item lands at this depth, so a
/// hostile file cannot grow the recursion of the passes that follow.
pub const MAX_LIST_DEPTH: Count<ListDepth> = Count::<ListDepth>::new(16);

/// The form `text` holds, and what the reader lost.
#[must_use]
pub fn read(text: &str) -> (ProseIr, FormReport) {
    read_events(super::super::markdown::read(text))
}

/// Map the events of a Markdown text onto the form, and report what it lost.
#[must_use]
pub fn read_events(events: Vec<MdEvent>) -> (ProseIr, FormReport) {
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
        reader.warnings,
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

#[derive(Default)]
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
    warnings: FormReport,
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
                    *style = IrListStyle::Todo(TodoMark::done_if(done));
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
    use crate::engine::prose::md_write;
    use ambre_prose_form::{FormConstruct as ProseConstruct, MarkdownFlavor};

    /// [`read`] over a file of `bytes`, which a test writes as text.
    fn read_bytes(bytes: &[u8]) -> Result<(ProseIr, FormReport), std::str::Utf8Error> {
        std::str::from_utf8(bytes).map(read)
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
                item(IrListStyle::Todo(TodoMark::Done), "done", Vec::new()),
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
        assert!(!report.entries().is_empty());
    }

    /// Written then read, the form comes back: the reader and the writer
    /// agree on every construct both carry.
    #[test]
    fn what_the_writer_writes_the_reader_reads_back() {
        let source = "## Plan\n\nA *b* **c** ~~d~~ `e` [f](https://g.org)\n\n1. one\n   - two\n\n- [ ] three\n\n| h | i |\n| --- | --- |\n| j | k |\n\n```\ncode\n```\n\n---\n";
        let (ir, _) = read_bytes(source.as_bytes()).expect("reads");
        let (written, _) = md_write::write(&ir, MarkdownFlavor::Gfm);
        assert_eq!(written, source);
    }
}
