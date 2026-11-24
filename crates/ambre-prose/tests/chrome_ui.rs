//! The Prose chrome beside the comment panel, mounted in headless Chrome.
//! It covers the suggestion popover, the format bar, the slash menu and a
//! ruler drag (`features/ambre-spec-prose-track-changes.md section 4`,
//! `features/ambre-spec-prose-toolbar.md section 4`,
//! `features/ambre-spec-prose-slash-command.md section 4`,
//! `features/ambre-spec-prose-ruler.md section 4`).
//!
//! Each test mounts one surface on signals it fills by hand. It drives the
//! surface as the pointer does, then reads the elements and the signal the
//! surface writes back. The chrome is wasm-only Leptos, so these tests run
//! under `wasm-bindgen-test-runner` in headless Chrome (`just test-wasm`). On
//! the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_core::id::{Id, Suggestion};
use ambre_core::index::Pos;
use ambre_core::{Css, Point, PrintLength, Pt, Px, View};
use ambre_prose::chrome::{self, ProseSignals};
use ambre_prose::events::SuggestionKind;
use ambre_prose::message::{ActiveMarks, TriState};
use ambre_prose::view::annotate::{AnnotationIntent, HoveredSuggestion, ScreenRect};
use ambre_prose::view::ruler::{RulerDrag, RulerHandle, RulerHandles, RulerState};
use ambre_prose::view::slash::SlashMenuState;
use ambre_prose::view::toolbar::ToolbarState;
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

/// The canvas corner every test puts on the page.
fn origin() -> Point<Px<Css>> {
    Point::new(Px::<Css>::new(40.0), Px::<Css>::new(30.0))
}

/// A point on the canvas, in view pixels.
fn at(x: f32, y: f32) -> Point<Px<View>> {
    Point::new(Px::<View>::new(x), Px::<View>::new(y))
}

/// The one element under `root` that `selector` names, or `None`.
fn one(root: &web_sys::Element, selector: &str) -> Option<web_sys::HtmlElement> {
    root.query_selector(selector)
        .unwrap()
        .map(|e| e.dyn_into().unwrap())
}

/// Every element under `root` that `selector` names.
fn all(root: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let found = root.query_selector_all(selector).unwrap();
    (0..found.length())
        .filter_map(|i| found.item(i))
        .map(|e| e.dyn_into().unwrap())
        .collect()
}

/// Dispatches a cancelable `mousedown` on `el`, and answers whether a
/// listener prevented its default, which keeps the focus where it was.
fn mousedown_is_held(el: &web_sys::HtmlElement) -> bool {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    // A pointer event is a mouse event, and the one init dictionary the
    // workspace builds for both.
    let ev = web_sys::PointerEvent::new_with_event_init_dict("mousedown", &init).unwrap();
    !el.dispatch_event(&ev).unwrap()
}

/// `PROSE-UI-148`: the popover shows over the suggestion the view names as
/// hovered, and over no other. A new hover moves the one popover, and no
/// hover takes it away.
#[wasm_bindgen_test]
async fn the_popover_sits_over_the_hovered_suggestion_alone() {
    let signals = ProseSignals::new();
    signals.canvas_origin.set(origin());
    let root =
        mount_in_container(move || view! { <chrome::ProseSuggestionPopover signals=signals /> });
    leptos::task::tick().await;
    assert!(one(&root, ".ambre-prose-suggestion").is_none(), "no hover");

    let rect = |x0: f32, y0: f32| ScreenRect {
        x0: Px::<View>::new(x0),
        y0: Px::<View>::new(y0),
        x1: Px::<View>::new(x0 + 50.0),
        y1: Px::<View>::new(y0 + 20.0),
    };
    signals.hovered.set(Some(HoveredSuggestion {
        suggestion_id: Id::<Suggestion>::mint(),
        kind: SuggestionKind::Insertion,
        rect: rect(100.0, 200.0),
    }));
    leptos::task::tick().await;
    let popovers = all(&root, ".ambre-prose-suggestion");
    assert_eq!(popovers.len(), 1);
    let r = popovers[0].get_bounding_client_rect();
    // The rectangle's corner, moved onto the page, then lifted 30 pixels.
    assert_eq!((r.left() as f32, r.top() as f32), (140.0, 200.0));
    assert!(one(&root, "[aria-label=\"Accept the insertion\"]").is_some());
    assert!(one(&root, "[aria-label=\"Reject the insertion\"]").is_some());

    signals.hovered.set(Some(HoveredSuggestion {
        suggestion_id: Id::<Suggestion>::mint(),
        kind: SuggestionKind::Deletion,
        rect: rect(300.0, 400.0),
    }));
    leptos::task::tick().await;
    let popovers = all(&root, ".ambre-prose-suggestion");
    assert_eq!(popovers.len(), 1, "one popover, over the new hover");
    let r = popovers[0].get_bounding_client_rect();
    assert_eq!((r.left() as f32, r.top() as f32), (340.0, 400.0));
    assert!(one(&root, "[aria-label=\"Accept the deletion\"]").is_some());
    assert!(one(&root, "[aria-label=\"Accept the insertion\"]").is_none());

    signals.hovered.set(None);
    leptos::task::tick().await;
    assert!(
        one(&root, ".ambre-prose-suggestion").is_none(),
        "hover gone"
    );
    root.remove();
}

