//! An identity provider the belt runs on the loopback (`PROOF-020`).
//!
//! A relay started with [`Provider::relay_env`] discovers it as its `google`
//! provider, through its normal configuration of an issuer (`SRV-321`). The
//! provider publishes its discovery document and its key. It hands out a code
//! for an authorization URL and a person, and answers that code with an
//! identity token it signs for the person. The code is good once, and only
//! under the verifier whose challenge the URL carried.
//!
//! A page never follows the provider's address: the client follows the
//! authorization host of Google alone (`CHROME-139`). The belt plays the
//! person and the provider instead. It asks the relay for the address, takes
//! the code here, and returns to the page as the provider would
//! ([`sign_in`]). From there the page runs the exchange of a release build.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use openidconnect::core::{
    CoreIdToken, CoreIdTokenClaims, CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey,
};
use openidconnect::{
    Audience, EmptyAdditionalClaims, EndUserEmail, EndUserName, IssuerUrl, JsonWebKeyId, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, PrivateSigningKey, StandardClaims, SubjectIdentifier,
};
use rsa::pkcs1::EncodeRsaPrivateKey;
use serde_json::{Value, json};

use crate::page::Page;

/// The client the provider knows, and its secret.
pub const CLIENT_ID: &str = "belt-client";
const CLIENT_SECRET: &str = "belt-secret";

/// The domain of the address of each person. The invite helpers of the belt
/// name a person by an address on it (`crate::murmure`).
pub const EMAIL_DOMAIN: &str = "dev.local";

/// What one authorization left for the token endpoint to check.
struct Grant {
    person: String,
    challenge: String,
    nonce: String,
}

struct Shared {
    issuer: String,
    key: CoreRsaPrivateSigningKey,
    /// The open grants, by the code each one handed out.
    grants: HashMap<String, Grant>,
}

/// A running provider. Its address is its issuer.
#[derive(Clone)]
pub struct Provider {
    pub issuer: String,
    shared: Arc<Mutex<Shared>>,
}

/// A fresh RSA key under the key id `kid`: the provider signs with it.
fn signing_key(kid: &str) -> CoreRsaPrivateSigningKey {
    let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).expect("an RSA key");
    let pem = key
        .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
        .expect("the key in PEM");
    CoreRsaPrivateSigningKey::from_pem(&pem, Some(JsonWebKeyId::new(kid.into())))
        .expect("a signing key")
}

/// The value of the query parameter `name` of `url`.
fn param(url: &reqwest::Url, name: &str) -> String {
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| panic!("no {name} in the authorization URL {url}"))
}

impl Provider {
    /// Starts the provider on a free port of the loopback.
    pub async fn spawn() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the provider");
        let issuer = format!("http://{}", listener.local_addr().expect("its address"));
        let provider = Self {
            issuer: issuer.clone(),
            shared: Arc::new(Mutex::new(Shared {
                issuer: issuer.clone(),
                key: signing_key("belt"),
                grants: HashMap::new(),
            })),
        };
        let metadata = json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/jwks"),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
        });
        let jwks = {
            let shared = provider.shared.lock().expect("the provider state");
            json!({ "keys": [shared.key.as_verification_key()] })
        };
        let shared = Arc::clone(&provider.shared);
        let app = axum::Router::new()
            .route(
                "/.well-known/openid-configuration",
                axum::routing::get(move || async move { axum::Json(metadata) }),
            )
            .route(
                "/jwks",
                axum::routing::get(move || async move { axum::Json(jwks) }),
            )
            .route(
                "/token",
                axum::routing::post(
                    move |axum::extract::Form(form): axum::extract::Form<
                        HashMap<String, String>,
                    >| async move { token(&shared, &form) },
                ),
            );
        tokio::spawn(async move { axum::serve(listener, app).await });
        provider
    }

    /// The variables that make a relay discover this provider as its `google`
    /// provider (`SRV-321`). The relay sends the redirect to the provider
    /// alone, which reads none: the belt returns to each page itself.
    pub fn relay_env(&self) -> [(&'static str, String); 4] {
        [
            ("OIDC_GOOGLE_CLIENT_ID", CLIENT_ID.to_owned()),
            ("OIDC_GOOGLE_CLIENT_SECRET", CLIENT_SECRET.to_owned()),
            ("OIDC_GOOGLE_ISSUER", self.issuer.clone()),
            (
                "OIDC_GOOGLE_REDIRECT_URI",
                "http://localhost/signin/google".to_owned(),
            ),
        ]
    }

    /// What the provider does once `person` signs in at `url`, the
    /// authorization URL the relay made. It keeps the challenge and the
    /// one-time value the URL carries, and returns the code it hands back.
    pub fn authorize(&self, url: &str, person: &str) -> String {
        let url = reqwest::Url::parse(url).expect("an authorization URL");
        assert_eq!(param(&url, "client_id"), CLIENT_ID, "another client");
        let code = format!("code-{}", uuid::Uuid::new_v4().simple());
        self.shared
            .lock()
            .expect("the provider state")
            .grants
            .insert(
                code.clone(),
                Grant {
                    person: person.to_owned(),
                    challenge: param(&url, "code_challenge"),
                    nonce: param(&url, "nonce"),
                },
            );
        code
    }
}

