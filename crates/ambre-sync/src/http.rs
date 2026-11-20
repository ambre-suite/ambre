use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use ambre_core::id::{Channel, Document, Id, Lease, Message, Meta, Segment, Snapshot, Space, User};
use ambre_core::{
    ApiFailure, FixedRole, HttpStatus, LocalFailure, Payload, RefusalCode, Relay, Seq,
    UnreadableCause,
};
use serde::{Deserialize, Serialize};

/// The two session tokens, each wiped on drop and redacted in `Debug`
/// (`SYNC-100`, `SYNC-061`). Their roles keep them apart: only the access
/// token presents itself as a bearer.
pub(crate) use ambre_core::{AccessToken, Credential, RefreshToken};
use ambre_core::{Ms, Sec, TimeSpan};

/// The session's two tokens, with the lifetime the relay announced for the
/// access token. The worker renews the pair ahead of that lifetime
/// (`SYNC-183`).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TokenPair {
    access_token: Credential<AccessToken>,
    refresh_token: Credential<RefreshToken>,
    access_expires_in: TimeSpan<Sec>,
}

impl TokenPair {
    /// The pair of `access_token` and `refresh_token`, the access token
    /// living `lifetime` from the instant the worker stores it. The relay's
    /// answer builds every pair the worker holds, so a test alone builds one
    /// by hand.
    #[cfg(test)]
    pub(crate) fn new(
        access_token: Credential<AccessToken>,
        refresh_token: Credential<RefreshToken>,
        lifetime: TimeSpan<Sec>,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            access_expires_in: lifetime,
        }
    }

    /// The token a request presents as its bearer.
    pub(crate) fn access_token(&self) -> &Credential<AccessToken> {
        &self.access_token
    }

    /// The token a renewal and a sign-out send.
    pub(crate) fn refresh_token(&self) -> &Credential<RefreshToken> {
        &self.refresh_token
    }

    /// How long the access token lives, as the relay announced it.
    pub(crate) fn lifetime(&self) -> TimeSpan<Sec> {
        self.access_expires_in
    }
}

pub(crate) type TokenStore = Rc<RefCell<Option<TokenPair>>>;

/// The marker of one kind of relay address. Sealed: every kind lives in
/// this module.
pub trait UrlKind: sealed::Sealed + 'static {
    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames>;
}

mod sealed {
    /// Closes [`super::UrlKind`].
    pub trait Sealed {}
}

/// One address of the kind `K`: the text a fetch, a socket or a log line
/// reads. Each kind has its own constructor, so no text a user typed becomes
/// a route.
pub struct Url<K: UrlKind> {
    text: String,
    kind: PhantomData<fn() -> K>,
}

impl<K: UrlKind> Url<K> {
    /// The address `text`, for this module alone.
    const fn at(text: String) -> Self {
        Self {
            text,
            kind: PhantomData,
        }
    }

    /// The address, as a fetch or the browser's socket constructor takes it.
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone or compare.

impl<K: UrlKind> Clone for Url<K> {
    fn clone(&self) -> Self {
        Self::at(self.text.clone())
    }
}

impl<K: UrlKind> PartialEq for Url<K> {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl<K: UrlKind> Eq for Url<K> {}

/// `NAME("...")`, as the derived `Debug` of the hand-written type printed
/// it.
impl<K: UrlKind> std::fmt::Debug for Url<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple(K::NAME.as_str()).field(&self.text).finish()
    }
}

impl<K: UrlKind> std::fmt::Display for Url<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// The relay as the page names it: a host and an optional port, such as
/// `relay.example` or `localhost:3000`. Every URL of the relay is built from
/// it, through [`Url<ApiRoot>`] or the socket address.
pub enum RelayHost {}

impl sealed::Sealed for RelayHost {}
impl UrlKind for RelayHost {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("RelayHost");
}

impl Url<RelayHost> {
    /// The host the page configured.
    pub fn new(host: String) -> Self {
        Self::at(host)
    }

    /// Whether the relay runs on this machine, which speaks plain HTTP.
    pub(crate) fn is_local(&self) -> bool {
        self.text.starts_with("localhost") || self.text.starts_with("127.0.0.1")
    }
}

/// The scheme of every relay address: plain HTTP for a relay on this
/// machine, TLS for any other. The socket follows the same choice, so a
/// local dev relay never sees a TLS handshake it cannot answer. The worker
/// takes the scheme of its page first (`SYNC-271`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UrlScheme {
    Http,
    Https,
}

impl UrlScheme {
    /// The scheme `host` speaks: `Http` on this machine, `Https` elsewhere.
    pub(crate) fn of(host: &Url<RelayHost>) -> Self {
        if host.is_local() {
            Self::Http
        } else {
            Self::Https
        }
    }

    /// The scheme of the page that started the worker, when its protocol
    /// names one. The relay sits at the origin of that page, so a page served
    /// over TLS reaches the relay over TLS, even on this machine (`SYNC-271`).
    /// A page with no scheme falls back to the rule of [`UrlScheme::of`].
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "only the wasm worker start and the tests read it")
    )]
    pub(crate) fn of_page(page: Option<Self>, host: &Url<RelayHost>) -> Self {
        page.unwrap_or_else(|| Self::of(host))
    }
}

impl std::fmt::Display for UrlScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Http => "http",
            Self::Https => "https",
        })
    }
}

/// The root of the relay's REST routes, `{scheme}://{host}/api`. Built in
/// one place, so no route adds or forgets the `/api` itself.
pub(crate) enum ApiRoot {}

impl sealed::Sealed for ApiRoot {}
impl UrlKind for ApiRoot {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ApiRoot");
}

impl Url<ApiRoot> {
    /// The root of `host`'s routes under `scheme`.
    pub(crate) fn of(scheme: UrlScheme, host: &Url<RelayHost>) -> Self {
        Self::at(format!("{scheme}://{host}/api"))
    }

    /// The URL of the route at `path` under this root.
    pub(crate) fn route(&self, path: impl Into<Url<RoutePath>>) -> Url<Route> {
        Url::at(format!("{}{}", self.text, path.into().text))
    }
}

/// The path of a relay route under the API root, starting with a slash, as
/// `/documents` does. A call site writes the path as a literal or a
/// `format!`, and the route reads it from this type alone.
pub(crate) enum RoutePath {}

impl sealed::Sealed for RoutePath {}
impl UrlKind for RoutePath {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("RoutePath");
}

impl From<&str> for Url<RoutePath> {
    fn from(path: &str) -> Self {
        Self::at(path.to_owned())
    }
}

impl From<String> for Url<RoutePath> {
    fn from(path: String) -> Self {
        Self::at(path)
    }
}

/// The address of the relay's socket, `{ws|wss}://{host}/api/ws`. It carries
/// no credential: the token travels in the socket's first frame, so no log
/// line, proxy record or URL holds it (`SYNC-061`, `ambre-spec-session.md`,
/// `SRV-031`).
pub(crate) enum Socket {}

impl sealed::Sealed for Socket {}
impl UrlKind for Socket {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SocketUrl");
}

impl Url<Socket> {
    /// The socket of `host`, under the TLS choice `scheme` makes for its
    /// routes.
    #[cfg_attr(
        not(any(test, target_arch = "wasm32")),
        expect(
            dead_code,
            reason = "only the wasm socket and a test build the socket address"
        )
    )]
    pub(crate) fn of(scheme: UrlScheme, host: &Url<RelayHost>) -> Self {
        let socket = match scheme {
            UrlScheme::Http => "ws",
            UrlScheme::Https => "wss",
        };
        Self::at(format!("{socket}://{host}/api/ws"))
    }
}

/// The address of the call socket of one call on the relay,
/// `{ws|wss}://{host}/api/calls/{call}/socket`, apart from the socket of the document plane
/// (`SRV-049`). It carries no credential: the token travels in its first frame (`SRV-031`).
pub(crate) enum CallSocket {}

impl sealed::Sealed for CallSocket {}
impl UrlKind for CallSocket {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CallSocketUrl");
}

impl Url<CallSocket> {
    /// The call socket of `call` on `host`, under the TLS choice `scheme` makes for its
    /// routes.
    #[cfg_attr(
        not(any(test, target_arch = "wasm32")),
        expect(
            dead_code,
            reason = "only the wasm socket and a test build the address"
        )
    )]
    pub(crate) fn of(
        scheme: UrlScheme,
        host: &Url<RelayHost>,
        call: ambre_core::id::Id<ambre_core::id::Call>,
    ) -> Self {
        let socket = match scheme {
            UrlScheme::Http => "ws",
            UrlScheme::Https => "wss",
        };
        Self::at(format!("{socket}://{host}/api/calls/{call}/socket"))
    }
}

/// The URL of one of the relay's REST routes. Only [`Url::route`] builds
/// one, and every helper that attaches the access token takes one. A
/// presigned object-storage link is not one, so the token never reaches the
/// storage host.
pub(crate) enum Route {}

impl sealed::Sealed for Route {}
impl UrlKind for Route {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("RelayUrl");
}

impl Url<Route> {
    /// The route at `path` below this one, such as `/media` below a
    /// document's route.
    pub(crate) fn join(&self, path: impl Into<Url<RoutePath>>) -> Self {
        Self::at(format!("{}{}", self.text, path.into().text))
    }

    /// This route with the one query parameter `key` set to `value`. The
    /// value is percent-encoded whole, so no character of it ends the value,
    /// opens another parameter or a fragment, or decodes as another character
    /// (`SYNC-179`).
    pub(crate) fn query(&self, key: QueryKey, value: &str) -> Self {
        let value = percent_encoding::utf8_percent_encode(value, QUERY_VALUE);
        Self::at(format!("{}?{}={value}", self.text, key.as_str()))
    }
}

/// The query parameters a relay route reads. A closed set, so a key never
/// comes from text a user typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryKey {
    /// The search text of `/users/search`.
    Query,
}

impl QueryKey {
    fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
        }
    }
}

/// Every byte but the unreserved characters of RFC 3986 is encoded in a
/// query value.
const QUERY_VALUE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Where a request goes: one of the relay's routes, or an object a presigned
/// link names. Only a relay route ever carries the access token. A presigned
/// link is a credential of its own, so its text never reaches a log: the
/// target reads as the link without its query.
///
/// It derives no `PartialEq`: a link compares in constant time only, and a
/// test compares the [`ShownTarget`] or the text it opens.
#[derive(Debug, Clone)]
pub(crate) enum Target {
    /// A route of the relay.
    Relay(Url<Route>),
    /// An object in storage, through its presigned link.
    Object(ambre_core::Credential<ambre_core::PresignedUrl>),
}

impl Target {
    /// Lends the full address to `send`, the one call that sends the
    /// request. A presigned link is lent from its wrapper, and no copy of it
    /// is kept here.
    fn open<R>(&self, send: impl FnOnce(&str) -> R) -> R {
        match self {
            Self::Relay(url) => send(url.as_str()),
            Self::Object(link) => link.present_link(send),
        }
    }

    /// The target as a log and an error show it, with no credential in it.
    pub(crate) fn shown(&self) -> ShownTarget {
        ShownTarget(self.to_string())
    }
}

#[cfg(test)]
impl Target {
    /// A relay route spelled out, for a test that builds an error by hand.
    pub(crate) fn relay(text: &str) -> Self {
        Self::Relay(Url::at(text.to_owned()))
    }

    /// The full address, for a test that reads where a request went.
    pub(crate) fn reveal(&self) -> &str {
        match self {
            Self::Relay(url) => url.as_str(),
            Self::Object(link) => link.secret().expose(),
        }
    }
}

impl From<&Url<Route>> for Target {
    fn from(url: &Url<Route>) -> Self {
        Self::Relay(url.clone())
    }
}

impl From<&ambre_core::Credential<ambre_core::PresignedUrl>> for Target {
    fn from(link: &ambre_core::Credential<ambre_core::PresignedUrl>) -> Self {
        Self::Object(link.clone())
    }
}

/// A target shows its address without its query. A relay query carries what a
/// user typed, a searched address among it, and a log line keeps none of it
/// (`SYNC-292`). An object link's query is its credential.
impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Relay(url) => {
                let text = url.as_str();
                f.write_str(text.split_once('?').map_or(text, |(path, _)| path))
            }
            Self::Object(link) => f.write_str(link.object_address()),
        }
    }
}

/// A [`Target`] as a log and an error show it: a relay route or an object
/// link, each without its query. It holds no credential, so an error that
/// keeps it compares and clones freely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShownTarget(String);

impl std::fmt::Display for ShownTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone)]
pub(crate) struct AuthContext {
    pub api_base: crate::http::Url<crate::http::RelayHost>,
    pub url_scheme: crate::http::UrlScheme,
    pub tokens: TokenStore,
    /// The gate every renewal of `tokens` passes, so two never run at once
    /// (`SYNC-184`). Each context over one store shares it.
    pub renewing: Rc<futures_util::lock::Mutex<()>>,
    /// Where a renewal reports that it replaced the pair, so the worker arms
    /// the next renewal ahead of the expiry. `None` for a context whose
    /// store is not the worker's yet.
    pub renewed: Option<ambre_bus::conduit::UnboundedSender<crate::SyncEvent>>,
}

#[derive(Serialize)]
struct RefreshRequest {
    refresh_token: Credential<RefreshToken>,
}

impl AuthContext {
    /// The root of the relay routes this session talks to.
    pub(crate) fn api_root(&self) -> Url<ApiRoot> {
        Url::<ApiRoot>::of(self.url_scheme, &self.api_base)
    }

    fn access_token(&self) -> Result<Credential<AccessToken>, HttpError> {
        self.tokens
            .borrow()
            .as_ref()
            .map(|t| t.access_token().clone())
            .ok_or(HttpError::Local(LocalFailure::NotSignedIn))
    }

    fn refresh_token(&self) -> Result<Credential<RefreshToken>, HttpError> {
        self.tokens
            .borrow()
            .as_ref()
            .map(|t| t.refresh_token().clone())
            .ok_or(HttpError::Local(LocalFailure::NotSignedIn))
    }

    /// Whether the store still holds the pair of `access`.
    fn holds(&self, access: &Credential<AccessToken>) -> bool {
        self.tokens
            .borrow()
            .as_ref()
            .is_some_and(|t| t.access_token().matches(access))
    }

