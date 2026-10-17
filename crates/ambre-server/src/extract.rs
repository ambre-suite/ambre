// ambre-server/src/extract.rs
// The JSON body every route reads, and the answer to one that does not decode.

//! The framework's own JSON extractor answers `422` to a body that does not
//! decode. This relay keeps `422` for a signature that does not verify, and
//! answers `409` to a payload that will not decode (`SRV-178`). Every route
//! therefore reads its body through [`Json`], never through `axum::Json`.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, OptionalFromRequest, Request};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use crate::error::AppError;

/// A request body read as JSON. A body that does not parse, or that names a
/// word outside a closed set, answers `409` (`SRV-178`). A missing content
/// type and a body that cannot be read keep the framework's answer.
pub struct Json<T>(pub T);

/// The answer to a body the framework refused. The caller learns that the
/// body does not decode, not where.
fn refused(rejection: JsonRejection) -> Response {
    match rejection {
        JsonRejection::JsonDataError(_) | JsonRejection::JsonSyntaxError(_) => AppError::Conflict {
            detail: "the request body does not decode".into(),
        }
        .into_response(),
        other => other.into_response(),
    }
}

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        <axum::Json<T> as FromRequest<S>>::from_request(req, state)
            .await
            .map(|axum::Json(body)| Json(body))
            .map_err(refused)
    }
}

/// A body the route may go without: none when the request carries no content
/// type, and the same answers as [`Json`] otherwise.
impl<T, S> OptionalFromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Option<Self>, Self::Rejection> {
        <axum::Json<T> as OptionalFromRequest<S>>::from_request(req, state)
            .await
            .map(|body| body.map(|axum::Json(body)| Json(body)))
            .map_err(refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// A closed set of one word, as a role or a document type is.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Word {
        Owner,
    }

    #[derive(serde::Deserialize)]
    struct Probe {
        #[allow(dead_code)]
        word: Word,
    }

    /// The status a route reading a [`Probe`] answers to this body.
    async fn answer(content_type: Option<&str>, body: &'static str) -> StatusCode {
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(|Json(_): Json<Probe>| async { StatusCode::OK }),
        );
        let mut req = Request::post("/");
        if let Some(ct) = content_type {
            req = req.header("content-type", ct);
        }
        let resp = app
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        resp.status()
    }

    #[tokio::test]
    async fn a_body_that_decodes_reaches_the_handler() {
        assert_eq!(
            answer(Some("application/json"), r#"{"word":"owner"}"#).await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn a_body_that_does_not_decode_answers_409() {
        let json = Some("application/json");
        assert_eq!(
            answer(json, r#"{"word":"superadmin"}"#).await,
            StatusCode::CONFLICT
        );
        assert_eq!(answer(json, r#"{"word":"#).await, StatusCode::CONFLICT);
        assert_eq!(answer(json, r#"{}"#).await, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn a_body_without_a_json_content_type_keeps_the_framework_answer() {
        assert_eq!(
            answer(None, r#"{"word":"owner"}"#).await,
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }
}
