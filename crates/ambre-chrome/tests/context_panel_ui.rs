//! Wasm tier: the frame of the context panel and the kit
//! (`ambre-spec-shell.md` sections 1 and 2).
//!
//! Each test mounts the panel into a container of its own, with sections
//! written from the kit, and reads the DOM a person would see.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::unwrap_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_chrome::commands::{Command, Home, Locale, Property, Reach, Reading};
use ambre_chrome::{
    Choice, ColourField, CommandButton, ContextPanel, DockSide, Field, Fold, LengthField,
    PanelPrefs, PanelSection, PanelWidth, Segmented, reach_markers,
};
use ambre_core::{
    Categories, Commands, Css, Mm, PanelSections, PrintLength, Properties, Px, Rgb, StaticKey,
};
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const PAGE: StaticKey<PanelSections> = StaticKey::<PanelSections>::new("t.page");
const TEXT: StaticKey<PanelSections> = StaticKey::<PanelSections>::new("t.text");

static BOLD: Command = Command {
    id: StaticKey::<Commands>::new("t.format.bold"),
    title: |_: &Locale| ambre_core::Validated::new("Bold"),
    icon: None,
    keybindings: &[],
    category: StaticKey::<Categories>::new("format"),
    home: Home::ContextPanel,
    description: None,
};

static INK: Property = Property {
    id: StaticKey::<Properties>::new("t.ink"),
    title: |_: &Locale| ambre_core::Validated::new("Text colour"),
    icon: None,
};

static MARGIN: Property = Property {
    id: StaticKey::<Properties>::new("t.margin"),
    title: |_: &Locale| ambre_core::Validated::new("Margin"),
    icon: None,
};

static ORIENT: Property = Property {
    id: StaticKey::<Properties>::new("t.orientation"),
    title: |_: &Locale| ambre_core::Validated::new("Orientation"),
    icon: None,
};

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

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

/// Two sections: a page section with an input and a tall filler, and a
/// text section with a command and a colour.
fn two_sections() -> Vec<PanelSection> {
    vec![
        PanelSection::new(PAGE, "Page", icondata::MdiFileDocumentOutline, || {
            view! {
                <Field label="Note"><input class="t-edit" type="text" /></Field>
                <div style="height:2000px"></div>
            }
            .into_any()
        }),
        PanelSection::new(TEXT, "Text", icondata::MdiFormatText, || {
            view! {
                <CommandButton command=&BOLD icon=None on_command=Callback::new(|_| {}) />
            }
            .into_any()
        }),
    ]
}

/// The shell stylesheet, as the page loads it.
const STYLES: &str = include_str!("../../ambre-app/styles.css");

/// Put the shell stylesheet into the page once.
fn with_styles() {
    if document().get_element_by_id("t-shell-styles").is_none() {
        let sheet = document().create_element("style").unwrap();
        sheet.set_id("t-shell-styles");
        sheet.set_text_content(Some(STYLES));
        document().head().unwrap().append_child(&sheet).unwrap();
    }
}

fn mount_panel(prefs: PanelPrefs) -> web_sys::Element {
    with_styles();
    mount_in_container(move || {
        view! {
            <div class="ambre-body" style="height:400px;width:900px">
                <ContextPanel prefs=prefs title="Nothing selected" sections=two_sections() />
            </div>
        }
    })
}

fn q(root: &web_sys::Element, sel: &str) -> web_sys::Element {
    root.query_selector(sel).unwrap().unwrap()
}

fn click(el: &web_sys::Element) {
    el.dyn_ref::<web_sys::HtmlElement>().unwrap().click();
}

fn ctrl_period() {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(".");
    init.set_ctrl_key(true);
    init.set_bubbles(true);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    web_sys::window().unwrap().dispatch_event(&ev).unwrap();
}

#[wasm_bindgen_test]
async fn the_panel_takes_one_frame_a_header_then_sections_that_fold() {
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    let root = mount_panel(prefs);
    leptos::task::tick().await;
    assert_eq!(
        q(&root, ".ambre-ctx-head .ambre-ctx-title")
            .text_content()
            .unwrap(),
        "Nothing selected"
    );
    let sections = root.query_selector_all(".ambre-ctx-section").unwrap();
    assert_eq!(sections.length(), 2);
    let text = q(&root, ".ambre-ctx-section[data-section=\"t.text\"]");
    assert_eq!(text.get_attribute("data-folded").unwrap(), "false");
    click(&q(&text, ".ambre-ctx-section-head"));
    leptos::task::tick().await;
    assert_eq!(text.get_attribute("data-folded").unwrap(), "true");
    assert_eq!(
        q(&text, ".ambre-ctx-section-head")
            .get_attribute("aria-expanded")
            .unwrap(),
        "false"
    );
}

