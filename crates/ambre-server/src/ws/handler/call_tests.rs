//! The call socket, database tier. It holds the admission, the cap and the sender indexes. It
//! holds the forwarding of the signed joins and of the wraps, and what the relay keeps of a call.

use super::*;

use ambre_core::{Payload, Sec, SenderKeyWraps, SignedJoins, TimeSpan};
use ambre_protocol::signaling::PARTICIPANT_CAP;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::test_helpers::{create_test_document, create_test_user, test_app_state};

/// The opening of a join that creates the call when it holds no state.
const START: redis_ops::CallOpening = redis_ops::CallOpening::Start;

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

    /// A first frame that starts the call, or joins it when it runs.
    fn join(&self, state: &AppState, channel: Id<Channel>) -> CallClientMsg {
        CallClientMsg::Start {
            token: self.token(state),
            channel,
        }
    }

    /// A first frame that joins a running call alone, as a join from a call message does.
    fn join_running(&self, state: &AppState, channel: Id<Channel>) -> CallClientMsg {
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
/// one, and neither token holds the user identifier or the key. Both name one member
/// pseudonym, and another member of the call names another (audit `N25`). The relay signs the
/// admission, and a stranger to the channel and a forged token are refused.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_member_joins_under_pseudonyms_the_relay_draws_per_join(pool: sqlx::PgPool) {
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let eve = Member::new(&pool, "eve", 2).await;
    let bob = Member::new(&pool, "bob", 3).await;
    let channel = channel_of(&pool, &alice, &[&bob]).await;
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
    assert_eq!(
        first.member, second.member,
        "one member is one member pseudonym"
    );
    let (third, _) = admitted(
        answer_join(&state, &call, bob.join(&state, channel))
            .await
            .expect("a join"),
    );
    assert_ne!(
        verified(&state, &third).member,
        first.member,
        "two members are two member pseudonyms"
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
    // One key per seat: one key holds a few seats alone (`SRV-306`).
    for seat in 1..PARTICIPANT_CAP.get() {
        let mut bytes = [0xEE; 32];
        bytes[0] = u8::try_from(seat).expect("a seat under 256");
        let key = FixedBytes::from_array(bytes);
        let join =
            redis_ops::join_call(&state.redis, &call, &channel, &key, &room, START, ttl).await;
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

/// `SRV-306`, audit `P42`: one key holds a few seats of a call, and takes a
/// bounded count of its sender indexes. Past the seats it reads the call as
/// full, past the indexes as exhausted, and another member is still admitted.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn one_member_holds_a_bounded_share_of_a_call(pool: sqlx::PgPool) {
    use rustis::commands::{GenericCommands, HashCommands};
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let channel = channel_of(&pool, &alice, &[&bob]).await;
    let call = Id::<Call>::mint();
    let room = FixedBytes::from_array([7; 16]);
    let ttl = state.config.call_state_ttl;
    let join = |key| redis_ops::join_call(&state.redis, &call, &channel, key, &room, START, ttl);
    for _ in 0..4 {
        assert!(matches!(
            join(&alice.key).await,
            Ok(redis_ops::CallJoin::Admitted { .. })
        ));
    }
    assert!(matches!(
        join(&alice.key).await,
        Ok(redis_ops::CallJoin::Refused(CallRefusal::Full))
    ));
    assert!(matches!(
        join(&bob.key).await,
        Ok(redis_ops::CallJoin::Admitted { .. })
    ));

    // Bob's seats leave, and his key has spent its share of the indexes.
    let keys = crate::redis_keys::CallStateKeys::of(&call);
    let _: usize = state.redis.del(keys.seats.as_str()).await.expect("del");
    let _: usize = state
        .redis
        .hset(
            keys.state.as_str(),
            [(format!("taken:{}", bob.key.to_hex()), "256".to_owned())],
        )
        .await
        .expect("hset");
    assert!(matches!(
        join(&bob.key).await,
        Ok(redis_ops::CallJoin::Refused(CallRefusal::Exhausted))
    ));
    assert!(matches!(
        join(&alice.key).await,
        Ok(redis_ops::CallJoin::Admitted { .. })
    ));
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
    // A call frame carries no wire version, so its first byte reads as an arbitrary one.
    // The plane refuses it either as an unauthenticated frame or as an older wire.
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
    let refusals = [
        Bounded::<CloseCode>::UNAUTHORIZED.get(),
        Bounded::<CloseCode>::UPDATE_REQUIRED.get(),
    ];
    assert!(
        closed.is_some_and(|code| refusals.contains(&code)),
        "the plane refuses a call frame, got {closed:?}"
    );

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

/// `ZK-022`, `SRV-248`: the state of a call in the ephemeral store holds the channel, the room
/// the relay drew and the next sender index. It also holds, per public key, the count of sender
/// indexes the key took (`SRV-306`). That count follows from the key of each sender index, which the relay learns
/// already. Its seats hold the key of each sender index, and its joins the opaque signed join
/// of each. Nothing else, and each key under the time to live.
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
    let taken = format!("taken:{}", alice.key.to_hex());
    assert_eq!(call_state.get(&taken).map(String::as_str), Some("1"));
    assert_eq!(
        fields(call_state),
        ["channel", "next", "room", taken.as_str()]
    );
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
/// finds it gone: the relay sends a notice and closes the socket. A rejoin is refused as
/// ended (`CALL-068`). A start under the same identifier is a fresh call, under a new room,
/// from the first sender index.
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
    // Past the time to live, the heartbeat still holds the state. The socket is read
    // meanwhile, so it answers each ping as a browser does (`CALL-073`).
    let _ = tokio::time::timeout(Duration::from_millis(3500), async {
        while a.next().await.is_some() {}
    })
    .await;
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
    let mut rejoin = call_socket(&root, call, &alice.join_running(&state, channel)).await;
    assert!(
        matches!(
            next_signal(&mut rejoin).await,
            Some(CallServerMsg::Refused(CallRefusal::Ended))
        ),
        "a rejoin of a lost call is refused as ended"
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

/// `SRV-251`, `CALL-063`: a seat whose socket stops its heartbeat leaves the roster, as on a
/// relay that died. It frees its place under the cap, while a beating seat holds the call.
/// The heartbeat that finds it gone announces its leave, and its index is never given again.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_seat_that_stops_its_heartbeat_leaves_the_roster_and_frees_its_place(pool: sqlx::PgPool) {
    use rustis::commands::HashCommands;
    let state = with_call_clock(&test_app_state(pool.clone()).await, 2, 1);
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let carol = Member::new(&pool, "carol", 3).await;
    let channel = channel_of(&pool, &alice, &[&bob, &carol]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;
    let mut b = call_socket(&root, call, &bob.join(&state, channel)).await;
    let (bob_seat, _) = seat_of(&mut b).await;
    // Alice's seat comes from a relay that stops at once: no socket beats for it.
    let ttl = state.config.call_state_ttl;
    let room = FixedBytes::from_array([0; 16]);
    let ghost =
        match redis_ops::join_call(&state.redis, &call, &channel, &alice.key, &room, START, ttl)
            .await
        {
            Ok(redis_ops::CallJoin::Admitted { sender, .. }) => sender,
            other => panic!("no seat: {other:?}"),
        };
    let announced =
        redis_ops::announce_join(&state.redis, &call, ghost, &Payload::new(vec![6; 192]), ttl)
            .await;
    assert_eq!(announced.ok(), Some(true));
    let keys = crate::redis_keys::CallStateKeys::of(&call);
    let seats: Vec<String> = state.redis.hkeys(keys.seats.as_str()).await.expect("seats");
    assert_eq!(seats.len(), 2, "two seats under the cap: {seats:?}");

    let left = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            match next_signal(&mut b).await {
                Some(CallServerMsg::Left(sender)) => return Some(sender),
                Some(_) => {}
                None => return None,
            }
        }
    })
    .await;
    assert_eq!(left, Ok(Some(ghost)), "bob hears the ghost leave");
    let seats: Vec<String> = state.redis.hkeys(keys.seats.as_str()).await.expect("seats");
    assert_eq!(
        seats,
        [bob_seat.get().to_string()],
        "the ghost freed its place"
    );

    let mut c = call_socket(&root, call, &carol.join(&state, channel)).await;
    let (carol_seat, roster) = seat_of(&mut c).await;
    assert!(
        roster.iter().all(|seated| seated.sender != ghost),
        "a later roster holds the ghost: {roster:?}"
    );
    assert!(
        carol_seat > ghost && carol_seat > bob_seat,
        "an index came back"
    );
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
    let joined =
        redis_ops::join_call(&state.redis, &call, &channel, &alice.key, &room, START, ttl).await;
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
    assert_eq!(present, 4, "the call holds its four keys");
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

/// What the relay answers `asker` on an ask of who is in `call` of `channel`, through the
/// handler of the route.
async fn ask(
    state: &Arc<AppState>,
    asker: &Member,
    call: Id<Call>,
    channel: Id<Channel>,
) -> ambre_protocol::signaling::CallRoster {
    use axum::response::IntoResponse;
    let answer = crate::routes::call_roster(
        State(state.clone()),
        crate::auth::AuthSession {
            user_id: asker.user,
            signing_pubkey: asker.key,
        },
        crate::extract::OpaquePathId::new(call),
        axum::extract::Query(crate::routes::RosterQuery { channel }),
    )
    .await
    .expect("an answer")
    .into_response();
    assert_eq!(answer.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(answer.into_body(), 1 << 20)
        .await
        .expect("a body");
    serde_json::from_slice(&body).expect("a roster")
}

/// Waits until `asker` reads `want` for `call`, or fails after 5 s.
async fn ask_until(
    state: &Arc<AppState>,
    asker: &Member,
    call: Id<Call>,
    channel: Id<Channel>,
    want: &ambre_protocol::signaling::CallRoster,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let read = ask(state, asker, call, channel).await;
        if read == *want {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the roster read {read:?}, not {want:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `CALL-068`: a join that starts nothing gets the notice that the call ended, when the relay
/// holds no state of its call. Then the socket closes. A call its last leave ended
/// refuses a join the same way, and keeps no state.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_join_of_a_call_the_relay_holds_no_state_of_is_refused_as_ended(pool: sqlx::PgPool) {
    use rustis::commands::GenericCommands;
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let channel = channel_of(&pool, &alice, &[&bob]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;
    let mut early = call_socket(&root, call, &bob.join_running(&state, channel)).await;
    assert!(matches!(
        next_signal(&mut early).await,
        Some(CallServerMsg::Refused(CallRefusal::Ended))
    ));
    assert!(next_signal(&mut early).await.is_none(), "the socket closes");
    let keys = crate::redis_keys::CallStateKeys::of(&call);
    let held: usize = state.redis.exists(keys.texts()).await.expect("exists");
    assert_eq!(held, 0, "a refused join creates no state");

    let mut a = call_socket(&root, call, &alice.join(&state, channel)).await;
    let _ = seat_of(&mut a).await;
    let mut b = call_socket(&root, call, &bob.join_running(&state, channel)).await;
    let _ = seat_of(&mut b).await;
    drop(a);
    drop(b);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while state.redis.exists(keys.texts()).await.unwrap_or(1) > 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the last leave ends the call"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut late = call_socket(&root, call, &bob.join_running(&state, channel)).await;
    assert!(matches!(
        next_signal(&mut late).await,
        Some(CallServerMsg::Refused(CallRefusal::Ended))
    ));
}

/// `CALL-069`: a leave frame takes the seat out, and the relay answers whether it was the
/// last, then closes. The other seat hears the leave. The last leave reads `Last`, and the
/// call keeps no state.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_leave_answers_whether_its_seat_was_the_last(pool: sqlx::PgPool) {
    use ambre_protocol::signaling::Departure;
    use rustis::commands::GenericCommands;
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let channel = channel_of(&pool, &alice, &[&bob]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;
    let mut a = call_socket(&root, call, &alice.join(&state, channel)).await;
    let (first, _) = seat_of(&mut a).await;
    let mut b = call_socket(&root, call, &bob.join_running(&state, channel)).await;
    let _ = seat_of(&mut b).await;

    send(&mut a, &CallClientMsg::Leave).await;
    assert!(matches!(
        next_signal(&mut a).await,
        Some(CallServerMsg::Departed(Departure::Others))
    ));
    assert!(next_signal(&mut a).await.is_none(), "the socket closes");
    let heard = heard_until_silent(&mut b).await;
    assert!(
        heard
            .iter()
            .any(|m| matches!(m, CallServerMsg::Left(s) if *s == first)),
        "the other seat hears the leave: {heard:?}"
    );

    let keys = crate::redis_keys::CallStateKeys::of(&call);
    send(&mut b, &CallClientMsg::Leave).await;
    let mut answers = Vec::new();
    while let Some(signal) = next_signal(&mut b).await {
        answers.push(signal);
    }
    assert!(
        answers
            .iter()
            .any(|m| matches!(m, CallServerMsg::Departed(Departure::Last))),
        "the last leave reads last: {answers:?}"
    );
    let held: usize = state.redis.exists(keys.texts()).await.expect("exists");
    assert_eq!(held, 0, "the last leave drops the state");
}

/// `CALL-070`: Carol, a member of the channel out of the call, reads the signed
/// join of each seat, as each seat announced it. Once the last seat leaves, she reads that the
/// relay holds no state.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_member_out_of_the_call_reads_the_signed_join_of_each_seat(pool: sqlx::PgPool) {
    use ambre_protocol::signaling::CallRoster;
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let carol = Member::new(&pool, "carol", 3).await;
    let channel = channel_of(&pool, &alice, &[&bob, &carol]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;
    let mut a = call_socket(&root, call, &alice.join(&state, channel)).await;
    let (sa, _) = seat_of(&mut a).await;
    send(&mut a, &CallClientMsg::Announce(Payload::new(vec![1; 192]))).await;
    let mut b = call_socket(&root, call, &bob.join_running(&state, channel)).await;
    let (sb, _) = seat_of(&mut b).await;
    send(&mut b, &CallClientMsg::Announce(Payload::new(vec![2; 192]))).await;
    let live = CallRoster::Live(vec![
        Seated {
            sender: sa,
            join: Payload::<SignedJoins>::new(vec![1; 192]),
        },
        Seated {
            sender: sb,
            join: Payload::<SignedJoins>::new(vec![2; 192]),
        },
    ]);
    ask_until(&state, &carol, call, channel, &live).await;
    send(&mut a, &CallClientMsg::Leave).await;
    send(&mut b, &CallClientMsg::Leave).await;
    ask_until(&state, &carol, call, channel, &CallRoster::Gone).await;
}

/// `CALL-071`: Eve, no member of the channel, reads the answer of a call with no state while
/// the call runs. So does an ask that names another channel.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_stranger_or_another_channel_reads_a_running_call_as_gone(pool: sqlx::PgPool) {
    use ambre_protocol::signaling::CallRoster;
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let eve = Member::new(&pool, "eve", 5).await;
    let channel = channel_of(&pool, &alice, &[]).await;
    let other = channel_of(&pool, &alice, &[]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;
    let mut a = call_socket(&root, call, &alice.join(&state, channel)).await;
    let _ = seat_of(&mut a).await;
    assert!(matches!(
        ask(&state, &alice, call, channel).await,
        CallRoster::Live(_)
    ));
    assert_eq!(ask(&state, &eve, call, channel).await, CallRoster::Gone);
    assert_eq!(ask(&state, &alice, call, other).await, CallRoster::Gone);
}

/// Every key of the store that names `call`, each with its type, its value and whether it
/// expires. Other tests share the store, so the dump reads the keys of this call alone.
async fn store_dump(state: &AppState, call: Id<Call>) -> Vec<(String, String)> {
    use rustis::commands::{GenericCommands, HashCommands, SetCommands};
    let mut keys: Vec<String> = state.redis.keys(format!("*{call}*")).await.expect("keys");
    keys.sort();
    let mut dump = Vec::new();
    for key in keys {
        let kind: String = state.redis.type_(key.as_str()).await.expect("a type");
        let value = match kind.as_str() {
            "hash" => {
                let mut fields: Vec<(String, String)> = state
                    .redis
                    .hgetall::<Vec<(String, String)>>(key.as_str())
                    .await
                    .expect("a hash");
                fields.sort();
                format!("{fields:?}")
            }
            "set" => {
                let mut members: Vec<String> =
                    state.redis.smembers(key.as_str()).await.expect("a set");
                members.sort();
                format!("{members:?}")
            }
            other => other.to_owned(),
        };
        let ttl: i64 = state.redis.ttl(key.as_str()).await.expect("a ttl");
        dump.push((key, format!("{value} expires {}", ttl > 0)));
    }
    dump
}

/// `CALL-072`, `ZK-024`: an ask writes nothing to the store. Every key that names the call reads
/// the same before and after the asks, with its fields and its expiry. No key joins them. The
/// time to live of the call shrinks under the asks alone, so an ask refreshes nothing of it.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn an_ask_changes_nothing_of_the_store(pool: sqlx::PgPool) {
    use rustis::commands::GenericCommands;
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let carol = Member::new(&pool, "carol", 3).await;
    let channel = channel_of(&pool, &alice, &[&carol]).await;
    let call = Id::<Call>::mint();
    let room = FixedBytes::from_array([4; 16]);
    let ttl = state.config.call_state_ttl;
    let joined =
        redis_ops::join_call(&state.redis, &call, &channel, &alice.key, &room, START, ttl).await;
    assert!(matches!(joined, Ok(redis_ops::CallJoin::Admitted { .. })));
    let keys = crate::redis_keys::CallStateKeys::of(&call);
    let before = store_dump(&state, call).await;
    let lived: i64 = state.redis.pttl(keys.state.as_str()).await.expect("a ttl");
    for _ in 0..3 {
        let _ = ask(&state, &carol, call, channel).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let after = store_dump(&state, call).await;
    let lives: i64 = state.redis.pttl(keys.state.as_str()).await.expect("a ttl");
    assert_eq!(before, after, "an ask wrote to the store");
    assert!(
        lives <= lived - 1000,
        "an ask refreshed the call: {lived} ms then {lives} ms"
    );
}

/// The relay half of a call that ends unwritten. The last seat of a call stops its heartbeat,
/// as when its participant crashes. The call reads as gone within the time to live, though no
/// leave came.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_call_whose_last_seat_stops_its_heartbeat_reads_as_gone(pool: sqlx::PgPool) {
    use ambre_protocol::signaling::CallRoster;
    let state = with_call_clock(&test_app_state(pool.clone()).await, 1, 1);
    let alice = Member::new(&pool, "alice", 1).await;
    let carol = Member::new(&pool, "carol", 3).await;
    let channel = channel_of(&pool, &alice, &[&carol]).await;
    let call = Id::<Call>::mint();
    let room = FixedBytes::from_array([4; 16]);
    let ttl = state.config.call_state_ttl;
    let joined =
        redis_ops::join_call(&state.redis, &call, &channel, &alice.key, &room, START, ttl).await;
    assert!(matches!(joined, Ok(redis_ops::CallJoin::Admitted { .. })));
    assert!(matches!(
        ask(&state, &carol, call, channel).await,
        CallRoster::Live(_)
    ));
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(ask(&state, &carol, call, channel).await, CallRoster::Gone);
}

/// `CALL-160`: Alice and Bob start a call in one channel at one instant. Each start names an
/// identifier of its own. The key of the live call of the channel
/// takes one start, and the other start sits in that call. So the relay holds one call under
/// one room, and its roster holds both starters. Once the last leaves, a start makes a new call.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn two_starts_at_once_make_one_call_that_holds_both_starters(pool: sqlx::PgPool) {
    use ambre_protocol::signaling::CallRoster;
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let carol = Member::new(&pool, "carol", 3).await;
    let channel = channel_of(&pool, &alice, &[&bob, &carol]).await;
    let (first, second) = (Id::<Call>::mint(), Id::<Call>::mint());
    let root = served(state.clone()).await;
    let (start_a, start_b) = (alice.join(&state, channel), bob.join(&state, channel));
    let (mut a, mut b) = tokio::join!(
        call_socket(&root, first, &start_a),
        call_socket(&root, second, &start_b),
    );
    let seated = |msg: Option<CallServerMsg>| match msg {
        Some(CallServerMsg::Admitted {
            call, admission, ..
        }) => (call, verified(&state, &admission).room),
        other => panic!("no admission: {other:?}"),
    };
    let ((ca, ra), (cb, rb)) = (
        seated(next_signal(&mut a).await),
        seated(next_signal(&mut b).await),
    );
    assert_eq!((ca, ra), (cb, rb), "one call, one room");
    assert!(
        ca == first || ca == second,
        "the call of one of the two starts"
    );
    send(&mut a, &CallClientMsg::Announce(Payload::new(vec![1; 192]))).await;
    send(&mut b, &CallClientMsg::Announce(Payload::new(vec![2; 192]))).await;
    let roster = loop {
        if let CallRoster::Live(seats) = ask(&state, &carol, ca, channel).await
            && seats.len() == 2
        {
            break seats;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let mut joins: Vec<u8> = roster.iter().map(|s| s.join.as_bytes()[0]).collect();
    joins.sort_unstable();
    assert_eq!(joins, vec![1, 2]);

    send(&mut a, &CallClientMsg::Leave).await;
    send(&mut b, &CallClientMsg::Leave).await;
    ask_until(&state, &carol, ca, channel, &CallRoster::Gone).await;
    let third = Id::<Call>::mint();
    let mut c = call_socket(&root, third, &carol.join(&state, channel)).await;
    assert_eq!(
        seated(next_signal(&mut c).await).0,
        third,
        "a new call once the last left"
    );
}

/// `CALL-161`: a call whose state the store lost with no last leave leaves the key of the
/// live call of its channel behind. A later start reads that call as ended, takes the key, and
/// creates its own call.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_start_takes_the_channel_from_a_call_the_store_lost(pool: sqlx::PgPool) {
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let channel = channel_of(&pool, &alice, &[&bob]).await;
    let root = served(state.clone()).await;
    let (lost, fresh) = (Id::<Call>::mint(), Id::<Call>::mint());
    let mut a = call_socket(&root, lost, &alice.join(&state, channel)).await;
    let _ = seat_of(&mut a).await;
    use rustis::commands::GenericCommands;
    let keys = crate::redis_keys::CallStateKeys::of(&lost);
    let _: usize = state.redis.del(keys.texts()).await.expect("a loss");
    let mut b = call_socket(&root, fresh, &bob.join(&state, channel)).await;
    match next_signal(&mut b).await {
        Some(CallServerMsg::Admitted { call, .. }) => assert_eq!(call, fresh),
        other => panic!("no admission: {other:?}"),
    }
}

/// `CALL-073`: a participant whose network died sends nothing, and answers no
/// ping. Its socket stays silent for one time to live, so the relay closes it and takes its
/// seat out. Its call, of which it held the last seat, then reads as gone. A socket that
/// answers the pings keeps its seat past the time to live.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_call_socket_silent_for_the_time_to_live_leaves_its_seat(pool: sqlx::PgPool) {
    use ambre_protocol::signaling::CallRoster;
    let state = with_call_clock(&test_app_state(pool.clone()).await, 2, 1);
    let alice = Member::new(&pool, "alice", 1).await;
    let carol = Member::new(&pool, "carol", 3).await;
    // Two channels, since a start in a channel with a live call sits in that call
    // (`CALL-160`).
    let channel = channel_of(&pool, &alice, &[&carol]).await;
    let other = channel_of(&pool, &alice, &[&carol]).await;
    let root = served(state.clone()).await;
    let (silent, answering) = (Id::<Call>::mint(), Id::<Call>::mint());
    let mut a = call_socket(&root, silent, &alice.join(&state, channel)).await;
    let _ = seat_of(&mut a).await;
    let mut b = call_socket(&root, answering, &alice.join(&state, other)).await;
    let _ = seat_of(&mut b).await;
    // Reading the socket answers each ping, as a browser does.
    let reader = tokio::spawn(async move { while b.next().await.is_some() {} });
    tokio::time::sleep(Duration::from_millis(4500)).await;
    assert_eq!(ask(&state, &carol, silent, channel).await, CallRoster::Gone);
    assert!(matches!(
        ask(&state, &carol, answering, other).await,
        CallRoster::Live(_)
    ));
    reader.abort();
    drop(a);
}

/// Audit `N25`: each heartbeat of a seat whose member is in the channel sends a fresh
/// admission of that seat, under a later expiry. A member removed from the channel gets no
/// renewal: the relay ends the seat, so the SFU ends its connection when the last admission
/// expires.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_heartbeat_renews_the_admission_of_a_member_in_the_channel_alone(pool: sqlx::PgPool) {
    use rustis::commands::HashCommands;
    let state = with_call_clock(&test_app_state(pool.clone()).await, 3, 1);
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let channel = channel_of(&pool, &alice, &[&bob]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;
    let mut socket = call_socket(&root, call, &bob.join(&state, channel)).await;
    let (first, sender) = match next_signal(&mut socket).await {
        Some(CallServerMsg::Admitted {
            admission, sender, ..
        }) => (verified(&state, &admission), sender),
        other => panic!("no admission: {other:?}"),
    };
    let renewed = loop {
        match next_signal(&mut socket).await {
            Some(CallServerMsg::Renewed(admission)) => break verified(&state, &admission),
            Some(_) => {}
            None => panic!("no renewal of a member's seat"),
        }
    };
    assert_eq!(
        Admission {
            expires: first.expires,
            ..renewed.clone()
        },
        first,
        "a renewal names the seat of the first admission"
    );
    assert!(renewed.expires >= first.expires);

    sqlx::query("DELETE FROM document_members WHERE user_id = $1")
        .bind(db::id_to_uuid(bob.user))
        .execute(&pool)
        .await
        .expect("a removal");
    let after = tokio::time::timeout(Duration::from_secs(5), async {
        let mut renewals = 0;
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(bytes))) => {
                    if let Ok(CallServerMsg::Renewed(_)) = StoredPayload::new(bytes.to_vec()).open()
                    {
                        renewals += 1;
                    }
                }
                Some(Ok(WsMessage::Close(_)) | Err(_)) | None => return renewals,
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .expect("the seat of a removed member ends");
    assert!(after <= 1, "{after} renewals reached a removed member");
    let keys = crate::redis_keys::CallStateKeys::of(&call);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let held: bool = state
        .redis
        .hexists(keys.seats.as_str(), sender.get().to_string())
        .await
        .expect("a read");
    assert!(!held, "the seat of a removed member stays in the roster");
}

/// Asks the relay for the live call of each channel of `asker` (`CALL-162`).
async fn ask_channel_calls(
    state: &Arc<AppState>,
    asker: &Member,
) -> Vec<ambre_protocol::signaling::ChannelCall> {
    use axum::response::IntoResponse;
    let answer = crate::routes::channel_calls(
        State(state.clone()),
        crate::auth::AuthSession {
            user_id: asker.user,
            signing_pubkey: asker.key,
        },
    )
    .await
    .expect("an answer")
    .into_response();
    assert_eq!(answer.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(answer.into_body(), 1 << 20)
        .await
        .expect("a body");
    let calls: ambre_protocol::signaling::ChannelCalls =
        serde_json::from_slice(&body).expect("the calls");
    calls.calls
}

/// `CALL-162`: an ask of the live calls names the call of each channel of the asker, and of
/// no other channel. A channel with no call is absent, and a stranger to every channel reads
/// none.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_member_reads_the_live_call_of_its_own_channels_alone(pool: sqlx::PgPool) {
    use ambre_protocol::signaling::ChannelCall;
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let carol = Member::new(&pool, "carol", 3).await;
    let dave = Member::new(&pool, "dave", 4).await;
    let with_carol = channel_of(&pool, &alice, &[&carol]).await;
    let with_bob = channel_of(&pool, &alice, &[&bob]).await;
    let quiet = channel_of(&pool, &alice, &[&bob, &carol]).await;
    let ttl = state.config.call_state_ttl;
    let (first, second) = (Id::<Call>::mint(), Id::<Call>::mint());
    for (channel, call) in [(with_carol, first), (with_bob, second)] {
        let claimed = redis_ops::claim_channel_call(&state.redis, &channel, &call, None, ttl).await;
        assert!(matches!(claimed, Ok(redis_ops::ChannelCall::Taken)));
    }
    assert_eq!(
        ask_channel_calls(&state, &carol).await,
        [ChannelCall {
            channel: with_carol,
            call: first
        }]
    );
    assert_eq!(
        ask_channel_calls(&state, &bob).await,
        [ChannelCall {
            channel: with_bob,
            call: second
        }]
    );
    let mut both = ask_channel_calls(&state, &alice).await;
    ambre_core::order::sort_by_id(&mut both, |c| c.call);
    let mut want = vec![
        ChannelCall {
            channel: with_carol,
            call: first,
        },
        ChannelCall {
            channel: with_bob,
            call: second,
        },
    ];
    ambre_core::order::sort_by_id(&mut want, |c| c.call);
    assert_eq!(both, want, "the quiet channel is absent");
    assert!(
        ask_channel_calls(&state, &dave).await.is_empty(),
        "a stranger reads none"
    );
}

/// `ZK-056`: an ask of the live calls writes nothing to the store. The key of the live call of
/// the channel reads the same before and after the asks, and its time to live shrinks under
/// the asks alone.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn an_ask_of_the_live_calls_changes_nothing_of_the_store(pool: sqlx::PgPool) {
    use rustis::commands::{GenericCommands, StringCommands};
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let carol = Member::new(&pool, "carol", 3).await;
    let channel = channel_of(&pool, &alice, &[&carol]).await;
    let call = Id::<Call>::mint();
    let ttl = state.config.call_state_ttl;
    let claimed = redis_ops::claim_channel_call(&state.redis, &channel, &call, None, ttl).await;
    assert!(matches!(claimed, Ok(redis_ops::ChannelCall::Taken)));
    let key = crate::redis_keys::ChannelCallKey::of(&channel);
    let mut before: Vec<String> = state
        .redis
        .keys(format!("*{channel}*"))
        .await
        .expect("keys");
    before.sort();
    let lived: i64 = state.redis.pttl(key.as_str()).await.expect("a ttl");
    for _ in 0..3 {
        assert_eq!(ask_channel_calls(&state, &carol).await.len(), 1);
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let mut after: Vec<String> = state
        .redis
        .keys(format!("*{channel}*"))
        .await
        .expect("keys");
    after.sort();
    let held: Option<String> = state.redis.get(key.as_str()).await.expect("the key");
    let lives: i64 = state.redis.pttl(key.as_str()).await.expect("a ttl");
    assert_eq!(before, after, "an ask added or dropped a key");
    assert_eq!(held, Some(call.to_string()), "an ask changed the key");
    assert!(
        lives <= lived - 1000,
        "an ask refreshed the key: {lived} ms then {lives} ms"
    );
}
