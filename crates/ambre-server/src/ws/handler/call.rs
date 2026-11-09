//! The call socket: the signalling route of a call on the relay (`SRV-049`).
//!
//! Endpoint: `GET /api/calls/{call}/socket`, apart from the socket of the document plane. Its
//! first frame joins: the access token, then the channel of the call. The relay checks the
//! membership of the channel and puts the session in the roster, under the cap (`CALL-021`).
//! It answers an admission to the SFU. It then forwards the joins, the leaves and the opaque
//! wraps of the call on one topic, until the socket closes, which is the leave.
//!
//! What the relay learns of a call is the list of `ZK-022`: the channel, the start and the end.
//! Per session, it learns the key and the instants of its join and leave. Per wrap, the size. The pseudonyms it draws for the SFU are kept nowhere but the room of the call.

use super::*;

use ambre_core::id::Call;
use ambre_core::{Count, ParticipantPseudonym, RoomPseudonym};
use ambre_protocol::signaling::{
    ADMISSION_LIFETIME, Admission, CallClientMsg, CallRefusal, CallServerMsg, WRAP_MAX,
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
    key: FixedBytes<PublicKey>,
}

/// What the first frame earns: a seat and the frame that admits it, or the notice of a
/// refusal.
pub(crate) enum CallAnswer {
    Admitted {
        key: FixedBytes<PublicKey>,
        frame: CallServerMsg,
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
    let (room, roster) = match db::join_call(&state.db, call, &channel, &key, &room).await? {
        db::CallJoin::Admitted { room, roster } => (room, roster),
        db::CallJoin::Refused(refusal) => return Ok(CallAnswer::Refused(refusal)),
    };
    let admission = Admission {
        room,
        participant: pseudonym::<ParticipantPseudonym>()?,
        expires: crate::s3::unix_now().plus(ADMISSION_LIFETIME),
    };
    let admission = match state.jwt.sign_admission(&admission) {
        Ok(admission) => admission,
        Err(e) => {
            // The seat is taken: give it back before the error ends the socket.
            db::leave_call(&state.db, call, &key).await?;
            return Err(AppError::Internal(format!("an admission encodes: {e}")));
        }
    };
    Ok(CallAnswer::Admitted {
        key,
        frame: CallServerMsg::Admitted { admission, roster },
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
    let (key, admitted) = match answer {
        Ok(CallAnswer::Admitted { key, frame }) => (key, frame),
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
    let seat = Seat { call, key };
    serve_seat(socket, &state, &seat, admitted).await;
    match db::leave_call(&state.db, &seat.call, &seat.key).await {
        Ok(_) => publish(&state, &seat.call, &CallServerMsg::Left(seat.key)).await,
        Err(e) => tracing::error!("call {}: a leave failed: {e}", seat.call),
    }
}

/// Serves one seat until its socket closes. The joins, the leaves and the wraps of the call go
/// out to the client. The wraps of the client go to the topic of the call.
async fn serve_seat(socket: WebSocket, state: &AppState, seat: &Seat, admitted: CallServerMsg) {
    let (mut sink, mut stream) = socket.split();
    let (topic_tx, mut topic_rx) = mpsc::channel::<StoredPayload>(256);
    let connection = ConnectionId::new();
    let topic = Topic::call(&seat.call);
    // The route is in place before the join is published, so this socket hears every signal
    // after its own admission.
    if state
        .pubsub
        .subscribe(topic.clone(), connection.clone(), topic_tx)
        .await
        .is_err()
    {
        return;
    }
    let serve = async {
        let Some(frame) = call_frame(&admitted) else {
            return;
        };
        if sink.send(frame.into_message()).await.is_err() {
            return;
        }
        publish(state, &seat.call, &CallServerMsg::Joined(seat.key)).await;
        let mut bucket =
            TokenBucket::new(state.config.ws_msg_burst, state.config.ws_msg_rate_per_sec);
        loop {
            tokio::select! {
                frame = payload::next_frame(&mut stream) => {
                    let Some(frame) = frame else { return };
                    if !bucket.try_consume(Instant::now()) {
                        continue;
                    }
                    match frame.open::<CallClientMsg>() {
                        Ok(CallClientMsg::Wrap { to, wrap }) if wrap.len() <= WRAP_MAX => {
                            let signal = CallServerMsg::Wrap { from: seat.key, to, wrap };
                            publish(state, &seat.call, &signal).await;
                        }
                        // A second join, an oversized wrap or a frame that does not decode is
                        // dropped: the session is bound once, by its first frame.
                        _ => {}
                    }
                }
                signal = topic_rx.recv() => {
                    let Some(signal) = signal else { return };
                    let addressed = match signal.open::<CallServerMsg>() {
                        Ok(CallServerMsg::Wrap { to, .. }) => to == seat.key,
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
