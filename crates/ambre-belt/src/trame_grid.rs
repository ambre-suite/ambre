//! A Trame spreadsheet edited end to end in the browser: typing, the keyboard,
//! the history, the clipboard, and two peers in two browsers.
//!
//! The grid draws on a canvas, so a test reads what the page exposes to the
//! user. The live region says the active cell and its raw text (`CHROME-023`).
//! The name box of the formula bar names the active cell. The status bar
//! totals carry a computed value. No test reads an internal state.

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::page::{Page, STEP};
    use crate::toile::tests::{invite_second, open_board_of};

    const ENTER: &str = "\u{E007}";
    const ESCAPE: &str = "\u{E00C}";
    const TAB: &str = "\u{E004}";
    const LEFT: &str = "\u{E012}";
    const UP: &str = "\u{E013}";
    const RIGHT: &str = "\u{E014}";
    const DOWN: &str = "\u{E015}";
    const SHIFT: &str = "\u{E008}";
    const CONTROL: &str = "\u{E009}";
    const HOME: &str = "\u{E011}";

    /// Puts the focus on the text pipe, where the grid hears the keyboard. A
    /// click in the grid does the same.
    async fn focus_grid(page: &Page) {
        page.browser
            .script("document.querySelector('.ambre-input').focus();", json!([]))
            .await;
    }

    /// Presses and releases each key of `keys` in turn.
    async fn press(page: &Page, keys: &[&str]) {
        let actions: Vec<Value> = keys
            .iter()
            .flat_map(|k| {
                [
                    json!({ "type": "keyDown", "value": k }),
                    json!({ "type": "keyUp", "value": k }),
                ]
            })
            .collect();
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": actions }]))
            .await;
    }

    /// Types `text`, one character after the other.
    async fn type_text(page: &Page, text: &str) {
        let keys: Vec<String> = text.chars().map(String::from).collect();
        press(page, &keys.iter().map(String::as_str).collect::<Vec<_>>()).await;
    }

    /// Presses `key` with `modifier` held.
    async fn chord(page: &Page, modifier: &str, key: &str) {
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": modifier },
                { "type": "keyDown", "value": key },
                { "type": "keyUp", "value": key },
                { "type": "keyUp", "value": modifier },
            ]}]))
            .await;
    }

    /// Selects the cell `col` columns right of `A1` and `row` rows below it,
    /// with the keyboard alone: `Ctrl+Home`, then the arrows. The first step
    /// down makes `Ctrl+Home` a move even from `A1`, since a selection that
    /// stays put is not announced again.
    async fn go_to(page: &Page, col: usize, row: usize) {
        focus_grid(page).await;
        press(page, &[DOWN]).await;
        chord(page, CONTROL, HOME).await;
        let keys: Vec<&str> = std::iter::repeat_n(RIGHT, col)
            .chain(std::iter::repeat_n(DOWN, row))
            .collect();
        if !keys.is_empty() {
            press(page, &keys).await;
        }
    }

    /// Waits until the cell at (`col`, `row`) reads `heard` in the live
    /// region, selecting it again until it does. A value from the Logic thread
    /// or from a peer lands a few frames after its gesture. The live region
    /// speaks only on a move.
    async fn wait_cell(page: &Page, col: usize, row: usize, heard: &str) {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            go_to(page, col, row).await;
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let said = page.announced().await;
            if said == heard {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the grid said {said:?}, not {heard:?}"
            );
        }
    }

    /// Waits until the status bar totals of the two cells right of (`col`,
    /// `row`) hold `total`, selecting them again until they do. The totals
    /// are the one place the page shows a computed value as text.
    async fn wait_totals(page: &Page, col: usize, row: usize, total: &str) {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            go_to(page, col, row).await;
            chord(page, SHIFT, RIGHT).await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let shown = page
                .browser
                .script(
                    "const s = document.querySelector('.statusbar-stats'); \
                     return s ? s.textContent : '';",
                    json!([]),
                )
                .await;
            let shown = shown.as_str().unwrap_or_default().to_owned();
            if shown.contains(total) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the status bar showed {shown:?}, not {total:?}"
            );
        }
    }

    /// Waits until the clipboard holds the ownership token a grid copy
    /// publishes once the Logic thread answered it. A paste sent before that
    /// races the copy, which no user waiting for the copy meets. Chrome holds
    /// the token under its `web ` type. Firefox refuses that type, so the token
    /// rides in the HTML table of the copy (`UI-THREAD-061`).
    async fn wait_copied(page: &Page) {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let held = page
                .browser
                .script_async(
                    "const done = arguments[0]; \
                     navigator.clipboard.read() \
                       .then(async items => { \
                         const types = items.flatMap(i => i.types); \
                         if (types.includes('web application/x-ambre-token')) return 'token'; \
                         const item = items.find(i => i.types.includes('text/html')); \
                         if (!item) return types; \
                         const html = await (await item.getType('text/html')).text(); \
                         return html.includes('<!--ambre-token:') ? 'token' : types; \
                       }) \
                       .then(done) \
                       .catch(e => done(e.name));",
                    json!([]),
                )
                .await;
            if held == json!("token") {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the clipboard holds {held}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Waits until the name box of the formula bar reads `cell`. The box
    /// follows the active cell on the next frame.
    async fn wait_name_box(page: &Page, cell: &str) {
        page.browser
            .wait_for(
                &format!("the name box on {cell}"),
                &format!(
                    "const n = document.querySelector('.name-box'); \
                     return !!n && n.textContent === {};",
                    Value::String(cell.to_owned())
                ),
                STEP,
            )
            .await;
    }

    /// `TRAME-UI-160`, `TRAME-UI-161`, `TRAME-UI-171`: a value typed and
    /// committed by `Enter` lands in its cell and moves the selection down. A
    /// value committed by `Tab` moves it right.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_typed_value_commits_with_enter_and_with_tab() {
        let page = Page::new_document("trame").await;
        go_to(&page, 0, 0).await;
        page.wait_announced("A1, empty").await;
        type_text(&page, "42").await;
        press(&page, &[ENTER]).await;
        page.wait_announced("A2, empty").await;
        type_text(&page, "7").await;
        press(&page, &[TAB]).await;
        page.wait_announced("B2, empty").await;
        wait_cell(&page, 0, 0, "A1, 42").await;
        wait_cell(&page, 0, 1, "A2, 7").await;
    }

    /// `TRAME-UI-165`: `Escape` closes the session and leaves the cell as it
    /// was stored, in the grid and in the formula bar.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn escape_leaves_the_cell_as_it_was_stored() {
        let page = Page::new_document("trame").await;
        go_to(&page, 0, 0).await;
        type_text(&page, "42").await;
        press(&page, &[ENTER]).await;
        wait_cell(&page, 0, 0, "A1, 42").await;
        type_text(&page, "7").await;
        press(&page, &[ESCAPE]).await;
        page.wait_announced("A1, 42").await;
        page.browser
            .wait_for(
                "the formula bar back on the stored value",
                "return document.getElementById('ambre-formula-input').value === '42';",
                STEP,
            )
            .await;
        // Moving away and back reads the cell afresh: the cancelled 7 never landed.
        press(&page, &[DOWN]).await;
        page.wait_announced("A2, empty").await;
        wait_cell(&page, 0, 0, "A1, 42").await;
    }

    /// `TRAME-UI-188`, `TRAME-UI-190`, `CHROME-027`: with a value in `B2`, `Shift+F10` opens
    /// the grid menu at that cell, with the focus on its first entry. Three
    /// presses of the down arrow reach Insert row above, and Enter runs it.
    /// The value moves down to `B3`, and the focus returns to the grid. An
    /// arrow that reached the grid would insert the row under `B2` instead.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_menu_key_opens_the_grid_menu_and_runs_an_entry() {
        const F10: &str = "\u{E03A}";
        let page = Page::new_document("trame").await;
        go_to(&page, 1, 1).await;
        type_text(&page, "42").await;
        press(&page, &[ENTER]).await;
        wait_cell(&page, 1, 1, "B2, 42").await;
        // The pointer rests far from where the menu opens: a browser that
        // fires a hover on the entry under a resting pointer moves no focus.
        page.browser
            .perform(json!([{ "type": "pointer", "id": "mouse",
                "parameters": { "pointerType": "mouse" }, "actions": [
                    { "type": "pointerMove", "x": 2, "y": 2 },
                ] }]))
            .await;

        // The menu flips upward where the window is too short for it. The
        // test reads its left edge alone, and the host tier holds the point.
        chord(&page, SHIFT, F10).await;
        page.browser
            .wait_for(
                "the focus on the first entry of the menu, over the grid",
                "const a = document.activeElement; \
                 const m = document.querySelector('.ambre-ctx-menu'); \
                 const c = document.getElementById('ambre-canvas').getBoundingClientRect(); \
                 if (!m || !a || !a.closest('.ambre-ctx-menu') || !a.textContent.includes('Cut')) \
                   return false; \
                 const r = m.getBoundingClientRect(); \
                 return r.left > c.left && r.left < c.right;",
                STEP,
            )
            .await;
        press(&page, &[DOWN, DOWN, DOWN]).await;
        page.browser
            .wait_for(
                "the focus on Insert row above",
                "const a = document.activeElement; \
                 return !!a && !!a.closest('.ambre-ctx-menu') \
                   && a.textContent.includes('Insert row above');",
                STEP,
            )
            .await;
        press(&page, &[ENTER]).await;
        page.browser
            .wait_for(
                "the menu closed, and the focus back on the grid",
                "const a = document.activeElement; \
                 return !document.querySelector('.ambre-ctx-menu') && !!a \
                   && a !== document.body && !a.closest('.ambre-ctx-menu');",
                STEP,
            )
            .await;
        wait_cell(&page, 1, 2, "B3, 42").await;
    }

    /// `TRAME-UI-182`, `TRAME-UI-091`: with no session open, each arrow moves
    /// the active cell one cell its way, and `Tab` one column right. The live
    /// region and the name box both follow it.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn the_arrows_and_tab_move_the_active_cell() {
        let page = Page::new_document("trame").await;
        go_to(&page, 0, 0).await;
        page.wait_announced("A1, empty").await;
        for (key, cell) in [
            (RIGHT, "B1"),
            (DOWN, "B2"),
            (LEFT, "A2"),
            (UP, "A1"),
            (TAB, "B1"),
            (TAB, "C1"),
        ] {
            press(&page, &[key]).await;
            page.wait_announced(&format!("{cell}, empty")).await;
            wait_name_box(&page, cell).await;
        }
    }

    /// `TRAME-UI-052`, `TRAME-151`: each commit is one undo item, so `Ctrl+Z`
    /// takes back the last edit alone, and `Ctrl+Y` brings it back.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn undo_takes_back_one_edit_and_redo_brings_it_back() {
        let page = Page::new_document("trame").await;
        go_to(&page, 0, 0).await;
        type_text(&page, "1").await;
        press(&page, &[ENTER]).await;
        wait_cell(&page, 0, 0, "A1, 1").await;
        type_text(&page, "2").await;
        press(&page, &[ENTER]).await;
        wait_cell(&page, 0, 0, "A1, 2").await;

        chord(&page, CONTROL, "z").await;
        wait_cell(&page, 0, 0, "A1, 1").await;
        chord(&page, CONTROL, "y").await;
        wait_cell(&page, 0, 0, "A1, 2").await;
    }

    /// `TRAME-UI-061`: a range copied with `Ctrl+C` and pasted with `Ctrl+V`
    /// lands in the same sheet, its formula shifted by the paste offset. The
    /// pasted formula computes from the pasted value.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_copied_range_pastes_with_its_formula_shifted() {
        let page = Page::new_document("trame").await;
        page.browser.permit("clipboard-read").await;
        go_to(&page, 0, 0).await;
        type_text(&page, "21").await;
        press(&page, &[TAB]).await;
        type_text(&page, "=A1*2").await;
        press(&page, &[ENTER]).await;
        wait_cell(&page, 1, 0, "B1, =A1*2").await;

        go_to(&page, 0, 0).await;
        chord(&page, SHIFT, RIGHT).await;
        chord(&page, CONTROL, "c").await;
        wait_copied(&page).await;
        go_to(&page, 0, 2).await;
        page.wait_announced("A3, empty").await;
        chord(&page, CONTROL, "v").await;
        wait_cell(&page, 0, 2, "A3, 21").await;
        wait_cell(&page, 1, 2, "B3, =A3*2").await;
        wait_totals(&page, 0, 2, "Sum: 63").await;
    }

    /// `TRAME-221`: two users in two browsers. A formula one types computes
    /// from a value the other types. Then both edit two cells at once, the
    /// second one's Sync worker paused so the two edits cross. Both screens
    /// end on the same text and the same computed value.
    ///
    /// The first commit writes row 3, so rows 1 to 3 exist before the two
    /// edits cross. Two first writes in one new row split it (`TRAME-261`).
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "pauses a worker through the DevTools Protocol, which Firefox does not speak"
    )]
    async fn two_peers_typing_at_once_converge_on_both_screens() {
        let alice = Page::new_document("trame").await;
        // The first commit creates the document on the relay, so a second user
        // can then open it.
        go_to(&alice, 2, 2).await;
        type_text(&alice, "0").await;
        press(&alice, &[ENTER]).await;
        wait_cell(&alice, 2, 2, "C3, 0").await;
        let bob = invite_second(&alice).await;
        open_board_of(&bob, &alice).await;
        wait_cell(&bob, 2, 2, "C3, 0").await;

        // Bob's formula reads a cell Alice has not typed yet.
        go_to(&bob, 1, 0).await;
        type_text(&bob, "=A1*2").await;
        press(&bob, &[ENTER]).await;
        wait_cell(&alice, 1, 0, "B1, =A1*2").await;
        go_to(&alice, 0, 0).await;
        type_text(&alice, "21").await;
        press(&alice, &[ENTER]).await;
        wait_cell(&bob, 0, 0, "A1, 21").await;
        // B1 reads 42 on both screens.
        wait_totals(&bob, 1, 0, "Sum: 42").await;
        wait_totals(&alice, 1, 0, "Sum: 42").await;

        let sync = bob.worker("ambre-sync").await;
        let paused = bob.pause_worker(&sync).await;
        go_to(&bob, 0, 1).await;
        type_text(&bob, "5").await;
        press(&bob, &[ENTER]).await;
        go_to(&alice, 1, 1).await;
        type_text(&alice, "=A2+A1").await;
        press(&alice, &[ENTER]).await;
        wait_cell(&bob, 0, 1, "A2, 5").await;
        wait_cell(&alice, 1, 1, "B2, =A2+A1").await;
        bob.resume_worker(paused).await;

        for page in [&alice, &bob] {
            wait_cell(page, 0, 1, "A2, 5").await;
            wait_cell(page, 1, 1, "B2, =A2+A1").await;
            // A2 is 5 and B2 is 26.
            wait_totals(page, 0, 1, "Sum: 31").await;
        }

        // Two first writes in one new row: row 4 exists on neither screen yet.
        // Each peer adds it under the uid the line before derives, so both
        // values land on row 4 and none moves down a row (`TRAME-261`).
        let paused = bob.pause_worker(&sync).await;
        go_to(&bob, 0, 3).await;
        type_text(&bob, "7").await;
        press(&bob, &[ENTER]).await;
        go_to(&alice, 1, 3).await;
        type_text(&alice, "8").await;
        press(&alice, &[ENTER]).await;
        wait_cell(&bob, 0, 3, "A4, 7").await;
        wait_cell(&alice, 1, 3, "B4, 8").await;
        bob.resume_worker(paused).await;

        for page in [&alice, &bob] {
            wait_cell(page, 0, 3, "A4, 7").await;
            wait_cell(page, 1, 3, "B4, 8").await;
        }
    }
}