/// `PROSE-UI-052`: each popover button hands its intent, naming the hovered
/// suggestion, back to the view. The DOM sends no command.
#[wasm_bindgen_test]
async fn each_popover_button_hands_its_intent_to_the_view() {
    let signals = ProseSignals::new();
    let suggestion_id = Id::<Suggestion>::mint();
    signals.hovered.set(Some(HoveredSuggestion {
        suggestion_id,
        kind: SuggestionKind::Insertion,
        rect: ScreenRect {
            x0: Px::<View>::new(100.0),
            y0: Px::<View>::new(200.0),
            x1: Px::<View>::new(150.0),
            y1: Px::<View>::new(220.0),
        },
    }));
    let root =
        mount_in_container(move || view! { <chrome::ProseSuggestionPopover signals=signals /> });
    leptos::task::tick().await;

    one(&root, ".ambre-prose-suggestion-accept")
        .unwrap()
        .click();
    assert_eq!(
        signals.intent.get_untracked(),
        Some(AnnotationIntent::Accept { suggestion_id })
    );
    signals.intent.set(None);
    one(&root, ".ambre-prose-suggestion-reject")
        .unwrap()
        .click();
    assert_eq!(
        signals.intent.get_untracked(),
        Some(AnnotationIntent::Reject { suggestion_id })
    );
    root.remove();
}

/// The `aria-pressed` value of each format bar control, in bar order.
fn pressed(root: &web_sys::Element) -> Vec<String> {
    all(root, ".ambre-format-control")
        .iter()
        .map(|c| c.get_attribute("aria-pressed").unwrap_or_default())
        .collect()
}

/// `PROSE-UI-053`: the bar is on the page only while the view gives it an
/// anchor. `PROSE-UI-096`: a mark the selection carries in part reads
/// mixed, never the state of either end.
#[wasm_bindgen_test]
async fn the_format_bar_follows_its_anchor_and_reports_each_mark() {
    let signals = ProseSignals::new();
    signals.canvas_origin.set(origin());
    let root = mount_in_container(
        move || view! { <chrome::ProseFormatBar signals=signals cover=RwSignal::new(ambre_client_core::cover::Cover::default()) /> },
    );
    leptos::task::tick().await;
    assert!(one(&root, ".ambre-format-bar").is_none(), "no selection");

    signals.toolbar.set(ToolbarState {
        anchor: Some(at(200.0, 100.0)),
        active: None,
    });
    leptos::task::tick().await;
    assert!(one(&root, ".ambre-format-bar").is_some());
    assert_eq!(pressed(&root), ["false"; 5], "no answer yet: nothing on");

    signals.toolbar.set(ToolbarState {
        anchor: Some(at(200.0, 100.0)),
        active: Some(ActiveMarks {
            bold: TriState::On,
            italic: TriState::Mixed,
            ..ActiveMarks::default()
        }),
    });
    leptos::task::tick().await;
    assert_eq!(pressed(&root), ["true", "mixed", "false", "false", "false"]);

    signals.toolbar.set(ToolbarState::default());
    leptos::task::tick().await;
    assert!(one(&root, ".ambre-format-bar").is_none(), "selection gone");
    root.remove();
}