    /// Renews the pair that holds `stale` over the browser's `fetch`. The
    /// socket calls it on its own refusal, and the worker ahead of the
    /// expiry. Only the wasm build reaches either.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(
            dead_code,
            reason = "only the wasm socket and timer renew the token pair over fetch"
        )
    )]
    pub(crate) async fn renew(&self, stale: &Credential<AccessToken>) -> Result<(), HttpError> {
        self.renew_via(&Fetch, stale).await
    }

    /// Renews the pair that holds `stale` through `transport`. The refresh
    /// token is the one credential the request carries. Each access token
    /// is renewed once (`SYNC-184`):
    /// * the gate makes a second caller wait for the renewal in flight;
    /// * a caller whose token is no longer stored sends nothing, since the
    ///   renewal it waited for already replaced it.
    ///
    /// The relay honours each refresh token once (`SRV-009`), so a second
    /// renewal of one pair would be refused and fail its caller. An answer
    /// that lands after a sign-out is dropped, so the sign-out leaves no
    /// token (`SYNC-100`).
    async fn renew_via(
        &self,
        transport: &impl Transport,
        stale: &Credential<AccessToken>,
    ) -> Result<(), HttpError> {
        let _turn = self.renewing.lock().await;
        if !self.holds(stale) {
            return Ok(());
        }
        let url = self.api_root().route("/auth/refresh");
        let body = RefreshRequest {
            refresh_token: self.refresh_token()?,
        };
        let next = post_json_via::<_, TokenPair>(transport, &url, &body).await?;
        if !self.holds(stale) {
            return Ok(());
        }
        *self.tokens.borrow_mut() = Some(next);
        if let Some(renewed) = &self.renewed {
            let _ = renewed.send(crate::SyncEvent::TokensRenewed);
        }
        Ok(())
    }

    /// Renews the pair the store holds now, through `transport`. A key store
    /// that finds the keys stored already renews its keyless pair, and the
    /// relay mints the next one under the registered key (`SRV-307`).
    pub(crate) async fn renew_held_via(&self, transport: &impl Transport) -> Result<(), HttpError> {
        let stale = self.access_token()?;
        self.renew_via(transport, &stale).await
    }

    /// Revokes the session's refresh token at the relay over the browser's
    /// `fetch` (`SRV-026`).
    pub(crate) async fn revoke(&self) -> Result<(), HttpError> {
        self.revoke_via(&Fetch).await
    }

    /// Revokes the session's refresh token through `transport`. The route
    /// takes the access token, so a refused one is renewed first. The body
    /// is built again after that renewal: the renewal replaced the refresh
    /// token, and revoking the old one would leave the new one alive.
    async fn revoke_via(&self, transport: &impl Transport) -> Result<(), HttpError> {
        let url = self.api_root().route("/auth/logout");
        let send = || async {
            // The same body as a renewal's: the refresh token alone.
            let body = RefreshRequest {
                refresh_token: self.refresh_token()?,
            };
            let mut request = Outgoing::json(Method::Post, (&url).into(), &body)?;
            let access = self.access_token()?;
            request.bearer = Some(access.clone());
            let resp = exchange(transport, &request).await?;
            Ok::<_, HttpError>((request, resp, access))
        };
        let (mut request, mut resp, sent) = send().await?;
        if resp.status == HttpStatus::new(401) {
            self.renew_via(transport, &sent).await?;
            (request, resp, _) = send().await?;
        }
        expect_ok(&request, &resp)
    }
}

/// Per-request timeout for every Sync HTTP call (M9). Without it a hung TCP
/// connection or a stalled proxy would block the Sync worker's request future
/// forever (the browser `fetch` has no default timeout).
const REQUEST_TIMEOUT: TimeSpan<Ms> = TimeSpan::<Ms>::new(30_000);

/// The ceiling of the body of a relay answer. A members answer with its log
/// is the largest one, and stays far under it (`SYNC-277`, audit `W206`,
/// `ambre-spec-budgets.md` section 1).
const ANSWER_CEILING: ambre_core::Count<ambre_core::Octets> =
    ambre_core::Count::new(16 * 1024 * 1024);

/// The ceiling of an object the store serves: a snapshot or a media blob.
/// It sits above the largest object the relay lets a client upload
/// (`SYNC-277`, `ambre-spec-budgets.md` section 1).
const OBJECT_CEILING: ambre_core::Count<ambre_core::Octets> =
    ambre_core::Count::new(128 * 1024 * 1024);

/// The method of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `PATCH`.
    Patch,
    /// `DELETE`.
    Delete,
}

impl Method {
    /// The method's name, as an error message carries it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

/// The content type of a request body. The client sends JSON to the relay
/// and opaque sealed bytes to object storage, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentType {
    /// `application/json`.
    Json,
    /// `application/octet-stream`.
    Octets,
}

/// The role of the bytes of a request or an answer body: JSON, or a sealed
/// blob. `Payload<HttpBody>`. `Debug` prints the length and none of the
/// bytes, as the body may carry a token.
pub(crate) enum HttpBody {}

impl ambre_core::PayloadKind for HttpBody {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("HttpBody");
}

/// The answer of a download is a blob some writer sealed, neither verified
/// nor opened yet: the one role an answer body becomes.
impl ambre_core::Becomes<crate::pipeline::FetchedBlob> for HttpBody {}

/// One request as the transport sends it. The policy below builds it, and
/// the transport only moves it.
#[derive(Clone)]
pub(crate) struct Outgoing {
    /// The method.
    pub method: Method,
    /// Where it goes.
    pub url: Target,
    /// The access token the `Authorization` header presents, on a request to
    /// the relay that needs one. The transport spells the header at send time.
    pub bearer: Option<Credential<AccessToken>>,
    /// The content type and the bytes of the body, if any.
    pub body: Option<(ContentType, Payload<HttpBody>)>,
    /// The SHA-256 digest of the body that an upload link signs. The
    /// transport sends it as `x-amz-checksum-sha256` (`SRV-310`).
    pub checksum: Option<ambre_core::FixedBytes<ambre_core::ObjectChecksum>>,
    /// How long the transport waits before it aborts the request.
    pub timeout: TimeSpan<Ms>,
    /// The most bytes the answer's body may hold. The transport stops
    /// reading past it (`SYNC-277`).
    pub ceiling: ambre_core::Count<ambre_core::Octets>,
}

impl Outgoing {
    /// A request with no body and no credential, under the per-request
    /// timeout and the ceiling of a relay answer.
    fn new(method: Method, url: Target) -> Self {
        Self {
            method,
            url,
            bearer: None,
            body: None,
            checksum: None,
            timeout: REQUEST_TIMEOUT,
            ceiling: ANSWER_CEILING,
        }
    }

    /// A request carrying `body` as JSON.
    ///
    /// Only the encode is generic: one copy per body type stays in the worker,
    /// and the rest is [`Outgoing::with_json`] once (`ambre-spec-budgets.md`
    /// section 5).
    fn json<T: Serialize>(method: Method, url: Target, body: &T) -> Result<Self, HttpError> {
        Self::with_json(
            method,
            url,
            serde_json::to_vec(body).map(Payload::<HttpBody>::new),
        )
    }

    /// A request carrying `encoded` as JSON, or the refusal of a body that
    /// did not encode.
    fn with_json(
        method: Method,
        url: Target,
        encoded: Result<Payload<HttpBody>, serde_json::Error>,
    ) -> Result<Self, HttpError> {
        let body = encoded.map_err(|e| {
            crate::log::warning!("Sync: {} {url}: serialize error: {e}", method.as_str());
            HttpError::Local(LocalFailure::Encode)
        })?;
        Ok(Self {
            body: Some((ContentType::Json, body)),
            ..Self::new(method, url)
        })
    }
}

/// One answer: its status and its body.
pub(crate) struct Incoming {
    /// The HTTP status.
    pub status: HttpStatus,
    /// The body, empty when there is none.
    pub body: Payload<HttpBody>,
}

/// What moves a request and brings its answer back. The renewal, the replay
/// and the error mapping live above it, so a host test drives them through a
/// scripted transport.
pub(crate) trait Transport {
    /// Sends `request`. An error is a request that got no answer at all.
    async fn send(&self, request: &Outgoing) -> Result<Incoming, TransportError>;

    /// Resolves once `ms` milliseconds have passed.
    async fn elapse(&self, span: TimeSpan<Ms>);
}

/// Why a request got no answer. The browser's own words ride in the variants
/// that carry them, as free text this build never reads back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TransportError {
    /// The body did not go into the request.
    Body(String),
    /// `fetch` failed: the network, the abort signal or a refused origin.
    Fetch(String),
    /// The answer came, and its body did not read whole.
    Truncated(String),
    /// No answer arrived within the request's deadline (`SYNC-071`).
    Deadline(TimeSpan<Ms>),
    /// The body passed the ceiling of its request, which the transport
    /// stopped reading at (`SYNC-277`, audit `W206`).
    Oversized(ambre_core::Count<ambre_core::Octets>),
}

/// Counts the bytes of a body as they arrive. It refuses the bytes past the
/// ceiling of the request before they are kept (`SYNC-277`, audit `W206`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyMeter {
    ceiling: ambre_core::Count<ambre_core::Octets>,
    read: ambre_core::Count<ambre_core::Octets>,
}

impl BodyMeter {
    /// A meter with nothing read, under `ceiling`.
    pub(crate) fn new(ceiling: ambre_core::Count<ambre_core::Octets>) -> Self {
        Self {
            ceiling,
            read: ambre_core::Count::new(0),
        }
    }

    /// Takes `more` bytes of the body.
    ///
    /// # Errors
    ///
    /// [`TransportError::Oversized`] when the body would pass the ceiling.
    pub(crate) fn take(
        &mut self,
        more: ambre_core::Count<ambre_core::Octets>,
    ) -> Result<(), TransportError> {
        let read = self.read.plus(more);
        if read > self.ceiling {
            return Err(TransportError::Oversized(self.ceiling));
        }
        self.read = read;
        Ok(())
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Body(reason) => write!(f, "body error: {reason}"),
            Self::Fetch(reason) => f.write_str(reason),
            Self::Truncated(reason) => write!(f, "binary error: {reason}"),
            Self::Deadline(span) => write!(f, "no answer within {} ms", span.get()),
            Self::Oversized(ceiling) => {
                write!(
                    f,
                    "the answer passes its ceiling of {} bytes",
                    ceiling.get()
                )
            }
        }
    }
}

/// The browser's `fetch`, the transport of the running worker.
pub(crate) struct Fetch;

/// Reads `stream` chunk by chunk under `meter`. A length header can lie or be
/// absent, so the count runs while the body arrives. The read stops at the
/// chunk that passes the ceiling (`SYNC-278`, audit `W206`).
async fn read_metered(
    stream: web_sys::ReadableStream,
    mut meter: BodyMeter,
) -> Result<Vec<u8>, TransportError> {
    use wasm_bindgen::JsCast;

    let truncated = |e: wasm_bindgen::JsValue| TransportError::Truncated(format!("{e:?}"));
    let reader: web_sys::ReadableStreamDefaultReader = stream.get_reader().unchecked_into();
    let mut body = Vec::new();
    loop {
        let chunk = wasm_bindgen_futures::JsFuture::from(reader.read())
            .await
            .map_err(truncated)?;
        let done = js_sys::Reflect::get(&chunk, &wasm_bindgen::JsValue::from_str("done"))
            .map_err(truncated)?
            .as_bool()
            .unwrap_or(true);
        if done {
            return Ok(body);
        }
        let value: js_sys::Uint8Array =
            js_sys::Reflect::get(&chunk, &wasm_bindgen::JsValue::from_str("value"))
                .map_err(truncated)?
                .unchecked_into();
        if let Err(oversized) = meter.take(ambre_core::Count::new(u64::from(value.length()))) {
            // The rest of the body is never read.
            let _ = reader.cancel();
            return Err(oversized);
        }
        body.extend(value.to_vec());
    }
}

/// Reads the body of an answer under `ceiling`. A length `stated` past the
/// ceiling is refused before any byte is read (`SYNC-277`). The bytes are
/// then counted as they arrive, since a length header can lie or be absent
/// (`SYNC-278`).
async fn read_body(
    stated: Option<ambre_core::Count<ambre_core::Octets>>,
    stream: Option<web_sys::ReadableStream>,
    ceiling: ambre_core::Count<ambre_core::Octets>,
) -> Result<Vec<u8>, TransportError> {
    if let Some(stated) = stated {
        BodyMeter::new(ceiling).take(stated)?;
    }
    match stream {
        Some(stream) => read_metered(stream, BodyMeter::new(ceiling)).await,
        None => Ok(Vec::new()),
    }
}

/// The count of bytes [`read_body`] keeps of `stream`, or `None` when it stops
/// the read past `ceiling`. The wasm-tier test of `SYNC-278` reads it: the
/// metered read runs only on a browser stream.
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub async fn metered_body_len(
    stated: Option<ambre_core::Count<ambre_core::Octets>>,
    stream: web_sys::ReadableStream,
    ceiling: ambre_core::Count<ambre_core::Octets>,
) -> Option<ambre_core::Count<ambre_core::Octets>> {
    let body = read_body(stated, Some(stream), ceiling).await.ok()?;
    Some(ambre_core::Count::new(u64::try_from(body.len()).ok()?))
}

impl Method {
    /// The same method, as gloo-net names it.
    fn gloo(self) -> gloo_net::http::Method {
        match self {
            Self::Get => gloo_net::http::Method::GET,
            Self::Post => gloo_net::http::Method::POST,
            Self::Put => gloo_net::http::Method::PUT,
            Self::Patch => gloo_net::http::Method::PATCH,
            Self::Delete => gloo_net::http::Method::DELETE,
        }
    }
}

impl Transport for Fetch {
    async fn send(&self, request: &Outgoing) -> Result<Incoming, TransportError> {
        // A new signal per request, because a timeout signal starts counting
        // down the moment it is created. gloo-net copies it into the request,
        // so it may be dropped here while `fetch` still holds it.
        let signal = web_sys::AbortSignal::timeout_with_u32(
            u32::try_from(request.timeout.get()).unwrap_or(u32::MAX),
        );
        let mut builder = request
            .url
            .open(gloo_net::http::RequestBuilder::new)
            .method(request.method.gloo())
            .abort_signal(Some(&signal));
        if let Some(token) = &request.bearer {
            // The value is wiped once the request holds its copy.
            builder = token.present_bearer(|value| builder.header("Authorization", value));
        }
        if let Some(checksum) = &request.checksum {
            // The link signs this header, so the store checks the body
            // against it and refuses any other (`SRV-310`).
            use base64::Engine as _;
            let value = base64::engine::general_purpose::STANDARD.encode(checksum.as_array());
            builder = builder.header("x-amz-checksum-sha256", &value);
        }
        let sent = match &request.body {
            Some((content_type, bytes)) => {
                let content_type = match content_type {
                    ContentType::Json => "application/json",
                    ContentType::Octets => "application/octet-stream",
                };
                builder
                    .header("Content-Type", content_type)
                    .body(bytes.as_bytes().to_vec())
                    .map_err(|e| TransportError::Body(e.to_string()))?
                    .send()
                    .await
            }
            None => builder.send().await,
        };
        let resp = sent.map_err(|e| TransportError::Fetch(e.to_string()))?;
        let stated = resp
            .headers()
            .get("content-length")
            .and_then(|text| text.trim().parse::<u64>().ok())
            .map(ambre_core::Count::new);
        let body = read_body(stated, resp.body(), request.ceiling).await?;
        Ok(Incoming {
            status: HttpStatus::new(resp.status()),
            body: Payload::<HttpBody>::new(body),
        })
    }

    async fn elapse(&self, span: TimeSpan<Ms>) {
        use wasm_bindgen::JsCast;

        let Ok(scope) = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>() else {
            // No timer to wait on: never resolve, and leave the abort signal
            // to end the request.
            return std::future::pending().await;
        };
        let timer = js_sys::Promise::new(&mut |resolve, _reject| {
            let _ = scope.set_timeout_with_callback_and_timeout_and_arguments_0(
                &resolve,
                i32::try_from(span.get()).unwrap_or(i32::MAX),
            );
        });
        let _ = wasm_bindgen_futures::JsFuture::from(timer).await;
    }
}

/// The body the relay sends with a refusal. Only its code is read, a word of
/// one closed set (`SRV-182`). A code this build does not hold fails the read,
/// so the refusal reads as its status alone.
#[derive(Deserialize)]
struct RefusalBody {
    error: RefusalCode,
}

