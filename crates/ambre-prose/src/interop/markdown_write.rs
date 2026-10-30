//! The Markdown writer: the intermediate form to text
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! It reads the form alone (`PROSE-IO-045`). What the flavour has no syntax
//! for is dropped or approximated, and each such construct raises a warning
//! (`PROSE-IO-018`). A table, a to-do item and a footnote are written in GFM
//! alone (`PROSE-IO-061`). The output depends on the form alone, so the same
//! document writes the same bytes (`PROSE-IO-003`).

use std::fmt::Write as _;

use ambre_bus::app::prose::{
    ConversionFormat, ConversionReport, MarkdownFlavor, UnsupportedFeature,
};

use crate::numbering::ListOrdinal;

use ambre_core::{Slant, Strikethrough, Weight};

use super::{
    Banding, IrBlock, IrListStyle, IrOrderedStyle, IrRun, IrTableRow, Merging, OwnStyle, Paging,
    ProseIr, Warnings,
};
use crate::mark::InlineCode;

/// Write `ir` as Markdown in `flavor`, and the report of what it lost.
#[must_use]
pub fn write(ir: &ProseIr, flavor: MarkdownFlavor) -> (String, ConversionReport) {
    let mut writer = Writer {
        flavor,
        out: String::new(),
        warnings: Warnings::new(ConversionFormat::Markdown),
    };
    if ir.meta.paged == Paging::Paged {
        writer.warnings.unsupported(UnsupportedFeature::PageFormat);
    }
    if ir.meta.bands == Banding::Banded {
        writer
            .warnings
            .unsupported(UnsupportedFeature::HeaderAndFooter);
    }
    writer.blocks(&ir.blocks, &Indent::default());
    if writer.flavor == MarkdownFlavor::Gfm {
        for note in &ir.footnotes {
            writer.separate();
            let text = writer.inline(&note.runs);
            let _ = writeln!(writer.out, "[^{}]: {text}", note.ordinal);
        }
    }
    (writer.out, writer.warnings.report())
}

struct Writer {
    flavor: MarkdownFlavor,
    out: String,
    warnings: Warnings,
}

/// A block's place in a run of list items: the previous item's style and the
/// ordinal the next ordered item takes.
struct ListRun {
    previous: Option<IrListStyle>,
    ordinal: ListOrdinal,
}

/// The spaces every line of a nested block starts with: the width of each
/// list marker it sits under.
#[derive(Default)]
struct Indent(String);

impl Indent {
    /// The indent of the blocks of an item whose line starts with `marker`.
    fn under(&self, marker: &str) -> Self {
        Self(format!("{}{}", self.0, " ".repeat(marker.chars().count())))
    }
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
    fn blocks(&mut self, blocks: &[IrBlock], indent: &Indent) {
        let mut run = ListRun {
            previous: None,
            ordinal: ListOrdinal::FIRST,
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
            run.ordinal = ListOrdinal::FIRST;
            self.separate();
            self.block(block, indent);
        }
    }

