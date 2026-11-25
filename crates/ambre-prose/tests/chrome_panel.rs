//! The Prose comment panel, mounted in headless Chrome
//! (`features/ambre-spec-prose-comments.md section 4`).
//!
//! The chrome is wasm-only Leptos, so these tests run under
//! `wasm-bindgen-test-runner` in headless Chrome (`just test-wasm`).
//! On the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::UiEvent;
use ambre_bus::{Authored, Instant};
use ambre_core::id::{Comment, Document, Id, Reply, User};
use ambre_core::index::Pos;
use ambre_prose::chrome::{self, ProseCommentsPanel, ProseSignals};
use ambre_prose::events::{CommentReply, CommentThread, ProseEvent};
use ambre_prose::message::{DocChar, ProseError, ProseRefusal};
use ambre_prose::ui::ProseUiState;
use ambre_prose::view::annotate::{LostReply, PanelThread};
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

/// The events the chrome reads in these tests: ProseEvent documents, no channel.
type Event = UiEvent<ProseEvent>;

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

fn thread(id: Id<Comment>, replies: &[&str]) -> CommentThread {
    CommentThread {
        comment_id: id,
        anchor_block: None,
        anchor_start: Pos::<DocChar>::ZERO,
        anchor_end: Pos::<DocChar>::new(3),
        orphaned: ambre_prose::message::AnchorState::Anchored,
        resolved: ambre_core::ThreadState::Open,
        replies: replies
            .iter()
            .map(|text| CommentReply {
                reply_id: Id::<Reply>::mint(),
                author: ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").ok(),
                user_id: Id::<User>::from_bytes([2; 16]),
                created_at: Instant::<Authored>::new(0),
                text: (*text).to_owned(),
            })
            .collect(),
    }
}

fn input_of(root: &web_sys::Element, nth: u32) -> web_sys::HtmlInputElement {
    root.query_selector_all(".ambre-prose-thread-input")
        .unwrap()
        .item(nth)
        .expect("a reply field")
        .dyn_into()
        .unwrap()
}

/// `PROSE-UI-231`: half a reply typed in a field, then the thread moves down
/// and a peer replies. The field is the same element, focused, with its text.
#[wasm_bindgen_test]
async fn a_reply_being_typed_survives_its_thread_moving() {
    let signals = ProseSignals::new();
    let (first, second) = (Id::<Comment>::mint(), Id::<Comment>::mint());
    signals.threads.set(vec![
        PanelThread {
            thread: thread(first, &["Is this right?"]),
            y: ambre_core::Px::<ambre_core::View>::new(100.0),
        },
        PanelThread {
            thread: thread(second, &["Typo"]),
            y: ambre_core::Px::<ambre_core::View>::new(300.0),
        },
    ]);
    let root = mount_in_container(move || view! { <ProseCommentsPanel signals=signals /> });

    let field = input_of(&root, 1);
    field.focus().unwrap();
    field.set_value("half a rep");
    field
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();

    // A scroll moves both threads, and a peer's reply grows the first.
    signals.threads.set(vec![
        PanelThread {
            thread: thread(first, &["Is this right?", "Yes"]),
            y: ambre_core::Px::<ambre_core::View>::new(40.0),
        },
        PanelThread {
            thread: thread(second, &["Typo"]),
            y: ambre_core::Px::<ambre_core::View>::new(240.0),
        },
    ]);
    // The effects that redraw the cards run on the next tick.
    leptos::task::tick().await;

    let after = input_of(&root, 1);
    assert!(after.is_same_node(Some(&field)), "the card was not rebuilt");
    assert_eq!(after.value(), "half a rep", "the draft is kept");
    let focused = document().active_element().expect("an element has focus");
    assert!(
        focused.is_same_node(Some(&field)),
        "the field keeps the focus"
    );
    let replies = root.query_selector_all(".ambre-prose-reply").unwrap();
    assert_eq!(replies.length(), 3, "the peer's reply is shown in place");
    let card: web_sys::HtmlElement = root
        .query_selector_all(".ambre-prose-thread")
        .unwrap()
        .item(1)
        .unwrap()
        .dyn_into()
        .unwrap();
    assert_eq!(
        card.style().get_property_value("top").unwrap(),
        "240px",
        "the card moved with its anchor"
    );
    root.remove();
}

