//! The account keys on the home screen, driven as a person drives them.
//!
//! No anonymous account setup exists (`SRV-319`): the keys faces follow a
//! provider return. A development relay holds no provider, so the belt's
//! server stands in for its exchange (`crate::serve`). The code of the return
//! names the person, and the relay holds an account under that name that owes
//! its keys. The setup face creates the keys on the Sync worker, which answers
//! with the 24 recovery words, then the session. A later return under the same
//! name shows the unlock face of that account. The recovery form hands typed
//! words back to the worker, which refuses a phrase that spells no seed before
//! it derives any key.

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
pub(crate) async fn follow(page: &Page, selector: &str) {
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

/// A name no other test signs in under: the relays of a run share one
/// database, and the stand-in signs in by name.
fn unique(name: &str) -> String {
    let tag = uuid::Uuid::new_v4().simple().to_string();
    format!("{name}-{}", tag.get(..8).unwrap_or_default())
}

/// The provider comes back to this tab for the person `name`, with the state
/// it was handed (`crate::serve`).
async fn provider_return(page: &Page, name: &str) {
    page.browser
        .script(
            "sessionStorage.setItem('ambre.signin.state', 'belt-state');",
            json!([]),
        )
        .await;
    page.browser
        .goto(&format!(
            "{}/signin/google?code={name}&state=belt-state",
            page.origin
        ))
        .await;
}

/// Signs `name` in through a provider return, sets up the keys the account
/// owes, and returns the 24 recovery words the page shows once.
async fn set_up(page: &Page, name: &str) -> String {
    provider_return(page, name).await;
    set_up_keys(page).await
}

/// Sets up the keys the account of a provider return owes, once the page shows
/// the setup face. Returns the 24 recovery words the page shows once.
pub(crate) async fn set_up_keys(page: &Page) -> String {
    page.browser
        .wait_for(
            "the setup face",
            "return !!document.querySelector('.ambre-keys-confirm');",
            SETUP,
        )
        .await;
    page.browser
        .script(
            "document.querySelector('.ambre-keys-passphrase').value = arguments[0]; \
             document.querySelector('.ambre-keys-confirm').value = arguments[0]; \
             document.querySelector('.ambre-signin-btn').click();",
            json!([STRONG]),
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
pub(crate) async fn wait_list(page: &Page, name: &str) {
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
    /// the setup showed as a seed. Signed out, the panel offers no recovery
    /// (`CHROME-135`), so the recovery runs from the unlock face. A reload
    /// before the sign-out resumes the session and shows no word either.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_setup_shows_the_words_once_and_the_recovery_reads_them() {
        let page = Page::home(Arc::new(Relay::start().await)).await;
        let name = unique("Ada");
        let phrase = set_up(&page, &name).await;
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

        // A reload resumes the session the device kept (`SRV-322`), and
        // shows no word. A sign-out then shows the sign-in panel. Signed out,
        // the panel links to no recovery: the form would fail as not signed in.
        page.browser.goto(&format!("{}/", page.origin)).await;
        wait_list(&page, &name).await;
        assert_eq!(
            page.browser.script(&gone, json!([])).await,
            Value::Bool(true),
            "the words are readable after a reload"
        );
        follow(&page, ".ambre-sign-out").await;
        page.browser
            .wait_for(
                "the sign-in panel with no recovery link",
                "return !!document.querySelector('.ambre-signin-input') \
                   && !document.querySelector('.ambre-keys') \
                   && !document.querySelector('.ambre-signin-recover');",
                BOOT,
            )
            .await;
        assert_eq!(
            page.browser.script(&gone, json!([])).await,
            Value::Bool(true),
            "the words are readable after the setup"
        );

        // The provider comes back to this tab, so a session holds and the
        // unlock face links to the recovery form (`CHROME-133`).
        provider_return(&page, &name).await;
        follow(&page, ".ambre-keys-recover").await;

        record_refusals(&page).await;
        let mistyped = format!("notaword {}", words[1..].join(" "));
        recover(&page, &mistyped).await;
        wait_refusal(&page, UNKNOWN_WORD).await;

        recover(&page, &words[1..].join(" ")).await;
        wait_refusal(&page, WRONG_COUNT).await;

        // The all-zero seed spells its last word `art`, never `abandon`.
        recover(&page, &["abandon"; 24].join(" ")).await;
        wait_refusal(&page, BAD_CHECKSUM).await;

        // The words the setup showed, in capitals, draw no refusal: the
        // session re-seals the seed, and the unlock face comes back.
        recover(&page, &phrase.to_uppercase()).await;
        page.browser
            .wait_for(
                "the unlock face after the recovery",
                "return !document.querySelector('.ambre-keys-words-field') \
                   && !!document.querySelector('.ambre-keys-recover');",
                SETUP,
            )
            .await;
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
        let name = unique("Ada");
        set_up(&page, &name).await;
        // The session is announced with the words. Wait for the user's name
        // in the header, then read that the words show in a modal over the list.
        page.browser
            .wait_for(
                "the session under the words",
                &format!(
                    "const user = document.querySelector('.ambre-landing-user'); \
                     return !!user && user.textContent === {};",
                    Value::String(name.clone())
                ),
                STEP,
            )
            .await;
        assert_eq!(
            page.browser
                .script(
                    "return !!document.querySelector('.ambre-doc-list') \
                       && !!document.querySelector('dialog[open] .ambre-recovery-words');",
                    json!([]),
                )
                .await,
            Value::Bool(true),
            "the words show in no modal over the list"
        );
        follow(&page, ".ambre-words-done").await;
        wait_list(&page, &name).await;
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
        // In the session, the development panel links to the recovery
        // (`CHROME-135`).
        page.browser
            .wait_for(
                "the recovery link in the session",
                "return !!document.querySelector('.ambre-signin-recover');",
                STEP,
            )
            .await;
    }

    /// `CHROME-133`, `CHROME-134`, `SYNC-293`: the unlock face of a provider
    /// account links to the recovery form. A recovery there re-seals the
    /// account under a new passphrase and opens the list on the recovered
    /// keys, with no second unlock.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_unlock_face_leads_to_a_recovery_that_opens_the_account() {
        let page = Page::home(Arc::new(Relay::start().await)).await;
        let name = unique("Ada");
        let phrase = set_up(&page, &name).await;
        follow(&page, ".ambre-words-done").await;
        wait_list(&page, &name).await;

        // The provider comes back to this tab, with the state it was handed.
        provider_return(&page, &name).await;
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
        // The list shows, and no gate asks the new passphrase again (`SYNC-293`).
        wait_list(&page, &name).await;
    }
}
