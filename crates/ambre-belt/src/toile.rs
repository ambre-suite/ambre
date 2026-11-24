//! A Toile board, driven in the browser.

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use crate::page::{Page, STEP, drag, drag_still, key, palette_row_js};

    /// Draws a rectangle with the shape tool from `a` to `b`, in canvas pixels.
    /// Then selects it with the select tool and waits until the board
    /// announces it. A drawn shape is not selected by its creation.
    ///
    /// The release clears the drag's preview, and the shape shows once the
    /// Logic thread answers with it. A click before that hits an empty board,
    /// as no user could click it. So the click waits for the screen to change
    /// on the top edge: geckodriver sends it within the frame of the release.
    /// The default fill is the white of the board, so only the stroke shows.
    pub(crate) async fn draw_rect(page: &Page, a: (f64, f64), b: (f64, f64)) {
        let (ox, oy) = page.canvas_origin().await;
        let centre = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let edge = top_edge(a, b);
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
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let seen = edge_colors(page, edge).await;
            if shows_stroke(&seen) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the board still shows {seen:?} where the rectangle was drawn"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        page.browser.perform(json!([key("v")])).await;
        click(page, centre).await;
        page.wait_announced("Rectangle").await;
    }

    /// The middle of the top edge of the rectangle drawn from `a` to `b`, in
    /// canvas pixels.
    pub(crate) fn top_edge(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
        ((a.0 + b.0) / 2.0, a.1.min(b.1))
    }

    /// The colours of a vertical strip of five pixels centred on `at`, in
    /// canvas pixels. The one-pixel stroke of a shape falls in it, wherever
    /// the rasteriser rounds its half pixel.
    pub(crate) async fn edge_colors(page: &Page, at: (f64, f64)) -> Vec<[u8; 3]> {
        let mut seen = Vec::with_capacity(5);
        for dy in -2..=2 {
            seen.push(page.color_at((at.0, at.1 + f64::from(dy))).await);
        }
        seen
    }

    /// Whether a strip shows the dark grey stroke of a shape (`#333333`),
    /// even spread over two pixels. A grid line (about `#d2d5da`) is too
    /// light, and the outline of a selection (about `#b06f0a`) is not grey.
    /// A strip read before the draw is no reference: the grid moves once the
    /// board holds an element.
    pub(crate) fn shows_stroke(strip: &[[u8; 3]]) -> bool {
        strip.iter().any(|p| {
            let (lo, hi) = (p.iter().min(), p.iter().max());
            lo.zip(hi)
                .is_some_and(|(lo, hi)| *hi < STROKE_CUT && hi - lo < GREY_SPREAD)
        })
    }

    /// The channel value under which a pixel belongs to a stroke: `#333333`
    /// on white, at half coverage, reads about 153.
    const STROKE_CUT: u8 = 170;

    /// The widest gap between the channels of a grey pixel.
    const GREY_SPREAD: u8 = 32;

    /// Clicks the canvas at `at`, in canvas pixels.
    async fn click(page: &Page, at: (f64, f64)) {
        let (ox, oy) = page.canvas_origin().await;
        let p = (ox + at.0, oy + at.1);
        page.browser.perform(json!([drag(p, p, 0)])).await;
    }

    /// Selects nothing, with the select tool armed: `Escape` clears the
    /// selection and reverts a creation tool to Select.
    pub(crate) async fn deselect(page: &Page) {
        page.browser.perform(json!([key("\u{E00C}")])).await;
        page.wait_announced("Nothing selected").await;
    }

    /// `TOILE-UI-394`: the tool key and the drag are dispatched in one task, so
    /// no frame runs between them. The drag still draws the rectangle. Driven
    /// input cannot show this: the driver lets a frame pass between two ticks.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_tool_key_arms_the_press_that_follows_it() {
        let page = Page::new_document("toile").await;
        page.focus_canvas().await;
        page.browser
            .script_async(
                "const done = arguments[0]; const c = document.querySelector('canvas'); \
                 const b = c.getBoundingClientRect(); \
                 const at = (x, y) => ({ clientX: b.left + x, clientY: b.top + y, bubbles: true, \
                   pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0, buttons: 1 }); \
                 document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'r', code: 'KeyR', bubbles: true })); \
                 document.activeElement.dispatchEvent(new KeyboardEvent('keyup', { key: 'r', code: 'KeyR', bubbles: true })); \
                 c.dispatchEvent(new PointerEvent('pointerdown', at(300, 200))); \
                 c.dispatchEvent(new PointerEvent('pointermove', at(360, 250))); \
                 c.dispatchEvent(new PointerEvent('pointermove', at(420, 300))); \
                 c.dispatchEvent(new PointerEvent('pointerup', { ...at(420, 300), buttons: 0 })); \
                 done(true);",
                json!([]),
            )
            .await;
        page.browser.perform(json!([key("v")])).await;
        click(&page, (360.0, 250.0)).await;
        page.wait_announced("Rectangle").await;
    }

    /// `TOILE-UI-395`: `Zoom in` picked in the palette, then a click, with no
    /// frame between them. The click lands where the zoom carried the
    /// rectangle, so it selects it. A click read before the zoom misses it.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_palette_command_runs_before_the_click_that_follows_it() {
        let page = Page::new_document("toile").await;
        let size = page
            .browser
            .script(
                "const r = document.querySelector('canvas').getBoundingClientRect(); \
                 return [r.width / 2, r.height / 2];",
                json!([]),
            )
            .await;
        let (cx, cy) = (
            size[0].as_f64().unwrap_or(0.0),
            size[1].as_f64().unwrap_or(0.0),
        );
        // A rectangle 300 pixels wide around the zoom's centre. A zoom of 1.2
        // carries its point at +140 to +168, past its edge at +150.
        draw_rect(&page, (cx - 150.0, cy - 60.0), (cx + 150.0, cy + 60.0)).await;
        deselect(&page).await;
        page.palette_row("Zoom in").await;
        // The modal holds the frame loop until its `close` event, so the
        // click sent from that event runs before any frame.
        page.browser
            .script_async(
                &format!(
                    "const [x, y, done] = arguments; \
                     const c = document.querySelector('canvas'); \
                     const b = c.getBoundingClientRect(); \
                     const at = (buttons) => ({{ clientX: b.left + x, clientY: b.top + y, bubbles: true, \
                       pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0, buttons }}); \
                     document.querySelector('.ambre-palette-modal').addEventListener('close', () => {{ \
                       c.dispatchEvent(new PointerEvent('pointerdown', at(1))); \
                       c.dispatchEvent(new PointerEvent('pointerup', at(0))); \
                       done(true); \
                     }}, {{ once: true }}); \
                     {}.click();",
                    palette_row_js("Zoom in")
                ),
                json!([cx + 168.0, cy]),
            )
            .await;
        page.wait_announced("Rectangle").await;
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
    pub(crate) async fn select_all_until(page: &Page, text: &str) {
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

    /// Signs a second user in, in a second browser, and shares the board of
    /// `first` with them. That browser has a profile of its own, so it holds
    /// nothing of the board.
    pub(crate) async fn invite_second(first: &Page) -> Page {
        let name = format!("belt-{}", uuid::Uuid::new_v4());
        let second = Page::home(first.relay()).await;
        second.sign_in(&name).await;
        first.share_with(&name).await;
        second
    }

    /// Opens the board of `first` on `second`, from the invite link `first`
    /// copied.
    pub(crate) async fn open_board_of(second: &Page, first: &Page) {
        let path = first.invite_link().await;
        second
            .browser
            .goto(&format!("{}{path}", second.origin))
            .await;
        second.wait_canvas().await;
    }

    /// A square of one colour, as a PNG file a user brings onto a board.
    fn square_png(rgb: [u8; 3]) -> Vec<u8> {
        let square = image::RgbImage::from_pixel(120, 120, image::Rgb(rgb));
        let mut png = std::io::Cursor::new(Vec::new());
        square
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("a PNG");
        png.into_inner()
    }

    /// `SYNC-140`: an image one user drops reaches a second user's screen. The
    /// first board shows the image once its upload is confirmed. The second
    /// browser fetches the blob and opens it. The seal authenticates every
    /// byte, so a byte either leg altered would leave the image unopened.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn an_image_crosses_the_object_store_whole() {
        const MAGENTA: [u8; 3] = [255, 0, 255];
        let first = Page::new_document("toile").await;
        // The board is empty: the upload also waits for its creation (`SYNC-145`).
        first
            .drop_file((300.0, 200.0), "one.png", "image/png", &square_png(MAGENTA))
            .await;
        select_all_until(&first, "Image").await;

        let second = invite_second(&first).await;
        open_board_of(&second, &first).await;
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let seen = second.color_at((300.0, 200.0)).await;
            if seen.iter().zip(MAGENTA).all(|(s, m)| s.abs_diff(m) <= 8) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the second screen shows {seen:?} where the image is"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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
    #[cfg_attr(
        feature = "firefox",
        ignore = "pauses a worker through the DevTools Protocol, which Firefox does not speak"
    )]
    async fn an_arrow_a_peer_strands_is_announced_on_both() {
        let first = Page::new_document("toile").await;
        draw_rect(&first, (300.0, 200.0), (360.0, 260.0)).await;
        draw_rect(&first, (500.0, 200.0), (560.0, 260.0)).await;
        deselect(&first).await;
        let second = invite_second(&first).await;
        open_board_of(&second, &first).await;
        select_all_until(&second, "2 elements selected").await;
        deselect(&second).await;
        let sync = second.worker("ambre-sync").await;
        let paused = second.pause_worker(&sync).await;

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

    /// `TOILE-UI-422`, `CHROME-027`: with a rectangle selected, `Shift+F10`
    /// opens the board's menu with the focus on its first entry. Four
    /// presses of the down arrow reach Delete, and Enter runs it: the
    /// rectangle leaves the screen, and the focus returns to the board.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_menu_key_opens_the_board_menu_and_runs_an_entry() {
        const SHIFT: &str = "\u{E008}";
        const F10: &str = "\u{E03A}";
        const DOWN: &str = "\u{E015}";
        const ENTER: &str = "\u{E007}";
        let page = Page::new_document("toile").await;
        let (a, b) = ((300.0, 200.0), (420.0, 300.0));
        let edge = top_edge(a, b);
        draw_rect(&page, a, b).await;

        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": SHIFT },
                { "type": "keyDown", "value": F10 },
                { "type": "keyUp", "value": F10 },
                { "type": "keyUp", "value": SHIFT },
            ]}]))
            .await;
        page.browser
            .wait_for(
                "the focus on the first entry of the menu",
                "const a = document.activeElement; \
                 return !!a && !!a.closest('.ambre-ctx-menu') && a.textContent.includes('Cut');",
                STEP,
            )
            .await;
        for _ in 0..4 {
            page.browser.perform(json!([key(DOWN)])).await;
        }
        page.browser
            .wait_for(
                "the focus on Delete",
                "const a = document.activeElement; \
                 return !!a && !!a.closest('.ambre-ctx-menu') && a.textContent.includes('Delete');",
                STEP,
            )
            .await;
        page.browser.perform(json!([key(ENTER)])).await;

        page.browser
            .wait_for(
                "the menu closed, and the focus back on the board",
                "const a = document.activeElement; \
                 return !document.querySelector('.ambre-ctx-menu') && !!a \
                   && a !== document.body && !a.closest('.ambre-ctx-menu');",
                STEP,
            )
            .await;
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let seen = edge_colors(&page, edge).await;
            if !shows_stroke(&seen) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the board still shows {seen:?} where the deleted rectangle was"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}
