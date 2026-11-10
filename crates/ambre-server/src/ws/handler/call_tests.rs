//! The call socket, database tier. It holds the admission, the cap and the sender indexes. It
//! holds the forwarding of the signed joins and of the wraps, and what the relay keeps of a call.

use super::*;

use ambre_core::{Payload, Sec, SenderKeyWraps, SignedJoins, TimeSpan};
use ambre_protocol::signaling::PARTICIPANT_CAP;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::test_helpers::{create_test_document, create_test_user, test_app_state};

/// A user of the relay, with the signing key its access token carries.
struct Member {
    user: Id<User>,
    key: FixedBytes<PublicKey>,
}

impl Member {
    async fn new(pool: &sqlx::PgPool, name: &str, key: u8) -> Self {
        let user = create_test_user(pool, &format!("{name}@ambre.app")).await;
        Self {
            user,
            key: FixedBytes::from_array([key; 32]),
        }
    }

    /// An access token of this member's session, as the relay mints one.
    fn token(&self, state: &AppState) -> ambre_core::Credential<ambre_core::AccessToken> {
        let iat = crate::s3::unix_now();
        state
            .jwt
            .mint(&crate::jwt::Claims {
                sub: self.user,
                spk: Some(self.key),
                iat,
                exp: iat.plus(TimeSpan::<Sec>::new(900)),
            })
            .expect("a token")
    }

    fn join(&self, state: &AppState, channel: Id<Channel>) -> CallClientMsg {
        CallClientMsg::Join {
            token: self.token(state),
            channel,
        }
    }
}

/// A channel of `owner`, with each of `members` a member of its `meta` document.
async fn channel_of(pool: &sqlx::PgPool, owner: &Member, members: &[&Member]) -> Id<Channel> {
    let channel = Id::<Channel>::mint();
    sqlx::query("INSERT INTO channels (id) VALUES ($1)")
        .bind(db::id_to_uuid(channel))
        .execute(pool)
        .await
        .expect("a channel");
    let meta = create_test_document(pool, owner.user, "murmure").await;
    sqlx::query(
        "INSERT INTO channel_documents (document_id, channel_id, kind) VALUES ($1, $2, 'meta')",
    )
    .bind(db::id_to_uuid(meta))
    .bind(db::id_to_uuid(channel))
    .execute(pool)
    .await
    .expect("a meta document");
    for member in members {
        db::add_document_member(
            pool,
            &meta,
            &db::NewMember {
                user_id: &member.user,
                role: ambre_core::Role::Editor,
                doc_key_enc: &ambre_core::Encoded::new(vec![6; 32]),
                invited_by: &owner.user,
            },
            None,
        )
        .await
        .expect("a member");
    }
    channel
}

/// The admission and the sender index of an admitted answer.
fn admitted(
    answer: CallAnswer,
) -> (
    ambre_core::Credential<ambre_core::CallAdmission>,
    Seq<SenderIndex>,
) {
    match answer {
        CallAnswer::Admitted {
            admission, sender, ..
        } => (admission, sender),
        CallAnswer::Refused(refusal) => panic!("refused: {refusal:?}"),
    }
}

/// The admission a token carries, after its signature verifies under the relay's key.
fn verified(
    state: &AppState,
    token: &ambre_core::Credential<ambre_core::CallAdmission>,
) -> Admission {
    use ed25519_dalek::Verifier;
    let wire = postcard::to_allocvec(token).expect("a token travels");
    let text: String = postcard::from_bytes(&wire).expect("a token is text");
    let bytes = hex::decode(text).expect("a token is hex");
    let (admission, signature) =
        Admission::read(&ambre_core::Payload::new(bytes)).expect("a token reads");
    let key = ed25519_dalek::VerifyingKey::from_bytes(
        state
            .jwt
            .admission_key()
            .expect("an admission key")
            .as_array(),
    )
    .expect("a key");
    let signed = admission.signed_bytes().expect("an admission encodes");
    key.verify(
        signed.as_bytes(),
        &ed25519_dalek::Signature::from_bytes(signature.as_array()),
    )
    .expect("the relay's signature verifies");
    admission
}

