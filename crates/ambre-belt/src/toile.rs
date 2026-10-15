//! A Toile board, driven in the browser.

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::page::{Page, STEP, drag, drag_still, key};

    /// Draws a rectangle with the shape tool from `a` to `b`, in canvas pixels.
    /// Then selects it with the select tool and waits until the board
    /// announces it. A drawn shape is not selected by its creation.
    async fn draw_rect(page: &Page, a: (f64, f64), b: (f64, f64)) {
        let (ox, oy) = page.canvas_origin().await;
        page.focus_canvas().await;
        page.browser.perform(json!([key("r")])).await;
        // An armed creation tool shows the crosshair (`toile-tools`).
        page.browser
            .wait_for(
                "the shape tool",
                "return document.getElementById('ambre-canvas').style.cursor === 'crosshair';",
                STEP,
            )
            .await;
        page.browser
            .perform(json!([drag((ox + a.0, oy + a.1), (ox + b.0, oy + b.1), 8)]))
            .await;
        page.browser.perform(json!([key("v")])).await;
        click(page, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)).await;
        page.wait_announced("Rectangle").await;
    }

    /// Clicks the canvas at `at`, in canvas pixels.
    async fn click(page: &Page, at: (f64, f64)) {
        let (ox, oy) = page.canvas_origin().await;
        let p = (ox + at.0, oy + at.1);
        page.browser.perform(json!([drag(p, p, 0)])).await;
    }

    /// Selects nothing, with the select tool armed: `Escape` clears the
    /// selection and reverts a creation tool to Select.
    async fn deselect(page: &Page) {
        page.browser.perform(json!([key("\u{E00C}")])).await;
        page.wait_announced("Nothing selected").await;
    }

    /// `TOILE-UI-256`: a drag with `Space` held pans, and once `Space` is up a
    /// click on the rectangle, where the pan carried it, selects it.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn releasing_space_gives_the_board_back_to_its_tool() {
        let page = Page::new_document("toile").await;
        draw_rect(&page, (300.0, 200.0), (420.0, 300.0)).await;
        deselect(&page).await;
        let (ox, oy) = page.canvas_origin().await;

        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": "\u{E00D}" },
            ]}]))
            .await;
        page.browser
            .perform(json!([drag_still(
                (ox + 600.0, oy + 300.0),
                (ox + 700.0, oy + 350.0),
                8
            )]))
            .await;
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyUp", "value": "\u{E00D}" },
            ]}]))
            .await;
        assert_eq!(
            page.announced().await,
            "Nothing selected",
            "the pan selected"
        );

        // The rectangle's centre was (360, 250); the pan moved it by (100, 50).
        click(&page, (460.0, 300.0)).await;
        page.wait_announced("Rectangle").await;
    }

    /// `TOILE-IO-085`: the SVG export of a board holding one rectangle lands in
    /// the browser's download directory, as an SVG drawing that rectangle.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn an_svg_export_reaches_the_download_directory() {
        let page = Page::new_document("toile").await;
        draw_rect(&page, (300.0, 200.0), (420.0, 300.0)).await;
        page.command("Export as SVG").await;
        let (name, bytes) = page.download().await;
        let svg = String::from_utf8(bytes).expect("an SVG is text");
        assert!(name.ends_with(".svg"), "{name}");
        assert!(svg.starts_with("<svg"), "{svg}");
        assert_eq!(svg.matches("<rect").count(), 1, "{svg}");
    }

    /// One rectangle, as a file a user brings onto a board.
    const ONE_RECT: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"60\">\
        <rect x=\"0\" y=\"0\" width=\"100\" height=\"60\" fill=\"#3a7\"/></svg>";

    /// `TOILE-IO-020`: a file dropped on the board lands on it, and so does a
    /// file chosen through the import picker.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_dropped_file_and_a_picked_file_both_land() {
        let page = Page::new_document("toile").await;
        page.drop_file(
            (300.0, 200.0),
            "one.svg",
            "image/svg+xml",
            ONE_RECT.as_bytes(),
        )
        .await;
        select_all_until(&page, "Rectangle").await;

        page.arm_picker().await;
        page.command("Import a file...").await;
        page.pick("two.svg", "image/svg+xml", ONE_RECT.as_bytes())
            .await;
        select_all_until(&page, "2 elements selected").await;
    }

    /// Selects everything until the board announces `text`: an import lands
    /// a few frames after the file is read.
    async fn select_all_until(page: &Page, text: &str) {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            page.command("Select all").await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let heard = page.announced().await;
            if heard == text {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the board said {heard:?}, not {text:?}"
            );
        }
    }

    /// `TOILE-044`: an arrow one peer draws between two shapes while another
    /// deletes one of them is announced on both, once each has both edits.
    /// The two peers are two users in two browsers. The second one's Sync
    /// worker is paused while it draws, so the two edits cross.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn an_arrow_a_peer_strands_is_announced_on_both() {
        let first = Page::new_document("toile").await;
        draw_rect(&first, (300.0, 200.0), (360.0, 260.0)).await;
        draw_rect(&first, (500.0, 200.0), (560.0, 260.0)).await;
        deselect(&first).await;
        let path = first
            .browser
            .script("return location.pathname;", json!([]))
            .await;
        let path = path.as_str().expect("a document route").to_owned();
        let name = format!("belt-{}", uuid::Uuid::new_v4());
        let second = Page::home(first.relay()).await;
        second.sign_in(&name).await;
        first.share_with(&name).await;

        second
            .browser
            .goto(&format!("{}{path}", second.origin))
            .await;
        second.wait_canvas().await;
        select_all_until(&second, "2 elements selected").await;
        deselect(&second).await;
        let workers = second.workers().await;
        let (sync, _) = workers
            .iter()
            .find(|(_, title)| title.contains("ambre-sync"))
            .unwrap_or_else(|| panic!("no Sync worker among {workers:?}"));
        let paused = second.pause_worker(sync).await;

        let (ox, oy) = second.canvas_origin().await;
        second.focus_canvas().await;
        second.browser.perform(json!([key("c")])).await;
        second
            .browser
            .wait_for(
                "the connector tool",
                "return document.getElementById('ambre-canvas').style.cursor === 'crosshair';",
                STEP,
            )
            .await;
        second
            .browser
            .perform(json!([drag(
                (ox + 330.0, oy + 230.0),
                (ox + 530.0, oy + 230.0),
                8
            )]))
            .await;

        click(&first, (530.0, 230.0)).await;
        first.wait_announced("Rectangle").await;
        first.browser.perform(json!([key("\u{E017}")])).await;
        first.wait_announced("Nothing selected").await;

        second.resume_worker(paused).await;
        // A toast stays a few seconds: both are watched at once.
        tokio::join!(
            second.wait_toast("An arrow lost an end"),
            first.wait_toast("An arrow lost an end"),
        );
    }

    /// `KINETIC-011`: a marquee the browser cancels mid-drag ends as a release
    /// where the pointer last was, so it selects what it covered.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_cancelled_mouse_drag_ends_where_the_pointer_was() {
        let page = Page::new_document("toile").await;
        draw_rect(&page, (300.0, 200.0), (360.0, 260.0)).await;
        draw_rect(&page, (400.0, 200.0), (460.0, 260.0)).await;
        deselect(&page).await;
        let (ox, oy) = page.canvas_origin().await;

        let mut moves = vec![
            json!({ "type": "pointerMove", "x": (ox + 250.0) as i64, "y": (oy + 150.0) as i64 }),
            json!({ "type": "pointerDown", "button": 0 }),
        ];
        for i in 1..=8 {
            let t = f64::from(i) / 8.0;
            moves.push(json!({
                "type": "pointerMove", "duration": 16,
                "x": (ox + 250.0 + 250.0 * t) as i64, "y": (oy + 150.0 + 160.0 * t) as i64,
            }));
        }
        page.browser
            .perform(json!([{ "type": "pointer", "id": "mouse",
                "parameters": { "pointerType": "mouse" }, "actions": moves }]))
            .await;
        // The button is still down: the browser takes the gesture over.
        page.browser
            .script(
                "document.querySelector('canvas').dispatchEvent(new PointerEvent('pointercancel', \
                   { bubbles: true, pointerId: 1, pointerType: 'mouse', isPrimary: true }));",
                json!([]),
            )
            .await;
        page.wait_announced("2 elements selected").await;
    }
}
