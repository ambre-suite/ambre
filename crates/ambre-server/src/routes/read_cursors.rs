// ambre-server/src/routes/read_cursors.rs
// The read cursors and the activity answer (`ambre-spec-server.md section
// 3`). The relay compares numbers it already routes on. It answers yes or no
// per document, never a count (`SRV-110`) and never the kind of an update,
// which stays sealed.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::sync::Arc;

use ambre_core::id::{Channel, Document, Id, User};

use crate::auth;
use crate::db;
use crate::error::{AppError, Resource};
use crate::extract::{Json, PathId};
use crate::state::AppState;

/// The body of a cursor move: the sequence number the caller has read up to.
#[derive(serde::Deserialize)]
pub struct ReadCursorRequest {
    pub seq: ambre_core::Seq,
}

/// One document of the activity answer.
#[derive(serde::Serialize)]
pub struct ActivityJson {
    #[serde(with = "ambre_core::serde_id::text")]
    pub channel_id: Id<Channel>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub document_id: Id<Document>,
    /// A segment or a thread, which the relay already routes on.
    #[serde(with = "ambre_core::serde_enums::channel_document_kind")]
    pub kind: ambre_core::ChannelDocumentKind,
    /// The document's last number is past the caller's cursor.
    pub active: ambre_core::activity::Activity,
    /// The caller's own cursor. The client counts its new messages against
    /// it; the relay still states no count (`SRV-110`).
    pub read: ambre_core::Seq,
    /// The document's last number. A client that holds nothing of the
    /// document resumes after it (`SRV-222`); it counts nothing with it.
    pub last: ambre_core::Seq,
}

