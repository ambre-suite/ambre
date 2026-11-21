//! The signalling socket of the SFU, served in process: the admission and the TURN credential.

use std::time::{Duration, Instant};

use ambre_core::{FixedBytes, Instant as At, UnixSec};
use ambre_protocol::signaling::Admission;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::{Addresses, Admitting, Wire, run, unix_now};
use crate::admit::{AdmissionKey, signed};
use crate::secrets::TurnSecret;
use crate::sim::client::Synth;
use crate::turn::{TurnServers, TurnUrl};

/// The key the test signs admissions with, in the place of the relay's.
fn relay_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[3; 32])
}

/// An SFU on free loopback ports that admits what [`relay_key`] signs, and hands one TURN
/// server. Answers its signalling URL.
async fn sfu() -> String {
    format!("ws://{}/call", sfu_at().await.signal)
}

/// As [`sfu`], and answers the three addresses.
async fn sfu_at() -> Addresses {
    sfu_with(
        ambre_metrics::ScrapeAccess::Loopback,
        crate::turn::TURN_LIFETIME,
        crate::turn::TURN_MARGIN,
    )
    .await
}

/// As [`sfu_at`], with its metrics listener behind `scrape`.
async fn sfu_scraped(scrape: ambre_metrics::ScrapeAccess) -> Addresses {
    sfu_with(scrape, crate::turn::TURN_LIFETIME, crate::turn::TURN_MARGIN).await
}

/// As [`sfu_at`], each TURN credential valid for `lifetime` and renewed `margin` before its
/// expiry.
async fn sfu_lasting(
    lifetime: ambre_core::TimeSpan<ambre_core::Sec>,
    margin: ambre_core::TimeSpan<ambre_core::Sec>,
) -> Addresses {
    sfu_with(ambre_metrics::ScrapeAccess::Loopback, lifetime, margin).await
}

/// As [`sfu_at`], with its metrics listener behind `scrape` and each TURN credential valid for
/// `lifetime`, renewed `margin` before its expiry.
async fn sfu_with(
    scrape: ambre_metrics::ScrapeAccess,
    lifetime: ambre_core::TimeSpan<ambre_core::Sec>,
    margin: ambre_core::TimeSpan<ambre_core::Sec>,
) -> Addresses {
    let free = |udp: bool| {
        if udp {
            std::net::UdpSocket::bind("127.0.0.1:0").and_then(|s| s.local_addr())
        } else {
            std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr())
        }
        .expect("a free port")
    };
    let addresses = Addresses {
        signal: free(false),
        media: free(true),
        metrics: free(false),
    };
    let key = FixedBytes::from_array(relay_key().verifying_key().to_bytes());
    let admitting = Admitting {
        key: AdmissionKey::new(&key).expect("a key"),
        turn: Some(
            TurnServers::new(
                vec![
                    ambre_core::Validated::<TurnUrl>::parse("turn:127.0.0.1:3478?transport=tcp")
                        .expect("a URL"),
                ],
                TurnSecret::new("north-wind".to_owned()),
            )
            .lasting(lifetime, margin),
        ),
        scrape,
    };
    tokio::spawn(run(addresses, admitting));
    let deadline = Instant::now() + Duration::from_secs(10);
    while tokio::net::TcpStream::connect(addresses.signal)
        .await
        .is_err()
    {
        assert!(Instant::now() < deadline, "the SFU did not listen");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    addresses
}

