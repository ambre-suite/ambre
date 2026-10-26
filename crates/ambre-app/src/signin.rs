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
const PENDING_STATE_KEY: &str = "ambre.signin.state";

/// The path prefix the provider returns to. The deployment registers
/// `/signin/<provider>` as the provider's redirect address.
const RETURN_PREFIX: &str = "/signin/";

/// A return from the provider, as read from the page's address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderReturn {
    pub provider: ambre_core::OidcProvider,
    pub code: ambre_core::OidcCode,
    pub state: ambre_core::OidcState,
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
        code: ambre_core::OidcCode::new(code),
        state: ambre_core::OidcState::new(state),
    })
}

/// Accept a return only if it carries the state this tab was handed
/// (`CHROME-101`). A return the tab did not start is a login forged by a third
/// party, and would sign the person in as someone else.
pub(crate) fn accept_return(
    ret: ProviderReturn,
    pending: Option<ambre_core::OidcState>,
) -> Option<ProviderReturn> {
    (pending.as_ref() == Some(&ret.state)).then_some(ret)
}

/// Whether `url` may be followed: an `https` address. The address is data from
/// the relay, and a `javascript:` one would run in this page's origin.
pub(crate) fn is_followable(url: &ambre_core::ProviderUrl) -> bool {
    web_sys::Url::new(url.reveal()).is_ok_and(|u| u.protocol() == "https:")
}

/// Follow the provider's address: remember its `state` for the return, then
/// `navigate` there (`CHROME-095`). Refused, and nothing remembered, unless the
/// address is followable.
pub(crate) fn follow_address(
    url: &ambre_core::ProviderUrl,
    state: &ambre_core::OidcState,
    navigate: impl FnOnce(&str),
) -> bool {
    if !is_followable(url) {
        tracing::warn!("Sign-in: the relay answered an address that is not https");
        return false;
    }
    let Some(storage) = session_storage() else {
        return false;
    };
    if storage.set_item(PENDING_STATE_KEY, state.reveal()).is_err() {
        return false;
    }
    navigate(url.reveal());
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
/// then `reload` replaces the page with a fresh boot of `/`. A new page holds
/// nothing of the session the old one showed, which no list of signals to
/// reset could promise.
pub(crate) fn leave_session(reload: impl FnOnce(&str)) {
    crate::platform::forget_dev_user();
    let _ = take_pending_state();
    reload("/");
}

fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok().flatten()
}

/// Read and clear the remembered state: one return per sign-in.
fn take_pending_state() -> Option<ambre_core::OidcState> {
    let storage = session_storage()?;
    let state = storage.get_item(PENDING_STATE_KEY).ok().flatten();
    let _ = storage.remove_item(PENDING_STATE_KEY);
    state.map(ambre_core::OidcState::new)
}

// -- Tests (wasm tier) -------------------------------------------------------
// The crate is `#![cfg(target_arch = "wasm32")]`; these run in headless Chrome
// through `just test-wasm`.
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    fn state(text: &str) -> ambre_core::OidcState {
        ambre_core::OidcState::new(text.to_owned())
    }

    fn google(state: &str) -> ProviderReturn {
        ProviderReturn {
            provider: ambre_core::OidcProvider::Google,
            code: ambre_core::OidcCode::new("4/0Ab".to_owned()),
            state: ambre_core::OidcState::new(state.to_owned()),
        }
    }

    /// A provider return is read from its path and query, the code decoded;
    /// any other address, or an error return, is none (`CHROME-096`).
    #[wasm_bindgen_test]
    fn a_provider_return_is_read_from_its_address() {
        assert_eq!(
            parse_return("/signin/google", "?code=4%2F0Ab&state=st1&scope=email"),
            Some(google("st1"))
        );
        assert_eq!(
            parse_return("/signin/google/", "?state=st1&code=4%2F0Ab"),
            Some(google("st1"))
        );
        assert_eq!(
            parse_return("/signin/google", "?error=access_denied&state=st1"),
            None
        );
        assert_eq!(parse_return("/signin/google", "?code=c"), None);
        assert_eq!(parse_return("/signin/..%2Fkeys", "?code=c&state=s"), None);
        assert_eq!(parse_return("/signin/Google", "?code=c&state=s"), None);
        assert_eq!(parse_return("/trame/x", "?code=c&state=s"), None);
        assert!(is_return_path("/signin/google"));
        assert!(!is_return_path("/"));
    }

    /// A return is accepted only with the state this tab was handed
    /// (`CHROME-101`).
    #[wasm_bindgen_test]
    fn a_return_this_tab_did_not_start_is_refused() {
        assert_eq!(
            accept_return(google("st1"), Some(state("st1"))),
            Some(google("st1"))
        );
        assert_eq!(accept_return(google("st1"), Some(state("st2"))), None);
        assert_eq!(accept_return(google("st1"), None), None);
    }

    /// Following the address remembers its state for the return and sends the
    /// browser there; an address that is not `https` is refused (`CHROME-095`).
    #[wasm_bindgen_test]
    fn the_provider_address_is_followed_and_its_state_kept() {
        let _ = take_pending_state();
        let mut went = None;
        let text = "https://accounts.example.org/o/oauth2/auth?state=st9";
        let url = ambre_core::ProviderUrl::new(text.to_owned());
        assert!(follow_address(&url, &state("st9"), |u| went = Some(u.to_owned())));
        assert_eq!(went.as_deref(), Some(text));
        assert_eq!(take_pending_state(), Some(state("st9")));
        assert_eq!(take_pending_state(), None, "a state serves one return");

        for bad in [
            "javascript:alert(1)",
            "http://accounts.example.org/",
            "/api/auth",
        ] {
            let mut went = false;
            assert!(
                !follow_address(
                    &ambre_core::ProviderUrl::new(bad.to_owned()),
                    &state("st9"),
                    |_| went = true
                ),
                "{bad} was followed"
            );
            assert!(!went);
            assert_eq!(take_pending_state(), None);
        }
    }

    /// A confirmed sign-out forgets the user the next boot would sign in,
    /// and replaces the page with a fresh boot of home (`CHROME-107`,
    /// `CHROME-108`).
    #[wasm_bindgen_test]
    fn a_sign_out_forgets_the_user_and_boots_home_afresh() {
        crate::platform::remember_dev_user("ada");
        session_storage()
            .unwrap()
            .set_item(PENDING_STATE_KEY, "st1")
            .unwrap();
        let mut went = None;
        leave_session(|u| went = Some(u.to_owned()));
        assert_eq!(went.as_deref(), Some("/"));
        assert_eq!(crate::platform::stored_dev_user(), None);
        assert_eq!(take_pending_state(), None);
    }
}
