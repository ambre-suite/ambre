//! One IME composition cycle in a Prose page and in a Trame cell, driven
//! through the browser's own input pipeline.
//!
//! The DevTools Protocol composes as an input method does:
//! `Input.imeSetComposition` sets the pre-edit, and `Input.insertText`
//! commits the final text. Chrome raises the composition events on the
//! focused pipe, as it does for a real input method. No event is raised by a
//! page script, so the pipe meets the event sequence a user's IME produces.
//!
//! A Toile text element is left out. An `Escape` sent just after a commit
//! closes its editor without the committed text, and plain typing keeps it.
//! That defect is reported, and a test of it would fail one run in a few.

use serde_json::{Value, json};

use crate::page::{Page, STEP};

/// The pre-edits an input method shows while a user types "nihongo" in kana
/// and converts it, before the commit.
const PREEDITS: [&str; 5] = [
    "\u{306b}",
    "\u{306b}\u{307b}",
    "\u{306b}\u{307b}\u{3093}",
    "\u{306b}\u{307b}\u{3093}\u{3054}",
    "\u{65e5}\u{672c}\u{8a9e}",
];

/// The committed text: "nihongo" in kanji.
const COMMITTED: &str = "\u{65e5}\u{672c}\u{8a9e}";

/// Records each composition event the page raises, with its data, in
/// `window.beltComposition`. A capture listener on the document sees the
/// events before the pipe does, and changes nothing.
///
/// Also records each uncaught error in `window.beltErrors`, with its stack. A
/// trap of the wasm module in a listener is one, and it stops the listener
/// half way. A contended lock on the main thread ends in such a trap: it calls
/// `Atomics.wait`, which the browser refuses (`ARCH-120`).
async fn record_composition(page: &Page) {
    page.browser
        .script(
            "window.beltComposition = []; window.beltErrors = []; \
             for (const t of ['compositionstart', 'compositionupdate', 'compositionend']) \
               document.addEventListener(t, e => beltComposition.push([t, e.data]), true); \
             window.addEventListener('error', \
               e => beltErrors.push(String((e.error && e.error.stack) || e.message)));",
            json!([]),
        )
        .await;
}

/// Asserts that no error escaped to the page since [`record_composition`].
async fn assert_no_page_error(page: &Page) {
    let errors = page
        .browser
        .script("return window.beltErrors || [];", json!([]))
        .await;
    assert_eq!(errors, json!([]), "the page raised an uncaught error");
}

/// Composes [`COMMITTED`] through every pre-edit of [`PREEDITS`] into the
/// focused element, then commits it.
async fn compose(page: &Page) {
    for preedit in PREEDITS {
        let end = preedit.encode_utf16().count();
        page.browser
            .cdp(
                "Input.imeSetComposition",
                json!({ "text": preedit, "selectionStart": end, "selectionEnd": end }),
            )
            .await;
    }
    page.browser
        .cdp("Input.insertText", json!({ "text": COMMITTED }))
        .await;
}

/// Asserts that the page raised one whole composition cycle: one start, an
/// update per pre-edit, and one end that carries the committed text.
async fn assert_one_cycle(page: &Page) {
    let seen = page
        .browser
        .wait_for(
            "the end of the composition",
            "const c = window.beltComposition; \
             return c.some(([t]) => t === 'compositionend') && c;",
            STEP,
        )
        .await;
    let kinds = |kind: &str| {
        seen.as_array()
            .into_iter()
            .flatten()
            .filter(|e| e[0] == kind)
            .count()
    };
    assert_eq!(kinds("compositionstart"), 1, "{seen}");
    assert!(kinds("compositionupdate") >= PREEDITS.len(), "{seen}");
    assert_eq!(kinds("compositionend"), 1, "{seen}");
    let last = seen.as_array().and_then(|a| a.last()).cloned();
    assert_eq!(last, Some(json!(["compositionend", COMMITTED])), "{seen}");
}

/// Asserts that `text` holds the committed run once, and no kana of a
/// pre-edit.
fn assert_committed_once(text: &str) {
    assert_eq!(text.matches(COMMITTED).count(), 1, "{text}");
    for kana in ['\u{306b}', '\u{307b}', '\u{3093}', '\u{3054}'] {
        assert!(!text.contains(kana), "a pre-edit was committed: {text}");
    }
}

