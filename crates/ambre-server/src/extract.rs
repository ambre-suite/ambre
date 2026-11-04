// ambre-server/src/extract.rs
// The JSON body every route reads, the identifiers of its path, and the answer
// to either one that does not decode.

//! The framework's own JSON extractor answers `422` to a body that does not
//! decode. This relay keeps `422` for a signature that does not verify, and
//! answers `409` to a payload that will not decode (`SRV-178`). Every route
//! therefore reads its body through [`Json`], never through `axum::Json`.

use ambre_core::id::{Channel, Document, Entity, Id, Org, Segment, Snapshot, Space, User};
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, FromRequestParts, OptionalFromRequest, Path, Request};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use crate::error::{AppError, Resource};

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

/// One segment of a route path, read into its own type.
pub trait PathPart: Sized {
    /// What the framework reads the segment into.
    type Raw: DeserializeOwned + Send;

    /// The segment's value, or the resource a 404 names when it does not
    /// parse.
    fn part(raw: Self::Raw) -> Result<Self, Resource>;
}

/// An entity whose identifier a route path carries, with the resource a 404 names.
trait Routed: Entity<Repr = ambre_core::Uuid> {
    const RESOURCE: Resource;
}

impl Routed for Document {
    const RESOURCE: Resource = Resource::Document;
}
impl Routed for Snapshot {
    const RESOURCE: Resource = Resource::Snapshot;
}
impl Routed for User {
    const RESOURCE: Resource = Resource::User;
}
impl Routed for Org {
    const RESOURCE: Resource = Resource::Organization;
}
impl Routed for Channel {
    const RESOURCE: Resource = Resource::Channel;
}
impl Routed for Space {
    const RESOURCE: Resource = Resource::Space;
}
impl Routed for Segment {
    const RESOURCE: Resource = Resource::Segment;
}

/// A path identifier reads as its text, and names its resource when it does not parse.
impl<E: Routed> PathPart for Id<E> {
    type Raw = String;

    fn part(raw: String) -> Result<Self, Resource> {
        Self::parse_str(&raw).map_err(|_| E::RESOURCE)
    }
}

/// A content address reads as the framework reads it: it checks nothing.
impl PathPart for ambre_core::Validated<ambre_core::ContentHash> {
    type Raw = ambre_core::Validated<ambre_core::ContentHash>;

    fn part(raw: Self::Raw) -> Result<Self, Resource> {
        Ok(raw)
    }
}

impl<A: PathPart, B: PathPart> PathPart for (A, B) {
    type Raw = (A::Raw, B::Raw);

    fn part((a, b): Self::Raw) -> Result<Self, Resource> {
        Ok((A::part(a)?, B::part(b)?))
    }
}

/// The parts of a path, read in order. A path the framework refuses keeps its
/// answer, and a part that does not parse names its resource.
async fn read_path<T: PathPart, S: Send + Sync>(
    parts: &mut Parts,
    state: &S,
) -> Result<Result<T, Resource>, Response> {
    let Path(raw) = Path::<T::Raw>::from_request_parts(parts, state)
        .await
        .map_err(IntoResponse::into_response)?;
    Ok(T::part(raw))
}

/// The identifiers of a route path. One that does not parse answers the 404
/// of its resource, as an identifier nobody holds does.
pub struct PathId<T>(pub T);

impl<T: PathPart, S: Send + Sync> FromRequestParts<S> for PathId<T> {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        read_path(parts, state)
            .await?
            .map(PathId)
            .map_err(|resource| AppError::NotFound { resource }.into_response())
    }
}

/// The identifiers of a channel route path. One that does not parse answers
/// the 403 of an identifier the caller may not use, so the route tells nothing
/// apart.
pub struct OpaquePathId<T>(pub T);

impl<T: PathPart, S: Send + Sync> FromRequestParts<S> for OpaquePathId<T> {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        read_path(parts, state)
            .await?
            .map(OpaquePathId)
            .map_err(|_| AppError::Forbidden.into_response())
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
        #[expect(
            dead_code,
            reason = "the probe exists for the parse of its field alone"
        )]
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

    /// The status and the body of `resp`.
    async fn read(resp: Response) -> (StatusCode, Vec<u8>) {
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        (status, body.to_vec())
    }

    /// What a route answers to `uri`, its path read as a document and a user,
    /// or as a channel behind a 403.
    async fn path_answer(uri: &str) -> (StatusCode, Vec<u8>) {
        let app = axum::Router::new()
            .route(
                "/d/{doc}/u/{user}",
                axum::routing::get(|PathId(_): PathId<(Id<Document>, Id<User>)>| async {
                    StatusCode::OK
                }),
            )
            .route(
                "/c/{channel}",
                axum::routing::get(|OpaquePathId(_): OpaquePathId<Id<Channel>>| async {
                    StatusCode::OK
                }),
            );
        let resp = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        read(resp).await
    }

    /// A path identifier that does not parse answers as one nobody holds: the
    /// 404 of its own resource, or the 403 of a channel route.
    #[tokio::test]
    async fn a_path_identifier_that_does_not_parse_answers_its_refusal() {
        let id = "00000000-0000-0000-0000-000000000001";
        assert_eq!(
            path_answer(&format!("/d/{id}/u/{id}")).await.0,
            StatusCode::OK
        );
        let missing = |resource| read(AppError::NotFound { resource }.into_response());
        assert_eq!(
            path_answer(&format!("/d/nope/u/{id}")).await,
            missing(Resource::Document).await
        );
        assert_eq!(
            path_answer(&format!("/d/{id}/u/nope")).await,
            missing(Resource::User).await
        );
        assert_eq!(path_answer(&format!("/c/{id}")).await.0, StatusCode::OK);
        assert_eq!(
            path_answer("/c/nope").await,
            read(AppError::Forbidden.into_response()).await
        );
    }

    #[tokio::test]
    async fn a_body_without_a_json_content_type_keeps_the_framework_answer() {
        assert_eq!(
            answer(None, r#"{"word":"owner"}"#).await,
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }
}
