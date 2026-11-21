//! Wasm tier: the control primitives and the layout of a panel section
//! (`ambre-spec-shell.md` section 2, the visual system).
//!
//! Each test mounts a panel under the shell stylesheet and the generated
//! token sheet, and reads the boxes and the attributes a person would see.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_chrome::commands::{Command, Home, KeyBinding, KeyChar, Locale, Property, Reading};
use ambre_chrome::{
    Choice, ChoiceRow, ChoiceView, CommandButton, ContextPanel, DockSide, Field, Fold, IconButton,
    LengthField, PanelPrefs, PanelSection, PanelWidth, Pressed, PropertyToggle, Select,
    SelectField, native_controls,
};
use ambre_core::index::Pos;
use ambre_core::{Categories, Commands, Mm, PanelSections, PrintLength, Properties, StaticKey};
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const PAGE: StaticKey<PanelSections> = StaticKey::<PanelSections>::new("t.page");

static BOLD: Command = Command {
    id: StaticKey::<Commands>::new("t.format.bold"),
    title: |_: &Locale| ambre_core::Validated::new("Bold"),
    icon: None,
    keybindings: &[KeyBinding::primary(KeyChar::B)],
    category: StaticKey::<Categories>::new("format"),
    home: Home::ContextPanel,
};

static ITALIC: Property = Property {
    id: StaticKey::<Properties>::new("t.italic"),
    title: |_: &Locale| ambre_core::Validated::new("Italic"),
    icon: None,
};

static MARGIN: Property = Property {
    id: StaticKey::<Properties>::new("t.margin"),
    title: |_: &Locale| ambre_core::Validated::new("Margin"),
    icon: None,
};

static PAPER: Property = Property {
    id: StaticKey::<Properties>::new("t.paper"),
    title: |_: &Locale| ambre_core::Validated::new("Paper"),
    icon: None,
};

