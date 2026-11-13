//! The SFU process: one UDP socket for the media, and a WebSocket for the signalling.
//!
//! The signalling listens on a port of its own, never on the relay (`CALL-036`, `SRV-049`).
//! A browser opens `/call` and sends the admission the relay issued it as the first frame, so
//! no address holds the token. The SFU verifies it, answers the TURN servers with a credential
//! minted for the participant's pseudonym, then reads the offer. It answers it, then sends
//! every offer the room needs when it gains a track. One task owns the [`Sfu`]: the sockets
//! hand it datagrams and commands, so no lock guards a connection.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ambre_core::{
    Count, CountKind, FixedBytes, ParticipantPseudonym, Payload, RoomPseudonym, Seq, TimeSpan,
    UnixSec, Validated,
};
use ambre_protocol::signaling::{
    CallParticipants, PARTICIPANT_CAP, SessionDescription, SfuMsg as Wire,
};
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use str0m::change::{SdpAnswer, SdpOffer};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, oneshot};

use crate::admit::AdmissionKey;
use crate::forward::{Participant, Room, Sfu, Signal};
use crate::turn::TurnServers;

/// Where the SFU listens.
#[derive(Clone, Copy, Debug)]
pub struct Addresses {
    /// The TCP address of the signalling WebSocket.
    pub signal: SocketAddr,
    /// The UDP address of the media. It is the candidate each browser connects to, so it names
    /// an address the browsers reach, never an unspecified one.
    pub media: SocketAddr,
}

/// Who the SFU admits, and the TURN servers it hands them.
#[derive(Debug)]
pub struct Admitting {
    /// The relay's public key, which signs every admission.
    pub key: AdmissionKey,
    /// The TURN servers, and the secret their credentials are minted under. `None` when the
    /// deployment runs none.
    pub turn: Option<TurnServers>,
}

/// What a signalling session asks of the task that owns the SFU.
enum Command {
    Join {
        room: Validated<Room>,
        participant: FixedBytes<ParticipantPseudonym>,
        offer: SdpOffer,
        signals: mpsc::UnboundedSender<Wire>,
        joined: oneshot::Sender<Seq<Participant>>,
    },
    Answer(Seq<Participant>, SdpAnswer),
    /// The participant's screen sender took a track: a share starts (`CALL-106`).
    Share(Seq<Participant>),
    /// A later offer of the participant, which adds its screen section (`CALL-112`).
    Offer(Seq<Participant>, SdpOffer),
    Leave(Seq<Participant>),
}

/// Binds both sockets, then serves until the signalling listener fails.
///
/// # Errors
///
/// The I/O error of a bind, or of the signalling listener.
pub async fn run(addresses: Addresses, admitting: Admitting) -> std::io::Result<()> {
    let udp = UdpSocket::bind(addresses.media).await?;
    let media = udp.local_addr()?;
    let listener = TcpListener::bind(addresses.signal).await?;
    let (commands, inbox) = mpsc::channel(64);
    let app = Router::new().route("/call", get(upgrade)).with_state(Gate {
        commands,
        admitting: Arc::new(admitting),
    });
    tokio::spawn(media_loop(udp, media, inbox));
    axum::serve(listener, app).await
}

/// How many datagrams one turn of the media loop reads.
enum Datagrams {}

impl CountKind for Datagrams {
    type Repr = u32;
    const NAME: &'static str = "DatagramCount";
}

/// The most datagrams one turn of the media loop reads before it polls the SFU.
const BATCH: Count<Datagrams> = Count::new(64);

/// Owns the SFU: reads the socket and the commands, and sends what the SFU answers.
async fn media_loop(udp: UdpSocket, media: SocketAddr, mut inbox: mpsc::Receiver<Command>) {
    let mut sfu = Sfu::new(media);
    let mut sessions: Vec<Session> = Vec::new();
    let mut buf = vec![0; 2000];
    loop {
        let wake = sfu.poll(Instant::now(), |t| {
            // UDP may drop a datagram anyway: a full socket buffer drops it here instead.
            let _ = udp.try_send_to(&t.contents, t.destination);
        });
        while let Some((id, Signal::Offer(offer))) = sfu.poll_signal() {
            if let Some(session) = sessions.iter().find(|s| s.id == id)
                && let Ok(offer) = Validated::<SessionDescription>::parse(offer.to_sdp_string())
            {
                let _ = session.signals.send(Wire::Offer(offer));
            }
        }
        tokio::select! {
            read = udp.recv_from(&mut buf) => {
                if let Ok((n, source)) = read
                    && let Some(datagram) = buf.get(..n)
                {
                    sfu.receive(Instant::now(), source, Payload::new(datagram.to_vec()));
                }
                // Every datagram already queued goes in before one poll, so the cost of a
                // poll spreads over a batch.
                for _ in 0..BATCH.get() {
                    let Ok((n, source)) = udp.try_recv_from(&mut buf) else {
                        break;
                    };
                    if let Some(datagram) = buf.get(..n) {
                        sfu.receive(Instant::now(), source, Payload::new(datagram.to_vec()));
                    }
                }
            }
            command = inbox.recv() => match command {
                Some(Command::Join { room, participant, offer, signals, joined }) => {
                    // The cap holds here too, and one admission seats one connection. A
                    // refusal drops `joined`, which closes the socket.
                    let in_room = Count::<CallParticipants>::of(
                        sessions.iter().filter(|s| s.room == room).count(),
                    );
                    let seated = sessions.iter().any(|s| s.participant == participant);
                    if in_room < PARTICIPANT_CAP
                        && !seated
                        && let Ok((id, answer)) = sfu.join(Instant::now(), room.clone(), offer)
                    {
                        if let Ok(answer) = Validated::<SessionDescription>::parse(answer.to_sdp_string()) {
                            let _ = signals.send(Wire::Answer(answer));
                        }
                        let _ = joined.send(id);
                        sessions.push(Session { id, room, participant, signals });
                    }
                }
                Some(Command::Answer(id, answer)) => sfu.accept_answer(id, answer),
                Some(Command::Share(id)) => sfu.share_started(Instant::now(), id),
                Some(Command::Offer(id, offer)) => {
                    let answer = sfu.accept_offer(id, offer).and_then(|answer| {
                        Validated::<SessionDescription>::parse(answer.to_sdp_string()).ok()
                    });
                    if let (Some(answer), Some(session)) =
                        (answer, sessions.iter().find(|s| s.id == id))
                    {
                        let _ = session.signals.send(Wire::Answer(answer));
                    }
                }
                Some(Command::Leave(id)) => {
                    sfu.leave(id);
                    sessions.retain(|s| s.id != id);
                }
                None => return,
            },
            () = tokio::time::sleep_until(wake.into()) => {}
        }
        sfu.tick(Instant::now());
    }
}

