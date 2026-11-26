//! The Murmure screenshots beside the mockup. The seed writes the space of the Murmure calls
//! mockup under `research/mockups` through the belt relay, as five people write it. The shots
//! then put each state of the app beside the render of that state in the mockup.
//!
//! The seed signs five members in, each in a browser of its own. The user creates the space
//! and its channels, and invites the four others. Each member then writes, reacts and replies
//! through the page, as a person does. Lea starts a call in `#design`, and two members join it.
//!
//! The recipe murmure-shots runs the seed on a database of its own, so each space shows
//! once in the user's sidebar. It then shoots each state of `#seg-state`, on each device and
//! in each theme. Each app shot lands beside the mockup render of the same state. A third
//! image sets the two side by side. A state the app cannot reach yet keeps its render. The
//! app shot beside it shows the closest state the app reaches, and its file name says so.
//!
//! The mockup draws what the app lacks: categories, a private channel and a forum. No control
//! of the app creates them, so the seed leaves them out. The shots then show the gap.

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};

    use crate::call::Sfu;
    use crate::driver::Browser;
    use crate::murmure::tests::{
        CHANNEL, DOWN, F10, LIVE, ROSTER_WAIT, SHIFT, THREAD, UP, camera_pressed, click_header,
        confirm_key, entry_js, follow, in_call, invite_into_space, message_member, murmure, open,
        open_thread, press, press_header, prompt, reachable, send, set_mute, share_pressed,
        take_keys, typed_and_sent, wait_unread,
    };
    use crate::page::{Page, STEP, drag};
    use crate::relay::Relay;

    /// The space of the mockup.
    const SPACE: &str = "North Studio";

    /// The two other spaces of the mockup's switcher: one with an unread message, one quiet.
    const OTHER_SPACES: [&str; 2] = ["Open Collective", "Family"];

    /// The public channels the user creates, in the order of the sidebar.
    const CHANNELS: [&str; 5] = ["general", "design", "roadmap", "ideas", "incidents"];

    /// The channel a member other than the user creates, which the user then has to join.
    const TO_JOIN: &str = "hiring";

    /// The message the thread, the reactions and the summary of the mockup hang on.
    const ROOT: &str = "I pushed v6 of the chrome mockup: the context menu, the tooltips, \
        and the panel docked on the left as an option.";

    /// The five display names of one seed. The dev sign-in derives the keys from the name.
    struct Names {
        me: String,
        lea: String,
        karim: String,
        ines: String,
        tom: String,
    }

    impl Names {
        /// The names the mockup shows. They are the same on every run, so a run needs a
        /// database of its own. Each run would add its spaces to the user's sidebar.
        fn mockup() -> Self {
            Self {
                me: "Michael".to_owned(),
                lea: "L\u{e9}a Martin".to_owned(),
                karim: "Karim Bensa\u{ef}d".to_owned(),
                ines: "In\u{e8}s Roux".to_owned(),
                tom: "Tom Leroy".to_owned(),
            }
        }

        /// The names of the mockup, each with the same fresh suffix, so a run on a shared
        /// database meets no account of an earlier run.
        fn fresh() -> Self {
            let tag = uuid::Uuid::new_v4().simple().to_string();
            let tag = &tag[..6];
            let m = Self::mockup();
            Self {
                me: format!("{} {tag}", m.me),
                lea: format!("{} {tag}", m.lea),
                karim: format!("{} {tag}", m.karim),
                ines: format!("{} {tag}", m.ines),
                tom: format!("{} {tag}", m.tom),
            }
        }
    }

    /// The five pages of one seed, and the SFU of its call.
    struct Cast {
        names: Names,
        me: Page,
        lea: Page,
        karim: Page,
        ines: Page,
        tom: Page,
        _sfu: Sfu,
    }

    /// Starts the SFU, the relay and the five pages, each signed in and on Murmure.
    /// `database` names the database of the relay, when the run has one of its own.
    async fn cast(names: Names, database: Option<String>) -> Cast {
        let sfu = Sfu::start(None).await;
        let mut env: Vec<(&str, String)> = sfu.relay_env().to_vec();
        if let Some(url) = database {
            env.push(("DATABASE_URL", url));
        }
        let relay = Arc::new(Relay::start_with(&env).await);
        let (me, lea, karim, ines, tom) = tokio::join!(
            murmure(Arc::clone(&relay), &names.me),
            murmure(Arc::clone(&relay), &names.lea),
            murmure(Arc::clone(&relay), &names.karim),
            murmure(Arc::clone(&relay), &names.ines),
            murmure(Arc::clone(&relay), &names.tom),
        );
        Cast {
            names,
            me,
            lea,
            karim,
            ines,
            tom,
            _sfu: sfu,
        }
    }

    /// Presses the list of the canvas `selector`, then moves the focus from message to message
    /// until the live region reads one that holds `text`. With no message focused, the first
    /// `Up` focuses the latest message.
    async fn focus_message(page: &Page, selector: &str, text: &str) {
        focus_message_within(page, selector, text, STEP).await;
    }

    /// As [`focus_message`], for a message that may reach the page within `within`.
    async fn focus_message_within(page: &Page, selector: &str, text: &str, within: Duration) {
        if !shows(page, selector, text, within).await {
            let heard = page.announced().await;
            panic!("no message of {selector} holds {text:?}; the last read {heard:?}");
        }
    }

    /// Whether a message of the canvas `selector` that holds `text` shows within `within`.
    /// The message it finds keeps the focus.
    async fn shows(page: &Page, selector: &str, text: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            let at = reachable(page, selector, false).await;
            page.browser.perform(json!([drag(at, at, 0)])).await;
            // A message focused earlier keeps the focus, so the scan walks down to the
            // latest message first, then up.
            for key in [UP].into_iter().chain([DOWN; 12]).chain([UP; 12]) {
                press(page, &[key]).await;
                tokio::time::sleep(Duration::from_millis(200)).await;
                if page.announced().await.contains(text) {
                    return true;
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
        }
    }

    /// Opens the menu of the focused message and runs its action `label`.
    async fn menu_action(page: &Page, label: &str) {
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": SHIFT },
                { "type": "keyDown", "value": F10 },
                { "type": "keyUp", "value": F10 },
                { "type": "keyUp", "value": SHIFT },
            ]}]))
            .await;
        let label = Value::String(label.to_owned());
        page.browser
            .wait_for(
                &format!("the action {label}"),
                &format!(
                    "const i = [...document.querySelectorAll('.ambre-ctx-item')] \
                       .find(e => e.textContent.includes({label})); \
                     if (!i) return false; i.click(); return true;"
                ),
                STEP,
            )
            .await;
    }

    /// Reacts with the emoji named `emoji` to the message of the channel that holds `text`,
    /// through the menu of the message and the picker.
    async fn react(page: &Page, text: &str, emoji: &str) {
        focus_message(page, CHANNEL, text).await;
        menu_action(page, "Add reaction").await;
        let emoji = Value::String(emoji.to_owned());
        page.browser
            .wait_for(
                &format!("the emoji {emoji} in the picker"),
                &format!(
                    "const b = [...document.querySelectorAll('.murmure-emoji-pick')] \
                       .find(b => b.title === {emoji}); \
                     if (!b) return false; b.click(); return true;"
                ),
                STEP,
            )
            .await;
        page.browser
            .wait_for(
                "the picker closed",
                "const s = document.querySelector('.murmure-emoji-search'); \
                 return !s || !s.checkVisibility();",
                STEP,
            )
            .await;
    }

    /// Types `before`, then `@` and the start of `name`, picks `name` in the list of
    /// mentions with Tab, then types `after` and sends the message.
    async fn send_mention(page: &Page, before: &str, name: &str, after: &str) {
        take_keys(page, CHANNEL).await;
        let keys = |text: &str| -> Vec<Value> {
            text.chars()
                .flat_map(|c| {
                    let k = c.to_string();
                    [
                        json!({ "type": "keyDown", "value": k }),
                        json!({ "type": "keyUp", "value": k }),
                    ]
                })
                .collect()
        };
        let query: String = name.chars().take(4).collect();
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard",
                "actions": keys(&format!("{before}@{query}")) }]))
            .await;
        let shown = Value::String(name.to_owned());
        page.browser
            .wait_for(
                &format!("{shown} in the list of mentions"),
                &format!(
                    "return [...document.querySelectorAll('.murmure-mention-list .murmure-mention')] \
                       .some(o => o.textContent.includes({shown}));"
                ),
                STEP,
            )
            .await;
        // A press picks the member as a pointer does; the list takes it on `mousedown`.
        page.browser
            .script(
                "const o = [...document.querySelectorAll('.murmure-mention-list .murmure-mention')] \
                   .find(o => o.textContent.includes(arguments[0])); \
                 o.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true })); \
                 return true;",
                json!([name]),
            )
            .await;
        page.browser
            .wait_for(
                &format!("{shown} picked"),
                "return !document.querySelector('ul.murmure-mention-list');",
                STEP,
            )
            .await;
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard",
                "actions": typed_and_sent(after) }]))
            .await;
    }

    /// Joins the live call of the shown channel from its call message.
    async fn join_call(page: &Page) {
        // The menu offers the join once the page read the call it names, so a menu without
        // it closes and opens again.
        let deadline = Instant::now() + ROSTER_WAIT;
        loop {
            focus_message_within(page, CHANNEL, LIVE, ROSTER_WAIT).await;
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
                    "the menu of the call message",
                    "const i = [...document.querySelectorAll('.ambre-ctx-item')]; \
                     if (!i.length) return null; \
                     const j = i.find(e => e.textContent.includes('Join call')); \
                     if (j) { j.click(); return 'joined'; } return 'other';",
                    STEP,
                )
                .await;
            if joined == json!("joined") {
                break;
            }
            press(page, &["\u{E00C}"]).await;
            assert!(
                Instant::now() < deadline,
                "no menu of the call message offered Join call"
            );
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        page.browser
            .wait_for(
                "the hang-up control",
                "return !!document.querySelector('.murmure-channel-header .murmure-call-hang-up');",
                STEP,
            )
            .await;
    }

    /// Turns the camera of `page` on.
    async fn camera_on(page: &Page) {
        click_header(page, ".murmure-call-camera").await;
        camera_pressed(page, "true").await;
    }

    /// Seeds the space of the mockup in the order of the mockup's `#design`. The root comes
    /// first, with its reactions and its thread, then two messages of Karim. The call and the
    /// mention land past the user's read cursor. The user shows `#general` at the end.
    async fn seed(c: &Cast) {
        let n = &c.names;
        prompt(&c.me, ".murmure-spaces", "murmure-new-space", SPACE).await;
        for channel in CHANNELS {
            prompt(&c.me, ".murmure-space", "murmure-new-channel", channel).await;
            // Each open sets the user's read cursor, so a later message counts unread
            // (`MURMURE-120`).
            open(&c.me, channel).await;
        }
        for (name, page) in [
            (&n.lea, &c.lea),
            (&n.karim, &c.karim),
            (&n.ines, &c.ines),
            (&n.tom, &c.tom),
        ] {
            let link = invite_into_space(&c.me, name).await;
            follow(page, &link).await;
        }
        other_spaces(c).await;
        prompt(&c.lea, ".murmure-space", "murmure-new-channel", TO_JOIN).await;
        open(&c.lea, TO_JOIN).await;

        // #roadmap: four messages the user has not read.
        open(&c.lea, "roadmap").await;
        for text in [
            "Q1 milestones are in the board.",
            "The calls epic moves to February.",
            "Prose comments ship first.",
            "Review on Thursday.",
        ] {
            // Each message shows in Lea's list before the next one is typed, so no two
            // drafts merge. The first message after a join is lost now and then, and the
            // seed then sends it once more and says so.
            send(&c.lea, CHANNEL, text).await;
            if !shows(&c.lea, CHANNEL, text, ROSTER_WAIT).await {
                eprintln!("SEED-LOST {text:?} never showed in the list of its author; sent again");
                send(&c.lea, CHANNEL, text).await;
                focus_message(&c.lea, CHANNEL, text).await;
            }
        }
        wait_unread(&c.me, "roadmap", 4).await;
        // #incidents: two mentions of the user.
        open(&c.karim, "incidents").await;
        send_mention(
            &c.karim,
            "",
            &n.me,
            "the relay restarted at 03:12, see the log.",
        )
        .await;
        send_mention(&c.karim, "", &n.me, "it is back, nothing lost.").await;

        // #design: the user reads it while the root, its reactions and its thread arrive.
        open(&c.me, "design").await;
        for page in [&c.lea, &c.karim, &c.ines, &c.tom] {
            open(page, "design").await;
        }
        send(&c.lea, CHANNEL, ROOT).await;
        for (page, emoji) in [
            (&c.me, "thumbs up"),
            (&c.karim, "thumbs up"),
            (&c.tom, "thumbs up"),
            (&c.ines, "party popper"),
            (&c.tom, "party popper"),
        ] {
            react(page, "chrome mockup", emoji).await;
        }
        for (page, text) in [
            (&c.ines, "The panel on the left works much better for me."),
            (
                &c.me,
                "It is a user setting, shared by the three apps. The side follows the account; \
                 the width stays on the device.",
            ),
            (
                &c.tom,
                "Agreed, as long as the right side stays the default.",
            ),
            (
                &c.karim,
                "Fine by me. And the collapsed state is remembered per app.",
            ),
        ] {
            open_thread(page, "chrome mockup").await;
            send(page, THREAD, text).await;
        }
        send(
            &c.karim,
            CHANNEL,
            "For Murmure we follow the same rule: no docked toolbar. Call controls stay in the \
             channel header.",
        )
        .await;
        send_code(
            &c.karim,
            "The 196 px band holds 4 faces. Beyond that, pages of nine: `MURMURE-654`",
            ".",
        )
        .await;

        // The user reads Karim's last message, then leaves #design: what follows lands past
        // the read cursor.
        focus_message(&c.me, CHANNEL, "MURMURE-654").await;
        open(&c.me, "general").await;
        send(&c.me, CHANNEL, "Welcome to North Studio.").await;

        // The direct messages: one the user read, one still unread.
        message_member(&c.lea, &n.me).await;
        direct_shown(&c.lea).await;
        send(
            &c.lea,
            CHANNEL,
            "Can you look at the tooltips before the review?",
        )
        .await;
        message_member(&c.tom, &n.me).await;
        direct_shown(&c.tom).await;
        send(&c.tom, CHANNEL, "The budget sheet is ready for Thursday.").await;
        open(&c.me, &n.lea).await;
        confirm_key_if_asked(&c.me).await;
        open(&c.me, "general").await;

        // The call: Lea starts it, Karim and Ines join, Lea and Karim turn their camera on.
        for page in [&c.lea, &c.karim, &c.ines, &c.tom] {
            open(page, "design").await;
        }
        click_header(&c.lea, ".murmure-call-start").await;
        c.lea
            .browser
            .wait_for(
                "Lea in her call",
                "return !!document.querySelector('.murmure-channel-header .murmure-call-hang-up');",
                ROSTER_WAIT,
            )
            .await;
        join_call(&c.karim).await;
        join_call(&c.ines).await;
        camera_on(&c.lea).await;
        camera_on(&c.karim).await;
        send_mention(
            &c.ines,
            "",
            &n.me,
            "are you joining us? We are going through the budget sheet in Trame.",
        )
        .await;
        // The link of the last invite stays in the user's toolbar until a press closes it.
        c.me.browser
            .script(
                "document.querySelector('.murmure-invite-link-close')?.click(); return true;",
                json!([]),
            )
            .await;
    }

    /// Types `text`, which ends on the backtick that closes its inline code, then `rest` and
    /// Enter. The composer turns a closed marker into its mark at its next frame. A person
    /// types the next key a frame later, and gets the code chip (`MURMURE-197`). The seed
    /// waits that frame, where a run of keys in one action would outpace it.
    async fn send_code(page: &Page, text: &str, rest: &str) {
        take_keys(page, CHANNEL).await;
        let typed: Vec<Value> = text
            .chars()
            .map(String::from)
            .flat_map(|k| {
                [
                    json!({ "type": "keyDown", "value": k }),
                    json!({ "type": "keyUp", "value": k }),
                ]
            })
            .collect();
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": typed }]))
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        page.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": typed_and_sent(rest) }]))
            .await;
    }

    /// Shows the space `space` in the sidebar of `page`, through the switcher.
    async fn show_space(page: &Page, space: &str) {
        let space = Value::String(space.to_owned());
        page.browser
            .wait_for(
                &format!("the space {space} shown"),
                &format!(
                    "const t = document.querySelector('.murmure-space-switch .ambre-select-trigger'); \
                     if (!t) return false; \
                     if (t.textContent.includes({space})) return true; \
                     const o = [...document.querySelectorAll('.ambre-select-option')] \
                       .find(o => o.textContent.startsWith({space})); \
                     if (o) o.click(); else t.click(); return false;"
                ),
                STEP,
            )
            .await;
    }

    /// Invites `invitee` into the open space from `page`, as `invite_into_space` does, for a
    /// member whose key the user already confirmed: no dialog asks again.
    async fn invite_known(page: &Page, invitee: &str) -> String {
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
        confirm_key_if_asked(page).await;
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

    /// The two other spaces of the switcher. Tom writes in the first, which the user opened
    /// once, so the user holds one unread message there. Each page then shows North Studio.
    async fn other_spaces(c: &Cast) {
        let [collective, family] = OTHER_SPACES;
        prompt(&c.me, ".murmure-spaces", "murmure-new-space", collective).await;
        show_space(&c.me, collective).await;
        prompt(&c.me, ".murmure-space", "murmure-new-channel", "general").await;
        open(&c.me, "general").await;
        let link = invite_known(&c.me, &c.names.tom).await;
        follow(&c.tom, &link).await;
        show_space(&c.tom, collective).await;
        open(&c.tom, "general").await;
        prompt(&c.me, ".murmure-spaces", "murmure-new-space", family).await;
        show_space(&c.me, family).await;
        show_space(&c.me, SPACE).await;
        send(&c.tom, CHANNEL, "The grant report is due on Monday.").await;
        show_space(&c.tom, SPACE).await;
    }

    /// Waits until `page` shows a direct message in place of `#design`.
    async fn direct_shown(page: &Page) {
        page.browser
            .wait_for(
                "the direct message shown",
                "const n = document.querySelector('.murmure-channel-name'); \
                 return !!n && !!n.textContent && n.textContent !== 'design';",
                STEP,
            )
            .await;
        page.wait_canvas().await;
    }

    /// Confirms the first sight of a member's key when the dialog asks, and goes on when it
    /// does not ask.
    async fn confirm_key_if_asked(page: &Page) {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let asked = page
            .browser
            .script(
                "return !!document.querySelector('.ambre-tofu-confirm');",
                json!([]),
            )
            .await;
        if asked == json!(true) {
            confirm_key(page).await;
        }
    }

    /// The names of the entries of the user's sidebar, in order.
    async fn entries(page: &Page) -> Vec<String> {
        page.browser
            .script(
                "return [...document.querySelectorAll('.murmure-entry-name')] \
                   .map(e => e.textContent);",
                json!([]),
            )
            .await
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e.as_str().map(str::to_owned))
            .collect()
    }

    /// The seed of the Murmure screenshots lands what the mockup shows and the app can hold.
    /// The user's sidebar lists the three spaces, the channels and the two direct messages.
    /// It counts four unread messages in `#roadmap` and two mentions in `#incidents`. In
    /// `#design`, the call holds Lea, Karim and Ines, and the root reads its four replies.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, the SFU, chromedriver and the services; run just test-browser"
    )]
    #[cfg_attr(
        feature = "firefox",
        ignore = "five browsers in one call outrun the Firefox runner of CI"
    )]
    async fn the_seed_holds_the_space_of_the_murmure_mockup() {
        let c = cast(Names::fresh(), None).await;
        seed(&c).await;
        let n = &c.names;
        let listed = entries(&c.me).await;
        for name in CHANNELS
            .iter()
            .copied()
            .chain([TO_JOIN, n.lea.as_str(), n.tom.as_str()])
        {
            assert!(listed.iter().any(|e| e == name), "{name} in {listed:?}");
        }
        let spaces = c
            .me
            .browser
            .script(
                "document.querySelector('.murmure-space-switch .ambre-select-trigger').click(); \
                 const o = [...document.querySelectorAll('.ambre-select-option')] \
                   .map(o => o.textContent); \
                 document.querySelector('.murmure-space-switch .ambre-select-trigger').click(); \
                 return o;",
                json!([]),
            )
            .await
            .to_string();
        for space in std::iter::once(SPACE).chain(OTHER_SPACES) {
            assert!(spaces.contains(space), "{space} in {spaces}");
        }
        wait_unread(&c.me, "roadmap", 4).await;
        c.me.browser
            .wait_for(
                "two mentions in incidents",
                &format!(
                    "const e = {}; const m = e && e.querySelector('.murmure-mentions'); \
                     return !!m && m.textContent === '2';",
                    entry_js("incidents")
                ),
                STEP,
            )
            .await;
        open(&c.me, "design").await;
        let deadline = Instant::now() + ROSTER_WAIT;
        loop {
            focus_message(&c.me, CHANNEL, LIVE).await;
            let heard = c.me.announced().await;
            if in_call(&heard, &[&n.lea, &n.karim, &n.ines]) {
                break;
            }
            assert!(Instant::now() < deadline, "the call message read {heard:?}");
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        focus_message(&c.me, CHANNEL, "chrome mockup").await;
        let heard = c.me.announced().await;
        assert!(heard.contains("4 replies"), "{heard}");
    }

    /// One device of the mockup's `#seg-device`, and the viewport the app takes for it.
    struct Device {
        name: &'static str,
        width: u32,
        height: u32,
        mobile: bool,
    }

    /// The desktop of the story, and the inner screen of the mockup's phone frame.
    const DEVICES: [Device; 2] = [
        Device {
            name: "desktop",
            width: 1440,
            height: 900,
            mobile: false,
        },
        Device {
            name: "phone",
            width: 390,
            height: 800,
            mobile: true,
        },
    ];

    /// The themes of the mockup's `#seg-theme` the shots take.
    const THEMES: [&str; 2] = ["light", "dark"];

    /// The states of the mockup's `#seg-state`, in the order the user's page reaches them.
    const STATES: [&str; 6] = ["channel", "band", "fill", "scene", "bubble", "present"];

    /// Where the shots land: an ignored directory, emptied by each run.
    fn shots_dir() -> PathBuf {
        let dir = std::env::var_os("MURMURE_SHOTS").map_or_else(
            || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/murmure-shots"),
            PathBuf::from,
        );
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory for the shots");
        dir
    }

    /// Whether the thread panel of `page` shows.
    async fn thread_open(page: &Page) -> bool {
        page.browser
            .script(
                "const c = document.querySelector('#murmure-thread-canvas'); \
                 return !!c && c.getBoundingClientRect().width > 0;",
                json!([]),
            )
            .await
            == json!(true)
    }

    /// Brings the user's page to `state`, and answers the name of the closest state when the
    /// app cannot reach `state` itself.
    async fn reach(c: &Cast, state: &str) -> Option<&'static str> {
        let me = &c.me;
        match state {
            "channel" => {
                open(me, "design").await;
                open_thread(me, "chrome mockup").await;
                None
            }
            "band" => {
                // Escape gives the thread back, and the call band shows above the composer.
                let deadline = Instant::now() + STEP;
                while thread_open(me).await {
                    let at = reachable(me, THREAD, false).await;
                    me.browser.perform(json!([drag(at, at, 0)])).await;
                    press(me, &["\u{E00C}"]).await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    assert!(Instant::now() < deadline, "Escape left the thread open");
                }
                join_call(&c.tom).await;
                camera_on(&c.tom).await;
                join_call(me).await;
                set_mute(me, "true").await;
                camera_on(me).await;
                None
            }
            "fill" => {
                click_header(me, ".murmure-call-fill").await;
                None
            }
            "scene" => {
                // The band comes back first, so the shot shows what a screen does on its own.
                click_header(me, ".murmure-call-fill").await;
                press_header(&c.ines, ".murmure-call-share").await;
                share_pressed(&c.ines, "true").await;
                None
            }
            "bubble" => {
                // The call follows no member into Prose yet (`MURMURE-726`): the bubble floats
                // over another channel.
                open(me, "general").await;
                Some("bubble-over-a-channel")
            }
            "present" => Some("bubble-over-a-channel"),
            other => panic!("no state {other} in the mockup"),
        }
    }

    /// Sets the viewport of `browser` to `device` at a device scale of 2, and its colour
    /// scheme to `theme`.
    async fn emulate(browser: &Browser, device: &Device, theme: &str) {
        browser
            .cdp(
                "Emulation.setDeviceMetricsOverride",
                json!({ "width": device.width, "height": device.height,
                    "deviceScaleFactor": 2, "mobile": device.mobile }),
            )
            .await;
        browser
            .cdp(
                "Emulation.setEmulatedMedia",
                json!({ "features": [{ "name": "prefers-color-scheme", "value": theme }] }),
            )
            .await;
    }

    /// Sets the theme of the app to `theme` through the account menu, as a person does. The
    /// page reads the colour scheme of the system at its boot alone (`UI-THREAD-066`).
    async fn app_theme(page: &Page, theme: &str) {
        let now = page
            .browser
            .script(
                "return document.documentElement.getAttribute('data-theme') || 'light';",
                json!([]),
            )
            .await;
        if now == json!(theme) {
            return;
        }
        page.browser
            .script(
                "document.querySelector('.ambre-account-menu .ambre-header-menu-button').click(); \
                 return true;",
                json!([]),
            )
            .await;
        page.browser
            .wait_for(
                "the theme item of the account menu",
                "const i = document.querySelector('.ambre-menu-theme'); \
                 if (!i) return false; i.click(); return true;",
                STEP,
            )
            .await;
        let theme = Value::String(theme.to_owned());
        page.browser
            .wait_for(
                &format!("the theme {theme}"),
                &format!(
                    "return document.documentElement.getAttribute('data-theme') === {theme} \
                       && !document.querySelector('.ambre-menu-theme');"
                ),
                STEP,
            )
            .await;
        // The toast that names the new theme fades before a shot.
        page.browser
            .wait_for(
                "the toast of the theme gone",
                "return ![...document.querySelectorAll('[role=status]')] \
                   .some(t => t.textContent.includes(' theme'));",
                STEP,
            )
            .await;
    }

    /// The PNG bytes of a DevTools capture of `browser`, cut to `clip` when one is given.
    async fn capture(browser: &Browser, clip: Option<Value>) -> Vec<u8> {
        use base64::Engine as _;

        let mut params = json!({ "format": "png" });
        if let Some(clip) = clip {
            params["clip"] = clip;
            params["captureBeyondViewport"] = json!(true);
        }
        let shot = browser.cdp("Page.captureScreenshot", params).await;
        base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().expect("the data of a capture"))
            .expect("a base64 capture")
    }

    /// Opens the mockup in a browser of its own.
    async fn open_mockup() -> Browser {
        let downloads = std::env::temp_dir().join(format!("belt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&downloads).expect("a download directory");
        let browser = Browser::start(&downloads).await;
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../research/mockups/murmure-calls.html")
            .canonicalize()
            .expect("the Murmure mockup");
        browser.goto(&format!("file://{}", file.display())).await;
        browser
            .cdp(
                "Emulation.setDeviceMetricsOverride",
                json!({ "width": 1440, "height": 1200, "deviceScaleFactor": 2, "mobile": false }),
            )
            .await;
        browser
    }

    /// The render of the mockup in `state`, on `device`, in `theme`: the toggles pressed as
    /// a person presses them, then the stage alone.
    async fn mockup_shot(
        browser: &Browser,
        state: &str,
        device: &str,
        theme: &str,
        switcher: bool,
    ) -> Vec<u8> {
        let rect = browser
            .wait_for(
                "the stage of the mockup",
                &format!(
                    "const press = (seg, v) => document.querySelector(`#${{seg}} button[data-v=\"${{v}}\"]`).click(); \
                     press('seg-state', {state}); press('seg-device', {device}); \
                     press('seg-theme', {theme}); const switcher = {switcher}; \
                     const s = document.getElementById('stage'); \
                     if (!s.firstElementChild) return null; \
                     if (switcher && !document.querySelector('.space-pop')) \
                       document.getElementById('space-btn').click(); \
                     const r = s.getBoundingClientRect(); \
                     return [r.left + scrollX, r.top + scrollY, r.width, r.height];",
                    state = Value::String(state.to_owned()),
                    device = Value::String(device.to_owned()),
                    theme = Value::String(theme.to_owned()),
                    switcher = switcher,
                ),
                STEP,
            )
            .await;
        let f = |i: usize| rect[i].as_f64().expect("a side of the stage");
        // The fonts and the transitions of the mockup settle before the capture.
        tokio::time::sleep(Duration::from_millis(400)).await;
        capture(
            browser,
            Some(json!({ "x": f(0), "y": f(1), "width": f(2), "height": f(3), "scale": 1 })),
        )
        .await
    }

    /// The image that sets `app` on the left and `mockup` on the right, both scaled to the
    /// taller height, on a neutral ground.
    fn side_by_side(app: &[u8], mockup: &[u8]) -> Vec<u8> {
        use image::imageops::FilterType;

        let read = |bytes: &[u8]| {
            image::load_from_memory(bytes)
                .expect("a PNG shot")
                .to_rgba8()
        };
        let (app, mockup) = (read(app), read(mockup));
        let height = app.height().max(mockup.height());
        let scaled = |img: image::RgbaImage| {
            if img.height() == height {
                return img;
            }
            let width =
                u32::try_from(u64::from(img.width()) * u64::from(height) / u64::from(img.height()))
                    .expect("a width that fits");
            image::imageops::resize(&img, width, height, FilterType::Triangle)
        };
        let (app, mockup) = (scaled(app), scaled(mockup));
        let gap = 48;
        let mut out = image::RgbaImage::from_pixel(
            app.width() + gap + mockup.width(),
            height,
            image::Rgba([128, 128, 128, 255]),
        );
        image::imageops::replace(&mut out, &app, 0, 0);
        image::imageops::replace(&mut out, &mockup, i64::from(app.width() + gap), 0);
        let mut png = Vec::new();
        out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("a PNG encoded");
        png
    }

    /// Writes the app shot `app`, the mockup render `render` and the pair under `stem`.
    fn write_pair(dir: &Path, stem: &str, app_name: &str, app: &[u8], render: &[u8]) {
        std::fs::write(dir.join(app_name), app).expect("the app shot written");
        std::fs::write(dir.join(format!("{stem}.mockup.png")), render)
            .expect("the mockup render written");
        std::fs::write(
            dir.join(format!("{stem}.pair.png")),
            side_by_side(app, render),
        )
        .expect("the pair written");
    }

    /// Opens the space switcher of the user's page, and waits for its list.
    async fn open_switcher(page: &Page) {
        page.browser
            .wait_for(
                "the list of the space switcher",
                "const t = document.querySelector('.murmure-space-switch .ambre-select-trigger'); \
                 if (!t) return false; \
                 if (t.getAttribute('aria-expanded') === 'true') return true; \
                 t.click(); return false;",
                STEP,
            )
            .await;
    }

    /// Closes what the switcher or the prompt of a new space left open.
    async fn close_overlays(page: &Page) {
        press(page, &["\u{E00C}"]).await;
        page.browser
            .wait_for(
                "the switcher and the prompt closed",
                "const open = [...document.querySelectorAll('.murmure-new-space-toggle')] \
                   .filter(t => t.getAttribute('aria-expanded') === 'true'); \
                 const s = document.querySelector('.murmure-space-switch .ambre-select-trigger'); \
                 if (s && s.getAttribute('aria-expanded') === 'true') open.push(s); \
                 open.forEach(t => t.click()); \
                 return !open.length;",
                STEP,
            )
            .await;
    }

    /// The shots of the space switcher, taken in the state `channel`. On a desktop, the open
    /// switcher of the app pairs with the open switcher of the mockup. So does the screen the
    /// control of a new space opens, which the mockup draws no further than that entry. The
    /// mockup's phone has no space button, so the phone shot pairs with its plain channel.
    async fn switcher_shots(c: &Cast, mockup: &Browser, dir: &Path, written: &mut Vec<String>) {
        for device in &DEVICES {
            for theme in THEMES {
                emulate(&c.me.browser, device, theme).await;
                app_theme(&c.me, theme).await;
                tokio::time::sleep(Duration::from_millis(1000)).await;
                let desktop = !device.mobile;
                let render = mockup_shot(mockup, "channel", device.name, theme, desktop).await;
                open_switcher(&c.me).await;
                tokio::time::sleep(Duration::from_millis(400)).await;
                let app = capture(&c.me.browser, None).await;
                let stem = format!("switcher-{}-{theme}", device.name);
                let name = if desktop {
                    format!("{stem}.app.png")
                } else {
                    format!("{stem}.app-beside-the-phone-channel.png")
                };
                write_pair(dir, &stem, &name, &app, &render);
                written.push(name);
                close_overlays(&c.me).await;
                if desktop {
                    c.me.browser
                        .wait_for(
                            "the field of a new space",
                            "if (document.querySelector('.murmure-new-space-field')) return true; \
                             document.querySelector('.murmure-new-space-toggle')?.click(); \
                             return false;",
                            STEP,
                        )
                        .await;
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    let app = capture(&c.me.browser, None).await;
                    let stem = format!("new-space-{}-{theme}", device.name);
                    let name = format!("{stem}.app-beside-the-open-switcher.png");
                    write_pair(dir, &stem, &name, &app, &render);
                    written.push(name);
                    close_overlays(&c.me).await;
                }
            }
        }
        emulate(&c.me.browser, &DEVICES[0], "light").await;
    }

    /// The recipe murmure-shots runs this test. The seed writes to the database that
    /// `SHOTS_DATABASE_URL` names. Each state, device and theme then writes its app shot, its
    /// mockup render and the pair into the shots directory of the target.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "writes the Murmure screenshots for a person; run just murmure-shots"]
    async fn write_the_murmure_shots() {
        let database = std::env::var("SHOTS_DATABASE_URL").expect(
            "SHOTS_DATABASE_URL names an empty database: run just murmure-shots, which makes one",
        );
        let dir = shots_dir();
        let c = cast(Names::mockup(), Some(database)).await;
        seed(&c).await;
        let mockup = open_mockup().await;
        let mut written = Vec::new();
        for state in STATES {
            let closest = reach(&c, state).await;
            for device in &DEVICES {
                for theme in THEMES {
                    emulate(&c.me.browser, device, theme).await;
                    app_theme(&c.me, theme).await;
                    // The page lays out the new viewport and paints its canvases again.
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    let app = capture(&c.me.browser, None).await;
                    let render = mockup_shot(&mockup, state, device.name, theme, false).await;
                    let stem = format!("{state}-{}-{theme}", device.name);
                    let app_name = closest.map_or_else(
                        || format!("{stem}.app.png"),
                        |near| format!("{stem}.app-closest-{near}.png"),
                    );
                    write_pair(&dir, &stem, &app_name, &app, &render);
                    written.push(app_name);
                }
            }
            emulate(&c.me.browser, &DEVICES[0], "light").await;
            if state == "channel" {
                switcher_shots(&c, &mockup, &dir, &mut written).await;
            }
        }
        println!(
            "MURMURE-SHOTS {}",
            json!({ "dir": dir.display().to_string(), "app": written })
        );
    }
}