/// `PROSE-UI-232`: a peer deletes the thread whose field holds text. The
/// draft goes, and the user is told the reply was not posted.
#[wasm_bindgen_test]
fn a_thread_deleted_under_a_draft_is_told() {
    let owner = Owner::new();
    owner.set();
    let signals = ProseSignals::new();
    let (kept, gone) = (Id::<Comment>::mint(), Id::<Comment>::mint());
    signals.drafts.update(|drafts| {
        drafts.insert(kept, "still here".to_owned());
        drafts.insert(gone, "half a reply".to_owned());
    });
    let mut view = ProseUiState::default();
    view.apply_event(&ProseEvent::CommentsUpdated {
        threads: vec![thread(kept, &["a"]), thread(gone, &["b"])],
    });
    assert!(signals.drop_lost_drafts(&view).is_empty());

    view.apply_event(&ProseEvent::CommentsUpdated {
        threads: vec![thread(kept, &["a"])],
    });
    let lost = signals.drop_lost_drafts(&view);
    assert_eq!(lost, [LostReply::Deleted]);
    assert!(
        lost[0].notice().contains("not posted"),
        "{}",
        lost[0].notice()
    );
    signals.drafts.with_untracked(|drafts| {
        assert_eq!(drafts.get(&kept).map(String::as_str), Some("still here"));
        assert!(!drafts.contains_key(&gone), "the lost draft is dropped");
    });
}

/// `PROSE-UI-232`: a reply sent as a peer deleted its thread is refused, and
/// the refusal reaches the user as a toast.
#[wasm_bindgen_test]
fn a_reply_refused_for_its_thread_reaches_the_user() {
    let owner = Owner::new();
    owner.set();
    let toasts = ambre_chrome::Toasts::new();
    let comment_id = Id::<Comment>::mint();
    let reply = ambre_prose::message::ProseCommand::ReplyToComment {
        comment_id,
        text: String::new(),
        now: Instant::<Authored>::new(0),
    };
    chrome::notify_event(
        &toasts,
        &Event::Viewport {
            document_id: Id::<Document>::mint(),
            event: ProseEvent::Error(ProseError::new(
                ambre_prose::message::RefusedAction::Command(reply.name()),
                ProseRefusal::NoSuchComment(comment_id),
            )),
        },
    );
    let root = mount_in_container(move || view! { <ambre_chrome::ToastHost toasts=toasts /> });
    let text = root
        .query_selector(".ambre-toast")
        .unwrap()
        .expect("the refusal is on screen")
        .text_content()
        .unwrap_or_default();
    assert!(text.contains("Your reply was not posted"), "{text}");
    root.remove();
}

/// `INTEROP-007`: what an export lost reaches the user as the shared
/// sentence, on screen and not in the log alone.
#[wasm_bindgen_test]
fn an_interop_report_reaches_the_user() {
    use ambre_bus::interop::{Direction, ExportedFile, FileFormat, InteropAnswer, Outcome};
    use ambre_prose::message::{ConversionReport, ProseConstruct, UnsupportedFeature};
    let owner = Owner::new();
    owner.set();
    let toasts = ambre_chrome::Toasts::new();
    let mut report = ConversionReport::new(Direction::Export, FileFormat::Markdown);
    report.record(
        ProseConstruct::Feature(UnsupportedFeature::Footnote),
        Outcome::Dropped,
    );
    report.record(
        ProseConstruct::Feature(UnsupportedFeature::Footnote),
        Outcome::Dropped,
    );
    chrome::notify_event(
        &toasts,
        &Event::Viewport {
            document_id: Id::<Document>::mint(),
            event: ProseEvent::Interop(InteropAnswer::ExportReady {
                file: ExportedFile {
                    filename: ambre_client_core::ExportFilename::named(
                        "notes",
                        ambre_client_core::ExportExtension::Markdown,
                    ),
                    media_type: ambre_core::MediaType::from("text/markdown"),
                    data: ambre_core::Payload::<ambre_client_core::FileBytes>::default(),
                },
                report,
            }),
        },
    );
    let root = mount_in_container(move || view! { <ambre_chrome::ToastHost toasts=toasts /> });
    let text = root
        .query_selector(".ambre-toast")
        .unwrap()
        .expect("the report is on screen")
        .text_content()
        .unwrap_or_default();
    assert!(text.contains("notes.md"), "{text}");
    assert!(text.contains("footnote (2)"), "{text}");
    root.remove();
}

