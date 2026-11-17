//! The seed of a Murmure channel, sent through the application as a person
//! sends it (`ambre-spec-murmure-phone-measures.md section 3.2`).
//!
//! The phone measures need a channel of 5 000 messages on the relay of the
//! phone stack. The seed signs in with the development name the phone signs
//! in with. That name derives the same keys on every device, so the phone
//! opens the space as its owner. A second member watches the channel, and
//! its unread count holds the number of messages the relay delivered.

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use serde_json::json;

    use crate::murmure::tests::{
        CHANNEL, enter, holds_keys, leave_pane, murmure, open, space_with_channel, take_keys,
        typed_and_sent, user, wait_unread_within,
    };
    use crate::page::Page;
    use crate::relay::Relay;

    /// Answers once the page has painted twice. Enter only asks to send, and
    /// the next paint reads the draft. So a message typed before that paint
    /// would join the draft of the one before.
    const PAINTED: &str = "const done = arguments[arguments.length - 1]; \
        requestAnimationFrame(() => requestAnimationFrame(() => done(true)));";

    /// The text of message `n` of `count`. Each text differs, so a message
    /// lost cannot hide behind a duplicate.
    fn text(n: u32, count: u32) -> String {
        format!("Seed {n} of {count}")
    }

    /// How long the watcher may wait for the last message: a minute, plus
    /// 20 ms per message for a relay that drains a backlog.
    fn drain(count: u32) -> Duration {
        Duration::from_secs(60) + Duration::from_millis(20 * u64::from(count))
    }

    /// The name of a seeded channel: its count, and a suffix that keeps two
    /// seeds of one account apart in the sidebar.
    fn channel_name(count: u32) -> String {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        format!("seed-{count}-{}", &suffix[..6])
    }

    /// `MURMURE-672`: two seeds of one count name two channels, so the
    /// second opens no channel the first filled.
    #[test]
    fn two_seeds_of_one_count_name_two_channels() {
        let (first, second) = (channel_name(5000), channel_name(5000));
        assert_ne!(first, second);
        assert!(first.starts_with("seed-5000-"), "{first}");
    }

    /// Fills a new channel of a new space with `count` messages from `seeder`,
    /// then waits until `watcher` counts each one unread. Both pages show
    /// Murmure, signed in as `seeder_name` and `watcher_name`. Answers the
    /// name of the channel.
    async fn seed(seeder: &Page, watcher: &Page, watcher_name: &str, count: u32) -> String {
        let channel = channel_name(count);
        space_with_channel(seeder, "Seed", &channel, watcher_name, watcher).await;
        // The watcher's cursor must be known for a count to replace the mark
        // (`MURMURE-120`): it opens the channel once and goes back.
        open(watcher, &channel).await;
        leave_pane(watcher).await;

        take_keys(seeder, CHANNEL).await;
        for n in 1..=count {
            if !holds_keys(seeder).await {
                take_keys(seeder, CHANNEL).await;
            }
            let actions = typed_and_sent(&text(n, count));
            seeder
                .browser
                .perform(json!([{ "type": "key", "id": "keyboard", "actions": actions }]))
                .await;
            seeder.browser.script_async(PAINTED, json!([])).await;
            if n % 500 == 0 {
                eprintln!("seed: {n} of {count} sent");
            }
        }
        wait_unread_within(watcher, &channel, count, drain(count)).await;
        channel
    }

    /// `MURMURE-671`: a seed of a small count, on the belt's own relay. The
    /// watcher counts every message unread, so the relay delivered each one.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build, chromedriver and the services; run just test-browser"
    )]
    async fn a_seed_fills_its_channel_to_the_count() {
        let relay = std::sync::Arc::new(Relay::start().await);
        let (seeder_name, watcher_name) = (user(), user());
        let seeder = murmure(std::sync::Arc::clone(&relay), &seeder_name).await;
        let watcher = murmure(relay, &watcher_name).await;
        seed(&seeder, &watcher, &watcher_name, 25).await;
    }

    /// `just belt-seed`: a seed of `SEED_MESSAGES` messages, 5 000 by
    /// default, on the belt's own relay. The watcher counts each one within
    /// the drain, so the seeder's Logic thread kept the pace of the keys.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "a seed of 5 000 messages on the belt relay, run by `just belt-seed`"]
    async fn seed_the_belt_relay() {
        let count: u32 = var("SEED_MESSAGES", "5000")
            .parse()
            .expect("SEED_MESSAGES is a count of messages");
        let relay = std::sync::Arc::new(Relay::start().await);
        let (seeder_name, watcher_name) = (user(), user());
        let seeder = murmure(std::sync::Arc::clone(&relay), &seeder_name).await;
        let watcher = murmure(relay, &watcher_name).await;
        let started = std::time::Instant::now();
        let channel = seed(&seeder, &watcher, &watcher_name, count).await;
        println!(
            "BELT-SEED {}",
            json!({
                "channel": channel,
                "messages": count,
                "seconds": started.elapsed().as_secs(),
            })
        );
    }

    /// The variable `name`, or `default` when it is unset.
    fn var(name: &str, default: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| default.to_owned())
    }

    /// `just phone-seed`: fills a channel of the phone stack at
    /// `SEED_ORIGIN` with `SEED_MESSAGES` messages, as `SEED_USER`. The
    /// phone then signs in with that name and opens the channel printed.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "a seed of the phone stack, run by `just phone-seed`"]
    async fn seed_the_phone_stack() {
        let origin = var("SEED_ORIGIN", "https://localhost:8443");
        let name = var("SEED_USER", "phone");
        let count: u32 = var("SEED_MESSAGES", "5000")
            .parse()
            .expect("SEED_MESSAGES is a count of messages");
        let watcher_name = format!("{name}-watcher");
        let seeder = enter(Page::at(&origin).await, &name).await;
        let watcher = enter(Page::at(&origin).await, &watcher_name).await;
        let started = std::time::Instant::now();
        let channel = seed(&seeder, &watcher, &watcher_name, count).await;
        println!(
            "PHONE-SEED {}",
            json!({
                "origin": origin,
                "user": name,
                "space": "Seed",
                "channel": channel,
                "messages": count,
                "seconds": started.elapsed().as_secs(),
            })
        );
    }
}
