//! Murmure, driven in the browser: two users in one channel, through the
//! relay.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use crate::page::{BOOT, Page, STEP, drag};
    use crate::relay::Relay;

    /// The WebDriver codes of the keys these tests press.
    const ENTER: &str = "\u{E007}";
    const UP: &str = "\u{E013}";
    const SHIFT: &str = "\u{E008}";
    const F10: &str = "\u{E03A}";

    /// A fresh dev user name. The relay gives it the address
    /// `<name>@dev.local`.
    fn user() -> String {
        format!("belt-{}", uuid::Uuid::new_v4().simple())
    }

    /// Serves the build in front of `relay`, signs `name` in and opens
    /// Murmure. Waits for the sidebar's control that creates a space.
    async fn murmure(relay: Arc<Relay>, name: &str) -> Page {
        let page = Page::home(relay).await;
        page.sign_in(name).await;
        page.browser
            .goto(&format!("{}/murmure/{}", page.origin, uuid::Uuid::now_v7()))
            .await;
        page.browser
            .wait_for(
                "the Murmure sidebar",
                "return !!document.querySelector('.murmure-new-space-toggle');",
                BOOT,
            )
            .await;
        page
    }

    /// Fills the prompt control whose form has the class `class`, found
    /// under `scope` with `text`, and submits it.
    async fn prompt(page: &Page, scope: &str, class: &str, text: &str) {
        let (scope, text) = (
            Value::String(scope.to_owned()),
            Value::String(text.to_owned()),
        );
        page.browser
            .wait_for(
                &format!("the control {class} under {scope}"),
                &format!(
                    "const s = document.querySelector({scope}); if (!s) return false; \
                     const f = s.querySelector('.{class}-field'); \
                     if (!f) {{ const t = s.querySelector('.{class}-toggle'); \
                       if (t) t.click(); return false; }} \
                     f.value = {text}; \
                     f.dispatchEvent(new Event('input', {{ bubbles: true }})); \
                     s.querySelector('form.{class}').requestSubmit(); \
                     return true;"
                ),
                STEP,
            )
            .await;
    }

    /// The script that answers the sidebar entry named `name`, or `null`.
    fn entry_js(name: &str) -> String {
        format!(
            "[...document.querySelectorAll('.murmure-entry')] \
               .find(e => e.querySelector('.murmure-entry-name')?.textContent === {})",
            Value::String(name.to_owned())
        )
    }

    /// Creates the space `space` with the public channel `channel`, which
    /// opens, then invites `invitee` into the space and confirms their key.
    async fn space_with_channel(page: &Page, space: &str, channel: &str, invitee: &str) {
        prompt(page, ".murmure-spaces", "murmure-new-space", space).await;
        // A channel asked for at once waits for its space to reach the relay
        // (`SYNC-189`), so one request opens it.
        prompt(page, ".murmure-space", "murmure-new-channel", channel).await;
        open(page, channel).await;
        prompt(
            page,
            ".murmure-space",
            "murmure-space-invite",
            &format!("{invitee}@dev.local"),
        )
        .await;
        confirm_key(page).await;
        page.wait_toast(&format!("Invited {invitee}")).await;
    }

    /// Clicks the sidebar entry `name` until the content shows its channel.
    /// A channel to join is joined by the first click and opened by the next.
    async fn open(page: &Page, name: &str) {
        let shown = Value::String(name.to_owned());
        page.browser
            .wait_for(
                &format!("the channel {shown} shown"),
                &format!(
                    "const n = document.querySelector('.murmure-channel-name'); \
                     if (n && n.textContent === {shown}) return true; \
                     const e = {}; if (e) e.click(); return false;",
                    entry_js(name)
                ),
                STEP,
            )
            .await;
        page.wait_canvas().await;
    }

    /// Presses and releases each of `keys` in turn: a character or a key code.
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

    /// A point of the canvas `selector` where a press reaches the canvas
    /// itself, the first from the left on one line. A toast may lie over the
    /// rest. The line is near the top, in the list, or `from_bottom` in the
    /// composer's field. That is above the row of a thread's composer that
    /// also sends the reply to the channel.
    async fn reachable(page: &Page, selector: &str, from_bottom: bool) -> (f64, f64) {
        let at = page
            .browser
            .script(
                "const c = document.querySelector(arguments[0]); \
                 const r = c.getBoundingClientRect(); \
                 const y = arguments[1] ? r.bottom - 45 : r.top + 20; \
                 for (let x = r.left + 40; x < r.right; x += 20) \
                   if (document.elementFromPoint(x, y) === c) return [x, y]; \
                 return null;",
                json!([selector, from_bottom]),
            )
            .await;
        let coord = |i: usize| {
            at[i]
                .as_f64()
                .unwrap_or_else(|| panic!("{selector} is covered"))
        };
        (coord(0), coord(1))
    }

    /// Presses the composer at the foot of the canvas `selector`, types
    /// `text` and sends it with Enter. A composer not yet drawn leaves the
    /// press to its list, so the press is made again until the input pipe
    /// holds the keyboard. Nothing on the page says the message left, so the
    /// peer's read is the check.
    async fn send(page: &Page, selector: &str, text: &str) {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let at = reachable(page, selector, true).await;
            page.browser.perform(json!([drag(at, at, 0)])).await;
            let piped = page
                .browser
                .script(
                    "return document.activeElement?.classList.contains('ambre-input');",
                    json!([]),
                )
                .await;
            if piped == json!(true) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the composer of {selector} never took the keys"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let keys: Vec<String> = text.chars().map(String::from).collect();
        press(page, &keys.iter().map(String::as_str).collect::<Vec<_>>()).await;
        press(page, &[ENTER]).await;
    }

    /// Presses the list of the canvas `selector`, then `Up` until the live
    /// region reads a message that holds `text`, and answers what it read.
    /// The press gives the keys to the list, and `Up` focuses its latest
    /// message, which a screen reader hears with its author.
    async fn read_latest(page: &Page, selector: &str, text: &str) -> String {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let at = reachable(page, selector, false).await;
            page.browser.perform(json!([drag(at, at, 0)])).await;
            press(page, &[UP]).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let heard = page.announced().await;
            if heard.contains(text) {
                return heard;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the latest message read {heard:?}, not {text:?}"
            );
        }
    }

    /// Waits until the live region reads a message that holds `text`, and
    /// answers what it read.
    async fn wait_heard(page: &Page, text: &str) -> String {
        let text = Value::String(text.to_owned());
        page.browser
            .wait_for(
                &format!("a message {text} heard"),
                &format!(
                    "const l = document.querySelector('.ambre-a11y-live'); \
                     return l && l.textContent.includes({text}) ? l.textContent : null;"
                ),
                STEP,
            )
            .await
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// The canvas of the channel's message list.
    const CHANNEL: &str = "canvas.ambre-canvas";

    /// `MURMURE-586`: two users in one channel, through the relay. A message
    /// one sends reaches the list the other holds open, said with its author.
    /// Bob joined after Ada opened the channel, so his answer reaches her only
    /// once her client resolves his signature (`SRV-177`).
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_message_one_member_sends_reaches_the_other() {
        let relay = Arc::new(Relay::start().await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
        // The first list a channel shows is not news (`MURMURE-208`), so a
        // message waits in it before Bob opens it.
        send(&ada_page, CHANNEL, "First words").await;
        open(&bob_page, "general").await;
        read_latest(&bob_page, CHANNEL, "First words").await;

        send(&ada_page, CHANNEL, "Hello from the belt").await;
        let heard = wait_heard(&bob_page, "Hello from the belt").await;
        assert!(heard.starts_with(&ada), "{heard}");

        send(&bob_page, CHANNEL, "Hello back").await;
        let heard = wait_heard(&ada_page, "Hello back").await;
        assert!(heard.starts_with(&bob), "{heard}");
    }

    /// Goes back from the shown channel to the channel list, as the control
    /// in the channel's header does. No channel shows then.
    async fn leave_pane(page: &Page) {
        page.browser
            .wait_for(
                "the channel list",
                "const b = document.querySelector('.murmure-back'); \
                 if (!document.querySelector('.murmure-channel-name')?.textContent) return true; \
                 if (b) b.click(); return false;",
                STEP,
            )
            .await;
    }

    /// Waits until the sidebar entry `name` counts `count` unread
    /// messages. The relay's mark may show first, until a count replaces it
    /// (`MURMURE-119`, `MURMURE-120`).
    async fn wait_unread(page: &Page, name: &str, count: u32) {
        page.browser
            .wait_for(
                &format!("{count} unread in {name}"),
                &format!(
                    "const e = {}; if (!e) return false; \
                     const c = e.querySelector('.murmure-count[aria-label=\"Unread messages\"]'); \
                     return !!c && c.textContent === '{count}';",
                    entry_js(name)
                ),
                STEP,
            )
            .await;
    }

    /// `MURMURE-587`: a message reaches a channel its member joined and went
    /// back from. The sidebar entry of that channel bears the unread mark,
    /// and opening it shows the message.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_message_to_a_closed_channel_marks_it_unread() {
        let relay = Arc::new(Relay::start().await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
        open(&bob_page, "general").await;
        leave_pane(&bob_page).await;

        send(&ada_page, CHANNEL, "While you were away").await;
        wait_unread(&bob_page, "general", 1).await;
        open(&bob_page, "general").await;
        read_latest(&bob_page, CHANNEL, "While you were away").await;
    }

    /// The canvas of the thread, in the panel beside the channel.
    const THREAD: &str = "#murmure-thread-canvas";

    /// Focuses the latest message of the channel that holds `text`, then
    /// runs the reply action of its menu, which opens its thread in the
    /// panel. The palette offers the action with no message to act on, so
    /// the menu of the focused message is the way (`MURMURE-419`).
    async fn open_thread(page: &Page, text: &str) {
        read_latest(page, CHANNEL, text).await;
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
                "the action Reply in thread",
                "const i = [...document.querySelectorAll('.ambre-ctx-item')] \
                   .find(e => e.textContent.includes('Reply in thread')); \
                 if (!i) return false; i.click(); return true;",
                STEP,
            )
            .await;
        page.browser
            .wait_for(
                "the thread canvas",
                "const c = document.querySelector('#murmure-thread-canvas'); \
                 return !!c && c.getBoundingClientRect().width > 0;",
                STEP,
            )
            .await;
    }

    /// `MURMURE-588`: a reply one member sends in a thread reaches the other
    /// member's thread on the same root, said with its author. The other
    /// member's client opens the thread once the root names it, so the
    /// thread must be on the relay by then.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_thread_reply_reaches_the_other_member() {
        let relay = Arc::new(Relay::start().await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
        send(&ada_page, CHANNEL, "A root to answer").await;
        open(&bob_page, "general").await;

        open_thread(&bob_page, "A root to answer").await;
        send(&bob_page, THREAD, "An answer in the thread").await;
        read_latest(&bob_page, THREAD, "An answer in the thread").await;

        open_thread(&ada_page, "A root to answer").await;
        let heard = read_latest(&ada_page, THREAD, "An answer in the thread").await;
        assert!(heard.starts_with(&bob), "{heard}");
    }

    /// Opens the member list of the shown channel and the direct message
    /// with the member `name`, confirming the first sight of their key.
    async fn message_member(page: &Page, name: &str) {
        let name = Value::String(name.to_owned());
        page.browser
            .wait_for(
                &format!("the member {name} to message"),
                &format!(
                    "const m = [...document.querySelectorAll('.murmure-member-open')] \
                       .find(b => b.querySelector('.murmure-member-name')?.textContent === {name}); \
                     if (m) {{ m.click(); return true; }} \
                     const t = document.querySelector('.murmure-members-toggle'); \
                     if (t && t.getAttribute('aria-expanded') !== 'true') t.click(); \
                     return false;"
                ),
                STEP,
            )
            .await;
        confirm_key(page).await;
    }

    /// Confirms the first sight of a member's key in the dialog that asks.
    async fn confirm_key(page: &Page) {
        page.browser
            .wait_for(
                "the key confirmation",
                "const b = document.querySelector('.ambre-tofu-confirm'); \
                 if (!b) return false; b.click(); return true;",
                STEP,
            )
            .await;
    }

    /// Waits for the direct message the sidebar lists, and answers the name
    /// of its entry.
    async fn wait_direct(page: &Page) -> String {
        page.browser
            .wait_for(
                "a direct message in the sidebar",
                "const e = document.querySelector('.murmure-section[data-group=direct] .murmure-entry-name'); \
                 return e ? e.textContent : null;",
                STEP,
            )
            .await
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// `MURMURE-589`: a member opens a direct message with another from the
    /// member list and writes in it. It lists in the other's sidebar, and
    /// opening it shows the message, said with its author. The other client
    /// watches the new channel, then opens it within the second. Its open
    /// asks a second replay of `meta` inside the relay's window (`SYNC-188`).
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_direct_message_reaches_the_member_it_names() {
        let relay = Arc::new(Relay::start().await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
        open(&bob_page, "general").await;
        leave_pane(&ada_page).await;

        message_member(&bob_page, &ada).await;
        assert_eq!(wait_direct(&bob_page).await, ada);
        send(&bob_page, CHANNEL, "Just between us").await;
        read_latest(&bob_page, CHANNEL, "Just between us").await;

        assert_eq!(wait_direct(&ada_page).await, bob);
        open(&ada_page, &bob).await;
        let heard = read_latest(&ada_page, CHANNEL, "Just between us").await;
        assert!(heard.starts_with(&bob), "{heard}");
    }
}
