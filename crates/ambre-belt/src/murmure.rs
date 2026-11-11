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
    const DOWN: &str = "\u{E015}";
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

    /// Records each peer connection and each worker the page makes from now on, and the
    /// notes a worker posts. The page makes neither before the join. It also records each
    /// track a sender takes, with whether its transform was there, and any data channel.
    const RECORD: &str = "window.beltPcs = []; window.beltNotes = []; \
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
                     if (s.type === 'outbound-rtp') out.sent.push([s.bytesSent, s.packetsSent]); \
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
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
        prompt(
            &ada_page,
            ".murmure-space",
            "murmure-space-invite",
            &format!("{carol}@dev.local"),
        )
        .await;
        confirm_key(&ada_page).await;
        ada_page.wait_toast(&format!("Invited {carol}")).await;
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

    /// Presses the list of the channel, then `Up`, until what the live region reads of the
    /// latest message satisfies `done`, within `within`. Answers what it read.
    async fn read_until(
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
            assert!(
                std::time::Instant::now() < deadline,
                "the latest message read {heard:?}, not {what}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        }
    }

    /// How long a reader may wait for who is in a call: two asks of the interval of the
    /// budget row, and the round trips.
    const ROSTER_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

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
    const LIVE: &str = "Call in progress";
    const ENDED: &str = "Call ended";

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
    fn last_channel() -> String {
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
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
        prompt(
            &ada_page,
            ".murmure-space",
            "murmure-space-invite",
            &format!("{carol}@dev.local"),
        )
        .await;
        confirm_key(&ada_page).await;
        ada_page.wait_toast(&format!("Invited {carol}")).await;
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
                |h| h.contains(LIVE) && h.contains(&ada),
                ROSTER_WAIT,
            )
            .await;
        }

        join_from_message(&bob_page).await;
        decoded_past(&[&ada_page, &bob_page], &[0.0; 2], 48_000.0).await;
        let heard = read_until(
            &carol_page,
            "Ada and Bob in the call",
            |h| h.contains(LIVE) && h.contains(&ada) && h.contains(&bob),
            ROSTER_WAIT,
        )
        .await;
        assert!(!heard.contains(&carol), "Carol is out of the call: {heard}");

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
        space_with_channel(&ada_page, "Belt", "general", &carol).await;
        open(&carol_page, "general").await;
        let channel = last_channel();
        start_call(&ada_page, &channel).await;
        read_until(
            &carol_page,
            "Ada in the call",
            |h| h.contains(LIVE) && h.contains(&ada),
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
    async fn click_header(page: &Page, selector: &str) {
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
    }

    /// The length of one window of [`Window`].
    const WINDOW: std::time::Duration = std::time::Duration::from_secs(3);

    /// The first audio stream `stats` sent, as its bytes and its packets.
    fn sent(stats: &Value) -> (f64, f64) {
        let first = &stats["sent"][0];
        (
            first[0].as_f64().unwrap_or(0.0),
            first[1].as_f64().unwrap_or(0.0),
        )
    }

    /// Reads one [`Window`] of the audio `sender` sends to `receiver`.
    async fn window(sender: &Page, receiver: &Page) -> Window {
        let (s0, r0) = (call_stats(sender).await, call_stats(receiver).await);
        tokio::time::sleep(WINDOW).await;
        let (s1, r1) = (call_stats(sender).await, call_stats(receiver).await);
        let ((b0, p0), (b1, p1)) = (sent(&s0), sent(&s1));
        let num = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
        Window {
            energy: num(&r1, "energy") - num(&r0, "energy"),
            decoded: num(&r1, "decoded") - num(&r0, "decoded"),
            packets: p1 - p0,
            bytes: b1 - b0,
        }
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
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
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

        let live = window(&ada_page, &bob_page).await;
        assert!(live.energy > 0.0, "Bob hears nothing of Ada: {live:?}");
        assert!(live.packets > 0.0, "Ada sends nothing: {live:?}");

        click_header(&ada_page, ".murmure-call-mute").await;
        mute_pressed(&ada_page, "true").await;
        // The frames already in the jitter buffer of Bob play out first.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let muted = window(&ada_page, &bob_page).await;
        // Bob decodes a frame every 20 ms, each of silence (`CALL-074`).
        assert!(
            muted.decoded >= 48_000.0 * 2.0,
            "Bob stopped decoding Ada: {muted:?}"
        );
        assert!(
            muted.energy < live.energy / 100.0,
            "Bob hears Ada through the mute: {muted:?} against {live:?}"
        );
        // The SFU reads the same rate and the same size (`CALL-075`, `ZK-025`).
        assert!(
            muted.packets >= live.packets * 0.9,
            "a mute changed the packet rate: {muted:?} against {live:?}"
        );
        let size = |w: &Window| w.bytes / w.packets;
        assert!(
            (size(&muted) - size(&live)).abs() < 0.5,
            "a mute changed the chunk size: {muted:?} against {live:?}"
        );

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
            |h| {
                h.split_once(LIVE)
                    .is_some_and(|(_, inside)| inside.contains(&bob) && !inside.contains(&ada))
            },
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
          if (d instanceof ArrayBuffer) window.beltPosted.push(new Uint8Array(d.slice(0))); \
          else if (ArrayBuffer.isView(d)) window.beltPosted.push(new Uint8Array(d.buffer.slice(d.byteOffset, d.byteOffset + d.byteLength))); \
          else window.beltPosted.push(new TextEncoder().encode(typeof d === 'string' ? d : (JSON.stringify(d) || ''))); \
        } catch (e) {} }; \
        for (const C of [Worker, MessagePort]) { let o = C.prototype; \
          while (!Object.prototype.hasOwnProperty.call(o, 'postMessage')) o = Object.getPrototypeOf(o); \
          const P = o.postMessage; \
          o.postMessage = function (m, ...r) { keep(m); return P.call(this, m, ...r); }; } \
        return true;";

    /// The samples the decoders of `page` concealed so far. A receiver makes up each frame that
    /// its sender did not send, or that the network lost.
    async fn concealed(page: &Page) -> f64 {
        call_stats(page).await["concealed"]
            .as_f64()
            .unwrap_or(f64::MAX)
    }

    /// The length of the windows that measure what a change of microphone conceals.
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
    /// mutes, then picks another microphone in the panel. Bob conceals no frame of hers, and
    /// still hears silence: the mute carried over. The call kept its peer connection, its call
    /// worker, its transform and its session. Ada unmutes, and Bob hears the new microphone.
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
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
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
        let start = concealed(&bob_page).await;
        tokio::time::sleep(SPAN).await;
        let steady = concealed(&bob_page).await - start;
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
        let start = concealed(&bob_page).await;
        let began = std::time::Instant::now();
        let label = pick(&ada_page, ".murmure-call-microphone", 1).await;
        assert!(tracks_taken(&ada_page, 2).await, "the transform changed");
        tokio::time::sleep(SPAN.saturating_sub(began.elapsed())).await;
        let across = concealed(&bob_page).await - start;
        ada_page
            .browser
            .script("window.beltOpening = 0; return true;", json!([]))
            .await;
        eprintln!("concealed: {steady} samples with no change, {across} across the change");
        // Each frame of Ada reaches Bob across the change (`CALL-078`). So Bob makes up no more
        // frames than in a window of the same length with no change. One frame is 960 samples.
        assert!(
            across <= steady + 960.0,
            "{across} samples concealed across the change, against {steady} with none"
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
            still.packets >= muted.packets * 0.9,
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
                    "const want = {}; const p = window.beltPlayers; \
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
    /// Headless Firefox lists no output. Its run reads that the panel offers no speaker, and
    /// that each slot plays through an element of its own.
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
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
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
            // No output listed: no control that does nothing (`MURMURE-609`).
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
                json!("none"),
                "a speaker choice that routes nothing"
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

    /// Sets the slider named `label` of the panel of `page` to `value`, as a hand would. Answers
    /// how many messages the page posted to a worker or a port while it applied the value.
    async fn slide(page: &Page, label: &str, value: &str) -> Value {
        page.browser
            .wait_for(
                &format!("the slider {label}"),
                &format!(
                    "const i = [...document.querySelectorAll('.murmure-call-panel input[type=range]')] \
                       .find((i) => i.getAttribute('aria-label') === {}); \
                     if (!i) return null; const n = window.beltPosted.length; \
                     i.value = {}; i.dispatchEvent(new Event('input', {{ bubbles: true }})); \
                     return window.beltPosted.length - n;",
                    Value::String(label.to_owned()),
                    Value::String(value.to_owned())
                ),
                STEP,
            )
            .await
    }

    /// Mutes or unmutes `page`, and waits for the header to read it.
    async fn set_mute(page: &Page, pressed: &str) {
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
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
        prompt(
            &ada_page,
            ".murmure-space",
            "murmure-space-invite",
            &format!("{carol}@dev.local"),
        )
        .await;
        confirm_key(&ada_page).await;
        ada_page.wait_toast(&format!("Invited {carol}")).await;
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
        assert_eq!(slide(&ada_page, &bob_label, "0").await, json!(0));

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
            stored.as_str().is_some_and(|t| t.contains(",0.0]")),
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
        space_with_channel(&ada_page, "Belt", "general", &bob).await;
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
        assert_eq!(slide(&ada_page, "Microphone gain", "0").await, json!(0));
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
        // chunk has one size, so the bytes of a window count its chunks. A padding packet of
        // the congestion control carries no chunk, so the rate counts chunks, not packets.
        let size = live.bytes / live.packets;
        let same = |w: &Window| {
            let chunks = w.bytes / size;
            assert!(
                (chunks - chunks.round()).abs() < 1e-6,
                "a gain changed the chunk size: {w:?} against {live:?}"
            );
            assert!(
                chunks >= live.packets * 0.9,
                "a gain changed the chunk rate: {w:?} against {live:?}"
            );
        };
        same(&silent);

        // A higher gain brings Ada back, decoded whole (`CALL-090`).
        assert_eq!(slide(&ada_page, "Microphone gain", "150").await, json!(0));
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
}