/// The ruler element sits on the column the view gave it, moved onto the page
/// by the canvas origin. Each handle is centred on the position the state
/// gives it (`PROSE-UI-065`). The element adds no offset of its own, at any
/// zoom.
#[wasm_bindgen_test]
async fn the_ruler_element_sits_on_the_column_it_was_given() {
    use ambre_core::{Css, Point, Px, View};
    use ambre_core::{PrintLength, Pt};
    use ambre_prose::view::ruler::{RulerHandle, RulerHandles, RulerState};
    let origin = Point::new(Px::<Css>::new(40.0), Px::<Css>::new(30.0));
    for zoom in [0.5_f32, 1.0, 2.0] {
        let signals = ProseSignals::new();
        let ruler = RulerState {
            visible: RulerHandles::Visible,
            page_left: Px::<View>::new(80.0),
            content_width: Px::<View>::new(451.0 * zoom),
            point_width: Px::<View>::new(4.0 / 3.0 * zoom),
            indent_first: PrintLength::<Pt>::new(18.0),
            indent_left: PrintLength::<Pt>::new(36.0),
            indent_right: PrintLength::<Pt>::new(12.0),
        };
        signals.canvas_origin.set(origin);
        signals.ruler.set(ruler);
        let container =
            mount_in_container(move || view! { <chrome::ProseRuler signals=signals /> });
        leptos::task::tick().await;

        let rect = container
            .query_selector(".ambre-prose-ruler")
            .unwrap()
            .expect("the ruler is on screen")
            .get_bounding_client_rect();
        assert_eq!(rect.left() as f32, 120.0, "zoom {zoom}");
        assert_eq!(rect.top() as f32, 30.0, "zoom {zoom}");
        assert!(
            (rect.width() as f32 - ruler.content_width.get()).abs() < 0.5,
            "zoom {zoom}"
        );
        let handles = container
            .query_selector_all(".ambre-prose-ruler-handle")
            .unwrap();
        assert_eq!(handles.length(), 3);
        let order = [
            RulerHandle::IndentFirst,
            RulerHandle::IndentLeft,
            RulerHandle::IndentRight,
        ];
        for (i, handle) in order.into_iter().enumerate() {
            let el: web_sys::Element = handles.item(i as u32).unwrap().dyn_into().unwrap();
            let r = el.get_bounding_client_rect();
            let centre = (r.left() + r.width() / 2.0) as f32;
            let expected = ruler.handle_x(handle).on_page(origin.x).get();
            assert!(
                (centre - expected).abs() < 0.5,
                "zoom {zoom}: {handle:?} at {centre}"
            );
        }
        container.remove();
    }
}

/// Each control of the format bar announces the title its command's
/// description carries (`UI-THREAD-036`).
#[wasm_bindgen_test]
async fn the_format_bar_announces_the_registry_labels() {
    let signals = ProseSignals::new();
    signals
        .toolbar
        .set(ambre_prose::view::toolbar::ToolbarState {
            anchor: Some(ambre_core::Point::new(
                ambre_core::Px::<ambre_core::View>::new(200.0),
                ambre_core::Px::<ambre_core::View>::new(100.0),
            )),
            active: None,
        });
    let root = mount_in_container(
        move || view! { <chrome::ProseFormatBar signals=signals cover=RwSignal::new(ambre_client_core::cover::Cover::default()) /> },
    );
    leptos::task::tick().await;

    let ids = ambre_prose::commands::BAR_MARKS
        .map(|(id, _)| id)
        .into_iter()
        .chain([ambre_prose::commands::LINK, ambre_prose::commands::COMMENT]);
    for id in ids {
        let command = ambre_commands::find(ambre_prose::commands::COMMANDS, id)
            .expect("the control is described");
        let expected = (command.title)(&ambre_commands::Locale::En);
        let found = root
            .query_selector(&format!("[aria-label=\"{expected}\"]"))
            .unwrap();
        assert!(found.is_some(), "{id:?}: no control announces {expected:?}");
    }
    // `CHROME-185`, `CHROME-186`: each icon names itself in a tooltip, with
    // the chord and the description its command declares.
    assert_eq!(
        ambre_chrome::untipped_icons(&root, ambre_prose::commands::COMMANDS),
        Vec::<String>::new()
    );
    // `CHROME-187`: the keyboard focus shows the tooltip of an icon.
    let icon: web_sys::HtmlElement = root
        .query_selector(".ambre-icon-button[data-reach^=\"command:\"]")
        .unwrap()
        .expect("an icon that reaches a command")
        .dyn_into()
        .unwrap();
    let tip = icon.query_selector(".ambre-tip").unwrap().unwrap();
    assert_eq!(tip.get_attribute("data-shown").as_deref(), Some("false"));
    icon.focus().unwrap();
    leptos::task::tick().await;
    assert_eq!(tip.get_attribute("data-shown").as_deref(), Some("true"));
    root.remove();
}

/// `UNDO-006`: the history the shell's undo and redo affordances read follows
/// the state the Logic thread reports. The page counts nothing itself: an
/// event that is not a history state leaves it as it was. Undo and redo live
/// in the Ambre menu, outside the page's context menu.
#[wasm_bindgen_test]
async fn the_history_follows_the_reported_state() {
    let signals = ProseSignals::new();
    let report = |event: ProseEvent| {
        signals.mirror_event(&Event::Viewport {
            document_id: Id::<Document>::mint(),
            event,
        });
    };
    let disabled = ambre_core::Availability::Disabled;
    let enabled = ambre_core::Availability::Enabled;
    assert_eq!(
        signals.history.get_untracked().undo,
        disabled,
        "a fresh page has no history"
    );
    assert_eq!(signals.history.get_untracked().redo, disabled);
    report(ProseEvent::UndoState {
        undo: enabled,
        redo: disabled,
    });
    assert_eq!(signals.history.get_untracked().undo, enabled);
    assert_eq!(signals.history.get_untracked().redo, disabled);
    let before = signals.history.get_untracked();
    report(ProseEvent::Synced {
        acked: ambre_core::Seq::<ambre_prose::message::TextAck>::nth(1),
    });
    assert_eq!(signals.history.get_untracked(), before);
    // The page's context menu holds no history entry.
    assert!(
        chrome::context_entries()
            .iter()
            .all(|e| !matches!(e, ambre_chrome::MenuEntry::Item(i) if i.label() == "Undo" || i.label() == "Redo"))
    );
}

