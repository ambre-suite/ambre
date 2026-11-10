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

use super::*;

use ambre_core::id::Call;
use ambre_core::{Count, ParticipantPseudonym, RoomPseudonym, SenderIndex, Seq};
use ambre_protocol::signaling::{
    ADMISSION_LIFETIME, Admission, CallClientMsg, CallRefusal, CallServerMsg, SIGNED_JOIN_MAX,
    Seated, WRAP_MAX,
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
}

/// What the first frame earns: a seat and the admission to the SFU, or the notice of a
/// refusal.
pub(crate) enum CallAnswer {
    Admitted {
        sender: Seq<SenderIndex>,
        room: FixedBytes<RoomPseudonym>,
        admission: ambre_core::Credential<ambre_core::CallAdmission>,
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
    let CallClientMsg::Join { token, channel } = first else {
        return Ok(CallAnswer::Refused(CallRefusal::Unauthorized));
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
    let joined = redis_ops::join_call(&state.redis, call, &channel, &key, &room, ttl).await?;
    let (room, sender) = match joined {
        redis_ops::CallJoin::Admitted { room, sender } => (room, sender),
        redis_ops::CallJoin::Refused(refusal) => return Ok(CallAnswer::Refused(refusal)),
    };
    let admission_room = room;
    let admission = Admission {
        room,
        participant: pseudonym::<ParticipantPseudonym>()?,
        expires: crate::s3::unix_now().plus(ADMISSION_LIFETIME),
    };
    let admission = match state.jwt.sign_admission(&admission) {
        Ok(admission) => admission,
        Err(e) => {
            // The seat is taken: give it back before the error ends the socket.
            redis_ops::leave_call(&state.redis, call, sender).await?;
            return Err(AppError::Internal(format!("no admission: {e}")));
        }
    };
    Ok(CallAnswer::Admitted {
        sender,
        room: admission_room,
        admission,
    })
}

/// A frame of the call socket, encoded.
fn call_frame(msg: &CallServerMsg) -> Option<StoredPayload> {
    StoredPayload::seal(msg).ok()
}

/// Publishes `msg` to every socket of `call`, on this relay and on the others.
async fn publish(state: &AppState, call: &Id<Call>, msg: &CallServerMsg) {
    if let Err(e) = redis_ops::call_signal(&state.redis, call, msg).await {
        tracing::warn!("call {call}: a signal was not published: {e}");
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
    let (sender, room, admission) = match answer {
        Ok(CallAnswer::Admitted {
            sender,
            room,
            admission,
        }) => (sender, room, admission),
        Ok(CallAnswer::Refused(refusal)) => {
            // The notice of `CALL-021` and of each other refusal, then the close.
            if let Some(frame) = call_frame(&CallServerMsg::Refused(refusal)) {
                let _ = socket.send(frame.into_message()).await;
            }
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
        Err(e) => {
            tracing::error!("call {call}: a join failed: {e}");
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let seat = Seat { call, sender, room };
    serve_seat(socket, &state, &seat, admission).await;
    match redis_ops::leave_call(&state.redis, &seat.call, seat.sender).await {
        Ok(_) => publish(&state, &seat.call, &CallServerMsg::Left(seat.sender)).await,
        Err(e) => tracing::error!("call {}: a leave failed: {e}", seat.call),
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
) {
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
        return;
    }
    let serve = async {
        let roster = match redis_ops::seated(&state.redis, &seat.call, seat.sender).await {
            Ok(roster) => roster,
            Err(e) => {
                tracing::error!("call {}: the roster was not read: {e}", seat.call);
                return;
            }
        };
        let admitted = CallServerMsg::Admitted {
            admission,
            sfu: state.config.call_sfu.clone(),
            sender: seat.sender,
            roster,
        };
        let Some(frame) = call_frame(&admitted) else {
            return;
        };
        if sink.send(frame.into_message()).await.is_err() {
            return;
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
        loop {
            tokio::select! {
                _ = hold.tick() => {
                    match redis_ops::hold_call(&state.redis, &seat.call, seat.sender, &seat.room, ttl).await {
                        // A seat whose field expired had no heartbeat: its leave is announced
                        // once, by the heartbeat that found it (`CALL-063`).
                        Ok(redis_ops::CallHold::Held { gone }) => {
                            for sender in gone {
                                publish(state, &seat.call, &CallServerMsg::Left(sender)).await;
                            }
                        }
                        // The store lost the call: it ended for this seat, with a notice
                        // (`SRV-249`).
                        Ok(redis_ops::CallHold::Lost) => {
                            if let Some(frame) = call_frame(&CallServerMsg::Lost) {
                                let _ = sink.send(frame.into_message()).await;
                            }
                            let _ = sink.send(Message::Close(None)).await;
                            return;
                        }
                        Err(e) => tracing::warn!("call {}: the seat was not refreshed: {e}", seat.call),
                    }
                }
                frame = payload::next_frame(&mut stream) => {
                    let Some(frame) = frame else { return };
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
                                Err(e) => tracing::warn!("call {}: a join was not kept: {e}", seat.call),
                            }
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
                    let Some(signal) = signal else { return };
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
                        return;
                    }
                }
            }
        }
    };
    crate::supervise::contained("call socket", serve).await;
    state.pubsub.unsubscribe(topic, connection).await;
}

#[cfg(test)]
#[path = "call_tests.rs"]
mod tests;