/// Why a request failed, kept as a value so a caller matches the case and
/// never the text. [`fmt::Display`] writes the text a log and a toast read:
/// the method and the URL, then the cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HttpError {
    /// The request failed on this device. No token is held, the body did not
    /// serialize, or the answer is not the one the request asked for.
    Local(LocalFailure),
    /// The request got no answer: the transport failed, or the deadline
    /// passed. `reason` is the transport's.
    NoAnswer {
        /// The method.
        method: Method,
        /// Where it went.
        url: ShownTarget,
        /// What the transport said.
        reason: TransportError,
    },
    /// The relay refused the request with `status`. `code` is the relay's
    /// error code, kept only with the status that may carry it. A 403
    /// carries a want of access or a setup still owed (`SYNC-134`). A 409
    /// carries a membership, a key or a log that moved under the request
    /// (`SYNC-143`, `SRV-104`).
    Refused {
        /// The method.
        method: Method,
        /// Where it went.
        url: ShownTarget,
        /// The status the relay answered.
        status: HttpStatus,
        /// The code its body named, on a status that may carry one.
        code: Option<RefusalCode>,
    },
    /// A successful answer whose body did not read, for `cause`. An unknown
    /// word is one only a newer relay writes (`SYNC-152`).
    Unreadable {
        /// The method.
        method: Method,
        /// Where it went.
        url: ShownTarget,
        /// What the parser said.
        reason: String,
        /// Whether the body was malformed, or named a word this build does
        /// not know.
        cause: UnreadableCause,
    },
}

impl HttpError {
    /// The refusal of `method` to `url` with `status`. The relay's `code` is
    /// kept only with the status that may carry it.
    pub(crate) fn refused(
        method: Method,
        url: &Target,
        status: HttpStatus,
        code: Option<RefusalCode>,
    ) -> Self {
        use RefusalCode::{Forbidden, KeyChanged, LogMoved, MembersChanged, SetupRequired};
        let code = match (status.get(), code) {
            (403, Some(code @ (SetupRequired | Forbidden)))
            | (409, Some(code @ (MembersChanged | KeyChanged | LogMoved))) => Some(code),
            _ => None,
        };
        Self::Refused {
            method,
            url: url.shown(),
            status,
            code,
        }
    }

    /// The relay's refusal code, when the relay answered with one.
    pub(crate) fn code(&self) -> Option<RefusalCode> {
        match self {
            Self::Refused { code, .. } => *code,
            _ => None,
        }
    }

    /// Whether the relay refused the request with `code`.
    pub(crate) fn is(&self, code: RefusalCode) -> bool {
        self.code() == Some(code)
    }

    /// Whether the relay refused the request with a bare 409: the state it
    /// names moved past what the request expects.
    pub(crate) fn is_conflict(&self) -> bool {
        matches!(
            self,
            Self::Refused {
                status,
                code: None,
                ..
            } if status.get() == 409
        )
    }

    /// The failure as the bus carries it to Logic and the UI.
    pub(crate) fn failure(&self) -> ApiFailure {
        match self {
            Self::Local(cause) => ApiFailure::Local(*cause),
            Self::NoAnswer { .. } => ApiFailure::NoAnswer,
            Self::Refused { status, code, .. } => ApiFailure::Relay {
                status: *status,
                code: *code,
            },
            Self::Unreadable { cause, .. } => ApiFailure::Unreadable { cause: *cause },
        }
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(cause) => cause.fmt(f),
            Self::NoAnswer {
                method,
                url,
                reason,
            } => write!(f, "{} {url}: {reason}", method.as_str()),
            Self::Refused {
                method,
                url,
                code: Some(code),
                ..
            } => write!(f, "{} {url}: {code}", method.as_str()),
            Self::Refused {
                method,
                url,
                status,
                code: None,
            } => write!(f, "{} {url}: HTTP {status}", method.as_str()),
            Self::Unreadable {
                method,
                url,
                reason,
                cause,
            } => {
                write!(f, "{} {url}: parse error: {reason}", method.as_str())?;
                // The text closes on the cause when the body names a word of
                // a closed set this build does not know.
                match cause {
                    UnreadableCause::Malformed => Ok(()),
                    UnreadableCause::UnknownWord => f.write_str(": UNKNOWN_WORD"),
                }
            }
        }
    }
}

/// An error a task reports to Logic: an answer of the relay, or a failure
/// on this device, which carries text alone.
pub(crate) trait Reported: std::fmt::Display {
    /// The failure as the bus carries it.
    fn failure(&self) -> ApiFailure;

    /// This error as the failure of `step`. The step's name and the error's
    /// text go to the log here: the bus carries the failure alone (`BUS-046`).
    fn at(&self, step: &str) -> Failed {
        crate::log::warning!("Sync: {step}: {self}");
        Failed {
            failure: self.failure(),
        }
    }
}

impl Reported for HttpError {
    fn failure(&self) -> ApiFailure {
        Self::failure(self)
    }
}

/// A failure a step met, such as a media leg's upload. The step's text went to
/// the log where the step failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Failed {
    /// The failure as the bus carries it.
    pub failure: ApiFailure,
}

impl Failed {
    /// A failure on this device, for `cause`.
    pub(crate) const fn local(cause: LocalFailure) -> Self {
        Self {
            failure: ApiFailure::Local(cause),
        }
    }
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.failure.fmt(f)
    }
}

impl Reported for Failed {
    fn failure(&self) -> ApiFailure {
        self.failure
    }
}

/// The error a refused answer reads as, or `Ok` for a success. The relay's
/// code is read from the body only on the statuses that may carry one.
fn expect_ok(request: &Outgoing, resp: &Incoming) -> Result<(), HttpError> {
    if resp.status.is_success() {
        return Ok(());
    }
    let code = if matches!(resp.status.get(), 403 | 409) {
        serde_json::from_slice::<RefusalBody>(resp.body.as_bytes())
            .ok()
            .map(|body| body.error)
    } else {
        None
    };
    Err(HttpError::refused(
        request.method,
        &request.url,
        resp.status,
        code,
    ))
}

/// The JSON body of a successful answer.
fn read_json<R: serde::de::DeserializeOwned>(
    request: &Outgoing,
    resp: &Incoming,
) -> Result<R, HttpError> {
    expect_ok(request, resp)?;
    // Only the decode is generic: the error mapping is `unreadable`, once.
    serde_json::from_slice(resp.body.as_bytes()).map_err(|e| unreadable(request, &e))
}

/// The error of a successful answer whose JSON body did not decode.
fn unreadable(request: &Outgoing, e: &serde_json::Error) -> HttpError {
    HttpError::Unreadable {
        method: request.method,
        url: request.url.shown(),
        // A word of a closed set this build does not know means the relay is
        // newer (`SYNC-152`).
        cause: if e.is_data() && e.to_string().starts_with("unknown variant `") {
            UnreadableCause::UnknownWord
        } else {
            UnreadableCause::Malformed
        },
        reason: e.to_string(),
    }
}

/// Sends `request` once. A request that got no answer reads as its method
/// and its URL, then the transport's reason.
async fn exchange(transport: &impl Transport, request: &Outgoing) -> Result<Incoming, HttpError> {
    use futures_util::future::{Either, select};

    // The deadline is raced here, not left to the transport alone, so a
    // request that never answers is abandoned on every transport (`SYNC-071`).
    let send = std::pin::pin!(transport.send(request));
    let deadline = std::pin::pin!(transport.elapse(request.timeout));
    let answer = match select(send, deadline).await {
        Either::Left((answer, _)) => answer,
        Either::Right(((), _)) => Err(TransportError::Deadline(request.timeout)),
    };
    // Every transport answers under the ceiling: a body past it is no
    // answer, whatever transport read it (`SYNC-277`).
    let answer = answer.and_then(|incoming| {
        BodyMeter::new(request.ceiling)
            .take(incoming.body.len())
            .map(|()| incoming)
    });
    answer.map_err(|reason| HttpError::NoAnswer {
        method: request.method,
        url: request.url.shown(),
        reason,
    })
}

/// Sends `request` under the session's access token. A 401 renews the pair
/// and replays the request once, with the new token (`SYNC-062`). The
/// answer to that replay is the caller's, whatever it is.
async fn exchange_auth(
    transport: &impl Transport,
    auth: &AuthContext,
    request: &mut Outgoing,
) -> Result<Incoming, HttpError> {
    let sent = auth.access_token()?;
    request.bearer = Some(sent.clone());
    let resp = exchange(transport, request).await?;
    if resp.status != HttpStatus::new(401) {
        return Ok(resp);
    }
    auth.renew_via(transport, &sent).await?;
    request.bearer = Some(auth.access_token()?);
    exchange(transport, request).await
}

/// An authenticated GET of JSON, reading `404 Not Found` as `Ok(None)`.
pub(crate) async fn get_json_opt_via<T: serde::de::DeserializeOwned>(
    transport: &impl Transport,
    url: &Url<Route>,
    auth: &AuthContext,
) -> Result<Option<T>, HttpError> {
    let mut request = Outgoing::new(Method::Get, url.into());
    let resp = exchange_auth(transport, auth, &mut request).await?;
    if resp.status == HttpStatus::new(404) {
        return Ok(None);
    }
    read_json(&request, &resp).map(Some)
}

/// An authenticated GET of JSON.
pub(crate) async fn get_json_auth_via<T: serde::de::DeserializeOwned>(
    transport: &impl Transport,
    url: &Url<Route>,
    auth: &AuthContext,
) -> Result<T, HttpError> {
    let mut request = Outgoing::new(Method::Get, url.into());
    let resp = exchange_auth(transport, auth, &mut request).await?;
    read_json(&request, &resp)
}

/// A POST of JSON that carries no credential, answered with JSON.
pub(crate) async fn post_json_via<T: Serialize, R: serde::de::DeserializeOwned>(
    transport: &impl Transport,
    url: &Url<Route>,
    body: &T,
) -> Result<R, HttpError> {
    let request = Outgoing::json(Method::Post, url.into(), body)?;
    let resp = exchange(transport, &request).await?;
    read_json(&request, &resp)
}

/// An authenticated POST of JSON, answered with JSON.
pub(crate) async fn post_json_auth_via<T: Serialize, R: serde::de::DeserializeOwned>(
    transport: &impl Transport,
    url: &Url<Route>,
    body: &T,
    auth: &AuthContext,
) -> Result<R, HttpError> {
    let mut request = Outgoing::json(Method::Post, url.into(), body)?;
    let resp = exchange_auth(transport, auth, &mut request).await?;
    read_json(&request, &resp)
}

/// An authenticated request carrying `body` as JSON, to a route that answers
/// with no body.
pub(crate) async fn send_json_auth_via<T: Serialize>(
    transport: &impl Transport,
    method: Method,
    url: &Url<Route>,
    body: &T,
    auth: &AuthContext,
) -> Result<(), HttpError> {
    let mut request = Outgoing::json(method, url.into(), body)?;
    let resp = exchange_auth(transport, auth, &mut request).await?;
    expect_ok(&request, &resp)
}

/// An authenticated request carrying `body` as JSON, answered with JSON.
pub(crate) async fn send_json_auth_read_via<T: Serialize, R: serde::de::DeserializeOwned>(
    transport: &impl Transport,
    method: Method,
    url: &Url<Route>,
    body: &T,
    auth: &AuthContext,
) -> Result<R, HttpError> {
    let mut request = Outgoing::json(method, url.into(), body)?;
    let resp = exchange_auth(transport, auth, &mut request).await?;
    read_json(&request, &resp)
}

/// HTTP GET to JSON, treating `404 Not Found` as `Ok(None)`.
pub(crate) async fn get_json_opt_auth<T: serde::de::DeserializeOwned>(
    url: &Url<Route>,
    auth: AuthContext,
) -> Result<Option<T>, HttpError> {
    get_json_opt_via(&Fetch, url, &auth).await
}

/// HTTP GET to JSON deserialized response.
pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(
    url: &Url<Route>,
) -> Result<T, HttpError> {
    let request = Outgoing::new(Method::Get, url.into());
    let resp = exchange(&Fetch, &request).await?;
    read_json(&request, &resp)
}

pub(crate) async fn get_json_auth<T: serde::de::DeserializeOwned>(
    url: &Url<Route>,
    auth: AuthContext,
) -> Result<T, HttpError> {
    get_json_auth_via(&Fetch, url, &auth).await
}

pub(crate) async fn post_json<T: Serialize, R: serde::de::DeserializeOwned>(
    url: &Url<Route>,
    body: &T,
) -> Result<R, HttpError> {
    post_json_via(&Fetch, url, body).await
}

pub(crate) async fn post_json_auth<T: Serialize, R: serde::de::DeserializeOwned>(
    url: &Url<Route>,
    body: &T,
    auth: AuthContext,
) -> Result<R, HttpError> {
    post_json_auth_via(&Fetch, url, body, &auth).await
}

/// The sealed blob of one snapshot of `document_id`: a presigned URL from the
/// relay, then the object itself. `None` when either step fails.
pub(crate) async fn download_snapshot(
    url_scheme: crate::http::UrlScheme,
    api_base: &crate::http::Url<crate::http::RelayHost>,
    document_id: Id<Document>,
    snapshot_id: Id<Snapshot>,
    auth: AuthContext,
) -> Option<Payload<crate::pipeline::FetchedBlob>> {
    let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, api_base)
        .route(format!("/documents/{document_id}/snapshot/download-url"));
    let body = SnapshotDownloadUrlRequest { snapshot_id };
    let resp = post_json_auth::<_, SnapshotDownloadUrlResponse>(&url, &body, auth)
        .await
        .ok()?;
    fetch_bytes(&resp.download_url).await.ok()
}

pub(crate) async fn delete_auth(url: &Url<Route>, auth: AuthContext) -> Result<(), HttpError> {
    let mut request = Outgoing::new(Method::Delete, url.into());
    let resp = exchange_auth(&Fetch, &auth, &mut request).await?;
    expect_ok(&request, &resp)
}

/// Sends an authenticated `DELETE` and answers the status the relay gave. Only
/// a request that reached no answer is an error.
pub(crate) async fn delete_status_auth(
    url: &Url<Route>,
    auth: AuthContext,
) -> Result<HttpStatus, HttpError> {
    let mut request = Outgoing::new(Method::Delete, url.into());
    exchange_auth(&Fetch, &auth, &mut request)
        .await
        .map(|resp| resp.status)
}

/// PUTs a sealed blob to a presigned object-storage URL. It takes what the seal
/// built and nothing else, so no plaintext reaches object storage (`SYNC-049`,
/// `ZK-001`).
pub(crate) async fn put_sealed(
    url: &ambre_core::Credential<ambre_core::PresignedUrl>,
    blob: &crate::pipeline::SealedBlob,
) -> Result<(), HttpError> {
    put_sealed_via(&Fetch, url, blob).await
}

/// A sealed blob to a presigned object-storage link, with no credential: the
/// link is its own.
pub(crate) async fn put_sealed_via(
    transport: &impl Transport,
    url: &ambre_core::Credential<ambre_core::PresignedUrl>,
    blob: &crate::pipeline::SealedBlob,
) -> Result<(), HttpError> {
    let request = Outgoing {
        body: Some((ContentType::Octets, blob.to_body())),
        ..Outgoing::new(Method::Put, url.into())
    };
    let resp = exchange(transport, &request).await?;
    expect_ok(&request, &resp)
}

