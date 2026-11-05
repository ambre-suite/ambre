//! A Prose document, driven in the browser.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use crate::page::{Page, STEP, drag};
    use crate::relay::Relay;

    /// Puts the caret in the document and types `text`, one key at a time.
    async fn write(page: &Page, text: &str) {
        let (ox, oy) = page.canvas_origin().await;
        let p = (ox + 300.0, oy + 200.0);
        page.browser.perform(json!([drag(p, p, 0)])).await;
        let mut keys = Vec::new();
        for c in text.chars() {
            keys.push(json!({ "type": "keyDown", "value": c.to_string() }));
            keys.push(json!({ "type": "keyUp", "value": c.to_string() }));
        }
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": keys }]))
            .await;
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

    /// Presses `Ctrl` with `key`, as a trusted chord the driver sends.
    async fn chord(page: &Page, key: &str) {
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": "\u{E009}" },
                { "type": "keyDown", "value": key },
                { "type": "keyUp", "value": key },
                { "type": "keyUp", "value": "\u{E009}" },
            ]}]))
            .await;
    }

    /// Records, in `window.beltClip`, the token the next copy event writes,
    /// how the asynchronous clipboard write ends, and the HTML the next paste
    /// event carries. Each listener runs in the capture phase of the window,
    /// before the page's own, and changes nothing the browser does.
    const RECORD_CLIPBOARD: &str = "window.beltClip = { token: null, ended: null, html: null }; \
        addEventListener('copy', e => { \
          const data = e.clipboardData; const set = data.setData.bind(data); \
          data.setData = (kind, value) => { \
            if (kind === 'application/x-ambre-token') beltClip.token = value; \
            return set(kind, value); }; }, true); \
        const write = navigator.clipboard.write.bind(navigator.clipboard); \
        navigator.clipboard.write = items => { \
          const written = write(items); \
          written.then(() => beltClip.ended = 'landed', e => beltClip.ended = e.name); \
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

        chord(&page, "a").await;
        chord(&page, "c").await;
        let ended = page
            .browser
            .wait_for(
                "the asynchronous clipboard write",
                "return beltClip.ended;",
                STEP,
            )
            .await;
        assert_eq!(ended, json!("landed"));
        let token = page
            .browser
            .script("return beltClip.token;", json!([]))
            .await;
        let token = token.as_str().expect("the copy event wrote a token");

        chord(&page, "v").await;
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
}