/// The shell stylesheet and the token sheet, as the page loads them.
const STYLES: &str = include_str!("../../ambre-app/styles.css");
const TOKENS: &str = include_str!("../../ambre-canvas/src/theme.css");

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Put the two stylesheets into the page once.
fn with_styles() {
    if document().get_element_by_id("t-control-styles").is_none() {
        let sheet = document().create_element("style").unwrap();
        sheet.set_id("t-control-styles");
        sheet.set_text_content(Some(&format!("{TOKENS}\n{STYLES}")));
        document().head().unwrap().append_child(&sheet).unwrap();
    }
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

fn q(root: &web_sys::Element, sel: &str) -> web_sys::Element {
    root.query_selector(sel).unwrap().unwrap()
}

fn all(root: &web_sys::Element, sel: &str) -> Vec<web_sys::Element> {
    let list = root.query_selector_all(sel).unwrap();
    (0..list.length())
        .filter_map(|i| list.item(i)?.dyn_into().ok())
        .collect()
}

fn key(el: &web_sys::Element, name: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(name);
    init.set_bubbles(true);
    init.set_cancelable(true);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

fn active() -> Option<web_sys::Element> {
    document().active_element()
}

/// One section that holds the controls of the kit: two fields whose labels
/// differ in length, a row of icons, and a list.
fn kit_section(
    margin: RwSignal<Reading<PrintLength<Mm>>>,
    paper: RwSignal<Reading<u8>>,
) -> Vec<PanelSection> {
    vec![PanelSection::new(
        PAGE,
        "Page",
        icondata::MdiFileDocumentOutline,
        move || {
            view! {
                <Field label="Top">
                    <LengthField property=&MARGIN value=margin on_write=Callback::new(|_| {}) />
                </Field>
                <Field label="A longer label">
                    <SelectField
                        property=&PAPER
                        choices=vec![
                            Choice { value: 0_u8, label: "A4", icon: None },
                            Choice { value: 1_u8, label: "Letter", icon: None },
                        ]
                        value=paper
                        on_write=Callback::new(|_| {})
                    />
                </Field>
                <Field label="Style">
                    <CommandButton
                        command=&BOLD
                        icon=icondata::BsTypeBold
                        on_command=Callback::new(|_| {})
                    />
                    <PropertyToggle
                        property=&ITALIC
                        icon=icondata::BsTypeItalic
                        pressed=Signal::stored(Pressed::On)
                        on_toggle=Callback::new(|()| {})
                    />
                </Field>
            }
            .into_any()
        },
    )]
}

fn mount_kit_panel() -> web_sys::Element {
    with_styles();
    let margin = RwSignal::new(Reading::Uniform(PrintLength::<Mm>::new(20.0)));
    let paper = RwSignal::new(Reading::Uniform(1_u8));
    mount_in_container(move || {
        let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
        view! {
            <div class="ambre-body" style="height:600px;width:900px;display:flex">
                <ContextPanel prefs=prefs title="Document" sections=kit_section(margin, paper) />
            </div>
        }
    })
}

/// The left edge of `el` in the viewport.
fn left(el: &web_sys::Element) -> f64 {
    el.get_bounding_client_rect().left()
}

#[wasm_bindgen_test]
async fn every_label_of_a_section_sits_in_one_column_and_every_control_in_the_next() {
    let root = mount_kit_panel();
    leptos::task::tick().await;
    let labels = all(&root, ".ambre-ctx-section-body .ambre-kit-label");
    let controls = all(&root, ".ambre-ctx-section-body .ambre-kit-control");
    assert_eq!(labels.len(), 3);
    assert_eq!(controls.len(), 3);
    let label_x: Vec<f64> = labels.iter().map(left).collect();
    let control_x: Vec<f64> = controls.iter().map(left).collect();
    assert!(
        label_x.iter().all(|x| (x - label_x[0]).abs() < 0.5),
        "the labels start on one line: {label_x:?}"
    );
    assert!(
        control_x.iter().all(|x| (x - control_x[0]).abs() < 0.5),
        "the controls start on one line, whatever the label: {control_x:?}"
    );
    let widest_label = labels
        .iter()
        .map(|l| l.get_bounding_client_rect().right())
        .fold(f64::MIN, f64::max);
    assert!(
        widest_label <= control_x[0],
        "no label runs under a control"
    );
    root.remove();
}

#[wasm_bindgen_test]
async fn every_icon_of_the_panel_sits_in_a_box_of_one_size() {
    let root = mount_kit_panel();
    leptos::task::tick().await;
    // The open frame: its rail is out of the layout.
    let buttons = all(&root, ".ambre-ctx-body .ambre-icon-button");
    // The collapse button, the bold command and the italic property.
    assert!(buttons.len() >= 3, "{} icon buttons", buttons.len());
    let section_boxes: Vec<(f64, f64)> = all(&root, ".ambre-ctx-section-body .ambre-icon-button")
        .iter()
        .map(|b| {
            let r = b.get_bounding_client_rect();
            (r.width(), r.height())
        })
        .collect();
    assert_eq!(section_boxes.len(), 2);
    assert!(
        section_boxes.iter().all(|b| *b == (28.0, 28.0)),
        "one box, the control height: {section_boxes:?}"
    );
    for button in &buttons {
        let glyph = q(button, "svg").get_bounding_client_rect();
        assert_eq!(
            (glyph.width(), glyph.height()),
            (16.0, 16.0),
            "one glyph size"
        );
        let style = web_sys::window()
            .unwrap()
            .get_computed_style(button)
            .unwrap()
            .unwrap();
        // The border of the toolkit, never the outset frame of the browser.
        assert_eq!(
            style.get_property_value("border-top-style").unwrap(),
            "solid",
            "no native frame"
        );
    }
    // The frame's own buttons are icon buttons too, and keep their hook.
    for hook in [
        ".ambre-ctx-collapse",
        ".ambre-ctx-open",
        ".ambre-ctx-rail-icon",
    ] {
        let class = q(&root, hook).get_attribute("class").unwrap();
        assert!(class.contains("ambre-icon-button"), "{hook}: {class}");
    }
    assert_eq!(native_controls(&root), Vec::<String>::new());
    root.remove();
}

#[wasm_bindgen_test]
async fn an_icon_button_names_itself_and_its_chord_in_a_tooltip_under_the_focus() {
    with_styles();
    let pressed = RwSignal::new(0_u32);
    let root = mount_in_container(move || {
        view! {
            <IconButton
                icon=icondata::BsTypeBold
                label="Bold"
                chord=Some(KeyBinding::primary(KeyChar::B))
                on_press=Callback::new(move |()| pressed.update(|n| *n += 1))
            />
        }
    });
    leptos::task::tick().await;
    let button: web_sys::HtmlElement = q(&root, ".ambre-icon-button").dyn_into().unwrap();
    let tip = q(&root, ".ambre-tip");
    assert_eq!(tip.get_attribute("data-shown").as_deref(), Some("false"));
    button.focus().unwrap();
    leptos::task::tick().await;
    assert_eq!(tip.get_attribute("data-shown").as_deref(), Some("true"));
    let text = tip.text_content().unwrap();
    assert!(text.contains("Bold"), "the name: {text}");
    assert!(q(&tip, ".ambre-kbd").is_object(), "the chord chip");
    assert_eq!(button.get_attribute("aria-label").as_deref(), Some("Bold"));
    assert!(button.get_attribute("aria-keyshortcuts").is_some());
    button.click();
    assert_eq!(pressed.get_untracked(), 1);
    leptos::task::tick().await;
    assert_eq!(tip.get_attribute("data-shown").as_deref(), Some("false"));
    root.remove();
}

#[wasm_bindgen_test]
async fn a_select_is_operable_from_the_keyboard_alone() {
    with_styles();
    let picked = RwSignal::new(None::<Pos<ChoiceRow>>);
    let selected = RwSignal::new(Some(Pos::<ChoiceRow>::from_usize(0)));
    let root = mount_in_container(move || {
        view! {
            <Select
                label="Paper"
                choices=vec![
                    ChoiceView::text("A4"),
                    ChoiceView::text("A5"),
                    ChoiceView::text("Letter"),
                ]
                selected=selected
                on_pick=Callback::new(move |p| picked.set(Some(p)))
            />
        }
    });
    leptos::task::tick().await;
    let trigger: web_sys::HtmlElement = q(&root, ".ambre-select-trigger").dyn_into().unwrap();
    let list = q(&root, "[role=listbox]");
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("false")
    );
    assert!(list.has_attribute("hidden"));
    assert_eq!(trigger.text_content().unwrap().trim(), "A4");

    // A click opens the list and leaves the focus where it was.
    let before = active();
    trigger.click();
    leptos::task::tick().await;
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("true")
    );
    assert_eq!(active(), before, "the pointer moves no focus");
    trigger.click();
    leptos::task::tick().await;
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("false")
    );

    trigger.focus().unwrap();
    key(&trigger, "ArrowDown");
    leptos::task::tick().await;
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("true")
    );
    assert!(!list.has_attribute("hidden"));
    let options = all(&list, "[role=option]");
    assert_eq!(
        active().as_ref(),
        options.first(),
        "the current choice takes the focus"
    );

    key(&options[0], "ArrowDown");
    assert_eq!(active().as_ref(), options.get(1));
    key(&options[1], "l");
    assert_eq!(active().as_ref(), options.get(2), "a first letter jumps");
    key(&options[2], "Enter");
    leptos::task::tick().await;
    assert_eq!(
        picked.get_untracked(),
        Some(Pos::<ChoiceRow>::from_usize(2))
    );
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("false")
    );
    assert_eq!(
        active().as_ref(),
        Some(trigger.as_ref()),
        "the focus goes back"
    );

    // Escape closes and picks nothing.
    picked.set(None);
    key(&trigger, "Enter");
    leptos::task::tick().await;
    key(&all(&list, "[role=option]")[0], "Escape");
    leptos::task::tick().await;
    assert_eq!(
        trigger.get_attribute("aria-expanded").as_deref(),
        Some("false")
    );
    assert_eq!(picked.get_untracked(), None);

    // A mixed selection shows the placeholder and selects no option.
    selected.set(None);
    leptos::task::tick().await;
    assert_eq!(trigger.text_content().unwrap().trim(), "Mixed");
    assert!(all(&list, "[aria-selected=true]").is_empty());
    root.remove();
}
