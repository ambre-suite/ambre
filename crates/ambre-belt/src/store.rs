//! The local store, as the browser holds it.

#[cfg(test)]
mod tests {
    use serde_json::json;

    use std::sync::Arc;

    use crate::page::{Page, STEP, drag};
    use crate::relay::Relay;

    /// `RESID-020`: once the local store opens, the page has asked for it to
    /// persist. A browser that has not granted the permission keeps the store
    /// best-effort. Once it grants it, the store reads persistent only when the
    /// page asks.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_local_store_is_asked_to_persist() {
        let page = Page::home(Arc::new(Relay::start().await)).await;
        assert!(
            !persisted(&page).await,
            "the store persisted without the permission"
        );
        page.browser.permit("persistent-storage").await;

        page.sign_in(&format!("belt-{}", uuid::Uuid::new_v4()))
            .await;
        page.open_new("toile").await;
        let deadline = std::time::Instant::now() + STEP;
        while !persisted(&page).await {
            assert!(
                std::time::Instant::now() < deadline,
                "the page never asked to persist"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// `RESID-017`: a write the quota refuses evicts the copy the relay holds
    /// of another document, then is tried once more. The quota here refuses
    /// that retry too, so the page pauses editing and says so (`RESID-018`).
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_refused_write_evicts_what_the_relay_holds_then_retries() {
        let page = Page::new_document("trame").await;
        let held = document_key(&page).await;
        type_cells(&page, 3).await;
        let deadline = std::time::Instant::now() + STEP;
        while stored(&page, &held).await == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "the first document was never stored"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // A route change within the page closes the first document without a
        // reload. Its view is gone, and once Sync holds it light the relay has
        // every change: the store may then evict its copy (`RESID-019`).
        page.browser
            .script(
                "history.pushState(null, '', arguments[0]); \
                 window.dispatchEvent(new PopStateEvent('popstate'));",
                json!([format!("/trame/{}", uuid::Uuid::now_v7())]),
            )
            .await;
        page.wait_canvas().await;
        // No event on the page marks the light state; three seconds is ample
        // on a local relay.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(
            stored(&page, &held).await > 0,
            "the route change evicted the first copy"
        );
        let open = document_key(&page).await;
        page.browser.script(FULL_QUOTA, json!([])).await;
        type_cells(&page, 1).await;
        page.wait_toast("This device is out of storage.").await;
        let refused = page
            .browser
            .script("return window.__beltRefused;", json!([]))
            .await;
        let refused = refused.as_array().cloned().unwrap_or_default();
        let tries = refused
            .iter()
            .filter(|k| k.as_str().is_some_and(|k| k.starts_with(&open)))
            .count();
        assert!(
            tries >= 2,
            "the refused write was not tried again: {refused:?}"
        );
        assert_eq!(
            stored(&page, &held).await,
            0,
            "the first document's copy is still stored"
        );
    }

    /// Makes the page's store refuse every snapshot and delta write as a full
    /// quota does: the transaction aborts, and its error names the quota.
    /// Chrome does not apply a quota set through the DevTools Protocol to
    /// IndexedDB, so no driver can fill a real one in a test's time. The shim
    /// records each refused key in `window.__beltRefused`.
    const FULL_QUOTA: &str = "window.__beltRefused = []; \
        const put = IDBObjectStore.prototype.put; \
        IDBObjectStore.prototype.put = function (value, key) { \
          const request = put.call(this, value, key); \
          if (this.name === 'snapshots' || this.name === 'deltas') { \
            const txn = this.transaction; \
            window.__beltRefused.push(String(key)); \
            Object.defineProperty(txn, 'error', { \
              get: () => new DOMException('the quota is full', 'QuotaExceededError') }); \
            queueMicrotask(() => txn.abort()); \
          } \
          return request; \
        };";

    /// The key prefix the store gives the open document: its id in hex.
    async fn document_key(page: &Page) -> String {
        let path = page
            .browser
            .script("return location.pathname;", json!([]))
            .await;
        let id = path
            .as_str()
            .and_then(|p| p.rsplit('/').next())
            .expect("a document route");
        id.replace('-', "")
    }

    /// How many snapshots and deltas the store holds under the key `prefix`.
    async fn stored(page: &Page, prefix: &str) -> u64 {
        page.browser
            .script_async(
                "const [prefix, done] = arguments; \
                 const open = indexedDB.open('ambre-persistence'); \
                 open.onerror = () => done(0); \
                 open.onsuccess = async () => { \
                   const db = open.result; let n = 0; \
                   for (const s of ['snapshots', 'deltas']) { \
                     const keys = await new Promise(ok => { \
                       const q = db.transaction(s).objectStore(s).getAllKeys(); \
                       q.onsuccess = () => ok(q.result); q.onerror = () => ok([]); }); \
                     n += keys.filter(k => String(k).startsWith(prefix)).length; } \
                   db.close(); done(n); };",
                json!([prefix]),
            )
            .await
            .as_u64()
            .unwrap_or(0)
    }

    /// Clicks the first cells and types one line into each of `n` of them.
    async fn type_cells(page: &Page, n: usize) {
        let (ox, oy) = page.canvas_origin().await;
        let p = (ox + 150.0, oy + 60.0);
        page.browser.perform(json!([drag(p, p, 0)])).await;
        let mut keys = Vec::new();
        for i in 0..n {
            for c in format!("line {i}").chars() {
                keys.push(json!({ "type": "keyDown", "value": c.to_string() }));
                keys.push(json!({ "type": "keyUp", "value": c.to_string() }));
            }
            keys.push(json!({ "type": "keyDown", "value": "\u{E007}" }));
            keys.push(json!({ "type": "keyUp", "value": "\u{E007}" }));
        }
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": keys }]))
            .await;
    }

    async fn persisted(page: &Page) -> bool {
        page.browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 navigator.storage.persisted().then(done, () => done(false));",
                json!([]),
            )
            .await
            .as_bool()
            .unwrap_or(false)
    }
}
