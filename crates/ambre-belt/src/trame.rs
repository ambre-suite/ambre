//! A Trame document driven in the browser.

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::busy::{FLOOD, clear_cell, pause_logic};
    use crate::page::{Page, STEP};

    /// `TRAME-IO-040`: a file the full queue cannot carry tells the user that
    /// it did not import.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn an_import_the_full_queue_refuses_says_so() {
        let page = Page::new_document("trame").await;
        let paused = pause_logic(&page).await;
        clear_cell(&page, FLOOD).await;
        page.wait_toast("The document is catching up.").await;

        page.arm_picker().await;
        page.command("Import CSV").await;
        page.pick("one.csv", "text/csv", b"a,b\n1,2\n").await;
        page.wait_toast("The file did not import").await;
        // The toast is the whole answer: let the worker go before the page closes.
        page.resume_worker(paused).await;
    }

    /// Selects `A1:B1`, and records each asynchronous clipboard write the page
    /// makes in `window.beltWrites`. A record holds whether the activation was
    /// still live when the page called the write, and how the write ended. The
    /// write itself is the browser's, unchanged.
    ///
    /// The clipboard read is granted, so a write resolves and reads back. A
    /// headless Chromium 141 holds a write of the `web ` types pending without
    /// it, inside the window or out. With it, a write lands whatever the
    /// activation. So the window is read where the write is called, not
    /// inferred from the outcome.
    async fn select_and_record(page: &Page) {
        page.browser.permit("clipboard-read").await;
        // A click in the grid focuses the pipe, where the copy event is raised.
        page.browser
            .script(
                "document.querySelector('.ambre-input').focus(); \
                 window.beltWrites = []; \
                 const write = navigator.clipboard.write.bind(navigator.clipboard); \
                 navigator.clipboard.write = items => { \
                   const record = { active: navigator.userActivation.isActive, ended: null }; \
                   beltWrites.push(record); \
                   const written = write(items); \
                   written.then(() => record.ended = 'landed', e => record.ended = e.name); \
                   return written; \
                 };",
                json!([]),
            )
            .await;
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": "\u{E008}" },
                { "type": "keyDown", "value": "\u{E014}" },
                { "type": "keyUp", "value": "\u{E014}" },
                { "type": "keyUp", "value": "\u{E008}" },
            ]}]))
            .await;
    }

    /// Copies the selection with `Ctrl+C`. The driver's key press is trusted,
    /// so it opens a user activation window.
    async fn copy(page: &Page) {
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": "\u{E009}" },
                { "type": "keyDown", "value": "c" },
                { "type": "keyUp", "value": "c" },
                { "type": "keyUp", "value": "\u{E009}" },
            ]}]))
            .await;
    }

    /// The first asynchronous write since `select_and_record`, once it ended:
    /// whether the activation was live at its call, and how it ended.
    async fn first_write(page: &Page) -> (bool, String) {
        let record = page
            .browser
            .wait_for(
                "the asynchronous clipboard write",
                "const w = window.beltWrites[0]; return w && w.ended && w;",
                STEP,
            )
            .await;
        (
            record["active"] == Value::Bool(true),
            record["ended"].as_str().unwrap_or_default().to_owned(),
        )
    }

    /// The types the clipboard holds, read back through the asynchronous API.
    async fn clipboard_types(page: &Page) -> Value {
        page.browser
            .script_async(
                "const done = arguments[0]; \
                 navigator.clipboard.read() \
                   .then(items => done(items.flatMap(i => i.types))) \
                   .catch(e => done(e.name));",
                json!([]),
            )
            .await
    }

    /// Whether the clipboard holds the type `kind`.
    fn holds(types: &Value, kind: &str) -> bool {
        types.as_array().is_some_and(|t| t.contains(&json!(kind)))
    }

    /// `UI-THREAD-009`: the write a copy defers to the next frame is called
    /// while the activation the key press opened is live. It lands, and the
    /// clipboard holds the private bytes and the token only that write
    /// publishes.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_deferred_copy_writes_inside_the_activation_window() {
        let page = Page::new_document("trame").await;
        select_and_record(&page).await;
        copy(&page).await;
        assert_eq!(first_write(&page).await, (true, "landed".to_owned()));
        let types = clipboard_types(&page).await;
        for kind in ["web application/x-ambre", "web application/x-ambre-token"] {
            assert!(holds(&types, kind), "{kind} in {types}");
        }
    }

    /// `UI-THREAD-009`, the control: the same copy, with the Logic thread held
    /// past the activation window, makes its write once the window has lapsed.
    /// So the reading above tells a write inside the window from one outside.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_copy_held_past_the_activation_window_writes_outside_it() {
        let page = Page::new_document("trame").await;
        select_and_record(&page).await;
        let paused = pause_logic(&page).await;
        copy(&page).await;
        // A transient activation lasts five seconds in Chromium.
        tokio::time::sleep(std::time::Duration::from_secs(7)).await;
        page.resume_worker(paused).await;
        let (active, _) = first_write(&page).await;
        assert!(!active, "the write was called inside the window");
    }
}
