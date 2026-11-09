//! The call socket, database tier: the admission, the cap, the forwarding of the wraps, and
//! what the relay keeps of a call.

use super::*;

use ambre_core::{Sec, TimeSpan};
use ambre_protocol::signaling::{PARTICIPANT_CAP, SenderKeyWraps};
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

/// The admission and the roster of an admitted answer.
fn admitted(answer: CallAnswer) -> (ambre_core::Credential<ambre_core::CallAdmission>, usize) {
    match answer {
        CallAnswer::Admitted {
            frame: CallServerMsg::Admitted { admission, roster },
            ..
        } => (admission, roster.len()),
        CallAnswer::Admitted { .. } => panic!("an admission frame"),
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
    let (first, before) = admitted(
        answer_join(&state, &call, alice.join(&state, channel))
            .await
            .expect("a join"),
    );
    let (second, roster) = admitted(
        answer_join(&state, &call, alice.join(&state, channel))
            .await
            .expect("a join"),
    );
    assert_eq!(
        (before, roster),
        (0, 1),
        "the second join reads the first in the roster"
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
    for _ in 1..PARTICIPANT_CAP.get() {
        let join = db::join_call(&pool, &call, &channel, &alice.key, &room).await;
        assert!(matches!(join, Ok(db::CallJoin::Admitted { .. })));
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
    let seats: i64 =
        sqlx::query_scalar("SELECT count(*) FROM call_participants WHERE call_id = $1")
            .bind(db::id_to_uuid(call))
            .fetch_one(&pool)
            .await
            .expect("a count");
    assert_eq!(seats, i64::from(PARTICIPANT_CAP.get()));
}

/// `SRV-049`, `ZK-022`: the call has an endpoint of its own. A join and a wrap travel on the call
/// socket, and the wrap reaches the session of its key alone. The socket of the document plane
/// refuses a call frame as it refuses any first frame but its own. The last leave ends the call.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn a_wrap_reaches_its_recipient_alone_on_the_call_socket(pool: sqlx::PgPool) {
    let state = test_app_state(pool.clone()).await;
    let alice = Member::new(&pool, "alice", 1).await;
    let bob = Member::new(&pool, "bob", 2).await;
    let carol = Member::new(&pool, "carol", 3).await;
    let channel = channel_of(&pool, &alice, &[&bob, &carol]).await;
    let call = Id::<Call>::mint();
    let root = served(state.clone()).await;

    let mut a = call_socket(&root, call, &alice.join(&state, channel)).await;
    assert!(matches!(
        next_signal(&mut a).await,
        Some(CallServerMsg::Admitted { .. })
    ));
    let mut b = call_socket(&root, call, &bob.join(&state, channel)).await;
    assert!(
        matches!(next_signal(&mut b).await, Some(CallServerMsg::Admitted { roster, .. }) if roster == vec![alice.key])
    );
    let mut c = call_socket(&root, call, &carol.join(&state, channel)).await;
    assert!(matches!(
        next_signal(&mut c).await,
        Some(CallServerMsg::Admitted { .. })
    ));

    let wrap = CallClientMsg::Wrap {
        to: bob.key,
        wrap: ambre_core::Payload::<SenderKeyWraps>::new(vec![7; 96]),
    };
    let frame = StoredPayload::seal(&wrap).expect("a frame encodes");
    a.send(WsMessage::Binary(frame.expose_bytes().to_vec().into()))
        .await
        .expect("a wrap");
    let heard = loop {
        match next_signal(&mut b).await {
            Some(CallServerMsg::Wrap { from, to, wrap }) => break (from, to, wrap.len()),
            Some(_) => continue,
            None => panic!("bob heard no wrap"),
        }
    };
    assert_eq!(heard, (alice.key, bob.key, ambre_core::Count::new(96)));
    // Carol hears the joins and no wrap: her socket ends silent after the joins.
    while let Some(signal) = next_signal(&mut c).await {
        assert!(
            !matches!(signal, CallServerMsg::Wrap { .. }),
            "carol heard a wrap"
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
        let ended: bool =
            sqlx::query_scalar("SELECT ended_at IS NOT NULL FROM calls WHERE id = $1")
                .bind(db::id_to_uuid(call))
                .fetch_one(&pool)
                .await
                .expect("the call");
        if ended {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the last leave ends the call"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `ZK-022`: the table of calls holds the channel, the room the relay drew, the start and the
/// end. The roster holds the key and the join instant of each session. Nothing else.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn the_call_tables_hold_only_what_the_relay_may_learn(pool: sqlx::PgPool) {
    let columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_name::text, column_name::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name LIKE 'call%' \
         ORDER BY table_name, column_name",
    )
    .fetch_all(&pool)
    .await
    .expect("query");
    let got: Vec<(&str, &str)> = columns
        .iter()
        .map(|(t, c)| (t.as_str(), c.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("call_participants", "call_id"),
            ("call_participants", "joined_at"),
            ("call_participants", "signing_pubkey"),
            ("calls", "channel_id"),
            ("calls", "ended_at"),
            ("calls", "id"),
            ("calls", "room"),
            ("calls", "started_at"),
        ]
    );
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
