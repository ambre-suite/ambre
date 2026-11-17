//! The account keys on the home screen, driven as a person drives them.
//!
//! A development relay answers the setup with no provider. The page reaches the
//! setup and the recovery through the two links of its development panel.
//! The setup creates the keys on the Sync worker, which answers with the 24
//! recovery words. The recovery form hands typed words back to that worker,
//! which refuses a phrase that spells no seed before it derives any key.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::page::{BOOT, Page, STEP};
use crate::relay::Relay;

/// A passphrase above the strength floor of `ZK-035`.
const STRONG: &str = "violet-harbor-quietly-folds-ninety-kites";

/// How long a setup may take: the worker derives the sealing key with
/// Argon2, in a debug build.
const SETUP: std::time::Duration = std::time::Duration::from_secs(90);

/// Clicks the development link `selector` once the panel shows it.
async fn follow(page: &Page, selector: &str) {
    page.browser
        .wait_for(
            selector,
            &format!(
                "const b = document.querySelector({}); if (!b) return false; \
                 b.click(); return true;",
                Value::String(selector.to_owned())
            ),
            BOOT,
        )
        .await;
}

/// Types `fields`, pairs of a selector and a text, into the form shown, then
/// submits it.
async fn submit(page: &Page, fields: Value) {
    page.browser
        .script(
            "for (const [selector, text] of arguments[0]) { \
               document.querySelector(selector).value = text; \
             } \
             document.querySelector('.ambre-keys-submit').click();",
            json!([fields]),
        )
        .await;
}

/// Submits `words` to the recovery form, with a strong new passphrase.
async fn recover(page: &Page, words: &str) {
    submit(
        page,
        json!([
            [".ambre-keys-words-field", words],
            [".ambre-keys-passphrase", STRONG],
            [".ambre-keys-confirm", STRONG],
        ]),
    )
    .await;
}

/// Records in `window.beltRefusals` each text the refusal line of the form
/// shows, from now on.
async fn record_refusals(page: &Page) {
    page.browser
        .script(
            "window.beltRefusals = []; \
             const line = document.querySelector('.ambre-keys-refusal'); \
             new MutationObserver(() => window.beltRefusals.push(line.textContent)) \
               .observe(line, { childList: true, characterData: true, subtree: true });",
            json!([]),
        )
        .await;
}

/// Waits until the refusal line reads `text`, and returns every text it
/// showed since the recording began.
async fn wait_refusal(page: &Page, text: &str) -> Vec<String> {
    page.browser
        .wait_for(
            text,
            &format!(
                "const line = document.querySelector('.ambre-keys-refusal'); \
                 return !!line && line.textContent === {};",
                Value::String(text.to_owned())
            ),
            STEP,
        )
        .await;
    let shown = page
        .browser
        .script("return window.beltRefusals;", json!([]))
        .await;
    serde_json::from_value(shown).expect("a list of refusal texts")
}

/// The words of a refusal of the recovery words, as `AuthFailure` spells them.
const UNKNOWN_WORD: &str =
    "Recovery failed: a word is not one of the recovery words. Check its spelling.";
const BAD_CHECKSUM: &str =
    "Recovery failed: the words do not match each other. Check each word and its order.";
const WRONG_COUNT: &str = "Recovery failed: the recovery phrase holds 24 words.";

#[cfg(test)]
mod tests {
    use super::*;

    /// `CHROME-099`, `CHROME-128`, `CHROME-130`: the setup shows the 24 recovery words
    /// once. The confirmation takes them off the page, and neither the page,
    /// a reload nor the web storage reads them afterwards. The recovery form
    /// then reads them back. The Sync worker refuses a mistyped word, a wrong
    /// count and a bad checksum, each in its own words. It passes the words
    /// the setup showed.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_setup_shows_the_words_once_and_the_recovery_reads_them() {
        let page = Page::home(Arc::new(Relay::start().await)).await;
        follow(&page, ".ambre-signin-setup").await;
        let address = format!("belt-{}@test.local", uuid::Uuid::new_v4());
        submit(
            &page,
            json!([
                [".ambre-keys-name", "Ada"],
                [".ambre-keys-email", address],
                [".ambre-keys-passphrase", STRONG],
                [".ambre-keys-confirm", STRONG],
            ]),
        )
        .await;

        let phrase = page
            .browser
            .wait_for(
                "the recovery words",
                "const items = document.querySelectorAll('.ambre-recovery-words li'); \
                 return items.length === 24 \
                   ? Array.from(items).map(item => item.textContent).join(' ') : false;",
                SETUP,
            )
            .await;
        let phrase = phrase.as_str().expect("the words as text").to_owned();
        let words: Vec<&str> = phrase.split(' ').collect();
        assert_eq!(words.len(), 24, "{phrase}");
        assert!(
            words
                .iter()
                .all(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase())),
            "a word is not a lowercase list word: {phrase}"
        );

        follow(&page, ".ambre-words-done").await;
        let gone = format!(
            "return !document.querySelector('.ambre-recovery-words') \
               && !document.body.innerHTML.includes({0}) \
               && !JSON.stringify(Object.assign({{}}, localStorage)).includes({0}) \
               && !JSON.stringify(Object.assign({{}}, sessionStorage)).includes({0});",
            Value::String(words[..3].join(" "))
        );
        page.browser
            .wait_for("the words gone from the page", &gone, STEP)
            .await;

        // A reload shows the sign-in panel, and no word.
        page.browser.goto(&format!("{}/", page.origin)).await;
        follow(&page, ".ambre-signin-recover").await;
        assert_eq!(
            page.browser.script(&gone, json!([])).await,
            Value::Bool(true),
            "the words are readable after the setup"
        );

        record_refusals(&page).await;
        let mistyped = format!("notaword {}", words[1..].join(" "));
        recover(&page, &mistyped).await;
        wait_refusal(&page, UNKNOWN_WORD).await;

        recover(&page, &words[1..].join(" ")).await;
        wait_refusal(&page, WRONG_COUNT).await;

        // The all-zero seed spells its last word `art`, never `abandon`.
        recover(&page, &["abandon"; 24].join(" ")).await;
        wait_refusal(&page, BAD_CHECKSUM).await;

        // The words the setup showed draw no refusal. The worker answers in
        // order, so the refusal that follows them shows they were read.
        record_refusals(&page).await;
        recover(&page, &phrase.to_uppercase()).await;
        recover(&page, &mistyped).await;
        let shown = wait_refusal(&page, UNKNOWN_WORD).await;
        assert!(
            shown
                .iter()
                .all(|text| text.is_empty() || text == UNKNOWN_WORD),
            "the words the setup showed were refused: {shown:?}"
        );
    }
}
