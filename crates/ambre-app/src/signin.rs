// ambre-app/src/signin.rs
// ============================================================================
// Provider sign-in on the UI thread (`ambre-spec-launcher.md` section 1.6).
//
// The page never speaks to the relay here. The control asks the Sync worker
// for the provider's address, and the page follows it (`CHROME-095`). The
// provider sends the browser back to `/signin/<provider>?code=..&state=..`,
// which the page reads once at boot and hands to the Sync worker. That worker
// exchanges it and keeps the tokens (`CHROME-096`). The page holds only the
// `state` it was handed, to refuse a return it did not start (`CHROME-101`).
// ============================================================================

/// sessionStorage key holding the `state` of the sign-in this tab started.
const PENDING_STATE_KEY: ambre_core::StaticKey<ambre_core::StorageKeys> =
    ambre_core::StaticKey::new("ambre.signin.state");

/// The path prefix the provider returns to. The deployment registers
/// `/signin/<provider>` as the provider's redirect address.
const RETURN_PREFIX: &str = "/signin/";

/// A return from the provider, as read from the page's address. Its code and
/// state are secrets, so it derives no `PartialEq`: the state compares in
/// constant time, through [`ambre_core::Credential::matches`].
#[derive(Debug, Clone)]
pub(crate) struct ProviderReturn {
    pub provider: ambre_core::OidcProvider,
    pub code: ambre_core::Credential<ambre_core::OidcCode>,
    pub state: ambre_core::Credential<ambre_core::OidcState>,
}

/// Whether `path` is a provider return address, whatever its query.
pub(crate) fn is_return_path(path: &str) -> bool {
    path.starts_with(RETURN_PREFIX)
}

/// Read a provider return from a path and its query string. The path must be
/// `/signin/<provider>`, for a provider of the closed set. The query must carry
/// both a `code` and a `state`. Otherwise the result is `None`. A
/// provider that answers an error carries no code. A crafted link that names
/// no provider reaches no worker.
pub(crate) fn parse_return(path: &str, query: &str) -> Option<ProviderReturn> {
    let provider = path.strip_prefix(RETURN_PREFIX)?;
    let provider = provider.strip_suffix('/').unwrap_or(provider);
    let provider = ambre_core::OidcProvider::parse(provider)?;
    let params = web_sys::UrlSearchParams::new_with_str(query).ok()?;
    let code = params.get("code").filter(|c| !c.is_empty())?;
    let state = params.get("state").filter(|s| !s.is_empty())?;
    Some(ProviderReturn {
        provider,
        code: ambre_core::Credential::new(code),
        state: ambre_core::Credential::new(state),
    })
}

/// Accept a return only if it carries the state this tab was handed
/// (`CHROME-101`). A return the tab did not start is a login forged by a third
/// party, and would sign the person in as someone else.
pub(crate) fn accept_return(
    ret: ProviderReturn,
    pending: Option<ambre_core::Credential<ambre_core::OidcState>>,
) -> Option<ProviderReturn> {
    pending
        .is_some_and(|pending| pending.matches(&ret.state))
        .then_some(ret)
}

/// The issuer of the `oidc` provider, as the client build names it in
/// `AMBRE_OIDC_ISSUER`. The build, never the relay, names it. So a relay
/// still cannot send the sign-in to a host of its choice (`CHROME-139`).
const BUILD_ISSUER: Option<&str> = option_env!("AMBRE_OIDC_ISSUER");

/// The issuer the client build names, when it is an `https` address.
fn build_issuer() -> Option<web_sys::Url> {
    BUILD_ISSUER
        .and_then(|text| web_sys::Url::new(text).ok())
        .filter(|issuer| issuer.protocol() == "https:")
}

/// The provider the sign-in offers: the `oidc` provider when the client build
/// names its issuer, Google otherwise (`ambre-spec-session.md` section 3).
pub(crate) fn offered_provider() -> ambre_core::OidcProvider {
    match build_issuer() {
        Some(_) => ambre_core::OidcProvider::Oidc,
        None => ambre_core::OidcProvider::Google,
    }
}

