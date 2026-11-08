//! A Prose document, driven in the browser.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use crate::page::{Page, STEP, drag};
    use crate::relay::Relay;

    /// The WebDriver codes of the keys these tests press.
    const ENTER: &str = "\u{E007}";
    const BACKSPACE: &str = "\u{E003}";
    const SHIFT: &str = "\u{E008}";
    const CONTROL: &str = "\u{E009}";
    const HOME: &str = "\u{E011}";
    const END: &str = "\u{E010}";
    const LEFT: &str = "\u{E012}";
    const UP: &str = "\u{E013}";
    const DOWN: &str = "\u{E015}";

    /// Puts the caret in the document and types `text`, one key at a time.
    async fn write(page: &Page, text: &str) {
        click_page(page).await;
        type_text(page, text).await;
    }

    /// Clicks the page, which puts the caret in the text and the keyboard
    /// focus on the input pipe.
    async fn click_page(page: &Page) {
        let (ox, oy) = page.canvas_origin().await;
        let p = (ox + 300.0, oy + 200.0);
        page.browser.perform(json!([drag(p, p, 0)])).await;
    }

    /// Types `text` at the caret, one key at a time.
    async fn type_text(page: &Page, text: &str) {
        let keys: Vec<String> = text.chars().map(String::from).collect();
        press(page, &keys.iter().map(String::as_str).collect::<Vec<_>>()).await;
    }

    /// Presses and releases each of `keys` in turn: a character or a key code.
    async fn press(page: &Page, keys: &[&str]) {
        let actions: Vec<Value> = keys
            .iter()
            .flat_map(|k| {
                [
                    json!({ "type": "keyDown", "value": k }),
                    json!({ "type": "keyUp", "value": k }),
                ]
            })
            .collect();
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": actions }]))
            .await;
    }

    /// Presses `key` while `held` stays down, such as `Ctrl+Shift+Z`.
    async fn chord(page: &Page, held: &[&str], key: &str) {
        let down = held
            .iter()
            .map(|k| json!({ "type": "keyDown", "value": k }));
        let up = held
            .iter()
            .rev()
            .map(|k| json!({ "type": "keyUp", "value": k }));
        let actions: Vec<Value> = down
            .chain([
                json!({ "type": "keyDown", "value": key }),
                json!({ "type": "keyUp", "value": key }),
            ])
            .chain(up)
            .collect();
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": actions }]))
            .await;
    }

    /// Takes the document out as Markdown and answers its text. The file
    /// leaves the download directory, so a later export is read alone.
    async fn export_markdown(page: &Page) -> String {
        page.command("Export to Markdown (.md)").await;
        let (name, bytes) = page.download().await;
        let _ = std::fs::remove_file(page.downloads.join(&name));
        String::from_utf8(bytes).expect("Markdown is text")
    }

    /// Exports until the Markdown holds every one of `parts`, and answers it.
    /// A peer's change reaches the document a few frames after the count.
    async fn wait_markdown(page: &Page, parts: &[&str]) -> String {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let text = export_markdown(page).await;
            if parts.iter().all(|p| text.contains(p)) {
                return text;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the Markdown {text:?} lacks one of {parts:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// Waits until the status bar counts `figure`, such as "17 with spaces".
    /// The Logic thread sends the count of the document it holds, so the
    /// figure says that a typed or loaded text has reached it.
    async fn wait_counted(page: &Page, figure: &str) {
        let figure = Value::String(figure.to_owned());
        page.browser
            .wait_for(
                &format!("the count {figure}"),
                &format!(
                    "return [...document.querySelectorAll('.ambre-statusbar .stat')] \
                       .some(s => s.textContent === {figure});"
                ),
                STEP,
            )
            .await;
    }

    /// `PROSE-IO-056`: a Markdown export of a typed paragraph lands in the
    /// browser's download directory, holding that paragraph.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_markdown_export_reaches_the_download_directory() {
        let page = Page::new_document("prose").await;
        write(&page, "Hello from the belt").await;
        wait_counted(&page, "19 with spaces").await;
        page.command("Export to Markdown (.md)").await;
        let (name, bytes) = page.download().await;
        let text = String::from_utf8(bytes).expect("Markdown is text");
        assert!(name.ends_with(".md"), "{name}");
        assert!(text.contains("Hello from the belt"), "{text}");
    }

    /// `PROSE-IO-065`: a native Prose file saved in one browser opens from the
    /// home screen of another as a Prose document holding the same text.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_native_prose_file_opens_from_the_home_screen() {
        let author = Page::new_document("prose").await;
        write(&author, "Carried in a file").await;
        // The save must not outrun the keys it carries.
        wait_counted(&author, "17 with spaces").await;
        author
            .command("Save to a file (.ambre, not encrypted)")
            .await;
        let (name, file) = author.download().await;
        assert!(name.ends_with(".ambre"), "{name}");
        drop(author);

        let reader = Page::home(Arc::new(Relay::start().await)).await;
        reader
            .sign_in(&format!("belt-{}", uuid::Uuid::new_v4()))
            .await;
        reader.arm_picker().await;
        reader
            .browser
            .script(
                "document.querySelector('.ambre-open-file').click();",
                json!([]),
            )
            .await;
        reader.pick(&name, "application/octet-stream", &file).await;
        reader
            .browser
            .wait_for(
                "a Prose route",
                "return location.pathname.startsWith('/prose/');",
                STEP,
            )
            .await;
        reader.wait_canvas().await;
        // A laid-out canvas is not yet a loaded file: the export waits until
        // the opened paragraph is there.
        wait_counted(&reader, "17 with spaces").await;

        reader.command("Export to Markdown (.md)").await;
        let (_, bytes) = reader.download().await;
        let text = String::from_utf8(bytes).expect("Markdown is text");
        assert!(text.contains("Carried in a file"), "{text}");
    }

    /// Records, in `window.beltClip`, the token the next copy event writes,
    /// how each asynchronous clipboard write ends, and the HTML the next paste
    /// event carries. Each listener runs in the capture phase of the window,
    /// before the page's own, and changes nothing the browser does.
    const RECORD_CLIPBOARD: &str = "window.beltClip = { token: null, writes: [], html: null }; \
        addEventListener('copy', e => { \
          const data = e.clipboardData; const set = data.setData.bind(data); \
          data.setData = (kind, value) => { \
            if (kind === 'application/x-ambre-token') beltClip.token = value; \
            return set(kind, value); }; }, true); \
        const write = navigator.clipboard.write.bind(navigator.clipboard); \
        navigator.clipboard.write = items => { \
          const record = { ended: null }; beltClip.writes.push(record); \
          const written = write(items); \
          written.then(() => record.ended = 'landed', e => record.ended = e.name); \
          return written; }; \
        addEventListener('paste', e => { \
          beltClip.html = e.clipboardData.getData('text/html'); }, true);";

    /// `UI-THREAD-057`: Firefox refuses the `web ` types, so a Prose copy
    /// carries its ownership token in its HTML alone. Firefox hands that HTML
    /// back to the next paste, and the token the copy event wrote reads back
    /// from it.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "firefox"),
        ignore = "Firefox alone refuses the `web ` types; run just test-browser-firefox"
    )]
    async fn the_token_reads_back_from_the_html_firefox_hands_back() {
        let page = Page::new_document("prose").await;
        write(&page, "Carried in the HTML").await;
        wait_counted(&page, "19 with spaces").await;
        page.browser.script(RECORD_CLIPBOARD, json!([])).await;

        chord(&page, &[CONTROL], "a").await;
        chord(&page, &[CONTROL], "c").await;
        // Firefox 153 refuses the first write, which carries the `web ` types,
        // and the second write lands.
        page.browser
            .wait_for(
                "a landed asynchronous clipboard write",
                "return beltClip.writes.some(w => w.ended === 'landed');",
                STEP,
            )
            .await;
        let token = page
            .browser
            .script("return beltClip.token;", json!([]))
            .await;
        let token = token.as_str().expect("the copy event wrote a token");

        chord(&page, &[CONTROL], "v").await;
        let html = page
            .browser
            .wait_for("the pasted HTML", "return beltClip.html;", STEP)
            .await;
        let html = html.as_str().unwrap_or_default();
        assert!(
            html.contains(&format!("<!--ambre-token:{token}-->")),
            "no token {token} in {html}"
        );
    }

    /// `PROSE-UI-183`: `Enter` splits the paragraph at the caret, and the keys
    /// typed right after it land in the new paragraph. `PROSE-037`: `Backspace`
    /// at the start of that paragraph joins it back onto the first.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn enter_splits_a_paragraph_and_backspace_joins_it_back() {
        let page = Page::new_document("prose").await;
        write(&page, "Helloworld").await;
        wait_counted(&page, "1 paragraph").await;
        wait_counted(&page, "10 characters").await;

        press(&page, &[LEFT, LEFT, LEFT, LEFT, LEFT, ENTER, "N", "e", "w"]).await;
        wait_counted(&page, "2 paragraphs").await;
        wait_counted(&page, "13 characters").await;
        let text = export_markdown(&page).await;
        assert_eq!(text.trim(), "Hello\n\nNewworld", "{text}");

        // The export took the focus: a click gives it back, and the keys then
        // carry the caret to the start of the second paragraph.
        click_page(&page).await;
        chord(&page, &[CONTROL], END).await;
        press(&page, &[HOME, BACKSPACE]).await;
        wait_counted(&page, "1 paragraph").await;
        let text = export_markdown(&page).await;
        assert_eq!(text.trim(), "HelloNewworld", "{text}");
    }

    /// `UNDO-010`, `UNDO-011`: `Ctrl+Z` takes the last key typed back,
    /// `Ctrl+Shift+Z` and `Ctrl+Y` both bring it again. Each step is read off
    /// the count the Logic thread reports, so each one reached the document.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_history_chords_undo_and_redo_in_the_browser() {
        let page = Page::new_document("prose").await;
        write(&page, "Hello").await;
        wait_counted(&page, "5 with spaces").await;
        // A key typed once the run is confirmed is a step of its own.
        type_text(&page, "s").await;
        wait_counted(&page, "6 with spaces").await;

        chord(&page, &[CONTROL], "z").await;
        wait_counted(&page, "5 with spaces").await;
        chord(&page, &[CONTROL, SHIFT], "z").await;
        wait_counted(&page, "6 with spaces").await;
        chord(&page, &[CONTROL], "z").await;
        wait_counted(&page, "5 with spaces").await;
        chord(&page, &[CONTROL], "y").await;
        wait_counted(&page, "6 with spaces").await;
        let text = export_markdown(&page).await;
        assert_eq!(text.trim(), "Hellos", "{text}");
    }

    /// The `aria-pressed` value of the format bar's control named `label`, or
    /// `None` while the bar is not on the page.
    async fn control_pressed(page: &Page, label: &str) -> Option<String> {
        page.browser
            .script(
                "const c = document.querySelector( \
                   `.ambre-format-bar [aria-label=\"${arguments[0]}\"]`); \
                 return c ? c.getAttribute('aria-pressed') : null;",
                json!([label]),
            )
            .await
            .as_str()
            .map(str::to_owned)
    }

    /// Waits until the format bar's control `label` reads `pressed`.
    async fn wait_pressed(page: &Page, label: &str, pressed: &str) {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let seen = control_pressed(page, label).await;
            if seen.as_deref() == Some(pressed) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the {label} control reads {seen:?}, not {pressed:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// `PROSE-UI-056`: `Ctrl+B` over a selected word bolds it, and the bar's
    /// Bold control then reads pressed. A press on that control takes the
    /// same mark off: the chord and the control reach one command. Each
    /// state is read off the exported file too.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_chord_and_its_control_toggle_one_mark() {
        let page = Page::new_document("prose").await;
        write(&page, "Make this bold").await;
        wait_counted(&page, "14 with spaces").await;
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": SHIFT },
                { "type": "keyDown", "value": LEFT }, { "type": "keyUp", "value": LEFT },
                { "type": "keyDown", "value": LEFT }, { "type": "keyUp", "value": LEFT },
                { "type": "keyDown", "value": LEFT }, { "type": "keyUp", "value": LEFT },
                { "type": "keyDown", "value": LEFT }, { "type": "keyUp", "value": LEFT },
                { "type": "keyUp", "value": SHIFT },
            ]}]))
            .await;
        wait_pressed(&page, "Bold", "false").await;

        chord(&page, &[CONTROL], "b").await;
        wait_pressed(&page, "Bold", "true").await;
        let text = export_markdown(&page).await;
        assert_eq!(text.trim(), "Make this **bold**", "{text}");

        page.browser
            .script(
                "document.querySelector('.ambre-format-bar [aria-label=\"Bold\"]').click();",
                json!([]),
            )
            .await;
        wait_pressed(&page, "Bold", "false").await;
        let text = export_markdown(&page).await;
        assert_eq!(text.trim(), "Make this bold", "{text}");
    }

    /// `CHROME-023`: the arrow keys walk the caret from paragraph to
    /// paragraph, and the live region names each one the caret enters, by its
    /// text. `Ctrl+Home` and a left arrow then place the keys typed after
    /// them, read off the exported file.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_keyboard_walks_the_caret_through_the_text() {
        let page = Page::new_document("prose").await;
        write(&page, "First").await;
        press(&page, &[ENTER]).await;
        type_text(&page, "Second").await;
        wait_counted(&page, "2 paragraphs").await;
        wait_counted(&page, "11 characters").await;

        press(&page, &[UP]).await;
        page.wait_announced("Paragraph, First").await;
        press(&page, &[DOWN]).await;
        page.wait_announced("Paragraph, Second").await;
        chord(&page, &[CONTROL], HOME).await;
        page.wait_announced("Paragraph, First").await;
        type_text(&page, "1").await;
        press(&page, &[END, LEFT]).await;
        type_text(&page, "2").await;
        wait_counted(&page, "13 characters").await;
        let text = export_markdown(&page).await;
        assert_eq!(text.trim(), "1Firs2t\n\nSecond", "{text}");
    }

    /// Signs a second user in, in a second browser, and shares the document of
    /// `first` with them. That browser has a profile of its own, so it holds
    /// nothing of the document.
    async fn invite_second(first: &Page) -> Page {
        let name = format!("belt-{}", uuid::Uuid::new_v4());
        let second = Page::home(first.relay()).await;
        second.sign_in(&name).await;
        first.share_with(&name).await;
        second
    }

    /// Opens the document of `first` on `second`.
    async fn open_document_of(second: &Page, first: &Page) {
        let path = first
            .browser
            .script("return location.pathname;", json!([]))
            .await;
        let path = path.as_str().expect("a document route");
        second
            .browser
            .goto(&format!("{}{path}", second.origin))
            .await;
        second.wait_canvas().await;
    }

    /// `PROSE-131`: a paragraph one user types reaches a second user who holds
    /// the document open, in a second browser.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_paragraph_one_peer_types_reaches_the_other() {
        let alice = Page::new_document("prose").await;
        write(&alice, "Hello").await;
        wait_counted(&alice, "5 characters").await;
        let bob = invite_second(&alice).await;
        open_document_of(&bob, &alice).await;
        wait_counted(&bob, "5 characters").await;

        // Bob holds the document open while Alice types on.
        type_text(&alice, " Bob").await;
        wait_counted(&alice, "8 characters").await;
        wait_counted(&bob, "8 characters").await;
        let text = wait_markdown(&bob, &["Hello Bob"]).await;
        assert_eq!(text.trim(), "Hello Bob", "{text}");
    }

    /// `PROSE-132`: two users type at once, each in a paragraph of their own.
    /// The second one's Sync worker is paused while both type, so the two
    /// edits cross. Both documents end on one text that keeps both edits.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "pauses a worker through the DevTools Protocol, which Firefox does not speak"
    )]
    async fn two_peers_typing_in_two_paragraphs_converge() {
        let alice = Page::new_document("prose").await;
        write(&alice, "Alpha").await;
        press(&alice, &[ENTER]).await;
        type_text(&alice, "Beta").await;
        wait_counted(&alice, "9 characters").await;
        let bob = invite_second(&alice).await;
        open_document_of(&bob, &alice).await;
        wait_counted(&bob, "9 characters").await;
        let sync = bob.worker("ambre-sync").await;
        let paused = bob.pause_worker(&sync).await;

        click_page(&bob).await;
        chord(&bob, &[CONTROL], END).await;
        type_text(&bob, " B2").await;
        chord(&alice, &[CONTROL], HOME).await;
        type_text(&alice, "A1 ").await;
        wait_counted(&alice, "11 characters").await;
        wait_counted(&bob, "11 characters").await;

        bob.resume_worker(paused).await;
        wait_counted(&alice, "13 characters").await;
        wait_counted(&bob, "13 characters").await;
        let theirs = wait_markdown(&bob, &["A1 Alpha", "Beta B2"]).await;
        let ours = wait_markdown(&alice, &["A1 Alpha", "Beta B2"]).await;
        assert_eq!(ours.trim(), "A1 Alpha\n\nBeta B2", "{ours}");
        assert_eq!(theirs, ours);
    }

    /// The thread texts the comment panel shows, in page order.
    async fn panel_replies(page: &Page) -> Vec<String> {
        page.browser
            .script(
                "return [...document.querySelectorAll('.ambre-prose-reply-text')] \
                   .map(e => e.textContent);",
                json!([]),
            )
            .await
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    }

    /// Waits until the comment panel shows `reply` among its replies.
    async fn wait_reply(page: &Page, reply: &str) {
        let reply = Value::String(reply.to_owned());
        page.browser
            .wait_for(
                &format!("the reply {reply} in the panel"),
                &format!(
                    "return [...document.querySelectorAll('.ambre-prose-reply-text')] \
                       .some(e => e.textContent === {reply});"
                ),
                STEP,
            )
            .await;
    }

    /// Opens, from the home screen of `page`, the Prose fixture whose one
    /// thread asks "Is this net or gross?" and holds one answer.
    async fn open_fixture_with_a_thread(page: &Page) {
        let file = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/current/prose/styled_document.ambre"),
        )
        .expect("the Prose fixture");
        page.arm_picker().await;
        page.browser
            .script(
                "document.querySelector('.ambre-open-file').click();",
                json!([]),
            )
            .await;
        page.pick("styled_document.ambre", "application/octet-stream", &file)
            .await;
        page.browser
            .wait_for(
                "a Prose route",
                "return location.pathname.startsWith('/prose/');",
                STEP,
            )
            .await;
        page.wait_canvas().await;
    }

    /// `PROSE-FEAT-190`: a reply posted from the comment panel lands last in
    /// its thread, as the panel then shows it. No gesture opens a thread yet
    /// (`ambre-spec-prose-comments.md` section 6), so the thread comes from a
    /// file. A file opens as a document of its own, never shared
    /// (`ambre-spec-docfile.md`), so no second browser reads the reply.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_reply_posted_from_the_panel_lands_last_in_its_thread() {
        let page = Page::home(Arc::new(Relay::start().await)).await;
        page.sign_in(&format!("belt-{}", uuid::Uuid::new_v4()))
            .await;
        open_fixture_with_a_thread(&page).await;
        wait_reply(&page, "Net.").await;

        page.browser
            .script(
                "const i = document.querySelector('.ambre-prose-thread-input'); \
                 i.focus(); i.value = 'Gross, in fact.'; \
                 i.dispatchEvent(new Event('input', { bubbles: true }));",
                json!([]),
            )
            .await;
        page.browser
            .script(
                "document.querySelector('.ambre-prose-thread-send').click();",
                json!([]),
            )
            .await;
        wait_reply(&page, "Gross, in fact.").await;
        assert_eq!(
            panel_replies(&page).await,
            ["Is this net or gross?", "Net.", "Gross, in fact."]
        );
    }
}
