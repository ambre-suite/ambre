use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use str0m::net::{Receive, Transmit};
use str0m::{Candidate, Event, Input, Output, Rtc};

use super::participant_config;

/// One side of an in-memory link: its connection, what it sent, and whether DTLS completed.
struct Side {
    rtc: Rtc,
    sent: VecDeque<Transmit>,
    connected: bool,
    wake: Instant,
}

impl Side {
    fn new(rtc: Rtc, now: Instant) -> Self {
        Self {
            rtc,
            sent: VecDeque::new(),
            connected: false,
            wake: now,
        }
    }

    /// Runs the connection until it asks to be woken, and keeps what it sends.
    fn poll(&mut self) {
        loop {
            match self.rtc.poll_output().expect("poll") {
                Output::Timeout(at) => {
                    self.wake = at;
                    return;
                }
                Output::Transmit(packet) => self.sent.push_back(packet),
                Output::Event(Event::Connected) => self.connected = true,
                Output::Event(_) => {}
            }
        }
    }

    fn receive(&mut self, now: Instant, packet: &Transmit) {
        let contents = (&packet.contents[..])
            .try_into()
            .expect("a datagram str0m sent");
        let input = Input::Receive(
            now,
            Receive {
                proto: packet.proto,
                source: packet.source,
                destination: packet.destination,
                contents,
            },
        );
        self.rtc.handle_input(input).expect("receive");
    }
}

fn address(last: u8, port: u16) -> SocketAddr {
    (Ipv4Addr::new(10, 0, 0, last), port).into()
}

#[test]
fn a_participant_completes_dtls_with_the_sfu_over_rust_crypto() {
    let start = Instant::now();
    let browser = Rtc::builder()
        .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
        .build(start);
    let mut browser = Side::new(browser, start);
    let mut sfu = Side::new(participant_config().build(start), start);

    let browser_host = Candidate::host(address(1, 5000), "udp").expect("candidate");
    let sfu_host = Candidate::host(address(2, 6000), "udp").expect("candidate");
    browser.rtc.add_local_candidate(browser_host.clone());
    browser.rtc.add_remote_candidate(sfu_host.clone());
    sfu.rtc.add_local_candidate(sfu_host);
    sfu.rtc.add_remote_candidate(browser_host);

    let browser_print = browser.rtc.direct_api().local_dtls_fingerprint().clone();
    let sfu_print = sfu.rtc.direct_api().local_dtls_fingerprint().clone();
    browser.rtc.direct_api().set_remote_fingerprint(sfu_print);
    sfu.rtc.direct_api().set_remote_fingerprint(browser_print);
    let browser_creds = browser.rtc.direct_api().local_ice_credentials();
    let sfu_creds = sfu.rtc.direct_api().local_ice_credentials();
    browser
        .rtc
        .direct_api()
        .set_remote_ice_credentials(sfu_creds);
    sfu.rtc
        .direct_api()
        .set_remote_ice_credentials(browser_creds);
    browser.rtc.direct_api().set_ice_controlling(true);
    sfu.rtc.direct_api().set_ice_controlling(false);
    browser.rtc.direct_api().start_dtls(true).expect("dtls");
    sfu.rtc.direct_api().start_dtls(false).expect("dtls");

    let mut now = start;
    let deadline = start + Duration::from_secs(10);
    while !(browser.connected && sfu.connected) {
        assert!(
            now < deadline,
            "no DTLS session within 10 s of simulated time"
        );
        browser.poll();
        sfu.poll();
        while let Some(packet) = browser.sent.pop_front() {
            sfu.receive(now, &packet);
        }
        while let Some(packet) = sfu.sent.pop_front() {
            browser.receive(now, &packet);
        }
        browser.poll();
        sfu.poll();
        if browser.sent.is_empty() && sfu.sent.is_empty() {
            now = browser
                .wake
                .min(sfu.wake)
                .max(now + Duration::from_millis(1));
            browser
                .rtc
                .handle_input(Input::Timeout(now))
                .expect("timeout");
            sfu.rtc.handle_input(Input::Timeout(now)).expect("timeout");
        }
    }
}

/// `CALL-067`: the SFU answers a participant's audio with Opus at the constant bitrate of a
/// call. So the browser encodes no 20 ms frame past the fixed size of a sealed chunk.
#[test]
fn the_sfu_asks_each_sender_for_opus_at_the_constant_bitrate_of_a_call() {
    let start = Instant::now();
    let mut browser = Rtc::builder()
        .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
        .build(start);
    let mut sfu = participant_config().build(start);
    let mut change = browser.sdp_api();
    change.add_media(
        str0m::media::MediaKind::Audio,
        str0m::media::Direction::SendRecv,
        None,
        None,
        None,
    );
    let (offer, _) = change.apply().expect("an offer");
    let answer = sfu.sdp_api().accept_offer(offer).expect("an answer");
    let sdp = answer.to_sdp_string();
    let opus = sdp
        .lines()
        .find_map(|line| {
            let pt = line
                .strip_prefix("a=rtpmap:")?
                .strip_suffix(" opus/48000/2")?;
            sdp.lines()
                .find(|l| l.starts_with(&format!("a=fmtp:{pt} ")))
        })
        .expect("an Opus fmtp in the answer");
    let bitrate = format!("maxaveragebitrate={}", ambre_core::AUDIO_BITRATE.get());
    assert!(opus.contains(&bitrate), "{opus}");
    assert!(opus.contains("cbr=1"), "{opus}");
}