/// A sealed blob to a presigned link that signs its SHA-256 digest: the
/// request carries the digest the link names, as the store requires
/// (`SRV-310`).
pub(crate) async fn put_checked_via(
    transport: &impl Transport,
    url: &ambre_core::Credential<ambre_core::PresignedUrl>,
    blob: &crate::pipeline::SealedBlob,
) -> Result<(), HttpError> {
    let request = Outgoing {
        body: Some((ContentType::Octets, blob.to_body())),
        checksum: Some(blob.checksum()),
        ..Outgoing::new(Method::Put, url.into())
    };
    let resp = exchange(transport, &request).await?;
    expect_ok(&request, &resp)
}

pub(crate) async fn patch_json_auth<T: Serialize>(
    url: &Url<Route>,
    body: &T,
    auth: AuthContext,
) -> Result<(), HttpError> {
    send_json_auth_via(&Fetch, Method::Patch, url, body, &auth).await
}

/// HTTP DELETE carrying a JSON body: a departure sends the entry it signs
/// (`SYNC-132`).
pub(crate) async fn delete_json_auth<T: Serialize>(
    url: &Url<Route>,
    body: &T,
    auth: AuthContext,
) -> Result<(), HttpError> {
    send_json_auth_via(&Fetch, Method::Delete, url, body, &auth).await
}

/// HTTP PUT of a JSON body, expecting a success with no body to read.
pub(crate) async fn put_json_auth<T: Serialize>(
    url: &Url<Route>,
    body: &T,
    auth: AuthContext,
) -> Result<(), HttpError> {
    send_json_auth_via(&Fetch, Method::Put, url, body, &auth).await
}

/// HTTP POST of a JSON body, expecting a success with no body to read.
pub(crate) async fn post_json_auth_ok<T: Serialize>(
    url: &Url<Route>,
    body: &T,
    auth: AuthContext,
) -> Result<(), HttpError> {
    send_json_auth_via(&Fetch, Method::Post, url, body, &auth).await
}

/// Downloads an object from the store under a presigned link: a snapshot or
/// a media blob. The body stops at the ceiling of an object (`SYNC-277`).
pub(crate) async fn fetch_bytes(
    url: &ambre_core::Credential<ambre_core::PresignedUrl>,
) -> Result<Payload<crate::pipeline::FetchedBlob>, HttpError> {
    let request = Outgoing {
        ceiling: OBJECT_CEILING,
        ..Outgoing::new(Method::Get, url.into())
    };
    let resp = exchange(&Fetch, &request).await?;
    expect_ok(&request, &resp)?;
    Ok(resp.body.into_role())
}

#[cfg(feature = "dev-login")]
#[derive(Serialize)]
pub(crate) struct DevLoginRequest {
    pub username: ambre_core::Validated<ambre_core::DisplayName>,
    pub signing_pub_key: ambre_core::FixedBytes<ambre_core::PublicKey>,
    pub exchange_pub_key: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
}

#[cfg(feature = "dev-login")]
#[derive(Deserialize)]
pub(crate) struct DevLoginResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    /// The pair, with the lifetime of its access token.
    #[serde(flatten)]
    pub tokens: TokenPair,
}

/// The relay's answer to `GET /api/auth/{provider}/login`.
#[derive(Deserialize)]
pub(crate) struct ProviderLoginResponse {
    pub url: ambre_core::Opaque<ambre_core::ProviderUrls>,
    pub state: ambre_core::Credential<ambre_core::OidcState>,
}

/// The provider's return, posted to `/api/auth/{provider}/callback`.
#[derive(Serialize)]
pub(crate) struct ProviderCallbackRequest {
    pub code: ambre_core::Credential<ambre_core::OidcCode>,
    pub state: ambre_core::Credential<ambre_core::OidcState>,
}

/// The relay's answer to a provider return: who signed in, and their tokens.
#[derive(Deserialize)]
pub(crate) struct ProviderCallbackResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub email: ambre_core::Email,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    /// The pair, with the lifetime of its access token.
    #[serde(flatten)]
    pub tokens: TokenPair,
}

#[derive(Deserialize)]
pub(crate) struct MasterKeyResponse {
    pub master_key_enc: Option<ambre_core::Encoded<ambre_core::WrappedMasterKey>>,
    pub master_key_salt: Option<ambre_core::FixedBytes<ambre_core::MasterKeySalt>>,
    /// The signing key the relay holds for the account. A recovery compares
    /// it with the key its words derive (`SYNC-249`).
    #[serde(default)]
    pub signing_pubkey: Option<ambre_core::FixedBytes<ambre_core::PublicKey>>,
}

/// The first key store of a signed-in account (`PUT /api/keys/master`). It
/// holds the sealed seed, its salt, the two public keys the seed derives and
/// the proof of the signing key. No seed, word or passphrase is representable
/// here (`SYNC-256`).
#[derive(Serialize)]
pub(crate) struct StoreMasterKeysRequest {
    pub master_key_enc: ambre_core::Encoded<ambre_core::WrappedMasterKey>,
    pub master_key_salt: ambre_core::FixedBytes<ambre_core::MasterKeySalt>,
    pub signing_pubkey: ambre_core::FixedBytes<ambre_core::PublicKey>,
    pub exchange_pubkey: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
    /// The signature of the signing key over the account and the two keys
    /// (`ambre_protocol::key_registration`, `SRV-294`).
    pub signature: ambre_core::FixedBytes<ambre_core::Signature>,
}

/// The relay's answer to a first key store: a fresh pair, bound to the new
/// signing key. The pair the store presented is stale from then on.
#[derive(Deserialize)]
pub(crate) struct KeysStoredResponse {
    /// The pair, with the lifetime of its access token.
    #[serde(flatten)]
    pub tokens: TokenPair,
}

impl MasterKeyResponse {
    /// Whether the account still owes its key setup: the relay holds no master
    /// material for it, and sends the fields `null`.
    pub fn setup_owed(&self) -> bool {
        self.master_key_enc.is_none()
    }
}

#[derive(Deserialize)]
pub(crate) struct AuthMeResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    pub email: ambre_core::Email,
}

/// `GET /api/channels/{id}/documents`: the documents a channel groups.
#[derive(Deserialize)]
pub(crate) struct ChannelDocumentsResponse {
    pub documents: Vec<ChannelDocumentEntry>,
}

#[derive(Deserialize)]
pub(crate) struct ChannelDocumentEntry {
    #[serde(with = "ambre_core::serde_id::text")]
    pub document_id: Id<Document>,
    /// `meta`, `segment`, `amendments` or `thread`.
    #[serde(with = "ambre_core::serde_enums::channel_document_kind")]
    pub kind: ambre_core::ChannelDocumentKind,
}

/// `GET /api/spaces/{id}/channels`: the channels the relay holds as public
/// in a space.
#[derive(Deserialize)]
pub(crate) struct SpaceChannelsResponse {
    #[serde(with = "ambre_core::serde_id::text::vec")]
    pub channels: Vec<Id<Channel>>,
}

/// `GET /api/channels`: every channel the caller is a member of, and every
/// space.
#[derive(Deserialize)]
pub(crate) struct MemberChannelsResponse {
    pub channels: Vec<MemberChannelEntry>,
    #[serde(with = "ambre_core::serde_id::text::vec")]
    pub spaces: Vec<Id<Space>>,
}

impl MemberChannelsResponse {
    /// The answer as the bus carries it: the channels, the spaces, and the
    /// other members of each direct message that lists some. The names stay
    /// sealed; the caller opens them.
    pub(crate) fn listing(
        &self,
    ) -> (
        Vec<ambre_bus::MemberChannel>,
        Vec<Id<Space>>,
        Vec<ambre_bus::DirectPeers>,
    ) {
        let channels = self
            .channels
            .iter()
            .map(MemberChannelEntry::member_channel)
            .collect();
        let peers = self
            .channels
            .iter()
            .filter(|entry| entry.space_id.is_none() && !entry.peers.is_empty())
            .map(|entry| ambre_bus::DirectPeers {
                channel_id: entry.channel_id,
                names: entry.peers.clone(),
            })
            .collect();
        (channels, self.spaces.clone(), peers)
    }
}

/// One channel of [`MemberChannelsResponse`]: its space, none for a direct
/// message, and whether the space lists it. It also carries the name sealed
/// beside its meta, with the caller's wrapped key of that meta (`SRV-219`).
#[derive(Deserialize)]
pub(crate) struct MemberChannelEntry {
    #[serde(with = "ambre_core::serde_id::text")]
    pub channel_id: Id<Channel>,
    #[serde(with = "ambre_core::serde_id::text::option")]
    pub space_id: Option<Id<Space>>,
    #[serde(rename = "public", with = "ambre_core::serde_enums::visibility_flag")]
    pub visibility: ambre_core::Visibility,
    #[serde(with = "ambre_core::serde_id::text")]
    pub meta_id: Id<Meta>,
    /// Base64 of the sealed name, none for a meta never named.
    #[serde(default)]
    pub name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>,
    /// Base64 of `nonce || ciphertext`.
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// Hex, `null` for a self-wrapped row.
    pub wrapper_exchange_pub_key: Option<ambre_core::FixedBytes<ambre_core::ExchangePublicKey>>,
    /// The display names of a direct message's other members (`SRV-220`).
    #[serde(default)]
    pub peers: Vec<ambre_core::Validated<ambre_core::DisplayName>>,
    /// The last number the relay gave in the meta (`SRV-223`), none from a
    /// relay that lists none.
    #[serde(default)]
    pub meta_last: Option<ambre_core::Seq<ambre_core::Relay>>,
}

impl MemberChannelEntry {
    /// The entry as the bus carries it. A channel in no space is a direct
    /// message, whatever its flag says.
    pub(crate) fn member_channel(&self) -> ambre_bus::MemberChannel {
        let place = match (self.space_id, self.visibility) {
            (Some(space), ambre_core::Visibility::Public) => ambre_bus::ChannelPlace::Public(space),
            (Some(space), ambre_core::Visibility::Private) => {
                ambre_bus::ChannelPlace::Private(space)
            }
            (None, _) => ambre_bus::ChannelPlace::Direct,
        };
        ambre_bus::MemberChannel {
            channel_id: self.channel_id,
            place,
        }
    }
}

/// `GET /api/activity`: per segment and thread of every channel of the
/// caller, whether it holds something past the caller's cursor.
#[derive(Deserialize)]
pub(crate) struct ActivityResponse {
    pub documents: Vec<ActivityEntry>,
}

#[derive(Deserialize)]
pub(crate) struct ActivityEntry {
    #[serde(with = "ambre_core::serde_id::text")]
    pub channel_id: Id<Channel>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub document_id: Id<Document>,
    #[serde(with = "ambre_core::serde_enums::channel_document_kind")]
    pub kind: ambre_core::ChannelDocumentKind,
    pub active: ambre_core::activity::Activity,
    /// The caller's own cursor. A relay that predates it answers none, which
    /// reads as nothing read.
    #[serde(default)]
    pub read: ambre_core::Seq<ambre_core::Relay>,
    /// The document's last number. A relay that predates it answers none,
    /// and a watch then starts from the head.
    #[serde(default)]
    pub last: Option<ambre_core::Seq<ambre_core::Relay>>,
}

/// `PUT /api/documents/{id}/read-cursor`.
#[derive(Serialize)]
pub(crate) struct ReadCursorRequest {
    pub seq: ambre_core::Seq<ambre_core::Relay>,
}

/// `POST /api/read-cursors/mark-read`: the documents to move to their last
/// number, or every one when absent.
#[derive(Serialize)]
pub(crate) struct MarkReadRequest {
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "ambre_core::serde_id::text::option_vec"
    )]
    pub documents: Option<Vec<Id<Document>>>,
}

#[derive(Deserialize)]
pub(crate) struct DocumentListResponse {
    pub documents: Vec<DocumentEntry>,
}

#[derive(Deserialize)]
pub(crate) struct DocumentEntry {
    #[serde(with = "ambre_core::serde_id::text")]
    pub id: Id<Document>,
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
    #[serde(default)]
    pub name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>,
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    /// The caller's wrapped copy of the DocumentKey (base64). Unwrapped via ECDH
    /// to decrypt `name_enc` for the launcher list.
    #[serde(default)]
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// Exchange pubkey (hex) of whoever wrapped `doc_key_enc`. `null` => fall
    /// back to our own key (self-wrapped owner row).
    #[serde(default)]
    pub wrapper_exchange_pub_key: Option<ambre_core::FixedBytes<ambre_core::ExchangePublicKey>>,
    /// Total member count; `> 1` => shared.
    #[serde(default)]
    pub member_count: ambre_core::Count<ambre_core::Members>,
    /// The relay's clock when it last wrote the document, in Unix
    /// milliseconds (`SYNC-181`).
    pub updated_at: ambre_core::Instant<ambre_core::Relayed>,
}

#[derive(Serialize)]
pub(crate) struct CreateDocumentRequest {
    #[serde(with = "ambre_core::serde_enums::doc_type")]
    pub doc_type: ambre_core::DocType,
    pub name_enc: Option<ambre_core::Encoded<ambre_core::SealedName>>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "ambre_core::serde_id::text::option"
    )]
    pub document_id: Option<Id<Document>>,
    /// The first entry of the membership log, as `membership_log::encode`
    /// writes it (`SYNC-132`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
    /// Whether the document is a Murmure space, which its members list. The
    /// JSON flag `space` is written for a space alone.
    #[serde(
        skip_serializing_if = "ambre_core::serde_enums::space_flag::is_personal",
        serialize_with = "ambre_core::serde_enums::space_flag::serialize"
    )]
    pub space: ambre_core::DocumentScope,
}

/// `POST /api/channels`: a channel of a space, with its first two documents.
#[derive(Serialize)]
pub(crate) struct CreateChannelRequest {
    /// A public channel, which its catalogue lists, or a private one.
    #[serde(rename = "public", with = "ambre_core::serde_enums::visibility_flag")]
    pub visibility: ambre_core::Visibility,
    #[serde(with = "ambre_core::serde_id::text")]
    pub channel_id: Id<Channel>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub space_id: Id<Space>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub meta_id: Id<Meta>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub segment_id: Id<Segment>,
}

/// `POST /api/channels/{id}/segments`: close the active segment `closing`
/// into `successor`, a document this user created for it.
#[derive(Serialize)]
pub(crate) struct CloseSegmentRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub closing: Id<Segment>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub successor: Id<Segment>,
}

/// The active segment after a close. `created` is false when an earlier
/// close registered another successor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(crate) struct ClosedSegmentResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub active: Id<Segment>,
    pub created: ambre_core::Creation,
}

/// `POST /api/channels/{id}/segments/{segment}/amendments`: the caller's new
/// document, to register as the amendments of the locked segment.
#[derive(Serialize)]
pub(crate) struct RegisterAmendmentsRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub amendments: Id<Document>,
}

/// The amendments document of a segment after a registration. `created` is
/// false when an earlier registration linked another.
#[derive(Deserialize)]
pub(crate) struct AmendmentsResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub amendments: Id<Document>,
    pub created: ambre_core::Creation,
}

/// `POST /api/channels/{id}/threads`: the caller's new document, to register
/// as the thread of a root message.
#[derive(Serialize)]
pub(crate) struct RegisterThreadRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub root: Id<Message>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub thread: Id<Document>,
}

