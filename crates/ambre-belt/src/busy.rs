//! A document whose Logic thread takes nothing, driven in the browser.
//!
//! The DevTools Protocol pauses the Logic worker in its debugger, so the
//! command queue between the views and that thread fills and stays full.

use serde_json::{Value, json};

use crate::page::{Page, Paused};

/// More presses than the queue holds: 1024 slots, then the outbox's 64.
pub const FLOOD: usize = 1200;

/// Pauses the page's Logic worker (`Page::pause_worker`).
///
/// A worker idle since its boot does not always stop: most runs drained a
/// flood sent after the pause. One command first, so the worker is awake
/// when the pause reaches it. With it, every run stopped.
pub async fn pause_logic(page: &Page) -> Paused {
    clear_cell(page, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let workers = page.workers().await;
    let (logic, _) = workers
        .iter()
        .find(|(_, title)| title == "logic")
        .unwrap_or_else(|| panic!("no Logic worker among {workers:?}"));
    page.pause_worker(logic).await
}

/// Presses `Delete` on the active cell `n` times. Each press clears the
/// cell through one lossless command, whatever the cell holds.
pub async fn clear_cell(page: &Page, n: usize) {
    page.focus_canvas().await;
    let mut keys = Vec::new();
    for _ in 0..n {
        keys.push(json!({ "type": "keyDown", "value": "\u{E017}" }));
        keys.push(json!({ "type": "keyUp", "value": "\u{E017}" }));
    }
    page.browser
        .perform(json!([{ "type": "key", "id": "keyboard", "actions": keys }]))
        .await;
}

/// Whether a toast on the page reads `text`.
async fn toasted(page: &Page, text: &str) -> bool {
    page.browser
        .script(
            "const [text] = arguments; \
             return [...document.querySelectorAll('[role=status]')] \
               .some(e => e.textContent.includes(text));",
            json!([text]),
        )
        .await
        == Value::Bool(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BUS-016`: a full command queue reaches the user as a busy notice, and
    /// the notice clears once the queue drains.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_full_queue_reaches_the_user_as_a_busy_notice() {
        let page = Page::new_document("trame").await;
        // A running Logic thread drains as fast as the keys come.
        clear_cell(&page, FLOOD).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            !toasted(&page, "catching up").await,
            "busy with Logic running"
        );

        let paused = pause_logic(&page).await;
        clear_cell(&page, FLOOD).await;
        page.wait_toast("The document is catching up.").await;
        page.resume_worker(paused).await;
        page.wait_toast("The document is up to date.").await;
    }
}
