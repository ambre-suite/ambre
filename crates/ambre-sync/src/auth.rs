use super::*;
use ambre_bus::AuthFailure;
use ambre_core::Flag as _;

impl SyncState {
    // -- Session handlers ------------------------------------------------------

    pub(crate) fn handle_unlock(
        &mut self,
        id: Seq<Request>,
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) {
        tracing::info!("Sync: Unlock");

        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let event_tx = self.event_tx.clone();
        let auth = self.auth_context();

        if let Some(tx) = event_tx {
            // WASM path: spawn an async HTTP task; the result flows back
            // through the SyncEvent queue into handle_sync_event.
            wasm_bindgen_futures::spawn_local(async move {
                let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                    .route("/keys/master");
                match http::get_json_auth::<http::MasterKeyResponse>(&url, auth.clone()).await {
                    Ok(resp) if resp.setup_owed() => {
                        // No master material to unlock: the account still owes
                        // its key setup, which is no failure (`SYNC-134`).
                        let _ = logic_tx.send(LogicEvent::Rest {
                            id: Some(id),
                            evt: RestEvent::AccountSetupRequired,
                        });
                    }
                    Ok(resp) => {
                        let master_seed = match open_stored_seed(&resp, &passphrase) {
                            Ok(seed) => seed,
                            Err(cause) => {
                                let _ = logic_tx.send(LogicEvent::AuthError { cause });
                                return;
                            }
                        };
                        let signing_key = master_seed.derive_signing_key();
                        let exchange_key = master_seed.derive_exchange_key();

                        let auth_url =
                            crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                                .route("/auth/me");
                        match http::get_json_auth::<http::AuthMeResponse>(&auth_url, auth.clone())
                            .await
                        {
                            Ok(me) => {
                                let tokens = match session_tokens(&auth.tokens) {
                                    Ok(tokens) => tokens,
                                    Err(failure) => {
                                        let _ = logic_tx.send(*failure);
                                        return;
                                    }
                                };
                                let user_id = me.user_id;
                                let _ = tx.send(SyncEvent::SessionReady {
                                    id: Some(id),
                                    user_id,
                                    display_name: me.display_name,
                                    email: me.email,
                                    master_seed,
                                    signing_key,
                                    exchange_key,
                                    tokens,
                                });
                            }
                            Err(e) => {
                                let _ = logic_tx.send(auth_failed(e));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = logic_tx.send(auth_failed(e));
                    }
                }
            });
        } else {
            // Host-test / offline path: generate keys locally. The user it
            // announces is the one it holds, so an opened document gets a
            // session identity (`AUTHOR-002`).
            let master_seed = MasterSeed::generate();
            let user_id = *self.user_id.get_or_insert_with(Id::<User>::mint);
            self.signing_key = Some(master_seed.derive_signing_key());
            self.exchange_key = Some(master_seed.derive_exchange_key());
            self.master_seed = Some(master_seed);
            self.open_websocket();
            self.reply(
                id,
                RestEvent::SessionReady {
                    user_id,
                    display_name: ambre_core::cut_display_name_to_fit("offline")
                        .unwrap_or_else(|| ambre_core::display_handle(&user_id)),
                    email: "offline@localhost".into(),
                },
            );
        }
    }

    /// Dev-only login: authenticate as `username` with keys derived
    /// deterministically from the username (no passphrase, no master-key
    /// round-trip). The server creates the user on the fly and is the sole gate
    /// (it rejects this unless `DEV_MODE` is set). Lets `?user=<name>` test
    /// multi-user behaviour on one machine; an edit survives a refresh because
    /// the same username always derives the same identity.
    /// `id` is `None` for the silent re-login, which no request asked for.
    pub(crate) fn handle_dev_login(
        &mut self,
        id: Option<Seq<Request>>,
        username: ambre_core::Validated<ambre_core::DisplayName>,
    ) {
        tracing::info!("Sync: DevLogin (username={username:?})");
        // Remember it so a WS auth rejection (close 4401) whose refresh also fails
        // can silently re-login with the same deterministic identity.
        self.dev_username = Some(username.clone());

        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let event_tx = self.event_tx.clone();

        // Deterministic seed: domain-separated BLAKE2b of the username. The same
        // username always yields the same keys, so a refresh re-derives the
        // identity with no server-side key storage.
        let master_seed = ambre_crypto::hash::dev_seed(&username);
        let signing_key = master_seed.derive_signing_key();
        let exchange_key = master_seed.derive_exchange_key();

        if let Some(tx) = event_tx {
            let signing_pub_key =
                ambre_core::FixedBytes::from_array(signing_key.verifying_key().to_bytes());
            let exchange_pub_key = crate::sharing::own_exchange_key(&exchange_key);
            wasm_bindgen_futures::spawn_local(async move {
                let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                    .route("/auth/dev/login");
                let body = http::DevLoginRequest {
                    username,
                    signing_pub_key,
                    exchange_pub_key,
                };
                match http::post_json::<_, http::DevLoginResponse>(&url, &body).await {
                    Ok(resp) => {
                        let user_id = resp.user_id;
                        let _ = tx.send(SyncEvent::SessionReady {
                            id,
                            user_id,
                            display_name: resp.display_name,
                            email: ambre_core::Email::new(String::new()),
                            master_seed,
                            signing_key,
                            exchange_key,
                            tokens: resp.tokens,
                        });
                    }
                    Err(e) => {
                        let _ = logic_tx.send(auth_failed(e));
                    }
                }
            });
        } else {
            // Host-test / offline path: adopt the deterministic keys locally.
            self.signing_key = Some(signing_key);
            self.exchange_key = Some(exchange_key);
            self.master_seed = Some(master_seed);
            self.open_websocket();
            let user_id = Id::<User>::mint();
            self.send_to_logic(LogicEvent::Rest {
                id,
                evt: RestEvent::SessionReady {
                    user_id,
                    display_name: username,
                    email: ambre_core::Email::new(String::new()),
                },
            });
        }
    }

    /// Ask the relay for `provider`'s authorization address and hand it to
    /// the UI, which sends the browser there (`CHROME-095`). Unauthenticated:
    /// no session exists yet.
    pub(crate) fn handle_provider_sign_in(
        &mut self,
        id: Seq<Request>,
        provider: ambre_core::OidcProvider,
    ) {
        tracing::info!("Sync: ProviderSignIn ({provider})");
        if self.event_tx.is_none() {
            self.send_to_logic(LogicEvent::AuthError {
                cause: AuthFailure::Api(ApiFailure::Local(ambre_core::LocalFailure::NoRelay)),
            });
            return;
        }
        let logic_tx = self.logic_tx.clone();
        let url = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base)
            .route(format!("/auth/{provider}/login"));
        wasm_bindgen_futures::spawn_local(async move {
            let evt = match http::get_json::<http::ProviderLoginResponse>(&url).await {
                Ok(resp) => LogicEvent::Rest {
                    id: Some(id),
                    evt: RestEvent::ProviderAddress {
                        url: resp.url,
                        state: resp.state,
                    },
                },
                Err(message) => auth_failed(message),
            };
            let _ = logic_tx.send(evt);
        });
    }

    /// Exchange a provider return for a session, then ask whether the account
    /// holds its keys. The tokens travel to [`handle_sync_event`] and stop
    /// there: the UI hears who signed in, never with what (`CHROME-096`).
    ///
    /// [`handle_sync_event`]: Self::handle_sync_event
    pub(crate) fn handle_provider_return(
        &mut self,
        id: Seq<Request>,
        provider: ambre_core::OidcProvider,
        code: ambre_core::Credential<ambre_core::OidcCode>,
        state: ambre_core::Credential<ambre_core::OidcState>,
    ) {
        tracing::info!("Sync: ProviderReturn ({provider})");
        let Some(tx) = self.event_tx.clone() else {
            self.send_to_logic(LogicEvent::AuthError {
                cause: AuthFailure::Api(ApiFailure::Local(ambre_core::LocalFailure::NoRelay)),
            });
            return;
        };
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        wasm_bindgen_futures::spawn_local(async move {
            let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route(format!("/auth/{provider}/callback"));
            let body = http::ProviderCallbackRequest { code, state };
            let resp = match http::post_json::<_, http::ProviderCallbackResponse>(&url, &body).await
            {
                Ok(resp) => resp,
                Err(message) => {
                    let _ = logic_tx.send(auth_failed(message));
                    return;
                }
            };
            // A context of its own: the worker's store takes the tokens only
            // once the whole sign-in has succeeded.
            let auth = http::AuthContext {
                api_base: api_base.clone(),
                url_scheme,
                tokens: Rc::new(RefCell::new(Some(resp.tokens))),
                renewing: Rc::new(futures_util::lock::Mutex::new(())),
                renewed: None,
            };
            let keys_url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                .route("/keys/master");
            let keys = match http::get_json_auth::<http::MasterKeyResponse>(&keys_url, auth.clone())
                .await
            {
                Ok(keys) => keys,
                Err(message) => {
                    let _ = logic_tx.send(auth_failed(message));
                    return;
                }
            };
            let Some(tokens) = auth.tokens.borrow_mut().take() else {
                return;
            };
            let _ = tx.send(SyncEvent::ProviderSignedIn {
                id,
                user_id: resp.user_id,
                display_name: resp.display_name,
                email: resp.email,
                keys_stored: ambre_bus::KeySetup::from_bool(!keys.setup_owed()),
                tokens,
            });
        });
    }

    pub(crate) fn handle_setup_account(
        &mut self,
        id: Seq<Request>,
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
        email: ambre_core::Email,
        display_name: ambre_core::Validated<ambre_core::DisplayName>,
    ) {
        tracing::info!("Sync: SetupAccount");

        // The strength floor runs first, before any key or request exists.
        let Some(passphrase) = self.admit_new_passphrase(passphrase) else {
            return;
        };

        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let master_seed = MasterSeed::generate();
                let recovery_words = master_seed.recovery_words();
                let signing_key = master_seed.derive_signing_key();
                let exchange_key = master_seed.derive_exchange_key();
                let signing_pub_key =
                    ambre_core::FixedBytes::from_array(signing_key.verifying_key().to_bytes());
                let exchange_pub_key = crate::sharing::own_exchange_key(&exchange_key);
                let http::SealedSeed {
                    master_key_enc,
                    master_key_salt,
                } = match seal_seed(&master_seed, &passphrase) {
                    Ok(sealed) => sealed,
                    Err(cause) => {
                        let _ = logic_tx.send(LogicEvent::AuthError { cause });
                        return;
                    }
                };

                // Zero-Knowledge (N4): only the wrapped master key and its salt go
                // over the wire; the passphrase stays in this worker and zeroizes
                // on drop.
                let url = crate::http::Url::<crate::http::ApiRoot>::of(url_scheme, &api_base)
                    .route("/auth/setup");
                let body = http::SetupAccountRequest {
                    email,
                    display_name,
                    master_key_enc,
                    master_key_salt,
                    signing_pubkey: signing_pub_key,
                    exchange_pubkey: exchange_pub_key,
                };
                match http::post_json::<_, http::SetupAccountResponse>(&url, &body).await {
                    Ok(resp) => {
                        let _ = tx.send(SyncEvent::AccountCreated {
                            id,
                            recovery_words,
                            master_seed,
                            signing_key,
                            exchange_key,
                            tokens: resp.tokens,
                        });
                    }
                    Err(e) => {
                        let _ = logic_tx.send(auth_failed(e));
                    }
                }
            });
        } else {
            // Host-test / offline path: the identity is not persisted anywhere, so
            // the supplied fields are dropped (passphrase zeroizes on drop).
            let _ = (passphrase, email, display_name);
            let master_seed = MasterSeed::generate();
            self.signing_key = Some(master_seed.derive_signing_key());
            self.exchange_key = Some(master_seed.derive_exchange_key());
            let recovery_words = master_seed.recovery_words();
            self.master_seed = Some(master_seed);
            self.open_websocket();
            self.reply(id, RestEvent::AccountCreated { recovery_words });
        }
    }

    /// Hold the strength floor on a passphrase that will seal the Master Seed.
    /// A passphrase an offline guess finds seals nothing, so a weak one is
    /// refused with its weakness alone and `None` ends the caller. The relay
    /// learns neither the passphrase nor its estimate (`ZK-035`, `ZK-036`).
    fn admit_new_passphrase(
        &self,
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) -> Option<ambre_crypto::strength::ChosenPassphrase> {
        ambre_crypto::strength::ChosenPassphrase::check(passphrase)
            .map_err(|weakness| {
                self.send_to_logic(LogicEvent::AuthError {
                    cause: AuthFailure::WeakPassphrase(weakness),
                });
            })
            .ok()
    }

    pub(crate) fn handle_change_passphrase(
        &mut self,
        current_passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
        new_passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) {
        tracing::info!("Sync: ChangePassphrase");
        // The new passphrase takes the floor before the current one is read,
        // so a weak choice costs no derivation and no request (`ZK-042`).
        let Some(new_passphrase) = self.admit_new_passphrase(new_passphrase) else {
            return;
        };
        // The re-seal under the new passphrase is not built yet. Both secrets
        // wipe their bytes on drop (H4).
        drop((current_passphrase, new_passphrase));
    }

    pub(crate) fn handle_recover_account(
        &mut self,
        id: Seq<Request>,
        recovery_words: Confidential<RecoveryWords>,
        new_passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) {
        tracing::info!("Sync: RecoverAccount");
        // The new passphrase takes the floor before the recovery words are
        // read: the words rebuild the seed, which a weak passphrase would
        // then seal (`ZK-042`).
        let Some(new_passphrase) = self.admit_new_passphrase(new_passphrase) else {
            return;
        };
        // The words are read whole before any key is derived: a mistyped
        // word or a bad checksum ends the recovery here (`ZK-043`).
        let master_seed = match ambre_crypto::keys::MasterSeed::from_recovery_words(&recovery_words)
        {
            Ok(master_seed) => master_seed,
            Err(refusal) => {
                self.send_to_logic(LogicEvent::AuthError {
                    cause: AuthFailure::BadRecoveryWords(refusal),
                });
                return;
            }
        };
        // The words are spent: only the seed they spell goes on (H4).
        drop(recovery_words);
        if self.event_tx.is_none() {
            // No relay holds the account, so nothing can take the re-seal.
            self.send_to_logic(LogicEvent::AuthError {
                cause: AuthFailure::Api(ApiFailure::Local(ambre_core::LocalFailure::NoRelay)),
            });
            return;
        }
        let logic_tx = self.logic_tx.clone();
        let api = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            let outcome =
                reseal_recovered_seed(&http::Fetch, &auth, &api, &master_seed, &new_passphrase)
                    .await;
            // The seed and the passphrase wipe their bytes here (H4). The
            // user unlocks under the new passphrase to open the session.
            drop((master_seed, new_passphrase));
            let evt = match outcome {
                Ok(()) => LogicEvent::Rest {
                    id: Some(id),
                    evt: RestEvent::AccountRecovered,
                },
                Err(ResealFailure::Auth(cause)) => LogicEvent::AuthError { cause },
                Err(ResealFailure::Http(e)) => auth_failed(e),
            };
            let _ = logic_tx.send(evt);
        });
    }

    /// Tear down the session (WebSocket, document sessions, keys). The
    /// `LoggedOut` response -- when there is a request to answer -- is sent by the
    /// caller (`Shutdown` performs the teardown without replying).
    pub(crate) fn handle_logout(&mut self) {
        if let Some(ws) = self.ws.take() {
            let _ = ws.close();
        }
        self.mock_ws_tx = None;
        self._onmessage = None;
        self._onclose = None;
        self._onerror = None;
        self.documents.clear();
        self.session_nonces.clear();
        self.light.clear();
        self.master_seed = None;
        self.signing_key = None;
        self.exchange_key = None;
        *self.tokens.borrow_mut() = None;
        self.renewal.cancel();
        tracing::info!("Sync: logged out, all keys zeroized");
    }

    /// A sign-out the user asked for. The session ends on the worker at once,
    /// then the relay revokes its refresh token (`SYNC-177`), and only then
    /// does `LoggedOut` answer. A failed revocation still signs out: the keys
    /// are gone already, and the token expires on its own.
    pub(crate) fn handle_sign_out(&mut self, id: Seq<Request>) {
        let held = self.tokens.borrow_mut().take();
        self.handle_logout();
        let (Some(tokens), Some(_)) = (held, self.event_tx.as_ref()) else {
            self.send_to_logic(LogicEvent::Rest {
                id: Some(id),
                evt: RestEvent::LoggedOut,
            });
            return;
        };
        // A context of its own, so a sign-in that follows at once starts
        // from an empty store.
        let auth = http::AuthContext {
            api_base: self.api_base.clone(),
            url_scheme: self.url_scheme,
            tokens: Rc::new(RefCell::new(Some(tokens))),
            renewing: Rc::new(futures_util::lock::Mutex::new(())),
            renewed: None,
        };
        let logic_tx = self.logic_tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = auth.revoke().await {
                tracing::warn!("Sync: the refresh token was not revoked: {e}");
            }
            let _ = logic_tx.send(LogicEvent::Rest {
                id: Some(id),
                evt: RestEvent::LoggedOut,
            });
        });
    }
}

