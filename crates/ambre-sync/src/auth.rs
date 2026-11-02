use super::*;

impl SyncState {
    // -- Session handlers ------------------------------------------------------

    pub(crate) fn handle_unlock(
        &mut self,
        id: Generation<Request>,
        passphrase: ambre_core::Passphrase,
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
                        // User-facing AuthError messages are deliberately generic.
                        // They never carry crypto detail (algorithm names, key/salt
                        // lengths, decrypt-failure specifics) that would leak the
                        // account's cryptographic shape to whatever renders it. The
                        // concrete cause is logged locally instead.
                        const CORRUPT: &str = "Unlock failed: account data is corrupted";
                        const BAD_PASS: &str =
                            "Unlock failed: incorrect passphrase or corrupted account data";

                        let (Some(master_key_enc), Some(salt)) =
                            (&resp.master_key_enc, &resp.master_key_salt)
                        else {
                            tracing::warn!("Sync: master key without its salt");
                            let _ = logic_tx.send(LogicEvent::AuthError {
                                message: CORRUPT.into(),
                            });
                            return;
                        };
                        let user_key =
                            match ambre_crypto::passphrase::derive_user_key(&passphrase, salt) {
                                Ok(k) => k,
                                Err(e) => {
                                    tracing::warn!("Sync: user-key derivation failed: {e}");
                                    let _ = logic_tx.send(LogicEvent::AuthError {
                                        message: BAD_PASS.into(),
                                    });
                                    return;
                                }
                            };
                        // The seed opens straight into its wrapper, so no bare
                        // copy of it is left on this path (`ZK-019`).
                        let master_seed = match MasterSeed::open_from_storage(
                            master_key_enc,
                            &user_key,
                            &ambre_core::Aad::of(b"ambre-master-seed"),
                        ) {
                            Ok(seed) => seed,
                            Err(e @ ambre_crypto::CryptoError::AeadFailed) => {
                                // Almost always the wrong passphrase (AEAD auth
                                // failure); could also be a tampered/corrupt row.
                                tracing::warn!("Sync: master-seed AEAD open failed: {e}");
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: BAD_PASS.into(),
                                });
                                return;
                            }
                            Err(e) => {
                                tracing::warn!("Sync: the stored master seed does not open: {e}");
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: CORRUPT.into(),
                                });
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
                    display_name: ambre_core::DisplayName::cut_to_fit("offline")
                        .unwrap_or_else(|| ambre_core::DisplayName::handle_of(&user_id)),
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
        id: Option<Generation<Request>>,
        username: ambre_core::DisplayName,
    ) {
        tracing::info!("Sync: DevLogin (username={username})");
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
                            tokens: http::TokenPair {
                                access_token: resp.access_token,
                                refresh_token: resp.refresh_token,
                                access_expires_in: resp.access_expires_in,
                            },
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
        id: Generation<Request>,
        provider: ambre_core::OidcProvider,
    ) {
        tracing::info!("Sync: ProviderSignIn ({provider})");
        if self.event_tx.is_none() {
            self.send_to_logic(LogicEvent::AuthError {
                message: "Sign-in failed: no relay to ask".into(),
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
        id: Generation<Request>,
        provider: ambre_core::OidcProvider,
        code: ambre_core::Credential<ambre_core::OidcCode>,
        state: ambre_core::Credential<ambre_core::OidcState>,
    ) {
        tracing::info!("Sync: ProviderReturn ({provider})");
        let Some(tx) = self.event_tx.clone() else {
            self.send_to_logic(LogicEvent::AuthError {
                message: "Sign-in failed: no relay to ask".into(),
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
                tokens: Rc::new(RefCell::new(Some(http::TokenPair {
                    access_token: resp.access_token,
                    refresh_token: resp.refresh_token,
                    access_expires_in: resp.access_expires_in,
                }))),
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
        id: Generation<Request>,
        passphrase: ambre_core::Passphrase,
        email: ambre_core::Email,
        display_name: ambre_core::DisplayName,
    ) {
        tracing::info!("Sync: SetupAccount");

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
                // Each step below fails only on a broken platform or a bug. The
                // account is not created then, and the user reads why.
                let mut salt = [0u8; 32];
                let wrapped = getrandom::fill(&mut salt)
                    .map_err(|e| format!("no CSPRNG for the account salt: {e}"))
                    .map(|()| ambre_core::FixedBytes::<ambre_core::MasterKeySalt>::from_array(salt))
                    .and_then(|salt| {
                        ambre_crypto::passphrase::derive_user_key(&passphrase, &salt)
                            .map(|user_key| (salt, user_key))
                            .map_err(|e| format!("user-key derivation failed: {e}"))
                    })
                    .and_then(|(salt, user_key)| {
                        master_seed
                            .wrap_for_storage(
                                &user_key.document_key(),
                                &ambre_core::Aad::of(b"ambre-master-seed"),
                            )
                            .map(|wrapped| (salt, wrapped))
                            .map_err(|e| format!("master-seed wrap failed: {e}"))
                    });
                let (master_key_salt, master_key_enc) = match wrapped {
                    Ok(pair) => pair,
                    Err(message) => {
                        tracing::warn!("Sync: {message}");
                        let _ = logic_tx.send(LogicEvent::AuthError { message });
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
                            tokens: http::TokenPair {
                                access_token: resp.access_token,
                                refresh_token: resp.refresh_token,
                                access_expires_in: resp.access_expires_in,
                            },
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

    pub(crate) fn handle_change_passphrase(
        &mut self,
        current_passphrase: ambre_core::Passphrase,
        new_passphrase: ambre_core::Passphrase,
    ) {
        tracing::info!("Sync: ChangePassphrase");
        // Placeholder until the passphrase-rotation flow lands. The secrets are
        // wiped when they leave scope here. `SecretBytes` zeroizes its bytes on
        // drop, so binding-and-dropping no longer leaves passphrase material in
        // freed memory (H4).
        drop((current_passphrase, new_passphrase));
    }

    pub(crate) fn handle_recover_account(
        &mut self,
        recovery_words: SecretString,
        new_passphrase: ambre_core::Passphrase,
    ) {
        tracing::info!("Sync: RecoverAccount");
        // Placeholder until account recovery lands. The BIP39 recovery phrase
        // (master-key equivalent) and the new passphrase are zeroized on drop by
        // `SecretString` / `SecretBytes` (H4).
        drop((recovery_words, new_passphrase));
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
        tracing::info!("Sync: logged out, all keys zeroized");
    }

    /// A sign-out the user asked for. The session ends on the worker at once,
    /// then the relay revokes its refresh token (`SYNC-177`), and only then
    /// does `LoggedOut` answer. A failed revocation still signs out: the keys
    /// are gone already, and the token expires on its own.
    pub(crate) fn handle_sign_out(&mut self, id: Generation<Request>) {
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
            message: "Unlock failed: the session ended".into(),
        })
    })
}

#[cfg(test)]
mod unlock_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::http::{Credential, TokenPair};
    use ambre_core::{Sec, TimeSpan};

    /// A sign-out during `/auth/me` empties the store: the unlock fails, and
    /// no token pair reaches the session.
    #[test]
    fn an_unlock_whose_store_emptied_fails_with_no_session() {
        let store: http::TokenStore = Rc::new(RefCell::new(None));
        let Err(failure) = session_tokens(&store) else {
            panic!("an empty store announced a session");
        };
        let LogicEvent::AuthError { message } = *failure else {
            panic!("the failure is {failure:?}");
        };
        assert_eq!(message, "Unlock failed: the session ended");
    }

    /// A held pair is the one the session announces, shared with the store.
    #[test]
    fn an_unlock_announces_the_pair_the_store_holds() {
        let pair = TokenPair {
            access_token: Credential::new("acc-s3cr3t".to_owned()),
            refresh_token: Credential::new("ref-s3cr3t".to_owned()),
            access_expires_in: TimeSpan::<Sec>::new(60),
        };
        let store: http::TokenStore = Rc::new(RefCell::new(Some(pair.clone())));
        let announced = session_tokens(&store).expect("the store holds a pair");
        assert!(announced.access_token.matches(&pair.access_token));
        assert!(announced.refresh_token.matches(&pair.refresh_token));
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::http::{Credential, TokenPair};
    use ambre_core::{Sec, TimeSpan};

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
            display_name: ambre_core::DisplayName::new("Ada").unwrap(),
            email: "ada@example.org".into(),
            keys_stored,
            tokens: TokenPair {
                access_token: Credential::new("access-s3cr3t".to_owned()),
                refresh_token: Credential::new("refresh-s3cr3t".to_owned()),
                access_expires_in: TimeSpan::<Sec>::new(60),
            },
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
        assert_eq!(stored.access_token.secret().expose(), "access-s3cr3t");
        assert_eq!(stored.refresh_token.secret().expose(), "refresh-s3cr3t");
        assert_eq!(state.socket_opens, 0);
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

    /// An account that owes its keys is marked so, which keeps the socket
    /// closed until the setup stores them.
    #[test]
    fn a_provider_sign_in_without_keys_marks_the_setup_owed() {
        let (mut state, _rx) = worker();
        state.handle_sync_event(signed_in(ambre_bus::KeySetup::Absent));
        assert!(state.setup_owed);
        state.handle_sync_event(SyncEvent::ReconnectNow);
        assert_eq!(state.socket_opens, 0);
    }
}