/// The thread document of a root after a registration. `created` is false
/// when an earlier registration linked another.
#[derive(Deserialize)]
pub(crate) struct ThreadResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub thread: Id<Document>,
    pub created: ambre_core::Creation,
}

/// `POST /api/channels/{id}/segments/{segment}/lease`: the lease, or none
/// and when to ask again.
#[derive(Deserialize)]
pub(crate) struct FinalizeLeaseResponse {
    pub lease: Option<GrantedLeaseResponse>,
    /// Milliseconds before asking again, `0` with a lease.
    pub retry_in: TimeSpan<Ms>,
}

/// A granted finalize lease.
#[derive(Deserialize)]
pub(crate) struct GrantedLeaseResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub lease_id: Id<Lease>,
    /// The last delta the final snapshot covers.
    pub horizon: Seq<Relay>,
    /// Milliseconds before the lease expires.
    pub expires_in: TimeSpan<Ms>,
}

/// `POST /api/channels/{id}/segments/{segment}/finalize`: commit the final
/// snapshot under the lease.
#[derive(Serialize)]
pub(crate) struct FinalizeSegmentRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub lease_id: Id<Lease>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub snapshot_id: Id<Snapshot>,
}

/// `POST /api/channels/{id}/segments/{segment}/fold-lease`: the fold lease,
/// or none and when to ask again.
#[derive(Deserialize)]
pub(crate) struct FoldLeaseResponse {
    pub lease: Option<GrantedFoldResponse>,
    /// Milliseconds before asking again, `0` with a lease.
    pub retry_in: TimeSpan<Ms>,
}

/// A granted fold lease.
#[derive(Deserialize)]
pub(crate) struct GrantedFoldResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub lease_id: Id<Lease>,
    /// The last delta of the segment the fold snapshot covers.
    pub horizon: Seq<Relay>,
    /// The amendments document to fold.
    #[serde(with = "ambre_core::serde_id::text")]
    pub amendments: Id<Document>,
    /// The last delta of the amendments the fold holds at least.
    pub amendments_seq: Seq<Relay>,
    /// Milliseconds before the lease expires.
    pub expires_in: TimeSpan<Ms>,
}

/// `POST /api/channels/{id}/segments/{segment}/fold`: commit the fold
/// snapshot under the lease.
#[derive(Serialize)]
pub(crate) struct FoldSegmentRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub lease_id: Id<Lease>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub snapshot_id: Id<Snapshot>,
    /// The last delta of the amendments the fold holds.
    pub amendments_seq: Seq<Relay>,
}

/// `POST /api/documents/{id}/snapshots/{snapshot}/confirm`: the amendments
/// document the confirmation erased, when the snapshot folds one in.
#[derive(Deserialize)]
pub(crate) struct ConfirmSnapshotResponse {
    #[serde(with = "ambre_core::serde_id::text::option")]
    pub folded: Option<Id<Document>>,
}

/// One member of a direct message, other than the caller, and the channel
/// key wrapped for them.
#[derive(Serialize)]
pub(crate) struct DirectMemberJson {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
}

/// `POST /api/channels/direct`: a direct message of the caller and
/// `members`, which the relay creates or finds.
#[derive(Serialize)]
pub(crate) struct CreateDirectMessageRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub channel_id: Id<Channel>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub meta_id: Id<Meta>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub segment_id: Id<Segment>,
    pub members: Vec<DirectMemberJson>,
    /// The channel key wrapped for the caller.
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// The identifier of the channel key, in hex.
    #[serde(with = "ambre_core::serde_id::text")]
    pub key_id: ambre_core::FixedBytes<ambre_core::KeyId>,
    /// The log of `meta`: its `Create`, one editor `Invite` per member in
    /// the order of `members`, then the caller's own `ChangeRole` to editor.
    pub entries: Vec<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

/// The direct message a request reached: the one it created, or the one that
/// already held its members.
#[derive(Deserialize)]
pub(crate) struct DirectMessageResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub channel_id: Id<Channel>,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only a test reads whether the relay created the direct message"
        )
    )]
    pub created: ambre_core::Creation,
}

/// `POST /api/channels/{id}/join`: the channel key, wrapped for the caller.
#[derive(Serialize)]
pub(crate) struct JoinChannelRequest {
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// The signed `SelfJoin` entry that extends meta's log, absent when the
    /// log is empty (`SYNC-132`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

/// `GET /api/channels/{id}/join`: the end of the log a self-join extends.
#[derive(Deserialize)]
pub(crate) struct JoinHeadResponse {
    /// The document whose log the self-join extends: the channel's meta.
    #[serde(with = "ambre_core::serde_id::text")]
    pub meta_id: Id<Meta>,
    /// The role the relay grants the self-join, which the entry must name.
    pub role: ambre_bus::Role,
    /// The number of entries, the position the self-join takes.
    pub length: ambre_core::Seq<ambre_protocol::LogPlaces>,
    /// The last entry (base64), absent on an empty log.
    #[serde(default)]
    pub last: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

#[derive(Deserialize)]
pub(crate) struct CreateDocumentResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub document_id: Id<Document>,
}

#[derive(Serialize)]
pub(crate) struct RenameDocumentRequest {
    pub name_enc: ambre_core::Encoded<ambre_core::SealedName>,
}

#[derive(Deserialize)]
pub(crate) struct MembersResponse {
    pub members: Vec<MemberEntry>,
    /// The document owes a rotation: a member left and nobody re-keyed since
    /// (`SRV-122`, `SYNC-128`).
    #[serde(default)]
    pub rotation_owed: ambre_core::RotationDebt,
    /// The membership log, oldest entry first (`SRV-104`).
    #[serde(default)]
    pub log: Vec<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

#[derive(Deserialize)]
pub(crate) struct MemberEntry {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    /// A key the relay served malformed costs its own row one use, never the
    /// whole answer (`ServedKey`).
    pub signing_pub_key: ServedKey<ambre_core::PublicKey>,
    pub exchange_pub_key: ServedKey<ambre_core::ExchangePublicKey>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    /// Exchange public key of whoever wrapped `doc_key_enc`. The ECDH unwrap must
    /// use this peer key, not our own `exchange_pub_key`.
    pub wrapper_exchange_pub_key: ServedKey<ambre_core::ExchangePublicKey>,
}

/// Response of `GET /api/documents/{id}/snapshot`: the latest snapshot's id and
/// its S3 key (mirrors the server's `SnapshotResponse`).
#[derive(Deserialize)]
pub(crate) struct SnapshotMetadataResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub snapshot_id: Id<Snapshot>,
    /// Highest delta `seq` folded into the snapshot: the open path gates its
    /// `RequestMissing` on this to skip the already-replayed history.
    pub covered_seq: Seq<Relay>,
    /// The author's signing key, hex. A `self` snapshot of this very user opens
    /// as it is.
    pub author_pubkey: ambre_core::FixedBytes<ambre_core::PublicKey>,
    /// For a `self` snapshot, the latest one another member confirmed, which
    /// the open starts from (`SRV-131`, `AUTHOR-022`).
    #[serde(default)]
    pub reference: Option<ReferenceSnapshot>,
    /// The confirmation another member signed. An open trusts the snapshot
    /// whole only once it verifies (`AUTHOR-030`).
    #[serde(default)]
    pub confirmation: Option<SignedConfirmation>,
}

/// The reference snapshot named beside a `self` one (`SRV-131`), with its
/// author and the confirmation another member signed (`AUTHOR-030`).
#[derive(Deserialize)]
pub(crate) struct ReferenceSnapshot {
    #[serde(with = "ambre_core::serde_id::text")]
    pub snapshot_id: Id<Snapshot>,
    pub covered_seq: Seq<Relay>,
    pub author_pubkey: ambre_core::FixedBytes<ambre_core::PublicKey>,
    #[serde(default)]
    pub confirmation: Option<SignedConfirmation>,
}

pub(crate) use ambre_protocol::snapshot_confirm::SignedConfirmation;

/// Body of `POST /api/documents/{id}/snapshots/{snapshot_id}/confirm`: the
/// confirmer's signature over the confirmation transcript, and the digest of
/// the blob the transcript names, hex (`AUTHOR-031`).
#[derive(Serialize)]
pub(crate) struct ConfirmSnapshotRequest {
    pub signature: ambre_core::FixedBytes<ambre_core::Signature>,
    pub blob: ambre_core::FixedBytes<ambre_core::SnapshotDigest>,
}

/// Body of `POST /api/documents/{id}/snapshots/{snapshot_id}/report`: the
/// reporter's signature over the report transcript, hex (`AUTHOR-023`).
#[derive(Serialize)]
pub(crate) struct ReportSnapshotRequest {
    pub signature: ambre_core::FixedBytes<ambre_core::Signature>,
}

/// Response of `POST /api/documents/{id}/snapshot/upload-id`: the id the
/// relay mints for a snapshot, which the blob seals before its digest is
/// known (`SYNC-258`, `SRV-310`).
#[derive(Deserialize)]
pub(crate) struct SnapshotUploadIdResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub snapshot_id: Id<Snapshot>,
}

/// Body of `POST /api/documents/{id}/snapshot/upload-url`: the size of the
/// sealed blob, the id it seals, and its SHA-256 digest, which the link signs
/// (`SRV-310`).
#[derive(Serialize)]
pub(crate) struct SnapshotUploadUrlRequest {
    #[serde(rename = "size_bytes")]
    pub size: ambre_core::Count<ambre_core::Octets>,
    #[serde(with = "ambre_core::serde_id::text")]
    pub snapshot_id: Id<Snapshot>,
    #[serde(rename = "sha256")]
    pub checksum: ambre_core::FixedBytes<ambre_core::ObjectChecksum>,
}

/// Response of `POST /api/documents/{id}/snapshot/upload-url`. It holds a
/// presigned PUT URL and the S3 key the confirmation names. The id the blob
/// seals came from `upload-id` already (`SYNC-258`).
#[derive(Deserialize)]
pub(crate) struct SnapshotUploadUrlResponse {
    pub upload_url: ambre_core::Credential<ambre_core::PresignedUrl>,
    pub s3_key: ambre_core::ObjectKey,
}

/// Body of `POST /api/documents/{id}/snapshot` (confirm upload + insert metadata).
/// Mirrors the server's `CreateSnapshotRequest`. It names no snapshot id: the
/// client learns the one the relay mints from the response (`SYNC-182`).
#[derive(Serialize)]
pub(crate) struct CreateSnapshotRequest {
    pub author_pubkey: ambre_core::FixedBytes<ambre_core::PublicKey>,
    pub s3_key: ambre_core::ObjectKey,
    /// Highest delta `seq` this client had applied when it took the snapshot.
    pub covered_seq: Seq<Relay>,
}

#[derive(Deserialize)]
pub(crate) struct CreateSnapshotResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub snapshot_id: Id<Snapshot>,
}

/// Body of `POST /api/documents/{id}/snapshot/download-url`.
#[derive(Serialize)]
pub(crate) struct SnapshotDownloadUrlRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub snapshot_id: Id<Snapshot>,
}

#[derive(Deserialize)]
pub(crate) struct SnapshotDownloadUrlResponse {
    pub download_url: ambre_core::Credential<ambre_core::PresignedUrl>,
}

#[derive(Deserialize)]
pub(crate) struct UserSearchResult {
    pub users: Vec<UserInfoEntry>,
}

#[derive(Deserialize)]
pub(crate) struct UserInfoEntry {
    #[serde(with = "ambre_core::serde_id::text")]
    pub id: Id<User>,
    pub email: ambre_core::Email,
    /// Whether a provider proved the address. A relay that sends no flag
    /// reads as an unproven address, the safer side (`CHROME-138`).
    #[serde(rename = "email_verified", default)]
    pub email_proof: ambre_core::EmailProof,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    /// The key an invitation's entry names for the invitee (`SYNC-132`).
    pub signing_pubkey: ambre_core::FixedBytes<ambre_core::PublicKey>,
    pub exchange_pubkey: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
}

#[derive(Deserialize)]
pub(crate) struct PubkeysResponse {
    pub signing_pubkey: ambre_core::FixedBytes<ambre_core::PublicKey>,
    pub exchange_pubkey: ambre_core::FixedBytes<ambre_core::ExchangePublicKey>,
}

#[derive(Serialize)]
pub(crate) struct InviteRequest {
    /// One of the two names the invitee (`SRV-224`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invitee_email: Option<ambre_core::Email>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "ambre_core::serde_id::text::option"
    )]
    pub invitee_id: Option<Id<User>>,
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// The key `doc_key_enc` wraps, in hex (`SYNC-143`).
    #[serde(with = "ambre_core::serde_id::text")]
    pub key_id: ambre_core::FixedBytes<ambre_core::KeyId>,
    /// The entry the inviter signs for this invitation (`SYNC-132`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

/// The body of a departure: the entry the leaver signs (`SYNC-132`).
#[derive(Serialize)]
pub(crate) struct LeaveRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

/// The body of a role change: the role and the entry the owner signs for it
/// (`SYNC-132`).
#[derive(Serialize)]
pub(crate) struct RoleChangeRequest {
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

#[derive(Deserialize)]
pub(crate) struct InviteResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
}

#[derive(Serialize)]
pub(crate) struct RotateKeyRequest {
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the rotation read, wrapped for or not (`SYNC-143`).
    #[serde(with = "ambre_core::serde_id::text::vec")]
    pub members: Vec<Id<User>>,
    /// The link to the key this rotation replaces, as [`encode_link`] writes
    /// it (`SYNC-118`).
    pub link: ambre_core::Encoded<ambre_core::EncodedKeyLink>,
}

/// One revoke: the member removed and the new key wrapped for everyone who
/// stays, applied by the relay in one transaction (`SYNC-126`).
#[derive(Serialize)]
pub(crate) struct RevokeRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub keys: Vec<RotateKeyEntry>,
    /// Every member the revoke read and keeps, wrapped for or not (`SYNC-143`).
    #[serde(with = "ambre_core::serde_id::text::vec")]
    pub members: Vec<Id<User>>,
    /// The link to the key this revoke replaces (`SYNC-118`).
    pub link: ambre_core::Encoded<ambre_core::EncodedKeyLink>,
    /// The entry the owner signs for this revoke (`SYNC-132`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

/// The links of one document's chain, oldest first (`SYNC-117`).
#[derive(Deserialize)]
pub(crate) struct KeyLinksResponse {
    pub links: Vec<ambre_core::Encoded<ambre_core::EncodedKeyLink>>,
}

/// The form a link takes in a request: its postcard bytes, in base64.
pub(crate) fn encode_link(
    link: &ambre_protocol::KeyLink,
) -> Result<
    ambre_core::Encoded<ambre_core::EncodedKeyLink>,
    ambre_protocol::error::SerializationError,
> {
    link.encode()
}

/// The links a response carries. One that does not decode is dropped: the
/// walk then stops before it, and the keys past it stay out of reach.
pub(crate) fn decode_links(resp: KeyLinksResponse) -> Vec<ambre_protocol::KeyLink> {
    resp.links
        .iter()
        .filter_map(|text| ambre_protocol::KeyLink::decode(text).ok())
        .collect()
}

