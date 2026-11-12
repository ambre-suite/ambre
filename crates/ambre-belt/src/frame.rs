//! The frame loop, read back from the page: behind a modal, and idle.
//!
//! A paused loop still takes its animation frames: each one runs the relay
//! and stops there (`CANVAS-058`). So a count of `requestAnimationFrame`
//! callbacks cannot tell a paused loop from a running one. The present can:
//! a paused frame draws nothing. A running frame presents when it rebuilds a
//! layer (`CANVAS-010`), and a spreadsheet rebuilds its overlay at each tick
//! of its read-out, twice a second. The canvas renders through WebGL2 on the
//! page's own thread, so a page script counts the draw calls of every
//! context. The product carries no hook for it.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::page::{Page, STEP};
use crate::relay::Relay;

/// Counts in `window.beltDraws` every draw and blit any WebGL context makes.
/// It wraps the methods of each context type. A call finds its method on the
/// type at each call, so the script may also run after boot. A second run in
/// one page wraps nothing again.
///
/// It also watches the animation frame callbacks after a key. The loop is the
/// callback that draws, and it hands the same function to each frame. Once
/// the first draw call after the key lands, `window.beltLate` counts the runs
/// of that loop after the key that drew nothing. A frame's timestamp cannot
/// tell two frames apart: Firefox runs one timestamp across a key.
const COUNT_DRAWS: &str = "if (window.beltDraws === undefined) { \
    window.beltDraws = 0; window.beltRan = null; window.beltLate = -1; \
    const raf = window.requestAnimationFrame; \
    window.requestAnimationFrame = function (cb) { \
      return raf.call(window, function (t) { \
        const drawn = window.beltDraws; \
        const r = cb(t); \
        if (window.beltRan && window.beltLate < 0) { \
          if (window.beltDraws > drawn) window.beltLate = window.beltRan.filter(f => f === cb).length; \
          else window.beltRan.push(cb); \
        } \
        return r; \
      }); \
    }; \
    window.addEventListener('keydown', function () { \
      window.beltRan = []; window.beltLate = -1; \
    }, true); \
    for (const C of [self.WebGL2RenderingContext, self.WebGLRenderingContext]) { \
      if (!C) continue; \
      for (const m of ['drawArrays', 'drawElements', 'drawArraysInstanced', \
                       'drawElementsInstanced', 'drawRangeElements', 'blitFramebuffer']) { \
        const f = C.prototype[m]; \
        if (!f) continue; \
        C.prototype[m] = function (...a) { window.beltDraws++; return f.apply(this, a); }; \
      } \
    } \
  }";

/// How long a window of frames is watched. A running loop presents some
/// sixty times in it, a software device included.
const WATCH: Duration = Duration::from_millis(1000);

/// Opens a new document of `kind` in a page that counts its draw calls from
/// the first script of every document it loads.
async fn counted_document(kind: &str) -> Page {
    let page = Page::home(Arc::new(Relay::start().await)).await;
    page.browser
        .cdp(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": COUNT_DRAWS }),
        )
        .await;
    page.sign_in(&format!("belt-{}", uuid::Uuid::new_v4()))
        .await;
    page.open_new(kind).await;
    page
}

/// Installs the draw counter in the page as it stands, on any browser. The
/// counting starts with the next call.
pub(crate) async fn count_draws(page: &Page) {
    page.browser
        .script(&format!("{COUNT_DRAWS} return true;"), json!([]))
        .await;
}

/// The draw calls the page has made since the counter was installed.
pub(crate) async fn draws(page: &Page) -> i64 {
    page.browser
        .script("return window.beltDraws;", json!([]))
        .await
        .as_i64()
        .expect("the draw counter is installed")
}

/// The draw calls the page made over one [`WATCH`].
pub(crate) async fn draws_over_a_watch(page: &Page) -> i64 {
    let before = draws(page).await;
    tokio::time::sleep(WATCH).await;
    draws(page).await - before
}

/// The runs of the loop that drew nothing after the last key, before the one
/// that drew. `0` is a draw on the next frame. Waits for that draw call.
pub(crate) async fn late_frames_after_key(page: &Page) -> i64 {
    page.browser
        .wait_for(
            "a draw call after the key",
            "return window.beltLate >= 0;",
            STEP,
        )
        .await;
    page.browser
        .script("return window.beltLate;", json!([]))
        .await
        .as_i64()
        .expect("the draw counter is installed")
}

/// Whether the command palette's modal is open.
async fn palette_open(page: &Page) -> Value {
    page.browser
        .script(
            "const d = document.querySelector('.ambre-palette-modal'); return !!d && d.open;",
            json!([]),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::key;

    /// `CHROME-011`, `CANVAS-058`: an open modal stops the frames, and
    /// `CHROME-088`, `CHROME-089`: a real Escape closes it and the frames
    /// come back. The palette is the modal, opened with its chord.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "installs its draw-call counter through the DevTools Protocol, which Firefox does not speak"
    )]
    async fn a_modal_stops_the_frames_and_escape_gives_them_back() {
        let page = counted_document("trame").await;
        assert!(draws_over_a_watch(&page).await > 0, "a running loop draws");

        page.focus_canvas().await;
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": "\u{E009}" },
                { "type": "keyDown", "value": "k" },
                { "type": "keyUp", "value": "k" },
                { "type": "keyUp", "value": "\u{E009}" },
            ]}]))
            .await;
        page.browser
            .wait_for(
                "the palette",
                "const d = document.querySelector('.ambre-palette-modal'); return !!d && d.open;",
                STEP,
            )
            .await;
        // The frame in flight when the modal opened may still present.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(draws_over_a_watch(&page).await, 0, "the open modal drew");

        page.browser.perform(json!([key("\u{E00C}")])).await;
        page.browser
            .wait_for(
                "the palette closed",
                "const d = document.querySelector('.ambre-palette-modal'); return !!d && !d.open;",
                STEP,
            )
            .await;
        assert_eq!(palette_open(&page).await, json!(false));
        // A loaded machine may be slow over the first frame. A loop left
        // suspended never draws again, so the wait is for one more draw call.
        let closed = draws(&page).await;
        page.browser
            .wait_for(
                "a draw call once the modal closed",
                &format!("return window.beltDraws > {closed};"),
                STEP,
            )
            .await;
    }
}
