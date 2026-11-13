//! The converter modules, each fetched on demand, driven in the browser.

#[cfg(test)]
mod tests {
    use crate::page::Page;
    use crate::toile::tests::{draw_rect, select_all_until};

    /// `ARCH-082`: in one session, a CSV import and a PDF export complete
    /// through the converter modules. Neither instance links a CSV reader or a
    /// PDF writer (`ARCH-078`). So each file can only come from a module the
    /// Sync worker fetched and checked. The import loads the tables module,
    /// and the export loads the PDF module beside it.
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
        page.wait_toast("Imported 2 rows and 2 columns").await;

        page.open_new("toile").await;
        draw_rect(&page, (300.0, 200.0), (420.0, 300.0)).await;
        page.command("Export deck as PDF").await;
        let (name, bytes) = page.download().await;
        assert!(name.ends_with(".pdf"), "{name}");
        assert!(bytes.starts_with(b"%PDF-"), "a PDF header");
    }

    /// `ARCH-097`, `TOILE-IO-131`: in the served build, a JPEG dropped on a
    /// board lands, its colour on screen. Main links no JPEG decoder
    /// (`ARCH-078`), so the thumbnail can only come from the images module
    /// that the converter thread loaded beside the Logic thread.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_jpeg_drop_lands_through_the_converter_thread() {
        const MAGENTA: [u8; 3] = [255, 0, 255];
        let square = image::RgbImage::from_pixel(120, 120, image::Rgb(MAGENTA));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
            .encode_image(&square)
            .expect("a JPEG");
        let page = Page::new_document("toile").await;
        page.drop_file((300.0, 200.0), "one.jpg", "image/jpeg", &jpeg)
            .await;
        select_all_until(&page, "Image").await;
        let deadline = std::time::Instant::now() + crate::page::STEP;
        loop {
            let seen = page.color_at((300.0, 200.0)).await;
            if seen.iter().zip(MAGENTA).all(|(s, m)| s.abs_diff(m) <= 16) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the board shows {seen:?} where the picture is"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
}