/// The token pair an unlock announces once `/auth/me` answered. That request
/// presented the access token, so only a sign-out during the await empties
/// the store. The unlock then fails and announces no session: an empty token
/// is no bearer.
fn session_tokens(store: &http::TokenStore) -> Result<http::TokenPair, Box<LogicEvent>> {
    store.borrow().as_ref().cloned().ok_or_else(|| {
        Box::new(LogicEvent::AuthError {
            cause: AuthFailure::SessionEnded,
        })
    })
}

#[cfg(test)]
mod unlock_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::http::{Credential, TokenPair};

    /// A sign-out during `/auth/me` empties the store: the unlock fails, and
    /// no token pair reaches the session.
    #[test]
    fn an_unlock_whose_store_emptied_fails_with_no_session() {
        let store: http::TokenStore = Rc::new(RefCell::new(None));
        let Err(failure) = session_tokens(&store) else {
            panic!("an empty store announced a session");
        };
        let LogicEvent::AuthError { cause } = *failure else {
            panic!("the failure is {failure:?}");
        };
        assert_eq!(cause, AuthFailure::SessionEnded);
    }

    /// A held pair is the one the session announces, shared with the store.
    #[test]
    fn an_unlock_announces_the_pair_the_store_holds() {
        let pair = TokenPair::new(
            Credential::new("acc-s3cr3t".to_owned()),
            Credential::new("ref-s3cr3t".to_owned()),
            ambre_core::TimeSpan::<ambre_core::Sec>::new(900),
        );
        let store: http::TokenStore = Rc::new(RefCell::new(Some(pair.clone())));
        let announced = session_tokens(&store).expect("the store holds a pair");
        assert!(announced.access_token().matches(pair.access_token()));
        assert!(announced.refresh_token().matches(pair.refresh_token()));
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::http::{Credential, TokenPair};

    fn worker() -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        (
            SyncState::new(
                sink,
                crate::http::Url::<crate::RelayHost>::new(String::new()),
            ),
            rx,
        )
    }

    fn signed_in(keys_stored: ambre_bus::KeySetup) -> SyncEvent {
        SyncEvent::ProviderSignedIn {
            id: ambre_bus::next_request(),
            user_id: Id::<User>::mint(),
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").unwrap(),
            email: "ada@example.org".into(),
            keys_stored,
            tokens: TokenPair::new(
                Credential::new("access-s3cr3t".to_owned()),
                Credential::new("refresh-s3cr3t".to_owned()),
                ambre_core::TimeSpan::<ambre_core::Sec>::new(900),
            ),
        }
    }

    /// A provider sign-in leaves its tokens on the worker (`CHROME-096`). What
    /// Logic hears of it names the person, and carries no byte of a token.
    /// No key is held yet, so no socket opens.
    #[test]
    fn a_provider_sign_in_keeps_its_tokens_on_the_worker() {
        let (mut state, rx) = worker();
        state.handle_sync_event(signed_in(ambre_bus::KeySetup::Stored));

        let stored = state.tokens.borrow();
        let stored = stored.as_ref().expect("the worker holds the session");
        assert_eq!(stored.access_token().secret().expose(), "access-s3cr3t");
        assert_eq!(stored.refresh_token().secret().expose(), "refresh-s3cr3t");
        assert_eq!(state.socket_opens.get(), 0);
        assert!(!state.setup_owed);

        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(told.iter().any(|e| matches!(
            e,
            LogicEvent::Rest {
                evt: RestEvent::ProviderSignedIn {
                    keys_stored: ambre_bus::KeySetup::Stored,
                    display_name,
                    ..
                },
                ..
            } if display_name.as_str() == "Ada"
        )));
        let heard = format!("{told:?}");
        assert!(!heard.contains("s3cr3t"), "a token reached Logic: {heard}");
    }

    /// The message Logic heard of a sign-in that had no relay to ask.
    fn no_relay(rx: &ambre_bus::conduit::Receiver<LogicEvent>) -> bool {
        rx.try_iter().any(|event| {
            matches!(
                event,
                LogicEvent::AuthError {
                    cause: AuthFailure::Api(ApiFailure::Local(ambre_core::LocalFailure::NoRelay))
                }
            )
        })
    }

    /// With no relay to ask, a provider sign-in and a provider return each
    /// fail at once, and say so. No answer would ever come.
    #[test]
    fn a_provider_sign_in_with_no_relay_fails_at_once() {
        let (mut state, rx) = worker();
        state.handle_provider_sign_in(ambre_bus::next_request(), ambre_core::OidcProvider::Google);
        assert!(no_relay(&rx), "the sign-in");
        state.handle_provider_return(
            ambre_bus::next_request(),
            ambre_core::OidcProvider::Google,
            ambre_core::Credential::new("code".to_owned()),
            ambre_core::Credential::new("state".to_owned()),
        );
        assert!(no_relay(&rx), "the return");
    }

    /// With no relay, a development sign-in adopts the identity its username
    /// derives, and answers that the session is ready.
    #[test]
    fn a_dev_login_with_no_relay_adopts_the_identity_of_its_username() {
        let (mut state, rx) = worker();
        let name = ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").unwrap();
        state.handle_dev_login(Some(ambre_bus::next_request()), name.clone());
        let derived = ambre_crypto::hash::dev_seed(&name).derive_signing_key();
        assert_eq!(
            state.signing_key.as_ref().map(|key| key.verifying_key()),
            Some(derived.verifying_key())
        );
        assert!(rx.try_iter().any(|event| matches!(
            event,
            LogicEvent::Rest {
                evt: RestEvent::SessionReady { display_name, .. },
                ..
            } if display_name.as_str() == "Ada"
        )));
    }

    /// An account that owes its keys is marked so, which keeps the socket
    /// closed until the setup stores them.
    #[test]
    fn a_provider_sign_in_without_keys_marks_the_setup_owed() {
        let (mut state, _rx) = worker();
        state.handle_sync_event(signed_in(ambre_bus::KeySetup::Absent));
        assert!(state.setup_owed);
        state.handle_sync_event(SyncEvent::ReconnectNow);
        assert_eq!(state.socket_opens.get(), 0);
    }
}

