use super::*;
use ambre_bus::AuthFailure;
use ambre_core::Flag as _;
use ambre_protocol::master_rotation::{MasterRotation, SealedSeed};

impl SyncState {
    // -- Session handlers ------------------------------------------------------

    /// Open the keys the relay stores for the signed-in account under
    /// `passphrase`. An unlock creates no key: an account that owes its keys
    /// hears that a setup is owed, and `SetUpKeys` alone creates them
    /// (`SYNC-270`, `SYNC-134`).
    pub(crate) fn handle_unlock(
        &mut self,
        id: Seq<Request>,
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) {
        crate::log::info!("Sync: Unlock");
        let Some(tx) = self.event_tx.clone() else {
            // No relay stores the keys, so there is nothing to open.
            drop(passphrase);
            self.send_to_logic(LogicEvent::AuthError {
                cause: AuthFailure::Api(ApiFailure::Local(ambre_core::LocalFailure::NoRelay)),
            });
            return;
        };
        let logic_tx = self.logic_tx.clone();
        let api = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        crate::spawn(async move {
            let outcome = open_keys(&http::Fetch, &auth, &api, &passphrase).await;
            // The passphrase wipes its bytes here (H4).
            drop(passphrase);
            match outcome {
                Ok(OpenedKeys { master_seed, me }) => {
                    let tokens = match session_tokens(&auth.tokens) {
                        Ok(tokens) => tokens,
                        Err(failure) => {
                            let _ = logic_tx.send(*failure);
                            return;
                        }
                    };
                    let _ = tx.send(SyncEvent::SessionReady {
                        id: Some(id),
                        user_id: me.user_id,
                        display_name: me.display_name,
                        email: me.email,
                        signing_key: master_seed.derive_signing_key(),
                        exchange_key: master_seed.derive_exchange_key(),
                        master_seed,
                        tokens,
                    });
                }
                // The same answer as a request the relay refused for a setup
                // owed: the client shows the setup, never an error.
                Err(UnlockFailure::SetupOwed) => {
                    let _ = logic_tx.send(LogicEvent::SetupRequired);
                }
                Err(UnlockFailure::Auth(cause)) => {
                    let _ = logic_tx.send(LogicEvent::AuthError { cause });
                }
                Err(UnlockFailure::Http(e)) => {
                    let _ = logic_tx.send(auth_failed(e));
                }
            }
        });
    }

