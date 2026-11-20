//! The SFU process: one UDP socket for the media, and a WebSocket for the signalling.
//!
//! The signalling listens on a port of its own, never on the relay (`CALL-036`, `SRV-049`).
//! A browser opens `/call` and sends the admission the relay issued it as the first frame, so
//! no address holds the token. The SFU verifies it, answers the TURN servers with a credential
//! minted for the participant's pseudonym, then reads the offer. It answers it, then sends
//! every offer the room needs when it gains a track. One task owns the [`Sfu`]: the sockets
//! hand it datagrams and commands, so no lock guards a connection.
//!
//! A seat lives while the relay renews its admission (audit `N25`). The client hands each
//! renewal on the same socket, and the SFU ends a seat whose last admission expired. It
//! seats one connection per member of a room: a join of a member ends the older seat.
//!
//! A renewal that keeps its seat also answers a fresh TURN credential once the last one nears
//! its expiry (audit `P60`). The page restarts ICE with it, so its next allocation opens under
//! a credential coturn still admits (`CALL-153`).
//!
//! One socket costs the SFU a bounded share (audit `N39`). A message past
//! [`SIGNAL_MESSAGE_MAX`] closes the socket. An admission serves one socket alone, so it mints
//! one TURN credential. A socket waits [`ADMISSION_DEADLINE`] for its admission, then as long
//! again for its offer.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ambre_core::{
    Count, FixedBytes, MemberPseudonym, Octets, ParticipantPseudonym, Payload, RoomPseudonym,
    SenderIndex, Seq, TimeSpan, UnixSec, Validated,
};
use ambre_protocol::signaling::{
    Admission as Claims, CallParticipants, PARTICIPANT_CAP, SESSION_DESCRIPTION_MAX,
    SessionDescription, SfuMsg as Wire, VideoChoice,
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
use crate::metrics::{Admission, Direction, SFU};
use crate::turn::TurnServers;
use ambre_core::NarrowCount;
use ambre_metrics::{ScrapeAccess, Whole};

/// Where the SFU listens.
#[derive(Clone, Copy, Debug)]
pub struct Addresses {
    /// The TCP address of the signalling WebSocket.
    pub signal: SocketAddr,
    /// The UDP address of the media. It is the candidate each browser connects to, so it names
    /// an address the browsers reach, never an unspecified one.
    pub media: SocketAddr,
    /// The TCP address of the metrics, which neither listener above serves (`CALL-101`).
    pub metrics: SocketAddr,
}

/// Who the SFU admits, and the TURN servers it hands them.
#[derive(Debug)]
pub struct Admitting {
    /// The relay's public key, which signs every admission.
    pub key: AdmissionKey,
    /// The TURN servers, and the secret their credentials are minted under. `None` when the
    /// deployment runs none.
    pub turn: Option<TurnServers>,
    /// Who may read a scrape of the metrics listener (`CALL-153`).
    pub scrape: ScrapeAccess,
}

/// What a signalling session asks of the task that owns the SFU.
enum Command {
    /// A socket verified an admission: the media loop marks it spent, and answers whether no
    /// socket spent it before. Only then does the socket mint a TURN credential (audit `N39`).
    Spend {
        participant: FixedBytes<ParticipantPseudonym>,
        expires: ambre_core::Instant<UnixSec>,
        fresh: oneshot::Sender<Freshness>,
    },
    Join {
        room: Validated<Room>,
        participant: FixedBytes<ParticipantPseudonym>,
        member: FixedBytes<MemberPseudonym>,
        sender: Seq<SenderIndex>,
        expires: ambre_core::Instant<UnixSec>,
        offer: SdpOffer,
        signals: mpsc::UnboundedSender<Wire>,
        joined: oneshot::Sender<Seq<Participant>>,
    },
    Answer(Seq<Participant>, SdpAnswer),
    /// The participant's screen sender took a track: a share starts (`CALL-106`).
    Share(Seq<Participant>),
    /// The participant chose the videos it receives (`CALL-126`).
    Videos(Seq<Participant>, VideoChoice),
    /// A later offer of the participant, which adds its screen section (`CALL-112`).
    Offer(Seq<Participant>, SdpOffer),
    /// A renewal of the participant's admission, verified: it keeps the seat while it names
    /// the seat (audit `N25`). The media loop answers whether it kept it.
    Renew(Seq<Participant>, Claims, oneshot::Sender<Renewal>),
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
    let metrics = TcpListener::bind(addresses.metrics).await?;
    let scrape = crate::metrics::router(admitting.scrape.clone());
    tokio::spawn(async move { axum::serve(metrics, scrape).await });
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

impl NarrowCount for Datagrams {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DatagramCount");
}

/// Whether an admission opened a socket before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Freshness {
    /// A socket spent the admission already: it serves this one nothing.
    Spent,
    /// No socket spent it before: this socket holds it now.
    Fresh,
}

/// What a verified renewal did to the seat of its socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Renewal {
    /// The renewal names another seat, or the seat ended: it keeps nothing.
    Ignored,
    /// The renewal names the seat, which lives to its expiry now.
    Kept,
}

