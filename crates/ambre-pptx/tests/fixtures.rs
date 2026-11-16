//! The reader against real PowerPoint files: one python-pptx wrote on the
//! default template of PowerPoint, and LibreOffice's save of the same deck.
//! `tests/fixtures/generated/generate.py` writes both, and
//! `fixtures/AUTHORED.md` says so.
#![cfg(feature = "codec")]
#![expect(
    clippy::panic,
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_core::{PrintLength, Pt, Rgba, Slant, Weight};
use ambre_pptx::model::{Geometry, ImportedDeck, Item, PictureFormat, Shape, Site, SlideSize};
use ambre_pptx::report::{DeckConstruct, DeckOutcome};

/// The deck the fixture `name` reads into.
fn read(name: &str) -> ImportedDeck {
    let path = format!(
        "{}/tests/fixtures/generated/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).expect("the fixture is committed");
    ambre_pptx::read(&bytes).expect("the fixture reads")
}

/// Both writers of the fixture.
const FILES: [&str; 2] = ["features.pptx", "libreoffice.pptx"];

/// The shape of `items` named `name`.
fn shape<'a>(items: &'a [Item], name: &str) -> &'a Shape {
    items
        .iter()
        .find_map(|item| match item {
            Item::Shape(shape) if shape.name == name => Some(shape),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no shape named {name}"))
}

/// The shape of `items` whose text opens on `text`.
fn text_shape<'a>(items: &'a [Item], text: &str) -> &'a Shape {
    items
        .iter()
        .find_map(|item| match item {
            Item::Shape(shape)
                if shape
                    .text
                    .as_ref()
                    .is_some_and(|t| t.text.starts_with(text)) =>
            {
                Some(shape)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text {text}"))
}

fn near(a: PrintLength<Pt>, b: f32) -> bool {
    (a.get() - b).abs() < 0.1
}

#[test]
fn every_slide_reads_in_deck_order_with_its_size() {
    for file in FILES {
        let deck = read(file).deck;
        assert_eq!(deck.slides.len(), 3, "{file}");
        assert_eq!(
            deck.size,
            SlideSize {
                width: PrintLength::<Pt>::new(720.0),
                height: PrintLength::<Pt>::new(540.0),
            },
            "{file}"
        );
        let names: Vec<&str> = deck.slides.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Quarterly plan", "Three shapes", ""], "{file}");
    }
}

#[test]
fn speaker_notes_reach_their_slide_line_by_line() {
    for file in FILES {
        let deck = read(file).deck;
        let notes: Vec<&str> = deck.slides.iter().map(|s| s.notes.as_str()).collect();
        assert_eq!(
            notes,
            [
                "Open with the plan.",
                "Point at each shape.\nThen move on.",
                ""
            ],
            "{file}"
        );
    }
}

/// The title placeholder of the slide carries no box of its own. It takes
/// the box of its layout, and the size and face of its master.
#[test]
fn a_placeholder_takes_the_box_and_the_style_its_layout_and_master_give_it() {
    for file in FILES {
        let deck = read(file).deck;
        let title = text_shape(&deck.slides[1].items, "Three shapes");
        assert!(near(title.at.origin.x, 36.0), "{file}: {:?}", title.at);
        assert!(near(title.at.origin.y, 21.6), "{file}: {:?}", title.at);
        assert!(near(title.at.width, 648.0), "{file}: {:?}", title.at);
        let text = title.text.as_ref().expect("a title holds its text");
        assert_eq!(text.size, PrintLength::<Pt>::new(44.0), "{file}");
        assert_eq!(text.typeface, "Calibri", "{file}");
        assert_eq!(text.colour, Rgba::from_bits(0x0000_00FF), "{file}");
        let body = text_shape(&deck.slides[1].items, "First point");
        assert_eq!(
            body.text.as_ref().map(|t| t.size),
            Some(PrintLength::<Pt>::new(32.0)),
            "{file}"
        );
    }
}

/// A shape the master draws lands on every slide, under the slide's own.
#[test]
fn a_shape_of_the_master_lands_on_every_slide() {
    for file in FILES {
        let deck = read(file).deck;
        for slide in &deck.slides {
            let mark = shape(&slide.items, "Master mark");
            assert_eq!(mark.fill, Some(Rgba::from_bits(0x1234_56FF)), "{file}");
            assert!(near(mark.at.origin.x, 684.0), "{file}");
        }
    }
}

#[test]
fn a_slide_background_reaches_the_slide() {
    for file in FILES {
        let deck = read(file).deck;
        assert_eq!(
            deck.slides[2].background,
            Some(Rgba::from_bits(0xF0F0_E0FF)),
            "{file}"
        );
        assert_eq!(
            deck.slides[0].background,
            Some(Rgba::from_bits(0xFFFF_FFFF)),
            "{file}: the master's theme background"
        );
    }
}

#[test]
fn shapes_keep_their_outline_their_fill_and_their_style_colour() {
    let deck = read("features.pptx").deck;
    let items = &deck.slides[1].items;
    let disc = shape(items, "Disc");
    assert_eq!(disc.geometry, Geometry::Ellipse);
    assert_eq!(disc.fill, Some(Rgba::from_bits(0x0080_00FF)));
    // The box takes the accent colour of the theme through its style.
    let square = shape(items, "Box");
    assert_eq!(square.geometry, Geometry::Rect);
    assert_eq!(square.fill, Some(Rgba::from_bits(0x4F81_BDFF)));
    assert!(matches!(
        shape(items, "Rounded Rectangle 5").geometry,
        Geometry::RoundRect { .. }
    ));
}

#[test]
fn a_text_box_keeps_its_own_size_ink_and_emphasis() {
    for file in FILES {
        let deck = read(file).deck;
        let note = text_shape(&deck.slides[1].items, "A note in red");
        let text = note.text.as_ref().expect("a text box holds its text");
        assert_eq!(text.size, PrintLength::<Pt>::new(24.0), "{file}");
        assert_eq!(text.colour, Rgba::from_bits(0xC000_00FF), "{file}");
        assert_eq!(text.slant, Slant::Italic, "{file}");
        assert_eq!(text.weight, Weight::Regular, "{file}");
    }
}

#[test]
fn a_connector_holds_on_to_the_shapes_it_joins() {
    for file in FILES {
        let deck = read(file).deck;
        let items = &deck.slides[1].items;
        let connector = items
            .iter()
            .find_map(|item| match item {
                Item::Connector(c) => Some(c),
                _ => None,
            })
            .expect("the slide holds a connector");
        let named = |end: &ambre_pptx::model::ConnectorEnd| {
            end.attached.map(|a| match items.get(a.item.to_usize()) {
                Some(Item::Shape(s)) => (s.name.clone(), a.site),
                _ => (String::new(), a.site),
            })
        };
        assert_eq!(
            named(&connector.start),
            Some(("Box".to_owned(), Site::Right)),
            "{file}"
        );
        assert_eq!(
            named(&connector.end),
            Some(("Disc".to_owned(), Site::Left)),
            "{file}"
        );
    }
}

#[test]
fn png_and_jpeg_pictures_land_and_a_video_lands_as_its_poster() {
    for file in FILES {
        let deck = read(file).deck;
        let formats: Vec<PictureFormat> = deck.slides[2]
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Picture(p) => Some(p.format),
                _ => None,
            })
            .collect();
        assert_eq!(
            formats,
            [PictureFormat::Png, PictureFormat::Jpeg, PictureFormat::Png],
            "{file}"
        );
    }
}