    fn block(&mut self, block: &IrBlock, indent: &Indent) {
        match block {
            IrBlock::Paragraph { styled, runs } => {
                if *styled == OwnStyle::Styled {
                    self.warnings
                        .unsupported(UnsupportedFeature::ParagraphFormatting);
                }
                let text = self.inline(runs);
                self.line(indent, &escape_line_start(&text));
            }
            IrBlock::Heading { level, runs } => {
                // The form carries the stored level (`PROSE-FEAT-091`); the
                // syntax spells one to six.
                let spelled = level.clamped();
                if spelled.get() != level.get() {
                    self.warnings.unsupported(UnsupportedFeature::HeadingLevel);
                }
                let text = self.inline(runs);
                let hashes = "#".repeat(usize::from(spelled.get()));
                self.line(indent, &format!("{hashes} {text}"));
            }
            IrBlock::CodeBlock { lang, text } => {
                let fence = fence_for(text);
                let info = ambre_bus::app::prose::CodeLang::stored(lang.as_ref());
                self.line(indent, &format!("{fence}{info}"));
                for line in text.split('\n') {
                    self.line(indent, line);
                }
                self.line(indent, &fence);
            }
            IrBlock::Table { merged, rows } => {
                if *merged == Merging::Merged {
                    self.warnings.unsupported(UnsupportedFeature::MergedCells);
                }
                self.table(rows, indent);
            }
            IrBlock::Image { .. } => {
                // The picture lives encrypted in the document and has no
                // address a Markdown file could name.
                self.warnings.unsupported(UnsupportedFeature::Image);
            }
            IrBlock::PageBreak => {
                self.warnings.unsupported(UnsupportedFeature::PageBreak);
                self.line(indent, "---");
            }
            IrBlock::HRule => self.line(indent, "---"),
            IrBlock::Toc => {
                self.warnings
                    .unsupported(UnsupportedFeature::TableOfContents);
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
        indent: &Indent,
    ) {
        let joins = matches!(
            (run.previous, style),
            (
                Some(IrListStyle::Bullet | IrListStyle::Todo(_)),
                IrListStyle::Bullet | IrListStyle::Todo(_)
            ) | (Some(IrListStyle::Ordered(_)), IrListStyle::Ordered(_))
        );
        if !joins {
            run.ordinal = ListOrdinal::FIRST;
            self.separate();
        }
        run.previous = Some(style);
        let marker = match style {
            IrListStyle::Bullet => "- ".to_owned(),
            IrListStyle::Todo(done) if self.flavor == MarkdownFlavor::Gfm => {
                format!("- [{}] ", if done.is_done() { 'x' } else { ' ' })
            }
            IrListStyle::Todo(_) => {
                self.warnings.unsupported(UnsupportedFeature::TodoItem);
                "- ".to_owned()
            }
            IrListStyle::Ordered(ordered) => {
                if ordered != IrOrderedStyle::Decimal {
                    self.warnings.unsupported(UnsupportedFeature::ListNumbering);
                }
                let marker = format!("{}. ", run.ordinal);
                run.ordinal = run.ordinal.next();
                marker
            }
        };
        let text = self.inline(runs);
        self.line(indent, &format!("{marker}{text}"));
        let inner = indent.under(&marker);
        let mut child_run = ListRun {
            previous: None,
            ordinal: ListOrdinal::FIRST,
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

    fn table(&mut self, rows: &[IrTableRow], indent: &Indent) {
        for cell in rows.iter().flat_map(|row| &row.cells) {
            if cell.styled == OwnStyle::Styled {
                self.warnings
                    .unsupported(UnsupportedFeature::CellFormatting);
            }
        }
        if self.flavor != MarkdownFlavor::Gfm {
            // No table syntax: each cell lands as a paragraph of its own
            // (`PROSE-IO-061`).
            self.warnings.unsupported(UnsupportedFeature::Table);
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
        self.warnings.unsupported(UnsupportedFeature::ColumnWidths);
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
                    if *styled == OwnStyle::Styled {
                        self.warnings
                            .unsupported(UnsupportedFeature::ParagraphFormatting);
                    }
                    parts.push(self.inline(runs));
                }
                IrBlock::Heading { runs, .. } => {
                    self.warnings.unsupported(UnsupportedFeature::HeadingInCell);
                    parts.push(self.inline(runs));
                }
                IrBlock::ListItem { runs, .. } => {
                    self.warnings.unsupported(UnsupportedFeature::ListInCell);
                    parts.push(self.inline(runs));
                }
                _ => self.warnings.unsupported(UnsupportedFeature::BlockInCell),
            }
        }
        if parts.iter().filter(|part| !part.is_empty()).count() > 1 {
            self.warnings
                .unsupported(UnsupportedFeature::ParagraphsInCell);
        }
        parts.retain(|part| !part.is_empty());
        parts.join(" ")
    }

    fn line(&mut self, indent: &Indent, text: &str) {
        if text.is_empty() {
            self.out.push('\n');
            return;
        }
        self.out.push_str(&indent.0);
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
            if run.code == InlineCode::Code {
                out.push_str(&code_span(&run.text));
            } else {
                out.push_str(&escape(&run.text));
            }
            if let Some(ordinal) = run.footnote_ref {
                if self.flavor == MarkdownFlavor::Gfm {
                    let _ = write!(out, "[^{ordinal}]");
                } else {
                    self.warnings.unsupported(UnsupportedFeature::Footnote);
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
        // A peer writes a target straight into the document, so one that
        // could run code is exported as its text alone (`PROSE-IO-067`).
        if let Some(url) = run.link.as_ref().filter(|url| crate::link::openable(url)) {
            wanted.push(Delimiter::Link(url.clone()));
        }
        if run.bold == Weight::Bold {
            wanted.push(Delimiter::Bold);
        }
        if run.italic == Slant::Italic {
            wanted.push(Delimiter::Italic);
        }
        if run.strikethrough == Strikethrough::Struck && self.flavor == MarkdownFlavor::Gfm {
            wanted.push(Delimiter::Strike);
        }
        wanted
    }

    fn lost_marks(&mut self, run: &IrRun) {
        let lost: [(bool, UnsupportedFeature); 8] = [
            (run.underline.as_bool(), UnsupportedFeature::Underline),
            (run.superscript.as_bool(), UnsupportedFeature::Superscript),
            (run.subscript.as_bool(), UnsupportedFeature::Subscript),
            (
                run.strikethrough.as_bool() && self.flavor != MarkdownFlavor::Gfm,
                UnsupportedFeature::Strikethrough,
            ),
            (run.styled.as_bool(), UnsupportedFeature::Styling),
            (run.comment.as_bool(), UnsupportedFeature::Comment),
            (run.suggestion.as_bool(), UnsupportedFeature::Suggestion),
            (run.cross_ref.as_bool(), UnsupportedFeature::CrossReference),
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
    Link(ambre_core::LinkUrl),
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

/// A link destination, in angle brackets when it holds a paren.
///
/// A space, a control character, an angle bracket and a backslash are
/// percent-encoded. A stored target thus cannot end the destination, the
/// paragraph or the block it sits in (`PROSE-IO-068`).
fn link_target(url: &ambre_core::LinkUrl) -> String {
    let mut inner = String::with_capacity(url.as_str().len());
    for c in url.as_str().chars() {
        if c <= ' ' || matches!(c, '\u{7f}' | '<' | '>' | '\\') {
            let _ = write!(inner, "%{:02X}", u32::from(c));
        } else {
            inner.push(c);
        }
    }
    if inner.contains(['(', ')']) {
        format!("<{inner}>")
    } else {
        inner
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
    use ambre_bus::app::prose::{ConversionWarning, FootnoteOrdinal};

    fn para(runs: Vec<IrRun>) -> IrBlock {
        IrBlock::Paragraph {
            styled: OwnStyle::Plain,
            runs,
        }
    }

    fn ir(blocks: Vec<IrBlock>) -> ProseIr {
        ProseIr {
            blocks,
            ..ProseIr::default()
        }
    }

    /// `PROSE-IO-067`: a target that could run code is exported as its text
    /// alone, and a web or mail target as a link.
    #[test]
    fn a_link_that_could_run_code_exports_as_its_text() {
        let link = |url: &str, text: &str| IrRun {
            link: Some(url.into()),
            ..IrRun::plain(text)
        };
        let doc = ir(vec![para(vec![
            link("javascript:alert(1)", "run"),
            IrRun::plain(" "),
            link(" data:text/html,x", "data"),
            IrRun::plain(" "),
            link("mailto:a@example.org", "mail"),
        ])]);
        let (text, _) = write(&doc, MarkdownFlavor::Gfm);
        assert_eq!(text, "run data [mail](mailto:a@example.org)\n");
    }

    /// `PROSE-IO-068`: a target the gate lets through still cannot end the
    /// destination, the paragraph or the block it sits in.
    #[test]
    fn a_link_target_cannot_break_out_of_its_destination() {
        let cases = [
            (
                "https://a\n\n[x](javascript:alert(1))",
                "[t](<https://a%0A%0A[x](javascript:alert(1))>)\n",
            ),
            ("https://a\n\n# H", "[t](https://a%0A%0A#%20H)\n"),
            ("https://a/)b", "[t](<https://a/)b>)\n"),
            ("https://a/<b>", "[t](https://a/%3Cb%3E)\n"),
            ("https://a/\\", "[t](https://a/%5C)\n"),
            ("https://a/\u{7f}\tb", "[t](https://a/%7F%09b)\n"),
        ];
        for (url, expected) in cases {
            let doc = ir(vec![para(vec![IrRun {
                link: Some(url.into()),
                ..IrRun::plain("t")
            }])]);
            let (text, _) = write(&doc, MarkdownFlavor::Gfm);
            assert_eq!(text, expected, "{url:?}");
        }
    }

    fn features(report: &ConversionReport) -> Vec<&'static str> {
        report
            .warnings
            .iter()
            .filter_map(|w| match w {
                ConversionWarning::FeatureNotSupported { feature, .. } => Some(feature.label()),
                _ => None,
            })
            .collect()
    }

    fn cell(text: &str) -> IrTableCell {
        IrTableCell {
            styled: OwnStyle::Plain,
            blocks: vec![para(vec![IrRun::plain(text)])],
        }
    }

    fn table() -> IrBlock {
        IrBlock::Table {
            merged: Merging::Unmerged,
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
            bold: Weight::Bold,
            ..IrRun::plain("bold")
        };
        let both = IrRun {
            bold: Weight::Bold,
            italic: Slant::Italic,
            ..IrRun::plain("both")
        };
        let link = IrRun {
            link: Some("https://example.org".into()),
            ..IrRun::plain("here")
        };
        let doc = ir(vec![
            IrBlock::Heading {
                level: crate::doc::StoredLevel::of(ambre_bus::app::prose::HeadingLevel::H2),
                runs: vec![IrRun::plain("Title")],
            },
            para(vec![bold, both, IrRun::plain(" and "), link]),
            IrBlock::CodeBlock {
                lang: ambre_bus::app::prose::CodeLang::new("rust"),
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
            item(
                IrListStyle::Todo(crate::events::TodoMark::Done),
                "done",
                Vec::new(),
            ),
        ]);
        let (text, _) = write(&doc, MarkdownFlavor::Gfm);
        assert_eq!(text, "1. one\n   - inner\n2. two\n\n- [x] done\n");
    }

    /// `PROSE-FEAT-091`: the form carries the stored level; the writer spells
    /// the nearest one Markdown has and says so.
    #[test]
    fn a_level_beyond_six_is_spelled_six_and_reported() {
        let doc = ir(vec![IrBlock::Heading {
            level: crate::doc::StoredLevel::from_stored(9),
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
                style: IrListStyle::Todo(crate::events::TodoMark::Open),
                runs: vec![IrRun::plain("task")],
                children: Vec::new(),
            },
            para(vec![IrRun {
                footnote_ref: Some(FootnoteOrdinal::FIRST),
                ..IrRun::plain("claim")
            }]),
        ]);
        doc.footnotes.push(IrFootnote {
            ordinal: FootnoteOrdinal::FIRST,
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
                    underline: ambre_core::Underline::Underlined,
                    ..IrRun::plain("u")
                },
                IrRun {
                    underline: ambre_core::Underline::Underlined,
                    styled: OwnStyle::Styled,
                    comment: crate::interop::Commenting::Commented,
                    ..IrRun::plain("v")
                },
            ]),
            IrBlock::PageBreak,
            IrBlock::Toc,
        ]);
        doc.meta = IrMeta {
            title: None,
            paged: Paging::Paged,
            bands: Banding::Banded,
        };
        let (_, report) = write(&doc, MarkdownFlavor::Gfm);
        assert_eq!(
            features(&report),
            [
                "page format",
                "header and footer",
                UnsupportedFeature::Underline.label(),
                UnsupportedFeature::Styling.label(),
                UnsupportedFeature::Comment.label(),
                "page break",
                "table of contents"
            ]
        );
        assert!(matches!(
            report.warnings[2],
            ConversionWarning::FeatureNotSupported { count, .. } if count.get() == 2
        ));
    }

    #[test]
    fn syntax_in_text_is_escaped() {
        let doc = ir(vec![
            para(vec![IrRun::plain("# not a heading *nor* [a link]")]),
            para(vec![IrRun::plain("1. not a list")]),
            para(vec![IrRun {
                code: InlineCode::Code,
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