/// The most datagrams one turn of the media loop reads before it polls the SFU.
const BATCH: Count<Datagrams> = Count::new(64);

/// How often the media loop ends the seats whose admission expired with no renewal. An
/// admission expires on a whole Unix second, so a finer sweep ends no seat sooner.
const EXPIRY_SWEEP: Duration = Duration::from_secs(1);

/// Owns the SFU: reads the socket and the commands, and sends what the SFU answers.
async fn media_loop(udp: UdpSocket, media: SocketAddr, mut inbox: mpsc::Receiver<Command>) {
    let mut sfu = Sfu::new(media);
    let mut sessions: Vec<Session> = Vec::new();
    // Every participant pseudonym a socket spent, until its admission expires. A token whose
    // socket closed serves no other socket: one admission seats one connection (`CALL-117`).
    let mut spent: Vec<(
        FixedBytes<ParticipantPseudonym>,
        ambre_core::Instant<UnixSec>,
    )> = Vec::new();
    let mut buf = vec![0; 2000];
    let mut sweep = Instant::now();
    loop {
        if Instant::now() >= sweep {
            sweep = Instant::now() + EXPIRY_SWEEP;
            expire(&mut sfu, &mut sessions, unix_now());
        }
        let wake = sfu.poll(Instant::now(), |t| {
            // UDP may drop a datagram anyway: a full socket buffer drops it here instead.
            let sent = udp.try_send_to(&t.contents, t.destination);
            SFU.datagrams.inc(if sent.is_ok() {
                Direction::Out
            } else {
                Direction::Dropped
            });
        });
        while let Some((id, signal)) = sfu.poll_signal() {
            let Some(session) = sessions.iter().find(|s| s.id == id) else {
                continue;
            };
            let wire = match signal {
                Signal::Offer(offer) => {
                    Validated::<SessionDescription>::parse(offer.to_sdp_string())
                        .ok()
                        .map(Wire::Offer)
                }
                Signal::TakenOver => Some(Wire::Unshared),
            };
            if let Some(wire) = wire {
                let _ = session.signals.send(wire);
            }
        }
        tokio::select! {
            read = udp.recv_from(&mut buf) => {
                if let Ok((n, source)) = read
                    && let Some(datagram) = buf.get(..n)
                {
                    SFU.datagrams.inc(Direction::In);
                    sfu.receive(Instant::now(), source, Payload::new(datagram.to_vec()));
                }
                // Every datagram already queued goes in before one poll, so the cost of a
                // poll spreads over a batch.
                for _ in 0..BATCH.get() {
                    let Ok((n, source)) = udp.try_recv_from(&mut buf) else {
                        break;
                    };
                    if let Some(datagram) = buf.get(..n) {
                        SFU.datagrams.inc(Direction::In);
                        sfu.receive(Instant::now(), source, Payload::new(datagram.to_vec()));
                    }
                }
            }
            command = inbox.recv() => match command {
                Some(Command::Spend { participant, expires, fresh }) => {
                    // One admission serves one socket: a second socket under it, after the
                    // first closed too, reads nothing (`CALL-117`, audit `N39`).
                    let now = unix_now();
                    spent.retain(|(_, until)| *until >= now);
                    let answer = if spent.iter().any(|(p, _)| *p == participant) {
                        Freshness::Spent
                    } else {
                        spent.push((participant, expires));
                        Freshness::Fresh
                    };
                    let _ = fresh.send(answer);
                }
                Some(Command::Join { room, participant, member, sender, expires, offer, signals, joined }) => {
                    // The socket spent its admission before it sent the offer, so the cap
                    // alone refuses here. A refusal drops `joined`, which closes the socket.
                    // One seat per member: a later join of the member ends the older seat,
                    // so a member who rejoins holds no stale seat (audit `N25`).
                    let older: Vec<Seq<Participant>> = sessions
                        .iter()
                        .filter(|s| s.room == room && s.member == member)
                        .map(|s| s.id)
                        .collect();
                    end(&mut sfu, &mut sessions, &older);
                    let in_room = Count::<CallParticipants>::of(
                        sessions.iter().filter(|s| s.room == room).count(),
                    );
                    if in_room < PARTICIPANT_CAP
                        && let Ok((id, answer)) = sfu.join(Instant::now(), room.clone(), sender, offer)
                    {
                        if let Ok(answer) = Validated::<SessionDescription>::parse(answer.to_sdp_string()) {
                            let _ = signals.send(Wire::Answer(answer));
                        }
                        let _ = joined.send(id);
                        sessions.push(Session { id, room, participant, member, sender, expires, signals });
                        SFU.admissions.inc(Admission::Seated);
                    } else {
                        SFU.admissions.inc(Admission::Full);
                    }
                    occupancy(&sessions);
                }
                Some(Command::Renew(id, claims, kept)) => {
                    // A renewal keeps the seat it names alone. One for another room,
                    // participant, member or sender index keeps nothing (audit `N25`).
                    let mut renewal = Renewal::Ignored;
                    if let Some(session) = sessions.iter_mut().find(|s| s.id == id && s.renewed_by(&claims)) {
                        renewal = Renewal::Kept;
                        session.expires = session.expires.max(claims.expires);
                        // The token of a renewal admits the participant too: it stays spent
                        // while it is valid, so it seats no second connection (`CALL-117`).
                        for (p, until) in &mut spent {
                            if *p == claims.participant {
                                *until = (*until).max(claims.expires);
                            }
                        }
                    }
                    let _ = kept.send(renewal);
                }
                Some(Command::Answer(id, answer)) => sfu.accept_answer(id, answer),
                Some(Command::Share(id)) => sfu.share_started(Instant::now(), id),
                Some(Command::Videos(id, choice)) => sfu.choose_videos(id, choice),
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
                    occupancy(&sessions);
                }
                None => return,
            },
            () = tokio::time::sleep_until(wake.min(sweep).into()) => {}
        }
        sfu.tick(Instant::now());
    }
}