/// The links of `document_id`, or none when the relay cannot serve them: the
/// frames sealed under a key older than the cache then stay closed.
pub(crate) async fn fetch_key_links(
    url_scheme: crate::http::UrlScheme,
    api_base: &crate::http::Url<crate::http::RelayHost>,
    document_id: Id<Document>,
    auth: AuthContext,
) -> Vec<ambre_protocol::KeyLink> {
    match try_fetch_key_links(url_scheme, api_base, document_id, auth).await {
        Ok(links) => links,
        Err(e) => {
            crate::log::warning!("Sync: key links of {document_id:?} unavailable: {e}");
            Vec::new()
        }
    }
}

/// [`fetch_key_links`], with the failure of the request kept. A check that
/// needs the links tells a relay out of reach apart from a link it lacks
/// (`SYNC-200`).
pub(crate) async fn try_fetch_key_links(
    url_scheme: crate::http::UrlScheme,
    api_base: &crate::http::Url<crate::http::RelayHost>,
    document_id: Id<Document>,
    auth: AuthContext,
) -> Result<Vec<ambre_protocol::KeyLink>, HttpError> {
    try_fetch_key_links_via(&Fetch, url_scheme, api_base, document_id, &auth).await
}

/// [`try_fetch_key_links`] over `transport`, which a host test scripts.
pub(crate) async fn try_fetch_key_links_via(
    transport: &impl Transport,
    url_scheme: crate::http::UrlScheme,
    api_base: &crate::http::Url<crate::http::RelayHost>,
    document_id: Id<Document>,
    auth: &AuthContext,
) -> Result<Vec<ambre_protocol::KeyLink>, HttpError> {
    let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, api_base)
        .route(format!("/documents/{document_id}/key-links"));
    get_json_auth_via::<KeyLinksResponse>(transport, &url, auth)
        .await
        .map(decode_links)
}

#[derive(Serialize)]
pub(crate) struct RotateKeyEntry {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
}

#[derive(Deserialize)]
pub(crate) struct RotateKeyResponse {
    pub status: ambre_core::RotationStatus,
}

/// The answer of a relay write that returns its outcome (`SRV-181`). A field
/// beside the status, such as a confirmed media's hash, is not read.
#[derive(Debug, Deserialize)]
pub(crate) struct WriteAnswer {
    pub status: ambre_core::WriteStatus,
}

impl WriteAnswer {
    /// Whether the relay answered the outcome of the write it was asked.
    /// Another word of the set answers some other write.
    pub(crate) fn expect(self, asked: ambre_core::WriteStatus) -> Result<(), HttpError> {
        if self.status == asked {
            Ok(())
        } else {
            crate::log::warning!("Sync: answered {:?}, not {asked:?}", self.status);
            Err(HttpError::Local(LocalFailure::UnexpectedAnswer))
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct DocumentMembersResponse {
    /// Every row the relay serves. An unknown role or a malformed id refuses
    /// the whole answer (`SRV-180`, `SYNC-038`). A malformed key refuses
    /// nothing here: each reader of the key meets it (`ServedKey`).
    pub members: Vec<DocumentMemberEntry>,
    /// The document owes a rotation (`SRV-122`, `SYNC-128`).
    #[serde(default)]
    pub rotation_owed: ambre_core::RotationDebt,
    /// The membership log, oldest entry first (`SRV-104`).
    #[serde(default)]
    pub log: Vec<ambre_core::Encoded<ambre_core::EncodedLogEntry>>,
}

/// A key as the relay serves it on one member row, read without refusing the
/// row. `None` when the text is not the hexadecimal of the key's width.
///
/// A member row carries three keys, and each has its own reader. The presence
/// directory skips a row whose signing key does not parse (`SYNC-015`). A
/// rotation reports a member whose exchange key does not parse by name
/// (`SYNC-043`). A malformed key thus costs its own row one use, never the
/// whole answer.
pub(crate) struct ServedKey<R: FixedRole>(Option<ambre_core::FixedBytes<R>>);

impl<R: FixedRole> ServedKey<R> {
    /// A key that parsed, as a test row holds it.
    #[cfg(test)]
    pub(crate) fn of(key: ambre_core::FixedBytes<R>) -> Self {
        Self(Some(key))
    }

    /// The key, or `None` when the relay served text that spells none.
    pub(crate) fn key(&self) -> Option<&ambre_core::FixedBytes<R>> {
        self.0.as_ref()
    }
}

impl<'de, R: FixedRole> Deserialize<'de> for ServedKey<R> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self(ambre_core::FixedBytes::from_hex(&text).ok()))
    }
}

#[derive(Deserialize)]
pub(crate) struct DocumentMemberEntry {
    #[serde(with = "ambre_core::serde_id::text")]
    pub user_id: Id<User>,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    pub email: ambre_core::Email,
    /// Ed25519 verifying key: feeds the presence directory (`presence_directory`).
    pub signing_pub_key: ServedKey<ambre_core::PublicKey>,
    pub exchange_pub_key: ServedKey<ambre_core::ExchangePublicKey>,
    pub doc_key_enc: ambre_core::Encoded<ambre_core::WrappedDocKey>,
    /// The relay's column, which the signed log overrides (`SYNC-110`). An
    /// unknown word refuses the whole answer when it is read (`SRV-180`).
    #[serde(with = "ambre_core::serde_enums::role")]
    pub role: ambre_core::Role,
    /// Exchange key of whoever wrapped `doc_key_enc`. Our own row unwraps
    /// against it, which yields the log anchor (`SYNC-104`).
    pub wrapper_exchange_pub_key: ServedKey<ambre_core::ExchangePublicKey>,
}

/// Asks for a link to put one sealed blob. It names the keyed address and the
/// sealed size, and nothing else of the blob (`SYNC-053`).
#[derive(Serialize)]
pub(crate) struct UploadUrlRequest {
    pub content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    #[serde(rename = "size_bytes")]
    pub size: ambre_core::Count<ambre_core::Octets>,
}

#[derive(Deserialize)]
pub(crate) struct UploadUrlResponse {
    pub upload_url: ambre_core::Credential<ambre_core::PresignedUrl>,
}

#[derive(Serialize)]
pub(crate) struct DownloadUrlRequest {
    pub content_hash: ambre_core::Validated<ambre_core::ContentHash>,
}

#[derive(Deserialize)]
pub(crate) struct DownloadUrlResponse {
    pub download_url: ambre_core::Credential<ambre_core::PresignedUrl>,
}

/// Confirms one put. The same two fields as [`UploadUrlRequest`] (`SYNC-053`).
#[derive(Serialize)]
pub(crate) struct ConfirmUploadRequest {
    pub content_hash: ambre_core::Validated<ambre_core::ContentHash>,
    #[serde(rename = "size_bytes")]
    pub size: ambre_core::Count<ambre_core::Octets>,
}

/// A transport that answers from a script and keeps every request it was
/// handed, for the host tests of the request policy.
#[cfg(test)]
pub(crate) mod scripted {
    use super::*;
    use ambre_core::{Ms, TimeSpan};
    use std::cell::Cell;
    use std::collections::VecDeque;

    /// Answers each request with the next scripted answer. A request past the
    /// script fails at once, or never answers once `silent` is set.
    #[derive(Default)]
    pub(crate) struct Scripted {
        answers: RefCell<VecDeque<Result<Incoming, String>>>,
        /// Every request handed over, in order.
        pub sent: RefCell<Vec<Outgoing>>,
        /// Whether a request past the script hangs instead of failing.
        pub silent: Cell<bool>,
        /// Every deadline a silent transport waited out, in milliseconds.
        pub waited: RefCell<Vec<TimeSpan<Ms>>>,
    }

    impl Scripted {
        /// A transport answering with `answers`, as a status and a JSON body,
        /// or as no answer at all.
        pub(crate) fn new(
            answers: impl IntoIterator<Item = Result<(u16, serde_json::Value), String>>,
        ) -> Self {
            let answers = answers
                .into_iter()
                .map(|answer| {
                    answer.map(|(status, body)| Incoming {
                        status: HttpStatus::new(status),
                        body: Payload::<HttpBody>::new(serde_json::to_vec(&body).unwrap()),
                    })
                })
                .collect();
            Self {
                answers: RefCell::new(answers),
                ..Self::default()
            }
        }

        /// The method and the URL of each request handed over.
        pub(crate) fn calls(&self) -> Vec<(Method, String)> {
            self.sent
                .borrow()
                .iter()
                .map(|r| (r.method, r.url.reveal().to_owned()))
                .collect()
        }
    }

    impl Transport for Scripted {
        async fn send(&self, request: &Outgoing) -> Result<Incoming, TransportError> {
            self.sent.borrow_mut().push(request.clone());
            let answer = self.answers.borrow_mut().pop_front();
            match answer {
                Some(answer) => answer.map_err(TransportError::Fetch),
                None if self.silent.get() => std::future::pending().await,
                None => Err(TransportError::Fetch("no answer scripted".to_owned())),
            }
        }

        /// A silent transport's clock jumps straight to the deadline. Any
        /// other never reaches it, so a scripted answer always wins.
        async fn elapse(&self, span: TimeSpan<Ms>) {
            if !self.silent.get() {
                return std::future::pending().await;
            }
            self.waited.borrow_mut().push(span);
        }
    }

    /// A session holding the access token `access` and the refresh token
    /// `refresh`, against `https://relay.test`.
    pub(crate) fn session(access: &str, refresh: &str) -> AuthContext {
        AuthContext {
            api_base: crate::http::Url::<crate::http::RelayHost>::new("relay.test".to_owned()),
            url_scheme: UrlScheme::Https,
            tokens: Rc::new(RefCell::new(Some(TokenPair::new(
                Credential::new(access.to_owned()),
                Credential::new(refresh.to_owned()),
                TimeSpan::<ambre_core::Sec>::new(900),
            )))),
            renewing: Rc::new(futures_util::lock::Mutex::new(())),
            renewed: None,
        }
    }