/// `ZK-023`: a member joins under a room pseudonym the relay drew for the call and a
/// participant pseudonym it drew for this join. A second join of the same member draws another
/// one, and neither token holds the user identifier or the key. The relay signs the admission,
/// and a stranger to the channel and a forged token are refused.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_member_joins_under_pseudonyms_the_relay_draws_per_join(pool: sqlx::PgPool) {
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let eve = Member::new(&pool, "eve", 2).await;
    let channel = channel_of(&pool, &alice, &[]).await;
    let call = Id::<Call>::mint();
    let (first, _) = admitted(
        answer_join(&state, &call, alice.join(&state, channel))
            .await
            .expect("a join"),
    );
    let (second, _) = admitted(
        answer_join(&state, &call, alice.join(&state, channel))
            .await
            .expect("a join"),
    );
    let (first, second) = (verified(&state, &first), verified(&state, &second));
    assert_eq!(first.room, second.room, "one call is one room");
    assert_ne!(
        first.participant, second.participant,
        "each join is a fresh participant"
    );
    for admission in [&first, &second] {
        let bytes = postcard::to_allocvec(admission).expect("an admission encodes");
        for named in [
            alice.user.as_bytes().as_slice(),
            alice.key.as_slice(),
            call.as_bytes(),
            channel.as_bytes(),
        ] {
            assert!(
                !bytes.windows(named.len()).any(|w| w == named),
                "an admission names {named:?}"
            );
        }
        assert!(admission.expires <= crate::s3::unix_now().plus(ADMISSION_LIFETIME));
    }
    assert!(matches!(
        answer_join(&state, &call, eve.join(&state, channel)).await,
        Ok(CallAnswer::Refused(CallRefusal::NotMember))
    ));
    let forged = CallClientMsg::Join {
        token: ambre_core::Credential::new("eyJforged".to_owned()),
        channel,
    };
    assert!(matches!(
        answer_join(&state, &call, forged).await,
        Ok(CallAnswer::Refused(CallRefusal::Unauthorized))
    ));
}

/// Serves the call socket and the socket of the document plane on a free port, and answers
/// the root of the sockets.
async fn served(state: Arc<AppState>) -> String {
    let app = axum::Router::new()
        .route("/api/calls/{call}/socket", axum::routing::get(call_upgrade))
        .route("/api/ws", axum::routing::get(crate::ws::ws_upgrade))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let root = format!("ws://{}", listener.local_addr().expect("an address"));
    tokio::spawn(async move { axum::serve(listener, app).await });
    root
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A call socket of `call` that sent `first`.
async fn call_socket(root: &str, call: Id<Call>, first: &CallClientMsg) -> Socket {
    let url = format!("{root}/api/calls/{call}/socket");
    let mut socket = tokio_tungstenite::connect_async(url)
        .await
        .expect("a socket")
        .0;
    let frame = StoredPayload::seal(first).expect("a frame encodes");
    socket
        .send(WsMessage::Binary(frame.expose_bytes().to_vec().into()))
        .await
        .expect("the first frame");
    socket
}

/// The next frame of the call socket, or `None` once it closes or falls silent for 5 s.
async fn next_signal(socket: &mut Socket) -> Option<CallServerMsg> {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .ok()??
            .ok()?;
        match frame {
            WsMessage::Binary(bytes) => {
                return StoredPayload::new(bytes.to_vec()).open().ok();
            }
            WsMessage::Close(_) => return None,
            _ => {}
        }
    }
}

/// `CALL-021`: a call at the participant cap refuses the next join with a notice, then closes
/// the socket. The roster keeps the cap, and the last seat under it is still admitted.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_join_at_the_participant_cap_is_refused_with_a_notice(pool: sqlx::PgPool) {
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let channel = channel_of(&pool, &alice, &[&bob]).await;
    let call = Id::<Call>::mint();
    let room = FixedBytes::from_array([3; 16]);
    let ttl = state.config.call_state_ttl;
    for _ in 1..PARTICIPANT_CAP.get() {
        let join =
            redis_ops::join_call(&state.redis, &call, &channel, &alice.key, &room, ttl).await;
        assert!(matches!(join, Ok(redis_ops::CallJoin::Admitted { .. })));
    }
    let root = served(state.clone()).await;
    let mut last = call_socket(&root, call, &bob.join(&state, channel)).await;
    assert!(matches!(
        next_signal(&mut last).await,
        Some(CallServerMsg::Admitted { .. })
    ));
    let mut refused = call_socket(&root, call, &bob.join(&state, channel)).await;
    assert!(matches!(
        next_signal(&mut refused).await,
        Some(CallServerMsg::Refused(CallRefusal::Full))
    ));
    assert!(
        next_signal(&mut refused).await.is_none(),
        "the socket closes after the notice"
    );
    let keys = crate::redis_keys::CallStateKeys::of(&call);
    let seats: usize = rustis::commands::HashCommands::hlen(&state.redis, keys.seats.as_str())
        .await
        .expect("a count");
    assert_eq!(seats, PARTICIPANT_CAP.get() as usize);
}

