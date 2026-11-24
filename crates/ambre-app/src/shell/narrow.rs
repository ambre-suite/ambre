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

/// The design tokens, as the page loads them.
const TOKENS: &str = include_str!("../../../ambre-canvas/src/theme.css");

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
        description: None,
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
    /// The strips of the canvas a fit keeps clear of, from the cover the
    /// panel published of itself (`CHROME-171`).
    clear: ambre_client_core::cover::Clearance,
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
    let cover = RwSignal::new(ambre_client_core::cover::Cover::default());
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
                    <ContextPanel prefs=prefs title="Board" sections=sections.clone() cover=cover />
                </ContextRegion>
            </AppShell>
        }
    });
    leptos::task::tick().await;
    let _ = frame.offset_width();
    // The panel measures itself once the page lays it out.
    for _ in 0..2 {
        let next = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .request_animation_frame(&resolve)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(next).await.unwrap();
    }
    let rect = |sel: &str| {
        inner
            .query_selector(sel)
            .unwrap()
            .expect("on screen")
            .get_bounding_client_rect()
    };
    let canvas = rect(".ambre-content");
    let edges = PanelEdges {
        clear: cover
            .get_untracked()
            .clearance(ambre_chrome::cover::page_box(&canvas)),
        canvas,
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

/// The panel publishes the box it covers (`CHROME-171`). A desktop panel
/// docks beside the canvas, so a fit keeps no strip for it. The sheet of a
/// phone covers the bottom of the canvas, so a fit keeps that strip clear.
#[wasm_bindgen_test]
async fn the_panel_publishes_its_cover_and_a_fit_clears_the_sheet_alone() {
    use ambre_chrome::DockSide;
    use ambre_client_core::cover::{COVER_GAP, Clearance};
    for side in [DockSide::Right, DockSide::Left] {
        let desk = panel_edges_at(1280, side).await;
        assert_eq!(desk.clear, Clearance::default(), "{side:?} on a desktop");
        let phone = panel_edges_at(360, side).await;
        let sheet = phone.panel.height() + f64::from(COVER_GAP.get());
        assert!(phone.panel.height() > 0.0);
        assert!(
            (f64::from(phone.clear.bottom.get()) - sheet).abs() < 0.5,
            "{side:?} on a phone: {:?}, sheet {sheet}",
            phone.clear
        );
        assert_eq!(phone.clear.top, ambre_core::Px::<ambre_core::Css>::ZERO);
    }
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

/// What the creation bar of one page shows, laid out with a context panel
/// docked to `side`, in a frame 1280 pixels wide.
struct CreationEdges {
    canvas: web_sys::DomRect,
    bar: web_sys::DomRect,
    /// The command each entry of the bar reaches, in order.
    reaches: Vec<String>,
    /// The box the bar published of itself (`CHROME-171`).
    covered: Option<ambre_client_core::cover::Covered>,
}

/// Lay out the creation bar of a page of `kind` in the content cell, as the
/// shell does, beside a context panel docked to `side`.
async fn creation_edges(kind: ambre_bus::DocType, side: ambre_chrome::DockSide) -> CreationEdges {
    use ambre_chrome::{
        AppShell, CanvasOverlay, ContextPanel, ContextRegion, Fold, Header, PanelPrefs,
        PanelSection, PanelWidth, Toolbar,
    };
    let frame: web_sys::HtmlIFrameElement = document()
        .create_element("iframe")
        .unwrap()
        .dyn_into()
        .unwrap();
    frame
        .set_attribute("style", &format!("width:1280px;height:{HEIGHT}px;border:0"))
        .unwrap();
    document().body().unwrap().append_child(&frame).unwrap();
    let inner = frame.content_document().unwrap();
    // The bar sits on the scales of the design tokens, as the page loads
    // them before the shell stylesheet.
    let sheet = inner.create_element("style").unwrap();
    sheet.set_text_content(Some(&format!("{TOKENS}\n{STYLES}")));
    inner.head().unwrap().append_child(&sheet).unwrap();
    inner
        .document_element()
        .unwrap()
        .set_attribute("style", "height:100%")
        .unwrap();
    inner
        .body()
        .unwrap()
        .set_attribute("style", "margin:0;height:100%")
        .unwrap();
    let prefs = PanelPrefs::new(side, PanelWidth::DEFAULT, Fold::Open);
    let cover = RwSignal::new(ambre_client_core::cover::Cover::default());
    let tool = RwSignal::new(ambre_toile::ui::Tool::Select);
    let sections = vec![PanelSection::new(
        ambre_core::StaticKey::new("t.board"),
        "Board",
        icondata::MdiViewDashboardOutline,
        || view! { <p>"Grid"</p> }.into_any(),
    )];
    let on_command = Callback::new(|_: StaticKey<Commands>| {});
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
                    <ContextPanel prefs=prefs title="Board" sections=sections.clone() cover=cover />
                </ContextRegion>
                <CanvasOverlay slot>
                    {super::app_root::creation_bar(kind, tool, on_command, cover)}
                </CanvasOverlay>
            </AppShell>
        }
    });
    leptos::task::tick().await;
    let _ = frame.offset_width();
    for _ in 0..2 {
        let next = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .request_animation_frame(&resolve)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(next).await.unwrap();
    }
    let bar = inner
        .query_selector(".ambre-creation-bar")
        .unwrap()
        .expect("the page lays out a creation bar");
    let entries = bar.query_selector_all("[data-reach]").unwrap();
    let reaches = (0..entries.length())
        .filter_map(|i| entries.item(i))
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .filter_map(|el| el.get_attribute("data-reach"))
        .collect();
    let edges = CreationEdges {
        canvas: inner
            .query_selector(".ambre-content")
            .unwrap()
            .expect("on screen")
            .get_bounding_client_rect(),
        bar: bar.get_bounding_client_rect(),
        reaches,
        covered: cover
            .get_untracked()
            .of(ambre_client_core::cover::Surface::CreationBar),
    };
    drop(handle);
    frame.remove();
    edges
}