/// Ends the seats whose admission expired before `now` with no renewal (audit `N25`).
fn expire(sfu: &mut Sfu, sessions: &mut Vec<Session>, now: ambre_core::Instant<UnixSec>) {
    let expired: Vec<Seq<Participant>> = sessions
        .iter()
        .filter(|s| s.expires < now)
        .map(|s| s.id)
        .collect();
    end(sfu, sessions, &expired);
}

/// Ends the seats `ids`. The connection drops, and so does the way to the signalling of
/// each, which closes its socket.
fn end(sfu: &mut Sfu, sessions: &mut Vec<Session>, ids: &[Seq<Participant>]) {
    if ids.is_empty() {
        return;
    }
    for id in ids {
        sfu.leave(*id);
    }
    sessions.retain(|s| !ids.contains(&s.id));
    occupancy(sessions);
}

/// Sets the two gauges of occupancy from the sessions the media loop serves.
fn occupancy(sessions: &[Session]) {
    SFU.participants.set(Whole::All, Count::of(sessions.len()));
    let mut rooms: Vec<&Validated<Room>> = sessions.iter().map(|s| &s.room).collect();
    rooms.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
    rooms.dedup_by(|a, b| a.as_str() == b.as_str());
    SFU.rooms.set(Whole::All, Count::of(rooms.len()));
}

/// One connection the media loop serves: what the SFU knows of a participant, its
/// pseudonyms alone (`ZK-023`).
struct Session {
    id: Seq<Participant>,
    room: Validated<Room>,
    participant: FixedBytes<ParticipantPseudonym>,
    member: FixedBytes<MemberPseudonym>,
    sender: Seq<SenderIndex>,
    /// When the last admission of the seat expires. The media loop ends the seat past it.
    expires: ambre_core::Instant<UnixSec>,
    signals: mpsc::UnboundedSender<Wire>,
}

impl Session {
    /// Whether `claims` renew this seat: the same room, participant, member and sender.
    fn renewed_by(&self, claims: &Claims) -> bool {
        room_of(&claims.room).is_some_and(|room| room == self.room)
            && claims.participant == self.participant
            && claims.member == self.member
            && claims.sender == self.sender
    }
}

/// What the signalling sessions share: the way to the media loop, and the admission.
#[derive(Clone)]
struct Gate {
    commands: mpsc::Sender<Command>,
    admitting: Arc<Admitting>,
}

/// How long an open socket may wait for its admission, and then for its offer (audit `N39`).
const ADMISSION_DEADLINE: Duration = Duration::from_secs(10);

