//! The HTML half of a paste, read through the markup engine the host links
//! in place, as a paste reads it (`features/ambre-spec-prose-clipboard.md`).
//! The walk itself lives in the markup module.

use ambre_core::Validated;
use ambre_prose_form::clip::{
    ClipBlock, ClipCell, ClipCover, ClipItem, ClipKind, ClipRun, ClipTable, ClipValue,
};

use crate::message::{CodeLang, HeadingLevel, MarkKey, OrderedStyle};

/// The items `html` reads into, as a paste reads them.
fn items(html: &str) -> Vec<ClipItem> {
    let converter = crate::logic::conversion::host_converter();
    match converter.convert(&ambre_convert::Conversion::ReadHtml {
        html: html.to_owned(),
    }) {
        Ok(ambre_convert::Converted::Pasted(items)) => items,
        other => panic!("the markup engine answers items: {other:?}"),
    }
}

/// The blocks `html` reads into, each table read as its cells' blocks.
fn read(html: &str) -> Vec<ClipBlock> {
    items(html)
        .into_iter()
        .flat_map(ClipItem::flattened)
        .collect()
}

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
    // Two elements per level: 126 open, just under the module's cap of 128.
    let shallow = deep("<div><span>", "</span></div>", 63);
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
    assert_eq!(
        blocks[1].kind,
        ClipKind::Code(Validated::<CodeLang>::parse("rust").ok())
    );
    assert_eq!(blocks[2].kind, ClipKind::Ordered(OrderedStyle::LowerRoman));
}

/// `PROSE-UI-219`: a colour spelled in any form lands in the one spelling
/// the document stores. A mark value the registry refuses is dropped, and
/// a size past the cap lands at it.
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
    // A size past the cap lands at the cap (`CANVAS-088`).
    assert_eq!(run(2), &vec![(MarkKey::from("fs"), text("1638"))]);
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

/// The texts of the cells of the one table among `items`, row by row. A
/// covered slot reads as `<` or `^`, the way it points.
fn table_of(items: &[ClipItem]) -> Vec<Vec<String>> {
    let tables: Vec<&ClipTable> = items
        .iter()
        .filter_map(|item| match item {
            ClipItem::Table(table) => Some(table),
            ClipItem::Block(_) => None,
        })
        .collect();
    assert_eq!(tables.len(), 1, "one table: {items:?}");
    tables[0]
        .rows
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| match cell {
                    ClipCell::Own(blocks) => texts(blocks).join("/"),
                    ClipCell::Covered(ClipCover::Left) => "<".to_owned(),
                    ClipCell::Covered(ClipCover::Above) => "^".to_owned(),
                })
                .collect()
        })
        .collect()
}

/// `PROSE-UI-225`: a web page's table reads on its grid. A header cell is a
/// cell, and a span covers its slots. A row span of zero reaches the last
/// row. A caption lands before the table, and a
/// missing cell is an empty one.
#[test]
fn a_web_table_reads_on_its_grid_with_its_merged_cells() {
    let html = concat!(
        "<p>before</p><table><caption>Sales</caption>",
        "<tr><th colspan=\"2\">Region</th><th rowspan=\"0\">Note</th></tr>",
        "<tr><td rowspan=\"2\">North</td><td>12</td></tr>",
        "<tr><td>14</td></tr><tr><td>South</td></tr></table><p>after</p>",
    );
    let items = items(html);
    let blocks: Vec<String> = items
        .iter()
        .map(|item| match item {
            ClipItem::Block(block) => texts(std::slice::from_ref(block)).join(""),
            ClipItem::Table(_) => "<table>".to_owned(),
        })
        .collect();
    assert_eq!(blocks, ["before", "Sales", "<table>", "after"]);
    assert_eq!(
        table_of(&items),
        [
            ["Region", "<", "Note"],
            ["North", "12", "^"],
            ["^", "14", "^"],
            ["South", "", "^"],
        ]
    );
}

