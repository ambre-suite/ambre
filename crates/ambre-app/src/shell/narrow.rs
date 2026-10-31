// ambre-app/src/shell/narrow.rs
// Browser-tier tests of the shell stylesheet at a set width. A width query
// reads the viewport of the frame it runs in. So each test mounts into an
// iframe of the width it reads.

use ambre_chrome::commands::{Command, CommandCategory, CommandId, Locale};
use ambre_chrome::{ContextMenu, MenuEntry};
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
        id: CommandId("clipboard.copy"),
        title: |_| "Copy".to_owned(),
        icon: None,
        keybindings: &[],
        category: CommandCategory("edit"),
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