/// One connection the media loop serves: what the SFU knows of a participant, its
/// pseudonyms alone (`ZK-023`).
struct Session {
    id: Seq<Participant>,
    room: Validated<Room>,
    participant: FixedBytes<ParticipantPseudonym>,
    signals: mpsc::UnboundedSender<Wire>,
}

/// What the signalling sessions share: the way to the media loop, and the admission.
#[derive(Clone)]
struct Gate {
    commands: mpsc::Sender<Command>,
    admitting: Arc<Admitting>,
}

/// How long an open socket may wait for its admission.
const ADMISSION_DEADLINE: Duration = Duration::from_secs(10);

/// Opens the signalling session of one browser.
async fn upgrade(ws: WebSocketUpgrade, State(gate): State<Gate>) -> Response {
    ws.on_upgrade(move |socket| session(socket, gate))
}

/// The instant of the wall clock, in Unix seconds, which an admission expires on.
fn unix_now() -> ambre_core::Instant<UnixSec> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(ambre_core::Instant::EPOCH, |d| {
            ambre_core::Instant::after_epoch(TimeSpan::from_duration(d))
        })
}

/// The room of a call on the SFU: its pseudonym, in hex, which the room rule admits.
fn room_of(room: &FixedBytes<RoomPseudonym>) -> Option<Validated<Room>> {
    Validated::parse(room.to_hex()).ok()
}

/// The text of the next message of `socket` as a [`Wire`], or `None` once it closes.
async fn next_wire(socket: &mut WebSocket) -> Option<Option<Wire>> {
    match socket.recv().await? {
        Ok(Message::Text(text)) => Some(serde_json::from_str(text.as_str()).ok()),
        Ok(Message::Close(_)) | Err(_) => None,
        Ok(_) => Some(None),
    }
}

/// One browser's signalling: its admission, the TURN servers, its offer, then the SFU's
/// offers and its answers. A socket without a valid admission closes before the SFU reads an
/// offer.
async fn session(mut socket: WebSocket, gate: Gate) {
    let first = tokio::time::timeout(ADMISSION_DEADLINE, next_wire(&mut socket)).await;
    let Ok(Some(Some(Wire::Admission(token)))) = first else {
        return;
    };
    let Ok(admission) = gate.admitting.key.admit(&token, unix_now()) else {
        return;
    };
    let Some(room) = room_of(&admission.room) else {
        return;
    };
    let ice = gate
        .admitting
        .turn
        .iter()
        .map(|turn| turn.credential(admission.participant, unix_now()))
        .collect();
    let Ok(text) = serde_json::to_string(&Wire::Ice(ice)) else {
        return;
    };
    if socket.send(Message::Text(text.into())).await.is_err() {
        return;
    }
    let Some(Some(Wire::Offer(offer))) = next_wire(&mut socket).await else {
        return;
    };
    let Ok(offer) = SdpOffer::from_sdp_string(offer.as_str()) else {
        return;
    };
    let commands = gate.commands;
    let (signals, mut outbox) = mpsc::unbounded_channel();
    let (joined, id) = oneshot::channel();
    let join = Command::Join {
        room,
        participant: admission.participant,
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
                    let Ok(answer) = SdpAnswer::from_sdp_string(answer.as_str()) else {
                        continue;
                    };
                    if commands.send(Command::Answer(id, answer)).await.is_err() {
                        break;
                    }
                }
                Some(Some(Wire::Share)) => {
                    if commands.send(Command::Share(id)).await.is_err() {
                        break;
                    }
                }
                Some(Some(Wire::Offer(offer))) => {
                    let Ok(offer) = SdpOffer::from_sdp_string(offer.as_str()) else {
                        continue;
                    };
                    if commands.send(Command::Offer(id, offer)).await.is_err() {
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

#[cfg(test)]
mod tests;
