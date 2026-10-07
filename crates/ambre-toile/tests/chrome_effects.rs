//! Toile chrome tests whose assertion waits for a Leptos effect.
//!
//! An effect runs on the next tick, so each test here is async and awaits
//! `leptos::task::tick()`. They live apart from `chrome_ui`: there a tick
//! would also run the effects of earlier tests, whose owners are gone.
//! They run under `wasm-pack test --headless --chrome crates/ambre-toile`
//! (`just test-wasm`). On the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]

use ambre_bus::app::toile::{DeckFrame, DeckView, ElementId};
use ambre_toile::chrome::{DeckIntent, DeckPanel, PresenterIntent, PresenterPanel};
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn mount_in_container<F, N>(f: F) -> web_sys::Element
where
    F: FnOnce() -> N + 'static,
    N: IntoView + 'static,
{
    let container = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&container).unwrap();
    let target: web_sys::HtmlElement = container.clone().dyn_into().unwrap();
    mount_to(target, f).forget();
    container
}

/// `TOILE-UI-316`: a peer's write reaches the notes area, and the caret keeps
/// its place in the text rather than jumping to the end.
#[wasm_bindgen_test]
async fn a_peer_note_write_keeps_the_caret() {
    let notes = RwSignal::new("Hello world".to_owned());
    let root = mount_in_container(move || {
        view! {
            <PresenterPanel
                open=RwSignal::new(true)
                index=RwSignal::new(0)
                count=RwSignal::new(1)
                notes=notes
                broadcasting=RwSignal::new(false)
                following=RwSignal::new(None)
                presenter_peer=RwSignal::new(None)
                on_intent=Callback::new(|_: PresenterIntent| {})
            />
        }
    });
    // The effect that seeds the area runs on the next tick.
    leptos::task::tick().await;
    let area: web_sys::HtmlTextAreaElement = root
        .query_selector("[aria-label=\"Speaker notes\"]")
        .unwrap()
        .expect("the notes area")
        .dyn_into()
        .unwrap();
    assert_eq!(area.value(), "Hello world");
    area.focus().unwrap();
    area.set_selection_range(5, 5).unwrap();

    notes.set("Dear Hello world".to_owned());
    leptos::task::tick().await;
    assert_eq!(area.value(), "Dear Hello world");
    assert_eq!(area.selection_start().unwrap(), Some(10), "after \"Hello\"");
    assert_eq!(area.selection_end().unwrap(), Some(10));
    root.remove();
}

/// The deck reads in playback order, an unnamed frame by its rank
/// (`TOILE-FEAT-026`).
#[wasm_bindgen_test]
async fn deck_panel_lists_the_slides_in_order() {
    let frame = |id: &str, name: &str| DeckFrame {
        id: ElementId(id.to_owned()),
        name: name.to_owned(),
    };
    let view = RwSignal::new(DeckView {
        slides: vec![frame("a", "Agenda"), frame("b", "")],
        off_deck: Vec::new(),
    });
    let root = mount_in_container(move || {
        view! {
            <DeckPanel
                state=RwSignal::new(true)
                view=view
                on_intent=Callback::new(|_: DeckIntent| {})
            />
        }
    });
    // A `prop:value` is written by an effect, on the next tick.
    leptos::task::tick().await;
    let names = root
        .query_selector_all(".ambre-deck-list .ambre-deck-name")
        .unwrap();
    assert_eq!(names.length(), 2);
    let field =
        |i: u32| -> web_sys::HtmlInputElement { names.item(i).unwrap().dyn_into().unwrap() };
    assert_eq!(field(0).value(), "Agenda");
    // An unnamed frame reads by its rank, and the field stays empty so typing
    // starts on a blank title rather than on the placeholder.
    assert_eq!(field(1).value(), "");
    assert_eq!(field(1).placeholder(), "Frame 2");
    root.remove();
}