/// `PUT /api/documents/{id}/read-cursor`: move the caller's cursor on one
/// document, forward or back. Any member may, a viewer included: reading is
/// what a viewer does.
pub async fn set_read_cursor(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    PathId(doc_id): PathId<Id<Document>>,
    Json(req): Json<ReadCursorRequest>,
) -> Result<impl IntoResponse, AppError> {
    let not_found = || AppError::NotFound {
        resource: Resource::Document,
    };
    db::get_document_member(&state.db, &doc_id, &auth.user_id)
        .await?
        .ok_or_else(not_found)?;
    // The register holds a signed 64-bit number: a cursor past it is refused,
    // never clamped.
    if i64::try_from(req.seq.0).is_err() {
        return Err(AppError::Conflict {
            detail: "sequence number out of range".into(),
        });
    }
    db::set_read_cursor(&state.db, &auth.user_id, &doc_id, req.seq).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/activity`: per segment and thread of every channel of the
/// caller, whether its last number is past the caller's cursor (`SRV-109`).
pub async fn get_activity(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
) -> Result<impl IntoResponse, AppError> {
    let json: Vec<ActivityJson> = last_numbers(&state, &auth.user_id)
        .await?
        .into_iter()
        .map(|(row, last)| {
            let channel_id =
                Id::<Channel>::from_uuid(ambre_core::Uuid::from_bytes(*row.channel_id.as_bytes()));
            let document_id = Id::<Document>::from_uuid(ambre_core::Uuid::from_bytes(
                *row.document_id.as_bytes(),
            ));
            ActivityJson {
                channel_id,
                document_id,
                kind: row.kind,
                active: ambre_core::activity::Activity::from_bool(last > row.cursor),
                read: row.cursor,
                last,
            }
        })
        .collect();
    Ok(super::DocumentRows::json(json))
}

/// The body of a move to the last number: the documents to move. When absent,
/// every segment and thread of every channel of the caller moves.
#[derive(serde::Deserialize, Default)]
pub struct MarkReadRequest {
    #[serde(default, with = "ambre_core::serde_id::text::option_vec")]
    pub documents: Option<Vec<Id<Document>>>,
}

/// `POST /api/read-cursors/mark-read`: move the caller's cursors to the last
/// number of each document. With no list, every segment and thread of every
/// channel of the caller (the Murmure unread spec). Only the caller's cursors move
/// (the Murmure unread spec), and a document outside the caller's channels is ignored.
pub async fn mark_read(
    State(state): State<Arc<AppState>>,
    auth: auth::AuthSession,
    body: Option<Json<MarkReadRequest>>,
) -> Result<impl IntoResponse, AppError> {
    let wanted: Option<std::collections::HashSet<Id<Document>>> = body
        .and_then(|Json(req)| req.documents)
        .map(|ids| ids.into_iter().collect());
    let cursors: Vec<(Id<Document>, ambre_core::Seq)> = last_numbers(&state, &auth.user_id)
        .await?
        .into_iter()
        .filter(|(row, last)| *last > row.cursor)
        .filter(|(row, _)| wanted.as_ref().is_none_or(|w| w.contains(&row.document_id)))
        .map(|(row, last)| (row.document_id, last))
        .collect();
    db::set_read_cursors(&state.db, &auth.user_id, &cursors).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Each channel document of the caller with its last number, the higher of
/// the register's and the ephemeral store's. A delta sequenced and not yet
/// flushed lives in the second alone.
async fn last_numbers(
    state: &AppState,
    user_id: &Id<User>,
) -> Result<Vec<(db::ActivityRow, ambre_core::Seq)>, AppError> {
    let rows = db::channel_activity(&state.db, user_id).await?;
    let ids: Vec<Id<Document>> = rows
        .iter()
        .map(|r| Id::<Document>::from_uuid(ambre_core::Uuid::from_bytes(*r.document_id.as_bytes())))
        .collect();
    let counters = crate::redis_ops::document_seqs(&state.redis, &ids).await?;
    Ok(rows
        .into_iter()
        .zip(counters)
        .map(|(row, counter)| {
            let last = row.durable_last.max(counter.unwrap_or_default());
            (row, last)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::PublicKey;
    use rustis::commands::StringCommands;

    fn uuid_of<E: ambre_core::id::Entity<Repr = ambre_core::Uuid>>(id: Id<E>) -> uuid::Uuid {
        uuid::Uuid::from_bytes(*id.uuid().as_bytes())
    }

    fn session(user_id: Id<User>, key: u8) -> auth::AuthSession {
        auth::AuthSession {
            user_id,
            signing_pubkey: PublicKey::from_array([key; 32]),
        }
    }

    /// A channel of `owner`: its `meta` and `segments` segments, each with a
    /// sequence counter in the ephemeral store. `members` are granted on
    /// `meta`, which grants the whole channel.
    async fn channel(
        state: &AppState,
        owner: Id<User>,
        members: &[Id<User>],
        counters: &[u64],
    ) -> (Id<Channel>, Id<Document>, Vec<Id<Document>>) {
        let pool = &state.db;
        let channel = Id::<Channel>::mint();
        sqlx::query("INSERT INTO channels (id) VALUES ($1)")
            .bind(uuid_of(channel))
            .execute(pool)
            .await
            .expect("channel");
        let meta = crate::test_helpers::create_test_document(pool, owner, "murmure").await;
        let mut documents = vec![(meta, ambre_core::ChannelDocumentKind::Meta, None)];
        let mut segments = Vec::new();
        for (index, &counter) in counters.iter().enumerate() {
            let segment = crate::test_helpers::create_test_document(pool, owner, "murmure").await;
            state
                .redis
                .set(format!("doc:{segment}:seq"), counter)
                .await
                .expect("counter");
            // One active segment per channel, the older ones final.
            let segment_state = if index == 0 { "active" } else { "final" };
            documents.push((
                segment,
                ambre_core::ChannelDocumentKind::Segment,
                Some(segment_state),
            ));
            segments.push(segment);
        }
        for (document, kind, segment_state) in documents {
            sqlx::query(
                "INSERT INTO channel_documents (document_id, channel_id, kind, state) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(uuid_of(document))
            .bind(uuid_of(channel))
            .bind(kind.as_str())
            .bind(segment_state)
            .execute(pool)
            .await
            .expect("group");
        }
        for member in members {
            db::add_document_member(
                pool,
                &meta,
                member,
                ambre_core::Role::Viewer,
                &ambre_core::WrappedDocKey::new([6u8; 32].to_vec()),
                &owner,
                None,
            )
            .await
            .expect("grant");
        }
        (channel, meta, segments)
    }

    async fn put_cursor(
        state: &Arc<AppState>,
        who: auth::AuthSession,
        document: Id<Document>,
        seq: u64,
    ) -> StatusCode {
        match set_read_cursor(
            State(state.clone()),
            who,
            PathId(document),
            Json(ReadCursorRequest {
                seq: ambre_core::Seq(seq),
            }),
        )
        .await
        {
            Ok(r) => r.into_response().status(),
            Err(e) => e.into_response().status(),
        }
    }

    /// The activity answer, as `document -> active`, with every entry's keys.
    async fn activity(
        state: &Arc<AppState>,
        who: auth::AuthSession,
    ) -> (Vec<(String, bool)>, Vec<Vec<String>>) {
        let (answer, keys, _) = activity_kinds(state, who).await;
        (answer, keys)
    }

    /// The activity answer, with every entry's keys and `document -> kind`.
    async fn activity_kinds(
        state: &Arc<AppState>,
        who: auth::AuthSession,
    ) -> (Vec<(String, bool)>, Vec<Vec<String>>, Vec<(String, String)>) {
        let resp = get_activity(State(state.clone()), who)
            .await
            .expect("activity")
            .into_response();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let entries = json["documents"].as_array().expect("documents").clone();
        let answer = entries
            .iter()
            .map(|e| {
                (
                    e["document_id"].as_str().expect("id").to_owned(),
                    e["active"].as_bool().expect("active"),
                )
            })
            .collect();
        let keys = entries
            .iter()
            .map(|e| {
                let mut k: Vec<String> = e.as_object().expect("object").keys().cloned().collect();
                k.sort();
                k
            })
            .collect();
        let kinds = entries
            .iter()
            .map(|e| {
                (
                    e["document_id"].as_str().expect("id").to_owned(),
                    e["kind"].as_str().expect("kind").to_owned(),
                )
            })
            .collect();
        (answer, keys, kinds)
    }

    fn active_of(answer: &[(String, bool)], document: Id<Document>) -> Option<bool> {
        let id = document.to_string();
        answer.iter().find(|(d, _)| *d == id).map(|(_, a)| *a)
    }

    async fn cursor_of(pool: &sqlx::PgPool, user: Id<User>, document: Id<Document>) -> Option<i64> {
        sqlx::query_scalar("SELECT seq FROM read_cursors WHERE user_id = $1 AND document_id = $2")
            .bind(uuid_of(user))
            .bind(uuid_of(document))
            .fetch_optional(pool)
            .await
            .expect("cursor")
    }

    /// `SRV-109`: per segment, whether its last number is past the caller's
    /// cursor. The last number is the higher of the ephemeral counter and the
    /// register's horizon, and `meta` never appears. `SRV-110`: each entry
    /// carries its channel, its document and a yes or no, and no count.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_activity_answer_compares_each_last_number_with_the_callers_cursor(
        pool: sqlx::PgPool,
    ) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = crate::test_helpers::create_test_user(&state.db, "alice@test.dev").await;
        let (_, meta, segments) = channel(&state, alice, &[], &[5, 3, 0]).await;
        let (read, unread, flushed) = (segments[0], segments[1], segments[2]);
        // The third segment's deltas are flushed and folded: its counter was
        // lost with the ephemeral store, and its snapshot's horizon remains.
        sqlx::query(
            "INSERT INTO snapshots (document_id, author_pubkey, s3_key, covered_seq) \
             VALUES ($1, $2, 'k', 7)",
        )
        .bind(uuid_of(flushed))
        .bind([1u8; 32].as_slice())
        .execute(&state.db)
        .await
        .expect("snapshot");
        let me = || session(alice, 1);
        assert_eq!(
            put_cursor(&state, me(), read, 5).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            put_cursor(&state, me(), flushed, 2).await,
            StatusCode::NO_CONTENT
        );

        let (answer, keys) = activity(&state, me()).await;
        assert_eq!(active_of(&answer, read), Some(false), "read up to its last");
        let kinds = [
            "active",
            "channel_id",
            "document_id",
            "kind",
            "last",
            "read",
        ];
        assert_eq!(
            active_of(&answer, unread),
            Some(true),
            "no cursor, three deltas"
        );
        assert_eq!(
            active_of(&answer, flushed),
            Some(true),
            "the horizon counts"
        );
        assert_eq!(active_of(&answer, meta), None, "meta carries no message");
        assert!(
            keys.iter().all(|k| *k == kinds),
            "an entry carries a yes or no and two numbers, and no count: {keys:?}"
        );
        let reads = read_of(&state, me()).await;
        for (document, cursor) in [(read, 5), (unread, 0), (flushed, 2)] {
            assert_eq!(reads.get(&document.to_string()), Some(&cursor));
        }
    }

    /// `SRV-221`: each entry names its document's kind, a segment or a
    /// thread. A client that holds nothing of the channel counts a thread
    /// apart from its segments.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn each_activity_entry_names_its_kind(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = crate::test_helpers::create_test_user(&state.db, "alice@test.dev").await;
        let (channel, _, segments) = channel(&state, alice, &[], &[2]).await;
        let thread = crate::test_helpers::create_test_document(&state.db, alice, "murmure").await;
        sqlx::query(
            "INSERT INTO channel_documents (document_id, channel_id, kind, thread_root) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(uuid_of(thread))
        .bind(uuid_of(channel))
        .bind(ambre_core::ChannelDocumentKind::Thread.as_str())
        .bind(uuid::Uuid::new_v4())
        .execute(&state.db)
        .await
        .expect("thread");
        let (_, _, mut kinds) = activity_kinds(&state, session(alice, 1)).await;
        kinds.sort();
        let mut expected = vec![
            (segments[0].to_string(), "segment".to_owned()),
            (thread.to_string(), "thread".to_owned()),
        ];
        expected.sort();
        assert_eq!(kinds, expected);
    }

    /// The activity answer, as `document -> the caller's cursor`.
    async fn read_of(
        state: &Arc<AppState>,
        who: auth::AuthSession,
    ) -> std::collections::HashMap<String, u64> {
        number_of(state, who, "read").await
    }

    /// The activity answer, as `document -> number`, read under `field`.
    async fn number_of(
        state: &Arc<AppState>,
        who: auth::AuthSession,
        field: &str,
    ) -> std::collections::HashMap<String, u64> {
        let resp = get_activity(State(state.clone()), who)
            .await
            .expect("activity")
            .into_response();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        json["documents"]
            .as_array()
            .expect("documents")
            .iter()
            .map(|e| {
                (
                    e["document_id"].as_str().expect("id").to_owned(),
                    e[field].as_u64().expect("number"),
                )
            })
            .collect()
    }

    /// `SRV-222`: each entry carries its document's last number, the higher
    /// of the ephemeral counter and the register's horizon, whatever the
    /// caller's cursor.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn each_activity_entry_carries_its_last_number(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = crate::test_helpers::create_test_user(&state.db, "alice@test.dev").await;
        let (_, _, segments) = channel(&state, alice, &[], &[5, 3, 0]).await;
        sqlx::query(
            "INSERT INTO snapshots (document_id, author_pubkey, s3_key, covered_seq) \
             VALUES ($1, $2, 'k', 7)",
        )
        .bind(uuid_of(segments[2]))
        .bind([1u8; 32].as_slice())
        .execute(&state.db)
        .await
        .expect("snapshot");
        assert_eq!(
            put_cursor(&state, session(alice, 1), segments[0], 5).await,
            StatusCode::NO_CONTENT
        );
        let lasts = number_of(&state, session(alice, 1), "last").await;
        for (document, last) in [(segments[0], 5), (segments[1], 3), (segments[2], 7)] {
            assert_eq!(lasts.get(&document.to_string()), Some(&last));
        }
    }

    /// The cursor belongs to the user, so a second device of
    /// the user, signing under another key, reads the cursor the first moved.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_cursor_moved_on_one_device_is_read_on_every_other(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = crate::test_helpers::create_test_user(&state.db, "alice@test.dev").await;
        let (_, _, segments) = channel(&state, alice, &[], &[4]).await;
        let (phone, laptop) = (session(alice, 1), session(alice, 2));
        assert_eq!(
            active_of(&activity(&state, laptop).await.0, segments[0]),
            Some(true)
        );
        put_cursor(&state, phone, segments[0], 4).await;
        let laptop = session(alice, 2);
        assert_eq!(
            active_of(&activity(&state, laptop).await.0, segments[0]),
            Some(false)
        );
    }

    /// Marking everything read moves the caller's cursor on every segment of
    /// every channel to its last number. For it as for a single move, the
    /// other member's cursors stay where they were.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn marking_everything_read_moves_the_callers_cursors_alone(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = crate::test_helpers::create_test_user(&state.db, "alice@test.dev").await;
        let bob = crate::test_helpers::create_test_user(&state.db, "bob@test.dev").await;
        let (_, _, first) = channel(&state, alice, &[bob], &[5, 2]).await;
        let (_, _, second) = channel(&state, alice, &[bob], &[9]).await;
        put_cursor(&state, session(bob, 2), first[0], 1).await;

        put_cursor(&state, session(alice, 1), first[1], 1).await;
        assert_eq!(
            cursor_of(&state.db, bob, first[1]).await,
            None,
            "a single move leaves the other member alone"
        );

        let resp = mark_read(State(state.clone()), session(alice, 1), None)
            .await
            .expect("mark all read")
            .into_response();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        for (segment, last) in [(first[0], 5), (first[1], 2), (second[0], 9)] {
            assert_eq!(cursor_of(&state.db, alice, segment).await, Some(last));
        }
        let (answer, _) = activity(&state, session(alice, 1)).await;
        assert!(answer.iter().all(|(_, active)| !active), "{answer:?}");

        assert_eq!(
            cursor_of(&state.db, bob, first[0]).await,
            Some(1),
            "marking all read leaves the other member alone"
        );
        assert_eq!(cursor_of(&state.db, bob, second[0]).await, None);
        let (answer, _) = activity(&state, session(bob, 2)).await;
        assert!(answer.iter().all(|(_, active)| *active), "{answer:?}");
    }

    /// A list moves the documents it names alone, and a document outside the
    /// caller's channels is ignored rather than written.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_listed_move_touches_the_listed_documents_alone(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = crate::test_helpers::create_test_user(&state.db, "alice@test.dev").await;
        let eve = crate::test_helpers::create_test_user(&state.db, "eve@test.dev").await;
        let (_, _, segments) = channel(&state, alice, &[], &[5, 2]).await;
        let (_, _, strangers) = channel(&state, eve, &[], &[8]).await;
        let body = MarkReadRequest {
            documents: Some(vec![segments[0], strangers[0]]),
        };
        mark_read(State(state.clone()), session(alice, 1), Some(Json(body)))
            .await
            .expect("mark read");
        assert_eq!(cursor_of(&state.db, alice, segments[0]).await, Some(5));
        assert_eq!(cursor_of(&state.db, alice, segments[1]).await, None);
        assert_eq!(cursor_of(&state.db, alice, strangers[0]).await, None);
    }

    /// A cursor moves back as well as forward, which marking a message unread
    /// needs. A caller who is no member moves none, and gets the same 404 as
    /// an unknown document.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_cursor_moves_back_and_only_for_a_member(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = crate::test_helpers::create_test_user(&state.db, "alice@test.dev").await;
        let eve = crate::test_helpers::create_test_user(&state.db, "eve@test.dev").await;
        let (_, _, segments) = channel(&state, alice, &[], &[6]).await;
        put_cursor(&state, session(alice, 1), segments[0], 6).await;
        put_cursor(&state, session(alice, 1), segments[0], 3).await;
        assert_eq!(cursor_of(&state.db, alice, segments[0]).await, Some(3));

        let refused = put_cursor(&state, session(eve, 3), segments[0], 6).await;
        let unknown = put_cursor(&state, session(eve, 3), Id::<Document>::mint(), 6).await;
        assert_eq!(refused, StatusCode::NOT_FOUND);
        assert_eq!(unknown, refused);
        assert_eq!(cursor_of(&state.db, eve, segments[0]).await, None);
    }

    /// `SRV-137`: removing a member from a channel's `meta` deletes the
    /// member's cursor on every document of the channel, and no one else's.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_removal_deletes_the_read_cursors_of_the_member_removed(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool).await;
        let alice = crate::test_helpers::create_test_user(&state.db, "alice@test.dev").await;
        let bob = crate::test_helpers::create_test_user(&state.db, "bob@test.dev").await;
        let (_, meta, segments) = channel(&state, alice, &[bob], &[3, 4]).await;
        let (_, _, other) = channel(&state, alice, &[bob], &[2]).await;
        for &segment in segments.iter().chain(&other) {
            put_cursor(&state, session(bob, 2), segment, 1).await;
            put_cursor(&state, session(alice, 1), segment, 1).await;
        }

        db::remove_document_member(&state.db, &meta, &bob, None, ambre_core::UnixSeconds(0))
            .await
            .expect("remove");
        for &segment in &segments {
            assert_eq!(cursor_of(&state.db, bob, segment).await, None);
            assert_eq!(cursor_of(&state.db, alice, segment).await, Some(1));
        }
        assert_eq!(
            cursor_of(&state.db, bob, other[0]).await,
            Some(1),
            "another channel keeps its member"
        );
    }
}
