//! A Trame document driven in the browser.

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::busy::{FLOOD, clear_cell, pause_logic};
    use crate::driver::Engine;
    use crate::page::{Page, STEP};

    /// `TRAME-IO-040`: a file the full queue cannot carry tells the user that
    /// it did not import.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "pauses a worker through the DevTools Protocol, which Firefox does not speak"
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
    /// clipboard holds what only that write publishes. On Chrome that is the
    /// private bytes and the token. Firefox refuses the `web ` types. So
    /// for a Trame copy, the write publishes the mirror alone
    /// (`UI-THREAD-055`).
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
        let published: &[&str] = match page.browser.engine() {
            Engine::Chrome => &["web application/x-ambre", "web application/x-ambre-token"],
            Engine::Firefox => &["text/plain"],
        };
        for kind in published {
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
    #[cfg_attr(
        feature = "firefox",
        ignore = "pauses a worker through the DevTools Protocol, which Firefox does not speak"
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

    /// The canvas as the screen shows it, cut out of a viewport screenshot.
    async fn canvas_shot(page: &Page) -> image::RgbImage {
        let rect = page
            .browser
            .script(
                "const r = document.querySelector('canvas').getBoundingClientRect(); \
                 return [r.left, r.top, r.width, r.height].map(Math.floor);",
                json!([]),
            )
            .await;
        let at = |i: usize| rect[i].as_u64().unwrap_or(0) as u32;
        let shot = image::load_from_memory(&page.browser.screenshot().await)
            .expect("a PNG screenshot")
            .to_rgb8();
        image::imageops::crop_imm(&shot, at(0), at(1), at(2), at(3)).to_image()
    }

    /// Scrolls the wheel over the grid by `steps` pixel deltas of `dy`, one
    /// every 16 ms, then lets go: no event follows the last one.
    async fn wheel(page: &Page, steps: u32, dy: i64) {
        let (ox, oy) = page.canvas_origin().await;
        let scrolls: Vec<Value> = (0..steps)
            .map(|_| {
                json!({ "type": "scroll", "x": (ox + 300.0) as i64, "y": (oy + 300.0) as i64,
                        "deltaX": 0, "deltaY": dy, "duration": 16 })
            })
            .collect();
        page.browser
            .perform(json!([{ "type": "wheel", "id": "wheel", "actions": scrolls }]))
            .await;
    }

    /// Whether the grid still moves once the wheel has stopped: two shots of
    /// the canvas, 400 ms apart. The first waits 250 ms after the last wheel
    /// event, so no frame of the wheel's own steps is still to paint. A glide
    /// of 0.75 px per ms, decaying over 325 ms (`DECAY`), still travels about
    /// 80 px between the two shots.
    async fn moves_after_the_wheel(page: &Page) -> bool {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let first = canvas_shot(page).await;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        canvas_shot(page).await != first
    }

    /// `KINETIC-002`, `KINETIC-009`: Firefox on Linux sends no momentum after a
    /// flick, so a pixel glide that stops keeps the grid moving on its own. A
    /// notch never starts that motion. The driver's wheel sends no momentum on
    /// either engine; the test runs where that absence is the platform's.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "firefox"),
        ignore = "the glide stands in for the momentum Firefox on Linux never sends; run just test-browser-firefox"
    )]
    async fn a_paused_glide_keeps_the_grid_moving_and_a_notch_never_does() {
        let page = Page::new_document("trame").await;
        // A notch first: the grid moves once, and is still after it.
        wheel(&page, 1, 120).await;
        assert!(
            !moves_after_the_wheel(&page).await,
            "a notch started a motion"
        );
        // A glide well under the notch size, then nothing.
        wheel(&page, 12, 12).await;
        assert!(moves_after_the_wheel(&page).await, "the glide stopped dead");
    }
}