/// Why a recovery wrote no re-seal.
enum ResealFailure {
    /// The worker refused the step itself: foreign words, or a seal that
    /// failed.
    Auth(AuthFailure),
    /// A request to the relay failed.
    Http(http::HttpError),
}

/// Open the Master Seed the relay stores under `passphrase`. The unlock reads
/// the seed through this one path, so a re-seal it cannot open fails a test
/// rather than a user (`SYNC-248`).
///
/// An error names its kind alone. Crypto detail would leak the shape of the
/// account to whatever renders it: an algorithm name, a key length, the cause
/// of a failed open. The worker logs the cause instead.
fn open_stored_seed(
    stored: &http::MasterKeyResponse,
    passphrase: &ambre_core::Confidential<ambre_core::Passphrase>,
) -> Result<MasterSeed, AuthFailure> {
    let (Some(master_key_enc), Some(salt)) = (&stored.master_key_enc, &stored.master_key_salt)
    else {
        tracing::warn!("Sync: master key without its salt");
        return Err(AuthFailure::CorruptAccount);
    };
    let user_key = ambre_crypto::passphrase::derive_user_key(passphrase, salt).map_err(|e| {
        tracing::warn!("Sync: user-key derivation failed: {e}");
        AuthFailure::BadPassphrase
    })?;
    // The seed opens straight into its wrapper, so no bare copy of it is left
    // on this path (`ZK-019`).
    MasterSeed::open_from_storage(
        master_key_enc,
        &user_key,
        &ambre_core::Aad::of(b"ambre-master-seed"),
    )
    .map_err(|e| match e {
        // Almost always the wrong passphrase; a tampered row reads the same.
        ambre_crypto::CryptoError::AeadFailed => {
            tracing::warn!("Sync: master-seed AEAD open failed: {e}");
            AuthFailure::BadPassphrase
        }
        e => {
            tracing::warn!("Sync: the stored master seed does not open: {e}");
            AuthFailure::CorruptAccount
        }
    })
}