    /// A fresh token pair as the relay's refresh route answers it.
    pub(crate) fn renewed(access: &str) -> serde_json::Value {
        serde_json::json!({
            "access_token": access,
            "refresh_token": "refresh-2",
            "access_expires_in": 900,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::scripted::{Scripted, renewed, session};
    use super::*;

    /// A URL prints its kind and its text, and a failure its text alone.
    #[test]
    fn a_url_and_a_failure_print_what_a_log_reads() {
        let url = Url::<RelayHost>::new("https://relay.test".to_owned());
        assert_eq!(format!("{url:?}"), r#"RelayHost("https://relay.test")"#);
        assert_eq!(
            Failed::local(ambre_core::LocalFailure::NoRelay).to_string(),
            "no relay is reachable from this session"
        );
    }

    /// `SYNC-277`, audit `W206`: a meter takes bytes up to its ceiling, and
    /// refuses the chunk that passes it, before the chunk is kept.
    #[test]
    fn a_body_meter_refuses_the_chunk_past_its_ceiling() {
        let ceiling = ambre_core::Count::<ambre_core::Octets>::new(10);
        let mut meter = BodyMeter::new(ceiling);
        assert!(meter.take(ambre_core::Count::new(6)).is_ok());
        assert!(
            meter.take(ambre_core::Count::new(4)).is_ok(),
            "the ceiling itself"
        );
        assert!(matches!(
            meter.take(ambre_core::Count::new(1)),
            Err(TransportError::Oversized(c)) if c == ceiling
        ));
    }

    /// `SYNC-277`, audit `W206`: an answer whose body passes the ceiling of
    /// its request is no answer, whatever transport read it. An object
    /// request holds a larger ceiling than a relay answer.
    #[tokio::test]
    async fn an_answer_past_its_ceiling_is_no_answer() {
        let large = "a".repeat(17 * 1024 * 1024);
        let transport = Scripted::new([
            Ok((200, serde_json::json!(large.clone()))),
            Ok((200, serde_json::json!(large))),
        ]);
        let answer = Outgoing::new(Method::Get, Target::relay("https://relay.test/api/x"));
        assert!(matches!(
            exchange(&transport, &answer).await,
            Err(HttpError::NoAnswer {
                reason: TransportError::Oversized(_),
                ..
            })
        ));
        let object = Outgoing {
            ceiling: OBJECT_CEILING,
            ..answer
        };
        assert!(exchange(&transport, &object).await.is_ok());
    }

    /// The refusal of a `GET` or a `POST`, as the relay's answer builds it.
    fn refusal(method: &str, url: &str, status: u16, code: Option<RefusalCode>) -> HttpError {
        let method = if method == "GET" {
            Method::Get
        } else {
            Method::Post
        };
        HttpError::refused(method, &Target::relay(url), HttpStatus::new(status), code)
    }

    /// A relay on this machine speaks plain HTTP, and any other relay TLS, so
    /// a remote relay never sees a request in the clear. Two addresses are
    /// one address only when their text is.
    #[test]
    fn only_a_relay_on_this_machine_speaks_plain_http() {
        let scheme = |host: &str| UrlScheme::of(&Url::<RelayHost>::new(host.to_owned()));
        assert_eq!(scheme("localhost:3000"), UrlScheme::Http);
        assert_eq!(scheme("127.0.0.1:3000"), UrlScheme::Http);
        assert_eq!(scheme("relay.example"), UrlScheme::Https);
        let host = Url::<RelayHost>::new("relay.example".to_owned());
        assert!(host == host.clone());
        assert!(host != Url::<RelayHost>::new("other.example".to_owned()));
    }

    /// `SYNC-271`: the protocol of the page picks the scheme. A stack served
    /// over TLS on this machine is reached over TLS. A page served in the
    /// clear on another host is reached in the clear.
    #[test]
    fn the_scheme_of_the_page_wins_over_the_host() {
        let page = |scheme: Option<UrlScheme>, host: &str| {
            UrlScheme::of_page(scheme, &Url::<RelayHost>::new(host.to_owned()))
        };
        assert_eq!(
            page(Some(UrlScheme::Https), "localhost:8443"),
            UrlScheme::Https
        );
        assert_eq!(
            page(Some(UrlScheme::Https), "127.0.0.1:8443"),
            UrlScheme::Https
        );
        assert_eq!(
            page(Some(UrlScheme::Http), "192.168.1.20:8080"),
            UrlScheme::Http
        );
        assert_eq!(page(None, "localhost:3000"), UrlScheme::Http);
        assert_eq!(page(None, "relay.example"), UrlScheme::Https);
    }

    /// An answer body becomes a fetched blob with every byte kept, and each
    /// prints its role and its length alone.
    #[test]
    fn an_answer_body_becomes_a_fetched_blob() {
        let body = Payload::<HttpBody>::new(b"token".to_vec());
        assert_eq!(format!("{body:?}"), "HttpBody(5 bytes)");
        let blob: Payload<crate::pipeline::FetchedBlob> = body.into_role();
        assert_eq!(blob, b"token");
        assert_eq!(format!("{blob:?}"), "FetchedBlob(5 bytes)");
    }

    /// The relay's JSON `bool` reads as a [`ambre_core::Creation`] and an
    /// `SYNC-181`: the client reads the number the relay sends, on the
    /// relay's clock. The relay half is `ambre_server::routes::tests::
    /// handler_tests::a_listed_document_sends_its_update_time_as_relay_milliseconds`,
    /// which pins the same number.
    #[test]
    fn a_listed_document_reads_its_update_time_unchanged() {
        let document = Id::<Document>::mint();
        let listed: DocumentListResponse = serde_json::from_value(serde_json::json!({
            "documents": [{
                "id": document.to_string(), "doc_type": "prose", "role": "owner",
                "updated_at": 1_767_312_245_123_u64,
            }],
        }))
        .expect("a listing");
        let entry = listed.documents.first().expect("one entry");
        assert_eq!(
            entry.updated_at,
            ambre_core::Instant::<ambre_core::Relayed>::new(1_767_312_245_123)
        );
        let text: Result<DocumentListResponse, _> = serde_json::from_value(serde_json::json!({
            "documents": [{
                "id": document.to_string(), "doc_type": "prose", "role": "owner",
                "updated_at": "2026-01-02T00:04:05.123+00:00",
            }],
        }));
        assert!(
            text.is_err(),
            "a text time is refused, never read as the epoch"
        );
    }

    /// [`ambre_core::activity::Activity`]: the answers keep the shape the relay writes.
    #[test]
    fn a_relay_bool_reads_as_its_flag() {
        use ambre_core::{Creation, activity::Activity};
        let segment = Id::<Segment>::mint();
        let document = Id::<Document>::mint();
        let channel = Id::<Channel>::mint();
        let closed: ClosedSegmentResponse = serde_json::from_value(serde_json::json!({
            "active": segment.to_string(), "created": true,
        }))
        .expect("a close answer");
        assert_eq!(closed.created, Creation::Created);
        let amended: AmendmentsResponse = serde_json::from_value(serde_json::json!({
            "amendments": document.to_string(), "created": false,
        }))
        .expect("an amendments answer");
        assert_eq!(amended.created, Creation::Existing);
        let thread: ThreadResponse = serde_json::from_value(serde_json::json!({
            "thread": document.to_string(), "created": true,
        }))
        .expect("a thread answer");
        assert_eq!(thread.created, Creation::Created);
        let direct: DirectMessageResponse = serde_json::from_value(serde_json::json!({
            "channel_id": channel.to_string(), "created": false,
        }))
        .expect("a direct message answer");
        assert_eq!(direct.created, Creation::Existing);
        let entry: ActivityEntry = serde_json::from_value(serde_json::json!({
            "channel_id": channel.to_string(), "document_id": document.to_string(),
            "kind": "segment", "active": true,
        }))
        .expect("an activity entry");
        assert_eq!(entry.active, Activity::Active);
    }

    /// `SYNC-179`: a query value reaches the relay as the text it was, each
    /// reserved character, the percent sign, a space and a non-ASCII letter
    /// encoded.
    #[test]
    fn a_query_value_is_percent_encoded_whole() {
        let url = Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/users/search")
        .query(QueryKey::Query, "a&b=c#d+e%f g@h-\u{e9}~");
        assert_eq!(
            url.as_str(),
            "https://relay.test/api/users/search?query=a%26b%3Dc%23d%2Be%25f%20g%40h-%C3%A9~"
        );
    }

    /// `SYNC-292`: an error on a search shows the route, never the address
    /// the query searched, encoded or not.
    #[test]
    fn an_error_on_a_search_shows_no_searched_address() {
        let url = Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/users/search")
        .query(QueryKey::Query, "ada@example.org");
        let target = Target::from(&url);
        let refused = HttpError::refused(Method::Get, &target, HttpStatus::new(500), None);
        for shown in [refused.to_string(), target.shown().to_string()] {
            assert!(shown.contains("/api/users/search"), "{shown}");
            for leak in ["ada", "example.org", "%40", "query="] {
                assert!(!shown.contains(leak), "{shown} shows {leak}");
            }
        }
    }

    fn is_setup_refusal(error: &HttpError) -> bool {
        error.is(RefusalCode::SetupRequired)
    }

    fn is_forbidden(error: &HttpError) -> bool {
        error.is(RefusalCode::Forbidden)
    }

    fn is_members_changed(error: &HttpError) -> bool {
        error.is(RefusalCode::MembersChanged)
    }

    fn is_key_changed(error: &HttpError) -> bool {
        error.is(RefusalCode::KeyChanged)
    }

    fn is_log_moved(error: &HttpError) -> bool {
        error.is(RefusalCode::LogMoved)
    }

    fn is_conflict(error: &HttpError) -> bool {
        error.is_conflict()
    }

    fn is_unknown_word(error: &HttpError) -> bool {
        error.failure()
            == ApiFailure::Unreadable {
                cause: UnreadableCause::UnknownWord,
            }
    }

    /// The `Authorization` value a request carried, if any.
    fn bearer_of(request: &Outgoing) -> Option<String> {
        request
            .bearer
            .as_ref()
            .map(|token| token.present_bearer(str::to_owned))
    }

    /// A route the token travels to is built from the session's own API
    /// root, so it names the relay and nothing else. A presigned storage link
    /// is a `&str`, and no `*_auth` helper takes one.
    /// A refusal keeps the relay's status and code as values, and reads as the
    /// text it read before. A code the status does not carry is dropped.
    #[test]
    fn a_refusal_carries_its_status_and_code() {
        let url = "https://relay.example/api/documents/d/share";
        let changed = HttpError::refused(
            Method::Post,
            &Target::relay(url),
            HttpStatus::new(409),
            Some(RefusalCode::KeyChanged),
        );
        assert_eq!(
            changed.failure(),
            ApiFailure::Relay {
                status: HttpStatus::new(409),
                code: Some(RefusalCode::KeyChanged),
            }
        );
        assert_eq!(changed.to_string(), format!("POST {url}: KEY_CHANGED"));
        let plain = HttpError::refused(
            Method::Post,
            &Target::relay(url),
            HttpStatus::new(500),
            Some(RefusalCode::KeyChanged),
        );
        assert_eq!(plain.code(), None);
        assert_eq!(plain.failure().status(), Some(HttpStatus::new(500)));
        assert_eq!(plain.to_string(), format!("POST {url}: HTTP 500"));
        assert_eq!(
            HttpError::Local(ambre_core::LocalFailure::NoRelay).failure(),
            ApiFailure::Local(ambre_core::LocalFailure::NoRelay)
        );
    }

    #[tokio::test]
    async fn a_relay_url_is_built_from_the_api_root() {
        let auth = session("access-1", "refresh-1");
        let url = auth.api_root().route("/documents/d").join("/members");
        assert_eq!(url.as_str(), "https://relay.test/api/documents/d/members");
        assert_eq!(url.to_string(), url.as_str());

        let transport = Scripted::new([Ok((200, serde_json::json!({ "ok": 1 })))]);
        let _: serde_json::Value = get_json_auth_via(&transport, &url, &auth).await.unwrap();
        let sent = transport.sent.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].url.reveal(), url.as_str());
        assert_eq!(bearer_of(&sent[0]).as_deref(), Some("Bearer access-1"));
    }

    /// `SYNC-062`: a request refused for an expired token renews the pair
    /// once, with the refresh token alone. It then replays the same request
    /// under the new token. Every request carries the per-request timeout.
    #[tokio::test]
    async fn a_refused_token_is_renewed_and_the_request_replayed_once() {
        let transport = Scripted::new([
            Ok((401, serde_json::json!({}))),
            Ok((200, renewed("access-2"))),
            Ok((200, serde_json::json!({ "ok": 1 }))),
        ]);
        let auth = session("access-1", "refresh-1");
        let url = &Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents");

        let answer: serde_json::Value =
            post_json_auth_via(&transport, url, &serde_json::json!({ "n": 7 }), &auth)
                .await
                .unwrap();

        assert_eq!(answer, serde_json::json!({ "ok": 1 }));
        let sent = transport.sent.borrow();
        let [first, renewal, replay] = sent.as_slice() else {
            panic!("expected three requests, got {}", sent.len());
        };
        assert_eq!(bearer_of(first).as_deref(), Some("Bearer access-1"));
        assert_eq!(renewal.url.reveal(), "https://relay.test/api/auth/refresh");
        assert_eq!(
            bearer_of(renewal),
            None,
            "the renewal carries no access token"
        );
        let asked: serde_json::Value =
            serde_json::from_slice(renewal.body.as_ref().unwrap().1.as_bytes()).unwrap();
        assert_eq!(asked, serde_json::json!({ "refresh_token": "refresh-1" }));
        assert_eq!(bearer_of(replay).as_deref(), Some("Bearer access-2"));
        assert_eq!(
            (replay.method, replay.url.reveal(), &replay.body),
            (first.method, first.url.reveal(), &first.body)
        );
        assert!(sent.iter().all(|r| r.timeout == REQUEST_TIMEOUT));
    }

    /// `SYNC-177`: a sign-out revokes the refresh token at the relay, under
    /// the access token. After a renewal it revokes the token the renewal
    /// handed out, not the one it replaced.
    #[tokio::test]
    async fn a_sign_out_revokes_the_refresh_token_it_holds() {
        let body_of = |request: &Outgoing| -> serde_json::Value {
            serde_json::from_slice(request.body.as_ref().unwrap().1.as_bytes()).unwrap()
        };
        let logout = "https://relay.test/api/auth/logout";

        let transport = Scripted::new([Ok((200, serde_json::json!({})))]);
        session("access-1", "refresh-1")
            .revoke_via(&transport)
            .await
            .unwrap();
        let sent = transport.sent.take();
        let [only] = sent.as_slice() else {
            panic!("expected one request, got {}", sent.len());
        };
        assert_eq!((only.method, only.url.reveal()), (Method::Post, logout));
        assert_eq!(bearer_of(only).as_deref(), Some("Bearer access-1"));
        assert_eq!(
            body_of(only),
            serde_json::json!({ "refresh_token": "refresh-1" })
        );

        let transport = Scripted::new([
            Ok((401, serde_json::json!({}))),
            Ok((200, renewed("access-2"))),
            Ok((200, serde_json::json!({}))),
        ]);
        session("access-1", "refresh-1")
            .revoke_via(&transport)
            .await
            .unwrap();
        let sent = transport.sent.take();
        let [_, _, replay] = sent.as_slice() else {
            panic!("expected three requests, got {}", sent.len());
        };
        assert_eq!(replay.url.reveal(), logout);
        assert_eq!(bearer_of(replay).as_deref(), Some("Bearer access-2"));
        assert_eq!(
            body_of(replay),
            serde_json::json!({ "refresh_token": "refresh-2" })
        );

        let refused = Scripted::new([Ok((500, serde_json::json!({})))]);
        assert!(
            session("a", "r").revoke_via(&refused).await.is_err(),
            "a refusal is reported"
        );
    }

    /// `SYNC-062`: the replay is the only one. A second refusal, or a renewal
    /// the relay refuses, ends the request with an error.
    #[tokio::test]
    async fn a_request_is_replayed_at_most_once() {
        let url = &Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents");
        let refused_twice = Scripted::new([
            Ok((401, serde_json::json!({}))),
            Ok((200, renewed("access-2"))),
            Ok((401, serde_json::json!({}))),
        ]);
        let auth = session("access-1", "refresh-1");
        let err = get_json_auth_via::<serde_json::Value>(&refused_twice, url, &auth)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), format!("GET {url}: HTTP 401"));
        assert_eq!(refused_twice.sent.borrow().len(), 3);

        let renewal_refused = Scripted::new([
            Ok((401, serde_json::json!({}))),
            Ok((401, serde_json::json!({}))),
        ]);
        let auth = session("access-1", "refresh-1");
        let err = get_json_auth_via::<serde_json::Value>(&renewal_refused, url, &auth)
            .await
            .unwrap_err();
        assert!(
            err.to_string().ends_with("/api/auth/refresh: HTTP 401"),
            "{err}"
        );
        assert_eq!(
            renewal_refused.sent.borrow().len(),
            2,
            "no replay without a token"
        );
    }

    /// The links of a chain read in order, and one that does not decode is
    /// dropped: the walk stops before it.
    #[test]
    fn the_links_of_a_chain_read_in_order_and_drop_an_unreadable_one() {
        let document_id = Id::<Document>::mint();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let keys: Vec<_> = (0..3)
            .map(|_| ambre_crypto::DocumentKey::generate())
            .collect();
        let links: Vec<_> = keys
            .windows(2)
            .map(|pair| {
                crate::pipeline::seal_link(
                    document_id,
                    &pair[0],
                    &pair[1],
                    ambre_protocol::LogHead::EMPTY,
                    &signing_key,
                )
                .expect("seal")
            })
            .collect();
        let mut texts: Vec<_> = links
            .iter()
            .map(|l| encode_link(l).expect("encode"))
            .collect();
        texts.insert(1, ambre_core::Encoded::new(b"not a link".to_vec()));
        let read = decode_links(KeyLinksResponse { links: texts });
        assert_eq!(read, links);
    }

    /// An upload to a presigned link carries the sealed bytes whole, as
    /// octets, and nothing else (`SYNC-049`).
    #[tokio::test]
    async fn an_upload_carries_the_sealed_blob_as_its_body() {
        let link = ambre_core::Credential::<ambre_core::PresignedUrl>::new(
            "https://store.test/bucket/object".to_owned(),
        );
        let blob = crate::pipeline::object_blob(
            &ambre_core::Payload::new(b"media".to_vec()),
            &ambre_core::Aad::of(b"aad"),
            &ambre_crypto::DocumentKey::generate(),
            &ed25519_dalek::SigningKey::from_bytes(&[7; 32]),
        )
        .expect("seal");
        let store = Scripted::new([Ok((200, serde_json::json!({})))]);
        put_sealed_via(&store, &link, &blob).await.expect("stored");
        let sent = store.sent.borrow();
        let [request] = sent.as_slice() else {
            panic!("one request");
        };
        assert_eq!(request.method, Method::Put);
        assert!(request.bearer.is_none(), "the link is its own credential");
        let (kind, body) = request.body.as_ref().expect("a body");
        assert_eq!(*kind, ContentType::Octets);
        assert_eq!(body.as_bytes(), blob.as_bytes());
    }

    /// A transport that yields once before each answer, as a network does,
    /// so two requests in flight interleave.
    struct Yielding(Scripted);

    impl Transport for Yielding {
        async fn send(&self, request: &Outgoing) -> Result<Incoming, TransportError> {
            tokio::task::yield_now().await;
            self.0.send(request).await
        }

        async fn elapse(&self, span: TimeSpan<Ms>) {
            self.0.elapse(span).await;
        }
    }

    /// The bodies of the renewals `transport` sent, in order.
    fn renewals(transport: &Scripted) -> Vec<serde_json::Value> {
        transport
            .sent
            .borrow()
            .iter()
            .filter(|r| r.url.reveal().ends_with("/api/auth/refresh"))
            .map(|r| serde_json::from_slice(r.body.as_ref().unwrap().1.as_bytes()).unwrap())
            .collect()
    }

    /// `SYNC-184`: two requests refused for one expired token renew the pair
    /// once. The second waits for the renewal in flight and replays under its
    /// token, where a renewal of its own would reuse a spent refresh token.
    #[tokio::test]
    async fn two_refusals_of_one_token_renew_the_pair_once() {
        let transport = Yielding(Scripted::new([
            Ok((401, serde_json::json!({}))),
            Ok((401, serde_json::json!({}))),
            Ok((200, renewed("access-2"))),
            Ok((200, serde_json::json!({ "ok": 1 }))),
            Ok((200, serde_json::json!({ "ok": 2 }))),
        ]));
        let auth = session("access-1", "refresh-1");
        let url = &Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents");

        let (first, second) = futures_util::future::join(
            get_json_auth_via::<serde_json::Value>(&transport, url, &auth),
            get_json_auth_via::<serde_json::Value>(&transport, url, &auth),
        )
        .await;

        assert!(first.is_ok() && second.is_ok(), "{first:?} {second:?}");
        assert_eq!(
            renewals(&transport.0),
            [serde_json::json!({ "refresh_token": "refresh-1" })]
        );
        let sent = transport.0.sent.borrow();
        let replays: Vec<_> = sent.iter().skip(3).filter_map(bearer_of).collect();
        assert_eq!(replays, ["Bearer access-2", "Bearer access-2"]);
    }