#[wasm_bindgen_test]
async fn the_rail_shows_one_icon_for_each_section() {
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Rail);
    let root = mount_panel(prefs);
    leptos::task::tick().await;
    let icons = root.query_selector_all(".ambre-ctx-rail-icon").unwrap();
    assert_eq!(icons.length(), 2);
    let labels: Vec<String> = (0..icons.length())
        .map(|i| {
            icons
                .item(i)
                .unwrap()
                .dyn_into::<web_sys::Element>()
                .unwrap()
                .get_attribute("aria-label")
                .unwrap()
        })
        .collect();
    assert_eq!(labels, ["Page", "Text"]);
    for i in 0..icons.length() {
        let icon: web_sys::Element = icons.item(i).unwrap().dyn_into().unwrap();
        assert!(
            icon.query_selector("svg").unwrap().is_some(),
            "each rail entry shows an icon"
        );
    }
}

#[wasm_bindgen_test]
async fn ctrl_period_collapses_an_open_panel_and_opens_a_collapsed_one() {
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    let root = mount_panel(prefs);
    leptos::task::tick().await;
    let panel = q(&root, ".ambre-ctx-panel");
    ctrl_period();
    leptos::task::tick().await;
    assert_eq!(panel.get_attribute("data-fold").unwrap(), "rail");
    ctrl_period();
    leptos::task::tick().await;
    assert_eq!(panel.get_attribute("data-fold").unwrap(), "open");
    // A click on the rail opens it too.
    click(&q(&root, ".ambre-ctx-collapse"));
    leptos::task::tick().await;
    assert_eq!(panel.get_attribute("data-fold").unwrap(), "rail");
    click(&q(&root, ".ambre-ctx-open"));
    leptos::task::tick().await;
    assert_eq!(panel.get_attribute("data-fold").unwrap(), "open");
}

#[wasm_bindgen_test]
async fn a_collapse_keeps_the_open_sections_the_scroll_and_the_edit() {
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    let root = mount_panel(prefs);
    leptos::task::tick().await;
    // Fold one section, type into the other, scroll the list.
    click(&q(
        &root,
        ".ambre-ctx-section[data-section=\"t.text\"] .ambre-ctx-section-head",
    ));
    let input: web_sys::HtmlInputElement = q(&root, ".t-edit").dyn_into().unwrap();
    input.set_value("half typed");
    let list = q(&root, ".ambre-ctx-sections");
    list.set_scroll_top(120);
    leptos::task::tick().await;
    let scrolled = list.scroll_top();
    assert!(scrolled > 0, "the list scrolls");

    ctrl_period();
    leptos::task::tick().await;
    ctrl_period();
    leptos::task::tick().await;

    let same: web_sys::HtmlInputElement = q(&root, ".t-edit").dyn_into().unwrap();
    assert!(
        same.is_same_node(Some(&input)),
        "the edit is the same field"
    );
    assert_eq!(same.value(), "half typed");
    assert_eq!(q(&root, ".ambre-ctx-sections").scroll_top(), scrolled);
    assert_eq!(
        q(&root, ".ambre-ctx-section[data-section=\"t.text\"]")
            .get_attribute("data-folded")
            .unwrap(),
        "true"
    );
    assert_eq!(prefs.width.get_untracked(), PanelWidth::DEFAULT);
}

#[wasm_bindgen_test]
async fn a_rail_icon_opens_the_panel_at_its_section() {
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    let root = mount_panel(prefs);
    leptos::task::tick().await;
    let text = q(&root, ".ambre-ctx-section[data-section=\"t.text\"]");
    click(&q(&text, ".ambre-ctx-section-head"));
    prefs.fold.set(Fold::Rail);
    leptos::task::tick().await;
    click(&q(&root, ".ambre-ctx-rail-icon[data-section=\"t.text\"]"));
    leptos::task::tick().await;
    assert_eq!(prefs.fold.get_untracked(), Fold::Open);
    assert_eq!(text.get_attribute("data-folded").unwrap(), "false");
}