// -- The context panel (`ambre-spec-shell.md` section 2) ---------------------

/// Mount the page's context panel over `signals`, and return the container.
fn mount_prose_panel(signals: ProseSignals) -> web_sys::Element {
    use ambre_chrome::{ContextPanel, DockSide, Fold, PanelPrefs, PanelWidth};
    let content = chrome::prose_panel(signals);
    let prefs = PanelPrefs::new(DockSide::Right, PanelWidth::DEFAULT, Fold::Open);
    mount_in_container(move || {
        view! { <ContextPanel prefs=prefs title=content.title sections=content.sections cover=RwSignal::new(ambre_client_core::cover::Cover::default()) /> }
    })
}

/// With no text selected the panel shows the page: its paper, its
/// orientation and its four margins, as the layout holds them.
#[wasm_bindgen_test]
async fn with_no_selection_the_panel_shows_the_page_the_layout_holds() {
    use ambre_core::{Mm, PrintLength};
    use ambre_prose::doc::PageFormat;
    use ambre_prose::view::panel;
    let signals = ProseSignals::new();
    let wide = PageFormat {
        width: PrintLength::<Mm>::new(297.0),
        height: PrintLength::<Mm>::new(210.0),
        margin_left: PrintLength::<Mm>::new(31.0),
        ..PageFormat::default()
    };
    signals.page.set(Some(wide));
    let root = mount_prose_panel(signals);
    leptos::task::tick().await;
    assert_eq!(
        root.query_selector(".ambre-ctx-title")
            .unwrap()
            .unwrap()
            .text_content()
            .unwrap(),
        "Document"
    );
    assert_eq!(ambre_chrome::repeated_title(&root), None);
    assert_eq!(ambre_chrome::native_controls(&root), Vec::<String>::new());
    assert_eq!(
        ambre_chrome::unreached_controls(&root),
        Vec::<String>::new()
    );
    let markers = ambre_chrome::reach_markers(&root);
    for id in [
        panel::PAPER,
        panel::ORIENTATION,
        panel::MARGIN_TOP,
        panel::MARGIN_BOTTOM,
        panel::MARGIN_LEFT,
        panel::MARGIN_RIGHT,
        panel::MARGINS,
    ] {
        let reach = ambre_chrome::commands::Reach::Property(id);
        assert!(
            markers.iter().any(|m| reach.matches_marker(m)),
            "{id:?} shows"
        );
    }
    let checked = root
        .query_selector(".ambre-kit-segmented [aria-checked=\"true\"]")
        .unwrap()
        .unwrap();
    assert_eq!(checked.get_attribute("aria-label").unwrap(), "Landscape");
    let left: web_sys::HtmlInputElement = root
        .query_selector("[aria-label=\"Left margin\"]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert_eq!(left.value(), "31", "a whole length shows no decimal");
    // Margins no preset names read as custom, and the four steppers show.
    let preset = root
        .query_selector("[aria-label=\"Margins\"] .ambre-select-value")
        .unwrap()
        .unwrap()
        .text_content()
        .unwrap();
    assert_eq!(preset, "Custom...");
    // Portrait and landscape draw two glyphs of distinct shapes.
    let glyph = |name: &str| {
        root.query_selector(&format!("[role=\"radio\"][aria-label=\"{name}\"] svg"))
            .unwrap()
            .unwrap()
            .inner_html()
    };
    assert_ne!(glyph("Portrait"), glyph("Landscape"));
    root.remove();
}

/// The margins show as one list of presets. A preset hides the four
/// steppers and hands its four margins to the view; custom shows them.
#[wasm_bindgen_test]
async fn a_margin_preset_hides_the_steppers_and_custom_shows_them() {
    use ambre_core::{Mm, PrintLength};
    use ambre_prose::doc::PageFormat;
    let signals = ProseSignals::new();
    signals.page.set(Some(PageFormat::default()));
    let root = mount_prose_panel(signals);
    leptos::task::tick().await;
    let steppers = || {
        root.query_selector("[aria-label=\"Top margin\"]")
            .unwrap()
            .is_some()
    };
    assert!(!steppers(), "a preset hides the steppers");
    let option = |label: &str| -> web_sys::HtmlElement {
        let list = root
            .query_selector("[role=\"listbox\"][aria-label=\"Margins\"]")
            .unwrap()
            .unwrap();
        let found = list.query_selector_all("[role=\"option\"]").unwrap();
        (0..found.length())
            .filter_map(|i| found.item(i))
            .filter_map(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
            .find(|o| o.text_content().unwrap_or_default() == label)
            .unwrap()
    };
    option("Wide").click();
    let asked = signals.page_write.get_untracked().expect("a format");
    assert_eq!(asked.margin_left, PrintLength::<Mm>::new(50.8));
    assert_eq!(asked.margin_top, PrintLength::<Mm>::new(25.4));
    assert_eq!(asked.width, PageFormat::default().width, "the paper stays");
    option("Custom...").click();
    leptos::task::tick().await;
    assert!(steppers(), "custom shows the steppers");
    root.remove();
}

/// A page control hands the whole format back to the view, one field changed.
#[wasm_bindgen_test]
async fn a_page_control_hands_the_whole_format_to_the_view() {
    use ambre_core::{Mm, PrintLength};
    use ambre_prose::doc::PageFormat;
    let signals = ProseSignals::new();
    signals.page.set(Some(PageFormat::default()));
    let root = mount_prose_panel(signals);
    leptos::task::tick().await;
    // A pick of custom shows the steppers before any margin moves.
    let custom: web_sys::HtmlElement = root
        .query_selector("[role=\"listbox\"][aria-label=\"Margins\"] [role=\"option\"]:last-child")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    custom.click();
    assert_eq!(
        signals.page_write.get_untracked(),
        None,
        "custom writes nothing"
    );
    leptos::task::tick().await;
    let more: web_sys::HtmlElement = root
        .query_selector("[aria-label=\"Top margin: more\"]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    more.click();
    let asked = signals
        .page_write
        .get_untracked()
        .expect("a format is asked for");
    assert_eq!(
        asked,
        PageFormat {
            margin_top: PageFormat::default().margin_top + PrintLength::<Mm>::new(1.0),
            ..PageFormat::default()
        }
    );
    // The orientation turns the page whole: the dimensions swap.
    let landscape: web_sys::HtmlElement = root
        .query_selector("[aria-label=\"Landscape\"]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    landscape.click();
    let turned = signals.page_write.get_untracked().unwrap();
    assert_eq!(turned.width, PageFormat::default().height);
    root.remove();
}

/// With a selection the panel shows the text: the marks, superscript,
/// subscript and clearing the formatting, each handing its id to the view
/// (`PROSE-UI-191`).
#[wasm_bindgen_test]
async fn superscript_subscript_and_clearing_reach_a_control_of_the_panel() {
    use ambre_prose::commands::{CLEAR_FORMATTING, SUBSCRIPT, SUPERSCRIPT};
    use ambre_prose::view::panel::{PanelKind, TEXT_COMMANDS};
    let signals = ProseSignals::new();
    signals
        .toolbar
        .set(ambre_prose::view::toolbar::ToolbarState {
            anchor: Some(ambre_core::Point::default()),
            active: None,
        });
    let root = mount_prose_panel(signals);
    leptos::task::tick().await;
    assert_eq!(
        root.query_selector(".ambre-ctx-title")
            .unwrap()
            .unwrap()
            .text_content()
            .unwrap(),
        "Selection"
    );
    assert_eq!(ambre_chrome::repeated_title(&root), None);
    assert_eq!(
        ambre_chrome::unreached_controls(&root),
        Vec::<String>::new()
    );
    let markers = ambre_chrome::reach_markers(&root);
    for id in TEXT_COMMANDS {
        let reach = ambre_chrome::commands::Reach::Command(id);
        assert!(
            markers.iter().any(|m| reach.matches_marker(m)),
            "{id:?} shows"
        );
    }
    for entry in PanelKind::Text.bar() {
        assert!(
            markers.iter().any(|m| entry.matches_marker(m)),
            "{entry:?} is in the panel"
        );
    }
    for id in [SUPERSCRIPT, SUBSCRIPT, CLEAR_FORMATTING] {
        let reach = ambre_chrome::commands::Reach::Command(id).marker();
        let button: web_sys::HtmlElement = root
            .query_selector(&format!("[data-reach=\"{reach}\"]"))
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        button.click();
        assert_eq!(signals.format.get_untracked(), Some(id));
    }
    root.remove();
}

/// `PROSE-UI-273`, `CHROME-183`: with the caret in a suggestion and no
/// pointer over it, the panel shows a section that accepts and rejects that
/// suggestion. Each button is a declared command the panel names, and a press
/// hands the view the popover's own intent. The section leaves with the caret.
#[wasm_bindgen_test]
async fn the_caret_in_a_suggestion_shows_its_accept_and_reject_in_the_panel() {
    use ambre_core::id::Suggestion;
    use ambre_core::{Px, View};
    use ambre_prose::commands::{ACCEPT_SUGGESTION, REJECT_SUGGESTION};
    use ambre_prose::events::SuggestionKind;
    use ambre_prose::view::annotate::{AnnotationIntent, HoveredSuggestion, ScreenRect};
    let signals = ProseSignals::new();
    let root = mount_prose_panel(signals);
    leptos::task::tick().await;
    let reach = |id| ambre_chrome::commands::Reach::Command(id).marker();
    let button = |id| {
        root.query_selector(&format!("[data-reach=\"{}\"]", reach(id)))
            .unwrap()
    };
    assert!(
        button(ACCEPT_SUGGESTION).is_none(),
        "no suggestion, no section"
    );

    let suggestion_id = Id::<Suggestion>::mint();
    let at = |x: f32| Px::<View>::new(x);
    signals.at_caret.set(Some(HoveredSuggestion {
        suggestion_id,
        kind: SuggestionKind::Insertion,
        rect: ScreenRect {
            x0: at(10.0),
            y0: at(10.0),
            x1: at(20.0),
            y1: at(30.0),
        },
    }));
    leptos::task::tick().await;
    assert_eq!(ambre_chrome::repeated_title(&root), None);
    assert_eq!(
        ambre_chrome::unreached_controls(&root),
        Vec::<String>::new()
    );
    for (id, intent) in [
        (
            ACCEPT_SUGGESTION,
            AnnotationIntent::Accept { suggestion_id },
        ),
        (
            REJECT_SUGGESTION,
            AnnotationIntent::Reject { suggestion_id },
        ),
    ] {
        let control: web_sys::HtmlElement =
            button(id).expect("the control shows").dyn_into().unwrap();
        // A control in the tab order is reachable from the keyboard.
        assert!(control.tab_index() >= 0, "{id:?} takes the focus");
        control.click();
        assert_eq!(signals.intent.get_untracked(), Some(intent));
        signals.intent.set(None);
    }

    signals.at_caret.set(None);
    leptos::task::tick().await;
    assert!(button(REJECT_SUGGESTION).is_none(), "the section leaves");
    root.remove();
}

/// `PROSE-UI-275`, `PROSE-UI-276`: the review section shows the
/// suggestion-mode control over the page, in the tab order. It is pressed
/// exactly while the reported mode is suggesting. A press, the chord and the
/// palette each hand the view the toggle.
#[wasm_bindgen_test]
async fn the_suggestion_mode_control_shows_the_mode_and_hands_its_toggle() {
    use ambre_chrome::{CommandHandler, KeyRoute, KeyTarget};
    use ambre_prose::commands::SUGGEST_CHANGES;
    use ambre_prose::events::EditMode;
    use ambre_prose::view::annotate::AnnotationIntent;
    let signals = ProseSignals::new();
    let root = mount_prose_panel(signals);
    leptos::task::tick().await;
    assert_eq!(ambre_chrome::repeated_title(&root), None);
    assert_eq!(
        ambre_chrome::unreached_controls(&root),
        Vec::<String>::new()
    );
    let marker = ambre_chrome::commands::Reach::Command(SUGGEST_CHANGES).marker();
    let control: web_sys::HtmlElement = root
        .query_selector(&format!("[data-reach=\"{marker}\"]"))
        .unwrap()
        .expect("the control shows over the page")
        .dyn_into()
        .unwrap();
    assert!(control.tab_index() >= 0, "the control takes the focus");
    assert_eq!(control.get_attribute("aria-pressed").unwrap(), "false");
    control.click();
    assert_eq!(
        signals.intent.get_untracked(),
        Some(AnnotationIntent::ToggleSuggesting)
    );
    signals.intent.set(None);
    signals.mode.set(EditMode::Suggesting);
    leptos::task::tick().await;
    assert_eq!(control.get_attribute("aria-pressed").unwrap(), "true");

    // The chord and the palette take the control's path.
    let handler = chrome::ProseHandler {
        format: signals.format,
        at_caret: signals.at_caret,
        intent: signals.intent,
        now: || Instant::<Authored>::new(0),
        zone: ambre_prose::message::TimeZone::default,
    };
    assert!(matches!(
        handler.on_key(SUGGEST_CHANGES, KeyTarget::Canvas),
        KeyRoute::Run(actions) if actions.is_empty()
    ));
    assert_eq!(
        signals.intent.get_untracked(),
        Some(AnnotationIntent::ToggleSuggesting)
    );
    signals.intent.set(None);
    assert!(handler.on_command(SUGGEST_CHANGES).is_empty());
    assert_eq!(
        signals.intent.get_untracked(),
        Some(AnnotationIntent::ToggleSuggesting)
    );
    root.remove();
}

/// `PROSE-UI-275`: the status chip shows while the session suggests and
/// leaves when it stops, so a folded panel never hides the mode. The chip
/// is a pressed reach of the command, and a press hands the toggle.
#[wasm_bindgen_test]
async fn the_status_chip_shows_while_suggesting_alone() {
    use ambre_prose::commands::SUGGEST_CHANGES;
    use ambre_prose::events::EditMode;
    use ambre_prose::view::annotate::AnnotationIntent;
    let signals = ProseSignals::new();
    let root = mount_in_container(move || view! { <chrome::ProseModeChip signals=signals /> });
    leptos::task::tick().await;
    let marker = ambre_chrome::commands::Reach::Command(SUGGEST_CHANGES).marker();
    let chip = || {
        root.query_selector(&format!("[role=\"status\"] [data-reach=\"{marker}\"]"))
            .unwrap()
    };
    assert!(chip().is_none(), "no chip while editing");
    signals.mode.set(EditMode::Suggesting);
    leptos::task::tick().await;
    let shown: web_sys::HtmlElement = chip().expect("the chip shows").dyn_into().unwrap();
    assert_eq!(shown.get_attribute("aria-pressed").unwrap(), "true");
    assert!(shown.tab_index() >= 0, "the chip takes the focus");
    assert!(
        shown.text_content().unwrap().contains("Suggest changes"),
        "the chip names the mode in words"
    );
    shown.click();
    assert_eq!(
        signals.intent.get_untracked(),
        Some(AnnotationIntent::ToggleSuggesting)
    );
    signals.mode.set(EditMode::Editing);
    leptos::task::tick().await;
    assert!(chip().is_none(), "the chip leaves with the mode");
    root.remove();
}

/// The selection the panel tests raise: an anchor, and the values `values`
/// the Logic thread read for it.
fn selection_with(signals: ProseSignals, values: ambre_prose::message::SelectionValues) {
    signals
        .toolbar
        .set(ambre_prose::view::toolbar::ToolbarState {
            anchor: Some(ambre_core::Point::default()),
            active: Some(ambre_prose::message::ActiveMarks {
                values,
                ..ambre_prose::message::ActiveMarks::default()
            }),
        });
}

/// The radio of a segmented control labelled `label`.
fn radio(root: &web_sys::Element, label: &str) -> web_sys::HtmlElement {
    root.query_selector(&format!("[role=\"radio\"][aria-label=\"{label}\"]"))
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap()
}

/// With a selection the panel shows four sections: text, paragraph,
/// paragraph style and actions. Each control reads the value the Logic
/// thread read, and hands each write to the view as one value
/// (`PROSE-UI-258`, `PROSE-UI-259`, `PROSE-UI-189`, `PROSE-UI-260`).
#[wasm_bindgen_test]
async fn with_a_selection_the_panel_shows_each_text_and_paragraph_control() {
    use ambre_chrome::commands::Reading;
    use ambre_prose::events::{Align, ListStyle, ParagraphStyle};
    use ambre_prose::message::{HeadingLevel, ParagraphKind, SelectionValues};
    use ambre_prose::view::panel::{self, PickedSelection, TextValue};
    let signals = ProseSignals::new();
    selection_with(
        signals,
        SelectionValues {
            align: Reading::Uniform(Align::Center),
            kind: Reading::Uniform(Some(ParagraphKind::Heading(HeadingLevel::H2))),
            ..SelectionValues::default()
        },
    );
    let root = mount_prose_panel(signals);
    leptos::task::tick().await;
    assert_eq!(ambre_chrome::repeated_title(&root), None);
    assert_eq!(ambre_chrome::native_controls(&root), Vec::<String>::new());
    assert_eq!(
        ambre_chrome::unreached_controls(&root),
        Vec::<String>::new()
    );
    let markers = ambre_chrome::reach_markers(&root);
    for id in [
        panel::FONT,
        panel::SIZE,
        panel::COLOUR,
        panel::HIGHLIGHT,
        panel::ALIGN,
        panel::LINE_SPACING,
        panel::INDENT_LEFT,
        panel::INDENT_FIRST,
        panel::INDENT_OUT,
        panel::INDENT_IN,
        panel::KIND,
    ] {
        let reach = ambre_chrome::commands::Reach::Property(id);
        assert!(
            markers.iter().any(|m| reach.matches_marker(m)),
            "{id:?} shows"
        );
    }
    for id in panel::ACTION_COMMANDS {
        let reach = ambre_chrome::commands::Reach::Command(id);
        assert!(
            markers.iter().any(|m| reach.matches_marker(m)),
            "{id:?} shows"
        );
    }
    assert_eq!(
        radio(&root, "Centre")
            .get_attribute("aria-checked")
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        radio(&root, "Heading 2")
            .get_attribute("aria-checked")
            .as_deref(),
        Some("true")
    );

    // A pick carries the selection the view published (`PROSE-UI-281`).
    let block = ambre_prose::events::BlockId::new(
        ambre_core::PeerId::new(1),
        ambre_core::OpCounter::new(0),
    );
    let picked = PickedSelection {
        anchor: (block, ambre_core::index::Pos::new(0)),
        focus: (block, ambre_core::index::Pos::new(5)),
    };
    signals.selection.set(Some(picked));
    radio(&root, "Justified").click();
    assert_eq!(
        signals.text_write.get_untracked().and_then(|w| w.selection),
        Some(picked),
        "the write keeps the selection of its pick"
    );
    assert_eq!(
        signals.text_write.get_untracked().map(|w| w.value),
        Some(TextValue::Paragraph(ParagraphStyle {
            align: Some(Align::Justify),
            ..ParagraphStyle::default()
        }))
    );
    radio(&root, "Quote").click();
    assert_eq!(
        signals.text_write.get_untracked().map(|w| w.value),
        Some(TextValue::Kind(ParagraphKind::Quote))
    );
    radio(&root, "Numbered list").click();
    assert_eq!(
        signals.text_write.get_untracked().map(|w| w.value),
        Some(TextValue::Kind(ParagraphKind::List(ListStyle::Ordered(
            ambre_prose::message::OrderedStyle::Decimal
        ))))
    );
    let more: web_sys::HtmlElement = root
        .query_selector("[aria-label=\"Left indent: more\"]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    more.click();
    let left = match signals.text_write.get_untracked().map(|w| w.value) {
        Some(TextValue::Paragraph(style)) => style.indent_left,
        _ => None,
    }
    .expect("an indent is one paragraph write")
    .in_unit::<ambre_core::Mm>();
    assert!((left.get() - 1.0).abs() < 0.01, "one millimetre: {left:?}");
    // One step in hands the step, which the view sends over the selection
    // (`PROSE-UI-267`).
    let step = ambre_chrome::commands::Reach::Property(panel::INDENT_IN).marker();
    let button: web_sys::HtmlElement = root
        .query_selector(&format!("[data-reach=\"{step}\"]"))
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    button.click();
    assert_eq!(
        signals.text_write.get_untracked().map(|w| w.value),
        Some(TextValue::Indent(ambre_prose::message::IndentStep::In))
    );
    // The link opens the field of the bar; nothing is written yet.
    signals.text_write.set(None);
    let link = ambre_chrome::commands::Reach::Command(ambre_prose::commands::LINK).marker();
    let button: web_sys::HtmlElement = root
        .query_selector(&format!("[data-reach=\"{link}\"]"))
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    button.click();
    assert_eq!(
        signals.prompt.get_untracked(),
        Some(ambre_prose::view::toolbar::BarPrompt::Link)
    );
    assert_eq!(signals.text_write.get_untracked().map(|w| w.value), None);
    root.remove();
}

/// A mixed reading shows no value as chosen, and says mixed
/// (`PROSE-UI-258`).
#[wasm_bindgen_test]
async fn a_mixed_paragraph_reads_mixed_in_the_panel() {
    use ambre_chrome::commands::Reading;
    use ambre_prose::message::SelectionValues;
    let signals = ProseSignals::new();
    selection_with(
        signals,
        SelectionValues {
            align: Reading::Mixed,
            ..SelectionValues::default()
        },
    );
    let root = mount_prose_panel(signals);
    leptos::task::tick().await;
    for label in ["Left", "Centre", "Right", "Justified"] {
        assert_eq!(
            radio(&root, label).get_attribute("aria-checked").as_deref(),
            Some("false")
        );
    }
    assert!(root.query_selector(".ambre-kit-mixed").unwrap().is_some());
    root.remove();
}

/// With no text selected the panel shows the body font of the document
/// (`PROSE-UI-261`).
#[wasm_bindgen_test]
async fn with_no_selection_the_panel_shows_the_default_fonts() {
    use ambre_prose::view::panel;
    let signals = ProseSignals::new();
    signals.fonts.set(Some(ambre_prose::doc::DocFonts {
        body: ambre_core::FontFamily::from_label("Lora"),
    }));
    let root = mount_prose_panel(signals);
    leptos::task::tick().await;
    let markers = ambre_chrome::reach_markers(&root);
    let reach = ambre_chrome::commands::Reach::Property(panel::BODY_FONT);
    assert!(
        markers.iter().any(|m| reach.matches_marker(m)),
        "the body font shows"
    );
    let titles: Vec<String> = {
        let list = root.query_selector_all(".ambre-ctx-section-head").unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i))
            .filter_map(|n| n.text_content())
            .collect()
    };
    assert!(
        titles.iter().any(|t| t.contains("Default styles")),
        "{titles:?}"
    );
    assert_eq!(
        ambre_chrome::unreached_controls(&root),
        Vec::<String>::new()
    );
    root.remove();
}