/// `PROSE-UI-098`: a press on each control hands that control's command id
/// back to the view, the one id its chord resolves to. The press keeps the
/// focus where it was, so the selection stays.
#[wasm_bindgen_test]
async fn a_control_press_hands_its_command_back_and_keeps_the_focus() {
    let signals = ProseSignals::new();
    signals.toolbar.set(ToolbarState {
        anchor: Some(at(200.0, 100.0)),
        active: None,
    });
    let root = mount_in_container(
        move || view! { <chrome::ProseFormatBar signals=signals cover=RwSignal::new(ambre_client_core::cover::Cover::default()) /> },
    );
    leptos::task::tick().await;

    let controls = all(&root, ".ambre-format-control");
    assert_eq!(controls.len(), ambre_prose::commands::FORMAT_BAR.len());
    for (control, (id, _)) in controls.iter().zip(ambre_prose::commands::FORMAT_BAR) {
        assert!(mousedown_is_held(control), "{id:?} takes the focus");
        signals.format.set(None);
        control.click();
        assert_eq!(signals.format.get_untracked(), Some(id));
    }
    root.remove();
}

/// `PROSE-UI-089`: the slash menu lists the items the view gives it, marks
/// the selected one, and hands back the place of the item clicked. A press
/// inside it keeps the focus in the text, and a click is the one gesture
/// that picks.
#[wasm_bindgen_test]
async fn the_slash_menu_hands_back_the_item_clicked() {
    let signals = ProseSignals::new();
    signals.canvas_origin.set(origin());
    let root = mount_in_container(move || view! { <chrome::ProseSlashMenu signals=signals /> });
    leptos::task::tick().await;
    assert!(one(&root, ".ambre-prose-slash-menu").is_none(), "no slash");

    signals.slash.set(SlashMenuState {
        anchor: Some(at(100.0, 50.0)),
        query: String::new(),
        items: vec!["Heading 1", "Bullet list", "Quote"].into(),
        selected: Pos::new(1),
    });
    leptos::task::tick().await;
    let menu = one(&root, ".ambre-prose-slash-menu").unwrap();
    let r = menu.get_bounding_client_rect();
    // The anchor moved onto the page, then dropped 4 pixels. No stylesheet
    // sets the list's margin, so the border box sits below it by that margin.
    let margin = window()
        .get_computed_style(&menu)
        .unwrap()
        .unwrap()
        .get_property_value("margin-top")
        .unwrap();
    let margin: f32 = margin.trim_end_matches("px").parse().unwrap();
    assert_eq!((r.left() as f32, r.top() as f32 - margin), (140.0, 84.0));
    let items = all(&root, ".ambre-prose-slash-item");
    let labels: Vec<_> = items.iter().map(|i| i.text_content().unwrap()).collect();
    assert_eq!(labels, ["Heading 1", "Bullet list", "Quote"]);
    let selected: Vec<_> = items
        .iter()
        .map(|i| i.get_attribute("aria-selected").unwrap())
        .collect();
    assert_eq!(selected, ["false", "true", "false"]);

    assert!(mousedown_is_held(&items[2]), "a press takes the focus");
    assert_eq!(signals.slash_pick.get_untracked(), None, "a press picks");
    items[2].click();
    assert_eq!(signals.slash_pick.get_untracked(), Some(Pos::new(2)));
    root.remove();
}

/// Dispatches the pointer event `kind` on `el`, at `x` on the page.
fn pointer(el: &web_sys::Element, kind: &str, x: f32) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_client_x(x as i32);
    init.set_client_y(40);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// `PROSE-UI-076`: a ruler handle in motion follows the pointer and hands
