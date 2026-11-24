//! The call socket: the signalling route of a call on the relay (`SRV-049`).
//!
//! Endpoint: `GET /api/calls/{call}/socket`, apart from the socket of the document plane. Its
//! first frame joins: the access token, then the channel of the call. The relay checks the
//! membership of the channel and seats the session under the cap (`CALL-021`). The seat holds
//! a sender index no earlier join of the call held (`CALL-056`). The relay answers an admission
//! to the SFU, the index and the signed join of each seat.
//!
//! It then forwards the signed joins,
//! the leaves and the opaque wraps of the call on one topic (`CALL-057`, `CALL-058`). The close
//! of the socket is the leave.
//!
//! What the relay learns of a call is the list of `ZK-022`: the channel, the start and the end.
//! Per session, it learns the key, the sender index, the opaque signed join and the instants of
//! its join and leave. Per wrap, the size. The pseudonyms it draws for the SFU are kept nowhere
//! but the room of the call. All of it lives in the ephemeral store, under a time to live each
//! open socket refreshes, and no table holds a call (`SRV-248`, `SRV-250`).
//!
//! A heartbeat sends a fresh admission of its seat while the member is in the channel. The
//! client hands it to the SFU. The SFU ends a seat no renewal reaches, so a
//! seat of the SFU lives while its relay seat does (audit `N25`).

use super::*;

use ambre_core::id::Call;
use ambre_core::{Count, ParticipantPseudonym, RoomPseudonym, SenderIndex, Seq};
use ambre_protocol::signaling::{
    ADMISSION_LIFETIME, Admission, CallClientMsg, CallRefusal, CallServerMsg, Departure,
    SIGNED_JOIN_MAX, Seated, WRAP_MAX,
};

/// The largest frame the call socket reads: a wrap at its cap, with room for the framing.
const CALL_FRAME_MAX: Count<ambre_core::Octets> = Count::new(4 * 1024);

/// HTTP upgrade of the call socket. As on the socket of the document plane, the address holds
/// no credential: the token is the first frame (`SRV-031`).
pub async fn call_upgrade(
    ws: WebSocketUpgrade,
    call: crate::extract::OpaquePathId<Id<Call>>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let call = call.into_inner();
    let max = CALL_FRAME_MAX.saturating_len().to_usize();
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| handle_call(socket, state, call))
}

/// One participant session in the roster, as the socket holds it.
struct Seat {
    call: Id<Call>,
    sender: Seq<SenderIndex>,
    /// The room of the call state the seat joined. A heartbeat that finds another room finds
    /// a fresh call, and this seat lost its own (`SRV-249`).
    room: FixedBytes<RoomPseudonym>,
    /// Whose session the seat is, and in which channel: a heartbeat renews the admission of a
    /// member of that channel alone (audit `N25`).
    holder: Holder,
    /// The admission of the seat, which each renewal signs again under a later expiry. The
    /// socket keeps it, and no store does.
    admission: Admission,
}

/// The member a seat belongs to, and the channel of its call.
#[derive(Clone, Copy)]
pub(crate) struct Holder {
    pub(crate) user: Id<User>,
    pub(crate) channel: Id<Channel>,
}

/// What the first frame earns: a seat and the admission to the SFU, or the notice of a
/// refusal.
pub(crate) enum CallAnswer {
    Admitted {
        /// The call the seat is in: the live call of the channel, when a start found one
        /// (`CALL-160`).
        call: Id<Call>,
        sender: Seq<SenderIndex>,
        room: FixedBytes<RoomPseudonym>,
        admission: ambre_core::Credential<ambre_core::CallAdmission>,
        /// What the token signs, which each renewal signs again.
        claims: Admission,
        holder: Holder,
    },
    Refused(CallRefusal),
}

/// Draws a pseudonym from the generator of the OS.
fn pseudonym<R: ambre_core::FixedRole<Array = [u8; ambre_core::PSEUDONYM_LEN]>>()
-> Result<FixedBytes<R>, AppError> {
    let mut bytes = [0u8; ambre_core::PSEUDONYM_LEN];
    getrandom::fill(&mut bytes).map_err(|e| AppError::Internal(format!("no CSPRNG: {e}")))?;
    Ok(FixedBytes::from_array(bytes))
}