    /// Dev-only login: authenticate as `username` with keys derived
    /// deterministically from the username (no passphrase, no master-key
    /// round-trip). The server creates the user on the fly and is the sole gate
    /// (it rejects this unless `DEV_MODE` is set). Lets `?user=<name>` test
    /// multi-user behaviour on one machine; an edit survives a refresh because
    /// the same username always derives the same identity.
    /// `id` is `None` for the silent re-login, which no request asked for.
    /// Behind the `dev-login` feature, which no release build turns on
    /// (`ZK-045`).
    #[cfg(feature = "dev-login")]
    // The feature twin: the mutation run builds no feature, so it compiles
    // none of its mutants. `just test-native` runs it with the feature.
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn handle_dev_login(
        &mut self,
        id: Option<Seq<Request>>,
        username: ambre_core::Validated<ambre_core::DisplayName>,
    ) {
        crate::log::info!("Sync: DevLogin (username={username:?})");
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
            crate::spawn(async move {
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
            // The user it announces is the one it holds, so an opened
            // document gets a session identity (`AUTHOR-002`).
            let user_id = *self.user_id.get_or_insert_with(Id::<User>::mint);
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

    /// A build without the `dev-login` feature holds no development sign-in
    /// (`ZK-045`). The command derives no key, sends nothing to the relay
    /// and keeps no name, whatever the relay says of its development mode.
    /// The refusal tells the page why no session follows.
    #[cfg(not(feature = "dev-login"))]
    pub(crate) fn handle_dev_login(
        &mut self,
        _id: Option<Seq<Request>>,
        _username: ambre_core::Validated<ambre_core::DisplayName>,
    ) {
        crate::log::warning!("Sync: DevLogin refused, this build holds no development sign-in");
        self.send_to_logic(LogicEvent::AuthError {
            cause: AuthFailure::NoDevSignIn,
        });
    }

    /// The name a development session signed in with, kept for the silent
    /// re-login after a failed refresh. `None` in a provider session.
    #[cfg(feature = "dev-login")]
    // The feature twin, skipped for the reason `handle_dev_login` gives.
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn dev_user(&self) -> Option<ambre_core::Validated<ambre_core::DisplayName>> {
        self.dev_username.clone()
    }

    /// A build without the `dev-login` feature holds no development name to
    /// sign in again with (`ZK-045`).
    #[cfg(not(feature = "dev-login"))]
    pub(crate) fn dev_user(&self) -> Option<ambre_core::Validated<ambre_core::DisplayName>> {
        None
    }

    /// Ask the relay for `provider`'s authorization address and hand it to
    /// the UI, which sends the browser there (`CHROME-095`). Unauthenticated:
    /// no session exists yet.
    pub(crate) fn handle_provider_sign_in(
        &mut self,
        id: Seq<Request>,
        provider: ambre_core::OidcProvider,
    ) {
        crate::log::info!("Sync: ProviderSignIn ({provider})");
        if self.event_tx.is_none() {
            self.send_to_logic(LogicEvent::AuthError {
                cause: AuthFailure::Api(ApiFailure::Local(ambre_core::LocalFailure::NoRelay)),
            });
            return;
        }
        let logic_tx = self.logic_tx.clone();
        let url = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base)
            .route(format!("/auth/{provider}/login"));
        crate::spawn(async move {
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
        crate::log::info!("Sync: ProviderReturn ({provider})");
        let Some(tx) = self.event_tx.clone() else {
            self.send_to_logic(LogicEvent::AuthError {
                cause: AuthFailure::Api(ApiFailure::Local(ambre_core::LocalFailure::NoRelay)),
            });
            return;
        };
        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        crate::spawn(async move {
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

    /// Set up the keys of the account this session signed in with the
    /// provider (`CHROME-098`). The floor runs first. A new seed is sealed
    /// under the passphrase and stored on that account. The worker then
    /// answers the recovery words, and the session opens (`SYNC-255`).
    ///
    /// A store the relay refuses because the account holds keys already
    /// answers with those keys, when the passphrase opens them (`SYNC-285`).
    pub(crate) fn handle_set_up_keys(
        &mut self,
        id: Seq<Request>,
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) {
        crate::log::info!("Sync: SetUpKeys");
        // A clone shares the one wiping value: it opens the keys a store
        // found stored already, and nothing else.
        let typed = passphrase.clone();
        let Some(passphrase) = self.admit_new_passphrase(passphrase) else {
            return;
        };
        let (Some(tx), Some(account)) = (self.event_tx.clone(), self.user_id) else {
            // No relay holds the account, or no sign-in named it, so nothing
            // can take the keys.
            let cause = if self.event_tx.is_none() {
                ambre_core::LocalFailure::NoRelay
            } else {
                ambre_core::LocalFailure::NotSignedIn
            };
            self.send_to_logic(LogicEvent::AuthError {
                cause: AuthFailure::Api(ApiFailure::Local(cause)),
            });
            return;
        };
        let logic_tx = self.logic_tx.clone();
        let api = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base);
        let auth = self.auth_context();
        crate::spawn(async move {
            let new_keys = NewKeys {
                account,
                master_seed: MasterSeed::generate(),
                passphrase: &passphrase,
                typed: &typed,
            };
            let outcome = store_new_keys(&http::Fetch, &auth, &api, new_keys).await;
            // The passphrase wipes its bytes here (H4).
            drop((passphrase, typed));
            match outcome {
                Ok(StoredKeys {
                    master_seed,
                    tokens,
                    me,
                }) => {
                    let _ = tx.send(SyncEvent::KeysSetUp {
                        id,
                        recovery_words: master_seed.recovery_words(),
                        signing_key: master_seed.derive_signing_key(),
                        exchange_key: master_seed.derive_exchange_key(),
                        master_seed,
                        tokens,
                        user_id: me.user_id,
                        display_name: me.display_name,
                        email: me.email,
                    });
                }
                Err(KeyWriteFailure::Auth(cause)) => {
                    let _ = logic_tx.send(LogicEvent::AuthError { cause });
                }
                Err(KeyWriteFailure::Http(e)) => {
                    let _ = logic_tx.send(auth_failed(e));
                }
            }
        });
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
        id: Seq<Request>,
        current_passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
        new_passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) {
        crate::log::info!("Sync: ChangePassphrase");
        // The new passphrase takes the floor before the current one is read,
        // so a weak choice costs no derivation and no request (`ZK-042`).
        let Some(new_passphrase) = self.admit_new_passphrase(new_passphrase) else {
            return;
        };
        let Some((auth, api)) = self.reseal_target() else {
            return;
        };
        self.spawn_reseal(id, RestEvent::PassphraseChanged, async move {
            let outcome = reseal_changed_passphrase(
                &http::Fetch,
                &auth,
                &api,
                &current_passphrase,
                &new_passphrase,
            )
            .await;
            // Both passphrases wipe their bytes here (H4). The user unlocks
            // under the new passphrase from now on.
            drop((current_passphrase, new_passphrase));
            outcome
        });
    }

    pub(crate) fn handle_recover_account(
        &mut self,
        id: Seq<Request>,
        recovery_words: Confidential<RecoveryWords>,
        new_passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) {
        crate::log::info!("Sync: RecoverAccount");
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
        let Some((auth, api)) = self.reseal_target() else {
            return;
        };
        self.spawn_reseal(id, RestEvent::AccountRecovered, async move {
            let outcome =
                reseal_recovered_seed(&http::Fetch, &auth, &api, &master_seed, &new_passphrase)
                    .await;
            // The seed and the passphrase wipe their bytes here (H4). The
            // user unlocks under the new passphrase to open the session.
            drop((master_seed, new_passphrase));
            outcome
        });
    }

    /// The session and the API root a re-seal writes to. With no relay, the
    /// worker answers the refusal itself and `None` ends the caller: nothing
    /// can take the re-seal.
    fn reseal_target(&self) -> Option<(http::AuthContext, crate::http::Url<crate::http::ApiRoot>)> {
        if self.event_tx.is_none() {
            self.send_to_logic(LogicEvent::AuthError {
                cause: AuthFailure::Api(ApiFailure::Local(ambre_core::LocalFailure::NoRelay)),
            });
            return None;
        }
        let api = crate::http::Url::<crate::http::ApiRoot>::of(self.url_scheme, &self.api_base);
        Some((self.auth_context(), api))
    }

    /// Run a re-seal, then answer request `id` with `done`, or with the
    /// refusal the re-seal met. A recovery and a passphrase change answer
    /// through this one path.
    // Its whole body is a task spawned on the event loop of the browser.
    // Only a relay reaches it, and no host test runs one, as for `crate::spawn`.
    #[cfg_attr(test, mutants::skip)]
    fn spawn_reseal(
        &self,
        id: Seq<Request>,
        done: RestEvent,
        reseal: impl std::future::Future<Output = Result<(), KeyWriteFailure>> + 'static,
    ) {
        let logic_tx = self.logic_tx.clone();
        crate::spawn(async move {
            let evt = match reseal.await {
                Ok(()) => LogicEvent::Rest {
                    id: Some(id),
                    evt: done,
                },
                Err(KeyWriteFailure::Auth(cause)) => LogicEvent::AuthError { cause },
                Err(KeyWriteFailure::Http(e)) => auth_failed(e),
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
        self._onopen = None;
        self._reconnect_timer = None;
        self.lifecycle.reset();
        // A call of the session ends with it, and the call worker drops its
        // keys (`CALL-029`).
        self.end_call();
        self.forget_session();
        self.master_seed = None;
        self.signing_key = None;
        self.exchange_key = None;
        *self.tokens.borrow_mut() = None;
        self.renewal.cancel();
        // The event queue is the generation of the session. Each task the
        // session spawned holds a sender of the queue that ends here. What
        // it answers after the sign-out lands nowhere, so no document key
        // comes back (`SYNC-281`, audit `W204`).
        if self.event_tx.is_some() {
            let (tx, rx) = ambre_bus::conduit::unbounded();
            self.event_tx = Some(tx);
            self.next_events = Some(rx);
        }
        crate::log::info!("Sync: logged out, all keys zeroized");
    }

    /// Drops every map that holds something of the session. That is the
    /// document keys, the roots and pins it vouched, and the plaintext deltas
    /// buffered before an open. It is the logs, footings and names too
    /// (`SYNC-275`, audit `W204`). A sign-in that follows starts from none.
    fn forget_session(&mut self) {
        self.user_id = None;
        self.setup_owed = false;
        self.documents.clear();
        self.doc_metadata.clear();
        self.log_footings.clear();
        self.session_nonces.clear();
        self.light.clear();
        self.self_join_logs.clear();
        self.channel_metas.clear();
        self.listed_channels.clear();
        self.listing_metas.clear();
        self.listed_keys.clear();
        self.creating_public.clear();
        self.space_rotations.clear();
        self.new_spaces.clear();
        self.held_creations.clear();
        self.roster_logs.clear();
        #[cfg(not(target_arch = "wasm32"))]
        self.roster_asks.clear();
        self.pending_opens.clear();
        self.pinned_inviters.clear();
        self.murmure_roots.clear();
        self.typed_opens.clear();
        self.pending_pre_session_updates.clear();
        self.held_outbox.borrow_mut().clear();
        self.replay_stops.clear();
        self.replay_asks.borrow_mut().clear();
        self.owed_cursors = crate::cursors::OwedCursors::default();
        #[cfg(feature = "dev-login")]
        {
            self.dev_username = None;
        }
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
        crate::spawn(async move {
            if let Err(e) = auth.revoke().await {
                crate::log::warning!("Sync: the refresh token was not revoked: {e}");
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

    /// `SYNC-270`: with no relay, an unlock fails at once and holds no key.
    /// An unlock never mints the keys it did not find.
    #[test]
    fn an_unlock_with_no_relay_creates_no_key() {
        let (mut state, rx) = worker();
        state.handle_unlock(
            ambre_bus::next_request(),
            ambre_core::Confidential::<ambre_core::Passphrase>::new("anchovy glider".to_owned()),
        );
        assert!(no_relay(&rx));
        assert!(state.master_seed.is_none());
        assert!(state.signing_key.is_none());
        assert_eq!(state.socket_opens.get(), 0);
    }

    /// With no relay, a development sign-in adopts the identity its username
    /// derives, and answers that the session is ready.
    #[cfg(feature = "dev-login")]
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

    /// A build without the development sign-in refuses a sign-in by a bare
    /// name (`ZK-045`). No key is derived and the socket stays closed. A
    /// failed refresh then signs in again with no name either.
    #[cfg(not(feature = "dev-login"))]
    #[test]
    fn a_build_without_the_dev_sign_in_derives_no_key_from_a_name() {
        let (mut state, rx) = worker();
        let name = ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").unwrap();
        state.handle_command(SyncCommand::Rest {
            id: ambre_bus::next_request(),
            cmd: RestCommand::DevLogin { username: name },
        });
        assert!(state.signing_key.is_none() && state.master_seed.is_none());
        assert!(matches!(
            rx.try_iter().collect::<Vec<_>>().as_slice(),
            [LogicEvent::AuthError {
                cause: AuthFailure::NoDevSignIn
            }]
        ));
        state.handle_sync_event(SyncEvent::Reauthenticated { ok: false });
        assert!(state.signing_key.is_none());
        assert_eq!(state.socket_opens.get(), 0);
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

/// Why a write of sealed keys did not land: a recovery's re-seal, or the
/// first store of a signed-in account.
enum KeyWriteFailure {
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
        crate::log::warning!("Sync: master key without its salt");
        return Err(AuthFailure::CorruptAccount);
    };
    let user_key = ambre_crypto::passphrase::derive_user_key(passphrase, salt).map_err(|e| {
        crate::log::warning!("Sync: user-key derivation failed: {e}");
        AuthFailure::BadPassphrase
    })?;
    // The seed opens straight into its wrapper, so no bare copy of it is left
    // on this path (`ZK-019`).
    // The seal binds the salt and the signing key the relay names for the
    // account. So a seed served for another account does not open
    // (`ZK-047`). A seed sealed before that binding opens without it.
    MasterSeed::open_from_storage(
        master_key_enc,
        &user_key,
        salt,
        stored.signing_pubkey.as_ref(),
    )
    .map_err(|e| match e {
        // Almost always the wrong passphrase; a tampered row reads the same.
        ambre_crypto::CryptoError::AeadFailed => {
            crate::log::warning!("Sync: master-seed AEAD open failed: {e}");
            AuthFailure::BadPassphrase
        }
        e => {
            crate::log::warning!("Sync: the stored master seed does not open: {e}");
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
) -> Result<SealedSeed, AuthFailure> {
    let mut salt = [0u8; 32];
    getrandom::fill(&mut salt).map_err(|e| {
        crate::log::warning!("Sync: no CSPRNG for the account salt: {e}");
        AuthFailure::SeedNotWrapped
    })?;
    let master_key_salt = ambre_core::FixedBytes::<ambre_core::MasterKeySalt>::from_array(salt);
    let sealing_key = ambre_crypto::passphrase::derive_sealing_key(passphrase, &master_key_salt)
        .map_err(|e| {
            crate::log::warning!("Sync: user-key derivation failed: {e}");
            AuthFailure::SeedNotWrapped
        })?;
    let master_key_enc = master_seed
        .wrap_for_storage(&sealing_key, &master_key_salt)
        .map_err(|e| {
            crate::log::warning!("Sync: master-seed wrap failed: {e}");
            AuthFailure::SeedNotWrapped
        })?;
    Ok(SealedSeed {
        master_key_enc,
        master_key_salt,
    })
}

/// Store the seed a recovery read back, sealed under the new passphrase.
///
/// The worker first reads the master material of the account. It refuses
/// words whose seed derives another signing key than the relay holds, before
/// any write. The re-seal moves no public key, so a foreign seed would leave
/// the account unopenable (`SYNC-249`). The one write carries the sealed seed,
/// its salt and the proof of the seed's signing key (`SYNC-248`, `SYNC-250`,
/// `SYNC-254`).
async fn reseal_recovered_seed(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    api: &crate::http::Url<crate::http::ApiRoot>,
    master_seed: &MasterSeed,
    new_passphrase: &ambre_crypto::strength::ChosenPassphrase,
) -> Result<(), KeyWriteFailure> {
    let stored: http::MasterKeyResponse =
        http::get_json_auth_via(transport, &api.route("/keys/master"), auth)
            .await
            .map_err(KeyWriteFailure::Http)?;
    let own = ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(
        master_seed.derive_signing_key().verifying_key().to_bytes(),
    );
    if stored.signing_pubkey.as_ref() != Some(&own) {
        return Err(KeyWriteFailure::Auth(AuthFailure::ForeignRecoveryWords));
    }
    // The proof names the seal it replaces. An account with no sealed seed
    // has none to replace, and the relay would refuse the write.
    let Some(replaces) = stored.master_key_salt else {
        crate::log::warning!("Sync: recovery of an account with no sealed seed");
        return Err(KeyWriteFailure::Auth(AuthFailure::CorruptAccount));
    };
    let sealed = seal_seed(master_seed, new_passphrase).map_err(KeyWriteFailure::Auth)?;
    send_rotation(
        transport,
        auth,
        api,
        prove_rotation(master_seed, replaces, sealed),
    )
    .await
}

/// Store the seed sealed under a new passphrase, in place of the seal under
/// the current one.
///
/// The worker reads the master material of the account and opens the seed
/// under `current`, as the unlock does. A passphrase that does not open it is
/// refused before any write (`SYNC-252`). The one write carries the seed
/// sealed under `new`, a fresh salt and the proof of the seed's signing key
/// (`SYNC-251`, `SYNC-253`, `SYNC-254`).
async fn reseal_changed_passphrase(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    api: &crate::http::Url<crate::http::ApiRoot>,
    current: &ambre_core::Confidential<ambre_core::Passphrase>,
    new: &ambre_crypto::strength::ChosenPassphrase,
) -> Result<(), KeyWriteFailure> {
    let stored: http::MasterKeyResponse =
        http::get_json_auth_via(transport, &api.route("/keys/master"), auth)
            .await
            .map_err(KeyWriteFailure::Http)?;
    let master_seed = open_stored_seed(&stored, current).map_err(KeyWriteFailure::Auth)?;
    // The open above read the salt, so the stored material holds one.
    let Some(replaces) = stored.master_key_salt else {
        return Err(KeyWriteFailure::Auth(AuthFailure::CorruptAccount));
    };
    let sealed = seal_seed(&master_seed, new).map_err(KeyWriteFailure::Auth)?;
    send_rotation(
        transport,
        auth,
        api,
        prove_rotation(&master_seed, replaces, sealed),
    )
    .await
}

/// The re-seal of `sealed` in place of the seal salted `replaces`, signed by
/// the signing key of `master_seed` (`SYNC-254`). The relay writes it only
/// under that key, and only over that seal (`SRV-278`, `SRV-279`).
fn prove_rotation(
    master_seed: &MasterSeed,
    replaces: ambre_core::FixedBytes<ambre_core::MasterKeySalt>,
    sealed: SealedSeed,
) -> MasterRotation {
    let transcript = ambre_protocol::master_rotation::rotation_transcript(&replaces, &sealed);
    let signing_key = master_seed.derive_signing_key();
    let signature = ed25519_dalek::Signer::sign(&signing_key, transcript.as_bytes()).to_bytes();
    MasterRotation {
        sealed,
        replaces,
        signature: ambre_core::FixedBytes::from_array(signature),
    }
}

/// Send the one write of a re-seal.
async fn send_rotation(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    api: &crate::http::Url<crate::http::ApiRoot>,
    rotation: MasterRotation,
) -> Result<(), KeyWriteFailure> {
    http::send_json_auth_via(
        transport,
        http::Method::Put,
        &api.route("/keys/master/rotate"),
        &rotation,
        auth,
    )
    .await
    .map_err(KeyWriteFailure::Http)
}

/// What the first store of a signed-in account leaves the worker. That is
/// the seed the relay holds, the pair the relay bound to its signing key, and
/// who the account is.
struct StoredKeys {
    master_seed: MasterSeed,
    tokens: http::TokenPair,
    me: http::AuthMeResponse,
}

/// What a first key store sends: a new seed for `account`, sealed under
/// `passphrase`. `typed` is the same passphrase before the floor, which opens
/// keys the relay holds already.
struct NewKeys<'a> {
    account: Id<User>,
    master_seed: MasterSeed,
    passphrase: &'a ambre_crypto::strength::ChosenPassphrase,
    typed: &'a ambre_core::Confidential<ambre_core::Passphrase>,
}

/// Store a new seed on the account `auth` signed in, sealed under the
/// passphrase (`SYNC-255`).
///
/// The one write carries the sealed seed, its salt and the two public keys the
/// seed derives. It adds the proof of the signing key, and nothing else
/// (`SYNC-256`, `SRV-294`). The relay answers a fresh pair, bound to the new
/// signing key, and the identity is read under it. The stored pair is stale
/// from the write on, so it reads nothing more.
///
/// A conflict means the account holds keys already: a store that landed
/// before a reload, for example. The worker then answers with those keys
/// when the passphrase opens them, and never with a second seed (`SYNC-285`).
async fn store_new_keys(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    api: &crate::http::Url<crate::http::ApiRoot>,
    keys: NewKeys<'_>,
) -> Result<StoredKeys, KeyWriteFailure> {
    let NewKeys {
        account,
        master_seed,
        passphrase,
        typed,
    } = keys;
    let SealedSeed {
        master_key_enc,
        master_key_salt,
    } = seal_seed(&master_seed, passphrase).map_err(KeyWriteFailure::Auth)?;
    let signing_key = master_seed.derive_signing_key();
    let signing_pubkey = ambre_core::FixedBytes::from_array(signing_key.verifying_key().to_bytes());
    let exchange_pubkey = crate::sharing::own_exchange_key(&master_seed.derive_exchange_key());
    let transcript = ambre_protocol::key_registration::registration_transcript(
        &account,
        &signing_pubkey,
        &exchange_pubkey,
    );
    let signature = ed25519_dalek::Signer::sign(&signing_key, transcript.as_bytes()).to_bytes();
    let body = http::StoreMasterKeysRequest {
        master_key_enc,
        master_key_salt,
        signing_pubkey,
        exchange_pubkey,
        signature: ambre_core::FixedBytes::from_array(signature),
    };
    let stored: http::KeysStoredResponse = match http::send_json_auth_read_via(
        transport,
        http::Method::Put,
        &api.route("/keys/master"),
        &body,
        auth,
    )
    .await
    {
        Ok(stored) => stored,
        Err(refused) if refused.is_conflict() => {
            return held_keys(transport, auth, api, typed, refused).await;
        }
        Err(e) => return Err(KeyWriteFailure::Http(e)),
    };
    let fresh = http::AuthContext {
        api_base: auth.api_base.clone(),
        url_scheme: auth.url_scheme,
        tokens: Rc::new(RefCell::new(Some(stored.tokens))),
        renewing: Rc::new(futures_util::lock::Mutex::new(())),
        renewed: None,
    };
    let me: http::AuthMeResponse =
        http::get_json_auth_via(transport, &api.route("/auth/me"), &fresh)
            .await
            .map_err(KeyWriteFailure::Http)?;
    let tokens = fresh
        .tokens
        .borrow_mut()
        .take()
        .ok_or(KeyWriteFailure::Auth(AuthFailure::SessionEnded))?;
    Ok(StoredKeys {
        master_seed,
        tokens,
        me,
    })
}

/// The keys the relay holds for the account, after a first store it refused
/// as `refused` (`SYNC-285`).
///
/// The worker reads the master material and opens the seed under `typed`, as
/// an unlock does. It then renews the pair: the relay mints the next one under
/// the registered key (`SRV-307`). An account that still owes its keys fails
/// with the refusal itself. A passphrase that does not open the stored seed
/// fails as a wrong passphrase, and nothing is written.
async fn held_keys(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    api: &crate::http::Url<crate::http::ApiRoot>,
    typed: &ambre_core::Confidential<ambre_core::Passphrase>,
    refused: http::HttpError,
) -> Result<StoredKeys, KeyWriteFailure> {
    let held: http::MasterKeyResponse =
        http::get_json_auth_via(transport, &api.route("/keys/master"), auth)
            .await
            .map_err(KeyWriteFailure::Http)?;
    if held.setup_owed() {
        return Err(KeyWriteFailure::Http(refused));
    }
    let master_seed = open_stored_seed(&held, typed).map_err(KeyWriteFailure::Auth)?;
    auth.renew_held_via(transport)
        .await
        .map_err(KeyWriteFailure::Http)?;
    let me: http::AuthMeResponse = http::get_json_auth_via(transport, &api.route("/auth/me"), auth)
        .await
        .map_err(KeyWriteFailure::Http)?;
    let tokens = auth
        .tokens
        .borrow()
        .as_ref()
        .cloned()
        .ok_or(KeyWriteFailure::Auth(AuthFailure::SessionEnded))?;
    Ok(StoredKeys {
        master_seed,
        tokens,
        me,
    })
}

/// The keys an unlock opened, and the identity of their account.
struct OpenedKeys {
    master_seed: MasterSeed,
    me: http::AuthMeResponse,
}

/// Why an unlock opened no key.
enum UnlockFailure {
    /// The relay stores no key for the account: a setup is owed.
    SetupOwed,
    /// The worker refused the step itself: a wrong passphrase.
    Auth(AuthFailure),
    /// A request to the relay failed.
    Http(http::HttpError),
}

/// Read the sealed seed of the signed-in account, open it under `passphrase`,
/// then read the identity of the account (`SYNC-270`).
///
/// The unlock only reads. An account that owes its keys ends on `SetupOwed`
/// before any key exists, and nothing is written to the relay.
async fn open_keys(
    transport: &impl http::Transport,
    auth: &http::AuthContext,
    api: &crate::http::Url<crate::http::ApiRoot>,
    passphrase: &ambre_core::Confidential<ambre_core::Passphrase>,
) -> Result<OpenedKeys, UnlockFailure> {
    let stored: http::MasterKeyResponse =
        http::get_json_auth_via(transport, &api.route("/keys/master"), auth)
            .await
            .map_err(UnlockFailure::Http)?;
    if stored.setup_owed() {
        return Err(UnlockFailure::SetupOwed);
    }
    let master_seed = open_stored_seed(&stored, passphrase).map_err(UnlockFailure::Auth)?;
    let me: http::AuthMeResponse = http::get_json_auth_via(transport, &api.route("/auth/me"), auth)
        .await
        .map_err(UnlockFailure::Http)?;
    Ok(OpenedKeys { master_seed, me })
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
            "master_key_salt": stored_salt(),
            "signing_pubkey": signer,
            "exchange_pubkey": null,
        })
    }

    /// The salt of the master material `stored` answers.
    fn stored_salt() -> ambre_core::FixedBytes<ambre_core::MasterKeySalt> {
        ambre_core::FixedBytes::from_array([2; 32])
    }

    /// The names of the fields of the JSON object `value`, sorted.
    fn fields_of(value: &serde_json::Value) -> Vec<&str> {
        let mut fields: Vec<&str> = value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        fields
    }

    /// `SYNC-254`: the re-seal `body` names the seal salted `replaces`, and
    /// the signing key of `seed` signs it, as the relay verifies it.
    fn assert_proves(body: &serde_json::Value, seed: &MasterSeed, replaces: &serde_json::Value) {
        assert_eq!(&body["replaces"], replaces);
        let rotation: MasterRotation = serde_json::from_value(body.clone()).expect("a rotation");
        let key = ed25519_dalek::VerifyingKey::from_bytes(signing_pubkey(seed).as_array())
            .expect("a key");
        let signature = ed25519_dalek::Signature::from_bytes(rotation.signature.as_array());
        assert!(
            key.verify_strict(rotation.transcript().as_bytes(), &signature)
                .is_ok()
        );
        // Another key does not sign it.
        let other = ed25519_dalek::VerifyingKey::from_bytes(
            signing_pubkey(&MasterSeed::generate()).as_array(),
        )
        .expect("a key");
        assert!(
            other
                .verify_strict(rotation.transcript().as_bytes(), &signature)
                .is_err()
        );
    }

    /// `SYNC-248`, `SYNC-254`: a recovery reads the account, then writes the
    /// seed sealed under the new passphrase. The write holds the sealed seed,
    /// its salt and the proof the seed's signing key signed over the seal it
    /// replaces. The unlock path opens it under the new passphrase.
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
        assert_eq!(fields_of(&body), ["replaces", "sealed", "signature"]);
        assert_eq!(
            fields_of(&body["sealed"]),
            ["master_key_enc", "master_key_salt"]
        );
        assert_proves(&body, &seed, &serde_json::json!(stored_salt()));

        // The unlock reads the stored material through `open_stored_seed`.
        let resealed = http::MasterKeyResponse {
            signing_pubkey: Some(signing_pubkey(&seed)),
            ..serde_json::from_value(body["sealed"].clone()).expect("material")
        };

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
                Err(KeyWriteFailure::Auth(AuthFailure::ForeignRecoveryWords))
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

    const CURRENT_PASSPHRASE: &str = "pelican harbor voltage minnow";

    fn current_passphrase() -> ambre_core::Confidential<ambre_core::Passphrase> {
        ambre_core::Confidential::<ambre_core::Passphrase>::new(CURRENT_PASSPHRASE.to_owned())
    }

    /// The master material the relay answers for `seed`, sealed under the
    /// current passphrase through the seal of the setup.
    fn sealed_under_current(seed: &MasterSeed) -> serde_json::Value {
        let chosen =
            ambre_crypto::strength::ChosenPassphrase::check(current_passphrase()).expect("strong");
        let mut material =
            serde_json::to_value(seal_seed(seed, &chosen).expect("sealed")).expect("json");
        material["signing_pubkey"] = serde_json::to_value(signing_pubkey(seed)).expect("json");
        material["exchange_pubkey"] = serde_json::Value::Null;
        material
    }

    /// `SYNC-251`, `SYNC-253`, `SYNC-254`: a change opens the seed under the
    /// current passphrase. It then writes the seed sealed under the new one
    /// and a fresh salt, with the proof over the seal it replaces. The unlock path opens the write under the new passphrase
    /// alone.
    #[tokio::test]
    async fn a_passphrase_change_re_seals_the_seed_that_a_later_unlock_opens() {
        let seed = MasterSeed::generate();
        let before = sealed_under_current(&seed);
        let transport = Scripted::new([
            Ok((200, before.clone())),
            Ok((200, serde_json::json!({ "status": "rotated" }))),
        ]);
        let auth = session("access-1", "refresh-1");
        let chosen =
            ambre_crypto::strength::ChosenPassphrase::check(new_passphrase()).expect("strong");

        let outcome =
            reseal_changed_passphrase(&transport, &auth, &root(), &current_passphrase(), &chosen)
                .await;
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
        assert_eq!(fields_of(&body), ["replaces", "sealed", "signature"]);
        assert_eq!(
            fields_of(&body["sealed"]),
            ["master_key_enc", "master_key_salt"]
        );
        assert_ne!(body["sealed"]["master_key_salt"], before["master_key_salt"]);
        assert_proves(&body, &seed, &before["master_key_salt"]);

        // The unlock reads the stored material through `open_stored_seed`.
        let resealed = http::MasterKeyResponse {
            signing_pubkey: Some(signing_pubkey(&seed)),
            ..serde_json::from_value(body["sealed"].clone()).expect("material")
        };
        let opened = open_stored_seed(&resealed, &new_passphrase()).expect("the unlock opens");
        assert_eq!(signing_pubkey(&opened), signing_pubkey(&seed));
        assert!(matches!(
            open_stored_seed(&resealed, &current_passphrase()),
            Err(AuthFailure::BadPassphrase)
        ));
    }

    /// `SYNC-270`: an unlock only reads. Stored keys open after two reads; an
    /// account that owes its keys hears a setup owed after one read. Neither
    /// unlock writes a key to the relay.
    #[tokio::test]
    async fn an_unlock_opens_stored_keys_and_creates_none() {
        let seed = MasterSeed::generate();
        let user = "66666666-6666-6666-6666-666666666666";
        let transport = Scripted::new([
            Ok((200, sealed_under_current(&seed))),
            Ok((
                200,
                serde_json::json!({
                    "user_id": user,
                    "display_name": "Ada",
                    "email": "ada@example.org",
                }),
            )),
        ]);
        let auth = session("access-1", "refresh-1");
        let Ok(OpenedKeys { master_seed, me }) =
            open_keys(&transport, &auth, &root(), &current_passphrase()).await
        else {
            panic!("the unlock did not open the stored keys");
        };
        assert_eq!(signing_pubkey(&master_seed), signing_pubkey(&seed));
        assert_eq!(me.user_id.to_string(), user);
        let reads = [
            (
                http::Method::Get,
                "https://relay.test/api/keys/master".to_owned(),
            ),
            (
                http::Method::Get,
                "https://relay.test/api/auth/me".to_owned(),
            ),
        ];
        assert_eq!(transport.calls(), reads);

        let owed = serde_json::json!({
            "master_key_enc": null,
            "master_key_salt": null,
            "signing_pubkey": null,
            "exchange_pubkey": null,
        });
        let transport = Scripted::new([Ok((200, owed))]);
        let outcome = open_keys(&transport, &auth, &root(), &current_passphrase()).await;
        assert!(matches!(outcome, Err(UnlockFailure::SetupOwed)));
        assert_eq!(transport.calls(), reads[..1]);
    }

    /// `SYNC-252`: a current passphrase that does not open the stored seed,
    /// and an account that owes its setup, draw the refusal after the one
    /// read. Nothing is written.
    #[tokio::test]
    async fn a_wrong_current_passphrase_writes_nothing() {
        let seed = MasterSeed::generate();
        let owed = serde_json::json!({
            "master_key_enc": null,
            "master_key_salt": null,
            "signing_pubkey": null,
            "exchange_pubkey": null,
        });
        for (material, refusal) in [
            (sealed_under_current(&seed), AuthFailure::BadPassphrase),
            (owed, AuthFailure::CorruptAccount),
        ] {
            let transport = Scripted::new([
                Ok((200, material)),
                Ok((200, serde_json::json!({ "status": "rotated" }))),
            ]);
            let auth = session("access-1", "refresh-1");
            let chosen =
                ambre_crypto::strength::ChosenPassphrase::check(new_passphrase()).expect("strong");

            let outcome =
                reseal_changed_passphrase(&transport, &auth, &root(), &new_passphrase(), &chosen)
                    .await;
            assert!(
                matches!(&outcome, Err(KeyWriteFailure::Auth(cause)) if *cause == refusal),
                "{refusal:?}"
            );
            assert_eq!(
                transport.calls(),
                [(
                    http::Method::Get,
                    "https://relay.test/api/keys/master".to_owned()
                )],
                "{refusal:?}"
            );
        }
    }

    /// `SYNC-255`, `SYNC-256`: the first store of a signed-in account writes
    /// one body under the held pair. The body holds the sealed seed, its salt
    /// and the two public keys. The identity is then read under the pair the
    /// relay bound to the new key, and a later unlock opens the stored seed.
    #[tokio::test]
    async fn a_key_setup_stores_a_seed_that_a_later_unlock_opens() {
        let seed = MasterSeed::generate();
        let user = "55555555-5555-5555-5555-555555555555";
        let transport = Scripted::new([
            Ok((
                200,
                serde_json::json!({
                    "status": "stored",
                    "access_token": "access-2",
                    "refresh_token": "refresh-2",
                    "access_expires_in": 900,
                }),
            )),
            Ok((
                200,
                serde_json::json!({
                    "user_id": user,
                    "display_name": "Ada",
                    "email": "ada@example.org",
                }),
            )),
        ]);
        let auth = session("access-1", "refresh-1");
        let chosen =
            ambre_crypto::strength::ChosenPassphrase::check(new_passphrase()).expect("strong");
        let account = Id::<User>::parse_str(user).expect("an id");
        let keys = NewKeys {
            account,
            master_seed: seed,
            passphrase: &chosen,
            typed: &new_passphrase(),
        };

        // The seed comes back with the answer: it is the seed the relay holds.
        let Ok(StoredKeys {
            master_seed: seed,
            tokens,
            me,
        }) = store_new_keys(&transport, &auth, &root(), keys).await
        else {
            panic!("the setup did not land");
        };
        assert_eq!(
            transport.calls(),
            [
                (
                    http::Method::Put,
                    "https://relay.test/api/keys/master".to_owned()
                ),
                (
                    http::Method::Get,
                    "https://relay.test/api/auth/me".to_owned()
                ),
            ]
        );
        let sent = transport.sent.borrow();
        let bearer = |at: usize| {
            sent[at]
                .bearer
                .as_ref()
                .map(|token| token.secret().expose().clone())
        };
        assert_eq!(bearer(0).as_deref(), Some("access-1"), "the store");
        assert_eq!(bearer(1).as_deref(), Some("access-2"), "the identity");
        assert_eq!(tokens.access_token().secret().expose(), "access-2");
        assert_eq!(me.display_name.as_str(), "Ada");
        assert_eq!(me.user_id.to_string(), user);

        let body: serde_json::Value =
            serde_json::from_slice(sent[0].body.as_ref().expect("a body").1.as_bytes())
                .expect("json");
        let mut fields: Vec<&str> = body
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "exchange_pubkey",
                "master_key_enc",
                "master_key_salt",
                "signature",
                "signing_pubkey"
            ]
        );
        let stored: http::MasterKeyResponse =
            serde_json::from_value(body.clone()).expect("material");
        assert_eq!(stored.signing_pubkey, Some(signing_pubkey(&seed)));
        // `SRV-294`: the signing key of the seed signs the account and the
        // two keys, as the relay verifies it.
        let exchange_key: ambre_core::FixedBytes<ambre_core::ExchangePublicKey> =
            serde_json::from_value(body["exchange_pubkey"].clone()).expect("a key");
        let proof: ambre_core::FixedBytes<ambre_core::Signature> =
            serde_json::from_value(body["signature"].clone()).expect("a signature");
        let transcript = ambre_protocol::key_registration::registration_transcript(
            &account,
            &signing_pubkey(&seed),
            &exchange_key,
        );
        let key = ed25519_dalek::VerifyingKey::from_bytes(signing_pubkey(&seed).as_array())
            .expect("a key");
        let signature = ed25519_dalek::Signature::from_bytes(proof.as_array());
        assert!(key.verify_strict(transcript.as_bytes(), &signature).is_ok());
        let opened = open_stored_seed(&stored, &new_passphrase()).expect("the unlock opens");
        assert_eq!(signing_pubkey(&opened), signing_pubkey(&seed));
        let exchange: ambre_core::FixedBytes<ambre_core::ExchangePublicKey> =
            serde_json::from_value(body["exchange_pubkey"].clone()).expect("a key");
        assert_eq!(
            exchange,
            crate::sharing::own_exchange_key(&seed.derive_exchange_key())
        );
    }

    /// A first key store of a new seed, sealed under `chosen`.
    fn new_keys<'a>(
        chosen: &'a ambre_crypto::strength::ChosenPassphrase,
        typed: &'a ambre_core::Confidential<ambre_core::Passphrase>,
    ) -> NewKeys<'a> {
        NewKeys {
            account: Id::<User>::mint(),
            master_seed: MasterSeed::generate(),
            passphrase: chosen,
            typed,
        }
    }

    /// A store the relay refuses ends the setup with its error, and reads no
    /// identity.
    #[tokio::test]
    async fn a_refused_key_store_reads_no_identity() {
        let transport = Scripted::new([Ok((500, serde_json::json!({ "code": "INTERNAL" })))]);
        let auth = session("access-1", "refresh-1");
        let chosen =
            ambre_crypto::strength::ChosenPassphrase::check(new_passphrase()).expect("strong");
        let typed = new_passphrase();
        let outcome = store_new_keys(&transport, &auth, &root(), new_keys(&chosen, &typed)).await;
        assert!(matches!(outcome, Err(KeyWriteFailure::Http(_))));
        assert_eq!(transport.calls().len(), 1);
    }

    /// The master material of a seed `seed` sealed under the passphrase
    /// `typed`, as the relay answers it.
    fn sealed_material(seed: &MasterSeed, typed: &str) -> serde_json::Value {
        let chosen =
            ambre_crypto::strength::ChosenPassphrase::check(ambre_core::Confidential::<
                ambre_core::Passphrase,
            >::new(typed.to_owned()))
            .expect("strong");
        let sealed = seal_seed(seed, &chosen).expect("sealed");
        serde_json::json!({
            "master_key_enc": sealed.master_key_enc,
            "master_key_salt": sealed.master_key_salt,
            "signing_pubkey": signing_pubkey(seed),
            "exchange_pubkey": crate::sharing::own_exchange_key(&seed.derive_exchange_key()),
        })
    }

    /// `SYNC-285`: a second store, sent after a first one landed, answers
    /// with the keys the relay holds. The worker opens the stored seed under
    /// the typed passphrase, renews the pair so it carries the registered key,
    /// and reads the identity under it. It writes nothing more, and announces
    /// the stored seed, never the one it sealed.
    #[tokio::test]
    async fn a_second_key_store_answers_the_keys_the_relay_holds() {
        let first = MasterSeed::generate();
        let user = "66666666-6666-6666-6666-666666666666";
        let transport = Scripted::new([
            Ok((409, serde_json::json!({ "code": "CONFLICT" }))),
            Ok((200, sealed_material(&first, NEW_PASSPHRASE))),
            Ok((200, crate::http::scripted::renewed("access-2"))),
            Ok((
                200,
                serde_json::json!({
                    "user_id": user,
                    "display_name": "Ada",
                    "email": "ada@example.org",
                }),
            )),
        ]);
        let auth = session("access-1", "refresh-1");
        let chosen =
            ambre_crypto::strength::ChosenPassphrase::check(new_passphrase()).expect("strong");
        let typed = new_passphrase();

        let Ok(StoredKeys {
            master_seed,
            tokens,
            me,
        }) = store_new_keys(&transport, &auth, &root(), new_keys(&chosen, &typed)).await
        else {
            panic!("the second store did not answer the held keys");
        };
        assert_eq!(signing_pubkey(&master_seed), signing_pubkey(&first));
        assert_eq!(tokens.access_token().secret().expose(), "access-2");
        assert_eq!(me.user_id.to_string(), user);
        assert_eq!(
            transport.calls(),
            [
                (
                    http::Method::Put,
                    "https://relay.test/api/keys/master".to_owned()
                ),
                (
                    http::Method::Get,
                    "https://relay.test/api/keys/master".to_owned()
                ),
                (
                    http::Method::Post,
                    "https://relay.test/api/auth/refresh".to_owned()
                ),
                (
                    http::Method::Get,
                    "https://relay.test/api/auth/me".to_owned()
                ),
            ]
        );
    }

    /// `SYNC-285`: a second store under a passphrase that does not open the
    /// stored seed fails as a wrong passphrase. It renews nothing and writes
    /// nothing more. A conflict on an account that still owes its keys fails
    /// with the refusal.
    #[tokio::test]
    async fn a_second_key_store_under_another_passphrase_opens_nothing() {
        let transport = Scripted::new([
            Ok((409, serde_json::json!({ "code": "CONFLICT" }))),
            Ok((
                200,
                sealed_material(
                    &MasterSeed::generate(),
                    "violet-harbor-quietly-folds-ninety-kites",
                ),
            )),
        ]);
        let auth = session("access-1", "refresh-1");
        let chosen =
            ambre_crypto::strength::ChosenPassphrase::check(new_passphrase()).expect("strong");
        let typed = new_passphrase();
        let outcome = store_new_keys(&transport, &auth, &root(), new_keys(&chosen, &typed)).await;
        assert!(matches!(
            outcome,
            Err(KeyWriteFailure::Auth(AuthFailure::BadPassphrase))
        ));
        assert_eq!(transport.calls().len(), 2);

        let owed = Scripted::new([
            Ok((409, serde_json::json!({ "code": "CONFLICT" }))),
            Ok((
                200,
                serde_json::json!({
                    "master_key_enc": null,
                    "master_key_salt": null,
                    "signing_pubkey": null,
                    "exchange_pubkey": null,
                }),
            )),
        ]);
        let outcome = store_new_keys(&owed, &auth, &root(), new_keys(&chosen, &typed)).await;
        assert!(matches!(outcome, Err(KeyWriteFailure::Http(_))));
        assert_eq!(owed.calls().len(), 2);
    }
}

#[cfg(test)]
mod set_up_keys_tests {
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

    /// `ZK-036` on the setup of a signed-in account: a weak passphrase draws
    /// its weakness, and nothing else, before any key exists.
    #[test]
    fn a_weak_setup_passphrase_draws_its_weakness_alone() {
        let (mut state, rx) = worker();
        state.handle_set_up_keys(
            ambre_bus::next_request(),
            ambre_core::Confidential::<ambre_core::Passphrase>::new("password".to_owned()),
        );
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(
            matches!(
                told.as_slice(),
                [LogicEvent::AuthError {
                    cause: AuthFailure::WeakPassphrase(_)
                }]
            ),
            "told {told:?}"
        );
        assert!(state.master_seed.is_none());
    }

    /// `SYNC-255`: once the keys are stored, the worker holds them and the new
    /// pair, and opens the socket the setup owed. It tells Logic the words,
    /// then the session.
    #[test]
    fn stored_keys_answer_the_words_then_the_session() {
        let (mut state, rx) = worker();
        state.setup_owed = true;
        let seed = MasterSeed::generate();
        let id = ambre_bus::next_request();
        let user_id = Id::<User>::mint();
        state.handle_sync_event(SyncEvent::KeysSetUp {
            id,
            recovery_words: seed.recovery_words(),
            signing_key: seed.derive_signing_key(),
            exchange_key: seed.derive_exchange_key(),
            master_seed: seed,
            tokens: TokenPair::new(
                Credential::new("access-2".to_owned()),
                Credential::new("refresh-2".to_owned()),
                ambre_core::TimeSpan::<ambre_core::Sec>::new(900),
            ),
            user_id,
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").unwrap(),
            email: "ada@example.org".into(),
        });
        assert!(!state.setup_owed);
        assert_eq!(state.socket_opens.get(), 1);
        assert_eq!(state.user_id, Some(user_id));
        assert!(state.signing_key.is_some());
        let told: Vec<LogicEvent> = rx
            .try_iter()
            .filter(|e| matches!(e, LogicEvent::Rest { .. }))
            .collect();
        assert!(
            matches!(
                told.as_slice(),
                [
                    LogicEvent::Rest {
                        id: Some(answered),
                        evt: RestEvent::AccountCreated { .. },
                    },
                    LogicEvent::Rest {
                        id: None,
                        evt: RestEvent::SessionReady { user_id: ready, .. },
                    },
                ] if *answered == id && *ready == user_id
            ),
            "told {told:?}"
        );
    }
}
