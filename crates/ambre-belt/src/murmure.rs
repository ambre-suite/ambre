//! Murmure, driven in the browser: two users in one channel, through the
//! relay.

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use crate::page::{BOOT, Page, STEP, drag};
    use crate::relay::Relay;

    /// The WebDriver codes of the keys these tests press.
    const ENTER: &str = "\u{E007}";
    const UP: &str = "\u{E013}";
    const DOWN: &str = "\u{E015}";
    const SHIFT: &str = "\u{E008}";
    const F10: &str = "\u{E03A}";

    /// A fresh dev user name. The relay gives it the address
    /// `<name>@dev.local`.
    pub(crate) fn user() -> String {
        format!("belt-{}", uuid::Uuid::new_v4().simple())
    }

    /// Serves the build in front of `relay`, signs `name` in and opens
    /// Murmure. Waits for the sidebar's control that creates a space.
    pub(crate) async fn murmure(relay: Arc<Relay>, name: &str) -> Page {
        enter(Page::home(relay).await, name).await
    }

    /// Signs `name` in on `page`, which shows the home screen, and opens
    /// Murmure. Waits for the sidebar's control that creates a space.
    pub(crate) async fn enter(page: Page, name: &str) -> Page {
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
    pub(crate) async fn prompt(page: &Page, scope: &str, class: &str, text: &str) {
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
    /// `invitee_page` then follows the invite link (`SYNC-236`).
    pub(crate) async fn space_with_channel(
        page: &Page,
        space: &str,
        channel: &str,
        invitee: &str,
        invitee_page: &Page,
    ) {
        prompt(page, ".murmure-spaces", "murmure-new-space", space).await;
        // A channel asked for at once waits for its space to reach the relay
        // (`SYNC-189`), so one request opens it.
        prompt(page, ".murmure-space", "murmure-new-channel", channel).await;
        open(page, channel).await;
        let link = invite_into_space(page, invitee).await;
        follow(invitee_page, &link).await;
    }

    /// Invites `invitee` into the open space from `page`, and confirms their
    /// key. Answers the path and the fragment of the invite link the toolbar
    /// then shows: a Murmure page pinned to the inviter (`SYNC-236`). The
    /// link of an earlier invite is put away first, so the one read is the
    /// link of this invite.
    pub(crate) async fn invite_into_space(page: &Page, invitee: &str) -> String {
        page.browser
            .script(
                "document.querySelector('.murmure-invite-link-close')?.click(); \
                 window.__beltInviteLink = null; return true;",
                json!([]),
            )
            .await;
        prompt(
            page,
            ".murmure-space",
            "murmure-space-invite",
            &format!("{invitee}@dev.local"),
        )
        .await;
        confirm_key(page).await;
        page.wait_toast(&format!("Invited {invitee}")).await;
        page.browser
            .wait_for(
                "the invite link in the toolbar",
                "const l = document.querySelector('.murmure-invite-link .ambre-share-link-input'); \
                 if (!l || !l.value) return false; \
                 window.__beltInviteLink = l.value; return true;",
                STEP,
            )
            .await;
        page.invite_link().await
    }

    /// Opens Murmure on `page` from the invite `link`, as the invitee does
    /// with the link the inviter sent. Its inviter roots every Murmure open
    /// of the session (`SYNC-235`). Waits for the sidebar.
    pub(crate) async fn follow(page: &Page, link: &str) {
        page.browser.goto(&format!("{}{link}", page.origin)).await;
        page.browser
            .wait_for(
                "the Murmure sidebar",
                "return !!document.querySelector('.murmure-new-space-toggle');",
                BOOT,
            )
            .await;
    }

    /// Clicks the sidebar entry `name` until the content shows its channel.
    /// A channel to join is joined by the first click and opened by the next.
    pub(crate) async fn open(page: &Page, name: &str) {
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

    /// Presses the composer at the foot of the canvas `selector` until the
    /// input pipe holds the keyboard. A composer not yet drawn leaves the
    /// press to its list, so the press is made again.
    pub(crate) async fn take_keys(page: &Page, selector: &str) {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let at = reachable(page, selector, true).await;
            page.browser.perform(json!([drag(at, at, 0)])).await;
            if holds_keys(page).await {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the composer of {selector} never took the keys"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Whether the input pipe holds the keyboard.
    pub(crate) async fn holds_keys(page: &Page) -> bool {
        page.browser
            .script(
                "return document.activeElement?.classList.contains('ambre-input');",
                json!([]),
            )
            .await
            == json!(true)
    }

    /// The key actions that type `text`, then send it with Enter.
    pub(crate) fn typed_and_sent(text: &str) -> Vec<Value> {
        text.chars()
            .map(String::from)
            .chain(std::iter::once(ENTER.to_owned()))
            .flat_map(|k| {
                [
                    json!({ "type": "keyDown", "value": k }),
                    json!({ "type": "keyUp", "value": k }),
                ]
            })
            .collect()
    }

    /// Presses the composer at the foot of the canvas `selector`, types
    /// `text` and sends it with Enter. Nothing on the page says the message
    /// left, so the peer's read is the check.
    async fn send(page: &Page, selector: &str, text: &str) {
        take_keys(page, selector).await;
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": typed_and_sent(text) }]))
            .await;
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
    pub(crate) const CHANNEL: &str = "canvas.ambre-canvas";

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
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
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

    /// `SYNC-241` to `SYNC-243`: Bob follows Ada's Murmure link once, then
    /// reloads Murmure with no link. The reload starts a new Sync worker with
    /// no root of the session. The roots the first session proved come back
    /// from the local store, so the channel opens and reads again.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn an_invitee_reopens_murmure_without_the_link_on_the_roots_it_kept() {
        let relay = Arc::new(Relay::start().await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        send(&ada_page, CHANNEL, "First words").await;
        open(&bob_page, "general").await;
        read_latest(&bob_page, CHANNEL, "First words").await;

        bob_page
            .browser
            .goto(&format!(
                "{}/murmure/{}",
                bob_page.origin,
                uuid::Uuid::now_v7()
            ))
            .await;
        open(&bob_page, "general").await;
        read_latest(&bob_page, CHANNEL, "First words").await;
    }

    /// Goes back from the shown channel to the channel list, as the control
    /// in the channel's header does. No channel shows then.
    pub(crate) async fn leave_pane(page: &Page) {
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
        wait_unread_within(page, name, count, STEP).await;
    }

    /// As [`wait_unread`], within `within`.
    pub(crate) async fn wait_unread_within(
        page: &Page,
        name: &str,
        count: u32,
        within: std::time::Duration,
    ) {
        page.browser
            .wait_for(
                &format!("{count} unread in {name}"),
                &format!(
                    "const e = {}; if (!e) return false; \
                     const c = e.querySelector('.murmure-count[aria-label=\"Unread messages\"]'); \
                     return !!c && c.textContent === '{count}';",
                    entry_js(name)
                ),
                within,
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
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
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
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
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
    pub(crate) async fn confirm_key(page: &Page) {
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
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
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

    /// Records each peer connection and each worker the page makes from now on, and the
    /// notes a worker posts. The page makes neither before the join. It also records each
    /// track a sender takes, with whether its transform was there, and any data channel.
    pub(crate) const RECORD: &str = "window.beltPcs = []; window.beltNotes = []; \
        const P = window.RTCPeerConnection; \
        window.RTCPeerConnection = class extends P { \
          constructor(...a) { super(...a); window.beltPcs.push(this); } }; \
        window.beltTracks = []; \
        const T = RTCRtpSender.prototype.replaceTrack; \
        RTCRtpSender.prototype.replaceTrack = function (t) { \
          window.beltTracks.push(!!t && !!this.transform); return T.call(this, t); }; \
        const A = P.prototype.addTrack; \
        P.prototype.addTrack = function (...a) { window.beltTracks.push(false); return A.apply(this, a); }; \
        const D = P.prototype.createDataChannel; \
        P.prototype.createDataChannel = function (...a) { window.beltChannels = true; return D.apply(this, a); }; \
        const W = window.Worker; window.beltWorkers = 0; \
        window.Worker = class extends W { constructor(...a) { super(...a); window.beltWorkers += 1; \
          this.addEventListener('message', (e) => window.beltNotes.push( \
          ArrayBuffer.isView(e.data) ? Array.from(e.data) : e.data)); } }; \
        return true;";

    /// What the peer connection of one page sent and decoded so far. Each audio stream gives
    /// its bytes and its packets, as the encoder and the decoder count them around the
    /// transforms. The page gives the samples it decoded, less the ones it concealed, its
    /// local description and the port of the peer it reached. The energy is the sum of the
    /// audio energy of each received stream, as the decoder measures it.
    async fn call_stats(page: &Page) -> Value {
        page.browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 const pcs = window.beltPcs || []; \
                 if (pcs.length === 0) { done({ pcs: 0 }); return; } \
                 const pc = pcs[0]; \
                 pc.getStats().then((stats) => { \
                   const out = { pcs: pcs.length, sdp: pc.localDescription && pc.localDescription.sdp, \
                     state: pc.connectionState, sent: [], received: [], decoded: 0, energy: 0, concealed: 0, remotePort: null, \
                     notes: window.beltNotes, tracks: window.beltTracks, channels: !!window.beltChannels }; \
                   stats.forEach((s) => { \
                     const pair = (s.type === 'transport' && stats.get(s.selectedCandidatePairId)) \
                       || (s.type === 'candidate-pair' && s.nominated && s); \
                     if (pair && pair.state === 'succeeded') { \
                       const r = stats.get(pair.remoteCandidateId); if (r) out.remotePort = r.port; } \
                     if (s.kind !== 'audio') return; \
                     if (s.type === 'outbound-rtp') out.sent.push([s.bytesSent, s.packetsSent, s.timestamp]); \
                     if (s.type === 'inbound-rtp') { \
                       out.received.push([s.bytesReceived, s.packetsReceived]); \
                       out.decoded += (s.totalSamplesReceived || 0) - (s.concealedSamples || 0); \
                       out.concealed += s.concealedSamples || 0; \
                       out.energy += s.totalAudioEnergy || 0; } \
                   }); \
                   done(out); }, (e) => done({ error: String(e) }));",
                json!([]),
            )
            .await
    }

    /// How many frames the call worker of one page dropped for their size (`CALL-067`).
    fn oversize(stats: &Value) -> usize {
        stats["notes"]
            .as_array()
            .map_or(0, |notes| notes.iter().filter(|n| **n == json!(3)).count())
    }

    /// The decoded samples of each page, once each has decoded `samples` more than `from`. Each
    /// reading also holds that no page dropped a frame for its size.
    async fn decoded_past(pages: &[&Page], from: &[f64], samples: f64) -> Vec<Value> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        loop {
            let mut seen = Vec::new();
            for page in pages {
                seen.push(call_stats(page).await);
            }
            for stats in &seen {
                assert_eq!(
                    oversize(stats),
                    0,
                    "frames dropped for their size: {stats:#}"
                );
            }
            let done = seen
                .iter()
                .zip(from)
                .all(|(s, base)| s["decoded"].as_f64().is_some_and(|d| d >= base + samples));
            if done {
                return seen;
            }
            if std::time::Instant::now() >= deadline {
                let mut consoles = Vec::new();
                for page in pages {
                    consoles.push(page.browser.console().await);
                }
                panic!("within 40 s, the pages decoded {seen:#?}, and logged {consoles:#?}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    /// `CALL-001`, `CALL-003`, `CALL-008` to `CALL-011`, `CALL-022`, `CALL-029`, `CALL-059`,
    /// `CALL-060`, `CALL-064`, `CALL-065` and `ZK-021`, in the browser.
    ///
    /// Three clients of the application join one call through the relay and the SFU, by a
    /// command of the bus. Each decodes the sealed audio of the others. Each page holds one
    /// peer connection, to the SFU alone, with no data channel.
    ///
    /// Carol leaves the channel. Once
    /// the switch passes, she still receives what the SFU forwards and decodes none of it, so
    /// the SFU forwards no clear audio. Ada and Bob still decode each other. Ada leaves the
    /// call, and her call worker erases its keys.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn three_clients_decode_each_others_sealed_audio_through_the_sfu() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob, carol) = (user(), user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        let carol_page = murmure(Arc::clone(&relay), &carol).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        let link = invite_into_space(&ada_page, &carol).await;
        follow(&carol_page, &link).await;
        open(&bob_page, "general").await;
        open(&carol_page, "general").await;
        // The pages of earlier tests of this process listed channels of their own, so the
        // channel is the one listed last.
        let channel = crate::serve::asked()
            .iter()
            .rev()
            .find_map(|path| {
                path.strip_prefix("/api/channels/")?
                    .strip_suffix("/documents")
                    .map(str::to_owned)
            })
            .expect("a page listed the documents of its channel");
        let pages = [&ada_page, &bob_page, &carol_page];
        for page in pages {
            page.browser.script(RECORD, json!([])).await;
        }
        // Ada starts the call: a join of a call the relay holds no state of is refused as
        // ended (`CALL-068`). The others join the call she started.
        let call = start_call(&ada_page, &channel).await;
        // The call message follows the seat of its starter (`MURMURE-408`).
        read_until(&bob_page, "a live call", |h| h.contains(LIVE), ROSTER_WAIT).await;
        for page in [&bob_page, &carol_page] {
            let joined = page
                .browser
                .script(
                    "window.wasmBindings.join_call(arguments[0], arguments[1]); return true;",
                    json!([call, channel]),
                )
                .await;
            assert_eq!(joined, json!(true), "the page has no join");
        }

        // One second of audio from each of the two others, at 48 kHz.
        let seen = decoded_past(&pages, &[0.0; 3], 96_000.0).await;
        for stats in &seen {
            assert!(
                !stats["sent"].as_array().is_none_or(Vec::is_empty),
                "no audio sent: {stats:#}"
            );
            assert_eq!(stats["pcs"], json!(1), "one peer connection: {stats:#}");
            assert_eq!(
                stats["remotePort"].as_u64(),
                Some(u64::from(sfu.media_port)),
                "the one peer is the SFU: {stats:#}"
            );
            // Each track reached its sender once the transform was there (`CALL-011`).
            assert_eq!(
                stats["tracks"],
                json!([true]),
                "a track before its transform: {stats:#}"
            );
            assert_eq!(stats["channels"], json!(false), "a data channel: {stats:#}");
            let sdp = stats["sdp"].as_str().unwrap_or_default();
            assert!(!sdp.contains("m=application"), "a data channel: {sdp}");
        }

        // Carol leaves the channel: the log no longer holds her, and the others switch their
        // keys within the delay (`CALL-027`, `CALL-060`).
        let left = carol_page
            .browser
            .script(
                "window.wasmBindings.leave_channel(arguments[0]); return true;",
                json!([channel]),
            )
            .await;
        assert_eq!(left, json!(true));
        tokio::time::sleep(std::time::Duration::from_secs(9)).await;
        let before: Vec<f64> = {
            let mut v = Vec::new();
            for page in pages {
                v.push(call_stats(page).await["decoded"].as_f64().unwrap_or(0.0));
            }
            v
        };
        decoded_past(&[&ada_page, &bob_page], &before[..2], 48_000.0).await;
        let carol_after = call_stats(&carol_page).await;
        let carol_more = carol_after["decoded"].as_f64().unwrap_or(0.0) - before[2];
        assert!(
            carol_more < 4_800.0,
            "Carol decoded {carol_more} samples after the switch: {carol_after:#}"
        );

        // No page dropped a frame for its size over the whole call: each encoder held the
        // constant bitrate the SFU asked (`CALL-067`).
        for page in pages {
            let stats = call_stats(page).await;
            assert_eq!(
                oversize(&stats),
                0,
                "frames dropped for their size: {stats:#}"
            );
        }

        // Ada leaves the call: her call worker erases every key, then ends (`CALL-029`).
        ada_page
            .browser
            .script("window.wasmBindings.leave_call(); return true;", json!([]))
            .await;
        ada_page
            .browser
            .wait_for(
                "the call worker erased its keys",
                "return (window.beltNotes || []).includes(2);",
                STEP,
            )
            .await;
    }

    /// Makes every receive transform of the page fail to attach. From now on, it also records
    /// each track the peer connection hands the page, and each media element the page plays.
    const NO_RECEIVE_TRANSFORM: &str = "window.beltPlayed = []; window.beltReceived = []; \
        const Q = window.RTCPeerConnection; \
        window.RTCPeerConnection = class extends Q { constructor(...a) { super(...a); \
          this.addEventListener('track', (e) => window.beltReceived.push(e.track)); } }; \
        Object.defineProperty(RTCRtpReceiver.prototype, 'transform', { configurable: true, \
          get() { return null; }, \
          set(v) { throw new Error('belt: no receive transform'); } }); \
        const play = HTMLMediaElement.prototype.play; \
        HTMLMediaElement.prototype.play = function () { \
          window.beltPlayed.push(this); return play.call(this); }; \
        return true;";

    /// What the page received and played. It counts the tracks the peer connection handed
    /// over and the live ones, and tells whether a played media element holds a live one.
    const PLAYED_REMOTE: &str = "const remote = window.beltReceived || []; \
        const live = remote.filter((t) => t.readyState === 'live'); \
        const played = (window.beltPlayed || []).some((el) => el.srcObject \
          && el.srcObject.getTracks().some((t) => live.includes(t))); \
        return { remote: remote.length, live: live.length, played };";

    /// `CALL-146`, audit `N40`, in the browser: Ada and Bob join one call, and no receive
    /// transform attaches on Bob's page. Bob's page plays no track it receives, and stops
    /// each one, so its decoder never reads what the SFU forwards unopened. Ada decodes Bob,
    /// so the call carries his audio.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_receiver_with_no_transform_plays_nothing() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        open(&bob_page, "general").await;
        let channel = last_channel();
        for page in [&ada_page, &bob_page] {
            page.browser.script(RECORD, json!([])).await;
        }
        bob_page
            .browser
            .script(NO_RECEIVE_TRANSFORM, json!([]))
            .await;

        let call = start_call(&ada_page, &channel).await;
        read_until(&bob_page, "a live call", |h| h.contains(LIVE), ROSTER_WAIT).await;
        let joined = bob_page
            .browser
            .script(
                "window.wasmBindings.join_call(arguments[0], arguments[1]); return true;",
                json!([call, channel]),
            )
            .await;
        assert_eq!(joined, json!(true), "the page has no join");
        decoded_past(&[&ada_page], &[0.0], 48_000.0).await;

        let seen = bob_page
            .browser
            .wait_for(
                "the receivers of Bob's page",
                &format!(
                    "const s = (() => {{ {PLAYED_REMOTE} }})(); return s && s.remote > 0 ? s : null;"
                ),
                STEP,
            )
            .await;
        assert_eq!(
            seen["played"],
            json!(false),
            "a track with no transform played: {seen}"
        );
        assert_eq!(
            seen["live"],
            json!(0),
            "a track with no transform is still live: {seen}"
        );
    }

    /// Presses the list of the channel, then `Up`, until what the live region reads of the
    /// latest message satisfies `done`, within `within`. Answers what it read.
    pub(crate) async fn read_until(
        page: &Page,
        what: &str,
        done: impl Fn(&str) -> bool,
        within: std::time::Duration,
    ) -> String {
        read_until_after(page, what, done, within, &[UP]).await
    }

    /// As [`read_until`], pressing `keys` in place of `Up`. `Up` then `Down` reads the latest
    /// message of a list whose focus already rests on it. Keys with `Down` press it once more
    /// when the first read misses.
    async fn read_until_after(
        page: &Page,
        what: &str,
        done: impl Fn(&str) -> bool,
        within: std::time::Duration,
        keys: &[&str],
    ) -> String {
        let deadline = std::time::Instant::now() + within;
        loop {
            let at = reachable(page, CHANNEL, false).await;
            page.browser.perform(json!([drag(at, at, 0)])).await;
            press(page, keys).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let mut heard = page.announced().await;
            if !done(&heard) && keys.contains(&DOWN) {
                // The focus rested above the latest message: one more step reaches it.
                press(page, &[DOWN]).await;
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                heard = page.announced().await;
            }
            if done(&heard) {
                return heard;
            }
            if std::time::Instant::now() >= deadline {
                // The console holds the warnings of the Sync worker, such as a roster that
                // was not read, which the announcement alone cannot tell apart.
                let console = page.browser.console().await;
                panic!("the latest message read {heard:?}, not {what}, and logged {console:#?}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        }
    }

    /// How long a reader may wait for who is in a call: two asks of the interval of the
    /// budget row, and the round trips.
    pub(crate) const ROSTER_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

    /// The titles of the menu of the latest message, which the keyboard opens on it once
    /// `Up` focused it. The menu closes again.
    async fn menu_of_latest(page: &Page) -> Vec<String> {
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": SHIFT },
                { "type": "keyDown", "value": F10 },
                { "type": "keyUp", "value": F10 },
                { "type": "keyUp", "value": SHIFT },
            ]}]))
            .await;
        let items = page
            .browser
            .wait_for(
                "the menu of the message",
                "const i = [...document.querySelectorAll('.ambre-ctx-item')]; \
                 return i.length ? i.map(e => e.textContent) : null;",
                STEP,
            )
            .await;
        press(page, &["\u{E00C}"]).await;
        items
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Focuses the latest message, a live call message, and runs Join call from its menu.
    async fn join_from_message(page: &Page) {
        read_until_after(
            page,
            "a live call",
            |h| h.contains(LIVE),
            ROSTER_WAIT,
            &[UP, DOWN],
        )
        .await;
        // The focus may rest on the message above the call message, as in a direct message
        // whose first message came before the call. `Down` then reaches the call message.
        for _ in 0..4 {
            page.browser
                .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                    { "type": "keyDown", "value": SHIFT },
                    { "type": "keyDown", "value": F10 },
                    { "type": "keyUp", "value": F10 },
                    { "type": "keyUp", "value": SHIFT },
                ]}]))
                .await;
            let joined = page
                .browser
                .wait_for(
                    "the menu of the message",
                    "const i = [...document.querySelectorAll('.ambre-ctx-item')]; \
                     if (!i.length) return null; \
                     const j = i.find(e => e.textContent.includes('Join call')); \
                     if (j) { j.click(); return 'joined'; } return 'other';",
                    STEP,
                )
                .await;
            if joined == json!("joined") {
                return;
            }
            press(page, &["\u{E00C}", DOWN]).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        panic!("no menu of a call message offered Join call");
    }

    /// What a call message says while its call runs, and once it ended.
    pub(crate) const LIVE: &str = "Call in progress";
    const ENDED: &str = "Call ended";
    const NOBODY: &str = "nobody yet";

    /// The members a call message names, read from its body alone, or `None` unless it reads
    /// a live call with its members. A line opens on its author, who is no member of the
    /// call, so a name matched anywhere in the line proves nothing (`MURMURE-409`).
    fn roster(heard: &str) -> Option<Vec<&str>> {
        let (_, inside) = heard.split_once(LIVE)?;
        let inside = inside.strip_prefix(": ")?;
        // A reply count or a reaction follows the body after a full stop.
        let inside = inside.split(". ").next().unwrap_or_default();
        if inside == NOBODY {
            return Some(Vec::new());
        }
        Some(inside.split(", ").collect())
    }

    /// Whether the call message reads exactly `members` in the call, in any order.
    fn in_call(heard: &str, members: &[&str]) -> bool {
        roster(heard).is_some_and(|mut seated| {
            let mut members = members.to_vec();
            seated.sort_unstable();
            members.sort_unstable();
            seated == members
        })
    }

    /// A roster read from the author's line: Ada wrote the message, and Bob alone is seated.
    #[test]
    fn a_roster_reads_the_body_and_never_the_author() {
        let line = "belt-ada, 07:07 PM: Call in progress: belt-bob";
        assert_eq!(roster(line), Some(vec!["belt-bob"]));
        assert!(!in_call(line, &["belt-ada", "belt-bob"]));
        assert!(!in_call(line, &["belt-ada"]));
        assert!(in_call(line, &["belt-bob"]));
        let both = "belt-ada, 07:07 PM: Call in progress: belt-bob, belt-ada. 1 reply";
        assert!(in_call(both, &["belt-ada", "belt-bob"]));
        assert_eq!(roster("belt-ada, 07:07 PM: Call in progress"), None);
        assert_eq!(
            roster("belt-ada, 07:07 PM: Call in progress: nobody yet"),
            Some(vec![])
        );
        assert!(!in_call("belt-ada, 07:07 PM: Call ended", &[]));
    }

    /// Starts a call in `channel` from the page of `page`, as the call button will, and
    /// answers its identifier.
    async fn start_call(page: &Page, channel: &str) -> String {
        let call = page
            .browser
            .script(
                "return window.wasmBindings.start_call(arguments[0]);",
                json!([channel]),
            )
            .await;
        let call = call.as_str().unwrap_or_default().to_owned();
        assert!(!call.is_empty(), "the page started no call");
        call
    }

    /// The channel the pages of this test listed last.
    pub(crate) fn last_channel() -> String {
        crate::serve::asked()
            .iter()
            .rev()
            .find_map(|path| {
                path.strip_prefix("/api/channels/")?
                    .strip_suffix("/documents")
                    .map(str::to_owned)
            })
            .expect("a page listed the documents of its channel")
    }

    /// `MURMURE-408`, `MURMURE-409`, `MURMURE-411`, `MURMURE-412`, `MURMURE-591`,
    /// `MURMURE-595`, `MURMURE-596`, in the browser.
    ///
    /// Ada starts a call in a channel, and every member reads its call message with Ada in
    /// it. Bob joins from the message, and the two decode each other. Carol, out of the call,
    /// reads both in it. Ada leaves, then Bob, whose leave the relay counts as the last. Every
    /// member reads the call ended, and Carol's menu offers no join.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_call_message_shows_who_is_in_the_call_until_it_ended() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob, carol) = (user(), user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        let carol_page = murmure(Arc::clone(&relay), &carol).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        let link = invite_into_space(&ada_page, &carol).await;
        follow(&carol_page, &link).await;
        open(&bob_page, "general").await;
        open(&carol_page, "general").await;
        let channel = last_channel();
        for page in [&ada_page, &bob_page] {
            page.browser.script(RECORD, json!([])).await;
        }

        start_call(&ada_page, &channel).await;
        for page in [&ada_page, &bob_page, &carol_page] {
            read_until(
                page,
                "Ada in the call",
                |h| in_call(h, &[&ada]),
                ROSTER_WAIT,
            )
            .await;
        }

        join_from_message(&bob_page).await;
        decoded_past(&[&ada_page, &bob_page], &[0.0; 2], 48_000.0).await;
        // Exactly Ada and Bob: Carol, out of the call, is no member of it.
        read_until(
            &carol_page,
            "Ada and Bob in the call",
            |h| in_call(h, &[&ada, &bob]),
            ROSTER_WAIT,
        )
        .await;

        for page in [&ada_page, &bob_page] {
            page.browser
                .script("window.wasmBindings.leave_call(); return true;", json!([]))
                .await;
            page.browser
                .wait_for(
                    "the call worker erased its keys",
                    "return (window.beltNotes || []).includes(2);",
                    STEP,
                )
                .await;
        }
        for page in [&ada_page, &bob_page, &carol_page] {
            read_until(page, "the call ended", |h| h.contains(ENDED), ROSTER_WAIT).await;
        }
        let menu = menu_of_latest(&carol_page).await;
        assert!(
            !menu.iter().any(|item| item.contains("Join call")),
            "a call that ended offers a join: {menu:?}"
        );
    }

    /// `MURMURE-592`, in the browser: Ada starts a call alone, and Carol reads her in it.
    /// Ada's browser dies with no leave, so no end is written. Carol reads the call ended
    /// once the relay holds no state of it.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_call_whose_last_participant_crashed_reads_ended() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, carol) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let carol_page = murmure(Arc::clone(&relay), &carol).await;
        space_with_channel(&ada_page, "Belt", "general", &carol, &carol_page).await;
        open(&carol_page, "general").await;
        let channel = last_channel();
        start_call(&ada_page, &channel).await;
        read_until(
            &carol_page,
            "Ada in the call",
            |h| in_call(h, &[&ada]),
            ROSTER_WAIT,
        )
        .await;
        // The browser quits under the call: the page sends no leave frame.
        drop(ada_page);
        read_until(
            &carol_page,
            "the call ended",
            |h| h.contains(ENDED),
            ROSTER_WAIT,
        )
        .await;
    }

    /// Clicks the control `selector` of the channel header once it shows.
    pub(crate) async fn click_header(page: &Page, selector: &str) {
        let selector = Value::String(format!(".murmure-channel-header {selector}"));
        page.browser
            .wait_for(
                &format!("the control {selector}"),
                &format!(
                    "const b = document.querySelector({selector}); \
                     if (!b) return false; b.click(); return true;"
                ),
                STEP,
            )
            .await;
    }

    /// The `aria-pressed` of the mute control, once it shows.
    async fn mute_pressed(page: &Page, pressed: &str) {
        let pressed = Value::String(pressed.to_owned());
        page.browser
            .wait_for(
                &format!("mute pressed {pressed}"),
                &format!(
                    "const b = document.querySelector('.murmure-channel-header .murmure-call-mute'); \
                     return !!b && b.getAttribute('aria-pressed') === {pressed};"
                ),
                STEP,
            )
            .await;
    }

    /// What one window of `WINDOW` reads of a call, each field the growth over the window. The
    /// receiver gives the energy and the samples it decoded, the sender its packets and bytes.
    #[derive(Debug)]
    struct Window {
        energy: f64,
        decoded: f64,
        packets: f64,
        bytes: f64,
        /// The time between the two reads of the sender, from the stamps of its statistics.
        span_ms: f64,
    }

    impl Window {
        /// The sealed chunks the sender sent. Every sealed audio chunk has one size, so the
        /// payload bytes count them. A packet with no payload carries no chunk.
        fn chunks(&self) -> f64 {
            chunks_of(self.bytes, "the sender", self)
        }
    }

    /// The sealed chunks in `bytes` of payload. Fails unless the bytes are a whole number of
    /// chunks: a payload of another size would tell the SFU something (`CALL-075`).
    fn chunks_of(bytes: f64, who: &str, context: &impl std::fmt::Debug) -> f64 {
        let size = ambre_core::AUDIO_CHUNK_SIZE.get() as f64;
        let chunks = (bytes / size).round();
        assert!(
            (chunks * size - bytes).abs() < 0.5,
            "{who} sent a payload that is no whole number of sealed chunks: {context:?}"
        );
        chunks
    }

    /// Samples, in the page, the first audio stream the page sends, every 100 ms from now on
    /// until [`PADDING_TRACE`] reads it. Each sample is the stamp of the statistics, the packets
    /// and the payload bytes. A sample of the page reads the statistics of one instant, where a
    /// window of the test reads them at its two edges alone.
    const PADDING_SAMPLE: &str = "const pc = (window.beltPcs || [])[0]; window.beltPadding = []; \
        window.beltPaddingTimer = setInterval(() => pc.getStats().then((stats) => { \
          let first = true; stats.forEach((s) => { \
            if (first && s.type === 'outbound-rtp' && s.kind === 'audio') { first = false; \
              window.beltPadding.push([s.timestamp, s.packetsSent, s.bytesSent]); } }); }), 100);";

    /// Stops the sampler of [`PADDING_SAMPLE`], and returns its samples.
    const PADDING_TRACE: &str =
        "clearInterval(window.beltPaddingTimer); return window.beltPadding || [];";

    /// The padding a sender sent, read by the page every 100 ms. Each sample holds the stamp
    /// of the statistics and the packets with no payload sent so far.
    #[derive(Debug)]
    struct PaddingTrace {
        samples: Vec<PaddingSample>,
    }

    /// One sample of a [`PaddingTrace`].
    #[derive(Debug, Clone, Copy)]
    struct PaddingSample {
        at_ms: f64,
        empty: f64,
    }

    impl PaddingTrace {
        /// The trace of the samples `raw` of [`PADDING_TRACE`]. Fails unless each sample holds
        /// a whole number of sealed chunks (`CALL-075`).
        fn of(raw: &Value, who: &str) -> Self {
            let samples = raw
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|row| {
                            let at = |i: usize| row[i].as_f64().unwrap_or(0.0);
                            PaddingSample {
                                at_ms: at(0),
                                empty: at(1) - chunks_of(at(2), who, row),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            Self { samples }
        }

        /// The stamp of each sample where the padding grew.
        fn rises(&self) -> Vec<f64> {
            self.samples
                .windows(2)
                .filter(|pair| pair[1].empty > pair[0].empty)
                .map(|pair| pair[1].at_ms)
                .collect()
        }

        /// The first and the last stamp of the trace.
        fn span(&self) -> Option<(f64, f64)> {
            Some((self.samples.first()?.at_ms, self.samples.last()?.at_ms))
        }
    }

    /// The probes of one trace: each a run of rises closer than `merge_ms` to the one before.
    /// The congestion control of the browser can send a probe as two clusters, 200 ms apart.
    /// One probe then rises over two samples.
    fn probes(rises: &[f64], merge_ms: f64) -> Vec<f64> {
        std::iter::once(None)
            .chain(rises.iter().copied().map(Some))
            .zip(rises.iter().copied())
            .filter(|(before, at)| before.is_none_or(|b| at - b >= merge_ms))
            .map(|(_, at)| at)
            .collect()
    }

    /// Why a padding trace shows something a mute would show (`ZK-025`).
    #[derive(Debug, PartialEq)]
    enum PaddingLeak {
        /// The sender pads while the witness, which never mutes, never does.
        PadsAlone,
        /// The witness sent one probe: the trace reads no probing interval from it.
        NoInterval,
        /// Two probes of the sender are closer than half the probing interval: one of the two
        /// is off the cadence of its congestion control.
        ExtraProbe { gap_ms: u64, interval_ms: u64 },
        /// The sender sent no probe for more than one and a half probing intervals: a probe
        /// of its cadence is missing.
        MissingProbe { gap_ms: u64, interval_ms: u64 },
    }

    /// Holds the padding of `sender` to the cadence of its congestion control, read from
    /// `witness`, which never mutes over the same span (`ZK-025`).
    ///
    /// The browser probes the uplink once per interval of its congestion control. It fills
    /// each probe with the chunks in its queue first, then pads the rest. So the number of
    /// packets with no payload in one probe varies from probe to probe, mute or not. The
    /// phase of the probes of two senders differs too.
    ///
    /// What a mute could show is the time of a probe: one more probe, or one less. A probe the
    /// mute adds splits an interval in two parts, and one of the two is at most half an
    /// interval. A probe the mute removes leaves a gap of two intervals. Half an interval is
    /// therefore the widest tolerance that still tells either case from the cadence.
    fn padding_cadence(sender: &PaddingTrace, witness: &PaddingTrace) -> Result<(), PaddingLeak> {
        let witness_rises = witness.rises();
        let sender_rises = sender.rises();
        if witness_rises.is_empty() {
            return if sender_rises.is_empty() {
                Ok(())
            } else {
                Err(PaddingLeak::PadsAlone)
            };
        }
        let longest = witness_rises
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold(0.0, f64::max);
        // The clusters of one probe are far closer than a quarter of an interval.
        let merge_ms = longest / 4.0;
        let witness_probes = probes(&witness_rises, merge_ms);
        let mut gaps: Vec<f64> = witness_probes
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect();
        if gaps.is_empty() {
            return Err(PaddingLeak::NoInterval);
        }
        gaps.sort_by(f64::total_cmp);
        let interval = gaps[gaps.len() / 2];
        let ms = |v: f64| v.round() as u64;
        let sender_probes = probes(&sender_rises, merge_ms);
        if let Some(gap) = sender_probes
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .find(|gap| *gap < interval / 2.0)
        {
            return Err(PaddingLeak::ExtraProbe {
                gap_ms: ms(gap),
                interval_ms: ms(interval),
            });
        }
        // The edges of the trace stand for a probe just outside it: a gap at an edge is at
        // most one interval.
        let Some((first, last)) = sender.span() else {
            return Ok(());
        };
        let edges = std::iter::once(first)
            .chain(sender_probes.iter().copied())
            .chain(std::iter::once(last))
            .collect::<Vec<_>>();
        match edges
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .find(|gap| *gap > interval * 1.5)
        {
            Some(gap) => Err(PaddingLeak::MissingProbe {
                gap_ms: ms(gap),
                interval_ms: ms(interval),
            }),
            None => Ok(()),
        }
    }

    /// A trace of 100 ms samples over `span_ms`, with `packets` of padding at each stamp of
    /// `probes`.
    fn synthetic(span_ms: f64, probes: &[(f64, f64)]) -> PaddingTrace {
        let samples = (0..=(span_ms / 100.0) as usize)
            .map(|i| {
                let at_ms = i as f64 * 100.0;
                let empty = probes
                    .iter()
                    .filter(|(at, _)| *at <= at_ms)
                    .map(|(_, n)| n)
                    .sum();
                PaddingSample { at_ms, empty }
            })
            .collect();
        PaddingTrace { samples }
    }

    /// The probes of the witness on Firefox 160: one each 5 s, some in two clusters.
    fn witness() -> PaddingTrace {
        synthetic(
            12_000.0,
            &[
                (1_000.0, 2.0),
                (6_000.0, 1.0),
                (6_200.0, 1.0),
                (11_000.0, 2.0),
            ],
        )
    }

    #[test]
    fn a_sender_out_of_phase_with_the_witness_shows_no_mute() {
        // Ada probes 2.5 s after Bob, with more packets per probe. A window of 4 s then holds
        // 4 packets of Ada and none of Bob, and a count of packets read that as a leak.
        let ada = synthetic(12_000.0, &[(3_500.0, 4.0), (8_500.0, 3.0)]);
        assert_eq!(padding_cadence(&ada, &witness()), Ok(()));
    }

    #[test]
    fn a_probe_the_mute_adds_breaks_the_cadence() {
        let ada = synthetic(
            12_000.0,
            &[
                (1_000.0, 2.0),
                (4_000.0, 1.0),
                (6_000.0, 2.0),
                (11_000.0, 2.0),
            ],
        );
        assert_eq!(
            padding_cadence(&ada, &witness()),
            Err(PaddingLeak::ExtraProbe {
                gap_ms: 2_000,
                interval_ms: 5_000
            })
        );
    }

    #[test]
    fn a_probe_the_mute_removes_breaks_the_cadence() {
        let ada = synthetic(12_000.0, &[(1_000.0, 2.0), (11_000.0, 2.0)]);
        assert_eq!(
            padding_cadence(&ada, &witness()),
            Err(PaddingLeak::MissingProbe {
                gap_ms: 10_000,
                interval_ms: 5_000
            })
        );
    }

    #[test]
    fn a_sender_that_pads_where_the_witness_never_does_shows_a_mute() {
        let ada = synthetic(12_000.0, &[(4_000.0, 2.0)]);
        assert_eq!(
            padding_cadence(&ada, &synthetic(12_000.0, &[])),
            Err(PaddingLeak::PadsAlone)
        );
        assert_eq!(
            padding_cadence(&synthetic(12_000.0, &[]), &synthetic(12_000.0, &[])),
            Ok(())
        );
    }

    #[test]
    fn a_witness_of_one_probe_gives_no_interval() {
        let one = synthetic(12_000.0, &[(4_000.0, 2.0)]);
        assert_eq!(padding_cadence(&one, &one), Err(PaddingLeak::NoInterval));
    }

    /// The time between two chunks of one sender. The encoder hands the transform one Opus
    /// frame of 20 ms at a time (`ambre_core::AUDIO_BITRATE`).
    const CHUNK_PERIOD_MS: f64 = 20.0;

    /// The length of one window of [`Window`].
    const WINDOW: std::time::Duration = std::time::Duration::from_secs(3);

    /// The first audio stream `stats` sent, as its bytes, its packets and the stamp of the read.
    fn sent(stats: &Value) -> (f64, f64, f64) {
        let first = &stats["sent"][0];
        let at = |i: usize| first[i].as_f64().unwrap_or(0.0);
        (at(0), at(1), at(2))
    }

    /// The statistics of a sender and of its receiver, read at one instant.
    struct Reading {
        sender: Value,
        receiver: Value,
    }

    /// Reads the statistics of `sender` and of `receiver`.
    async fn read(sender: &Page, receiver: &Page) -> Reading {
        Reading {
            sender: call_stats(sender).await,
            receiver: call_stats(receiver).await,
        }
    }

    /// The [`Window`] from the reading `from` to the reading `to`.
    fn between(from: &Reading, to: &Reading) -> Window {
        let ((b0, p0, t0), (b1, p1, t1)) = (sent(&from.sender), sent(&to.sender));
        let num = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
        Window {
            energy: num(&to.receiver, "energy") - num(&from.receiver, "energy"),
            decoded: num(&to.receiver, "decoded") - num(&from.receiver, "decoded"),
            packets: p1 - p0,
            bytes: b1 - b0,
            span_ms: t1 - t0,
        }
    }

    /// Reads one [`Window`] of the audio `sender` sends to `receiver`.
    async fn window(sender: &Page, receiver: &Page) -> Window {
        let start = read(sender, receiver).await;
        tokio::time::sleep(WINDOW).await;
        between(&start, &read(sender, receiver).await)
    }

    /// `MURMURE-599`, `MURMURE-603`, `MURMURE-604`, `MURMURE-605`, `CALL-074`, `CALL-075` and
    /// `ZK-025`, in the browser.
    ///
    /// Ada calls Bob by the call button in the header of their direct message, and Bob joins
    /// from the call message. Each decodes the other. Ada mutes: Bob still decodes a frame every
    /// 20 ms, and each holds silence alone. Ada sends at the same rate and the same size as
    /// before, so the SFU reads a silence. Ada unmutes, and Bob hears her again.
    ///
    /// Ada hangs up, and Bob reads her out of the call. Bob hangs up last, and the call ends.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        open(&bob_page, "general").await;
        leave_pane(&ada_page).await;
        message_member(&bob_page, &ada).await;
        assert_eq!(wait_direct(&bob_page).await, ada);
        send(&bob_page, CHANNEL, "Call me").await;
        assert_eq!(wait_direct(&ada_page).await, bob);
        open(&ada_page, &bob).await;
        read_latest(&ada_page, CHANNEL, "Call me").await;
        for page in [&ada_page, &bob_page] {
            page.browser.script(RECORD, json!([])).await;
        }

        // The call button names itself, and takes the keyboard focus (`MURMURE-605`).
        let focused = ada_page
            .browser
            .wait_for(
                "the call button",
                "const b = document.querySelector('.murmure-channel-header .murmure-call-start'); \
                 if (!b) return null; b.focus(); \
                 return [b.getAttribute('aria-label'), document.activeElement === b];",
                STEP,
            )
            .await;
        assert_eq!(focused, json!(["Start a call", true]));
        click_header(&ada_page, ".murmure-call-start").await;
        join_from_message(&bob_page).await;
        decoded_past(&[&ada_page, &bob_page], &[0.0; 2], 48_000.0).await;
        mute_pressed(&ada_page, "false").await;
        assert!(
            ada_page
                .browser
                .script(
                    "return !document.querySelector('.murmure-channel-header .murmure-call-start');",
                    json!([])
                )
                .await
                == json!(true),
            "a page in a call offers no call button (`MURMURE-603`)"
        );

        // Each page samples its padding from the live window to the end of the unmute window.
        // The trace then holds the press of the mute and of the unmute.
        for page in [&ada_page, &bob_page] {
            page.browser.script(PADDING_SAMPLE, json!([])).await;
        }
        let live = window(&ada_page, &bob_page).await;
        assert!(live.energy > 0.0, "Bob hears nothing of Ada: {live:?}");
        assert!(live.packets > 0.0, "Ada sends nothing: {live:?}");

        let at_mute = read(&ada_page, &bob_page).await;
        click_header(&ada_page, ".murmure-call-mute").await;
        mute_pressed(&ada_page, "true").await;
        // The frames already in the jitter buffer of Bob play out first.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let start = read(&ada_page, &bob_page).await;
        tokio::time::sleep(WINDOW).await;
        let end = read(&ada_page, &bob_page).await;
        let muted = between(&start, &end);
        // From before the press to the end of the mute window: the instant of the mute.
        let across = between(&at_mute, &end);
        // Bob decodes a frame every 20 ms, each of silence (`CALL-074`).
        assert!(
            muted.decoded >= 48_000.0 * 2.0,
            "Bob stopped decoding Ada: {muted:?}"
        );
        assert!(
            muted.energy < live.energy / 100.0,
            "Bob hears Ada through the mute: {muted:?} against {live:?}"
        );
        println!("mute windows: live {live:?}, muted {muted:?}, across {across:?}");
        // The SFU reads the same chunks at the same rate (`CALL-075`). The span of the mute
        // holds its press. Over that span,
        // over the mute window and over the live window, Ada sends one chunk per frame. Each
        // window reads against the frame of the encoder, so no window carries the edge error
        // of another. A read of the statistics may count the chunk in flight at its stamp.
        // So the tolerance is one chunk at each of the two edges.
        for w in [&live, &across, &muted] {
            let expected = w.span_ms / CHUNK_PERIOD_MS;
            assert!(
                (w.chunks() - expected).abs() <= 2.0,
                "a mute changed the chunk rate: {w:?} against {expected} chunks"
            );
        }

        click_header(&ada_page, ".murmure-call-mute").await;
        mute_pressed(&ada_page, "false").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let back = window(&ada_page, &bob_page).await;
        // The fake microphone beeps, so a window holds more or less of its sound: the unmute
        // reads against the silence of the mute.
        assert!(
            back.energy > muted.energy * 100.0 && back.energy > 0.0,
            "Bob hears nothing of Ada after the unmute: {back:?} against {muted:?}"
        );
        // A packet with no payload is the padding of the congestion control of the browser.
        // The SFU reads it from a member who stays quiet too. Through the mute and the
        // unmute, Ada probes at the cadence of Bob, who never mutes (`ZK-025`).
        let ada_padding = PaddingTrace::of(
            &ada_page.browser.script(PADDING_TRACE, json!([])).await,
            "Ada",
        );
        let bob_padding = PaddingTrace::of(
            &bob_page.browser.script(PADDING_TRACE, json!([])).await,
            "Bob",
        );
        println!(
            "padding rises: Ada {:?}, Bob {:?}",
            ada_padding.rises(),
            bob_padding.rises()
        );
        assert_eq!(
            padding_cadence(&ada_padding, &bob_padding),
            Ok(()),
            "the padding of Ada shows the mute: {ada_padding:?} against {bob_padding:?}"
        );

        // Ada hangs up: her call worker erases its keys, and Bob reads her out of the call
        // (`MURMURE-604`).
        click_header(&ada_page, ".murmure-call-hang-up").await;
        ada_page
            .browser
            .wait_for(
                "the call worker erased its keys",
                "return (window.beltNotes || []).includes(2);",
                STEP,
            )
            .await;
        // Out of the call, her header offers the call button again.
        ada_page
            .browser
            .wait_for(
                "the call button back",
                "return !!document.querySelector('.murmure-channel-header .murmure-call-start');",
                STEP,
            )
            .await;
        read_until_after(
            &bob_page,
            "Bob alone in the call",
            // The author of the message is Ada: who is in the call follows its state.
            |h| in_call(h, &[&bob]),
            ROSTER_WAIT,
            &[UP, DOWN],
        )
        .await;
        click_header(&bob_page, ".murmure-call-hang-up").await;
        read_until_after(
            &bob_page,
            "the call ended",
            |h| h.contains(ENDED),
            ROSTER_WAIT,
            &[UP, DOWN],
        )
        .await;
    }

    /// Records, on top of [`RECORD`], what a change of device touches:
    /// * each microphone the page opens, and the constraints it asks with;
    /// * each transform a sender holds when it takes a track;
    /// * each local description, and each element that plays;
    /// * the bytes of each message the page posts to a worker or a port (`ZK-026`).
    ///
    /// A microphone opens `window.beltOpening` ms late, as a real device does.
    const DEVICES: &str = "window.beltMics = []; window.beltCaptures = []; window.beltTransforms = []; \
        window.beltDescriptions = 0; window.beltPlayers = []; window.beltPosted = []; \
        const G = MediaDevices.prototype.getUserMedia; \
        MediaDevices.prototype.getUserMedia = function (c) { \
          window.beltCaptures.push(JSON.stringify(c)); \
          return G.call(this, c).then((s) => new Promise((ok) => setTimeout(() => { \
            window.beltMics.push(s.getAudioTracks()[0]); ok(s); }, window.beltOpening || 0))); }; \
        const R = RTCRtpSender.prototype.replaceTrack; \
        RTCRtpSender.prototype.replaceTrack = function (t) { \
          if (t) window.beltTransforms.push(this.transform); return R.call(this, t); }; \
        const L = RTCPeerConnection.prototype.setLocalDescription; \
        RTCPeerConnection.prototype.setLocalDescription = function (...a) { \
          window.beltDescriptions += 1; return L.apply(this, a); }; \
        const Y = HTMLMediaElement.prototype.play; \
        HTMLMediaElement.prototype.play = function () { \
          if (!window.beltPlayers.includes(this)) window.beltPlayers.push(this); return Y.call(this); }; \
        const keep = (d) => { try { \
          const kind = Object.prototype.toString.call(d); \
          if (kind === '[object ArrayBuffer]' || kind === '[object SharedArrayBuffer]') \
            window.beltPosted.push(new Uint8Array(d.slice(0))); \
          else if (ArrayBuffer.isView(d)) window.beltPosted.push(new Uint8Array(d.buffer.slice(d.byteOffset, d.byteOffset + d.byteLength))); \
          else window.beltPosted.push(new TextEncoder().encode(typeof d === 'string' ? d : (JSON.stringify(d) || ''))); \
        } catch (e) {} }; \
        for (const C of [Worker, MessagePort]) { let o = C.prototype; \
          while (!Object.prototype.hasOwnProperty.call(o, 'postMessage')) o = Object.getPrototypeOf(o); \
          const P = o.postMessage; \
          o.postMessage = function (m, ...r) { keep(m); return P.call(this, m, ...r); }; } \
        return true;";

    /// The samples the decoders of `page` concealed so far. A receiver makes up each frame that
    /// its sender did not send, that the network lost, or that came too late to play.
    async fn concealed(page: &Page) -> f64 {
        call_stats(page).await["concealed"]
            .as_f64()
            .unwrap_or(f64::MAX)
    }

    /// The length of the window that measures what a change of microphone conceals.
    const SPAN: std::time::Duration = std::time::Duration::from_secs(4);

    /// Opens the device panel of the header, once it shows.
    async fn open_devices(page: &Page) {
        click_header(page, ".murmure-call-devices").await;
        page.browser
            .wait_for(
                "the device panel",
                "return !!document.querySelector('.murmure-channel-header .murmure-call-panel');",
                STEP,
            )
            .await;
    }

    /// Picks the entry `at` of the select `selector` of the device panel, and answers its text.
    async fn pick(page: &Page, selector: &str, at: usize) -> String {
        let selector = Value::String(format!(".murmure-channel-header {selector}"));
        page.browser
            .wait_for(
                &format!("the entry {at} of {selector}"),
                &format!(
                    "const s = document.querySelector({selector}); \
                     if (!s || s.options.length <= {at}) return null; \
                     s.value = String({at}); s.dispatchEvent(new Event('change', {{ bubbles: true }})); \
                     return s.options[{at}].textContent;"
                ),
                STEP,
            )
            .await
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// The identifier the browser gives the device labelled `label`.
    async fn device_id(page: &Page, label: &str) -> String {
        page.browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 navigator.mediaDevices.enumerateDevices().then((l) => { \
                   const d = l.find((d) => d.label === arguments[0]); done(d ? d.deviceId : null); });",
                json!([label]),
            )
            .await
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// Waits until a sender of `page` took `count` tracks in all, and answers whether each
    /// held the same transform.
    async fn tracks_taken(page: &Page, count: usize) -> bool {
        page.browser
            .wait_for(
                &format!("{count} tracks taken"),
                &format!(
                    "const t = window.beltTransforms; if (t.length < {count}) return null; \
                     return t.every((x) => !!x && x === t[0]);"
                ),
                STEP,
            )
            .await
            == json!(true)
    }

    /// `CALL-078`, `CALL-079`, `CALL-080`, `CALL-081`, `MURMURE-607`, `MURMURE-608`,
    /// `MURMURE-610`, `MURMURE-612`, `MURMURE-613` and `ZK-026`, in the browser.
    ///
    /// Before the grant, the panel shows the default microphone alone, and why. Ada calls Bob,
    /// mutes, then picks another microphone in the panel. Bob conceals no more frames of hers
    /// than she conceals of his over the same window. Bob still hears silence: the mute carried
    /// over. The call kept its peer connection, its call worker, its transform and its session.
    /// Ada unmutes, and Bob hears the new microphone.
    ///
    /// The microphone goes away: the call sends the default one, and the header says so. No
    /// message to a worker carried a device identifier or a label. The next call of Ada opens
    /// the chosen microphone.
    ///
    /// Headless Chrome offers two fake microphones besides the default one. Headless Firefox
    /// offers one, so its run picks that one by its identifier, which opens a new track and
    /// replaces it all the same.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_member_changes_its_microphone_during_a_call_with_no_gap() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        open(&bob_page, "general").await;
        leave_pane(&ada_page).await;
        message_member(&bob_page, &ada).await;
        assert_eq!(wait_direct(&bob_page).await, ada);
        send(&bob_page, CHANNEL, "Call me").await;
        assert_eq!(wait_direct(&ada_page).await, bob);
        open(&ada_page, &bob).await;
        read_latest(&ada_page, CHANNEL, "Call me").await;
        for page in [&ada_page, &bob_page] {
            page.browser.script(RECORD, json!([])).await;
            page.browser.script(DEVICES, json!([])).await;
        }

        // Before the grant, the browser names no device (`MURMURE-608`).
        ada_page
            .browser
            .script(
                "const E = MediaDevices.prototype.enumerateDevices; window.beltEnumerate = E; \
                 navigator.mediaDevices.enumerateDevices = () => E.call(navigator.mediaDevices).then((l) => \
                   l.map((d) => Object.create(MediaDeviceInfo.prototype, { kind: { value: d.kind }, \
                     deviceId: { value: '' }, label: { value: '' }, groupId: { value: '' } }))); \
                 return true;",
                json!([]),
            )
            .await;
        open_devices(&ada_page).await;
        let hidden = ada_page
            .browser
            .wait_for(
                "the panel before the grant",
                "const p = document.querySelector('.murmure-channel-header .murmure-call-panel'); \
                 const s = p && p.querySelector('.murmure-call-microphone'); \
                 const h = p && p.querySelector('.murmure-call-hint'); \
                 if (!s || !h) return null; \
                 return [[...s.options].map((o) => o.textContent), !!p.querySelector('.murmure-call-speaker')];",
                STEP,
            )
            .await;
        assert_eq!(hidden, json!([["Default microphone"], false]));
        ada_page
            .browser
            .script(
                "delete navigator.mediaDevices.enumerateDevices; return true;",
                json!([]),
            )
            .await;
        click_header(&ada_page, ".murmure-call-devices").await;

        click_header(&ada_page, ".murmure-call-start").await;
        join_from_message(&bob_page).await;
        decoded_past(&[&ada_page, &bob_page], &[0.0; 2], 48_000.0).await;
        assert!(tracks_taken(&ada_page, 1).await);
        let before = ada_page
            .browser
            .script(
                "return [window.beltPcs.length, window.beltWorkers, window.beltDescriptions];",
                json!([]),
            )
            .await;
        click_header(&ada_page, ".murmure-call-mute").await;
        mute_pressed(&ada_page, "true").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let muted = window(&ada_page, &bob_page).await;

        // The panel lists each microphone after the default one (`MURMURE-607`).
        open_devices(&ada_page).await;
        let listed = ada_page
            .browser
            .wait_for(
                "the named microphones",
                "const s = document.querySelector('.murmure-channel-header .murmure-call-microphone'); \
                 if (!s || s.options.length < 2) return null; \
                 return [...s.options].map((o) => o.textContent);",
                STEP,
            )
            .await;
        assert_eq!(listed[0], json!("Default microphone"), "{listed}");
        // A real microphone takes hundreds of milliseconds to open, where a fake one takes tens.
        ada_page
            .browser
            .script("window.beltOpening = 500; return true;", json!([]))
            .await;
        // Ada receives Bob over the same window, a stream the change does not touch. A stall of
        // the machine delays the frames of both streams at once, so Bob and Ada conceal alike.
        // A reference window at another time misses that stall, and blames the change for it.
        let start = concealed(&bob_page).await;
        let control_start = concealed(&ada_page).await;
        let began = std::time::Instant::now();
        let label = pick(&ada_page, ".murmure-call-microphone", 1).await;
        assert!(tracks_taken(&ada_page, 2).await, "the transform changed");
        tokio::time::sleep(SPAN.saturating_sub(began.elapsed())).await;
        let across = concealed(&bob_page).await - start;
        let control = concealed(&ada_page).await - control_start;
        ada_page
            .browser
            .script("window.beltOpening = 0; return true;", json!([]))
            .await;
        eprintln!("concealed: {across} samples across the change, {control} with no change");
        // Each frame of Ada reaches Bob across the change (`CALL-078`). So Bob makes up no more
        // frames than Ada makes up of Bob in the same window. One frame is 960 samples.
        assert!(
            across <= control + 960.0,
            "{across} samples concealed across the change, against {control} with none"
        );
        // The same peer connection, call worker and session (`CALL-079`).
        let after = ada_page
            .browser
            .script(
                "return [window.beltPcs.length, window.beltWorkers, window.beltDescriptions];",
                json!([]),
            )
            .await;
        assert_eq!(after, before, "the change renegotiated or spawned");

        // The mute carried over to the new track (`CALL-080`).
        mute_pressed(&ada_page, "true").await;
        let still = window(&ada_page, &bob_page).await;
        assert!(
            still.energy <= muted.energy * 10.0 + 1e-6,
            "Bob hears Ada through the mute after the change: {still:?} against {muted:?}"
        );
        assert!(
            still.chunks() >= muted.chunks() * 0.9,
            "{still:?} against {muted:?}"
        );

        click_header(&ada_page, ".murmure-call-mute").await;
        mute_pressed(&ada_page, "false").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let back = window(&ada_page, &bob_page).await;
        assert!(
            back.energy > still.energy * 100.0 && back.energy > 0.0,
            "Bob hears nothing of the new microphone: {back:?} against {still:?}"
        );

        // The choice stays on this browser (`MURMURE-612`), and no message to a worker carries
        // a device identifier or a label (`ZK-026`, `MURMURE-613`).
        let id = device_id(&ada_page, &label).await;
        assert!(!id.is_empty(), "no identifier for {label:?}");
        let stored = ada_page
            .browser
            .script(
                "return localStorage.getItem('ambre.call.microphone');",
                json!([]),
            )
            .await;
        assert_eq!(stored, json!(id));
        let leaked = ada_page
            .browser
            .script(
                "const has = (b, n) => { outer: for (let i = 0; i + n.length <= b.length; i++) { \
                   for (let j = 0; j < n.length; j++) if (b[i + j] !== n[j]) continue outer; return true; } return false; }; \
                 const texts = arguments[0].map((t) => new TextEncoder().encode(t)); \
                 return [window.beltPosted.length, arguments[0].filter((t, i) => window.beltPosted.some((b) => has(b, texts[i])))];",
                json!([[id, label]]),
            )
            .await;
        assert!(
            leaked[0].as_u64().is_some_and(|n| n > 0),
            "no message recorded: {leaked}"
        );
        assert_eq!(leaked[1], json!([]), "a device left the page");

        // The microphone goes away: its track ends, and the browser lists it no more. The call
        // sends the default one, and the header says so (`CALL-081`, `MURMURE-610`).
        ada_page
            .browser
            .script(
                "const gone = arguments[0]; const E = MediaDevices.prototype.enumerateDevices; \
                 navigator.mediaDevices.enumerateDevices = () => E.call(navigator.mediaDevices).then((l) => \
                   l.filter((d) => d.deviceId !== gone)); \
                 const t = window.beltMics[window.beltMics.length - 1]; t.stop(); \
                 t.dispatchEvent(new Event('ended')); \
                 navigator.mediaDevices.dispatchEvent(new Event('devicechange')); return true;",
                json!([id]),
            )
            .await;
        let notice = ada_page
            .browser
            .wait_for(
                "the fallback notice",
                "const n = document.querySelector('.murmure-channel-header .murmure-call-fallback'); \
                 return n ? [n.getAttribute('role'), n.textContent] : null;",
                STEP,
            )
            .await;
        assert_eq!(notice[0], json!("status"), "{notice}");
        assert!(
            notice[1]
                .as_str()
                .is_some_and(|t| t.starts_with("Microphone gone")),
            "{notice}"
        );
        assert!(
            tracks_taken(&ada_page, 3).await,
            "the fallback changed the transform"
        );
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let fallen = window(&ada_page, &bob_page).await;
        assert!(
            fallen.energy > still.energy * 100.0 && fallen.energy > 0.0,
            "Bob hears nothing after the fallback: {fallen:?}"
        );

        // The device comes back, and the next call of Ada opens it (`MURMURE-612`).
        ada_page
            .browser
            .script(
                "delete navigator.mediaDevices.enumerateDevices; return true;",
                json!([]),
            )
            .await;
        click_header(&ada_page, ".murmure-call-hang-up").await;
        click_header(&ada_page, ".murmure-call-start").await;
        let exact = ada_page
            .browser
            .wait_for(
                "the next capture",
                "const c = window.beltCaptures; return c.length >= 4 ? c[c.length - 1] : null;",
                STEP,
            )
            .await;
        assert!(
            exact
                .as_str()
                .is_some_and(|c| c.contains(&format!("\"exact\":{}", json!(id)))),
            "the next call opened another microphone: {exact}"
        );
        click_header(&ada_page, ".murmure-call-hang-up").await;
        click_header(&bob_page, ".murmure-call-hang-up").await;
    }

    /// The `sinkId` of each audio element of `page`, once each reads `sink` and every audio
    /// receiver has one. Answers the count of elements and of audio receivers.
    async fn sinks(page: &Page, sink: &str) -> Value {
        page.browser
            .wait_for(
                &format!("every slot on {sink:?}"),
                &format!(
                    "const want = {}; const p = window.beltPlayers.filter((e) => \
                       e.srcObject && e.srcObject.getAudioTracks().length > 0); \
                     const r = window.beltPcs[0].getReceivers().filter((r) => r.track.kind === 'audio'); \
                     if (p.length === 0 || p.length < r.length || !p.every((e) => e.sinkId === want)) return null; \
                     const tracks = new Set(p.map((e) => e.srcObject.getAudioTracks()[0])); \
                     return [p.length, r.length, tracks.size];",
                    Value::String(sink.to_owned())
                ),
                STEP,
            )
            .await
    }

    /// `CALL-077`, `CALL-082`, `CALL-083`, `MURMURE-609` and `MURMURE-610`, in the browser.
    ///
    /// Bob picks a speaker before the call, then joins Ada's call. Each audio slot plays through
    /// an element of its own, on that speaker. Bob picks another speaker during the call, and
    /// every slot moves to it. The speaker goes away: every slot plays on the default output,
    /// and the header says so.
    ///
    /// Headless Firefox lists an output only over an audio server. Its run reads that the panel
    /// offers a speaker exactly where the browser lists one. It also reads that each slot plays
    /// through an element of its own.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_chosen_speaker_plays_every_slot_and_gives_way_when_it_goes() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        open(&bob_page, "general").await;
        leave_pane(&ada_page).await;
        message_member(&bob_page, &ada).await;
        assert_eq!(wait_direct(&bob_page).await, ada);
        send(&bob_page, CHANNEL, "Call me").await;
        assert_eq!(wait_direct(&ada_page).await, bob);
        open(&ada_page, &bob).await;
        read_latest(&ada_page, CHANNEL, "Call me").await;
        for page in [&ada_page, &bob_page] {
            page.browser.script(RECORD, json!([])).await;
            page.browser.script(DEVICES, json!([])).await;
        }
        let firefox = cfg!(feature = "firefox");

        // Out of the call, the panel is where Bob sets the speaker of the next call.
        let mut first = String::new();
        if !firefox {
            open_devices(&bob_page).await;
            let label = pick(&bob_page, ".murmure-call-speaker", 1).await;
            first = device_id(&bob_page, &label).await;
            assert!(!first.is_empty(), "no identifier for {label:?}");
            click_header(&bob_page, ".murmure-call-devices").await;
        }

        click_header(&ada_page, ".murmure-call-start").await;
        join_from_message(&bob_page).await;
        decoded_past(&[&ada_page, &bob_page], &[0.0; 2], 48_000.0).await;
        // Each slot plays through an element of its own (`CALL-077`), on the speaker chosen
        // before it arrived (`CALL-082`).
        let slots = sinks(&bob_page, &first).await;
        assert_eq!(slots[0], slots[1], "{slots}");
        assert_eq!(slots[0], slots[2], "two slots share an element: {slots}");

        open_devices(&bob_page).await;
        if firefox {
            // Headless Firefox lists an output only over an audio server. The panel offers a
            // speaker exactly where the browser lists one: no control that does nothing
            // (`MURMURE-609`).
            let listed = bob_page
                .browser
                .script_async(
                    "const done = arguments[arguments.length - 1]; \
                     navigator.mediaDevices.enumerateDevices().then((l) => done(l.some((d) => \
                       d.kind === 'audiooutput' && d.deviceId && d.deviceId !== 'default')));",
                    json!([]),
                )
                .await;
            let want = if listed == json!(true) {
                "speaker"
            } else {
                "none"
            };
            let offered = bob_page
                .browser
                .wait_for(
                    "the microphone choice",
                    "const p = document.querySelector('.murmure-channel-header .murmure-call-panel'); \
                     return p && p.querySelector('.murmure-call-microphone') \
                       ? (p.querySelector('.murmure-call-speaker') ? 'speaker' : 'none') : null;",
                    STEP,
                )
                .await;
            assert_eq!(
                offered,
                json!(want),
                "a speaker choice that routes nothing, or none that routes"
            );
            click_header(&bob_page, ".murmure-call-hang-up").await;
            click_header(&ada_page, ".murmure-call-hang-up").await;
            return;
        }

        let label = pick(&bob_page, ".murmure-call-speaker", 2).await;
        let second = device_id(&bob_page, &label).await;
        assert_ne!(second, first);
        sinks(&bob_page, &second).await;

        // The speaker goes away: every slot plays on the default output (`CALL-083`), and the
        // header says so (`MURMURE-610`).
        bob_page
            .browser
            .script(
                "const gone = arguments[0]; const E = MediaDevices.prototype.enumerateDevices; \
                 navigator.mediaDevices.enumerateDevices = () => E.call(navigator.mediaDevices).then((l) => \
                   l.filter((d) => d.deviceId !== gone)); \
                 navigator.mediaDevices.dispatchEvent(new Event('devicechange')); return true;",
                json!([second]),
            )
            .await;
        let notice = bob_page
            .browser
            .wait_for(
                "the fallback notice",
                "const n = document.querySelector('.murmure-channel-header .murmure-call-fallback'); \
                 return n ? n.textContent : null;",
                STEP,
            )
            .await;
        assert!(
            notice
                .as_str()
                .is_some_and(|t| t.starts_with("Speaker gone")),
            "{notice}"
        );
        sinks(&bob_page, "").await;
        click_header(&bob_page, ".murmure-call-hang-up").await;
        click_header(&ada_page, ".murmure-call-hang-up").await;
    }

    /// Replaces the capture of the page with one that fails with the error named `name`.
    async fn refuse_microphone(page: &Page, name: &str) {
        page.browser
            .script(
                "navigator.mediaDevices.getUserMedia = () => \
                   Promise.reject(new DOMException('refused by the belt', arguments[0])); \
                 return true;",
                json!([name]),
            )
            .await;
    }

    /// The call sockets the pages of this process opened so far.
    fn call_sockets() -> usize {
        crate::serve::asked()
            .iter()
            .filter(|path| path.starts_with("/api/calls/") && path.ends_with("/socket"))
            .count()
    }

    /// `MURMURE-601`, `MURMURE-602` and `CALL-076`, in the browser.
    ///
    /// Ada presses the call button, and the browser refuses the microphone. The header reads
    /// that the microphone is blocked, as an alert, and keeps the call button. The page spawned
    /// no call worker, made no peer connection and opened no call socket. So the relay seats
    /// no one, and no call message follows. A browser with no capture device reads that no
    /// microphone is found.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_refused_microphone_starts_no_call_and_says_why() {
        let relay = Arc::new(Relay::start().await);
        let ada = user();
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        prompt(&ada_page, ".murmure-spaces", "murmure-new-space", "Belt").await;
        prompt(
            &ada_page,
            ".murmure-space",
            "murmure-new-channel",
            "general",
        )
        .await;
        open(&ada_page, "general").await;
        ada_page.browser.script(RECORD, json!([])).await;
        let sockets = call_sockets();

        let cases = [
            ("NotAllowedError", "Microphone blocked"),
            ("NotFoundError", "No microphone found"),
        ];
        for (error, said) in cases {
            refuse_microphone(&ada_page, error).await;
            click_header(&ada_page, ".murmure-call-start").await;
            let shown = ada_page
                .browser
                .wait_for(
                    &format!("the refusal {said:?}"),
                    &format!(
                        "const r = document.querySelector('.murmure-channel-header .murmure-call-refusal'); \
                         return r && r.textContent.startsWith({}) ? r.getAttribute('role') : null;",
                        Value::String(said.to_owned())
                    ),
                    STEP,
                )
                .await;
            assert_eq!(shown, json!("alert"), "the refusal is no alert");
        }
        // Whatever the page would send has had the time to leave.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let page = ada_page
            .browser
            .script(
                "return [window.beltWorkers, window.beltPcs.length, \
                   !!document.querySelector('.murmure-channel-header .murmure-call-start')];",
                json!([]),
            )
            .await;
        assert_eq!(
            page,
            json!([0, 0, true]),
            "a refused microphone made a worker or a peer connection, or hid the call button"
        );
        assert_eq!(
            call_sockets(),
            sockets,
            "a refused microphone opened a call socket"
        );
    }

    /// Each audio element of `page` that plays a receiver of its call. Each one gives the track
    /// of the receiver, the energy it decoded so far, the level its sender sent and the volume
    /// of the element. The level is the audio level of the last packet. The sender writes it in
    /// the clear header extension, before any volume of a receiver.
    async fn slots(page: &Page) -> Value {
        page.browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 const pc = (window.beltPcs || [])[0]; if (!pc) { done([]); return; } \
                 const rs = pc.getReceivers().filter((r) => r.track && r.track.kind === 'audio'); \
                 Promise.all(rs.map((r) => r.getStats().then((st) => { let energy = 0; \
                   st.forEach((s) => { if (s.type === 'inbound-rtp') energy += s.totalAudioEnergy || 0; }); \
                   const e = window.beltPlayers.find((p) => p.srcObject && p.srcObject.getAudioTracks()[0] === r.track); \
                   const level = Math.max(0, ...r.getSynchronizationSources().map((s) => s.audioLevel || 0)); \
                   return { track: r.track.id, energy, level, volume: e ? e.volume : null }; }))) \
                   .then(done, (e) => done({ error: String(e) }));",
                json!([]),
            )
            .await
    }

    /// One slot over a window of [`WINDOW`]. It holds its track, the energy it played, the
    /// loudest level its sender sent and the volume of its element at the end.
    #[derive(Debug)]
    struct Slot {
        track: String,
        energy: f64,
        sent: f64,
        volume: f64,
    }

    /// What one window of [`WINDOW`] reads of the slots of a receiver.
    #[derive(Debug)]
    struct Heard {
        slots: Vec<Slot>,
    }

    impl Heard {
        /// The energy the receiver played. Chrome measures the energy after the volume of the
        /// element, and Firefox before it. So each slot takes the square of its volume again,
        /// since the element scales each sample by its volume. At a volume of 0 or 1 both agree.
        fn played(&self) -> f64 {
            self.slots
                .iter()
                .map(|s| s.energy * s.volume * s.volume)
                .sum()
        }

        /// The slot whose sender sent the loudest level: the one sender that speaks while the
        /// others mute.
        fn speaking(&self) -> &Slot {
            self.slots
                .iter()
                .max_by(|a, b| a.sent.total_cmp(&b.sent))
                .unwrap_or_else(|| panic!("no slot: {self:?}"))
        }
    }

    /// Reads one [`Heard`] window of `page`, the levels sampled each 100 ms.
    async fn heard(page: &Page) -> Heard {
        let field = |v: &Value, track: &str, key: &str| {
            v.as_array()
                .and_then(|a| a.iter().find(|s| s["track"] == json!(track)))
                .and_then(|s| s[key].as_f64())
                .unwrap_or(0.0)
        };
        let before = slots(page).await;
        let began = std::time::Instant::now();
        let mut sent: Vec<(String, f64)> = Vec::new();
        let mut after = before.clone();
        while began.elapsed() < WINDOW {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            after = slots(page).await;
            for s in after.as_array().into_iter().flatten() {
                let track = s["track"].as_str().unwrap_or_default().to_owned();
                let level = s["level"].as_f64().unwrap_or(0.0);
                match sent.iter_mut().find(|(t, _)| *t == track) {
                    Some((_, max)) => *max = max.max(level),
                    None => sent.push((track, level)),
                }
            }
        }
        let slots = after
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                let track = s["track"].as_str().unwrap_or_default().to_owned();
                Slot {
                    energy: field(&after, &track, "energy") - field(&before, &track, "energy"),
                    sent: sent
                        .iter()
                        .find(|(t, _)| *t == track)
                        .map_or(0.0, |(_, level)| *level),
                    volume: s["volume"].as_f64().unwrap_or(1.0),
                    track,
                }
            })
            .collect();
        Heard { slots }
    }

    /// The first byte of each roster ask of the Logic thread on the wire, from a real one. A
    /// reorder of the commands of the bus moves it, and this reads the new one.
    fn roster_tag() -> u64 {
        use ambre_core::id::{Call, Channel, Document, Id};
        let ask = ambre_bus::SyncCommand::AskCallRoster {
            call: Id::<Call>::mint(),
            channel: Id::<Channel>::mint(),
            meta: Id::<Document>::mint(),
        };
        postcard::to_allocvec(&ask)
            .ok()
            .and_then(|bytes| bytes.first().copied())
            .map(u64::from)
            .expect("a roster ask encodes")
    }

    /// Sets the slider named `label` of the panel of `page` to `value`, as a hand would. Answers
    /// the first bytes of each message the page posted to a worker or a port until the value
    /// applied. The panel applies the last value of a drag once the drag rests (`CHROME-015`).
    ///
    /// A roster ask is left out, since no volume makes one. The Logic thread posts one every
    /// interval while the view shows a channel with a live call (`MURMURE-595`).
    async fn slide(page: &Page, label: &str, value: &str) -> Value {
        page.browser
            .wait_for(
                &format!("the slider {label}"),
                &format!(
                    "return !![...document.querySelectorAll('.murmure-call-panel input[type=range]')] \
                       .find((i) => i.getAttribute('aria-label') === {});",
                    Value::String(label.to_owned())
                ),
                STEP,
            )
            .await;
        let sent = page
            .browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 const i = [...document.querySelectorAll('.murmure-call-panel input[type=range]')] \
                   .find((i) => i.getAttribute('aria-label') === arguments[0]); \
                 const n = window.beltPosted.length; \
                 i.value = arguments[1]; i.dispatchEvent(new Event('input', { bubbles: true })); \
                 setTimeout(() => done(window.beltPosted.slice(n) \
                   .map((p) => Array.from(p).slice(0, 16))), 500);",
                json!([label, value]),
            )
            .await;
        let tag = roster_tag();
        Value::Array(
            sent.as_array()
                .into_iter()
                .flatten()
                .filter(|post| post.get(0).and_then(Value::as_u64) != Some(tag))
                .cloned()
                .collect(),
        )
    }

    /// Mutes or unmutes `page`, and waits for the header to read it.
    pub(crate) async fn set_mute(page: &Page, pressed: &str) {
        let now = page
            .browser
            .script(
                "const b = document.querySelector('.murmure-channel-header .murmure-call-mute'); \
                 return b ? b.getAttribute('aria-pressed') : null;",
                json!([]),
            )
            .await;
        if now != json!(pressed) {
            click_header(page, ".murmure-call-mute").await;
        }
        mute_pressed(page, pressed).await;
    }

    /// Joins the call `call` of `channel` from `page`, by the binding a belt calls.
    async fn join(page: &Page, call: &str, channel: &str) {
        let joined = page
            .browser
            .script(
                "window.wasmBindings.join_call(arguments[0], arguments[1]); return true;",
                json!([call, channel]),
            )
            .await;
        assert_eq!(joined, json!(true), "the page has no join");
    }

    /// Waits until one slot of `page` carries a sender that speaks, over a window.
    async fn one_heard(page: &Page) -> Heard {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        loop {
            let window = heard(page).await;
            if window.speaking().sent > 0.0 {
                return window;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "within 40 s, no sender spoke into a slot: {window:?}"
            );
        }
    }

    /// `CALL-084` to `CALL-088`, `CALL-095`, `MURMURE-614`, `MURMURE-615`, `MURMURE-617`,
    /// `MURMURE-618` and `ZK-027`, in the browser.
    ///
    /// Ada, Bob and Carol share a call. Ada turns Bob down to nothing in the panel. While Bob
    /// alone speaks, Ada receives him and plays nothing of him. While Carol alone speaks, Ada
    /// plays her. The volume posts nothing to a worker, and stays in the storage of Ada.
    ///
    /// Bob and Carol leave, then Carol joins again first and Bob after her. Bob now reaches Ada
    /// through another slot, under another sender index, and Ada still plays nothing of him.
    /// Where an element keeps its own volume, the panel offers none.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_participant_turned_down_stays_silent_from_slot_to_slot() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob, carol) = (user(), user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        let carol_page = murmure(Arc::clone(&relay), &carol).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        let link = invite_into_space(&ada_page, &carol).await;
        follow(&carol_page, &link).await;
        open(&bob_page, "general").await;
        open(&carol_page, "general").await;
        open(&ada_page, "general").await;
        let channel = last_channel();
        for page in [&ada_page, &bob_page, &carol_page] {
            page.browser.script(RECORD, json!([])).await;
            page.browser.script(DEVICES, json!([])).await;
        }
        let call = start_call(&ada_page, &channel).await;
        read_until(&bob_page, "a live call", |h| h.contains(LIVE), ROSTER_WAIT).await;
        // Bob joins before Carol, so each takes a slot of Ada in that order.
        join(&bob_page, &call, &channel).await;
        decoded_past(&[&ada_page, &bob_page], &[0.0; 2], 48_000.0).await;
        join(&carol_page, &call, &channel).await;
        let base = call_stats(&ada_page).await["decoded"]
            .as_f64()
            .unwrap_or(0.0);
        decoded_past(&[&ada_page], &[base], 96_000.0).await;

        // The panel offers a volume for each other participant, named by its member
        // (`MURMURE-614`).
        open_devices(&ada_page).await;
        let (bob_label, carol_label) = (format!("Volume of {bob}"), format!("Volume of {carol}"));
        let named = ada_page
            .browser
            .wait_for(
                "a volume for Bob and for Carol",
                "const l = [...document.querySelectorAll('.murmure-call-panel .murmure-call-volume')] \
                   .map((i) => i.getAttribute('aria-label')).sort(); \
                 return l.length === 2 ? l : null;",
                STEP,
            )
            .await;
        let mut want = vec![bob_label.clone(), carol_label.clone()];
        want.sort();
        assert_eq!(named, json!(want));
        // The volume applies on the page, and posts nothing to a worker (`CALL-095`, `ZK-027`).
        assert_eq!(slide(&ada_page, &bob_label, "0").await, json!([]));

        // Bob alone speaks: Ada decodes him, and plays nothing of him (`CALL-085`).
        set_mute(&carol_page, "true").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let bob_alone = one_heard(&ada_page).await;
        let bob_slot = bob_alone.speaking().track.clone();
        assert_eq!(bob_alone.speaking().volume, 0.0, "{bob_alone:?}");
        assert!(bob_alone.played() < 1e-6, "Ada plays Bob: {bob_alone:?}");

        // Carol alone speaks: Ada plays her at her own volume (`CALL-086`).
        set_mute(&bob_page, "true").await;
        set_mute(&carol_page, "false").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let carol_alone = one_heard(&ada_page).await;
        assert_eq!(carol_alone.speaking().volume, 1.0, "{carol_alone:?}");
        assert_ne!(carol_alone.speaking().track, bob_slot, "{carol_alone:?}");
        assert!(
            carol_alone.played() > 0.0,
            "Ada does not play Carol: {carol_alone:?}"
        );
        set_mute(&bob_page, "false").await;

        // The volume stays on Ada's browser (`MURMURE-618`).
        let stored = ada_page
            .browser
            .script(
                "return localStorage.getItem('ambre.call.volumes');",
                json!([]),
            )
            .await;
        assert!(
            stored
                .as_str()
                .is_some_and(|t| t.ends_with("=0") && !t.contains(',')),
            "{stored}"
        );

        // Bob and Carol leave. Carol joins again first, so she takes the slot Bob held, and Bob
        // the one she held.
        for page in [&bob_page, &carol_page] {
            page.browser
                .script("window.wasmBindings.leave_call(); return true;", json!([]))
                .await;
            page.browser
                .wait_for(
                    "the call worker erased its keys",
                    "return (window.beltNotes || []).includes(2);",
                    STEP,
                )
                .await;
            page.browser
                .script(
                    "window.beltNotes = []; window.beltPcs = []; return true;",
                    json!([]),
                )
                .await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        join(&carol_page, &call, &channel).await;
        decoded_past(&[&carol_page], &[0.0], 48_000.0).await;
        join(&bob_page, &call, &channel).await;
        decoded_past(&[&bob_page], &[0.0], 96_000.0).await;

        // Bob alone speaks again, through another slot: Ada still plays nothing of him
        // (`CALL-084`, `CALL-087`, `CALL-088`).
        set_mute(&carol_page, "true").await;
        set_mute(&bob_page, "false").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let moved = one_heard(&ada_page).await;
        assert_ne!(
            moved.speaking().track,
            bob_slot,
            "Bob kept his slot: {moved:?}"
        );
        assert_eq!(moved.speaking().volume, 0.0, "{moved:?}");
        assert!(
            moved.played() < 1e-6,
            "Ada plays Bob after his move: {moved:?}"
        );
        set_mute(&carol_page, "false").await;
        set_mute(&bob_page, "true").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let carol_moved = one_heard(&ada_page).await;
        assert_eq!(carol_moved.speaking().volume, 1.0, "{carol_moved:?}");
        assert!(
            carol_moved.played() > 0.0,
            "Ada does not play Carol after her move: {carol_moved:?}"
        );

        // The arrow keys move a volume, which Carol then plays at (`MURMURE-617`).
        let focused = ada_page
            .browser
            .script(
                "const i = [...document.querySelectorAll('.murmure-call-panel .murmure-call-volume')] \
                   .find((i) => i.getAttribute('aria-label') === arguments[0]); \
                 i.focus(); return document.activeElement === i;",
                json!([carol_label]),
            )
            .await;
        assert_eq!(focused, json!(true));
        press(&ada_page, &["\u{E012}"]).await;
        let moved_by_key = ada_page
            .browser
            .wait_for(
                "the volume of Carol moved by the arrow",
                "const i = document.activeElement; \
                 return i && i.value === '95' ? i.getAttribute('aria-label') : null;",
                STEP,
            )
            .await;
        assert_eq!(moved_by_key, json!(carol_label));

        // An element that keeps its own volume, as on iOS: the panel offers no volume, and
        // still offers the gain (`MURMURE-615`).
        click_header(&ada_page, ".murmure-call-devices").await;
        ada_page
            .browser
            .script(
                "Object.defineProperty(HTMLMediaElement.prototype, 'volume', \
                   { get() { return 1; }, set(v) {}, configurable: true }); return true;",
                json!([]),
            )
            .await;
        open_devices(&ada_page).await;
        let offered = ada_page
            .browser
            .wait_for(
                "the gain with no volume",
                "const p = document.querySelector('.murmure-call-panel'); \
                 return p && p.querySelector('.murmure-call-gain') \
                   ? p.querySelectorAll('.murmure-call-volume').length : null;",
                STEP,
            )
            .await;
        assert_eq!(offered, json!(0), "a volume the element cannot take");
        for page in [&ada_page, &bob_page, &carol_page] {
            click_header(page, ".murmure-call-hang-up").await;
        }
    }

    /// The gain controls of `page`, once the panel shows them. Answers whether the automatic
    /// gain is checked, then the gain in percent.
    async fn gain_controls(page: &Page) -> Value {
        page.browser
            .wait_for(
                "the gain controls",
                "const g = document.querySelector('.murmure-call-panel .murmure-call-gain'); \
                 const a = document.querySelector('.murmure-call-panel .murmure-call-automatic-gain'); \
                 return g && a ? [a.checked, g.value] : null;",
                STEP,
            )
            .await
    }

    /// The constraints the last capture of `page` asked, once it made `count` in all.
    async fn capture(page: &Page, count: usize) -> Value {
        let text = page
            .browser
            .wait_for(
                &format!("{count} captures"),
                &format!(
                    "const c = window.beltCaptures; return c.length >= {count} ? c[c.length - 1] : null;"
                ),
                STEP,
            )
            .await;
        serde_json::from_str(text.as_str().unwrap_or_default()).unwrap_or_default()
    }

    /// `CALL-089` to `CALL-094`, `MURMURE-616`, `MURMURE-617` and `ZK-027`, in the browser.
    ///
    /// Ada calls Bob, and opens the panel: the meter reads her microphone. She sets the gain to
    /// nothing by hand. The microphone opens again with the automatic gain off and the echo
    /// canceller on, and Bob decodes silence. Ada sends at the same rate and size, under the
    /// same transform, peer connection and session. A higher gain brings her back.
    ///
    /// A mute still silences her, and a change of microphone keeps the gain. The arrow keys move
    /// the gain, and the automatic gain comes back by its checkbox.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_gain_set_by_hand_scales_the_sound_at_one_rate() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        open(&bob_page, "general").await;
        leave_pane(&ada_page).await;
        message_member(&bob_page, &ada).await;
        assert_eq!(wait_direct(&bob_page).await, ada);
        send(&bob_page, CHANNEL, "Call me").await;
        assert_eq!(wait_direct(&ada_page).await, bob);
        open(&ada_page, &bob).await;
        read_latest(&ada_page, CHANNEL, "Call me").await;
        for page in [&ada_page, &bob_page] {
            page.browser.script(RECORD, json!([])).await;
            page.browser.script(DEVICES, json!([])).await;
        }
        click_header(&ada_page, ".murmure-call-start").await;
        join_from_message(&bob_page).await;
        decoded_past(&[&ada_page, &bob_page], &[0.0; 2], 48_000.0).await;
        assert!(tracks_taken(&ada_page, 1).await);
        let first = capture(&ada_page, 1).await;
        assert_eq!(first["audio"]["autoGainControl"], json!(true), "{first}");
        let before = ada_page
            .browser
            .script(
                "return [window.beltPcs.length, window.beltWorkers, window.beltDescriptions];",
                json!([]),
            )
            .await;
        let live = window(&ada_page, &bob_page).await;
        assert!(live.energy > 0.0, "Bob hears nothing of Ada: {live:?}");

        // The panel offers the gain, its automatic return and a meter that reads the
        // microphone (`MURMURE-616`), each named (`MURMURE-617`).
        open_devices(&ada_page).await;
        assert_eq!(gain_controls(&ada_page).await, json!([true, "100"]));
        let meter = ada_page
            .browser
            .wait_for(
                "the meter reads the microphone",
                "const m = document.querySelector('.murmure-call-panel .murmure-call-meter'); \
                 return m && m.value > 0 ? [m.tagName, m.getAttribute('aria-label')] : null;",
                STEP,
            )
            .await;
        assert_eq!(meter, json!(["METER", "Microphone level"]));

        // A gain of nothing by hand. The microphone opens again with the automatic gain off,
        // and with the echo canceller and the noise filter on (`CALL-089`, `CALL-091`).
        slide(&ada_page, "Microphone gain", "0").await;
        let manual = capture(&ada_page, 2).await;
        assert_eq!(manual["audio"]["autoGainControl"], json!(false), "{manual}");
        assert_eq!(manual["audio"]["echoCancellation"], json!(true), "{manual}");
        assert_eq!(manual["audio"]["noiseSuppression"], json!(true), "{manual}");
        assert!(tracks_taken(&ada_page, 2).await, "the transform changed");
        assert_eq!(gain_controls(&ada_page).await, json!([false, "0"]));
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let silent = window(&ada_page, &bob_page).await;
        // The gain scales what Ada sends (`CALL-090`).
        assert!(
            silent.energy < live.energy / 100.0,
            "Bob hears Ada through a gain of nothing: {silent:?} against {live:?}"
        );
        // The same rate and size, and no renegotiation (`CALL-092`, `ZK-027`). Every sealed
        // chunk has the fixed size, so the bytes of a window count its chunks. A padding packet
        // of the congestion control carries no chunk, so the rate counts chunks, not packets.
        let same = |w: &Window| {
            assert!(
                w.chunks() >= live.chunks() * 0.9,
                "a gain changed the chunk rate: {w:?} against {live:?}"
            );
        };
        same(&silent);

        // A higher gain brings Ada back, decoded whole (`CALL-090`).
        slide(&ada_page, "Microphone gain", "150").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let loud = window(&ada_page, &bob_page).await;
        assert!(
            loud.energy > silent.energy * 100.0 && loud.energy > 0.0,
            "Bob hears nothing of Ada at a gain: {loud:?} against {silent:?}"
        );
        assert!(loud.decoded >= 48_000.0 * 2.0, "{loud:?}");
        same(&loud);

        // Under the gain, a mute silences Ada (`CALL-093`).
        click_header(&ada_page, ".murmure-call-mute").await;
        mute_pressed(&ada_page, "true").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let muted = window(&ada_page, &bob_page).await;
        assert!(
            muted.energy < loud.energy / 100.0,
            "Bob hears Ada through the mute: {muted:?} against {loud:?}"
        );
        click_header(&ada_page, ".murmure-call-mute").await;
        mute_pressed(&ada_page, "false").await;

        // A change of microphone keeps the gain. The new capture has the automatic gain off and
        // feeds the same graph, so the sender takes no new track (`CALL-094`).
        pick(&ada_page, ".murmure-call-microphone", 1).await;
        let changed = capture(&ada_page, 3).await;
        assert_eq!(
            changed["audio"]["autoGainControl"],
            json!(false),
            "{changed}"
        );
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        assert_eq!(gain_controls(&ada_page).await, json!([false, "150"]));
        let kept = window(&ada_page, &bob_page).await;
        assert!(
            kept.energy > muted.energy * 100.0 && kept.energy > 0.0,
            "Bob hears nothing after the change: {kept:?}"
        );
        let taken = ada_page
            .browser
            .script("return window.beltTransforms.length;", json!([]))
            .await;
        assert_eq!(
            taken,
            json!(2),
            "the change of microphone replaced the track"
        );

        // The arrow keys move the gain (`MURMURE-617`).
        let focused = ada_page
            .browser
            .script(
                "const g = document.querySelector('.murmure-call-panel .murmure-call-gain'); \
                 g.focus(); return [g.getAttribute('aria-label'), document.activeElement === g];",
                json!([]),
            )
            .await;
        assert_eq!(focused, json!(["Microphone gain", true]));
        press(&ada_page, &["\u{E012}"]).await;
        assert_eq!(gain_controls(&ada_page).await, json!([false, "145"]));

        // The automatic gain comes back by its checkbox: the microphone opens again with the
        // automatic gain on, and the sender takes the capture (`CALL-089`).
        let checked = ada_page
            .browser
            .script(
                "const a = document.querySelector('.murmure-call-panel .murmure-call-automatic-gain'); \
                 a.click(); return true;",
                json!([]),
            )
            .await;
        assert_eq!(checked, json!(true));
        let automatic = capture(&ada_page, 4).await;
        assert_eq!(
            automatic["audio"]["autoGainControl"],
            json!(true),
            "{automatic}"
        );
        assert!(tracks_taken(&ada_page, 3).await, "the transform changed");
        let after = ada_page
            .browser
            .script(
                "return [window.beltPcs.length, window.beltWorkers, window.beltDescriptions];",
                json!([]),
            )
            .await;
        assert_eq!(after, before, "a gain renegotiated or spawned (`CALL-092`)");
        assert_eq!(
            ada_page
                .browser
                .script("return localStorage.getItem('ambre.call.gain');", json!([]))
                .await,
            Value::Null
        );
        click_header(&ada_page, ".murmure-call-hang-up").await;
        click_header(&bob_page, ".murmure-call-hang-up").await;
    }

    /// The video a page sends and decodes. It gives the width and the frames encoded of each
    /// encoding it sends. It gives the encodings its video sender asks, and the width of its
    /// camera. It gives the width and the frames decoded of each video it receives, by stats
    /// id, and whether each video receiver holds a transform.
    pub(crate) async fn video_stats(page: &Page) -> Value {
        page.browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 const pc = (window.beltPcs || [])[0]; \
                 if (!pc) { done({ sent: [], received: [], opened: false }); return; } \
                 pc.getStats().then((stats) => { \
                   const video = pc.getSenders().find((s) => s.track && s.track.kind === 'video'); \
                   const out = { sent: [], received: [], tracks: window.beltTracks, \
                     asked: video ? video.getParameters().encodings.map((e) => \
                       [e.rid, e.scaleResolutionDownBy, e.maxBitrate]) : [], \
                     camera: video ? video.track.getSettings().width || 0 : 0, \
                     opened: pc.getTransceivers().filter((t) => t.receiver.track.kind === 'video' \
                       && /recv/.test(t.currentDirection || '')).every((t) => !!t.receiver.transform) }; \
                   stats.forEach((s) => { \
                     if (s.kind !== 'video') return; \
                     if (s.type === 'outbound-rtp') out.sent.push({ rid: s.rid || null, \
                       width: s.frameWidth || 0, frames: s.framesEncoded || 0, \
                       limit: s.qualityLimitationReason || null }); \
                     if (s.type === 'inbound-rtp' && s.framesDecoded) out.received.push({ \
                       id: s.id, width: s.frameWidth || 0, frames: s.framesDecoded }); \
                   }); \
                   done(out); }, (e) => done({ error: String(e) }));",
                json!([]),
            )
            .await
    }

    /// The width the camera opens at: the top size of the call video row.
    const TOP_WIDTH: u64 = 1280;

    /// The width `page` encodes its layer `rid` at now, 0 before its first frame. A headless
    /// engine whose encoder lacks the CPU scales the camera down, so every layer shrinks
    /// together (the call video row of `ambre-spec-budgets.md section 1`).
    async fn sent_width(page: &Page, rid: &str) -> u64 {
        video_stats(page).await["sent"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["rid"].as_str() == Some(rid))
            .and_then(|s| s["width"].as_u64())
            .unwrap_or(0)
    }

    /// Waits until `page` decodes exactly two videos: the top layer of `top`, and the lowest
    /// layer of `low`. Each reads the width its sender encodes it at now. Each grew past `frames`
    /// more pictures than `before` held. Answers the stats id of the top layer.
    async fn top_and_low(
        page: &Page,
        before: &Value,
        frames: u64,
        top: &Page,
        low: &Page,
    ) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let (top_width, low_width) = (sent_width(top, "h").await, sent_width(low, "l").await);
            let stats = video_stats(page).await;
            let received = stats["received"].as_array().cloned().unwrap_or_default();
            let grew = |entry: &Value| {
                let id = entry["id"].as_str().unwrap_or_default();
                let was = before["received"]
                    .as_array()
                    .and_then(|r| r.iter().find(|b| b["id"].as_str() == Some(id)))
                    .and_then(|b| b["frames"].as_u64())
                    .unwrap_or(0);
                entry["frames"].as_u64().unwrap_or(0) >= was + frames
            };
            let at = |width: u64| {
                received
                    .iter()
                    .filter(|r| width > 0 && r["width"].as_u64() == Some(width) && grew(r))
                    .collect::<Vec<_>>()
            };
            let (top, low) = (at(top_width), at(low_width));
            let apart = top_width != low_width || top.len() == 2;
            if received.len() == 2 && !top.is_empty() && !low.is_empty() && apart {
                assert_eq!(
                    stats["opened"],
                    json!(true),
                    "a video without its transform"
                );
                return top
                    .first()
                    .and_then(|t| t["id"].as_str())
                    .unwrap_or_default()
                    .to_owned();
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "within 60 s, the page decoded {stats:#}, not the top layer at \
                     {top_width} and the lowest at {low_width}, and logged {:#?}",
                    page.browser.console().await
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    /// `CALL-008`, `CALL-096`, `CALL-099` and the video half of `CALL-001`, `CALL-011` and
    /// `CALL-022`, in the browser.
    ///
    /// Ada, Bob and Carol join one call with a fake camera. Each sends three VP8 encodings at
    /// three sizes on one sender, over its one peer connection. While Ada alone speaks, Bob and
    /// Carol each decode her video at the top size and the other's at the lowest. Once Carol
    /// alone speaks, the top size moves to her video at Bob. Each video sender took its transform
    /// before the camera, and each video receiver opens through the transform of the call worker.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn three_members_decode_each_others_video_at_the_size_the_sfu_chooses() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob, carol) = (user(), user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        let carol_page = murmure(Arc::clone(&relay), &carol).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        let link = invite_into_space(&ada_page, &carol).await;
        follow(&carol_page, &link).await;
        open(&bob_page, "general").await;
        open(&carol_page, "general").await;
        open(&ada_page, "general").await;
        let channel = last_channel();
        let pages = [&ada_page, &bob_page, &carol_page];
        for page in pages {
            page.browser.script(RECORD, json!([])).await;
        }
        let call = ada_page
            .browser
            .script(
                "return window.wasmBindings.start_video_call(arguments[0]);",
                json!([channel]),
            )
            .await;
        let call = call.as_str().unwrap_or_default().to_owned();
        assert!(!call.is_empty(), "the page started no call");
        read_until(&bob_page, "a live call", |h| h.contains(LIVE), ROSTER_WAIT).await;
        for page in [&bob_page, &carol_page] {
            let joined = page
                .browser
                .script(
                    "window.wasmBindings.join_video_call(arguments[0], arguments[1]); \
                     return true;",
                    json!([call, channel]),
                )
                .await;
            assert_eq!(joined, json!(true), "the page has no join");
        }
        decoded_past(&pages, &[0.0; 3], 48_000.0).await;

        // Each sender sends three encodings at three sizes, on one sender of one peer
        // connection, and took the camera after its transform (`CALL-008`, `CALL-011`,
        // `CALL-096`).
        for page in pages {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            let sent = loop {
                let stats = video_stats(page).await;
                // The sizes keep their ratio when a starved encoder scales the camera down.
                let width = |rid: &str| {
                    stats["sent"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|s| s["rid"].as_str() == Some(rid))
                        .filter(|s| s["frames"].as_u64().is_some_and(|f| f > 0))
                        .and_then(|s| s["width"].as_u64())
                        .unwrap_or(0)
                };
                let (l, m, h) = (width("l"), width("m"), width("h"));
                if l > 0 && m == 2 * l && h == 4 * l {
                    break stats;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "within 30 s, the page sent {stats:#}"
                );
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            };
            assert_eq!(sent["tracks"], json!([true, true]), "{sent:#}");
            // The sizes and caps the sender asks are those of the call video row.
            assert_eq!(
                sent["asked"],
                json!([["l", 4, 150_000], ["m", 2, 500_000], ["h", 1, 1_500_000]]),
                "{sent:#}"
            );
            assert_eq!(sent["camera"], json!(TOP_WIDTH), "{sent:#}");
            let stats = call_stats(page).await;
            assert_eq!(stats["pcs"], json!(1), "one peer connection: {stats:#}");
            let sdp = stats["sdp"].as_str().unwrap_or_default();
            assert!(sdp.contains("VP8"), "no VP8 offered: {sdp}");
        }

        // Ada alone speaks: the others decode her at the top size, and each other at the lowest
        // (`CALL-099`).
        set_mute(&bob_page, "true").await;
        set_mute(&carol_page, "true").await;
        let bob_before = video_stats(&bob_page).await;
        let carol_before = video_stats(&carol_page).await;
        let ada_at_bob = top_and_low(&bob_page, &bob_before, 30, &ada_page, &carol_page).await;
        top_and_low(&carol_page, &carol_before, 30, &ada_page, &bob_page).await;

        // Carol alone speaks: the top size moves to her video at Bob.
        set_mute(&ada_page, "true").await;
        set_mute(&carol_page, "false").await;
        let bob_before = video_stats(&bob_page).await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let carol_at_bob =
                top_and_low(&bob_page, &bob_before, 30, &carol_page, &ada_page).await;
            if carol_at_bob != ada_at_bob {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "within 60 s, the top size stayed on Ada's video at Bob"
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        for page in pages {
            click_header(page, ".murmure-call-hang-up").await;
        }
    }

    /// Hands each worker the page spawns to `window.beltWorkers`, with the address of its
    /// script. The DevTools Protocol runs it before the script of each document.
    const WORKERS: &str = "window.beltSpawned = []; const W = window.Worker; \
        window.Worker = class extends W { constructor(url, ...a) { super(url, ...a); \
          window.beltSpawned.push([String(url), this]); } };";

    /// A page of Murmure signed in as `name`, whose workers [`WORKERS`] records from the boot.
    async fn murmure_with_workers(relay: Arc<Relay>, name: &str) -> Page {
        let page = Page::home(relay).await;
        page.sign_in(name).await;
        page.browser
            .cdp(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": WORKERS }),
            )
            .await;
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

    /// Hands the page `page` the event `event`, as its Sync worker posts one.
    async fn from_sync(page: &Page, event: &ambre_bus::LogicEvent) {
        let bytes = postcard::to_allocvec(event).expect("an event encodes");
        let handed = page
            .browser
            .script(
                "const sync = window.beltSpawned.filter(([url]) => /sync/i.test(url)); \
                 if (sync.length !== 1) return sync.length; \
                 sync[0][1].dispatchEvent(new MessageEvent('message', \
                   { data: new Uint8Array(arguments[0]).buffer })); \
                 return true;",
                json!([bytes]),
            )
            .await;
        assert_eq!(handed, json!(true), "the page spawned one Sync worker");
    }

    /// The first byte of a command of the bus that chooses videos.
    fn choice_tag() -> u8 {
        let choose = ambre_bus::SyncCommand::ChooseVideos(Default::default());
        postcard::to_allocvec(&choose)
            .ok()
            .and_then(|bytes| bytes.first().copied())
            .expect("a choice encodes")
    }

    /// Each choice of videos `page` posted to a worker, from post `from` on, oldest first.
    async fn choices(page: &Page, from: u64) -> Vec<ambre_protocol::signaling::VideoChoice> {
        let posted = page
            .browser
            .script(
                "return window.beltPosted.slice(arguments[0]).map((p) => Array.from(p));",
                json!([from]),
            )
            .await;
        let tag = choice_tag();
        posted
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|post| serde_json::from_value::<Vec<u8>>(post.clone()).ok())
            .filter(|bytes| bytes.first() == Some(&tag))
            .filter_map(|bytes| match postcard::from_bytes(&bytes) {
                Ok(ambre_bus::SyncCommand::ChooseVideos(choice)) => Some(choice),
                _ => None,
            })
            .collect()
    }

    /// Waits until the latest choice of videos of `page` holds, by `holds`, and answers it.
    async fn chosen(
        page: &Page,
        what: &str,
        holds: impl Fn(&ambre_protocol::signaling::VideoChoice) -> bool,
    ) -> ambre_protocol::signaling::VideoChoice {
        let deadline = std::time::Instant::now() + ROSTER_WAIT;
        loop {
            let all = choices(page, 0).await;
            if let Some(last) = all.last().filter(|c| holds(c)) {
                return last.clone();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "within {ROSTER_WAIT:?}, no choice of {what}: {all:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// The first byte of a command of the bus that sends a presence sample, which no history
    /// keeps.
    fn sample_tag() -> u64 {
        let sample = ambre_bus::SyncCommand::SendEphemeral {
            document_id: ambre_core::id::Id::mint(),
            payload: ambre_core::Payload::new(Vec::new()),
        };
        postcard::to_allocvec(&sample)
            .ok()
            .and_then(|bytes| bytes.first().copied())
            .map(u64::from)
            .expect("a sample encodes")
    }

    /// The sender indexes `from`, then each after it up to `to` included.
    fn senders(from: u64, to: u64) -> Vec<ambre_core::Seq<ambre_core::SenderIndex>> {
        (from..=to).map(ambre_core::Seq::new).collect()
    }

    /// `MURMURE-654`, `MURMURE-667`, `MURMURE-668` and `MURMURE-670`, in the browser.
    ///
    /// Ada, Bob and Carol join one call with a fake camera. Ada's page then hears ten more members
    /// admitted, whom no seat of the SFU carries, so her grid holds two pages. Each choice she
    /// posts names the senders of the tiles her grid shows, in its order. A turn of the page posts
    /// the senders of the next page alone.
    ///
    /// Ada pins Bob from her list of the participants. Her choice names him first and large, and
    /// she decodes his video at the top size while Carol alone speaks. Her turns and her pin post
    /// nothing else but the roster ask.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "records the workers through the DevTools Protocol, which Firefox does not speak"
    )]
    async fn a_page_switch_and_a_pin_choose_the_videos_the_grid_shows() {
        use ambre_bus::{CallSignal, LogicEvent};
        use ambre_core::id::{Id, User};
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob, carol) = (user(), user(), user());
        let ada_page = murmure_with_workers(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        let carol_page = murmure(Arc::clone(&relay), &carol).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        let link = invite_into_space(&ada_page, &carol).await;
        follow(&carol_page, &link).await;
        open(&bob_page, "general").await;
        open(&carol_page, "general").await;
        open(&ada_page, "general").await;
        let channel = last_channel();
        let pages = [&ada_page, &bob_page, &carol_page];
        for page in pages {
            page.browser.script(RECORD, json!([])).await;
        }
        ada_page.browser.script(DEVICES, json!([])).await;
        let call = ada_page
            .browser
            .script(
                "return window.wasmBindings.start_video_call(arguments[0]);",
                json!([channel]),
            )
            .await;
        let call = call.as_str().unwrap_or_default().to_owned();
        assert!(!call.is_empty(), "the page started no call");
        read_until(&bob_page, "a live call", |h| h.contains(LIVE), ROSTER_WAIT).await;
        for page in [&bob_page, &carol_page] {
            page.browser
                .script(
                    "window.wasmBindings.join_video_call(arguments[0], arguments[1]); \
                     return true;",
                    json!([call, channel]),
                )
                .await;
        }
        decoded_past(&pages, &[0.0; 3], 48_000.0).await;

        // Two others take one page: the choice names both, and nobody large (`MURMURE-667`).
        let both = chosen(&ada_page, "Bob and Carol", |c| c.senders().len() == 2).await;
        assert_eq!(both.large(), None, "{both:?}");
        let real = both.senders().to_vec();

        // Ten members more: the first page holds the nine admitted first (`MURMURE-654`).
        for index in 1_000..1_010 {
            let admitted = LogicEvent::Call(CallSignal::Admitted {
                sender: ambre_core::Seq::new(index),
                member: Id::<User>::mint(),
            });
            from_sync(&ada_page, &admitted).await;
        }
        let mut first: Vec<_> = real.clone();
        first.extend(senders(1_000, 1_006));
        let first_page = chosen(&ada_page, "the first page", |c| c.senders().len() == 9).await;
        assert_eq!(first_page.senders(), first.as_slice(), "{first_page:?}");
        assert_eq!(first_page.large(), None);

        // A turn of the page asks for the senders of the next page alone.
        let posted = ada_page
            .browser
            .script("return window.beltPosted.length;", json!([]))
            .await
            .as_u64()
            .unwrap_or_default();
        click_header(&ada_page, ".murmure-call-page-next").await;
        let next = senders(1_007, 1_009);
        chosen(&ada_page, "the second page", |c| {
            c.senders() == next.as_slice()
        })
        .await;
        click_header(&ada_page, ".murmure-call-page-previous").await;
        chosen(&ada_page, "the first page again", |c| {
            c.senders() == first.as_slice()
        })
        .await;

        // Ada pins Bob from her list: he leads the choice, large, and the page holds eight
        // more (`MURMURE-668`).
        click_header(&ada_page, ".murmure-call-roster").await;
        let pinned = ada_page
            .browser
            .wait_for(
                "the pin button of Bob",
                &format!(
                    "const item = [...document.querySelectorAll('.murmure-call-roster-list li')] \
                       .find((li) => li.textContent === {}); \
                     const pin = item && item.querySelector('.murmure-call-pin'); \
                     if (!pin) return null; pin.click(); return true;",
                    Value::String(bob.clone())
                ),
                STEP,
            )
            .await;
        assert_eq!(pinned, json!(true));
        let with_pin = chosen(&ada_page, "Bob pinned", |c| c.large().is_some()).await;
        let bob_sender = with_pin.large().expect("a large sender");
        assert!(real.contains(&bob_sender), "{with_pin:?}");
        let carol_sender = real.iter().copied().find(|s| *s != bob_sender);
        let mut expected = vec![bob_sender];
        expected.extend(carol_sender);
        expected.extend(senders(1_000, 1_006));
        assert_eq!(with_pin.senders(), expected.as_slice());

        // A turn and a pin post a choice of videos, and nothing else but the roster ask. The
        // presence sample each page sends at its interval passes too: no history keeps it
        // (`MURMURE-670`).
        let (tag, roster, sample) = (u64::from(choice_tag()), roster_tag(), sample_tag());
        let posts = ada_page
            .browser
            .script(
                "return window.beltPosted.slice(arguments[0]).map((p) => p[0]);",
                json!([posted]),
            )
            .await;
        let others: Vec<&Value> = posts
            .as_array()
            .into_iter()
            .flatten()
            .filter(|first| ![Some(tag), Some(roster), Some(sample)].contains(&first.as_u64()))
            .collect();
        assert!(others.is_empty(), "the turns and the pin posted {others:?}");

        // Carol alone speaks, and Ada still decodes Bob at the top size (`MURMURE-668`,
        // `CALL-128`).
        set_mute(&ada_page, "true").await;
        set_mute(&bob_page, "true").await;
        let before = video_stats(&ada_page).await;
        top_and_low(&ada_page, &before, 30, &bob_page, &carol_page).await;
        for page in pages {
            click_header(page, ".murmure-call-hang-up").await;
        }
    }

    /// The pixels of the channel canvas of `page`, as a screenshot shows them, cut to the canvas.
    /// A screenshot reads what the GPU drew. Every element but the canvas hides during the shot,
    /// so a toast or a menu over the canvas reads as nothing.
    async fn canvas_shot(page: &Page) -> image::RgbImage {
        let rect = page
            .browser
            .script(
                "const s = document.createElement('style'); s.id = 'belt-canvas-alone'; \
                 s.textContent = '* { visibility: hidden !important; } ' + arguments[0] + \
                   ' { visibility: visible !important; }'; \
                 document.head.append(s); \
                 const r = document.querySelector(arguments[0]).getBoundingClientRect(); \
                 return [r.left, r.top, r.width, r.height, window.devicePixelRatio || 1];",
                json!([CHANNEL]),
            )
            .await;
        let at = |i: usize| rect[i].as_f64().unwrap_or(0.0);
        let ratio = at(4);
        let png = page.browser.screenshot().await;
        page.browser
            .script(
                "document.getElementById('belt-canvas-alone')?.remove(); return true;",
                json!([]),
            )
            .await;
        let shot = image::load_from_memory(&png)
            .expect("a PNG screenshot")
            .to_rgb8();
        let (x, y) = ((at(0) * ratio) as u32, (at(1) * ratio) as u32);
        let width = ((at(2) * ratio) as u32).min(shot.width().saturating_sub(x));
        let height = ((at(3) * ratio) as u32).min(shot.height().saturating_sub(y));
        image::imageops::crop_imm(&shot, x, y, width, height).to_image()
    }

    /// The pixels two shots of one canvas differ by, and the box that holds them, as left, top,
    /// right and bottom. A pixel differs once its three channels moved by more than 6 together.
    /// A shot of a still canvas reads the same bytes, and the fake camera of Firefox is one
    /// colour that shifts its shade slowly.
    fn changed(a: &image::RgbImage, b: &image::RgbImage) -> (u64, Option<[u32; 4]>) {
        let mut count = 0;
        let mut bounds: Option<[u32; 4]> = None;
        for (x, y, pa) in a.enumerate_pixels() {
            let Some(pb) = b.get_pixel_checked(x, y) else {
                continue;
            };
            let moved: i32 =
                pa.0.iter()
                    .zip(pb.0.iter())
                    .map(|(p, q)| (i32::from(*p) - i32::from(*q)).abs())
                    .sum();
            if moved > 6 {
                count += 1;
                bounds = Some(match bounds {
                    None => [x, y, x, y],
                    Some([l, t, r, d]) => [l.min(x), t.min(y), r.max(x), d.max(y)],
                });
            }
        }
        (count, bounds)
    }

    /// The pixels two shots of one canvas differ by in the rows above `limit`, as [`changed`]
    /// counts a moved pixel.
    fn moved_above(a: &image::RgbImage, b: &image::RgbImage, limit: u32) -> u64 {
        a.enumerate_pixels()
            .filter(|(_, y, _)| *y < limit)
            .filter(|(x, y, pa)| {
                b.get_pixel_checked(*x, *y).is_some_and(|pb| {
                    let moved: i32 =
                        pa.0.iter()
                            .zip(pb.0.iter())
                            .map(|(p, q)| (i32::from(*p) - i32::from(*q)).abs())
                            .sum();
                    moved > 6
                })
            })
            .count() as u64
    }

    /// The `ambre:video-paint` measures of `page` so far, in milliseconds.
    async fn video_paints(page: &Page) -> Vec<f64> {
        page.browser
            .script(
                "return performance.getEntriesByName('ambre:video-paint').map((e) => e.duration);",
                json!([]),
            )
            .await
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .collect()
    }

    /// The count of video elements in the page of `page`.
    async fn video_elements(page: &Page) -> Value {
        page.browser
            .script(
                "return document.querySelectorAll('video').length;",
                json!([]),
            )
            .await
    }

    /// The `aria-pressed` of the camera button, once it shows.
    async fn camera_pressed(page: &Page, pressed: &str) {
        let pressed = Value::String(pressed.to_owned());
        page.browser
            .wait_for(
                &format!("camera pressed {pressed}"),
                &format!(
                    "const b = document.querySelector('.murmure-channel-header .murmure-call-camera'); \
                     return !!b && !b.disabled && b.getAttribute('aria-pressed') === {pressed};"
                ),
                STEP,
            )
            .await;
    }

    /// What the video sender of `page` holds. The answer gives the local descriptions the page
    /// set, whether the sender holds a track, and whether it holds its first transform.
    async fn video_sender(page: &Page) -> Value {
        page.browser
            .script(
                "const pc = (window.beltPcs || [])[0]; \
                 const t = pc.getTransceivers().find((t) => t.direction === 'sendonly'); \
                 if (!window.beltVideoTransform) window.beltVideoTransform = t.sender.transform; \
                 return [window.beltDescriptions, !!t.sender.track, \
                   !!t.sender.transform && t.sender.transform === window.beltVideoTransform];",
                json!([]),
            )
            .await
    }

    /// `CALL-015`, `CALL-100`, `MURMURE-620` to `MURMURE-623` and `MURMURE-625`, in the browser.
    ///
    /// Ada calls Bob from the header of their channel, and Bob joins from the call message. Ada
    /// mutes, so no outline of a voice moves. No video arrives, and two readbacks of Bob's
    /// canvas read the same.
    ///
    /// Ada turns her camera on by the camera button. Bob's canvas draws her video. Two readbacks
    /// two seconds apart differ in one band as the fake camera moves, and no video element enters
    /// either page. Ada's sender took the camera with its transform, set no local description and
    /// posted nothing.
    ///
    /// Ada unmutes and turns her camera off, and her sender holds no track. Bob's browser refuses his
    /// camera: his header says why, and he still hears Ada.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_member_draws_the_video_of_another_on_its_canvas() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        open(&bob_page, "general").await;
        open(&ada_page, "general").await;
        for page in [&ada_page, &bob_page] {
            page.browser.script(RECORD, json!([])).await;
            page.browser.script(DEVICES, json!([])).await;
        }
        click_header(&ada_page, ".murmure-call-start").await;
        join_from_message(&bob_page).await;
        let heard = decoded_past(&[&ada_page, &bob_page], &[0.0; 2], 48_000.0).await;
        // The call message names both members once the roster settles, so the list holds still.
        read_until(
            &bob_page,
            "Ada and Bob in the call",
            |h| in_call(h, &[&ada, &bob]),
            ROSTER_WAIT,
        )
        .await;
        // No key of Bob drives his canvas: it shows what arrives.
        bob_page
            .browser
            .script("document.activeElement?.blur(); return true;", json!([]))
            .await;
        // The fake microphone of Chrome beeps, and Bob's canvas outlines Ada's tile at each
        // beep (`MURMURE-653`). Ada mutes until her video is read, so no pixel moves for her
        // voice. The 1 s that follows outlasts the hold of the outline.
        click_header(&ada_page, ".murmure-call-mute").await;
        mute_pressed(&ada_page, "true").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // No video arrives: the canvas holds still (`MURMURE-621`).
        let still = canvas_shot(&bob_page).await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let (moved, bounds) = changed(&still, &canvas_shot(&bob_page).await);
        assert!(
            moved < 50,
            "{moved} pixels moved with no video, in {bounds:?}"
        );
        assert!(video_paints(&bob_page).await.is_empty());

        // Ada turns her camera on: her sender takes it by a replace, under its transform, and
        // nothing renegotiates or reaches a worker (`CALL-100`, `MURMURE-622`, `MURMURE-625`).
        let before = video_sender(&ada_page).await;
        assert_eq!(
            before[1],
            json!(false),
            "a camera before the press: {before}"
        );
        let posted = ada_page
            .browser
            .script("return window.beltPosted.length;", json!([]))
            .await;
        click_header(&ada_page, ".murmure-call-camera").await;
        camera_pressed(&ada_page, "true").await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let after = loop {
            let after = video_sender(&ada_page).await;
            if after[1] == json!(true) || std::time::Instant::now() >= deadline {
                break after;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        };
        assert_eq!(
            after,
            json!([before[0], true, true]),
            "{before} then {after}"
        );
        let tag = roster_tag();
        let posts = ada_page
            .browser
            .script(
                "return window.beltPosted.slice(arguments[0]).map((p) => p[0]);",
                json!([posted]),
            )
            .await;
        let posts: Vec<&Value> = posts
            .as_array()
            .into_iter()
            .flatten()
            .filter(|tag_of| tag_of.as_u64() != Some(tag))
            .collect();
        assert!(posts.is_empty(), "the camera posted {posts:?}");

        // Bob's canvas draws Ada's video: two readbacks differ, in one band (`CALL-015`,
        // `MURMURE-620`).
        bob_page
            .browser
            .wait_for(
                "thirty paints of a video",
                "return performance.getEntriesByName('ambre:video-paint').length >= 30;",
                std::time::Duration::from_secs(40),
            )
            .await;
        let first = canvas_shot(&bob_page).await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let second = canvas_shot(&bob_page).await;
        let (moved, bounds) = changed(&first, &second);
        let ratio = bob_page
            .browser
            .script("return window.devicePixelRatio || 1;", json!([]))
            .await
            .as_f64()
            .unwrap_or(1.0);
        assert!(moved >= 1_000, "only {moved} pixels moved under a video");
        let [_, top, _, bottom] = bounds.expect("a box of moved pixels");
        assert!(
            f64::from(bottom - top) <= 180.0 * ratio,
            "the moved pixels span {top} to {bottom}, past the band"
        );
        for page in [&ada_page, &bob_page] {
            assert_eq!(
                video_elements(page).await,
                json!(0),
                "a video element in the page"
            );
        }
        let mut paints = video_paints(&bob_page).await;
        paints.sort_by(f64::total_cmp);
        let at = |q: f64| paints[((paints.len() as f64 - 1.0) * q).round() as usize];
        eprintln!(
            "video paints on {:?}: {} frames, median {:.2} ms, p95 {:.2} ms, max {:.2} ms",
            bob_page.browser.engine(),
            paints.len(),
            at(0.5),
            at(0.95),
            at(1.0)
        );

        // Ada speaks again: the video check above read her video alone.
        click_header(&ada_page, ".murmure-call-mute").await;
        mute_pressed(&ada_page, "false").await;

        // Ada turns her camera off: her sender gives it up, and nothing renegotiates
        // (`MURMURE-622`, `CALL-100`).
        click_header(&ada_page, ".murmure-call-camera").await;
        camera_pressed(&ada_page, "false").await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let off = video_sender(&ada_page).await;
            if off[1] == json!(false) {
                assert_eq!(off, json!([before[0], false, true]), "{before} then {off}");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the sender kept the camera: {off}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // Bob's browser refuses his camera: the header says why, and the call goes on
        // (`MURMURE-623`).
        bob_page
            .browser
            .script(
                "const G = MediaDevices.prototype.getUserMedia; \
                 MediaDevices.prototype.getUserMedia = function (c) { \
                   return c && c.video ? Promise.reject(new DOMException('refused by the belt', \
                     'NotAllowedError')) : G.call(this, c); }; return true;",
                json!([]),
            )
            .await;
        click_header(&bob_page, ".murmure-call-camera").await;
        let notice = bob_page
            .browser
            .wait_for(
                "the refusal of the camera",
                "const n = document.querySelector('.murmure-channel-header .murmure-call-camera-refusal'); \
                 return n ? [n.getAttribute('role'), n.textContent] : null;",
                STEP,
            )
            .await;
        assert_eq!(
            notice,
            json!(["alert", "Camera blocked: the call goes on with its audio"])
        );
        camera_pressed(&bob_page, "false").await;
        let decoded = |stats: &Value| stats["decoded"].as_f64().unwrap_or(0.0);
        decoded_past(&[&bob_page], &[decoded(&heard[1])], 96_000.0).await;
        for page in [&ada_page, &bob_page] {
            click_header(page, ".murmure-call-hang-up").await;
        }
    }

    /// The pixels of the canvas `selector`, cut out of a screenshot of the whole page. The shot
    /// hides nothing, so no element loses the focus to it.
    async fn shot_of(page: &Page, selector: &str) -> image::RgbImage {
        let rect = page
            .browser
            .script(
                "const r = document.querySelector(arguments[0]).getBoundingClientRect(); \
                 return [r.left, r.top, r.width, r.height, window.devicePixelRatio || 1];",
                json!([selector]),
            )
            .await;
        let at = |i: usize| rect[i].as_f64().unwrap_or(0.0);
        let ratio = at(4);
        let shot = image::load_from_memory(&page.browser.screenshot().await)
            .expect("a PNG screenshot")
            .to_rgb8();
        let (x, y) = ((at(0) * ratio) as u32, (at(1) * ratio) as u32);
        let width = ((at(2) * ratio) as u32).min(shot.width().saturating_sub(x));
        let height = ((at(3) * ratio) as u32).min(shot.height().saturating_sub(y));
        image::imageops::crop_imm(&shot, x, y, width, height).to_image()
    }

    /// `CANVAS-093`, `CANVAS-094` and `CANVAS-010`, in the browser, over the content canvas and
    /// a cell (`CANVAS-070`).
    ///
    /// A channel and its thread show, and the channel's composer holds the keys. Nothing
    /// changes for a second, and neither canvas draws. Two readbacks around that second read
    /// the same pixels, and those show the message. A key typed then draws on the next frame.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, a driver and the services; run just test-browser"
    )]
    async fn an_idle_channel_draws_nothing_and_holds_its_last_frame() {
        let page = murmure(Arc::new(Relay::start().await), &user()).await;
        prompt(&page, ".murmure-spaces", "murmure-new-space", "Idle").await;
        prompt(&page, ".murmure-space", "murmure-new-channel", "quiet").await;
        open(&page, "quiet").await;
        send(&page, CHANNEL, "Still life").await;
        open_thread(&page, "Still life").await;
        // The keys go back to the channel's composer, as a press gives them.
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let at = reachable(&page, CHANNEL, true).await;
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
                "the composer never took the keys"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        crate::frame::count_draws(&page).await;
        // The frames the press asked for, and the relay's answers, land first.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let before = shot_of(&page, CHANNEL).await;
        let drawn = crate::frame::draws_over_a_watch(&page).await;
        let after = shot_of(&page, CHANNEL).await;
        assert_eq!(drawn, 0, "an idle page drew");
        assert_eq!(changed(&before, &after), (0, None), "the idle canvas moved");
        let colours: std::collections::HashSet<[u8; 3]> = after.pixels().map(|p| p.0).collect();
        assert!(
            colours.len() > 2,
            "the canvas shows no frame: {} colours",
            colours.len()
        );

        press(&page, &["x"]).await;
        assert_eq!(
            crate::frame::late_frames_after_key(&page).await,
            0,
            "frames ran after the key before one drew"
        );
    }

    /// A screen whose picture moves, for a capture of the screen. Chrome captures a fake screen
    /// that moves, under `--use-fake-ui-for-media-stream`. Firefox captures the X display of its
    /// virtual screen, which holds still. So under Firefox the root window of that display takes
    /// a new colour every 250 ms, through `xsetroot`, until the guard drops. No colour comes back
    /// within a minute, so two readbacks seconds apart never read the same one.
    ///
    /// A screen that holds still reads as a share no canvas draws. So the guard fails the test at
    /// once when `xsetroot` cannot paint, and again when its loop stopped before a check. The
    /// virtual display refuses a connection now and then. So the loop skips a failed paint, and
    /// stops after four in a row: a second of a still screen.
    struct MovingScreen(Option<std::process::Child>);

    impl MovingScreen {
        fn start(engine: crate::driver::Engine) -> Self {
            if engine != crate::driver::Engine::Firefox {
                return Self(None);
            }
            assert!(
                std::env::var_os("DISPLAY").is_some(),
                "a capture of the screen under Firefox needs the X display of xvfb-run"
            );
            let painted = std::process::Command::new("xsetroot")
                .args(["-solid", "#000000"])
                .status();
            assert!(
                painted
                    .as_ref()
                    .is_ok_and(std::process::ExitStatus::success),
                "xsetroot cannot paint the screen of xvfb-run ({painted:?}): install \
                 x11-xserver-utils, as the Firefox job of CI does"
            );
            let child = std::process::Command::new("sh")
                .args([
                    "-c",
                    "i=0; f=0; while :; do i=$((i + 1)); \
                     if xsetroot -solid \"$(printf '#%02x%02x%02x' $((i * 37 % 256)) \
                       $((i * 91 % 256)) $((i * 53 % 256)))\"; then f=0; \
                     else f=$((f + 1)); [ $f -ge 4 ] && exit 1; fi; sleep 0.25; done",
                ])
                .spawn()
                .expect("xsetroot moves the virtual screen of Firefox");
            Self(Some(child))
        }

        /// Fails the test when the loop that moves the screen stopped: `xsetroot` lost the
        /// display, or it exited.
        fn check(&mut self) {
            if let Some(child) = self.0.as_mut() {
                let state = child.try_wait();
                assert!(
                    matches!(state, Ok(None)),
                    "the loop that moves the screen of xvfb-run stopped: {state:?}"
                );
            }
        }
    }

    impl Drop for MovingScreen {
        fn drop(&mut self) {
            if let Some(child) = self.0.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// Presses the control `selector` of the header of `page` with the pointer, as a hand does.
    /// The browser asks a capture of the screen to come from such a gesture, and a click a script
    /// dispatches is none.
    async fn press_header(page: &Page, selector: &str) {
        let selector = Value::String(format!(".murmure-channel-header {selector}"));
        let at = page
            .browser
            .wait_for(
                &format!("the control {selector}"),
                &format!(
                    "const b = document.querySelector({selector}); \
                     if (!b || b.disabled) return null; b.scrollIntoView(); \
                     const r = b.getBoundingClientRect(); \
                     return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)];"
                ),
                STEP,
            )
            .await;
        let (x, y) = (at[0].as_i64().unwrap_or(0), at[1].as_i64().unwrap_or(0));
        page.browser
            .perform(json!([{ "type": "pointer", "id": "mouse",
                "parameters": { "pointerType": "mouse" }, "actions": [
                    { "type": "pointerMove", "x": x, "y": y },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 },
                ] }]))
            .await;
    }

    /// The `aria-pressed` of the share button, once it shows and takes a press.
    async fn share_pressed(page: &Page, pressed: &str) {
        let pressed = Value::String(pressed.to_owned());
        page.browser
            .wait_for(
                &format!("share pressed {pressed}"),
                &format!(
                    "const b = document.querySelector('.murmure-channel-header .murmure-call-share'); \
                     return !!b && !b.disabled && b.getAttribute('aria-pressed') === {pressed};"
                ),
                STEP,
            )
            .await;
    }

    /// The notice of the share in the header of `page`: its role and its text.
    async fn share_notice(page: &Page) -> Value {
        page.browser
            .wait_for(
                "the notice of the share",
                "const n = document.querySelector('.murmure-channel-header .murmure-call-share-notice'); \
                 return n ? [n.getAttribute('role'), n.textContent] : null;",
                STEP,
            )
            .await
    }

    /// What the screen sender of `page` holds, once the first share added it. The answer gives the
    /// local descriptions the page set, whether the sender holds a track, and whether it holds its
    /// first transform. Then it
    /// gives its encodings, and the codec, the width and the frames of what it sent. The screen sender is the second send-only
    /// transceiver, after the camera.
    async fn screen_sender(page: &Page) -> Value {
        page.browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 const pc = (window.beltPcs || [])[0]; \
                 const t = pc.getTransceivers().filter((t) => t.direction === 'sendonly')[1]; \
                 if (!t) { done([window.beltDescriptions, false, false, [], null, 0, 0]); return; } \
                 if (!window.beltScreenTransform) window.beltScreenTransform = t.sender.transform; \
                 const encodings = t.sender.getParameters().encodings.map((e) => \
                   [e.rid || null, e.maxFramerate || null, e.maxBitrate || null, e.active !== false]); \
                 const head = [window.beltDescriptions, !!t.sender.track, \
                   !!t.sender.transform && t.sender.transform === window.beltScreenTransform, encodings]; \
                 if (!t.sender.track) { done(head.concat([null, 0, 0])); return; } \
                 t.sender.getStats().then((stats) => { \
                   let codec = null, width = 0, frames = 0; \
                   stats.forEach((s) => { if (s.type !== 'outbound-rtp') return; \
                     const c = stats.get(s.codecId); codec = c ? c.mimeType : codec; \
                     width = Math.max(width, s.frameWidth || 0); frames += s.framesEncoded || 0; }); \
                   done(head.concat([codec, width, frames])); }, (e) => done({ error: String(e) }));",
                json!([]),
            )
            .await
    }

    /// What the screen slot of `page` decoded: the frames, the width, and whether its receiver
    /// holds a transform. The screen slot is the tenth video receiver, after the nine video slots
    /// the SFU offers first.
    async fn screen_received(page: &Page) -> Value {
        page.browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 const pc = (window.beltPcs || [])[0]; \
                 const t = pc.getTransceivers().filter((t) => t.receiver.track.kind === 'video' \
                   && /recv/.test(t.currentDirection || ''))[9]; \
                 if (!t) { done([0, 0, false]); return; } \
                 t.receiver.getStats().then((stats) => { \
                   let frames = 0, width = 0; \
                   stats.forEach((s) => { if (s.type !== 'inbound-rtp') return; \
                     frames += s.framesDecoded || 0; width = Math.max(width, s.frameWidth || 0); }); \
                   done([frames, width, !!t.receiver.transform]); }, (e) => done({ error: String(e) }));",
                json!([]),
            )
            .await
    }

    /// Waits until the screen slot of `page` decoded `frames` more frames than `from`, and
    /// answers what it read then.
    async fn screen_decodes(page: &Page, from: f64, frames: f64) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        loop {
            let read = screen_received(page).await;
            if read[0].as_f64().unwrap_or(0.0) >= from + frames {
                return read;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the screen slot decoded {read} past {from}, not {frames} more"
            );
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    /// The frames each of the three camera encodings of `page` sent, once each one sent more
    /// than it had in `before` (none at first). Waits 30 s at most.
    async fn camera_layers(page: &Page, before: &Value) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let stats = video_stats(page).await;
            let frames = |rid: &str, of: &Value| {
                of["sent"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|s| s["rid"] == json!(rid))
                    .and_then(|s| s["frames"].as_u64())
                    .unwrap_or(0)
            };
            if ["l", "m", "h"]
                .iter()
                .all(|rid| frames(rid, &stats) > frames(rid, before) + 10)
            {
                return stats;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "within 30 s, the camera did not send its three encodings: {stats:#}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    /// Waits until `page` decodes thirty frames more of a video at the top width of the camera.
    async fn bob_decodes_top(page: &Page) {
        let top = |stats: &Value| -> u64 {
            stats["received"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["width"].as_u64() == Some(TOP_WIDTH))
                .filter_map(|r| r["frames"].as_u64())
                .sum()
        };
        let from = top(&video_stats(page).await);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let stats = video_stats(page).await;
            if top(&stats) >= from + 30 {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "within 30 s, no camera decoded at the top size during the share: {stats:#}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    /// `CALL-102`, `CALL-103`, `CALL-115`, `CALL-106` to `CALL-111`, `MURMURE-629` to
    /// `MURMURE-632`, `MURMURE-634`, `ZK-032` and `ZK-033`, in the browser.
    ///
    /// Ada calls Bob from the header of their channel, and Bob joins from the call message. Bob's
    /// browser refuses his capture: his header says why, his screen sender takes no track, and
    /// the call goes on. Ada presses the share button. Her screen sender takes the screen under
    /// its first transform, as one VP8 encoding capped at 30 fps and 1.5 Mbit/s. Her page sets no
    /// local description. Bob decodes it on his screen slot through its transform, and his canvas
    /// draws it in a tile taller than any tile of a camera.
    ///
    /// Bob then shares too. His share is the last one: Ada decodes his screen, her own share
    /// stops, and her header says that another member shares. Ada takes the screen back, and
    /// Bob's share stops the same way. Last, Ada's browser ends her track, as its bar does: her
    /// share stops, and nothing renegotiates.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
    )]
    async fn a_member_shares_its_screen_and_a_later_share_takes_it_over() {
        let sfu = crate::call::Sfu::start(None).await;
        let relay = Arc::new(Relay::start_with(&sfu.relay_env()).await);
        let (ada, bob) = (user(), user());
        let ada_page = murmure(Arc::clone(&relay), &ada).await;
        let bob_page = murmure(Arc::clone(&relay), &bob).await;
        let mut screen = MovingScreen::start(ada_page.browser.engine());
        space_with_channel(&ada_page, "Belt", "general", &bob, &bob_page).await;
        open(&bob_page, "general").await;
        open(&ada_page, "general").await;
        for page in [&ada_page, &bob_page] {
            page.browser.script(RECORD, json!([])).await;
            page.browser.script(DEVICES, json!([])).await;
        }
        click_header(&ada_page, ".murmure-call-start").await;
        join_from_message(&bob_page).await;
        let heard = decoded_past(&[&ada_page, &bob_page], &[0.0; 2], 48_000.0).await;
        read_until(
            &bob_page,
            "Ada and Bob in the call",
            |h| h.contains(&ada) && h.contains(&bob),
            ROSTER_WAIT,
        )
        .await;
        for page in [&ada_page, &bob_page] {
            page.browser
                .script("document.activeElement?.blur(); return true;", json!([]))
                .await;
        }

        // Bob's browser refuses his capture: the header says why, the call goes on, and his
        // screen sender takes nothing (`CALL-109`, `MURMURE-634`).
        bob_page
            .browser
            .script(
                "window.beltDisplay = MediaDevices.prototype.getDisplayMedia; \
                 MediaDevices.prototype.getDisplayMedia = () => Promise.reject( \
                   new DOMException('cancelled by the belt', 'NotAllowedError')); return true;",
                json!([]),
            )
            .await;
        press_header(&bob_page, ".murmure-call-share").await;
        assert_eq!(
            share_notice(&bob_page).await,
            json!([
                "alert",
                "Screen not shared: the share was refused or cancelled"
            ])
        );
        share_pressed(&bob_page, "false").await;
        let refused = screen_sender(&bob_page).await;
        assert_eq!(
            refused[1],
            json!(false),
            "a refused capture sent: {refused}"
        );
        let decoded = |stats: &Value| stats["decoded"].as_f64().unwrap_or(0.0);
        decoded_past(&[&bob_page], &[decoded(&heard[1])], 96_000.0).await;
        bob_page
            .browser
            .script(
                "MediaDevices.prototype.getDisplayMedia = window.beltDisplay; return true;",
                json!([]),
            )
            .await;

        // Ada turns her camera on, and her three encodings send (`CALL-096`).
        click_header(&ada_page, ".murmure-call-camera").await;
        camera_pressed(&ada_page, "true").await;
        let camera_before = camera_layers(&ada_page, &json!(null)).await;

        // Ada shares: her first share adds the screen sender by one offer. Her screen sender takes
        // the screen under its first transform, as one VP8 encoding. Nothing reaches a
        // worker but the start and that offer (`CALL-115`, `CALL-112`, `MURMURE-632`,
        // `MURMURE-631`, `ZK-032`).
        let before = screen_sender(&ada_page).await;
        assert_eq!(
            before.as_array().map(|a| a[1..4].to_vec()),
            Some(vec![json!(false), json!(false), json!([])]),
            "a screen sender before the first share: {before}"
        );
        let posted = ada_page
            .browser
            .script("return window.beltPosted.length;", json!([]))
            .await;
        press_header(&ada_page, ".murmure-call-share").await;
        share_pressed(&ada_page, "true").await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let sent = loop {
            let sent = screen_sender(&ada_page).await;
            if sent[6].as_f64().unwrap_or(0.0) >= 5.0 || std::time::Instant::now() >= deadline {
                break sent;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        };
        assert_eq!(
            sent.as_array().map(|a| a[..4].to_vec()),
            Some(vec![
                json!(before[0].as_u64().unwrap_or(0) + 1),
                json!(true),
                json!(true),
                json!([[null, 30, 1_500_000, true]])
            ]),
            "{before} then {sent}"
        );
        assert_eq!(sent[4], json!("video/VP8"), "{sent}");
        let width = sent[5].as_u64().unwrap_or(0);
        assert!(
            (1..=1920).contains(&width),
            "the screen left at {width} pixels across"
        );
        // The page posts the offer that adds the screen section, and the start of the share as one
        // byte with no field. The Sync worker carries both to the SFU (`CALL-106`).
        let tag = roster_tag();
        let start = postcard::to_allocvec(&ambre_bus::SyncCommand::ShareStarted)
            .expect("a share start encodes");
        let offer_tag = postcard::to_allocvec(&ambre_bus::SyncCommand::CallDescription(
            ambre_bus::PageDescription::Offer(
                ambre_core::Validated::parse("v=0".to_owned()).expect("a description"),
            ),
        ))
        .ok()
        .and_then(|bytes| bytes.first().copied())
        .map(u64::from)
        .expect("a description encodes");
        let posts = ada_page
            .browser
            .script(
                "return window.beltPosted.slice(arguments[0]).map((p) => Array.from(p));",
                json!([posted]),
            )
            .await;
        let posts: Vec<&Value> = posts
            .as_array()
            .into_iter()
            .flatten()
            .filter(|bytes| bytes[0].as_u64() != Some(tag))
            .collect();
        let (offers, others): (Vec<&Value>, Vec<&Value>) = posts
            .iter()
            .partition(|bytes| bytes[0].as_u64() == Some(offer_tag));
        assert_eq!(offers.len(), 1, "the share posted {posts:?}");
        assert_eq!(others, vec![&json!(start)], "the share posted {posts:?}");

        // The camera of Ada keeps its three encodings across the offer. Bob still decodes it at
        // its top size while the screen goes too (`CALL-113`).
        camera_layers(&ada_page, &camera_before).await;
        bob_decodes_top(&bob_page).await;

        // Bob decodes Ada's screen on his screen slot, through its transform. His canvas draws
        // it in a tile taller than any tile of a camera (`CALL-102`, `CALL-106`, `CALL-110`,
        // `MURMURE-629`).
        let shown = screen_decodes(&bob_page, 0.0, 10.0).await;
        assert_eq!(
            shown[2],
            json!(true),
            "Bob's screen slot holds no transform: {shown}"
        );
        bob_page
            .browser
            .wait_for(
                "thirty paints of a video",
                "return performance.getEntriesByName('ambre:video-paint').length >= 30;",
                std::time::Duration::from_secs(40),
            )
            .await;
        let ratio = bob_page
            .browser
            .script("return window.devicePixelRatio || 1;", json!([]))
            .await
            .as_f64()
            .unwrap_or(1.0);
        // A tile of a camera is 164 pixels tall at most: the 180 of its band less two gaps. Its
        // row sits at the bottom of the band. So a thousand moved pixels above the 164 pixels
        // over the lowest moved row lie in no camera tile. The span of a box is no proof: a few
        // pixels of codec noise in a still screen can stretch it.
        //
        // One pair of shots is no proof of a still screen either. The fake screen of Chrome
        // sweeps a disc that comes back to the same angle about every 0.6 s. Two shots a
        // whole number of turns apart differ by its clock text alone. So each later shot
        // comes 0.7 s after the one before, and reads against the first. A shot off the
        // period of the sweep reads the disc at another angle.
        let first = canvas_shot(&bob_page).await;
        let mut read = Vec::new();
        let (moved, above, reach) = loop {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            let second = canvas_shot(&bob_page).await;
            let (moved, bounds) = changed(&first, &second);
            let [_, _, _, bottom] = bounds.unwrap_or_default();
            let reach = bottom.saturating_sub((164.0 * ratio) as u32);
            let above = moved_above(&first, &second, reach);
            read.push((moved, above, reach));
            if (moved >= 1_000 && above >= 1_000) || read.len() == 8 {
                break (moved, above, reach);
            }
        };
        screen.check();
        assert!(
            moved >= 1_000,
            "only {moved} pixels moved under a screen, over {read:?}"
        );
        assert!(
            above >= 1_000,
            "{above} pixels moved above row {reach}, out of {moved} under a screen: \
             no tile taller than a camera moved, over {read:?} (moved, above, row)"
        );
        for page in [&ada_page, &bob_page] {
            assert_eq!(
                video_elements(page).await,
                json!(0),
                "a video element in the page"
            );
        }

        // Bob shares too, and his share is the last one: Ada decodes his screen, her share
        // stops, and her header says so. Bob reads no screen of his own (`CALL-106`, `CALL-107`,
        // `CALL-111`, `MURMURE-630`).
        let ada_seen = screen_received(&ada_page).await[0].as_f64().unwrap_or(0.0);
        let bob_first = screen_sender(&bob_page).await;
        press_header(&bob_page, ".murmure-call-share").await;
        share_pressed(&bob_page, "true").await;
        screen_decodes(&ada_page, ada_seen, 10.0).await;
        assert_eq!(
            share_notice(&ada_page).await,
            json!(["status", "Another member shares the screen now"])
        );
        share_pressed(&ada_page, "false").await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let stopped = screen_sender(&ada_page).await;
            if stopped[1] == json!(false) {
                assert_eq!(stopped[0], sent[0], "the take-over renegotiated: {stopped}");
                assert_eq!(stopped[2], json!(true), "{stopped}");
                assert_eq!(
                    stopped[3],
                    json!([[null, 30, 1_500_000, false]]),
                    "{stopped}"
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Ada's sender kept her screen: {stopped}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let bob_seen = screen_received(&bob_page).await[0].as_f64().unwrap_or(0.0);
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let bob_now = screen_received(&bob_page).await[0].as_f64().unwrap_or(0.0);
        assert!(
            bob_now - bob_seen < 2.0,
            "Bob decoded {} frames of a screen while his share is the last one",
            bob_now - bob_seen
        );

        // Ada takes the screen back with a second share. The SFU tells Bob, whose share stops
        // and whose header says so: a sharer seen before counts too (`CALL-106`, `CALL-107`,
        // `MURMURE-630`). Neither page renegotiates past its first share.
        let bob_before = screen_sender(&bob_page).await;
        assert_eq!(
            bob_before[0].as_u64(),
            bob_first[0].as_u64().map(|n| n + 1),
            "Bob's first share offered {bob_first} then {bob_before}"
        );
        let ada_again = screen_sender(&ada_page).await;
        let bob_seen = screen_received(&bob_page).await[0].as_f64().unwrap_or(0.0);
        press_header(&ada_page, ".murmure-call-share").await;
        share_pressed(&ada_page, "true").await;
        screen_decodes(&bob_page, bob_seen, 10.0).await;
        assert_eq!(
            share_notice(&bob_page).await,
            json!(["status", "Another member shares the screen now"])
        );
        share_pressed(&bob_page, "false").await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let stopped = screen_sender(&bob_page).await;
            if stopped[1] == json!(false) {
                assert_eq!(
                    stopped[0], bob_before[0],
                    "Bob's stop renegotiated: {stopped}"
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Bob's sender kept his screen: {stopped}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let ada_back = screen_sender(&ada_page).await;
        assert_eq!(
            ada_back[0], ada_again[0],
            "Ada's second share renegotiated: {ada_again} then {ada_back}"
        );

        // Ada's browser ends her track, as its bar does: her share stops, under the same
        // session and transform (`CALL-108`, `CALL-103`).
        let ada_before = screen_sender(&ada_page).await;
        ada_page
            .browser
            .script(
                "const pc = window.beltPcs[0]; \
                 const t = pc.getTransceivers().filter((t) => t.direction === 'sendonly')[1]; \
                 const track = t.sender.track; track.stop(); \
                 track.dispatchEvent(new Event('ended')); return true;",
                json!([]),
            )
            .await;
        share_pressed(&ada_page, "false").await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let ended = screen_sender(&ada_page).await;
            if ended[1] == json!(false) {
                assert_eq!(ended[0], ada_before[0], "the end renegotiated: {ended}");
                assert_eq!(ended[2], json!(true), "{ended}");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Ada's sender kept an ended screen: {ended}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        for page in [&ada_page, &bob_page] {
            click_header(page, ".murmure-call-hang-up").await;
        }
    }
}