/// Sends `msg` on a call socket.
async fn send(socket: &mut Socket, msg: &CallClientMsg) {
    let frame = StoredPayload::seal(msg).expect("a frame encodes");
    socket
        .send(WsMessage::Binary(frame.expose_bytes().to_vec().into()))
        .await
        .expect("a frame");
}

/// The sender index and the roster of the first frame of an admitted socket.
async fn seat_of(socket: &mut Socket) -> (Seq<SenderIndex>, Vec<Seated>) {
    match next_signal(socket).await {
        Some(CallServerMsg::Admitted { sender, roster, .. }) => (sender, roster),
        other => panic!("no admission: {other:?}"),
    }
}

/// The signals a socket hears until it falls silent for 5 s.
async fn heard_until_silent(socket: &mut Socket) -> Vec<CallServerMsg> {
    let mut heard = Vec::new();
    while let Some(signal) = next_signal(socket).await {
        heard.push(signal);
    }
    heard
}

/// `CALL-058`, `SRV-049`, `ZK-022`: the call has an endpoint of its own. The last leave deletes
/// every key of the call. A wrap travels on the
/// call socket, and reaches the seat of its sender index alone, even a second session of the
/// same member. The socket of the document plane refuses a call frame as it refuses any first
/// frame but its own. The last leave ends the call.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_wrap_reaches_its_recipient_alone_on_the_call_socket(pool: sqlx::PgPool) {
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let channel = channel_of(&pool, &alice, &[&bob]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;

    let mut a = call_socket(&root, call, &alice.join(&state, channel)).await;
    let (from, _) = seat_of(&mut a).await;
    let mut b = call_socket(&root, call, &bob.join(&state, channel)).await;
    let (to, _) = seat_of(&mut b).await;
    // A second session of Bob, under the same key: the wrap is not for it.
    let mut c = call_socket(&root, call, &bob.join(&state, channel)).await;
    let _ = seat_of(&mut c).await;

    send(
        &mut a,
        &CallClientMsg::Wrap {
            to,
            wrap: ambre_core::Payload::<SenderKeyWraps>::new(vec![7; 80]),
        },
    )
    .await;
    let heard = loop {
        match next_signal(&mut b).await {
            Some(CallServerMsg::Wrap { from, to, wrap }) => break (from, to, wrap.len()),
            Some(_) => continue,
            None => panic!("bob heard no wrap"),
        }
    };
    assert_eq!(heard, (from, to, ambre_core::Count::new(80)));
    // The second session hears no wrap: its socket ends silent.
    for signal in heard_until_silent(&mut c).await {
        assert!(
            !matches!(signal, CallServerMsg::Wrap { .. }),
            "another seat heard a wrap"
        );
    }

    // The socket of the document plane takes no call frame: its first frame must authenticate.
    let mut plane = tokio_tungstenite::connect_async(format!("{root}/api/ws"))
        .await
        .expect("a socket")
        .0;
    let join = StoredPayload::seal(&alice.join(&state, channel)).expect("a frame encodes");
    plane
        .send(WsMessage::Binary(join.expose_bytes().to_vec().into()))
        .await
        .expect("a frame");
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(frame)) = plane.next().await {
            if let WsMessage::Close(frame) = frame {
                return frame.map(|f| u16::from(f.code));
            }
        }
        None
    })
    .await
    .expect("the plane answers");
    assert_eq!(closed, Some(Bounded::<CloseCode>::UNAUTHORIZED.get()));

    for socket in [&mut a, &mut b, &mut c] {
        socket.close(None).await.expect("a close");
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let keys = crate::redis_keys::CallStateKeys::of(&call);
        let left: usize = rustis::commands::GenericCommands::exists(&state.redis, keys.texts())
            .await
            .expect("the call");
        if left == 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the last leave deletes every key of the call"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `SRV-250`: no table of the relay schema holds a call, a roster or a join.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn no_table_of_the_relay_schema_holds_a_call(pool: sqlx::PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.columns \
         WHERE table_schema = current_schema() \
         AND (table_name LIKE '%call%' OR column_name LIKE '%call%' \
              OR column_name LIKE '%sender%' OR column_name LIKE '%room%')",
    )
    .fetch_all(&pool)
    .await
    .expect("query");
    assert!(tables.is_empty(), "a table holds a call: {tables:?}");
}