#[test]
fn a_group_lands_ungrouped_with_its_members_in_place() {
    for file in FILES {
        let deck = read(file).deck;
        let items = &deck.slides[2].items;
        let rect = shape(items, "Rectangle 4");
        assert!(near(rect.at.origin.x, 36.0), "{file}: {:?}", rect.at);
        assert!(near(rect.at.origin.y, 180.0), "{file}: {:?}", rect.at);
    }
}

/// What the deck cannot hold is named in the report: never dropped in
/// silence.
#[test]
fn what_the_deck_cannot_hold_is_named_in_the_report() {
    let report = read("features.pptx").report;
    let count = |construct, outcome| report.count(construct, outcome).get();
    assert_eq!(
        count(DeckConstruct::Transition, DeckOutcome::Approximated),
        1
    );
    assert!(count(DeckConstruct::Animation, DeckOutcome::Dropped) >= 1);
    assert_eq!(count(DeckConstruct::SmartArt, DeckOutcome::Dropped), 1);
    assert_eq!(count(DeckConstruct::Chart, DeckOutcome::Dropped), 1);
    assert_eq!(count(DeckConstruct::Table, DeckOutcome::Dropped), 1);
    assert_eq!(count(DeckConstruct::Video, DeckOutcome::Approximated), 1);
    assert_eq!(count(DeckConstruct::Group, DeckOutcome::Approximated), 1);
    assert_eq!(count(DeckConstruct::RichText, DeckOutcome::Approximated), 1);
    assert_eq!(
        count(DeckConstruct::PresetShape, DeckOutcome::Approximated),
        1
    );
    let resaved = read("libreoffice.pptx").report;
    for construct in [
        DeckConstruct::Chart,
        DeckConstruct::Table,
        DeckConstruct::Video,
    ] {
        assert!(
            resaved.entries().iter().any(|e| e.construct == construct),
            "{construct:?}"
        );
    }
}
