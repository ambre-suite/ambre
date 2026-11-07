//! The SFU process: one UDP socket for the media, and a WebSocket for the signalling.
//!
//! The signalling listens on a port of its own, never on the relay (`CALL-036`, `SRV-049`).
//! A browser opens `/room/<name>`, sends its offer, and reads the answer, then every offer the
//! SFU sends when the room gains a track. One task owns the [`Sfu`]: the sockets hand it
//! datagrams and commands, so no lock guards a connection.

use std::net::SocketAddr;
use std::time::Instant;

use ambre_core::{Seq, Validated};
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use serde::{Deserialize, Serialize};
use str0m::change::{SdpAnswer, SdpOffer};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, oneshot};

use crate::forward::{Participant, Room, Sfu, Signal};

/// Where the SFU listens.
#[derive(Clone, Copy, Debug)]
pub struct Addresses {
    /// The TCP address of the signalling WebSocket.
    pub signal: SocketAddr,
    /// The UDP address of the media. It is the candidate each browser connects to, so it names
    /// an address the browsers reach, never an unspecified one.
    pub media: SocketAddr,
}

/// A message on the signalling socket, in either direction: one session description. Its
/// JSON form is `{"offer": "<sdp>"}` or `{"answer": "<sdp>"}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Wire {
    Offer(#[serde(with = "sdp_offer")] SdpOffer),
    Answer(#[serde(with = "sdp_answer")] SdpAnswer),
}

/// The text form of an offer, which str0m parses.
mod sdp_offer {
    use serde::{Deserialize, Deserializer, Serializer};
    use str0m::change::SdpOffer;

    pub fn serialize<S: Serializer>(offer: &SdpOffer, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_str(&offer.to_sdp_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<SdpOffer, D::Error> {
        let text = String::deserialize(input)?;
        SdpOffer::from_sdp_string(&text).map_err(serde::de::Error::custom)
    }
}

/// The text form of an answer, which str0m parses.
mod sdp_answer {
    use serde::{Deserialize, Deserializer, Serializer};
    use str0m::change::SdpAnswer;

    pub fn serialize<S: Serializer>(answer: &SdpAnswer, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_str(&answer.to_sdp_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<SdpAnswer, D::Error> {
        let text = String::deserialize(input)?;
        SdpAnswer::from_sdp_string(&text).map_err(serde::de::Error::custom)
    }
}

/// What a signalling session asks of the task that owns the SFU.
enum Command {
    Join {
        room: Validated<Room>,
        offer: SdpOffer,
        signals: mpsc::UnboundedSender<Wire>,
        joined: oneshot::Sender<Seq<Participant>>,
    },
    Answer(Seq<Participant>, SdpAnswer),
    Leave(Seq<Participant>),
}

/// Binds both sockets, then serves until the signalling listener fails.
///
/// # Errors
///
/// The I/O error of a bind, or of the signalling listener.
pub async fn run(addresses: Addresses) -> std::io::Result<()> {
    let udp = UdpSocket::bind(addresses.media).await?;
    let media = udp.local_addr()?;
    let listener = TcpListener::bind(addresses.signal).await?;
    let (commands, inbox) = mpsc::channel(64);
    let app = Router::new()
        .route("/room/{room}", get(upgrade))
        .with_state(commands);
    tokio::spawn(media_loop(udp, media, inbox));
    axum::serve(listener, app).await
}

/// Owns the SFU: reads the socket and the commands, and sends what the SFU answers.
async fn media_loop(udp: UdpSocket, media: SocketAddr, mut inbox: mpsc::Receiver<Command>) {
    let mut sfu = Sfu::new(media);
    let mut sessions: Vec<(Seq<Participant>, mpsc::UnboundedSender<Wire>)> = Vec::new();
    let mut buf = vec![0; 2000];
    loop {
        let wake = sfu.poll(Instant::now(), |t| {
            // UDP may drop a datagram anyway: a full socket buffer drops it here instead.
            let _ = udp.try_send_to(&t.contents, t.destination);
        });
        while let Some((id, Signal::Offer(offer))) = sfu.poll_signal() {
            if let Some((_, out)) = sessions.iter().find(|(s, _)| *s == id) {
                let _ = out.send(Wire::Offer(offer));
            }
        }
        tokio::select! {
            read = udp.recv_from(&mut buf) => {
                if let Ok((n, source)) = read
                    && let Some(datagram) = buf.get(..n).and_then(|d| d.try_into().ok())
                {
                    sfu.receive(Instant::now(), source, datagram);
                }
            }
            command = inbox.recv() => match command {
                Some(Command::Join { room, offer, signals, joined }) => {
                    if let Ok((id, answer)) = sfu.join(Instant::now(), room, offer) {
                        let _ = signals.send(Wire::Answer(answer));
                        let _ = joined.send(id);
                        sessions.push((id, signals));
                    }
                }
                Some(Command::Answer(id, answer)) => sfu.accept_answer(id, answer),
                Some(Command::Leave(id)) => {
                    sfu.leave(id);
                    sessions.retain(|(s, _)| *s != id);
                }
                None => return,
            },
            () = tokio::time::sleep_until(wake.into()) => {}
        }
        sfu.tick(Instant::now());
    }
}

/// Opens the signalling session of one browser. A room name the rule refuses fails the path
/// extractor, which answers 400.
async fn upgrade(
    ws: WebSocketUpgrade,
    Path(room): Path<Validated<Room>>,
    State(commands): State<mpsc::Sender<Command>>,
) -> Response {
    ws.on_upgrade(move |socket| session(socket, room, commands))
}

/// The text of the next message of `socket` as a [`Wire`], or `None` once it closes.
async fn next_wire(socket: &mut WebSocket) -> Option<Option<Wire>> {
    match socket.recv().await? {
        Ok(Message::Text(text)) => Some(serde_json::from_str(text.as_str()).ok()),
        Ok(Message::Close(_)) | Err(_) => None,
        Ok(_) => Some(None),
    }
}

/// One browser's signalling: its offer first, then the SFU's offers and its answers.
async fn session(mut socket: WebSocket, room: Validated<Room>, commands: mpsc::Sender<Command>) {
    let Some(Some(Wire::Offer(offer))) = next_wire(&mut socket).await else {
        return;
    };
    let (signals, mut outbox) = mpsc::unbounded_channel();
    let (joined, id) = oneshot::channel();
    let join = Command::Join {
        room,
        offer,
        signals,
        joined,
    };
    if commands.send(join).await.is_err() {
        return;
    }
    let Ok(id) = id.await else {
        return;
    };
    loop {
        tokio::select! {
            out = outbox.recv() => {
                let Some(text) = out.and_then(|w| serde_json::to_string(&w).ok()) else {
                    break;
                };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            read = next_wire(&mut socket) => match read {
                Some(Some(Wire::Answer(answer))) => {
                    if commands.send(Command::Answer(id, answer)).await.is_err() {
                        break;
                    }
                }
                Some(_) => {}
                None => break,
            },
        }
    }
    let _ = commands.send(Command::Leave(id)).await;
}