/// The most times a start reads the key of the live call of its channel. Each try past the
/// first follows a call that ended or a start that took the key between two reads.
const CLAIM_TRIES: ambre_core::Count<ClaimTries> = ambre_core::Count::new(3);

/// The reads of the key of the live call of a channel by one start, the first included.
enum ClaimTries {}

impl ambre_core::NarrowCount for ClaimTries {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ClaimTries");
}

/// Takes a seat in `call`, or in the live call of `channel` for a start (`CALL-160`).
/// Answers the call of the seat and the answer of the store.
///
/// A start takes the key of the live call of the channel and creates its call. A key that
/// names another call makes the start a join of that call. A join of that call refused as
/// ended frees the key for the start.
async fn seat_in(
    state: &AppState,
    call: &Id<Call>,
    channel: &Id<ambre_core::id::Channel>,
    key: &FixedBytes<ambre_core::PublicKey>,
    room: &FixedBytes<RoomPseudonym>,
    opening: redis_ops::CallOpening,
    ttl: ambre_core::TimeSpan<ambre_core::Sec>,
) -> Result<(Id<Call>, redis_ops::CallJoin), AppError> {
    let redis = &state.redis;
    if opening == redis_ops::CallOpening::Join {
        let joined = redis_ops::join_call(redis, call, channel, key, room, opening, ttl).await?;
        return Ok((*call, joined));
    }
    let mut stale = None;
    let mut tries = ambre_core::Count::<ClaimTries>::ZERO;
    while tries < CLAIM_TRIES {
        tries = tries.incremented();
        match redis_ops::claim_channel_call(redis, channel, call, stale.as_ref(), ttl).await? {
            redis_ops::ChannelCall::Taken => {
                let joined =
                    redis_ops::join_call(redis, call, channel, key, room, opening, ttl).await?;
                return Ok((*call, joined));
            }
            redis_ops::ChannelCall::Held(live) => {
                let join = redis_ops::CallOpening::Join;
                match redis_ops::join_call(redis, &live, channel, key, room, join, ttl).await? {
                    redis_ops::CallJoin::Refused(CallRefusal::Ended) => stale = Some(live),
                    joined => return Ok((live, joined)),
                }
            }
        }
    }
    Err(AppError::Internal(
        "a start found the live call of its channel changing at each read".into(),
    ))
}

/// Answers the first frame of a call socket. A session that joins is in the roster on return,
/// and the caller owes its leave.
///
/// # Errors
/// A database error, or no generator of the OS.
pub(crate) async fn answer_join(
    state: &AppState,
    call: &Id<Call>,
    first: CallClientMsg,
) -> Result<CallAnswer, AppError> {
    let (token, channel, opening) = match first {
        CallClientMsg::Join { token, channel } => (token, channel, redis_ops::CallOpening::Join),
        CallClientMsg::Start { token, channel } => (token, channel, redis_ops::CallOpening::Start),
        _ => return Ok(CallAnswer::Refused(CallRefusal::Unauthorized)),
    };
    let Ok(session) = auth::authenticate_token(state, &token) else {
        return Ok(CallAnswer::Refused(CallRefusal::Unauthorized));
    };
    if !db::is_channel_member(&state.db, &channel, &session.user_id).await? {
        return Ok(CallAnswer::Refused(CallRefusal::NotMember));
    }
    let key = session.signing_pubkey;
    let room = pseudonym::<RoomPseudonym>()?;
    let ttl = state.config.call_state_ttl;
    let (call, joined) = seat_in(state, call, &channel, &key, &room, opening, ttl).await?;
    let call = &call;
    let (room, sender) = match joined {
        redis_ops::CallJoin::Admitted { room, sender } => (room, sender),
        redis_ops::CallJoin::Refused(refusal) => return Ok(CallAnswer::Refused(refusal)),
    };
    let participant = pseudonym::<ParticipantPseudonym>()?;
    let signed = state.jwt.member_pseudonym(&room, &key).and_then(|member| {
        let claims = Admission {
            room,
            participant,
            member,
            sender,
            expires: crate::s3::unix_now().plus(ADMISSION_LIFETIME),
        };
        Ok((state.jwt.sign_admission(&claims)?, claims))
    });
    let (admission, claims) = match signed {
        Ok(signed) => signed,
        Err(e) => {
            // The seat is taken: give it back before the error ends the socket.
            redis_ops::leave_call(&state.redis, call, sender).await?;
            return Err(AppError::Internal(format!("no admission: {e}")));
        }
    };
    Ok(CallAnswer::Admitted {
        call: *call,
        sender,
        room,
        admission,
        claims,
        holder: Holder {
            user: session.user_id,
            channel,
        },
    })
}

