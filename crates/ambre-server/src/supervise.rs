// ============================================================================
// What keeps one panic from ending more than the work it hit (`SRV-101`).
//
// A request runs on its connection's task, and a socket on its own task. The
// fan-out hub, the flusher and the compaction run as one task each for the
// whole process. A panic unwinds the task it happens on. For a request or a
// socket that is the right blast radius, once the socket deregisters and the
// request is answered.
//
// For a process-wide task it is every client at once. Each of those runs its work one unit at a time and survives a unit that
// panics.
// ============================================================================

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;

use axum::response::{IntoResponse, Response};
use futures_util::FutureExt;
use tower_http::catch_panic::CatchPanicLayer;

use crate::error::AppError;

/// Runs `work` and turns a panic inside it into `None`.
///
/// `task` names the work in the log line. The panic's own message is never
/// logged here: it may quote a value the relay handles, and a log line carries
/// no payload (`ZK-006`). Asserting unwind safety is sound because every
/// caller either drops the state `work` borrowed or rebuilds what it serves
/// from it.
pub async fn contained<F: Future>(task: &'static str, work: F) -> Option<F::Output> {
    match AssertUnwindSafe(work).catch_unwind().await {
        Ok(output) => Some(output),
        Err(_) => {
            tracing::error!(
                task,
                "a panic ended one unit of work; the relay keeps serving"
            );
            None
        }
    }
}

/// Replaces the panic hook of the standard library with one that logs where
/// a panic happened and never what it said (`SRV-297`).
///
/// The default hook writes the message to stderr before [`contained`] or
/// [`catch_panics`] can catch the unwind. A message may quote a value the
/// relay handles, so it reached the log stream that `ZK-006` keeps clean.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| log_panic(info.location())));
}

/// Logs a panic at `location`. It is handed the location alone, so no
/// message and no payload can reach the line.
fn log_panic(location: Option<&std::panic::Location<'_>>) {
    match location {
        Some(at) => tracing::error!(file = at.file(), line = at.line(), "a panic"),
        None => tracing::error!("a panic at an unknown place"),
    }
}

/// The answer to a request whose handler panicked: the generic `500` of any
/// infrastructure failure, so the caller learns nothing of the cause
/// (`SRV-096`).
fn answer_a_panic(_: Box<dyn Any + Send + 'static>) -> Response {
    AppError::Internal("a request handler panicked".into()).into_response()
}

/// The layer that answers a panicking request instead of dropping its
/// connection (`SRV-101`).
pub fn catch_panics() -> CatchPanicLayer<fn(Box<dyn Any + Send + 'static>) -> Response> {
    CatchPanicLayer::custom(answer_a_panic as fn(Box<dyn Any + Send + 'static>) -> Response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    /// `SRV-101`: a unit of work that panics returns `None`, and the next unit
    /// runs. The flusher and the compaction loop over exactly this.
    #[tokio::test]
    async fn a_panicking_unit_of_work_leaves_the_next_one_running() {
        let hit = contained("test", async { panic!("forced by a test") }).await;
        assert_eq!(hit, None::<()>);
        assert_eq!(contained("test", async { 7 }).await, Some(7));
    }

    /// A writer that keeps every byte a test subscriber writes.
    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// `SRV-297`: the line of a panic names its file and its line, and the
    /// hook hands it nothing else, so no message can reach it.
    #[test]
    fn a_panic_logs_its_place_alone() {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let here = std::panic::Location::caller();
        tracing::subscriber::with_default(subscriber, || log_panic(Some(here)));
        let line = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(line.contains("a panic"), "{line}");
        assert!(line.contains(here.file()), "{line}");
        assert!(line.contains(&here.line().to_string()), "{line}");
    }

    /// A handler that panics with a cause the answer must not carry.
    async fn boom() -> &'static str {
        panic!("secret cause")
    }

    /// `SRV-101`: a handler that panics answers the generic `500`,
    /// and the router keeps answering the next request.
    #[tokio::test]
    async fn a_panicking_handler_answers_a_generic_error_and_the_router_keeps_serving() {
        let app: Router = Router::new()
            .route("/boom", get(boom))
            .route("/ok", get(|| async { "fine" }))
            .layer(catch_panics());

        let hit = app
            .clone()
            .oneshot(Request::get("/boom").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(hit.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(hit.into_body(), 1024).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("Internal server error"), "{text}");
        assert!(!text.contains("secret cause"), "{text}");

        let next = app
            .oneshot(Request::get("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(next.status(), StatusCode::OK);
    }
}
