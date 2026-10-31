//! The Prose comment panel, mounted in headless Chrome
//! (`features/ambre-spec-prose-comments.md section 4`).
//!
//! The chrome is wasm-only Leptos, so these tests run under
//! `wasm-bindgen-test-runner` in headless Chrome (`just test-wasm`).
//! On the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::app::prose::{DocOffset, ProseError, ProseRefusal};
use ambre_bus::{AppEvent, UiEvent};
use ambre_bus::{Authored, Instant};
use ambre_core::id::{Comment, Document, Id, Reply, User};
use ambre_prose::chrome::{self, ProseCommentsPanel, ProseSignals};
use ambre_prose::events::{CommentReply, CommentThread, ProseEvent};
use ambre_prose::ui::ProseUiState;
use ambre_prose::view::annotate::{LostReply, PanelThread};
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

/// The page point at `(x, y)`, in CSS px.
fn css_point(x: f32, y: f32) -> ambre_core::Point<ambre_core::Px<ambre_core::Css>> {
    ambre_core::Point::new(
        ambre_core::Px::<ambre_core::Css>::new(x),
        ambre_core::Px::<ambre_core::Css>::new(y),
    )
}

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
        anchor_start: DocOffset::ZERO,
        anchor_end: DocOffset::new(3),
        orphaned: ambre_bus::app::prose::AnchorState::Anchored,
        resolved: ambre_core::ThreadState::Open,
        replies: replies
            .iter()
            .map(|text| CommentReply {
                reply_id: Id::<Reply>::mint(),
                author: ambre_core::DisplayName::new("Bob").ok(),
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
    let reply = ambre_bus::app::prose::ProseCommand::ReplyToComment {
        comment_id,
        text: String::new(),
        now: Instant::<Authored>::new(0),
    };
    chrome::notify_event(
        &toasts,
        &UiEvent::Viewport {
            document_id: Id::<Document>::mint(),
            event: AppEvent::Prose(ProseEvent::Error(ProseError::new(
                ambre_bus::app::prose::RefusedAction::Command(reply.name()),
                ProseRefusal::NoSuchComment(comment_id),
            ))),
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

/// The ruler element sits on the column the view gave it, moved onto the page
/// by the canvas origin. Each handle is centred on the position the state
/// gives it (`PROSE-UI-065`). The element adds no offset of its own, at any
/// zoom.
#[wasm_bindgen_test]
async fn the_ruler_element_sits_on_the_column_it_was_given() {
    use ambre_core::Pt;
    use ambre_core::{Css, Point, Px, View};
    use ambre_prose::view::ruler::{RulerHandle, RulerHandles, RulerState};
    let origin = Point::new(Px::<Css>::new(40.0), Px::<Css>::new(30.0));
    for zoom in [0.5_f32, 1.0, 2.0] {
        let signals = ProseSignals::new();
        let ruler = RulerState {
            visible: RulerHandles::Visible,
            page_left: Px::<View>::new(80.0),
            content_width: Px::<View>::new(451.0 * zoom),
            point_width: Px::<View>::new(4.0 / 3.0 * zoom),
            indent_first: Pt::new(18.0),
            indent_left: Pt::new(36.0),
            indent_right: Pt::new(12.0),
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
    let root = mount_in_container(move || view! { <chrome::ProseFormatBar signals=signals /> });
    leptos::task::tick().await;

    for (id, _) in ambre_prose::commands::FORMAT_BAR {
        let command = ambre_commands::find(ambre_prose::commands::COMMANDS, id)
            .expect("the control is described");
        let expected = (command.title)(&ambre_commands::Locale::En);
        let found = root
            .query_selector(&format!("[aria-label=\"{expected}\"]"))
            .unwrap();
        assert!(found.is_some(), "{id:?}: no control announces {expected:?}");
    }
    root.remove();
}

/// The labels of the menu rows the user cannot pick.
fn disabled_rows(root: &web_sys::Element) -> Vec<String> {
    let rows = root.query_selector_all("[aria-disabled=\"true\"]").unwrap();
    (0..rows.length())
        .filter_map(|i| rows.item(i))
        .map(|row| row.text_content().unwrap_or_default())
        .collect()
}

/// `UNDO-006`: the page menu's undo and redo rows follow the history state the
/// Logic thread reports. The menu counts nothing itself: an event that is not
/// a history state leaves the rows as they were.
#[wasm_bindgen_test]
async fn the_history_rows_follow_the_reported_state() {
    let signals = ProseSignals::new();
    let pos = RwSignal::new(Some(css_point(10.0, 10.0)));
    let entries = Signal::derive(move || chrome::context_entries(signals.history.get()));
    let root = mount_in_container(move || {
        view! { <ambre_chrome::ContextMenu pos=pos entries=entries on_command=Callback::new(|_| ()) /> }
    });
    let report = |event: ProseEvent| {
        signals.mirror_event(&UiEvent::Viewport {
            document_id: Id::<Document>::mint(),
            event: AppEvent::Prose(event),
        });
    };
    leptos::task::tick().await;
    assert!(
        disabled_rows(&root).iter().any(|row| row.contains("Undo"))
            && disabled_rows(&root).iter().any(|row| row.contains("Redo")),
        "a fresh page has no history: {:?}",
        disabled_rows(&root)
    );

    report(ProseEvent::UndoState {
        undo: ambre_core::Availability::Enabled,
        redo: ambre_core::Availability::Disabled,
    });
    leptos::task::tick().await;
    let rows = disabled_rows(&root);
    assert!(!rows.iter().any(|row| row.contains("Undo")), "{rows:?}");
    assert!(rows.iter().any(|row| row.contains("Redo")), "{rows:?}");

    report(ProseEvent::Synced {
        acked: ambre_bus::app::prose::TextAck::new(1),
    });
    leptos::task::tick().await;
    assert_eq!(disabled_rows(&root), rows);

    report(ProseEvent::UndoState {
        undo: ambre_core::Availability::Disabled,
        redo: ambre_core::Availability::Enabled,
    });
    leptos::task::tick().await;
    let rows = disabled_rows(&root);
    assert!(rows.iter().any(|row| row.contains("Undo")), "{rows:?}");
    assert!(!rows.iter().any(|row| row.contains("Redo")), "{rows:?}");
    root.remove();
}