fn pointer(kind: &str, x: i32) -> web_sys::PointerEvent {
    let init = web_sys::PointerEventInit::new();
    init.set_client_x(x);
    init.set_pointer_id(7);
    init.set_bubbles(true);
    init.set_is_primary(true);
    web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap()
}

#[wasm_bindgen_test]
async fn a_drag_of_the_edge_sets_the_width_between_the_two_bounds() {
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    let root = mount_panel(prefs);
    leptos::task::tick().await;
    let panel = q(&root, ".ambre-ctx-panel");
    let right = panel.get_bounding_client_rect().right() as i32;
    let edge = q(&root, ".ambre-ctx-edge");
    edge.dispatch_event(&pointer("pointerdown", right - 300))
        .unwrap();
    edge.dispatch_event(&pointer("pointermove", right - 360))
        .unwrap();
    assert_eq!(prefs.width.get_untracked().get(), Px::<Css>::new(360.0));
    edge.dispatch_event(&pointer("pointermove", right - 2000))
        .unwrap();
    assert_eq!(prefs.width.get_untracked(), PanelWidth::MAX);
    edge.dispatch_event(&pointer("pointermove", right - 10))
        .unwrap();
    assert_eq!(prefs.width.get_untracked(), PanelWidth::MIN);
    edge.dispatch_event(&pointer("pointerup", right - 10))
        .unwrap();
    // After the release, a move sets nothing.
    edge.dispatch_event(&pointer("pointermove", right - 400))
        .unwrap();
    assert_eq!(prefs.width.get_untracked(), PanelWidth::MIN);
    leptos::task::tick().await;
    assert_eq!(
        panel.get_attribute("style").unwrap(),
        "--ctx-panel-w: 240px;",
        "the frame lays out the width it holds"
    );
}

#[wasm_bindgen_test]
async fn each_kit_control_carries_what_it_reaches_and_shows_a_mixed_value() {
    let mixed = RwSignal::new(Reading::<Option<Rgb>>::Mixed);
    let margin = RwSignal::new(Reading::Uniform(PrintLength::<Mm>::new(20.0)));
    let written = RwSignal::new(None::<PrintLength<Mm>>);
    let orient = RwSignal::new(Reading::Uniform(1_u8));
    let root = mount_in_container(move || {
        view! {
            <CommandButton command=&BOLD icon=None on_command=Callback::new(|_| {}) />
            <ColourField
                property=&INK
                icon=icondata::MdiFormatColorText
                value=mixed
                default=Signal::stored(Rgb::from_bits(0))
                reset_label="Automatic"
                on_write=Callback::new(|_| {})
            />
            <LengthField property=&MARGIN value=margin on_write=Callback::new(move |m| written.set(Some(m))) />
            <Segmented
                property=&ORIENT
                choices=vec![
                    Choice { value: 0_u8, label: "Portrait", icon: None },
                    Choice { value: 1_u8, label: "Landscape", icon: None },
                ]
                value=orient
                on_write=Callback::new(|_| {})
            />
        }
    });
    leptos::task::tick().await;
    let markers = reach_markers(&root);
    let expected = [
        Reach::Command(BOLD.id),
        Reach::Property(INK.id),
        Reach::Property(MARGIN.id),
        Reach::Property(ORIENT.id),
    ];
    assert_eq!(markers.len(), expected.len());
    for (marker, reach) in markers.iter().zip(expected) {
        assert!(reach.matches_marker(marker), "{marker} names {reach:?}");
    }
    // A mixed colour says so; a uniform choice is checked.
    assert!(q(&root, ".ambre-kit-colour .ambre-kit-mixed").is_object());
    let checked = q(&root, ".ambre-kit-segmented [aria-checked=\"true\"]");
    assert_eq!(checked.get_attribute("aria-label").unwrap(), "Landscape");
    // A stepper writes the length alone, one step on.
    click(&q(&root, ".ambre-stepper .ambre-stepper-step:last-of-type"));
    assert_eq!(written.get_untracked(), Some(PrintLength::<Mm>::new(21.0)));
}