/// nothing back. `PROSE-UI-063`: its release hands back one value, the
/// indent where the pointer let go.
#[wasm_bindgen_test]
async fn a_ruler_drag_hands_back_one_release() {
    let signals = ProseSignals::new();
    let ruler = RulerState {
        visible: RulerHandles::Visible,
        page_left: Px::<View>::new(80.0),
        content_width: Px::<View>::new(451.0),
        point_width: Px::<View>::new(4.0 / 3.0),
        indent_first: PrintLength::<Pt>::new(0.0),
        indent_left: PrintLength::<Pt>::new(36.0),
        indent_right: PrintLength::<Pt>::new(0.0),
    };
    signals.canvas_origin.set(origin());
    signals.ruler.set(ruler);
    let root = mount_in_container(move || view! { <chrome::ProseRuler signals=signals /> });
    leptos::task::tick().await;

    // The left indent handle, the second of the three.
    let handle: web_sys::Element = all(&root, ".ambre-prose-ruler-handle")[1].clone().into();
    let centre = |el: &web_sys::Element| {
        let r = el.get_bounding_client_rect();
        (r.left() + r.width() / 2.0) as f32
    };
    let start = centre(&handle);
    assert_eq!(
        handle.text_content().as_deref(),
        Some(""),
        "no glyph draws the marker"
    );
    assert_eq!(handle.get_attribute("data-dragging"), None);
    pointer(&handle, "pointerdown", start);
    leptos::task::tick().await;
    // `PROSE-UI-257`: the marker in hand shows its drag state.
    assert_eq!(
        handle.get_attribute("data-dragging").as_deref(),
        Some("true")
    );
    for step in 1..=3 {
        pointer(&handle, "pointermove", start + 20.0 * step as f32);
        leptos::task::tick().await;
        assert_eq!(signals.ruler_release.get_untracked(), None, "step {step}");
        assert!(
            (centre(&handle) - (start + 20.0 * step as f32)).abs() < 0.5,
            "step {step}: the handle stays behind the pointer"
        );
    }
    pointer(&handle, "pointerup", start + 60.0);
    leptos::task::tick().await;

    let dropped = Px::<Css>::new(start + 60.0).in_canvas(origin().x);
    let expected = RulerDrag::press(RulerHandle::IndentLeft, dropped).release(&ruler);
    assert_eq!(signals.ruler_release.get_untracked(), Some(expected));
    assert_eq!(
        handle.get_attribute("data-dragging"),
        None,
        "the drag state ends"
    );
    signals.ruler_release.set(None);
    pointer(&handle, "pointerup", start + 80.0);
    assert_eq!(signals.ruler_release.get_untracked(), None, "one release");
    root.remove();
}

/// `PROSE-UI-248`: the spelling popover sits under the pressed word, says it
/// is looking up until the corrections arrive, then lists them. A pick and
/// the Ignore button each hand their intent, naming the word's range, back to
/// the view. The DOM sends no command.
#[wasm_bindgen_test]
async fn the_spelling_popover_lists_the_corrections_and_hands_back_a_pick() {
    use ambre_convert::spelling::SpellWord;
    use ambre_core::Validated;
    use ambre_core::index::Span;
    use ambre_prose::message::DocChar;
    use ambre_prose::view::annotate::{Offered, SpellingPopover};

    let signals = ProseSignals::new();
    signals.canvas_origin.set(origin());
    let block_id = ambre_prose::events::block_id(loro::TreeID::new(1, 0));
    let range = Span::<DocChar>::new(Pos::<DocChar>::new(2), Pos::<DocChar>::new(9));
    let open = |suggestions| SpellingPopover {
        block_id,
        range,
        rect: ScreenRect {
            x0: Px::<View>::new(100.0),
            y0: Px::<View>::new(200.0),
            x1: Px::<View>::new(150.0),
            y1: Px::<View>::new(220.0),
        },
        suggestions,
    };
    let root =
        mount_in_container(move || view! { <chrome::ProseSpellingPopover signals=signals /> });
    leptos::task::tick().await;
    assert!(
        one(&root, ".ambre-prose-spelling").is_none(),
        "nothing pressed"
    );

    signals.spelling.set(Some(open(Offered::Waiting)));
    leptos::task::tick().await;
    let r = one(&root, ".ambre-prose-spelling")
        .unwrap()
        .get_bounding_client_rect();
    // Under the word: its corner moved onto the page, 4 pixels below it.
    assert_eq!((r.left() as f32, r.top() as f32), (140.0, 254.0));
    assert!(one(&root, ".ambre-prose-spelling-waiting").is_some());

    let receive = Validated::<SpellWord>::parse("receive").unwrap();
    let relieve = Validated::<SpellWord>::parse("relieve").unwrap();
    signals
        .spelling
        .set(Some(open(Offered::Ready(vec![receive.clone(), relieve]))));
    leptos::task::tick().await;
    let picks = all(&root, ".ambre-prose-spelling-pick");
    assert_eq!(picks.len(), 2);
    assert_eq!(picks[0].text_content().as_deref(), Some("receive"));
    picks[0].click();
    assert_eq!(
        signals.intent.get_untracked(),
        Some(AnnotationIntent::Correct {
            block_id,
            range,
            replacement: receive,
        })
    );
    one(&root, ".ambre-prose-spelling-ignore").unwrap().click();
    assert_eq!(
        signals.intent.get_untracked(),
        Some(AnnotationIntent::Ignore { block_id, range })
    );

    signals.spelling.set(Some(open(Offered::Ready(Vec::new()))));
    leptos::task::tick().await;
    assert!(one(&root, ".ambre-prose-spelling-none").is_some());
    signals.spelling.set(None);
    leptos::task::tick().await;
    assert!(one(&root, ".ambre-prose-spelling").is_none(), "closed");
    root.remove();
}