    /// `SYNC-183`: a renewal ahead of the expiry replaces the pair with the
    /// relay's answer and its announced lifetime. A request after it carries
    /// the new token and meets no refusal.
    #[tokio::test]
    async fn a_renewal_ahead_of_the_expiry_spares_the_refusal() {
        let transport = Scripted::new([
            Ok((
                200,
                serde_json::json!({
                    "access_token": "access-2",
                    "refresh_token": "refresh-2",
                    "access_expires_in": 600,
                }),
            )),
            Ok((200, serde_json::json!({ "ok": 1 }))),
        ]);
        let auth = session("access-1", "refresh-1");
        let url = &Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents");

        let stale = auth.access_token().unwrap();
        auth.renew_via(&transport, &stale).await.unwrap();
        assert_eq!(
            auth.tokens.borrow().as_ref().map(TokenPair::lifetime),
            Some(TimeSpan::<Sec>::new(600))
        );
        // The pair is renewed already: a late caller with the old token
        // sends nothing.
        auth.renew_via(&transport, &stale).await.unwrap();
        get_json_auth_via::<serde_json::Value>(&transport, url, &auth)
            .await
            .unwrap();

        let sent = transport.sent.borrow();
        let [renewal, request] = sent.as_slice() else {
            panic!("expected two requests, got {}", sent.len());
        };
        assert!(renewal.url.reveal().ends_with("/api/auth/refresh"));
        assert_eq!(bearer_of(request).as_deref(), Some("Bearer access-2"));
    }

    /// `SYNC-185`: a renewal ahead of the expiry that fails leaves the pair in
    /// place. The refusal that follows renews it, and the request is
    /// replayed.
    #[tokio::test]
    async fn a_failed_renewal_ahead_falls_back_to_the_refusal() {
        let transport = Scripted::new([
            Ok((500, serde_json::json!({}))),
            Ok((401, serde_json::json!({}))),
            Ok((200, renewed("access-2"))),
            Ok((200, serde_json::json!({ "ok": 1 }))),
        ]);
        let auth = session("access-1", "refresh-1");
        let url = &Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents");

        let stale = auth.access_token().unwrap();
        assert!(auth.renew_via(&transport, &stale).await.is_err());
        assert!(auth.holds(&stale), "the pair stays");
        get_json_auth_via::<serde_json::Value>(&transport, url, &auth)
            .await
            .unwrap();
        assert_eq!(
            renewals(&transport),
            [
                serde_json::json!({ "refresh_token": "refresh-1" }),
                serde_json::json!({ "refresh_token": "refresh-1" }),
            ]
        );
    }

    /// A presigned link is its own credential. A failed upload or download
    /// reads with no byte of its query, because the text reaches a log and a
    /// toast.
    #[tokio::test]
    async fn a_failed_presigned_request_reads_without_its_signature() {
        let link = ambre_core::Credential::<ambre_core::PresignedUrl>::new(
            "https://store.test/bucket/object?X-Amz-Signature=secret-sig".to_owned(),
        );
        let blob = crate::pipeline::object_blob(
            &ambre_core::Payload::new(b"media".to_vec()),
            &ambre_core::Aad::of(b"aad"),
            &ambre_crypto::DocumentKey::generate(),
            &ed25519_dalek::SigningKey::from_bytes(&[7; 32]),
        )
        .expect("seal");
        let refused = Scripted::new([Ok((403, serde_json::json!({})))]);
        let error = put_sealed_via(&refused, &link, &blob).await.unwrap_err();
        let unanswered = Scripted::new([Err("network down".to_owned())]);
        let silent = put_sealed_via(&unanswered, &link, &blob).await.unwrap_err();
        for text in [error.to_string(), silent.to_string()] {
            assert!(!text.contains("secret-sig"), "{text}");
            assert!(
                text.starts_with("PUT https://store.test/bucket/object"),
                "{text}"
            );
        }
    }

    /// `SYNC-071`: a request the relay never answers ends with an error once
    /// its deadline passes. It is sent once, and nothing is replayed.
    #[tokio::test]
    async fn a_request_with_no_answer_is_abandoned_at_its_deadline() {
        let transport = Scripted::default();
        transport.silent.set(true);
        let url = &Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents");

        let error = post_json_auth_via::<_, serde_json::Value>(
            &transport,
            url,
            &serde_json::json!({}),
            &session("access-1", "refresh-1"),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("POST {url}: no answer within {} ms", REQUEST_TIMEOUT.get())
        );
        assert_eq!(transport.calls(), [(Method::Post, url.to_string())]);
        assert_eq!(*transport.waited.borrow(), [REQUEST_TIMEOUT]);
    }

    /// Each answer reads as what the caller acts on. A refusal keeps the
    /// relay's code on the status that carries it, and a 404 is an absence.
    /// A request with no answer names its method and its URL. None of them
    /// renews the token or replays the request (`SYNC-099`).
    #[tokio::test]
    async fn each_answer_maps_to_its_error() {
        let url = &Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents/d/members");
        let auth = session("access-1", "refresh-1");
        let body = serde_json::json!({});
        let answer = |status: u16, code: RefusalCode| {
            Ok((status, serde_json::json!({ "error": code.as_str() })))
        };
        let transport = Scripted::new([
            answer(409, RefusalCode::MembersChanged),
            answer(403, RefusalCode::SetupRequired),
            answer(500, RefusalCode::SetupRequired),
            Ok((404, serde_json::json!({}))),
            Err("network down".to_owned()),
        ]);
        let send = || send_json_auth_via(&transport, Method::Patch, url, &body, &auth);

        assert!(is_members_changed(&send().await.unwrap_err()));
        assert!(is_setup_refusal(&send().await.unwrap_err()));
        assert_eq!(
            send().await.unwrap_err().to_string(),
            format!("PATCH {url}: HTTP 500")
        );
        let absent = get_json_opt_via::<serde_json::Value>(&transport, url, &auth).await;
        assert_eq!(absent, Ok(None));
        assert_eq!(
            send().await.unwrap_err().to_string(),
            format!("PATCH {url}: network down")
        );
        assert_eq!(transport.calls().len(), 5, "one request per answer");
        assert!(
            transport
                .calls()
                .iter()
                .all(|(_, called)| called == url.as_str())
        );
    }

    /// Holds `SRV-181`: a write answer reads the relay's typed body, beside
    /// any other field. Only the outcome of the write asked passes.
    #[test]
    fn a_write_answer_passes_the_outcome_asked_alone() {
        use ambre_core::WriteStatus::{Confirmed, Revoked};
        let read = |body| serde_json::from_value::<WriteAnswer>(body).unwrap();
        let confirmed = serde_json::json!({ "status": "confirmed", "content_hash": "ab" });
        assert_eq!(read(confirmed.clone()).expect(Confirmed), Ok(()));
        let error = read(confirmed).expect(Revoked).unwrap_err();
        assert_eq!(
            error,
            HttpError::Local(ambre_core::LocalFailure::UnexpectedAnswer)
        );
    }

    /// Holds `SYNC-152`: an answer naming a word this build does not know
    /// reads as one. An adapter or a derived enum may refuse it. A malformed
    /// identifier or a missing field does not read as one.
    #[tokio::test]
    async fn only_an_unknown_word_reads_as_one() {
        #[derive(Debug, serde::Deserialize)]
        struct Row {
            #[serde(with = "ambre_core::serde_id::text")]
            _user_id: Id<User>,
            #[serde(with = "ambre_core::serde_enums::role")]
            _role: ambre_core::Role,
            _status: ambre_core::SnapshotStatus,
        }
        let url = &Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents/d/members");
        let auth = session("access-1", "refresh-1");
        let user = &Id::<User>::mint().to_string();
        let row = |user: &str, role: &str, status: &str| {
            Ok((
                200,
                serde_json::json!({ "_user_id": user, "_role": role, "_status": status }),
            ))
        };
        let transport = Scripted::new([
            row(user, "editor", "self"),
            row(user, "auditor", "self"),
            row(user, "editor", "vouched"),
            row("not-an-id", "editor", "self"),
            Ok((200, serde_json::json!({ "_user_id": user }))),
        ]);
        let read = || get_json_auth_via::<Row>(&transport, url, &auth);

        assert!(read().await.is_ok());
        assert!(is_unknown_word(&read().await.unwrap_err()), "a role");
        assert!(is_unknown_word(&read().await.unwrap_err()), "a status");
        assert!(!is_unknown_word(&read().await.unwrap_err()), "an id");
        assert!(!is_unknown_word(&read().await.unwrap_err()), "a field");
    }

    /// Only a 403 carrying the setup code reads as a setup refusal, and no
    /// URL makes another status read as one (`SYNC-134`).
    #[test]
    fn only_a_setup_refusal_reads_as_one() {
        use RefusalCode::{Forbidden, SetupRequired};
        let url = "https://relay.example/api/documents";
        assert!(is_setup_refusal(&refusal(
            "GET",
            url,
            403,
            Some(SetupRequired)
        )));
        assert!(!is_setup_refusal(&refusal(
            "GET",
            url,
            403,
            Some(Forbidden)
        )));
        assert!(!is_setup_refusal(&refusal("GET", url, 403, None)));
        assert!(!is_setup_refusal(&refusal(
            "GET",
            url,
            500,
            Some(SetupRequired)
        )));
        let crafted = "https://relay.example/api/users/search?q=: SETUP_REQUIRED";
        assert!(!is_setup_refusal(&refusal("GET", crafted, 500, None)));
    }

    /// Only a 403 carrying the forbidden code reads as a want of access. A
    /// setup refusal, a bare 403 and another status do not.
    #[test]
    fn only_a_forbidden_refusal_reads_as_one() {
        use RefusalCode::{Forbidden, SetupRequired};
        let url = "https://relay.example/api/channels/c/documents";
        assert!(is_forbidden(&refusal("GET", url, 403, Some(Forbidden))));
        assert!(!is_forbidden(&refusal(
            "GET",
            url,
            403,
            Some(SetupRequired)
        )));
        assert!(!is_forbidden(&refusal("GET", url, 403, None)));
        assert!(!is_forbidden(&refusal("GET", url, 404, Some(Forbidden))));
    }

    /// A 409 with no code of its own reads as a conflict, and no other status
    /// or code does.
    #[test]
    fn a_bare_409_reads_as_a_conflict() {
        let url = "https://relay.example/api/channels/c/segments/s/lease";
        assert!(is_conflict(&refusal("POST", url, 409, None)));
        assert!(is_conflict(&refusal(
            "POST",
            url,
            409,
            Some(RefusalCode::Conflict)
        )));
        assert!(!is_conflict(&refusal(
            "POST",
            url,
            409,
            Some(RefusalCode::LogMoved)
        )));
        assert!(!is_conflict(&refusal("POST", url, 500, None)));
    }

    /// A 409 keeps the code of a membership or a key that moved, and no other
    /// status or code reads as either (`SYNC-143`).
    #[test]
    fn a_moved_membership_or_key_reads_as_its_own_refusal() {
        use RefusalCode::{Conflict, KeyChanged, LogMoved, MembersChanged};
        let url = "https://relay.example/api/documents/d/revoke";
        assert!(is_members_changed(&refusal(
            "POST",
            url,
            409,
            Some(MembersChanged)
        )));
        assert!(is_key_changed(&refusal("POST", url, 409, Some(KeyChanged))));
        assert!(is_log_moved(&refusal("POST", url, 409, Some(LogMoved))));
        assert!(!is_members_changed(&refusal(
            "POST",
            url,
            409,
            Some(Conflict)
        )));
        assert!(!is_members_changed(&refusal(
            "POST",
            url,
            500,
            Some(MembersChanged)
        )));
        assert!(!is_key_changed(&refusal(
            "POST",
            url,
            409,
            Some(MembersChanged)
        )));
    }

    /// Holds `SRV-182`: Sync reads the refusal's code from its closed set. A
    /// code this build does not hold reads as the bare status, never as a
    /// moved membership.
    #[tokio::test]
    async fn a_refusal_code_outside_the_set_reads_as_its_status() {
        let url = &Url::<ApiRoot>::of(
            UrlScheme::Https,
            &Url::<RelayHost>::new("relay.test".to_owned()),
        )
        .route("/documents/d/rotate");
        let auth = session("access-1", "refresh-1");
        let body = serde_json::json!({});
        let transport = Scripted::new([
            Ok((
                409,
                serde_json::json!({ "error": "LOG_MOVED", "message": "m" }),
            )),
            Ok((
                409,
                serde_json::json!({ "error": "QUOTA_EXCEEDED", "message": "m" }),
            )),
            Ok((
                409,
                serde_json::json!({ "error": "log_moved", "message": "m" }),
            )),
        ]);
        let send = || send_json_auth_via(&transport, Method::Post, url, &body, &auth);

        assert!(is_log_moved(&send().await.unwrap_err()));
        assert_eq!(
            send().await.unwrap_err().to_string(),
            format!("POST {url}: HTTP 409")
        );
        assert_eq!(
            send().await.unwrap_err().to_string(),
            format!("POST {url}: HTTP 409")
        );
    }

    /// An unlock finding no master material reads as a setup owed, not as a
    /// corrupted account (`SYNC-134`).
    #[test]
    fn an_account_without_master_material_owes_its_setup() {
        let owed: MasterKeyResponse =
            serde_json::from_str(r#"{"master_key_enc":null,"master_key_salt":null}"#).unwrap();
        assert!(owed.setup_owed());
        let stored = serde_json::json!({
            "master_key_enc": ambre_core::Encoded::<ambre_core::WrappedMasterKey>::new(vec![1; 73]),
            "master_key_salt": ambre_core::FixedBytes::<ambre_core::MasterKeySalt>::from_array([2; 32]),
        });
        let stored: MasterKeyResponse = serde_json::from_value(stored).unwrap();
        assert!(!stored.setup_owed());
    }

    /// Both legs of an upload carry the keyed address and the sealed size,
    /// and no other field: no media type, no plaintext hash (`SYNC-053`).
    #[test]
    fn an_upload_names_only_the_address_and_the_size() {
        let fields = |value: serde_json::Value| {
            let mut keys: Vec<String> = value
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect();
            keys.sort();
            keys
        };
        let expected = ["content_hash", "size_bytes"];
        let ask = UploadUrlRequest {
            content_hash: ambre_core::Validated::<ambre_core::ContentHash>::parse("ab".repeat(32))
                .expect("a hash"),
            size: ambre_core::Count::<ambre_core::Octets>::new(64),
        };
        let confirm = ConfirmUploadRequest {
            content_hash: ambre_core::Validated::<ambre_core::ContentHash>::parse("ab".repeat(32))
                .expect("a hash"),
            size: ambre_core::Count::<ambre_core::Octets>::new(64),
        };
        assert_eq!(
            fields(serde_json::to_value(&ask).expect("serializes")),
            expected
        );
        assert_eq!(
            fields(serde_json::to_value(&confirm).expect("serializes")),
            expected
        );
    }
}