/// A fresh admission of `seat` while its member is in the channel (audit `N25`). It holds
/// the claims of the first one under a later expiry. `None` once the member left the channel,
/// so the seat ends and the SFU drops its connection when the last admission expires.
///
/// # Errors
/// A database error.
async fn renewal(
    state: &AppState,
    seat: &Seat,
) -> Result<Option<ambre_core::Credential<ambre_core::CallAdmission>>, AppError> {
    if !db::is_channel_member(&state.db, &seat.holder.channel, &seat.holder.user).await? {
        return Ok(None);
    }
    let claims = Admission {
        expires: crate::s3::unix_now().plus(ADMISSION_LIFETIME),
        ..seat.admission.clone()
    };
    state
        .jwt
        .sign_admission(&claims)
        .map(Some)
        .map_err(|e| AppError::Internal(format!("no admission: {e}")))
}

/// A frame of the call socket, encoded.
fn call_frame(msg: &CallServerMsg) -> Option<StoredPayload> {
    StoredPayload::seal(msg).ok()
}

/// Publishes `msg` to every socket of `call`, on this relay and on the others.
async fn publish(state: &AppState, call: &Id<Call>, msg: &CallServerMsg) {
    if let Err(e) = redis_ops::call_signal(&state.redis, call, msg).await {
        tracing::warn!("a call signal was not published: {e}");
    }
}

