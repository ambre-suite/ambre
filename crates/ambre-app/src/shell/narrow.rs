// ambre-app/src/shell/narrow.rs
// Browser-tier tests of the shell stylesheet at a set width. A width query
// reads the viewport of the frame it runs in. So each test mounts into an
// iframe of the width it reads.

use ambre_chrome::commands::{Command, Home, Locale};
use ambre_chrome::{ContextMenu, MenuEntry};
use ambre_core::{Categories, Commands, StaticKey};
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;

/// The page point at `(x, y)`, in CSS px.
fn css_point(x: f32, y: f32) -> ambre_core::Point<ambre_core::Px<ambre_core::Css>> {
    ambre_core::Point::new(
        ambre_core::Px::<ambre_core::Css>::new(x),
        ambre_core::Px::<ambre_core::Css>::new(y),
    )
}

/// The shell stylesheet, as the page loads it.
const STYLES: &str = include_str!("../../styles.css");

/// The frame's height, which a pinned sheet's bottom edge meets.
const HEIGHT: f64 = 300.0;

/// Open a context menu at `(150, 60)` in a frame of `width` pixels. Return the
/// menu's left, right and bottom edges, in the frame's coordinates.
async fn menu_edges_at(width: u32) -> (f64, f64, f64) {
    let frame: web_sys::HtmlIFrameElement = document()
        .create_element("iframe")
        .unwrap()
        .dyn_into()
        .unwrap();
    let style = format!("width:{width}px;height:{HEIGHT}px;border:0");
    frame.set_attribute("style", &style).unwrap();
    document().body().unwrap().append_child(&frame).unwrap();
    let inner = frame.content_document().unwrap();
    let sheet = inner.create_element("style").unwrap();
    sheet.set_text_content(Some(STYLES));
    inner.head().unwrap().append_child(&sheet).unwrap();

    let pos = RwSignal::new(Some(css_point(150.0, 60.0)));
    let copy = Command {
        id: StaticKey::<Commands>::new("clipboard.copy"),
        title: |_| ambre_core::Validated::new("Copy"),
        icon: None,
        keybindings: &[],
        category: StaticKey::<Categories>::new("edit"),
        home: Home::ContextMenu,
    };
    let entries = Signal::stored(vec![MenuEntry::command(&copy, &Locale::En)]);
    let handle = mount_to(inner.body().unwrap(), move || {
        view! { <ContextMenu pos=pos entries=entries on_command=Callback::new(|_| {}) /> }
    });
    leptos::task::tick().await;
    let _ = frame.offset_width();

    let rect = inner
        .query_selector(".ambre-ctx-menu")
        .unwrap()
        .expect("the menu is on screen")
        .get_bounding_client_rect();
    let edges = (rect.left(), rect.right(), rect.bottom());
    pos.set(None);
    leptos::task::tick().await;
    drop(handle);
    frame.remove();
    edges
}

/// On a phone the menu is a sheet on the bottom edge, whatever the click
/// point (`CHROME-032`). A tablet keeps it floating where the click was.
#[wasm_bindgen_test]
async fn a_phone_menu_is_a_sheet_on_the_bottom_edge() {
    for width in [360, 599] {
        let (left, right, bottom) = menu_edges_at(width).await;
        assert_eq!(left, 0.0, "a {width} px phone");
        assert_eq!(right, f64::from(width), "a {width} px phone");
        assert_eq!(bottom, HEIGHT, "a {width} px phone");
    }
    let (left, right, bottom) = menu_edges_at(600).await;
    assert_eq!(left, 150.0, "the narrowest tablet floats at the click");
    assert!(
        right < 600.0 && bottom < HEIGHT,
        "the narrowest tablet floats"
    );
}

/// The edges of the canvas and of the context panel, in a frame of `width`
/// pixels, with the panel docked to `side`.
struct PanelEdges {
    canvas: web_sys::DomRect,
    panel: web_sys::DomRect,
}

