//! The HTML half of a paste, read through the markup engine the host links
//! in place, as a paste reads it (`features/ambre-spec-prose-clipboard.md`).
//! The walk itself lives in the markup module.

use ambre_core::Validated;
use ambre_prose_form::clip::{ClipBlock, ClipKind, ClipRun, ClipValue};

use crate::message::{CodeLang, HeadingLevel, MarkKey, OrderedStyle};

/// The blocks `html` reads into, as a paste reads them.
fn read(html: &str) -> Vec<ClipBlock> {
    let converter = crate::logic::conversion::host_converter();
    match converter.convert(&ambre_convert::Conversion::ReadHtml {
        html: html.to_owned(),
    }) {
        Ok(ambre_convert::Converted::Pasted(blocks)) => blocks,
        other => panic!("the markup engine answers blocks: {other:?}"),
    }
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
