//! The Markdown writer: the intermediate form to text
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! It reads the form alone (`PROSE-IO-045`). What the flavour has no syntax
//! for is dropped or approximated, and each such construct raises a warning
//! (`PROSE-IO-018`). A table, a to-do item and a footnote are written in GFM
//! alone (`PROSE-IO-061`). The output depends on the form alone, so the same
//! document writes the same bytes (`PROSE-IO-003`).

use std::fmt::Write as _;

use ambre_bus::app::prose::{ConversionFormat, ConversionReport, MarkdownFlavor};

use super::{IrBlock, IrListStyle, IrOrderedStyle, IrRun, IrTableRow, ProseIr, Warnings};

/// The deepest heading Markdown spells.
const MAX_HEADING: u8 = 6;

/// Write `ir` as Markdown in `flavor`, and the report of what it lost.
#[must_use]
pub fn write(ir: &ProseIr, flavor: MarkdownFlavor) -> (String, ConversionReport) {
    let mut writer = Writer {
        gfm: flavor == MarkdownFlavor::Gfm,
        out: String::new(),
        warnings: Warnings::new(ConversionFormat::Markdown),
    };
    if ir.meta.paged {
        writer.warnings.unsupported("page format");
    }
    if ir.meta.bands {
        writer.warnings.unsupported("header and footer");
    }
    writer.blocks(&ir.blocks, "");
    if writer.gfm {
        for note in &ir.footnotes {
            writer.separate();
            let text = writer.inline(&note.runs);
            let _ = writeln!(writer.out, "[^{}]: {text}", note.ordinal);
        }
    }
    (writer.out, writer.warnings.report())
}

struct Writer {
    gfm: bool,
    out: String,
    warnings: Warnings,
}

/// A block's place in a run of list items: the previous item's style and the
/// ordinal the next ordered item takes.
struct ListRun {
    previous: Option<IrListStyle>,
    ordinal: u32,
}