/// Lay out a document page with a context panel docked to `side`, in a frame
/// of `width` pixels, and read the edges.
async fn panel_edges_at(width: u32, side: ambre_chrome::DockSide) -> PanelEdges {
    use ambre_chrome::{
        AppShell, ContextPanel, ContextRegion, Fold, Header, PanelPrefs, PanelSection, PanelWidth,
        Toolbar,
    };
    let frame: web_sys::HtmlIFrameElement = document()
        .create_element("iframe")
        .unwrap()
        .dyn_into()
        .unwrap();
    let style = format!("width:{width}px;height:{HEIGHT}px;border:0");
    frame.set_attribute("style", &style).unwrap();
    document().body().unwrap().append_child(&frame).unwrap();
    let inner = frame.content_document().unwrap();
    let sheet = inner.create_element("style").unwrap();
    sheet.set_text_content(Some(STYLES));
    inner.head().unwrap().append_child(&sheet).unwrap();
    let html = inner.document_element().unwrap();
    html.set_attribute("style", "height:100%").unwrap();
    inner
        .body()
        .unwrap()
        .set_attribute("style", "margin:0;height:100%")
        .unwrap();

    let prefs = PanelPrefs::new(side, PanelWidth::DEFAULT, Fold::Open);
    let sections = vec![PanelSection::new(
        ambre_core::StaticKey::new("t.board"),
        "Board",
        icondata::MdiViewDashboardOutline,
        || view! { <p>"Grid"</p> }.into_any(),
    )];
    let handle = mount_to(inner.body().unwrap(), move || {
        view! {
            <AppShell rows=ambre_chrome::ShellRows {
                toolbar: ambre_chrome::RowShown::Absent,
                formula_bar: ambre_chrome::RowShown::Absent,
                tabs: ambre_chrome::RowShown::Absent,
            }>
                <Header slot>"Ambre"</Header>
                <Toolbar slot>""</Toolbar>
                <ContextRegion slot>
                    <ContextPanel prefs=prefs title="Board" sections=sections.clone() />
                </ContextRegion>
            </AppShell>
        }
    });
    leptos::task::tick().await;
    let _ = frame.offset_width();
    let rect = |sel: &str| {
        inner
            .query_selector(sel)
            .unwrap()
            .expect("on screen")
            .get_bounding_client_rect()
    };
    let edges = PanelEdges {
        canvas: rect(".ambre-content"),
        panel: rect(".ambre-ctx-panel"),
    };
    drop(handle);
    frame.remove();
    edges
}

/// On a desktop the panel docks to the side the user chose, the right by
/// default (`CHROME-193`). It sits beside the canvas, never over it
/// (`CHROME-198`): the canvas ends where the panel starts.
#[wasm_bindgen_test]
async fn a_desktop_panel_docks_beside_the_canvas_on_its_side() {
    use ambre_chrome::DockSide;
    let right = panel_edges_at(1280, DockSide::default()).await;
    assert_eq!(right.panel.right(), 1280.0, "the right by default");
    assert_eq!(
        right.canvas.right(),
        right.panel.left(),
        "the canvas ends at the panel"
    );
    assert_eq!(right.panel.width(), 300.0);
    let left = panel_edges_at(1280, DockSide::Left).await;
    assert_eq!(left.panel.left(), 0.0);
    assert_eq!(
        left.canvas.left(),
        left.panel.right(),
        "the canvas starts after the panel"
    );
    assert_eq!(left.canvas.right(), 1280.0);
}

/// On a phone the panel is a sheet on the bottom edge, whatever its side
/// (`CHROME-167`).
#[wasm_bindgen_test]
async fn a_phone_panel_is_a_sheet_on_the_bottom_edge_whatever_its_side() {
    use ambre_chrome::DockSide;
    for side in [DockSide::Right, DockSide::Left] {
        for width in [360, 599] {
            let edges = panel_edges_at(width, side).await;
            assert_eq!(edges.panel.left(), 0.0, "{side:?} at {width} px");
            assert_eq!(
                edges.panel.right(),
                f64::from(width),
                "{side:?} at {width} px"
            );
            assert_eq!(edges.panel.bottom(), HEIGHT, "{side:?} at {width} px");
        }
    }
}