/// Whether `url` may be followed: an `https` address on the authorization
/// host of a provider the client knows. The host of the `oidc` provider is the
/// host of the issuer the build names, port included. The address is data
/// from the relay. A `javascript:` one would run in this page's origin. Any
/// other host would show a page of the relay's choice where the user expects
/// the provider's password prompt (`CHROME-139`).
pub(crate) fn is_followable(url: &ambre_core::Opaque<ambre_core::ProviderUrls>) -> bool {
    web_sys::Url::new(url.as_str()).is_ok_and(|u| {
        u.protocol() == "https:"
            && ambre_core::OidcProvider::ALL.iter().any(|provider| {
                match provider.authorization_host() {
                    Some(host) => u.host() == host,
                    None => build_issuer().is_some_and(|issuer| issuer.host() == u.host()),
                }
            })
    })
}

/// Follow the provider's address: remember its `state` for the return, then
/// `navigate` there (`CHROME-095`). Refused, and nothing remembered, unless the
/// address is followable.
pub(crate) fn follow_address(
    url: &ambre_core::Opaque<ambre_core::ProviderUrls>,
    state: &ambre_core::Credential<ambre_core::OidcState>,
    navigate: impl FnOnce(&ambre_core::Opaque<ambre_core::ProviderUrls>),
) -> bool {
    if !is_followable(url) {
        tracing::warn!("Sign-in: the relay answered an address off every provider host");
        return false;
    }
    let Some(storage) = session_storage() else {
        return false;
    };
    if state
        .present_state(|text| storage.set_item(PENDING_STATE_KEY.as_str(), text))
        .is_err()
    {
        return false;
    }
    navigate(url);
    true
}

/// The return this page booted on, if it is one this tab started. It is read
/// once, and the remembered state is cleared. The address becomes `/`, so the
/// code neither lingers in the history nor replays on a reload.
pub(crate) fn take_boot_return() -> Option<ProviderReturn> {
    let window = web_sys::window()?;
    let location = window.location();
    let path = location.pathname().ok()?;
    if !is_return_path(&path) {
        return None;
    }
    let query = location.search().unwrap_or_default();
    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/"));
    }
    let pending = take_pending_state();
    let ret = parse_return(&path, &query)?;
    let accepted = accept_return(ret, pending);
    if accepted.is_none() {
        tracing::warn!("Sign-in: refused a provider return this tab did not start");
    }
    accepted
}

/// The page's part of a sign-out the Sync worker confirmed (`CHROME-107`,
/// `CHROME-108`). What would sign someone in at the next boot is forgotten,
/// then `reload` replaces the page with a fresh boot of home. A new page holds
/// nothing of the session the old one showed, which no list of signals to
/// reset could promise.
pub(crate) fn leave_session(reload: impl FnOnce()) {
    crate::dev_login::forget_user();
    let _ = take_pending_state();
    reload();
}

fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok().flatten()
}

/// Read and clear the remembered state: one return per sign-in.
fn take_pending_state() -> Option<ambre_core::Credential<ambre_core::OidcState>> {
    let storage = session_storage()?;
    let state = storage.get_item(PENDING_STATE_KEY.as_str()).ok().flatten();
    let _ = storage.remove_item(PENDING_STATE_KEY.as_str());
    state.map(ambre_core::Credential::new)
}