/// `CHROME-153`: Prose, Trame and Toile each lay out the one creation bar.
/// It sits centred at the bottom of the content, beside a panel on either side.
/// The bars share one height and one bottom edge. `CHROME-152`: each holds
/// every command its table homes in the creation bar, in order, and no other.
#[wasm_bindgen_test]
async fn the_three_pages_lay_out_one_creation_bar_at_one_place() {
    use ambre_bus::DocType;
    use ambre_chrome::DockSide;
    let mut shapes = Vec::new();
    for kind in [DocType::Prose, DocType::Trame, DocType::Toile] {
        for side in [DockSide::Right, DockSide::Left] {
            let edges = creation_edges(kind, side).await;
            let (canvas, bar) = (&edges.canvas, &edges.bar);
            let centre = |r: &web_sys::DomRect| r.left() + r.width() / 2.0;
            assert!(
                (centre(bar) - centre(canvas)).abs() < 1.0,
                "{kind:?} {side:?}: the bar is centred on the content"
            );
            let gap = canvas.bottom() - bar.bottom();
            assert!(
                gap > 0.0 && gap < 24.0,
                "{kind:?} {side:?}: the bar sits on the bottom edge, {gap} px above it"
            );
            assert!(
                bar.left() >= canvas.left() && bar.right() <= canvas.right(),
                "{kind:?} {side:?}: the panel covers none of the bar"
            );
            assert!(
                edges.covered.is_some(),
                "{kind:?} {side:?}: the bar publishes its box"
            );
            let homed = ambre_chrome::creation_entries(super::app_root::palette_table(kind));
            assert_eq!(
                edges.reaches.len(),
                homed.len(),
                "{kind:?}: {:?}",
                edges.reaches
            );
            for (mark, command) in edges.reaches.iter().zip(&homed) {
                assert!(
                    ambre_chrome::commands::Reach::Command(command.id).matches_marker(mark),
                    "{kind:?}: {mark} for {:?}",
                    command.id
                );
            }
            shapes.push((bar.height(), gap));
        }
    }
    let (height, gap) = shapes[0];
    for (h, g) in &shapes {
        assert!(
            (h - height).abs() < 0.5 && (g - gap).abs() < 0.5,
            "one shape: {shapes:?}"
        );
    }
}
