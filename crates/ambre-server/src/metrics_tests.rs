use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;
use tower_http::cors::CorsLayer;

use super::{Delta, Failure, RELAY, ROUTE, Socket, router};
use ambre_metrics::{Label, ScrapeAccess};

/// The spellings of every value of `L`.
fn spellings<L: Label>() -> Vec<&'static str> {
    L::ALL.iter().map(|l| l.value().as_str()).collect()
}

/// The body of a `GET` of `path` on `app`, with its status and media type.
async fn get(app: axum::Router, path: &str) -> (StatusCode, String, String) {
    let answer = app
        .oneshot(Request::get(path).body(Body::empty()).expect("a request"))
        .await
        .expect("an answer");
    let status = answer.status();
    let kind = answer
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = axum::body::to_bytes(answer.into_body(), 1 << 20)
        .await
        .expect("a body");
    (
        status,
        kind,
        String::from_utf8(body.to_vec()).expect("text"),
    )
}

/// `SRV-257`, `ZK-029`, `ZK-031`: the scrape answers the Prometheus text format. Each label
/// value is the spelling of a variant of its closed `enum`. So no series names a document, a
/// user, a call, a connection or an address. The count of series is the one of the start,
/// whatever the relay served.
#[tokio::test]
async fn the_scrape_answers_the_text_format_under_closed_labels_alone() {
    RELAY.deltas.inc(Delta::Sequenced);
    RELAY.failures.inc(Failure::Broadcast);
    let (status, kind, body) = get(router(ScrapeAccess::Loopback), ROUTE).await;
    assert_eq!(status, StatusCode::OK);
    assert!(kind.starts_with("text/plain; version=0.0.4"), "{kind}");
    let closed: [(&str, &str, Vec<&str>); 4] = [
        ("ambre_relay_sockets", "socket", spellings::<Socket>()),
        ("ambre_relay_deltas_total", "outcome", spellings::<Delta>()),
        (
            "ambre_relay_failures_total",
            "failure",
            spellings::<Failure>(),
        ),
        (
            "ambre_relay_sequencing_seconds_bucket",
            "le",
            super::SEQUENCING_BOUNDS
                .iter()
                .map(|(_, t)| t.as_str())
                .chain(std::iter::once("+Inf"))
                .collect(),
        ),
    ];
    let samples: Vec<&str> = body.lines().filter(|l| !l.starts_with('#')).collect();
    for sample in &samples {
        let (series, _) = sample.rsplit_once(' ').expect("a series and a value");
        match series.split_once('{') {
            None => assert!(
                [
                    "ambre_relay_sequencing_seconds_sum",
                    "ambre_relay_sequencing_seconds_count"
                ]
                .contains(&series),
                "an unlabelled series: {series}"
            ),
            Some((name, labels)) => {
                let (key, value) = labels
                    .strip_suffix('}')
                    .and_then(|l| l.split_once('='))
                    .expect("one label");
                let value = value.trim_matches('"');
                let known = closed
                    .iter()
                    .any(|(n, k, values)| *n == name && *k == key && values.contains(&value));
                assert!(known, "a label no closed set declares: {sample}");
            }
        }
    }
    let series: usize = closed.iter().map(|(_, _, values)| values.len()).sum();
    assert_eq!(
        samples.len(),
        series + 2,
        "the series are the ones of the start"
    );
}

/// `SRV-257`: the public listener serves no metric: neither at the root nor under the API.
#[sqlx::test]
#[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
async fn the_public_listener_serves_no_metric(pool: sqlx::PgPool) {
    let state = crate::test_helpers::test_app_state(pool).await;
    let app = crate::public_router(state, CorsLayer::new());
    for path in [ROUTE, "/api/metrics"] {
        let (status, _, body) = get(app.clone(), path).await;
        assert_ne!(status, StatusCode::OK, "{path}");
        assert!(!body.contains("ambre_relay_"), "{path}: {body}");
    }
}

/// The status of a scrape of `app` that presents `authorization`.
async fn status_with(app: axum::Router, authorization: Option<&str>) -> StatusCode {
    let mut request = Request::get(ROUTE);
    if let Some(value) = authorization {
        request = request.header(header::AUTHORIZATION, value);
    }
    app.oneshot(request.body(Body::empty()).expect("a request"))
        .await
        .expect("an answer")
        .status()
}

/// `SRV-307`: a listener off the loopback answers the scrape that presents
/// the operator's token, and `401` with no metric to any other.
#[tokio::test]
async fn a_scrape_off_the_loopback_presents_the_token() {
    let shared: std::net::SocketAddr = "10.0.0.7:9464".parse().expect("an address");
    assert!(ScrapeAccess::for_listener(shared, None).is_err());
    let token = ambre_core::Credential::new("north-wind".to_owned());
    let access = ScrapeAccess::for_listener(shared, Some(token)).expect("an access");
    assert_eq!(
        status_with(router(access.clone()), Some("Bearer north-wind")).await,
        StatusCode::OK
    );
    for refused in [None, Some("Bearer south-wind"), Some("north-wind")] {
        let answer = router(access.clone())
            .oneshot({
                let mut request = Request::get(ROUTE);
                if let Some(value) = refused {
                    request = request.header(header::AUTHORIZATION, value);
                }
                request.body(Body::empty()).expect("a request")
            })
            .await
            .expect("an answer");
        assert_eq!(answer.status(), StatusCode::UNAUTHORIZED, "{refused:?}");
        let body = axum::body::to_bytes(answer.into_body(), 1 << 20)
            .await
            .expect("a body");
        assert!(!String::from_utf8_lossy(&body).contains("ambre_relay"));
    }
}
