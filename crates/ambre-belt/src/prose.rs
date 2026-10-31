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
}
