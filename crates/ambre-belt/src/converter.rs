//! The converter module, fetched on demand, driven in the browser.

#[cfg(test)]
mod tests {
    use crate::page::Page;
    use crate::toile::tests::draw_rect;

    /// `ARCH-082`: in one session, a CSV import and a PDF export complete
    /// through the converter module. Neither instance links a CSV reader or a
    /// PDF writer (`ARCH-078`). So each file can only come from the module the
    /// Sync worker fetched and checked. The import loads it, and the export
    /// runs on the instance the import loaded.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_csv_import_and_a_pdf_export_run_through_the_fetched_module() {
        let page = Page::new_document("trame").await;
        page.arm_picker().await;
        page.command("Import CSV").await;
        page.pick("one.csv", "text/csv", b"a,b\n1,2\n").await;
        page.wait_toast("Imported 2 rows x 2 columns").await;

        page.open_new("toile").await;
        draw_rect(&page, (300.0, 200.0), (420.0, 300.0)).await;
        page.command("Export deck as PDF").await;
        let (name, bytes) = page.download().await;
        assert!(name.ends_with(".pdf"), "{name}");
        assert!(bytes.starts_with(b"%PDF-"), "a PDF header");
    }
}