/// The token endpoint. The code it handed out, under the verifier whose
/// challenge the authorization carried, answers with an identity token signed
/// for the person of that code. Anything else is refused. A code is good once.
fn token(shared: &Mutex<Shared>, form: &HashMap<String, String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let mut shared = shared.lock().expect("the provider state");
    let Some(grant) = form.get("code").and_then(|code| shared.grants.remove(code)) else {
        return refused();
    };
    let verifier = PkceCodeVerifier::new(form.get("code_verifier").cloned().unwrap_or_default());
    if PkceCodeChallenge::from_code_verifier_sha256(&verifier).as_str() != grant.challenge {
        return refused();
    }
    let now = chrono::Utc::now();
    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new(shared.issuer.clone()).expect("the issuer"),
        vec![Audience::new(CLIENT_ID.into())],
        now + chrono::Duration::minutes(5),
        now,
        StandardClaims::new(SubjectIdentifier::new(format!("belt-{}", grant.person)))
            .set_email(Some(EndUserEmail::new(format!(
                "{}@{EMAIL_DOMAIN}",
                grant.person
            ))))
            .set_email_verified(Some(true))
            .set_name(Some(EndUserName::new(grant.person.clone()).into())),
        EmptyAdditionalClaims {},
    )
    .set_nonce(Some(Nonce::new(grant.nonce)));
    let id_token = CoreIdToken::new(
        claims,
        &shared.key,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        None,
        None,
    )
    .expect("a signed identity token");
    axum::Json(json!({
        "access_token": "belt-access",
        "token_type": "bearer",
        "id_token": id_token.to_string(),
    }))
    .into_response()
}

fn refused() -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::BAD_REQUEST,
        axum::Json(json!({ "error": "invalid_grant" })),
    )
        .into_response()
}

/// The return of `provider` for `person`: the code it handed out and the
/// state of the sign-in the relay of `page` started.
async fn provider_return(page: &Page, provider: &Provider, person: &str) -> (String, String) {
    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("an HTTP client");
    let answered = http
        .get(format!("{}/api/auth/google/login", page.relay().base))
        .header(crate::serve::FORWARDED_FOR, crate::serve::page_address())
        .send()
        .await
        .expect("the relay answers the sign-in")
        .error_for_status()
        .expect("the relay starts a provider sign-in: is it started with Provider::relay_env?")
        .bytes()
        .await
        .expect("the answer of the sign-in");
    let login: Value = serde_json::from_slice(&answered).expect("the sign-in as JSON");
    let url = login["url"].as_str().expect("the address of the provider");
    let state = login["state"].as_str().expect("the state").to_owned();
    (provider.authorize(url, person), state)
}

/// Signs `person` in on `page` through `provider`, as a release build signs
/// in, then sets up the keys the new account owes. Waits for the document
/// list under that person.
///
/// The page stores the state of the sign-in, as following the address would.
/// It then boots on the return, and its Sync worker has the relay exchange the
/// code with the provider. No name and no seed comes from the belt.
pub async fn sign_in(page: &Page, provider: &Provider, person: &str) {
    let (code, state) = provider_return(page, provider, person).await;
    page.browser
        .script(
            "sessionStorage.setItem('ambre.signin.state', arguments[0]); return true;",
            json!([state]),
        )
        .await;
    let mut back =
        reqwest::Url::parse(&format!("{}/signin/google", page.origin)).expect("the return address");
    back.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &state);
    page.browser.goto(back.as_str()).await;
    crate::account::set_up_keys(page).await;
    crate::account::follow(page, ".ambre-words-done").await;
    crate::account::wait_list(page, person).await;
}

