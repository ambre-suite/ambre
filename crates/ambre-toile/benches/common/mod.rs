//! The deck the PDF export benches record: the "Toile PDF export, 50 slides"
//! row of `ambre-spec-budgets.md section 1`, and the slice row beside it.
//!
//! Each slide is a frame that holds a title, a sticky and four shapes. A
//! page therefore shapes text and fills paths as a working deck does.
//! The deck holds no picture: a page draws a picture from its 256 px
//! thumbnail today, and the full-resolution story changes what that costs.

use ambre_draw::FontContext;
use ambre_toile::message::{InitContent, ShapeKind, ToileCommand, ToileEvent};
use ambre_toile::{ElementKind, ToileState};
use kurbo::{Point, Size};

/// The slides of the deck the budget row names.
pub const SLIDES: usize = 50;

/// The body of each sticky: about 300 characters of words.
const BODY: &str = "Ambre records each slide of the deck through the screen walk, \
    one page per pass of the Logic thread, so an edit typed meanwhile lands at once. \
    The recording owns the pages it derived at the request and the pictures it \
    decoded, and the PDF module writes the whole draw list once the last page is in.";

/// Create `kind` at `at`, sized `size`, holding `content`, and name it.
fn create(
    st: &mut ToileState,
    kind: ElementKind,
    at: Point,
    size: Size,
    content: InitContent,
) -> ambre_core::id::Id<ambre_core::id::Element> {
    st.handle_command(ToileCommand::CreateElement {
        kind,
        at,
        size,
        content,
        fill: None,
        correlate: None,
    })
    .display
    .into_iter()
    .find_map(|e| match e {
        ToileEvent::ElementCreated { id, .. } => Some(id),
        _ => None,
    })
    .unwrap_or_else(|| panic!("a create names its element"))
}

/// A deck of `slides` slides side by side. The peer is pinned, so the ids
/// and the count repeat from run to run.
pub fn deck(slides: usize) -> ToileState {
    let mut st = ToileState::with_peer(ambre_core::PeerId::new(1));
    let mut order = Vec::with_capacity(slides);
    for i in 0..slides {
        let x = i as f64 * 1_400.0;
        let frame = create(
            &mut st,
            ElementKind::Frame,
            Point::new(x, 0.0),
            Size::new(1_280.0, 720.0),
            InitContent::None,
        );
        order.push(
            st.frame(&frame)
                .unwrap_or_else(|| panic!("a frame is a frame")),
        );
        create(
            &mut st,
            ElementKind::Text,
            Point::new(x + 80.0, 60.0),
            Size::new(900.0, 80.0),
            InitContent::Text(format!("Slide {} of the quarterly review", i + 1)),
        );
        create(
            &mut st,
            ElementKind::Sticky,
            Point::new(x + 80.0, 200.0),
            Size::new(520.0, 420.0),
            InitContent::Text(BODY.to_owned()),
        );
        for (k, shape) in [
            ShapeKind::Rect,
            ShapeKind::Ellipse,
            ShapeKind::Rect,
            ShapeKind::Ellipse,
        ]
        .into_iter()
        .enumerate()
        {
            create(
                &mut st,
                ElementKind::Shape(shape),
                Point::new(
                    x + 700.0 + (k % 2) as f64 * 260.0,
                    200.0 + (k / 2) as f64 * 220.0,
                ),
                Size::new(220.0, 180.0),
                InitContent::None,
            );
        }
    }
    st.handle_command(ToileCommand::SetSlideOrder { order });
    st
}

/// A font context sealed to the faces the application ships, as the Logic
/// thread seeds its own (`LogicCommand::LoadFonts`).
pub fn shipped_fonts() -> FontContext {
    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
    let mut font_cx = ambre_draw::sealed_font_context();
    for file in ["NotoSans-Regular.ttf", "DejaVuSerif.ttf"] {
        let bytes = std::fs::read(format!("{DIR}{file}"))
            .unwrap_or_else(|e| panic!("the workspace ships {file}: {e}"));
        ambre_draw::register_face(
            &mut font_cx,
            &ambre_draw::FontFace::new(ambre_core::Payload::new(bytes)),
        );
    }
    font_cx
}