/// `PROSE-UI-225`: Word writes each cell as paragraphs of its own, so a
/// cell keeps its blocks, its list item and its marks.
#[test]
fn a_word_table_keeps_the_blocks_of_each_cell() {
    let html = concat!(
        "<table class=MsoTableGrid border=1 cellspacing=0 cellpadding=0>",
        "<tr style='mso-yfti-irow:0'><td width=302 valign=top style='width:226.55pt'>",
        "<p class=MsoNormal><b>Nom<o:p></o:p></b></p></td>",
        "<td width=302 valign=top><p class=MsoNormal>Valeur<o:p></o:p></p></td></tr>",
        "<tr><td width=604 colspan=2 valign=top>",
        "<p class=MsoNormal>Ligne une<o:p></o:p></p>",
        "<p class=MsoListParagraphCxSpFirst style='mso-list:l0 level1 lfo1'>",
        "<![if !supportLists]><span style='mso-list:Ignore'>&#183;</span><![endif]>",
        "Point<o:p></o:p></p></td></tr></table>",
    );
    let items = items(html);
    assert_eq!(
        table_of(&items),
        [["Nom", "Valeur"], ["Ligne une/Point", "<"]]
    );
    let ClipItem::Table(table) = &items[0] else {
        panic!("a table");
    };
    let ClipCell::Own(first) = &table.rows[0].cells[0] else {
        panic!("a cell");
    };
    assert_eq!(keys(&first[0].runs[0]), ["b"]);
    let ClipCell::Own(merged) = &table.rows[1].cells[0] else {
        panic!("a cell");
    };
    assert_eq!(kinds(merged), [ClipKind::Paragraph, ClipKind::Bullet]);
}

/// `PROSE-UI-225`: a range copied from Google Sheets or Excel reads as its
/// grid, its style sheet and its fragment markers dropped.
#[test]
fn a_sheet_range_reads_as_its_grid() {
    let sheets = concat!(
        "<google-sheets-html-origin><style type=\"text/css\"><!--td {border: 1px solid #cccccc;}--></style>",
        "<table xmlns=\"http://www.w3.org/1999/xhtml\" cellspacing=\"0\" cellpadding=\"0\" dir=\"ltr\" border=\"1\">",
        "<colgroup><col width=\"100\"/><col width=\"100\"/></colgroup><tbody>",
        "<tr style=\"height:21px;\"><td style=\"overflow:hidden;padding:2px 3px 2px 3px;\">Item</td>",
        "<td style=\"overflow:hidden;padding:2px 3px 2px 3px;text-align:right;\" data-sheets-value=\"{&quot;1&quot;:3,&quot;3&quot;:4}\">4</td></tr>",
        "<tr style=\"height:21px;\"><td colspan=\"2\" rowspan=\"1\" style=\"font-weight:bold;\">Total</td></tr>",
        "</tbody></table>",
    );
    assert_eq!(table_of(&items(sheets)), [["Item", "4"], ["Total", "<"]]);
    let excel = concat!(
        "<html><body><!--StartFragment--><table border=0 cellpadding=0 cellspacing=0 width=128>",
        "<col width=64 span=2><tr height=20><td height=20 class=xl65 width=64>1</td>",
        "<td class=xl65 width=64>2</td></tr><tr height=20><td height=20 class=xl65>3</td>",
        "<td class=xl65></td></tr><!--EndFragment--></table></body></html>",
    );
    assert_eq!(table_of(&items(excel)), [["1", "2"], ["3", ""]]);
}

/// `PROSE-UI-240`: a table inside a cell reads as one block per cell of its
/// own. A table that reaches past 64 columns reads as one block per cell,
/// so a crafted span never grows the grid.
#[test]
fn a_nested_or_too_wide_table_reads_as_its_cells() {
    let nested = "<table><tr><td>outer</td><td><table><tr><td>a</td><td>b</td></tr></table></td></tr></table>";
    assert_eq!(table_of(&items(nested)), [["outer", "a/b"]]);
    for span in ["65", "1000", "99999999999999"] {
        let wide = format!(
            "<table><tr><td>x</td><td colspan=\"{span}\">y</td></tr><tr><td>z</td></tr></table>"
        );
        let items = items(&wide);
        assert!(
            items.iter().all(|item| matches!(item, ClipItem::Block(_))),
            "{span}"
        );
        assert_eq!(texts(&read(&wide)), ["x", "y", "z"]);
    }
    let edge = "<table><tr><td colspan=\"63\">a</td><td>b</td></tr></table>";
    assert_eq!(table_of(&items(edge))[0].len(), 64);
}

/// `PROSE-UI-225`: spans that would overlap stop at the slot another cell
/// holds, so each slot is one cell's, and the grid stays a rectangle.
#[test]
fn overlapping_spans_stop_at_the_cell_they_meet() {
    let html = concat!(
        "<table><tr><td>a</td><td rowspan=\"2\">b</td></tr>",
        "<tr><td colspan=\"3\">c</td></tr></table>",
    );
    assert_eq!(table_of(&items(html)), [["a", "b"], ["c", "^"]]);
}