#[cfg(test)]
mod tests {
    use openidconnect::core::{CoreClient, CoreProviderMetadata};
    use openidconnect::{
        AuthorizationCode, ClientId, ClientSecret, CsrfToken, RedirectUrl, Scope, TokenResponse,
    };

    use super::*;
    use crate::relay::Relay;

    /// A client of the provider at `issuer`, as the relay discovers one.
    async fn discover(
        issuer: &str,
        http: &reqwest::Client,
    ) -> CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    > {
        let metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(issuer.to_owned()).expect("the issuer"),
            http,
        )
        .await
        .expect("the provider is discovered");
        CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(CLIENT_ID.to_owned()),
            Some(ClientSecret::new(CLIENT_SECRET.to_owned())),
        )
        .set_redirect_uri(
            RedirectUrl::new("http://localhost/signin/google".to_owned()).expect("a redirect"),
        )
    }

    /// `PROOF-020`: two people sign in at the provider at once, each under the
    /// code and the verifier of their own authorization. The relay's client
    /// verifies each identity token against the published key and the
    /// one-time value, and reads that person's subject, verified address and
    /// name. A code is good once, and only under its own verifier.
    #[tokio::test]
    async fn each_person_signs_in_at_the_provider_under_a_verified_identity() {
        let provider = Provider::spawn().await;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("an HTTP client");
        let client = discover(&provider.issuer, &http).await;
        let http = &http;
        let authorize = |person: &str| {
            let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
            let (url, _, nonce) = client
                .authorize_url(
                    openidconnect::core::CoreAuthenticationFlow::AuthorizationCode,
                    CsrfToken::new_random,
                    Nonce::new_random,
                )
                .add_scope(Scope::new("email".to_owned()))
                .set_pkce_challenge(challenge)
                .url();
            (provider.authorize(url.as_str(), person), verifier, nonce)
        };
        let exchange = |code: &str, verifier: PkceCodeVerifier| {
            let request = client
                .exchange_code(AuthorizationCode::new(code.to_owned()))
                .expect("a token endpoint")
                .set_pkce_verifier(verifier);
            async move { request.request_async(http).await }
        };

        let (ada_code, ada_verifier, ada_nonce) = authorize("ada");
        let (bob_code, bob_verifier, bob_nonce) = authorize("bob");
        for (person, code, verifier, nonce) in [
            ("bob", &bob_code, bob_verifier, bob_nonce),
            ("ada", &ada_code, ada_verifier, ada_nonce),
        ] {
            let answer = exchange(code, verifier)
                .await
                .expect("the code is exchanged");
            let claims = answer
                .id_token()
                .expect("an identity token")
                .claims(&client.id_token_verifier(), &nonce)
                .expect("the identity token verifies");
            assert_eq!(claims.subject().as_str(), format!("belt-{person}"));
            assert_eq!(
                claims.email().map(|e| e.as_str().to_owned()),
                Some(format!("{person}@{EMAIL_DOMAIN}"))
            );
            assert_eq!(claims.email_verified(), Some(true));
            let name = claims
                .name()
                .and_then(|n| n.get(None))
                .map(|n| n.as_str().to_owned());
            assert_eq!(name.as_deref(), Some(person));
        }

        let (again, _, _) = authorize("ada");
        let replayed = exchange(&ada_code, PkceCodeVerifier::new("x".repeat(43))).await;
        assert!(replayed.is_err(), "a code is good once");
        let other = PkceCodeVerifier::new("a-verifier-the-authorization-never-made-0123".into());
        assert!(
            exchange(&again, other).await.is_err(),
            "a code under another verifier"
        );
    }

    /// `PROOF-020`, `SRV-321`, in the browser: a relay configured with the
    /// provider as its issuer signs a page in. The page runs the exchange of a
    /// provider return, and sets up the keys of the new account. The document
    /// list then shows under the person's name. The stand-in of the belt would
    /// read the code as a name, and show the list under that code instead.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs a build and chromedriver; run just test-browser"
    )]
    async fn a_page_signs_in_through_the_provider_of_the_belt() {
        let provider = Provider::spawn().await;
        let relay = Arc::new(Relay::start_with(&provider.relay_env()).await);
        let page = Page::home(relay).await;
        let person = format!("ada-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        sign_in(&page, &provider, &person).await;
    }
}
