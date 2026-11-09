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
    };
    let key = FixedBytes::from_array(relay_key().verifying_key().to_bytes());
    let admitting = Admitting {
        key: AdmissionKey::new(&key).expect("a key"),
        turn: Some(TurnServers::new(
            vec![
                ambre_core::Validated::<TurnUrl>::parse("turn:127.0.0.1:3478?transport=tcp")
                    .expect("a URL"),
            ],
            TurnSecret::new("north-wind".to_owned()),
        )),
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
    format!("ws://{}/call", addresses.signal)
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
/// the answer to its offer. One admission seats one connection.
#[tokio::test(flavor = "multi_thread")]
async fn a_socket_is_served_on_a_signed_admission_alone() {
    let url = sfu().await;
    let admission = |participant: u8, expires: At<UnixSec>| Admission {
        room: FixedBytes::from_array([1; 16]),
        participant: FixedBytes::from_array([participant; 16]),
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
    assert!(next(&mut twice).await.is_some_and(|v| v["ice"].is_array()));
    offer(&mut twice).await;
    assert_eq!(
        next(&mut twice).await,
        None,
        "a seated admission seats no second connection"
    );
}