/// `ZK-022`, `SRV-248`: the state of a call in the ephemeral store holds four fields. They are
/// the channel, the room the relay drew, the next sender index and the end. Its seats hold the key of each
/// sender index, and its joins the opaque signed join of each. Nothing else, and each key
/// under the time to live.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn the_state_of_a_call_holds_only_what_the_relay_may_learn(pool: sqlx::PgPool) {
    use rustis::commands::{GenericCommands, HashCommands};
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let channel = channel_of(&pool, &alice, &[]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;
    let mut a = call_socket(&root, call, &alice.join(&state, channel)).await;
    let (sender, _) = seat_of(&mut a).await;
    send(&mut a, &CallClientMsg::Announce(Payload::new(vec![9; 192]))).await;
    let keys = crate::redis_keys::CallStateKeys::of(&call);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let joins: std::collections::HashMap<String, String> = loop {
        let joins: std::collections::HashMap<String, String> = state
            .redis
            .hgetall(keys.joins.as_str())
            .await
            .expect("joins");
        if !joins.is_empty() || std::time::Instant::now() > deadline {
            break joins;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let fields = |map: std::collections::HashMap<String, String>| {
        let mut fields: Vec<String> = map.into_keys().collect();
        fields.sort();
        fields
    };
    let call_state: std::collections::HashMap<String, String> = state
        .redis
        .hgetall(keys.state.as_str())
        .await
        .expect("state");
    assert_eq!(fields(call_state), ["channel", "next", "room"]);
    let seats: std::collections::HashMap<String, String> = state
        .redis
        .hgetall(keys.seats.as_str())
        .await
        .expect("seats");
    assert_eq!(
        seats.get(&sender.get().to_string()),
        Some(&alice.key.to_hex())
    );
    assert_eq!(fields(seats), [sender.get().to_string()]);
    assert_eq!(
        joins.get(&sender.get().to_string()),
        Some(&hex::encode([9; 192]))
    );
    let field_ttl: Vec<i64> = state
        .redis
        .httl(keys.seats.as_str(), [sender.get().to_string()])
        .await
        .expect("a field ttl");
    assert!(
        field_ttl
            .iter()
            .all(|t| *t > 0 && *t <= state.config.call_state_ttl.get() as i64),
        "the seat lives {field_ttl:?} s"
    );
    for key in keys.texts() {
        let ttl: i64 = state.redis.ttl(key).await.expect("a ttl");
        assert!(
            ttl > 0 && ttl <= state.config.call_state_ttl.get() as i64,
            "{key} lives {ttl} s"
        );
    }
}

/// A relay whose call state lives `ttl` and beats every `heartbeat`, over the services of
/// `base`.
fn with_call_clock(base: &Arc<AppState>, ttl: u64, heartbeat: u64) -> Arc<AppState> {
    Arc::new(AppState {
        config: Arc::new(crate::state::Config {
            call_state_ttl: ambre_core::TimeSpan::new(ttl),
            call_heartbeat: ambre_core::TimeSpan::new(heartbeat),
            ..crate::state::Config::default()
        }),
        db: base.db.clone(),
        redis: base.redis.clone(),
        pubsub: base.pubsub.clone(),
        s3: base.s3.clone(),
        s3_bucket: base.s3_bucket.clone(),
        jwt: base.jwt.clone(),
        oidc_clients: base.oidc_clients.clone(),
    })
}

/// `SRV-249`: the store loses the state of a call under an open socket, as a restart of the
/// store does. A heartbeat keeps the state past its time to live first. Then the next beat
/// finds it gone: the relay sends a notice and closes the socket. A rejoin starts a fresh call,
/// under a new room, from the first sender index.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_call_whose_state_is_lost_ends(pool: sqlx::PgPool) {
    use rustis::commands::GenericCommands;
    let state = with_call_clock(&test_app_state(pool.clone()).await, 2, 1);
    let alice = Member::new(&pool, "alice", 1).await;
    let channel = channel_of(&pool, &alice, &[]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;
    let mut a = call_socket(&root, call, &alice.join(&state, channel)).await;
    let first = match next_signal(&mut a).await {
        Some(CallServerMsg::Admitted { admission, .. }) => verified(&state, &admission),
        other => panic!("no admission: {other:?}"),
    };
    let keys = crate::redis_keys::CallStateKeys::of(&call);
    // Past the time to live, the heartbeat still holds the state.
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let held: usize = state
        .redis
        .exists(keys.state.as_str())
        .await
        .expect("exists");
    assert_eq!(held, 1, "a beating call holds its state");
    let _: usize = state.redis.del(keys.texts()).await.expect("a loss");
    let heard = tokio::time::timeout(Duration::from_secs(5), async {
        let mut heard = Vec::new();
        loop {
            match a.next().await {
                Some(Ok(WsMessage::Binary(bytes))) => {
                    heard.extend(
                        StoredPayload::new(bytes.to_vec())
                            .open::<CallServerMsg>()
                            .ok(),
                    );
                }
                Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => return heard,
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .expect("the socket of a lost call closes");
    assert!(
        heard.iter().any(|m| matches!(m, CallServerMsg::Lost)),
        "no notice of the loss: {heard:?}"
    );
    let mut again = call_socket(&root, call, &alice.join(&state, channel)).await;
    match next_signal(&mut again).await {
        Some(CallServerMsg::Admitted {
            admission, sender, ..
        }) => {
            assert_eq!(sender, Seq::new(0), "a fresh call counts from the start");
            assert_ne!(
                verified(&state, &admission).room,
                first.room,
                "a fresh room"
            );
        }
        other => panic!("no admission: {other:?}"),
    }
}

/// `SRV-248`: a seat that no heartbeat holds, as on a relay that stopped, leaves nothing of
/// its call past the time to live. Its seat field, then every key of the call, are gone.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn nothing_of_a_call_outlives_its_time_to_live_without_a_heartbeat(pool: sqlx::PgPool) {
    use rustis::commands::GenericCommands;
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let channel = channel_of(&pool, &alice, &[]).await;
    let call = Id::<Call>::mint();
    let ttl = ambre_core::TimeSpan::new(1);
    let room = FixedBytes::from_array([4; 16]);
    let joined = redis_ops::join_call(&state.redis, &call, &channel, &alice.key, &room, ttl).await;
    let Ok(redis_ops::CallJoin::Admitted { sender, .. }) = joined else {
        panic!("no seat: {joined:?}");
    };
    let announced = redis_ops::announce_join(
        &state.redis,
        &call,
        sender,
        &Payload::new(vec![5; 192]),
        ttl,
    )
    .await;
    assert_eq!(announced.ok(), Some(true));
    let keys = crate::redis_keys::CallStateKeys::of(&call);
    let present: usize = state.redis.exists(keys.texts()).await.expect("exists");
    assert_eq!(present, 3, "the call holds its three keys");
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let left: usize = state.redis.exists(keys.texts()).await.expect("exists");
    assert_eq!(left, 0, "a key of the call outlived its time to live");
}

/// An admission the relay issues verifies under the admission key alone, never under the key
/// of the access tokens. The SFU holds `SFU_ADMISSION_KEY`. So it refuses a token signed with
/// the session key, and holds no key that verifies a session
/// (`ambre-spec-call-media.md` section 3, the admission token).
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn an_admission_never_verifies_under_the_session_key(pool: sqlx::PgPool) {
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let channel = channel_of(&pool, &alice, &[]).await;
    let (token, _) = admitted(
        answer_join(&state, &Id::<Call>::mint(), alice.join(&state, channel))
            .await
            .expect("a join"),
    );
    let admission = verified(&state, &token);
    let wire = postcard::to_allocvec(&token).expect("a token travels");
    let text: String = postcard::from_bytes(&wire).expect("a token is text");
    let (_, signature) =
        Admission::read(&ambre_core::Payload::new(hex::decode(text).expect("hex")))
            .expect("a token reads");
    let session = crate::jwt::JwtSeed::from_array(crate::test_helpers::TEST_JWT_SEED);
    let session_key =
        ed25519_dalek::VerifyingKey::from_bytes(&session.verifying_key_bytes()).expect("a key");
    let signed = admission.signed_bytes().expect("an admission encodes");
    assert!(
        session_key
            .verify_strict(
                signed.as_bytes(),
                &ed25519_dalek::Signature::from_bytes(signature.as_array()),
            )
            .is_err(),
        "an admission verifies under the session key"
    );
}

/// `CALL-056`: each join of a call gets a sender index that no earlier join of the call held,
/// a join after a leave included. Past the last index a key identifier carries, a join is
/// refused with a notice, and the relay reuses no index.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn each_join_of_a_call_gets_a_sender_index_no_earlier_join_held(pool: sqlx::PgPool) {
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let channel = channel_of(&pool, &alice, &[&bob]).await;
    let call = Id::<Call>::mint();
    let mut given = Vec::new();
    for member in [&alice, &bob, &alice, &bob, &alice] {
        let (_, sender) = admitted(
            answer_join(&state, &call, member.join(&state, channel))
                .await
                .expect("a join"),
        );
        given.push(sender);
        // Every other join leaves at once, so a later join could take its index back.
        if given.len() % 2 == 0 {
            redis_ops::leave_call(&state.redis, &call, sender)
                .await
                .expect("a leave");
        }
    }
    let mut distinct = given.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        given.len(),
        "an index was given twice: {given:?}"
    );
    assert!(given.windows(2).all(|w| w.first() < w.get(1)), "{given:?}");

    let keys = crate::redis_keys::CallStateKeys::of(&call);
    let _: usize = rustis::commands::HashCommands::hset(
        &state.redis,
        keys.state.as_str(),
        [("next", ambre_core::SENDER_INDEX_LIMIT.get().to_string())],
    )
    .await
    .expect("the counter at the limit");
    assert!(matches!(
        answer_join(&state, &call, bob.join(&state, channel)).await,
        Ok(CallAnswer::Refused(CallRefusal::Exhausted))
    ));
}

/// `CALL-057`: the signed join a seat announces reaches every other seat unchanged. A seat
/// in the call hears it. A seat that joins later reads it in its roster, with its index. The seat hears no echo of its own join, and a second announce changes
/// nothing.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_signed_join_reaches_every_other_seat_unchanged(pool: sqlx::PgPool) {
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let carol = Member::new(&pool, "carol", 3).await;
    let channel = channel_of(&pool, &alice, &[&bob, &carol]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;
    let join = |byte: u8| Payload::<SignedJoins>::new(vec![byte; 192]);

    let mut b = call_socket(&root, call, &bob.join(&state, channel)).await;
    let (bob_sender, roster) = seat_of(&mut b).await;
    assert!(roster.is_empty(), "no seat announced yet: {roster:?}");
    let mut a = call_socket(&root, call, &alice.join(&state, channel)).await;
    let (alice_sender, roster) = seat_of(&mut a).await;
    assert!(roster.is_empty(), "bob has not announced: {roster:?}");
    send(&mut a, &CallClientMsg::Announce(join(1))).await;
    send(&mut a, &CallClientMsg::Announce(join(2))).await;

    let heard = loop {
        match next_signal(&mut b).await {
            Some(CallServerMsg::Joined(seated)) => break seated,
            Some(_) => continue,
            None => panic!("bob heard no join"),
        }
    };
    assert_eq!(
        heard,
        Seated {
            sender: alice_sender,
            join: join(1)
        }
    );
    let mut c = call_socket(&root, call, &carol.join(&state, channel)).await;
    let (carol_sender, roster) = seat_of(&mut c).await;
    assert_eq!(
        roster,
        vec![Seated {
            sender: alice_sender,
            join: join(1)
        }]
    );
    assert!(![alice_sender, bob_sender].contains(&carol_sender));
    for signal in heard_until_silent(&mut a).await {
        assert!(
            !matches!(&signal, CallServerMsg::Joined(seated) if seated.sender == alice_sender),
            "alice heard her own join"
        );
    }
    for signal in heard_until_silent(&mut b).await {
        assert!(
            !matches!(signal, CallServerMsg::Joined(_)),
            "a second announce reached bob"
        );
    }
}
