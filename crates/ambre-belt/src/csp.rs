//! The content policy of the page, read back from the page under it.
//!
//! The post-build hook writes the policy into the page (`SRV-260`). A test
//! below the page reads the text of that policy. Only a browser can say that
//! the built application boots under it, and that an injected script runs
//! nowhere. A policy violation fires `securitypolicyviolation` at the document,
//! so a listener installed before the first script of the page records each
//! one.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::page::{Page, STEP};
use crate::relay::Relay;

/// Records in `window.beltViolations` each violation of the page's policy. A
/// record holds the directive, the blocked address, the text sample and the
/// class of the element.
const RECORD_VIOLATIONS: &str = "if (window.beltViolations === undefined) { \
    window.beltViolations = []; \
    document.addEventListener('securitypolicyviolation', function (e) { \
      window.beltViolations.push(e.effectiveDirective + ' ' + e.blockedURI + ' ' \
        + (e.sample || '') + ' ' + ((e.target && e.target.className) || '')); \
    }, true); \
  }";

/// The violations the page recorded since it loaded.
async fn violations(page: &Page) -> Vec<String> {
    let found = page
        .browser
        .script("return window.beltViolations || null;", json!([]))
        .await;
    let Value::Array(found) = found else {
        panic!("the page installed no violation listener: {found}");
    };
    found
        .into_iter()
        .map(|v| v.as_str().unwrap_or_default().to_owned())
        .collect()
}

/// Opens the home screen in a page that records each policy violation from
/// the first script of every document it loads.
async fn watched_home() -> Page {
    let page = Page::home(Arc::new(Relay::start().await)).await;
    page.browser
        .cdp(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": RECORD_VIOLATIONS }),
        )
        .await;
    page.browser.goto(&format!("{}/", page.origin)).await;
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SRV-260`: the built application boots under the policy its build
    /// writes, and opens a board, a spreadsheet and a text document with no
    /// violation. Its threads and its workers start, which the policy admits
    /// from the page's own origin alone, and none of its code evaluates a
    /// string. A script injected into the page, an injected event handler and
    /// a `blob:` worker each run nowhere. Chrome exempts the driver's own code
    /// from the policy, so the probe cannot try an eval. The empty list of
    /// violations holds that half.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "installs its violation listener through the DevTools Protocol, which Firefox does not speak"
    )]
    async fn the_page_runs_the_scripts_it_ships_and_no_other() {
        let page = watched_home().await;
        let policy = page
            .browser
            .script(
                "const m = document.querySelector('meta[http-equiv=\"Content-Security-Policy\"]'); \
                 return m ? m.content : '';",
                json!([]),
            )
            .await;
        let policy = policy.as_str().unwrap_or_default();
        assert!(
            policy.contains("script-src 'self' 'wasm-unsafe-eval' 'sha256-"),
            "{policy}"
        );
        assert!(!policy.contains("unsafe-inline"), "{policy}");
        assert!(!policy.contains("'unsafe-eval'"), "{policy}");

        page.sign_in(&format!("belt-{}", uuid::Uuid::new_v4()))
            .await;
        for kind in ["toile", "trame", "prose"] {
            page.open_new(kind).await;
            assert_eq!(violations(&page).await, Vec::<String>::new(), "{kind}");
        }
        assert_eq!(
            page.browser
                .script("return self.crossOriginIsolated;", json!([]))
                .await,
            json!(true)
        );

        let refused = page
            .browser
            .script(
                "const s = document.createElement('script'); \
                 s.textContent = 'window.beltInjected = true;'; \
                 document.body.appendChild(s); \
                 const i = document.createElement('img'); \
                 i.setAttribute('onerror', 'window.beltHandled = true;'); \
                 i.src = 'data:,'; \
                 document.body.appendChild(i); \
                 try { \
                   new Worker(URL.createObjectURL(new Blob([''], { type: 'text/javascript' }))); \
                 } catch (e) {} \
                 return window.beltInjected === true;",
                json!([]),
            )
            .await;
        assert_eq!(refused, json!(false));
        for (what, directive) in [
            ("the injected script", "script-src-elem inline"),
            ("the injected handler", "script-src-attr inline"),
            ("the blob worker", "worker-src blob"),
        ] {
            page.browser
                .wait_for(
                    &format!("the violation of {what}"),
                    &format!(
                        "return (window.beltViolations || []).some(v => v.startsWith({}));",
                        Value::String(directive.to_owned())
                    ),
                    STEP,
                )
                .await;
        }
        assert_eq!(
            page.browser
                .script("return window.beltHandled === true;", json!([]))
                .await,
            json!(false)
        );
    }
}
