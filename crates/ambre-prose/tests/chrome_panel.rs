//! The Prose comment panel, mounted in headless Chrome
//! (`features/ambre-spec-prose-comments.md section 4`).
//!
//! The chrome is wasm-only Leptos, so these tests run under
//! `wasm-pack test --headless --chrome crates/ambre-prose` (`just test-wasm`).
//! On the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_bus::app::prose::{ProseError, ProseRefusal};
use ambre_bus::{AppEvent, DocumentId, UiEvent};
use ambre_core::{UserId, Uuid};
use ambre_prose::chrome::{self, ProseCommentsPanel, ProseSignals};
use ambre_prose::events::{CommentReply, CommentThread, ProseEvent};
use ambre_prose::ui::ProseUiState;
use ambre_prose::view::annotate::PanelThread;
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

fn thread(id: Uuid, replies: &[&str]) -> CommentThread {
    CommentThread {
        comment_id: id,
        anchor_block: None,
        anchor_start: 0,
        anchor_end: 3,
        orphaned: false,
        resolved: false,
        replies: replies
            .iter()
            .map(|text| CommentReply {
                reply_id: Uuid::new(),
                author: "Bob".to_owned(),
                user_id: UserId::from_bytes([2; 16]),
                created_at: 0,
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
    let (first, second) = (Uuid::new(), Uuid::new());
    signals.threads.set(vec![
        PanelThread {
            thread: thread(first, &["Is this right?"]),
            y_px: 100.0,
        },
        PanelThread {
            thread: thread(second, &["Typo"]),
            y_px: 300.0,
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
            y_px: 40.0,
        },
        PanelThread {
            thread: thread(second, &["Typo"]),
            y_px: 240.0,
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
    let (kept, gone) = (Uuid::new(), Uuid::new());
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
    let notices = signals.drop_lost_drafts(&view);
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("not posted"), "{}", notices[0]);
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
    chrome::notify_event(
        &toasts,
        &UiEvent::Viewport {
            document_id: DocumentId::new(),
            event: AppEvent::Prose(ProseEvent::Error(ProseError::new(
                "replying to the comment",
                ProseRefusal::NoSuchComment(Uuid::new()),
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
