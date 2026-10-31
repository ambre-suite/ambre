//! The system notifications of an open tab (the Murmure notifications spec).
//!
//! The Murmure view decides which message notifies, and this module raises
//! it. A permission the browser refuses raises nothing, and the badges of
//! the sidebar stay as they are.

use std::cell::Cell;
use std::rc::Rc;

use ambre_core::id::{Channel, Id};
use ambre_murmure::MessageRef;
use ambre_murmure::chrome::Notice;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Notification, NotificationOptions, NotificationPermission};

/// What a click on a notice runs: open its channel at its message.
pub(crate) type Opener = Rc<dyn Fn(Id<Channel>, Option<MessageRef>)>;

/// What the page does with a notice, given the browser's permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    /// Raise it.
    Raise,
    /// Ask for the permission, then raise it if granted.
    Ask,
    /// Raise nothing.
    Nothing,
}

/// Whether this page has already asked for the notification permission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Question {
    #[default]
    Unasked,
    Asked,
}

/// The act a permission calls for. The page asks once: a question left
/// unanswered is not asked again.
pub(crate) fn act(permission: NotificationPermission, asked: Question) -> Act {
    match permission {
        NotificationPermission::Granted => Act::Raise,
        NotificationPermission::Default if asked == Question::Unasked => Act::Ask,
        _ => Act::Nothing,
    }
}

/// Raise `notice`, asking for the permission first when none was given.
/// A click on it calls `open` with its channel and its message. A browser
/// with no notifications raises nothing.
pub(crate) fn deliver(notice: Notice, asked: &Rc<Cell<Question>>, open: Opener) {
    if !has_notifications() {
        return;
    }
    match act(Notification::permission(), asked.get()) {
        Act::Raise => raise(&notice, open),
        Act::Nothing => {}
        Act::Ask => {
            asked.set(Question::Asked);
            let Ok(question) = Notification::request_permission() else {
                return;
            };
            wasm_bindgen_futures::spawn_local(async move {
                let _ = wasm_bindgen_futures::JsFuture::from(question).await;
                if Notification::permission() == NotificationPermission::Granted {
                    raise(&notice, open);
                }
            });
        }
    }
}

/// Whether the page is on screen. A hidden page shows no channel to read.
pub(crate) fn page_visible() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .is_some_and(|document| !document.hidden())
}

/// Whether the page offers the notification API at all.
fn has_notifications() -> bool {
    web_sys::window().is_some_and(|window| {
        js_sys::Reflect::has(&window, &JsValue::from_str("Notification")).unwrap_or(false)
    })
}

/// Show one notification. Its tag is the channel, so a second message of
/// the same channel replaces the first rather than stacking.
fn raise(notice: &Notice, open: Opener) {
    let options = NotificationOptions::new();
    options.set_tag(&notice.channel_id.to_string());
    let Ok(shown) = Notification::new_with_options(&notice.title, &options) else {
        tracing::warn!("Notification refused by the browser");
        return;
    };
    let (channel_id, at) = (notice.channel_id, notice.at);
    let target = shown.clone();
    let click = Closure::once_into_js(move || {
        open(channel_id, at);
        if let Some(window) = web_sys::window() {
            let _ = window.focus();
        }
        target.close();
    });
    shown.set_onclick(Some(click.unchecked_ref()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// A granted permission raises, and a refused one raises nothing (the
    /// Murmure notifications spec). An unanswered one is asked once.
    #[wasm_bindgen_test]
    fn a_refused_permission_raises_nothing_and_an_open_one_is_asked_once() {
        assert_eq!(
            act(NotificationPermission::Granted, Question::Asked),
            Act::Raise
        );
        assert_eq!(
            act(NotificationPermission::Denied, Question::Unasked),
            Act::Nothing
        );
        assert_eq!(
            act(NotificationPermission::Default, Question::Unasked),
            Act::Ask
        );
        assert_eq!(
            act(NotificationPermission::Default, Question::Asked),
            Act::Nothing
        );
    }
}
