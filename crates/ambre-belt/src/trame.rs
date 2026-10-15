//! A Trame document driven in the browser.

#[cfg(test)]
mod tests {
    use crate::busy::{FLOOD, clear_cell, pause_logic};
    use crate::page::Page;

    /// `TRAME-IO-040`: a file the full queue cannot carry tells the user that
    /// it did not import.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn an_import_the_full_queue_refuses_says_so() {
        let page = Page::new_document("trame").await;
        let paused = pause_logic(&page).await;
        clear_cell(&page, FLOOD).await;
        page.wait_toast("The document is catching up.").await;

        page.arm_picker().await;
        page.command("Import CSV").await;
        page.pick("one.csv", "text/csv", b"a,b\n1,2\n").await;
        page.wait_toast("The file did not import").await;
        // The toast is the whole answer: let the worker go before the page closes.
        page.resume_worker(paused).await;
    }
}