async fn handle_call(mut socket: WebSocket, state: Arc<AppState>, call: Id<Call>) {
    let first = tokio::time::timeout(AUTH_DEADLINE, payload::first_frame(&mut socket))
        .await
        .ok()
        .flatten()
        .and_then(|frame| frame.open::<CallClientMsg>().ok());
    let answer = match first {
        Some(first) => answer_join(&state, &call, first).await,
        None => Ok(CallAnswer::Refused(CallRefusal::Unauthorized)),
    };
    let (call, sender, room, admission, claims, holder) = match answer {
        Ok(CallAnswer::Admitted {
            call: seated,
            sender,
            room,
            admission,
            claims,
            holder,
        }) => (seated, sender, room, admission, claims, holder),
        Ok(CallAnswer::Refused(refusal)) => {
            // The notice of `CALL-021` and of each other refusal, then the close.
            if let Some(frame) = call_frame(&CallServerMsg::Refused(refusal)) {
                let _ = socket.send(frame.into_message()).await;
            }
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
        Err(e) => {
            tracing::error!("a call join failed: {e}");
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let seat = Seat {
        call,
        sender,
        room,
        holder,
        admission: claims,
    };
    let _open = crate::metrics::OpenSocket::of(crate::metrics::Socket::Call);
    let exit = serve_seat(socket, &state, &seat, admission).await;
    // A leave frame took the seat out already and told the client whether it was the last.
    if exit == SeatExit::Closed {
        leave(&state, &seat).await;
    }
}

/// What the call socket read next.
enum Heard {
    /// A frame of the call protocol.
    Frame(StoredPayload),
    /// A pong, or any frame of no meaning to the call: the peer is still there.
    Other,
    /// The socket closed, or failed.
    Closed,
}

/// The next thing the call socket reads, a pong included, so the liveness of the peer is
/// known (`CALL-073`).
async fn next_heard<S>(stream: &mut S) -> Heard
where
    S: futures_util::Stream<Item = Result<Message, axum::Error>> + Unpin,
{
    match stream.next().await {
        Some(Ok(Message::Binary(data))) => Heard::Frame(StoredPayload::new(data.to_vec())),
        Some(Ok(Message::Close(_)) | Err(_)) | None => Heard::Closed,
        Some(Ok(_)) => Heard::Other,
    }
}

/// How the serve of a seat ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SeatExit {
    /// The socket closed, or the call ended for the seat: the leave is still owed.
    Closed,
    /// The client sent a leave frame, and the relay took the seat out.
    Left,
}

/// Takes the seat out of its call, and announces the leave to the others. Answers whether
/// the seat was the last, `None` on a store error.
async fn leave(state: &AppState, seat: &Seat) -> Option<Departure> {
    match redis_ops::leave_call(&state.redis, &seat.call, seat.sender).await {
        Ok(departure) => {
            publish(state, &seat.call, &CallServerMsg::Left(seat.sender)).await;
            // The last leave frees the channel for its next call (`CALL-160`).
            if departure == Departure::Last
                && let Err(e) =
                    redis_ops::release_channel_call(&state.redis, &seat.holder.channel, &seat.call)
                        .await
            {
                tracing::warn!("a channel call was not released: {e}");
            }
            Some(departure)
        }
        Err(e) => {
            tracing::error!("a call leave failed: {e}");
            None
        }
    }
}

/// Serves one seat until its socket closes. The signed joins, the leaves and the wraps of the
/// call go out to the client. The signed join and the wraps of the client go to the topic of
/// the call.
async fn serve_seat(
    socket: WebSocket,
    state: &AppState,
    seat: &Seat,
    admission: ambre_core::Credential<ambre_core::CallAdmission>,
) -> SeatExit {
    let (mut sink, mut stream) = socket.split();
    let (topic_tx, mut topic_rx) = mpsc::channel::<StoredPayload>(256);
    let connection = ConnectionId::new();
    let topic = Topic::call(&seat.call);
    // The route is in place before the roster is read, so this socket hears every join
    // announced after the read. A join announced in between arrives twice, and the client
    // refuses the second copy as seen.
    if state
        .pubsub
        .subscribe(topic.clone(), connection.clone(), topic_tx)
        .await
        .is_err()
    {
        return SeatExit::Closed;
    }
    let serve = async {
        let roster = match redis_ops::seated(&state.redis, &seat.call, seat.sender).await {
            Ok(roster) => roster,
            Err(e) => {
                tracing::error!("a call roster was not read: {e}");
                return SeatExit::Closed;
            }
        };
        let admitted = CallServerMsg::Admitted {
            call: seat.call,
            admission,
            sfu: state.config.call_sfu.clone(),
            sender: seat.sender,
            roster,
        };
        let Some(frame) = call_frame(&admitted) else {
            return SeatExit::Closed;
        };
        if sink.send(frame.into_message()).await.is_err() {
            return SeatExit::Closed;
        }
        let mut bucket =
            TokenBucket::new(state.config.ws_msg_burst, state.config.ws_msg_rate_per_sec);
        // The heartbeat runs several times per time to live, so one late beat loses nothing.
        let ttl = state.config.call_state_ttl;
        let period = state
            .config
            .call_heartbeat
            .to_duration()
            .max(Duration::from_secs(1));
        let mut hold = tokio::time::interval(period);
        hold.tick().await;
        // A peer that lost its network closes nothing. Each beat pings the socket, and a
        // socket that answered nothing, a pong included, for one time to live ends its
        // seat (`CALL-073`).
        let mut heard = Instant::now();
        loop {
            tokio::select! {
                _ = hold.tick() => {
                    if heard.elapsed() > ttl.to_duration() {
                        return SeatExit::Closed;
                    }
                    if sink.send(Message::Ping(Default::default())).await.is_err() {
                        return SeatExit::Closed;
                    }
                    match redis_ops::hold_call(&state.redis, &seat.call, seat.sender, &seat.room, ttl).await {
                        // A seat whose field expired had no heartbeat: its leave is announced
                        // once, by the heartbeat that found it (`CALL-063`).
                        Ok(redis_ops::CallHold::Held { gone }) => {
                            // The key of the live call of the channel lives as long as the
                            // call (`CALL-160`).
                            if let Err(e) = redis_ops::hold_channel_call(
                                &state.redis,
                                &seat.holder.channel,
                                &seat.call,
                                ttl,
                            )
                            .await
                            {
                                tracing::warn!("a channel call was not held: {e}");
                            }
                            for sender in gone {
                                publish(state, &seat.call, &CallServerMsg::Left(sender)).await;
                            }
                            // A member removed from the channel gets no renewal, and its seat
                            // ends: the SFU drops its connection once its admission expires
                            // (audit `N25`).
                            match renewal(state, seat).await {
                                Ok(Some(admission)) => {
                                    let renewed = call_frame(&CallServerMsg::Renewed(admission));
                                    if let Some(frame) = renewed
                                        && sink.send(frame.into_message()).await.is_err()
                                    {
                                        return SeatExit::Closed;
                                    }
                                }
                                Ok(None) => {
                                    let _ = sink.send(Message::Close(None)).await;
                                    return SeatExit::Closed;
                                }
                                Err(e) => tracing::warn!("a call admission was not renewed: {e}"),
                            }
                        }
                        // The store lost the call: it ended for this seat, with a notice
                        // (`SRV-249`).
                        Ok(redis_ops::CallHold::Lost) => {
                            if let Some(frame) = call_frame(&CallServerMsg::Lost) {
                                let _ = sink.send(frame.into_message()).await;
                            }
                            let _ = sink.send(Message::Close(None)).await;
                            return SeatExit::Closed;
                        }
                        Err(e) => tracing::warn!("a call seat was not refreshed: {e}"),
                    }
                }
                frame = next_heard(&mut stream) => {
                    heard = Instant::now();
                    let frame = match frame {
                        Heard::Frame(frame) => frame,
                        Heard::Other => continue,
                        Heard::Closed => return SeatExit::Closed,
                    };
                    if !bucket.try_consume(Instant::now()) {
                        continue;
                    }
                    match frame.open::<CallClientMsg>() {
                        Ok(CallClientMsg::Announce(join)) if join.len() <= SIGNED_JOIN_MAX => {
                            let ttl = state.config.call_state_ttl;
                            match redis_ops::announce_join(&state.redis, &seat.call, seat.sender, &join, ttl).await {
                                Ok(true) => {
                                    let signal = CallServerMsg::Joined(Seated { sender: seat.sender, join });
                                    publish(state, &seat.call, &signal).await;
                                }
                                Ok(false) => {}
                                Err(e) => tracing::warn!("a call join was not kept: {e}"),
                            }
                        }
                        // The leave answers whether the seat was the last, then closes
                        // (`CALL-069`).
                        Ok(CallClientMsg::Leave) => {
                            let Some(departure) = leave(state, seat).await else {
                                return SeatExit::Closed;
                            };
                            if let Some(frame) = call_frame(&CallServerMsg::Departed(departure)) {
                                let _ = sink.send(frame.into_message()).await;
                            }
                            let _ = sink.send(Message::Close(None)).await;
                            return SeatExit::Left;
                        }
                        Ok(CallClientMsg::Wrap { to, wrap }) if wrap.len() <= WRAP_MAX => {
                            let signal = CallServerMsg::Wrap { from: seat.sender, to, wrap };
                            publish(state, &seat.call, &signal).await;
                        }
                        // A second join, an oversized frame or a frame that does not decode is
                        // dropped: the session is bound once, by its first frame.
                        _ => {}
                    }
                }
                signal = topic_rx.recv() => {
                    let Some(signal) = signal else { return SeatExit::Closed };
                    // A wrap goes to the seat of its index alone (`CALL-058`), and a seat
                    // hears no echo of its own join or leave.
                    let addressed = match signal.open::<CallServerMsg>() {
                        Ok(CallServerMsg::Wrap { to, .. }) => to == seat.sender,
                        Ok(CallServerMsg::Joined(Seated { sender, .. }) | CallServerMsg::Left(sender)) => {
                            sender != seat.sender
                        }
                        Ok(_) => true,
                        Err(_) => false,
                    };
                    if addressed && sink.send(signal.into_message()).await.is_err() {
                        return SeatExit::Closed;
                    }
                }
            }
        }
    };
    let exit = crate::supervise::contained("call socket", serve)
        .await
        .unwrap_or(SeatExit::Closed);
    state.pubsub.unsubscribe(topic, connection).await;
    exit
}

#[cfg(test)]
#[path = "call_tests.rs"]
mod tests;