/// The largest message, and the largest frame, the signalling socket reads (audit `N39`). It
/// holds an offer at the cap of a session description, each of its bytes escaped once in JSON.
/// Past it the socket closes, so one socket holds that much of the SFU at most.
pub const SIGNAL_MESSAGE_MAX: Count<Octets> =
    Count::new(SESSION_DESCRIPTION_MAX.get().saturating_mul(2));

/// Opens the signalling session of one browser, under the message bound.
async fn upgrade(ws: WebSocketUpgrade, State(gate): State<Gate>) -> Response {
    let max = SIGNAL_MESSAGE_MAX.saturating_len().to_usize();
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| session(socket, gate))
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

/// Sends `wire` on `socket` as JSON text. Answers whether it left.
async fn send_wire(socket: &mut WebSocket, wire: &Wire) -> bool {
    match serde_json::to_string(wire) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => false,
    }
}

/// One browser's signalling: its admission, the TURN servers, its offer, then the SFU's
/// offers and its answers. A socket without a valid admission closes before the SFU reads an
/// offer.
async fn session(mut socket: WebSocket, gate: Gate) {
    let first = tokio::time::timeout(ADMISSION_DEADLINE, next_wire(&mut socket)).await;
    let Ok(Some(Some(Wire::Admission(token)))) = first else {
        SFU.admissions.inc(Admission::Refused);
        return;
    };
    let Ok(admission) = gate.admitting.key.admit(&token, unix_now()) else {
        SFU.admissions.inc(Admission::Refused);
        return;
    };
    let Some(room) = room_of(&admission.room) else {
        SFU.admissions.inc(Admission::Refused);
        return;
    };
    // The socket spends the admission before the SFU mints a credential. A second socket
    // under it gets no TURN credential, and holds no unfinished message (audit `N39`).
    let (fresh, answer) = oneshot::channel();
    let spend = Command::Spend {
        participant: admission.participant,
        expires: admission.expires,
        fresh,
    };
    if gate.commands.send(spend).await.is_err() || answer.await != Ok(Freshness::Fresh) {
        SFU.admissions.inc(Admission::Refused);
        return;
    }
    let minted = unix_now();
    // When the last credential the socket read expires. A renewal near it mints a fresh one.
    let mut held = gate.admitting.turn.as_ref().map(|turn| turn.expiry(minted));
    let ice = gate
        .admitting
        .turn
        .iter()
        .map(|turn| turn.credential(admission.participant, minted))
        .collect();
    if !send_wire(&mut socket, &Wire::Ice(ice)).await {
        return;
    }
    // An admitted socket that sends no offer within the deadline closes (audit `N39`).
    let offered = tokio::time::timeout(ADMISSION_DEADLINE, next_wire(&mut socket)).await;
    let Ok(Some(Some(Wire::Offer(offer)))) = offered else {
        return;
    };
    let Ok(offer) = SdpOffer::from_sdp_string(offer.as_str()) else {
        return;
    };
    let commands = gate.commands;
    let admitting = gate.admitting;
    let (signals, mut outbox) = mpsc::unbounded_channel();
    let (joined, id) = oneshot::channel();
    let join = Command::Join {
        room,
        participant: admission.participant,
        member: admission.member,
        sender: admission.sender,
        expires: admission.expires,
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
                // A renewal verifies as the first admission did. A forged or expired one
                // keeps nothing, and the seat ends when its last admission expires.
                Some(Some(Wire::Admission(token))) => {
                    let Ok(claims) = admitting.key.admit(&token, unix_now()) else {
                        continue;
                    };
                    let (kept, renewal) = oneshot::channel();
                    if commands.send(Command::Renew(id, claims, kept)).await.is_err() {
                        break;
                    }
                    // A renewal that kept the seat answers a fresh credential once the last
                    // one nears its expiry, under the pseudonym of the seat (`CALL-153`).
                    let kept = renewal.await == Ok(Renewal::Kept);
                    let now = unix_now();
                    let fresh = match (admitting.turn.as_ref(), held) {
                        (Some(turn), Some(last)) if kept && turn.is_due(last, now) => {
                            held = Some(turn.expiry(now));
                            Some(turn.credential(admission.participant, now))
                        }
                        _ => None,
                    };
                    if let Some(fresh) = fresh
                        && !send_wire(&mut socket, &Wire::Ice(vec![fresh])).await
                    {
                        break;
                    }
                }
                Some(Some(Wire::Share)) => {
                    if commands.send(Command::Share(id)).await.is_err() {
                        break;
                    }
                }
                Some(Some(Wire::Videos(choice))) => {
                    if commands.send(Command::Videos(id, choice)).await.is_err() {
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