/// Seal `master_seed` under `passphrase` and a fresh salt, as the relay
/// stores it. The setup and the recovery seal through this one path, and the
/// seed never leaves the worker in the clear (`SYNC-248`). Each step fails only
/// on a broken platform or a bug.
fn seal_seed(
    master_seed: &MasterSeed,
    passphrase: &ambre_crypto::strength::ChosenPassphrase,
) -> Result<http::SealedSeed, AuthFailure> {
    let mut salt = [0u8; 32];
    getrandom::fill(&mut salt).map_err(|e| {
        tracing::warn!("Sync: no CSPRNG for the account salt: {e}");
        AuthFailure::SeedNotWrapped
    })?;
    let master_key_salt = ambre_core::FixedBytes::<ambre_core::MasterKeySalt>::from_array(salt);
    let sealing_key = ambre_crypto::passphrase::derive_sealing_key(passphrase, &master_key_salt)
        .map_err(|e| {
            tracing::warn!("Sync: user-key derivation failed: {e}");
            AuthFailure::SeedNotWrapped
        })?;
    let master_key_enc = master_seed
        .wrap_for_storage(&sealing_key, &ambre_core::Aad::of(b"ambre-master-seed"))
        .map_err(|e| {
            tracing::warn!("Sync: master-seed wrap failed: {e}");
            AuthFailure::SeedNotWrapped
        })?;
    Ok(http::SealedSeed {
        master_key_enc,
        master_key_salt,
    })
}