/// A section that starts folded folds on its first showing alone: a user's
/// unfold holds when the sections are handed again.
#[wasm_bindgen_test]
async fn a_section_folded_first_folds_once_and_an_unfold_holds() {
    let sections = RwSignal::new(vec![
        PanelSection::new(PAGE, "Page", icondata::MdiFileDocumentOutline, || {
            view! { <span>"page"</span> }.into_any()
        }),
        PanelSection::new(TEXT, "Text", icondata::MdiFormatText, || {
            view! { <span>"text"</span> }.into_any()
        })
        .folded_first(),
    ]);
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    let root = mount_in_container(move || {
        view! { <ContextPanel prefs=prefs title="Nothing selected" sections=sections /> }
    });
    leptos::task::tick().await;
    let folded = |key: &str| {
        q(
            &root,
            &format!(".ambre-ctx-section[data-section=\"{key}\"]"),
        )
        .get_attribute("data-folded")
        .unwrap()
    };
    assert_eq!(folded("t.page"), "false");
    assert_eq!(folded("t.text"), "true", "folded on its first showing");
    click(&q(
        &root,
        ".ambre-ctx-section[data-section=\"t.text\"] .ambre-ctx-section-head",
    ));
    leptos::task::tick().await;
    assert_eq!(folded("t.text"), "false");
    // The selection moves and the application hands its sections again.
    sections.update(|s| s.reverse());
    leptos::task::tick().await;
    assert_eq!(folded("t.text"), "false", "the user's unfold holds");
}

/// A format control that names a glyph shows it as the toolkit's icon button.
/// Its label is the tooltip and the accessible name, and it keeps its state.
#[wasm_bindgen_test]
async fn a_format_bar_control_shows_its_glyph_as_an_icon_button() {
    use ambre_chrome::{FormatBar, FormatControl, Pressed};
    let root = mount_in_container(move || {
        let controls = vec![FormatControl {
            label: "Bold".to_owned(),
            pressed: Pressed::On,
            on_press: Callback::new(|()| {}),
            icon: Some(icondata::BsTypeBold),
            chord: None,
            description: None,
            reach: None,
        }];
        view! { <FormatBar anchor=ambre_core::Point::default() controls=controls /> }
    });
    leptos::task::tick().await;
    let control = q(&root, ".ambre-format-bar .ambre-icon-button");
    assert_eq!(control.get_attribute("aria-label").unwrap(), "Bold");
    assert_eq!(control.get_attribute("aria-pressed").unwrap(), "true");
    assert!(control.query_selector("svg").unwrap().is_some(), "a glyph");
    assert_eq!(
        q(&control, ".ambre-tip").text_content().unwrap(),
        "Bold",
        "the label is the tooltip"
    );
}

/// The subtitle shows the count the application hands, under the title, and
/// an empty one shows nothing (`CHROME-241`).
#[wasm_bindgen_test]
async fn the_subtitle_counts_the_selection_under_the_title() {
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    let subtitle = RwSignal::new(String::from("4 words"));
    with_styles();
    let root = mount_in_container(move || {
        view! {
            <ContextPanel prefs=prefs title="Selection" subtitle=subtitle sections=two_sections() />
        }
    });
    leptos::task::tick().await;
    assert_eq!(
        q(&root, ".ambre-ctx-head .ambre-ctx-subtitle")
            .text_content()
            .unwrap(),
        "4 words"
    );
    subtitle.set(String::new());
    leptos::task::tick().await;
    assert!(
        root.query_selector(".ambre-ctx-subtitle")
            .unwrap()
            .is_none()
    );
}

/// The header of the panel moves it to the other side, and the side is the
/// one the shell keeps (`CHROME-242`).
#[wasm_bindgen_test]
async fn the_panel_header_moves_the_panel_to_the_other_side() {
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    let root = mount_panel(prefs);
    leptos::task::tick().await;
    let dock = q(&root, ".ambre-ctx-head .ambre-ctx-dock");
    assert_eq!(
        dock.get_attribute("aria-label").unwrap(),
        "Dock the panel to the left"
    );
    click(&dock);
    leptos::task::tick().await;
    assert_eq!(prefs.side.get_untracked(), DockSide::Left);
    assert_eq!(
        q(&root, ".ambre-ctx-panel")
            .get_attribute("data-side")
            .unwrap(),
        "left"
    );
    click(&q(&root, ".ambre-ctx-head .ambre-ctx-dock"));
    leptos::task::tick().await;
    assert_eq!(prefs.side.get_untracked(), DockSide::Right);
}