/// The status line, the headers and the body of a `GET` of `path` on `at`, over HTTP/1.1.
async fn http_get(at: std::net::SocketAddr, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(at)
        .await
        .expect("a connection");
    let request = format!("GET {path} HTTP/1.1\r\nHost: sfu\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("the request");
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .await
        .expect("the answer");
    answer
}

/// The status line, the headers and the body of a `GET` of `path` on `at` that presents
/// `authorization`, over HTTP/1.1.
async fn http_get_with(at: std::net::SocketAddr, path: &str, authorization: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(at)
        .await
        .expect("a connection");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: sfu\r\nAuthorization: {authorization}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("the request");
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .await
        .expect("the answer");
    answer
}

/// `CALL-153`: a metrics listener behind a token answers the scrape that presents it, and
/// `401` with no metric to any other. An address off the loopback with no token refuses to
/// start.
#[tokio::test(flavor = "multi_thread")]
async fn a_scrape_behind_a_token_presents_it() {
    use ambre_core::{Credential, ScrapeToken};
    use ambre_metrics::ScrapeAccess;

    let shared: std::net::SocketAddr = "10.0.0.7:3482".parse().expect("an address");
    assert!(ScrapeAccess::for_listener(shared, None).is_err());
    let token = Credential::<ScrapeToken>::new("south-wind".to_owned());
    let addresses = sfu_scraped(ScrapeAccess::Token(token)).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while tokio::net::TcpStream::connect(addresses.metrics)
        .await
        .is_err()
    {
        assert!(Instant::now() < deadline, "the metrics did not listen");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let admitted = http_get_with(
        addresses.metrics,
        crate::metrics::ROUTE,
        "Bearer south-wind",
    )
    .await;
    assert!(admitted.starts_with("HTTP/1.1 200"), "{admitted}");
    assert!(admitted.contains("ambre_sfu_participants"), "{admitted}");
    for refused in [
        http_get(addresses.metrics, crate::metrics::ROUTE).await,
        http_get_with(
            addresses.metrics,
            crate::metrics::ROUTE,
            "Bearer north-wind",
        )
        .await,
    ] {
        assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");
        assert!(!refused.contains("ambre_sfu"), "{refused}");
    }
}

/// `CALL-101`, `ZK-029`, `ZK-031`: the metrics answer in the Prometheus text format on the
/// metrics port alone, and the signalling port serves none. Every label value is the spelling
/// of a variant of its closed `enum`. So no series names a room, a participant or an address.
/// The count of series is the one of the start.
#[tokio::test(flavor = "multi_thread")]
async fn the_metrics_answer_on_their_own_port_under_closed_labels_alone() {
    use crate::metrics::{Admission as Seat, Direction};
    use ambre_metrics::Label;

    let addresses = sfu_at().await;
    let url = format!("ws://{}/call", addresses.signal);
    let admission = Admission {
        room: FixedBytes::from_array([7; 16]),
        participant: FixedBytes::from_array([8; 16]),
        member: FixedBytes::from_array([8; 16]),
        sender: ambre_core::Seq::new(8),
        expires: unix_now().plus(ambre_core::TimeSpan::new(60)),
    };
    let mut socket = admitted(&url, &admission, &relay_key()).await;
    assert!(next(&mut socket).await.is_some_and(|v| v["ice"].is_array()));
    offer(&mut socket).await;
    assert!(
        next(&mut socket)
            .await
            .is_some_and(|v| v["answer"].is_string())
    );

    let scrape = http_get(addresses.metrics, crate::metrics::ROUTE).await;
    let (head, body) = scrape.split_once("\r\n\r\n").expect("a head and a body");
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head.contains("text/plain; version=0.0.4"), "{head}");
    let signal = http_get(addresses.signal, crate::metrics::ROUTE).await;
    assert!(
        signal.starts_with("HTTP/1.1 404"),
        "the signalling port serves no metric: {signal}"
    );

    let spellings = |all: &[&str]| all.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let directions = spellings(
        &Direction::ALL
            .iter()
            .map(|d| d.value().as_str())
            .collect::<Vec<_>>(),
    );
    let outcomes = spellings(
        &Seat::ALL
            .iter()
            .map(|o| o.value().as_str())
            .collect::<Vec<_>>(),
    );
    let samples: Vec<&str> = body.lines().filter(|l| !l.starts_with('#')).collect();
    for sample in &samples {
        let (series, _) = sample.rsplit_once(' ').expect("a series and a value");
        match series.split_once('{') {
            None => assert!(
                ["ambre_sfu_participants", "ambre_sfu_rooms"].contains(&series),
                "an unlabelled series: {series}"
            ),
            Some((name, labels)) => {
                let (key, value) = labels
                    .strip_suffix('}')
                    .and_then(|l| l.split_once('='))
                    .expect("one label");
                let value = value.trim_matches('"').to_owned();
                match (name, key) {
                    ("ambre_sfu_datagrams_total", "direction") => {
                        assert!(directions.contains(&value), "{sample}");
                    }
                    ("ambre_sfu_admissions_total", "outcome") => {
                        assert!(outcomes.contains(&value), "{sample}");
                    }
                    _ => panic!("a label no closed set declares: {sample}"),
                }
            }
        }
    }
    assert_eq!(samples.len(), 2 + Direction::ALL.len() + Seat::ALL.len());
    let pseudonyms = ["07".repeat(16), "08".repeat(16)];
    assert!(
        pseudonyms.iter().all(|p| !body.contains(p.as_str())) && !body.contains("127.0.0.1"),
        "no pseudonym and no address in a scrape: {body}"
    );
    let seated = samples
        .iter()
        .find_map(|l| l.strip_prefix("ambre_sfu_admissions_total{outcome=\"seated\"} "))
        .and_then(|v| v.parse::<u64>().ok())
        .expect("the seated series");
    assert!(seated >= 1, "the seat is counted");
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A socket that sent `admission` as its first frame.
async fn admitted(url: &str, admission: &Admission, key: &ed25519_dalek::SigningKey) -> Socket {
    let (mut socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("a socket");
    let token = signed(key, admission);
    let text = serde_json::to_string(&Wire::Admission(token)).expect("a frame");
    socket
        .send(Message::Text(text.into()))
        .await
        .expect("the admission");
    socket
}

/// The next text frame as JSON, or `None` once the socket closes or falls silent for 3 s.
async fn next(socket: &mut Socket) -> Option<Value> {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .ok()??
            .ok()?;
        match frame {
            Message::Text(text) => return serde_json::from_str(text.as_str()).ok(),
            Message::Close(_) => return None,
            _ => {}
        }
    }
}

/// Sends an offer of a synthetic participant on `socket`.
async fn offer(socket: &mut Socket) {
    let now = Instant::now();
    let (_, offer) = Synth::new(now, now, 1, "127.0.0.1:9".parse().expect("an address"));
    let text = json!({ "offer": offer.to_sdp_string() }).to_string();
    socket
        .send(Message::Text(text.into()))
        .await
        .expect("the offer");
}

/// `ZK-023`: the SFU serves a socket on an admission the relay signed alone. A
/// forged and an expired admission close the socket before the SFU reads anything more. An
/// admitted participant first reads a TURN credential the SFU minted under its pseudonym, then
/// the answer to its offer. One admission serves one socket.
#[tokio::test(flavor = "multi_thread")]
async fn a_socket_is_served_on_a_signed_admission_alone() {
    let url = sfu().await;
    let admission = |participant: u8, expires: At<UnixSec>| Admission {
        room: FixedBytes::from_array([1; 16]),
        participant: FixedBytes::from_array([participant; 16]),
        member: FixedBytes::from_array([participant; 16]),
        sender: ambre_core::Seq::new(u64::from(participant)),
        expires,
    };
    let later = unix_now().plus(ambre_core::TimeSpan::new(60));

    let forger = ed25519_dalek::SigningKey::from_bytes(&[4; 32]);
    let mut forged = admitted(&url, &admission(2, later), &forger).await;
    assert_eq!(
        next(&mut forged).await,
        None,
        "a forged admission is served nothing"
    );
    let mut expired = admitted(&url, &admission(2, At::new(1)), &relay_key()).await;
    assert_eq!(
        next(&mut expired).await,
        None,
        "an expired admission is served nothing"
    );

    let mut socket = admitted(&url, &admission(2, later), &relay_key()).await;
    let ice = next(&mut socket).await.expect("the TURN servers");
    let server = &ice["ice"][0];
    assert_eq!(server["urls"], json!(["turn:127.0.0.1:3478?transport=tcp"]));
    let username = server["username"].as_str().expect("a username");
    let (expiry, name) = username.split_once(':').expect("an expiry and a name");
    assert_eq!(
        name,
        "02".repeat(16),
        "the credential names the pseudonym alone"
    );
    assert!(expiry.parse::<u64>().expect("an expiry") > later.get());
    assert!(server["credential"].as_str().is_some_and(|c| c.len() == 28));
    offer(&mut socket).await;
    let answer = next(&mut socket).await.expect("an answer");
    assert!(answer["answer"].is_string(), "{answer}");

    let mut twice = admitted(&url, &admission(2, later), &relay_key()).await;
    assert_eq!(
        next(&mut twice).await,
        None,
        "a seated admission serves a second socket"
    );
}

/// `CALL-117`, audit `N12`: an admission whose connection left seats nobody again while it
/// is still valid. A captured token is worth one seat, never a second one after the first.
/// The second socket reads no TURN credential, and the SFU reads no offer of it.
#[tokio::test(flavor = "multi_thread")]
async fn an_admission_seats_no_connection_after_its_own_left() {
    let url = sfu().await;
    let admission = Admission {
        room: FixedBytes::from_array([1; 16]),
        participant: FixedBytes::from_array([5; 16]),
        member: FixedBytes::from_array([5; 16]),
        sender: ambre_core::Seq::new(5),
        expires: unix_now().plus(ambre_core::TimeSpan::new(60)),
    };
    let mut first = admitted(&url, &admission, &relay_key()).await;
    assert!(next(&mut first).await.is_some_and(|v| v["ice"].is_array()));
    offer(&mut first).await;
    assert!(
        next(&mut first)
            .await
            .is_some_and(|v| v["answer"].is_string())
    );
    first.close(None).await.expect("a close");
    drop(first);
    // The leave reaches the media loop after the close.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut again = admitted(&url, &admission, &relay_key()).await;
    assert_eq!(
        next(&mut again).await,
        None,
        "a spent admission serves a second socket"
    );
}

/// Whether `socket` closes within `span`. A frame read on the way is skipped.
async fn closes_within(socket: &mut Socket, span: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + span;
    loop {
        match tokio::time::timeout_at(deadline, socket.next()).await {
            Err(_) => return false,
            Ok(None | Some(Err(_) | Ok(Message::Close(_)))) => return true,
            Ok(Some(Ok(_))) => {}
        }
    }
}

/// Sends `admission`, signed by the relay's key, on a socket already seated.
async fn renew(socket: &mut Socket, admission: &Admission) {
    let token = signed(&relay_key(), admission);
    let text = serde_json::to_string(&Wire::Admission(token)).expect("a frame");
    socket
        .send(Message::Text(text.into()))
        .await
        .expect("the renewal");
}

/// A socket seated under `admission`: it read the TURN servers and the answer to its offer.
async fn seated(url: &str, admission: &Admission) -> Socket {
    let mut socket = admitted(url, admission, &relay_key()).await;
    assert!(next(&mut socket).await.is_some_and(|v| v["ice"].is_array()));
    offer(&mut socket).await;
    assert!(
        next(&mut socket)
            .await
            .is_some_and(|v| v["answer"].is_string()),
        "the seat is refused"
    );
    socket
}

/// Audit `N25`: a member who closed its relay socket gets no renewal. The SFU ends its seat
/// once the last admission expires, so the seat leaves the room and the cap.
#[tokio::test(flavor = "multi_thread")]
async fn a_seat_whose_admission_expires_with_no_renewal_ends() {
    let url = sfu().await;
    let admission = Admission {
        room: FixedBytes::from_array([1; 16]),
        participant: FixedBytes::from_array([20; 16]),
        member: FixedBytes::from_array([20; 16]),
        sender: ambre_core::Seq::new(20),
        expires: unix_now().plus(ambre_core::TimeSpan::new(2)),
    };
    let mut socket = seated(&url, &admission).await;
    assert!(
        closes_within(&mut socket, Duration::from_secs(8)).await,
        "a seat outlived its admission"
    );
}

/// Audit `N25`, `CALL-120`: a renewal of the same seat keeps it past its first expiry. A
/// renewal that names another participant, member or sender index keeps nothing. The token
/// of a renewal seats no second connection while it is valid.
#[tokio::test(flavor = "multi_thread")]
async fn a_renewal_keeps_its_own_seat_alone() {
    let url = sfu().await;
    let first = |participant: u8| Admission {
        room: FixedBytes::from_array([1; 16]),
        participant: FixedBytes::from_array([participant; 16]),
        member: FixedBytes::from_array([participant; 16]),
        sender: ambre_core::Seq::new(u64::from(participant)),
        expires: unix_now().plus(ambre_core::TimeSpan::new(2)),
    };
    let later = unix_now().plus(ambre_core::TimeSpan::new(60));
    let kept = first(21);
    let mut socket = seated(&url, &kept).await;
    let renewal = Admission {
        expires: later,
        ..kept.clone()
    };
    renew(&mut socket, &renewal).await;

    let mut others = Vec::new();
    for (index, mutate) in [
        |a: &mut Admission| a.participant = FixedBytes::from_array([99; 16]),
        |a: &mut Admission| a.member = FixedBytes::from_array([99; 16]),
        |a: &mut Admission| a.sender = ambre_core::Seq::new(99),
    ]
    .into_iter()
    .enumerate()
    {
        let own = first(22 + u8::try_from(index).expect("a small index"));
        let mut other = seated(&url, &own).await;
        let mut wrong = Admission {
            expires: later,
            ..own
        };
        mutate(&mut wrong);
        renew(&mut other, &wrong).await;
        others.push(other);
    }
    for mut other in others {
        assert!(
            closes_within(&mut other, Duration::from_secs(8)).await,
            "a renewal of another seat kept this one"
        );
    }
    assert!(
        !closes_within(&mut socket, Duration::from_secs(1)).await,
        "a renewal of the seat did not keep it"
    );

    let mut again = admitted(&url, &renewal, &relay_key()).await;
    assert_eq!(
        next(&mut again).await,
        None,
        "the token of a renewal serves a second socket"
    );
}

/// Audit `N25`: a member holds one seat of a room. A join under another admission of the
/// same member ends the older seat, so seats a member stopped renewing never fill the room.
#[tokio::test(flavor = "multi_thread")]
async fn a_join_of_a_member_ends_its_older_seat() {
    let url = sfu().await;
    let later = unix_now().plus(ambre_core::TimeSpan::new(60));
    let admission = |participant: u8, member: u8| Admission {
        room: FixedBytes::from_array([1; 16]),
        participant: FixedBytes::from_array([participant; 16]),
        member: FixedBytes::from_array([member; 16]),
        sender: ambre_core::Seq::new(u64::from(participant)),
        expires: later,
    };
    let mut older = seated(&url, &admission(30, 40)).await;
    let mut stranger = seated(&url, &admission(31, 41)).await;
    let mut newer = seated(&url, &admission(32, 40)).await;
    assert!(
        closes_within(&mut older, Duration::from_secs(3)).await,
        "a member holds two seats"
    );
    assert!(!closes_within(&mut stranger, Duration::from_secs(1)).await);
    assert!(!closes_within(&mut newer, Duration::from_secs(1)).await);
}

/// An admission of a fresh participant and member to room 1, valid for 60 s.
fn fresh(participant: u8) -> Admission {
    Admission {
        room: FixedBytes::from_array([1; 16]),
        participant: FixedBytes::from_array([participant; 16]),
        member: FixedBytes::from_array([participant; 16]),
        sender: ambre_core::Seq::new(u64::from(participant)),
        expires: unix_now().plus(ambre_core::TimeSpan::new(60)),
    }
}

/// `CALL-150`, audit `N39`: a message one byte past the bound closes the socket, before and
/// after the admission. A socket under the bound reads its TURN servers and its answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_past_the_bound_closes_the_socket() {
    let url = sfu().await;
    let past = usize::try_from(super::SIGNAL_MESSAGE_MAX.get())
        .expect("a bound that fits")
        .saturating_add(1);
    let big = |field: &str| {
        let mut frame = serde_json::Map::new();
        frame.insert(field.to_owned(), Value::String("a".repeat(past)));
        Value::Object(frame).to_string()
    };

    let (mut first, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("a socket");
    // A frame of the bound alone, past it by its framing: the socket closes on its read.
    let _ = first.send(Message::Text(big("admission").into())).await;
    assert!(
        closes_within(&mut first, Duration::from_secs(3)).await,
        "a first frame past the bound kept the socket"
    );

    let mut admitted_socket = admitted(&url, &fresh(40), &relay_key()).await;
    assert!(
        next(&mut admitted_socket)
            .await
            .is_some_and(|v| v["ice"].is_array())
    );
    let _ = admitted_socket
        .send(Message::Text(big("offer").into()))
        .await;
    assert!(
        closes_within(&mut admitted_socket, Duration::from_secs(3)).await,
        "an offer past the bound kept the socket"
    );

    seated(&url, &fresh(41)).await;
}

/// `CALL-151`, audit `N39`: an admitted socket that sends no offer closes once the deadline
/// passes, and not before it.
#[tokio::test(flavor = "multi_thread")]
async fn an_admitted_socket_with_no_offer_closes_at_the_deadline() {
    let url = sfu().await;
    let mut socket = admitted(&url, &fresh(42), &relay_key()).await;
    assert!(next(&mut socket).await.is_some_and(|v| v["ice"].is_array()));
    let deadline = super::ADMISSION_DEADLINE;
    assert!(
        !closes_within(&mut socket, deadline.saturating_sub(Duration::from_secs(2))).await,
        "the socket closed before the deadline"
    );
    assert!(
        closes_within(&mut socket, Duration::from_secs(5)).await,
        "a socket with no offer outlived the deadline"
    );
}

/// `CALL-152`, audit `N39`: one socket under an admission waits for its offer. A second
/// socket under the same admission reads no TURN credential, and closes. So one admission
/// mints one credential, and holds one unfinished message at most.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_socket_under_one_admission_reads_no_credential() {
    let url = sfu().await;
    let admission = fresh(43);
    let mut first = admitted(&url, &admission, &relay_key()).await;
    assert!(next(&mut first).await.is_some_and(|v| v["ice"].is_array()));
    for _ in 0..3 {
        let mut more = admitted(&url, &admission, &relay_key()).await;
        assert_eq!(
            next(&mut more).await,
            None,
            "a second socket under one admission read something"
        );
    }
    offer(&mut first).await;
    assert!(
        next(&mut first)
            .await
            .is_some_and(|v| v["answer"].is_string()),
        "the first socket lost its seat to the others"
    );
}

/// Whether `socket` sends no text frame within `span`. A close counts as silence.
async fn quiet_for(socket: &mut Socket, span: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + span;
    loop {
        match tokio::time::timeout_at(deadline, socket.next()).await {
            Err(_) | Ok(None | Some(Err(_) | Ok(Message::Close(_)))) => return true,
            Ok(Some(Ok(Message::Text(_)))) => return false,
            Ok(Some(Ok(_))) => {}
        }
    }
}

/// The expiry and the name of the one TURN credential of an `ice` frame, and its password.
fn credential_of(frame: &Value) -> (u64, String, String) {
    let server = &frame["ice"][0];
    let username = server["username"].as_str().expect("a username");
    let (expiry, name) = username.split_once(':').expect("an expiry and a name");
    let password = server["credential"].as_str().expect("a password");
    (
        expiry.parse().expect("an expiry"),
        name.to_owned(),
        password.to_owned(),
    )
}

/// `CALL-154`, audit `P60`: a credential lives the TURN lifetime alone. A renewal that keeps
/// the seat answers a fresh credential under the same pseudonym once the last one expires
/// within the margin, and none before. A renewal of another seat answers none, in the margin
/// too.
#[tokio::test(flavor = "multi_thread")]
async fn a_renewal_near_the_expiry_of_the_credential_answers_a_fresh_one() {
    let lifetime = ambre_core::TimeSpan::new(6);
    let url = format!(
        "ws://{}/call",
        sfu_lasting(lifetime, ambre_core::TimeSpan::new(3))
            .await
            .signal
    );
    let kept = fresh(50);
    let minted = unix_now();
    let mut socket = admitted(&url, &kept, &relay_key()).await;
    let first = next(&mut socket).await.expect("the TURN servers");
    let (first_expiry, name, first_password) = credential_of(&first);
    assert!(
        first_expiry <= minted.plus(lifetime).get() + 1,
        "a credential outlives the TURN lifetime: {first}"
    );
    offer(&mut socket).await;
    assert!(
        next(&mut socket)
            .await
            .is_some_and(|v| v["answer"].is_string())
    );
    let other_seat = fresh(51);
    let mut other = seated(&url, &other_seat).await;

    let renewal = |admission: &Admission| Admission {
        expires: unix_now().plus(ambre_core::TimeSpan::new(61)),
        ..admission.clone()
    };
    renew(&mut socket, &renewal(&kept)).await;
    assert!(
        quiet_for(&mut socket, Duration::from_millis(500)).await,
        "a renewal before the margin answered a credential"
    );

    tokio::time::sleep(Duration::from_millis(3500)).await;
    let mut wrong = renewal(&other_seat);
    wrong.participant = FixedBytes::from_array([99; 16]);
    renew(&mut other, &wrong).await;
    assert!(
        quiet_for(&mut other, Duration::from_millis(500)).await,
        "a renewal of another seat answered a credential"
    );
    renew(&mut socket, &renewal(&kept)).await;
    let again = next(&mut socket).await.expect("a fresh credential");
    let (expiry, again_name, password) = credential_of(&again);
    assert_eq!(
        again_name, name,
        "the fresh credential names another pseudonym"
    );
    assert!(
        expiry > first_expiry,
        "the fresh credential expires no later: {again}"
    );
    assert_ne!(password, first_password);
    renew(&mut socket, &renewal(&kept)).await;
    assert!(
        quiet_for(&mut socket, Duration::from_millis(500)).await,
        "a renewal right after a fresh credential answered another one"
    );
}