impl Writer {
    /// A blank line before the next block, unless nothing is written yet.
    fn separate(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.out.push('\n');
        }
    }

    /// Write sibling blocks, each line prefixed with `indent`.
    fn blocks(&mut self, blocks: &[IrBlock], indent: &str) {
        let mut run = ListRun {
            previous: None,
            ordinal: 1,
        };
        for block in blocks {
            if let IrBlock::ListItem {
                style,
                runs,
                children,
            } = block
            {
                self.item(&mut run, *style, runs, children, indent);
                continue;
            }
            run.previous = None;
            run.ordinal = 1;
            self.separate();
            self.block(block, indent);
        }
    }

    fn block(&mut self, block: &IrBlock, indent: &str) {
        match block {
            IrBlock::Paragraph { styled, runs } => {
                if *styled {
                    self.warnings.unsupported("paragraph formatting");
                }
                let text = self.inline(runs);
                self.line(indent, &escape_line_start(&text));
            }
            IrBlock::Heading { level, runs } => {
                // The form carries the stored level (`PROSE-FEAT-091`); the
                // syntax spells one to six.
                let spelled = (*level).clamp(1, MAX_HEADING);
                if spelled != *level {
                    self.warnings.unsupported("heading level outside 1 to 6");
                }
                let text = self.inline(runs);
                let hashes = "#".repeat(usize::from(spelled));
                self.line(indent, &format!("{hashes} {text}"));
            }
            IrBlock::CodeBlock { lang, text } => {
                let fence = fence_for(text);
                let info = lang.as_deref().unwrap_or("");
                self.line(indent, &format!("{fence}{info}"));
                for line in text.split('\n') {
                    self.line(indent, line);
                }
                self.line(indent, &fence);
            }
            IrBlock::Table { merged, rows } => self.table(*merged, rows, indent),
            IrBlock::Image { .. } => {
                // The picture lives encrypted in the document and has no
                // address a Markdown file could name.
                self.warnings.unsupported("image");
            }
            IrBlock::PageBreak => {
                self.warnings.unsupported("page break");
                self.line(indent, "---");
            }
            IrBlock::HRule => self.line(indent, "---"),
            IrBlock::Toc => {
                self.warnings.unsupported("table of contents");
                self.line(indent, "<!-- toc -->");
            }
            IrBlock::ListItem { .. } => {}
        }
    }

    fn item(
        &mut self,
        run: &mut ListRun,
        style: IrListStyle,
        runs: &[IrRun],
        children: &[IrBlock],
        indent: &str,
    ) {
        let joins = matches!(
            (run.previous, style),
            (
                Some(IrListStyle::Bullet | IrListStyle::Todo(_)),
                IrListStyle::Bullet | IrListStyle::Todo(_)
            ) | (Some(IrListStyle::Ordered(_)), IrListStyle::Ordered(_))
        );
        if !joins {
            run.ordinal = 1;
            self.separate();
        }
        run.previous = Some(style);
        let marker = match style {
            IrListStyle::Bullet => "- ".to_owned(),
            IrListStyle::Todo(done) if self.gfm => {
                format!("- [{}] ", if done { 'x' } else { ' ' })
            }
            IrListStyle::Todo(_) => {
                self.warnings.unsupported("to-do item");
                "- ".to_owned()
            }
            IrListStyle::Ordered(ordered) => {
                if ordered != IrOrderedStyle::Decimal {
                    self.warnings.unsupported("list numbering style");
                }
                let marker = format!("{}. ", run.ordinal);
                run.ordinal += 1;
                marker
            }
        };
        let text = self.inline(runs);
        self.line(indent, &format!("{marker}{text}"));
        let inner = format!("{indent}{}", " ".repeat(marker.chars().count()));
        let mut child_run = ListRun {
            previous: None,
            ordinal: 1,
        };
        for (index, child) in children.iter().enumerate() {
            if let IrBlock::ListItem {
                style,
                runs,
                children,
            } = child
            {
                // A list opening the item sits on the next line, so the
                // parent item stays tight.
                if index == 0 {
                    child_run.previous = Some(*style);
                }
                self.item(&mut child_run, *style, runs, children, &inner);
                continue;
            }
            child_run.previous = None;
            self.separate();
            self.block(child, &inner);
        }
    }

    fn table(&mut self, merged: bool, rows: &[IrTableRow], indent: &str) {
        if merged {
            self.warnings.unsupported("merged cells");
        }
        for cell in rows.iter().flat_map(|row| &row.cells) {
            if cell.styled {
                self.warnings.unsupported("cell formatting");
            }
        }
        if !self.gfm {
            // No table syntax: each cell lands as a paragraph of its own
            // (`PROSE-IO-061`).
            self.warnings.unsupported("table");
            for row in rows {
                for cell in &row.cells {
                    self.blocks(&cell.blocks, indent);
                }
            }
            return;
        }
        let cols = rows.iter().map(|row| row.cells.len()).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        // A pipe table sizes its columns to the reader's screen.
        self.warnings.unsupported("column widths");
        for (index, row) in rows.iter().enumerate() {
            let mut line = String::from("|");
            for col in 0..cols {
                let text = row
                    .cells
                    .get(col)
                    .map(|cell| self.cell(&cell.blocks))
                    .unwrap_or_default();
                let _ = write!(line, " {text} |");
            }
            self.line(indent, &line);
            if index == 0 {
                let rule = "|".to_owned() + &" --- |".repeat(cols);
                self.line(indent, &rule);
            }
        }
    }

    /// A cell's text on one line. A pipe table holds one line per cell, so a
    /// second block joins the first with a space.
    fn cell(&mut self, blocks: &[IrBlock]) -> String {
        let mut parts = Vec::new();
        for block in blocks {
            match block {
                IrBlock::Paragraph { styled, runs } => {
                    if *styled {
                        self.warnings.unsupported("paragraph formatting");
                    }
                    parts.push(self.inline(runs));
                }
                IrBlock::Heading { runs, .. } => {
                    self.warnings.unsupported("heading in a table cell");
                    parts.push(self.inline(runs));
                }
                IrBlock::ListItem { runs, .. } => {
                    self.warnings.unsupported("list in a table cell");
                    parts.push(self.inline(runs));
                }
                _ => self.warnings.unsupported("block in a table cell"),
            }
        }
        if parts.iter().filter(|part| !part.is_empty()).count() > 1 {
            self.warnings
                .unsupported("several paragraphs in a table cell");
        }
        parts.retain(|part| !part.is_empty());
        parts.join(" ")
    }

    fn line(&mut self, indent: &str, text: &str) {
        if text.is_empty() {
            self.out.push('\n');
            return;
        }
        self.out.push_str(indent);
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// Runs as inline Markdown. The delimiters open and close as a stack, so
    /// a mark that spans several runs is written once.
    fn inline(&mut self, runs: &[IrRun]) -> String {
        let mut out = String::new();
        let mut open: Vec<Delimiter> = Vec::new();
        for run in runs {
            self.lost_marks(run);
            let wanted = self.delimiters(run);
            let keep = open.iter().zip(&wanted).take_while(|(a, b)| a == b).count();
            while open.len() > keep {
                if let Some(delimiter) = open.pop() {
                    out.push_str(&delimiter.close());
                }
            }
            for delimiter in wanted.iter().skip(keep) {
                out.push_str(delimiter.open());
                open.push(delimiter.clone());
            }
            if run.code {
                out.push_str(&code_span(&run.text));
            } else {
                out.push_str(&escape(&run.text));
            }
            if let Some(ordinal) = run.footnote_ref {
                if self.gfm {
                    let _ = write!(out, "[^{ordinal}]");
                } else {
                    self.warnings.unsupported("footnote");
                }
            }
        }
        while let Some(delimiter) = open.pop() {
            out.push_str(&delimiter.close());
        }
        out
    }

    /// The delimiters a run opens, outermost first. The link is outermost,
    /// so a mark change inside a link does not break it.
    fn delimiters(&self, run: &IrRun) -> Vec<Delimiter> {
        let mut wanted = Vec::new();
        if let Some(url) = &run.link {
            wanted.push(Delimiter::Link(url.clone()));
        }
        if run.bold {
            wanted.push(Delimiter::Bold);
        }
        if run.italic {
            wanted.push(Delimiter::Italic);
        }
        if run.strikethrough && self.gfm {
            wanted.push(Delimiter::Strike);
        }
        wanted
    }

    fn lost_marks(&mut self, run: &IrRun) {
        let lost: [(bool, &'static str); 8] = [
            (run.underline, "underline"),
            (run.superscript, "superscript"),
            (run.subscript, "subscript"),
            (run.strikethrough && !self.gfm, "strikethrough"),
            (run.styled, "colour, size or font"),
            (run.comment, "comment"),
            (run.suggestion, "suggestion"),
            (run.cross_ref, "cross reference"),
        ];
        for (on, feature) in lost {
            if on {
                self.warnings.unsupported(feature);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Delimiter {
    Link(String),
    Bold,
    Italic,
    Strike,
}

impl Delimiter {
    fn open(&self) -> &'static str {
        match self {
            Self::Link(_) => "[",
            Self::Bold => "**",
            Self::Italic => "*",
            Self::Strike => "~~",
        }
    }

    fn close(&self) -> String {
        match self {
            Self::Link(url) => format!("]({})", link_target(url)),
            Self::Bold => "**".to_owned(),
            Self::Italic => "*".to_owned(),
            Self::Strike => "~~".to_owned(),
        }
    }
}

/// A link destination, in angle brackets when it holds a space or a paren.
fn link_target(url: &str) -> String {
    if url.contains([' ', '(', ')', '<', '>']) {
        let inner = url.replace('<', "%3C").replace('>', "%3E");
        format!("<{inner}>")
    } else {
        url.to_owned()
    }
}

/// Text with every character Markdown reads as syntax escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '|' | '!' | '&'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A paragraph whose first characters would open a block: a heading, a list
/// item, a quote or a rule.
fn escape_line_start(text: &str) -> String {
    let trimmed = text.trim_start();
    let lead = text.len() - trimmed.len();
    let opens_block = trimmed.starts_with(['#', '-', '+', '=', '>']) || ordinal_prefix(trimmed);
    if opens_block {
        format!("{}\\{}", &text[..lead], trimmed)
    } else {
        text.to_owned()
    }
}

/// Whether a line opens with `1.` or `1)`, an ordered list marker.
fn ordinal_prefix(text: &str) -> bool {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && text[digits..].starts_with(['.', ')'])
}

/// A code span whose delimiter is longer than any backtick run inside.
fn code_span(text: &str) -> String {
    let longest = longest_run(text, '`');
    let ticks = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{ticks}{pad}{text}{pad}{ticks}")
}

/// A code fence longer than any backtick run the code holds.
fn fence_for(text: &str) -> String {
    "`".repeat(longest_run(text, '`').max(2) + 1)
}

fn longest_run(text: &str, c: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in text.chars() {
        if ch == c {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interop::{IrFootnote, IrMeta, IrTableCell};
    use ambre_bus::app::prose::ConversionWarning;

    fn para(runs: Vec<IrRun>) -> IrBlock {
        IrBlock::Paragraph {
            styled: false,
            runs,
        }
    }

    fn ir(blocks: Vec<IrBlock>) -> ProseIr {
        ProseIr {
            blocks,
            ..ProseIr::default()
        }
    }

    fn features(report: &ConversionReport) -> Vec<&'static str> {
        report
            .warnings
            .iter()
            .filter_map(|w| match w {
                ConversionWarning::FeatureNotSupported { feature, .. } => Some(*feature),
                _ => None,
            })
            .collect()
    }

    fn cell(text: &str) -> IrTableCell {
        IrTableCell {
            styled: false,
            blocks: vec![para(vec![IrRun::plain(text)])],
        }
    }

    fn table() -> IrBlock {
        IrBlock::Table {
            merged: false,
            rows: vec![
                IrTableRow {
                    cells: vec![cell("a"), cell("b")],
                },
                IrTableRow {
                    cells: vec![cell("1"), cell("2")],
                },
            ],
        }
    }

    #[test]
    fn blocks_and_marks_write_as_markdown() {
        let bold = IrRun {
            bold: true,
            ..IrRun::plain("bold")
        };
        let both = IrRun {
            bold: true,
            italic: true,
            ..IrRun::plain("both")
        };
        let link = IrRun {
            link: Some("https://example.org".to_owned()),
            ..IrRun::plain("here")
        };
        let doc = ir(vec![
            IrBlock::Heading {
                level: 2,
                runs: vec![IrRun::plain("Title")],
            },
            para(vec![bold, both, IrRun::plain(" and "), link]),
            IrBlock::CodeBlock {
                lang: Some("rust".to_owned()),
                text: "fn main() {}".to_owned(),
            },
        ]);
        let (text, report) = write(&doc, MarkdownFlavor::Gfm);
        assert_eq!(
            text,
            "## Title\n\n**bold*both*** and [here](https://example.org)\n\n```rust\nfn main() {}\n```\n"
        );
        assert!(report.warnings.is_empty(), "{report:?}");
    }

    #[test]
    fn nested_lists_indent_under_their_marker() {
        let item = |style, text: &str, children| IrBlock::ListItem {
            style,
            runs: vec![IrRun::plain(text)],
            children,
        };
        let doc = ir(vec![
            item(
                IrListStyle::Ordered(IrOrderedStyle::Decimal),
                "one",
                vec![item(IrListStyle::Bullet, "inner", Vec::new())],
            ),
            item(
                IrListStyle::Ordered(IrOrderedStyle::Decimal),
                "two",
                Vec::new(),
            ),
            item(IrListStyle::Todo(true), "done", Vec::new()),
        ]);
        let (text, _) = write(&doc, MarkdownFlavor::Gfm);
        assert_eq!(text, "1. one\n   - inner\n2. two\n\n- [x] done\n");
    }

    /// `PROSE-FEAT-091`: the form carries the stored level; the writer spells
    /// the nearest one Markdown has and says so.
    #[test]
    fn a_level_beyond_six_is_spelled_six_and_reported() {
        let doc = ir(vec![IrBlock::Heading {
            level: 9,
            runs: vec![IrRun::plain("deep")],
        }]);
        let (text, report) = write(&doc, MarkdownFlavor::Gfm);
        assert_eq!(text, "###### deep\n");
        assert_eq!(features(&report), ["heading level outside 1 to 6"]);
    }

    /// `PROSE-IO-061`: CommonMark has no table, to-do or footnote, so none is
    /// written, and each is reported.
    #[test]
    fn commonmark_writes_no_table_todo_or_footnote() {
        let mut doc = ir(vec![
            table(),
            IrBlock::ListItem {
                style: IrListStyle::Todo(false),
                runs: vec![IrRun::plain("task")],
                children: Vec::new(),
            },
            para(vec![IrRun {
                footnote_ref: Some(1),
                ..IrRun::plain("claim")
            }]),
        ]);
        doc.footnotes.push(IrFootnote {
            ordinal: 1,
            runs: vec![IrRun::plain("source")],
        });
        let (text, report) = write(&doc, MarkdownFlavor::CommonMark);
        assert!(!text.contains('|'), "{text}");
        assert!(!text.contains("[ ]"), "{text}");
        assert!(!text.contains("[^"), "{text}");
        assert!(!text.contains("source"), "{text}");
        assert_eq!(features(&report), ["table", "to-do item", "footnote"]);
        let (gfm, _) = write(&doc, MarkdownFlavor::Gfm);
        assert!(gfm.contains("| a | b |\n| --- | --- |\n| 1 | 2 |"), "{gfm}");
        assert!(gfm.contains("- [ ] task"), "{gfm}");
        assert!(
            gfm.contains("claim[^1]") && gfm.contains("[^1]: source"),
            "{gfm}"
        );
    }

    /// `PROSE-IO-018`: the page model and every mark Markdown lacks are named.
    #[test]
    fn what_markdown_lacks_is_reported_once_each() {
        let mut doc = ir(vec![
            para(vec![
                IrRun {
                    underline: true,
                    ..IrRun::plain("u")
                },
                IrRun {
                    underline: true,
                    styled: true,
                    comment: true,
                    ..IrRun::plain("v")
                },
            ]),
            IrBlock::PageBreak,
            IrBlock::Toc,
        ]);
        doc.meta = IrMeta {
            title: None,
            paged: true,
            bands: true,
        };
        let (_, report) = write(&doc, MarkdownFlavor::Gfm);
        assert_eq!(
            features(&report),
            [
                "page format",
                "header and footer",
                "underline",
                "colour, size or font",
                "comment",
                "page break",
                "table of contents"
            ]
        );
        assert!(matches!(
            report.warnings[2],
            ConversionWarning::FeatureNotSupported { count: 2, .. }
        ));
    }

    #[test]
    fn syntax_in_text_is_escaped() {
        let doc = ir(vec![
            para(vec![IrRun::plain("# not a heading *nor* [a link]")]),
            para(vec![IrRun::plain("1. not a list")]),
            para(vec![IrRun {
                code: true,
                ..IrRun::plain("a`b")
            }]),
        ]);
        let (text, _) = write(&doc, MarkdownFlavor::Gfm);
        assert_eq!(
            text,
            "\\# not a heading \\*nor\\* \\[a link\\]\n\n\\1. not a list\n\n``a`b``\n"
        );
    }
}