/// Focuses the page's pipe, the hidden field that takes the typed text.
async fn focus_pipe(page: &Page) {
    page.browser
        .script("document.querySelector('.ambre-input').focus();", json!([]))
        .await;
}

/// Waits until the Prose status bar counts `figure`, such as "6 with spaces".
/// An uncaught error of the page ends the wait at once and fails with its
/// stack, so a count that never comes names its cause.
async fn wait_counted(page: &Page, figure: &str) {
    let figure = Value::String(figure.to_owned());
    page.browser
        .wait_for(
            &format!("the count {figure}"),
            &format!(
                "if (window.beltErrors && beltErrors.length) return beltErrors; \
                 return [...document.querySelectorAll('.ambre-statusbar .stat')] \
                   .some(s => s.textContent === {figure});"
            ),
            STEP,
        )
        .await;
    assert_no_page_error(page).await;
}

/// The text of the one file the page downloads next.
async fn downloaded_text(page: &Page) -> String {
    let (_, bytes) = page.download().await;
    String::from_utf8(bytes).expect("the export is text")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{drag, key};

    /// `CHROME-019`, `CHROME-018`: a composition in a Prose page commits its
    /// final text once. The count holds the typed prefix and the three kanji
    /// alone, so no pre-edit reached the document, and the export reads the
    /// committed run back.
    ///
    /// `ARCH-120`: the UI thread and the Logic thread segment the kanji at the
    /// same time, and no uncaught error reaches the page. A warning of ICU on
    /// the standard error once trapped the commit in about one run in fifty.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "composes through the DevTools input methods, which Firefox does not speak"
    )]
    async fn a_composition_in_a_prose_page_commits_its_text_once() {
        let page = Page::new_document("prose").await;
        let (ox, oy) = page.canvas_origin().await;
        let p = (ox + 300.0, oy + 200.0);
        page.browser.perform(json!([drag(p, p, 0)])).await;
        let keys: Vec<Value> = ["H", "i", " "]
            .iter()
            .flat_map(|k| {
                [
                    json!({ "type": "keyDown", "value": k }),
                    json!({ "type": "keyUp", "value": k }),
                ]
            })
            .collect();
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": keys }]))
            .await;
        wait_counted(&page, "3 with spaces").await;
        record_composition(&page).await;
        compose(&page).await;
        assert_one_cycle(&page).await;
        wait_counted(&page, "6 with spaces").await;

        page.command("Export to Markdown (.md)").await;
        let text = downloaded_text(&page).await;
        assert!(text.contains(&format!("Hi {COMMITTED}")), "{text}");
        assert_committed_once(&text);
        assert_no_page_error(&page).await;
    }

    /// `CHROME-019`, `CHROME-018`: a composition in a Trame cell commits its
    /// final text once, into the cell. The CSV export reads the cell back.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "composes through the DevTools input methods, which Firefox does not speak"
    )]
    async fn a_composition_in_a_trame_cell_commits_its_text_once() {
        let page = Page::new_document("trame").await;
        focus_pipe(&page).await;
        record_composition(&page).await;
        compose(&page).await;
        assert_one_cycle(&page).await;
        // The formula bar shows the cell's editor, so it reads the commit
        // before Enter writes the cell.
        page.browser
            .wait_for(
                "the commit in the formula bar",
                &format!(
                    "return document.querySelector('#ambre-formula-input').value === {};",
                    Value::String(COMMITTED.to_owned())
                ),
                STEP,
            )
            .await;
        page.browser.perform(json!([key("\u{E007}")])).await;
        // Enter writes the cell and moves the selection down.
        page.browser
            .wait_for(
                "the selection on A2",
                "const l = document.querySelector('.ambre-a11y-live'); \
                 return !!l && l.textContent.startsWith('A2');",
                STEP,
            )
            .await;

        page.command("Export CSV").await;
        let text = downloaded_text(&page).await;
        assert!(text.starts_with(COMMITTED), "{text}");
        assert_committed_once(&text);
        assert_no_page_error(&page).await;
    }
}