/// Store the seed a recovery read back, sealed under the new passphrase.
///
/// The worker first reads the master material of the account. It refuses
/// words whose seed derives another signing key than the relay holds, before
/// any write. The re-seal moves no public key, so a foreign seed would leave
/// the account unopenable (`SYNC-249`). The one write carries the sealed seed
/// and its salt alone (`SYNC-248`, `SYNC-250`).
async fn reseal_recovered_seed(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    api: &crate::http::Url<crate::http::ApiRoot>,
    master_seed: &MasterSeed,
    new_passphrase: &ambre_crypto::strength::ChosenPassphrase,
) -> Result<(), ResealFailure> {
    let stored: http::MasterKeyResponse =
        http::get_json_auth_via(transport, &api.route("/keys/master"), auth)
            .await
            .map_err(ResealFailure::Http)?;
    let own = ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(
        master_seed.derive_signing_key().verifying_key().to_bytes(),
    );
    if stored.signing_pubkey.as_ref() != Some(&own) {
        return Err(ResealFailure::Auth(AuthFailure::ForeignRecoveryWords));
    }
    let sealed = seal_seed(master_seed, new_passphrase).map_err(ResealFailure::Auth)?;
    http::send_json_auth_via(
        transport,
        http::Method::Put,
        &api.route("/keys/master/rotate"),
        &sealed,
        auth,
    )
    .await
    .map_err(ResealFailure::Http)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::scripted::{Scripted, session};

    fn root() -> crate::http::Url<crate::http::ApiRoot> {
        crate::http::Url::<crate::http::ApiRoot>::of(
            crate::http::UrlScheme::Https,
            &crate::http::Url::<crate::http::RelayHost>::new("relay.test".to_owned()),
        )
    }

    const NEW_PASSPHRASE: &str = "anchovy glider ruckus tadpole";

    fn new_passphrase() -> ambre_core::Confidential<ambre_core::Passphrase> {
        ambre_core::Confidential::<ambre_core::Passphrase>::new(NEW_PASSPHRASE.to_owned())
    }

    /// The seed of the first BIP39 vector, read back from its words.
    fn recovered_seed() -> MasterSeed {
        let words = ambre_core::Confidential::<ambre_core::RecoveryWords>::new(format!(
            "{}art",
            "abandon ".repeat(23)
        ));
        MasterSeed::from_recovery_words(&words).expect("valid words")
    }

    fn signing_pubkey(seed: &MasterSeed) -> ambre_core::FixedBytes<ambre_core::PublicKey> {
        ambre_core::FixedBytes::from_array(seed.derive_signing_key().verifying_key().to_bytes())
    }

    /// The master material the relay answers, under the signing key `signer`.
    fn stored(signer: Option<ambre_core::FixedBytes<ambre_core::PublicKey>>) -> serde_json::Value {
        serde_json::json!({
            "master_key_enc": ambre_core::Encoded::<ambre_core::WrappedMasterKey>::new(vec![1; 72]),
            "master_key_salt": ambre_core::FixedBytes::<ambre_core::MasterKeySalt>::from_array([2; 32]),
            "signing_pubkey": signer,
            "exchange_pubkey": null,
        })
    }

    /// `SYNC-248`: a recovery reads the account, then writes the seed sealed
    /// under the new passphrase. The write holds the sealed seed and its salt
    /// alone, and the unlock path opens it under the new passphrase.
    #[tokio::test]
    async fn a_recovery_re_seals_the_seed_that_a_later_unlock_opens() {
        let seed = recovered_seed();
        let transport = Scripted::new([
            Ok((200, stored(Some(signing_pubkey(&seed))))),
            Ok((200, serde_json::json!({ "status": "rotated" }))),
        ]);
        let auth = session("access-1", "refresh-1");
        let chosen =
            ambre_crypto::strength::ChosenPassphrase::check(new_passphrase()).expect("strong");

        let outcome = reseal_recovered_seed(&transport, &auth, &root(), &seed, &chosen).await;
        assert!(outcome.is_ok());
        assert_eq!(
            transport.calls(),
            [
                (
                    http::Method::Get,
                    "https://relay.test/api/keys/master".to_owned()
                ),
                (
                    http::Method::Put,
                    "https://relay.test/api/keys/master/rotate".to_owned()
                ),
            ]
        );

        let sent = transport.sent.borrow();
        let body: serde_json::Value =
            serde_json::from_slice(sent[1].body.as_ref().expect("a body").1.as_bytes())
                .expect("json");
        let fields: Vec<&str> = body
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(fields, ["master_key_enc", "master_key_salt"]);

        // The unlock reads the stored material through `open_stored_seed`.
        let resealed: http::MasterKeyResponse = serde_json::from_value(body).expect("material");
        let opened = open_stored_seed(&resealed, &new_passphrase()).expect("the unlock opens");
        assert_eq!(signing_pubkey(&opened), signing_pubkey(&seed));
        let wrong = ambre_core::Confidential::<ambre_core::Passphrase>::new("not it".to_owned());
        assert!(matches!(
            open_stored_seed(&resealed, &wrong),
            Err(AuthFailure::BadPassphrase)
        ));
    }

    /// `SYNC-249`: words whose seed is not the account's, and an account that
    /// holds no key, draw the refusal after the one read. Nothing is written.
    #[tokio::test]
    async fn recovery_words_of_another_account_write_nothing() {
        let seed = recovered_seed();
        for signer in [Some(signing_pubkey(&MasterSeed::generate())), None] {
            let transport = Scripted::new([
                Ok((200, stored(signer))),
                Ok((200, serde_json::json!({ "status": "rotated" }))),
            ]);
            let auth = session("access-1", "refresh-1");
            let chosen =
                ambre_crypto::strength::ChosenPassphrase::check(new_passphrase()).expect("strong");

            let outcome = reseal_recovered_seed(&transport, &auth, &root(), &seed, &chosen).await;
            assert!(matches!(
                outcome,
                Err(ResealFailure::Auth(AuthFailure::ForeignRecoveryWords))
            ));
            assert_eq!(
                transport.calls(),
                [(
                    http::Method::Get,
                    "https://relay.test/api/keys/master".to_owned()
                )]
            );
        }
    }
}
