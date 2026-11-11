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
          this.addEventListener('message', (e) => window.beltNotes.push(e.data)); } }; \
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
                     state: pc.connectionState, sent: [], received: [], decoded: 0, energy: 0, remotePort: null, \
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
        read_until(page, "a live call", |h| h.contains(LIVE), ROSTER_WAIT).await;
        // The focus may rest on the message above the call message, as in a direct message
        // whose first message came before the call: `Down` then reaches the call message.
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

    /// `MURMURE-599`, `MURMURE-602`, `MURMURE-603`, `MURMURE-604`, `CALL-074`, `CALL-075` and
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

        // The call button names itself, and takes the keyboard focus (`MURMURE-604`).
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
            "a page in a call offers no call button (`MURMURE-602`)"
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
        // (`MURMURE-603`).
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

    /// `MURMURE-600`, `MURMURE-601` and `CALL-076`, in the browser.
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
}