// -- Tests (wasm tier) -------------------------------------------------------
// The crate is `#![cfg(target_arch = "wasm32")]`; these run in headless Chrome
// through `just test-wasm`.
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    fn state(text: &str) -> ambre_core::Credential<ambre_core::OidcState> {
        ambre_core::Credential::new(text.to_owned())
    }

    /// A return as its parts read, for a test that compares two.
    fn read(ret: Option<ProviderReturn>) -> Option<(ambre_core::OidcProvider, String, String)> {
        ret.map(|ret| {
            (
                ret.provider,
                ret.code.secret().expose().clone(),
                ret.state.secret().expose().clone(),
            )
        })
    }

    /// A state as its text reads.
    fn text(state: Option<ambre_core::Credential<ambre_core::OidcState>>) -> Option<String> {
        state.map(|state| state.secret().expose().clone())
    }

    fn google(state: &str) -> ProviderReturn {
        ProviderReturn {
            provider: ambre_core::OidcProvider::Google,
            code: ambre_core::Credential::new("4/0Ab".to_owned()),
            state: ambre_core::Credential::new(state.to_owned()),
        }
    }

    /// A provider return is read from its path and query, the code decoded;
    /// any other address, or an error return, is none (`CHROME-096`).
    #[wasm_bindgen_test]
    fn a_provider_return_is_read_from_its_address() {
        assert_eq!(
            read(parse_return(
                "/signin/google",
                "?code=4%2F0Ab&state=st1&scope=email"
            )),
            read(Some(google("st1")))
        );
        assert_eq!(
            read(parse_return("/signin/google/", "?state=st1&code=4%2F0Ab")),
            read(Some(google("st1")))
        );
        assert!(parse_return("/signin/google", "?error=access_denied&state=st1").is_none());
        assert!(parse_return("/signin/google", "?code=c").is_none());
        assert!(parse_return("/signin/..%2Fkeys", "?code=c&state=s").is_none());
        assert!(parse_return("/signin/Google", "?code=c&state=s").is_none());
        assert!(parse_return("/trame/x", "?code=c&state=s").is_none());
        assert!(is_return_path("/signin/google"));
        assert!(!is_return_path("/"));
    }

    /// A return is accepted only with the state this tab was handed
    /// (`CHROME-101`).
    #[wasm_bindgen_test]
    fn a_return_this_tab_did_not_start_is_refused() {
        assert_eq!(
            read(accept_return(google("st1"), Some(state("st1")))),
            read(Some(google("st1")))
        );
        assert!(accept_return(google("st1"), Some(state("st2"))).is_none());
        assert!(accept_return(google("st1"), None).is_none());
    }

    /// Following the address remembers its state for the return and sends the
    /// browser there (`CHROME-095`). An address that is not `https` is
    /// refused, and so is one off the provider's authorization host
    /// (`CHROME-139`).
    #[wasm_bindgen_test]
    fn the_provider_address_is_followed_and_its_state_kept() {
        let _ = take_pending_state();
        let mut went = None;
        let address = "https://accounts.google.com/o/oauth2/v2/auth?state=st9";
        let url = ambre_core::Opaque::<ambre_core::ProviderUrls>::new(address.to_owned());
        assert!(follow_address(&url, &state("st9"), |u| went = Some(u.as_str().to_owned())));
        assert_eq!(went.as_deref(), Some(address));
        assert_eq!(text(take_pending_state()).as_deref(), Some("st9"));
        assert!(take_pending_state().is_none(), "a state serves one return");

        for bad in [
            "javascript:alert(1)",
            "http://accounts.google.com/",
            "/api/auth",
            "https://accounts.example.org/o/oauth2/auth",
            "https://accounts.google.com.evil.example/o/oauth2/auth",
            "https://accounts.google.com@evil.example/",
            "https://accounts.google.com:8443/o/oauth2/auth",
        ] {
            let mut went = false;
            assert!(
                !follow_address(
                    &ambre_core::Opaque::<ambre_core::ProviderUrls>::new(bad.to_owned()),
                    &state("st9"),
                    |_| went = true
                ),
                "{bad} was followed"
            );
            assert!(!went);
            assert!(take_pending_state().is_none());
        }
    }

    /// A confirmed sign-out forgets the user the next boot would sign in,
    /// and replaces the page with a fresh boot of home (`CHROME-107`,
    /// `CHROME-108`).
    #[wasm_bindgen_test]
    fn a_sign_out_forgets_the_user_and_boots_home_afresh() {
        #[cfg(feature = "dev-login")]
        crate::dev_login::remember_user(
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("ada").unwrap(),
        );
        session_storage()
            .unwrap()
            .set_item(PENDING_STATE_KEY.as_str(), "st1")
            .unwrap();
        let mut reloaded = false;
        leave_session(|| reloaded = true);
        assert!(reloaded, "the page boots afresh");
        #[cfg(feature = "dev-login")]
        assert_eq!(crate::dev_login::stored_user(), None);
        assert!(take_pending_state().is_none());
    }
}
