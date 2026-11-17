//! The account keys on the home screen, driven as a person drives them.
//!
//! A development relay answers the setup with no provider. The page reaches the
//! setup and the recovery through the two links of its development panel.
//! The setup creates the keys on the Sync worker, which answers with the 24
//! recovery words, then the session. The recovery form hands typed words back
//! to that worker, which refuses a phrase that spells no seed before it derives
//! any key.
//!
//! The faces of a provider account need a provider return. The belt's server
//! stands in for its exchange with the account a setup created
//! (`crate::serve`), so the page unlocks a real account of the relay.

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

/// A second passphrase above the floor, which a recovery sets.
const RECOVERED: &str = "amber-lantern-gravel-sixty-orchids-hum";

/// Sets up an account named `name` through the development panel, and returns
/// the 24 recovery words the page shows once.
async fn set_up(page: &Page, name: &str) -> String {
    follow(page, ".ambre-signin-setup").await;
    let address = format!("belt-{}@test.local", uuid::Uuid::new_v4());
    submit(
        page,
        json!([
            [".ambre-keys-name", name],
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
    phrase.as_str().expect("the words as text").to_owned()
}

/// Waits until the document list shows under the user `name`.
async fn wait_list(page: &Page, name: &str) {
    page.browser
        .wait_for(
            "the document list of the user",
            &format!(
                "const user = document.querySelector('.ambre-landing-user'); \
                 return !!document.querySelector('.ambre-doc-list') \
                   && !document.querySelector('.ambre-keys') \
                   && !!user && user.textContent === {};",
                Value::String(name.to_owned())
            ),
            SETUP,
        )
        .await;
}

/// Types `passphrase` into the unlock face, once it shows, and submits it.
async fn unlock(page: &Page, passphrase: &str) {
    page.browser
        .wait_for(
            "the unlock face",
            &format!(
                "const field = document.querySelector('.ambre-keys-passphrase'); \
                 const button = document.querySelector('.ambre-signin-btn'); \
                 if (!field || !button || button.disabled \
                   || !document.querySelector('.ambre-keys-recover')) return false; \
                 field.value = {}; \
                 button.click(); \
                 return true;",
                Value::String(passphrase.to_owned())
            ),
            SETUP,
        )
        .await;
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
    /// count and a bad checksum, each in its own words. It reads the words
    /// the setup showed as a seed.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_setup_shows_the_words_once_and_the_recovery_reads_them() {
        let page = Page::home(Arc::new(Relay::start().await)).await;
        let phrase = set_up(&page, "Ada").await;
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

        // The words the setup showed draw no refusal of the words. A
        // signed-out page has no session to re-seal, so the worker answers
        // the refusal of the re-seal, never one of the words.
        recover(&page, &phrase.to_uppercase()).await;
        let answer = page
            .browser
            .wait_for(
                "the answer to the words the setup showed",
                "const line = document.querySelector('.ambre-keys-refusal'); \
                 return !!line && line.textContent.length > 0 ? line.textContent : false;",
                SETUP,
            )
            .await;
        let answer = answer.as_str().unwrap_or_default().to_owned();
        assert!(
            ![UNKNOWN_WORD, BAD_CHECKSUM, WRONG_COUNT].contains(&answer.as_str()),
            "the words the setup showed were refused: {answer}"
        );
    }

    /// `CHROME-132`, `SYNC-261`: a setup ends in a session. The words cover
    /// the list until the person confirms them, and the list of the new user
    /// then shows, with no sign-in panel between.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_setup_ends_in_the_session_once_the_words_are_confirmed() {
        let page = Page::home(Arc::new(Relay::start().await)).await;
        set_up(&page, "Ada").await;
        // The session is announced with the words: wait for the user's name in
        // the header, then read that the words still cover the list.
        page.browser
            .wait_for(
                "the session under the words",
                "const user = document.querySelector('.ambre-landing-user'); \
                 return !!user && user.textContent === 'Ada';",
                STEP,
            )
            .await;
        assert_eq!(
            page.browser
                .script(
                    "return !document.querySelector('.ambre-doc-list') \
                       && !!document.querySelector('.ambre-recovery-words');",
                    json!([]),
                )
                .await,
            Value::Bool(true),
            "the list shows over the words"
        );
        follow(&page, ".ambre-words-done").await;
        wait_list(&page, "Ada").await;
        assert_eq!(
            page.browser
                .script(
                    "return !document.querySelector('.ambre-signin-input');",
                    json!([]),
                )
                .await,
            Value::Bool(true),
            "the sign-in panel shows after the setup"
        );
    }

    /// `CHROME-133`, `CHROME-134`: the unlock face of a provider account links
    /// to the recovery form. A recovery there re-seals the account under a new
    /// passphrase. The unlock face then refuses the old passphrase and opens
    /// the list under the new one.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_unlock_face_leads_to_a_recovery_that_opens_the_account() {
        let page = Page::home(Arc::new(Relay::start().await)).await;
        let phrase = set_up(&page, "Ada").await;
        follow(&page, ".ambre-words-done").await;
        wait_list(&page, "Ada").await;

        // The provider comes back to this tab, with the state it was handed.
        page.browser
            .script(
                "sessionStorage.setItem('ambre.signin.state', 'belt-state');",
                json!([]),
            )
            .await;
        page.browser
            .goto(&format!(
                "{}/signin/google?code=belt-code&state=belt-state",
                page.origin
            ))
            .await;
        follow(&page, ".ambre-keys-recover").await;

        // A refusal reads on the form that covers the gate.
        record_refusals(&page).await;
        recover(&page, &["abandon"; 24].join(" ")).await;
        wait_refusal(&page, BAD_CHECKSUM).await;

        submit(
            &page,
            json!([
                [".ambre-keys-words-field", phrase],
                [".ambre-keys-passphrase", RECOVERED],
                [".ambre-keys-confirm", RECOVERED],
            ]),
        )
        .await;
        page.browser
            .wait_for(
                "the unlock face after the recovery",
                "return !document.querySelector('.ambre-keys-words-field') \
                   && !!document.querySelector('.ambre-keys-recover');",
                SETUP,
            )
            .await;

        unlock(&page, STRONG).await;
        page.browser
            .wait_for(
                "the old passphrase refused",
                "const line = document.querySelector('.ambre-keys-refusal'); \
                 return !!line && line.textContent.length > 0;",
                SETUP,
            )
            .await;
        unlock(&page, RECOVERED).await;
        wait_list(&page, "Ada").await;
    }
}
