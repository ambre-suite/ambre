use super::*;

impl SyncState {
    // -- Session handlers ------------------------------------------------------

    pub(crate) fn handle_unlock(&mut self, id: RequestId, passphrase: SecretBytes) {
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
                let url = format!("{url_scheme}://{api_base}/api/keys/master");
                match http::get_json_auth::<http::MasterKeyResponse>(&url, auth.clone()).await {
                    Ok(resp) if resp.setup_owed() => {
                        // No master material to unlock: the account still owes
                        // its key setup, which is no failure (`SYNC-134`).
                        let _ = logic_tx.send(LogicEvent::Rest {
                            id,
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

                        let salt = match base64::engine::general_purpose::URL_SAFE
                            .decode(&resp.master_key_salt)
                        {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::warn!("Sync: master_key_salt base64 decode failed: {e}");
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: CORRUPT.into(),
                                });
                                return;
                            }
                        };
                        let master_key_enc = match base64::engine::general_purpose::URL_SAFE
                            .decode(&resp.master_key_enc)
                        {
                            Ok(k) => k,
                            Err(e) => {
                                tracing::warn!("Sync: master_key_enc base64 decode failed: {e}");
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: CORRUPT.into(),
                                });
                                return;
                            }
                        };
                        let Some((nonce, ciphertext)) = master_key_enc.split_first_chunk::<24>()
                        else {
                            tracing::warn!(
                                "Sync: master_key_enc too short ({} bytes)",
                                master_key_enc.len()
                            );
                            let _ = logic_tx.send(LogicEvent::AuthError {
                                message: CORRUPT.into(),
                            });
                            return;
                        };
                        let user_key =
                            match ambre_crypto::passphrase::derive_user_key(&passphrase, &salt) {
                                Ok(k) => k,
                                Err(e) => {
                                    tracing::warn!("Sync: user-key derivation failed: {e}");
                                    let _ = logic_tx.send(LogicEvent::AuthError {
                                        message: BAD_PASS.into(),
                                    });
                                    return;
                                }
                            };
                        let user_doc_key = DocumentKey::from_array(user_key);
                        let seed_bytes = match ambre_crypto::aead::open(
                            nonce,
                            ciphertext,
                            b"ambre-master-seed",
                            &user_doc_key,
                        ) {
                            Ok(bytes) => bytes,
                            Err(e) => {
                                // Almost always the wrong passphrase (AEAD auth
                                // failure); could also be a tampered/corrupt row.
                                tracing::warn!("Sync: master-seed AEAD open failed: {e}");
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: BAD_PASS.into(),
                                });
                                return;
                            }
                        };
                        let seed_arr: [u8; 32] = match seed_bytes.try_into() {
                            Ok(arr) => arr,
                            Err(_) => {
                                tracing::warn!("Sync: decrypted master seed has unexpected length");
                                let _ = logic_tx.send(LogicEvent::AuthError {
                                    message: CORRUPT.into(),
                                });
                                return;
                            }
                        };
                        let master_seed = MasterSeed::from_array(seed_arr);
                        let signing_key = master_seed.derive_signing_key();
                        let exchange_key = master_seed.derive_exchange_key();

                        let auth_url = format!("{url_scheme}://{api_base}/api/auth/me");
                        match http::get_json_auth::<http::AuthMeResponse>(&auth_url, auth.clone())
                            .await
                        {
                            Ok(me) => {
                                let user_id = UserId::parse_str(&me.user_id)
                                    .unwrap_or_else(|_| UserId::new());
                                let _ = tx.send(SyncEvent::SessionReady {
                                    id,
                                    user_id,
                                    display_name: me.display_name,
                                    email: me.email,
                                    master_seed,
                                    signing_key,
                                    exchange_key,
                                    tokens: auth.tokens.borrow().as_ref().cloned().unwrap_or(
                                        http::TokenPair {
                                            access_token: http::SecretToken::default(),
                                            refresh_token: http::SecretToken::default(),
                                            access_expires_in: 0,
                                        },
                                    ),
                                });
                            }
                            Err(e) => {
                                let _ = logic_tx.send(LogicEvent::AuthError { message: e });
                            }
                        }
                    }
                    Err(e) => {
                        let _ = logic_tx.send(LogicEvent::AuthError { message: e });
                    }
                }
            });
        } else {
            // Host-test / offline path: generate keys locally. The user it
            // announces is the one it holds, so an opened document gets a
            // session identity (`AUTHOR-002`).
            let master_seed = MasterSeed::generate();
            let user_id = *self.user_id.get_or_insert_with(UserId::new);
            self.signing_key = Some(master_seed.derive_signing_key());
            self.exchange_key = Some(master_seed.derive_exchange_key());
            self.master_seed = Some(master_seed);
            self.open_websocket();
            self.reply(
                id,
                RestEvent::SessionReady {
                    user_id,
                    display_name: "offline".into(),
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
    pub(crate) fn handle_dev_login(&mut self, id: RequestId, username: String) {
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
        let master_seed = MasterSeed::from_array(ambre_crypto::hash::chain_hash(
            b"ambre-dev-seed",
            username.as_bytes(),
        ));
        let signing_key = master_seed.derive_signing_key();
        let exchange_key = master_seed.derive_exchange_key();

        if let Some(tx) = event_tx {
            let signing_pub_key = hex::encode(signing_key.verifying_key().to_bytes());
            let exchange_pub_key = {
                let pub_key = x25519_dalek::PublicKey::from(&exchange_key);
                hex::encode(pub_key.as_bytes())
            };
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("{url_scheme}://{api_base}/api/auth/dev/login");
                let body = http::DevLoginRequest {
                    username,
                    signing_pub_key,
                    exchange_pub_key,
                };
                match http::post_json::<_, http::DevLoginResponse>(&url, &body).await {
                    Ok(resp) => {
                        let user_id =
                            UserId::parse_str(&resp.user_id).unwrap_or_else(|_| UserId::new());
                        let _ = tx.send(SyncEvent::SessionReady {
                            id,
                            user_id,
                            display_name: resp.display_name,
                            email: String::new(),
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
                        let _ = logic_tx.send(LogicEvent::AuthError { message: e });
                    }
                }
            });
        } else {
            // Host-test / offline path: adopt the deterministic keys locally.
            self.signing_key = Some(signing_key);
            self.exchange_key = Some(exchange_key);
            self.master_seed = Some(master_seed);
            self.open_websocket();
            self.reply(
                id,
                RestEvent::SessionReady {
                    user_id: UserId::new(),
                    display_name: username,
                    email: String::new(),
                },
            );
        }
    }

    /// Ask the relay for `provider`'s authorization address and hand it to
    /// the UI, which sends the browser there (`CHROME-095`). Unauthenticated:
    /// no session exists yet.
    pub(crate) fn handle_provider_sign_in(&mut self, id: RequestId, provider: String) {
        tracing::info!("Sync: ProviderSignIn ({provider})");
        if !is_provider_name(&provider) {
            self.send_to_logic(LogicEvent::AuthError {
                message: PROVIDER_REFUSED.into(),
            });
            return;
        }
        if self.event_tx.is_none() {
            self.send_to_logic(LogicEvent::AuthError {
                message: "Sign-in failed: no relay to ask".into(),
            });
            return;
        }
        let logic_tx = self.logic_tx.clone();
        let url = format!(
            "{}://{}/api/auth/{provider}/login",
            self.url_scheme, self.api_base
        );
        wasm_bindgen_futures::spawn_local(async move {
            let evt = match http::get_json::<http::ProviderLoginResponse>(&url).await {
                Ok(resp) => LogicEvent::Rest {
                    id,
                    evt: RestEvent::ProviderAddress {
                        url: resp.url,
                        state: resp.state,
                    },
                },
                Err(message) => LogicEvent::AuthError { message },
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
        id: RequestId,
        provider: String,
        code: String,
        state: String,
    ) {
        tracing::info!("Sync: ProviderReturn ({provider})");
        if !is_provider_name(&provider) {
            self.send_to_logic(LogicEvent::AuthError {
                message: PROVIDER_REFUSED.into(),
            });
            return;
        }
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
            let url = format!("{url_scheme}://{api_base}/api/auth/{provider}/callback");
            let body = http::ProviderCallbackRequest { code, state };
            let resp = match http::post_json::<_, http::ProviderCallbackResponse>(&url, &body).await
            {
                Ok(resp) => resp,
                Err(message) => {
                    let _ = logic_tx.send(LogicEvent::AuthError { message });
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
            let keys_url = format!("{url_scheme}://{api_base}/api/keys/master");
            let keys = match http::get_json_auth::<http::MasterKeyResponse>(&keys_url, auth.clone())
                .await
            {
                Ok(keys) => keys,
                Err(message) => {
                    let _ = logic_tx.send(LogicEvent::AuthError { message });
                    return;
                }
            };
            let Some(tokens) = auth.tokens.borrow_mut().take() else {
                return;
            };
            let _ = tx.send(SyncEvent::ProviderSignedIn {
                id,
                user_id: UserId::parse_str(&resp.user_id).unwrap_or_else(|_| UserId::new()),
                display_name: resp.display_name,
                email: resp.email,
                keys_stored: !keys.setup_owed(),
                tokens,
            });
        });
    }

    pub(crate) fn handle_setup_account(
        &mut self,
        id: RequestId,
        passphrase: SecretBytes,
        email: String,
        display_name: String,
    ) {
        tracing::info!("Sync: SetupAccount");

        let logic_tx = self.logic_tx.clone();
        let api_base = self.api_base.clone();
        let url_scheme = self.url_scheme;
        let event_tx = self.event_tx.clone();

        if let Some(tx) = event_tx {
            wasm_bindgen_futures::spawn_local(async move {
                let master_seed = MasterSeed::generate();
                let recovery_words = encode(master_seed.recovery_bytes());
                let signing_key = master_seed.derive_signing_key();
                let exchange_key = master_seed.derive_exchange_key();
                let signing_pub_key = hex::encode(signing_key.verifying_key().to_bytes());
                let exchange_pub_key = {
                    let pub_key = x25519_dalek::PublicKey::from(&exchange_key);
                    hex::encode(pub_key.as_bytes())
                };
                // Each step below fails only on a broken platform or a bug. The
                // account is not created then, and the user reads why.
                let mut salt = [0u8; 32];
                let wrapped = getrandom::fill(&mut salt)
                    .map_err(|e| format!("no CSPRNG for the account salt: {e}"))
                    .and_then(|()| {
                        ambre_crypto::passphrase::derive_user_key(&passphrase, &salt)
                            .map_err(|e| format!("user-key derivation failed: {e}"))
                    })
                    .and_then(|user_key| {
                        master_seed
                            .wrap_for_storage(
                                &DocumentKey::from_array(user_key),
                                b"ambre-master-seed",
                            )
                            .map_err(|e| format!("master-seed wrap failed: {e}"))
                    });
                let (nonce, master_key_enc) = match wrapped {
                    Ok(pair) => pair,
                    Err(message) => {
                        tracing::warn!("Sync: {message}");
                        let _ = logic_tx.send(LogicEvent::AuthError { message });
                        return;
                    }
                };
                let mut master_key_with_nonce = nonce.to_vec();
                master_key_with_nonce.extend_from_slice(&master_key_enc);

                // Zero-Knowledge (N4): only the wrapped master key and its salt go
                // over the wire; the passphrase stays in this worker and zeroizes
                // on drop.
                let url = format!("{url_scheme}://{api_base}/api/auth/setup");
                let body = http::SetupAccountRequest {
                    email,
                    display_name,
                    master_key_enc: base64::engine::general_purpose::URL_SAFE
                        .encode(&master_key_with_nonce),
                    master_key_salt: base64::engine::general_purpose::URL_SAFE.encode(salt),
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
                        let _ = logic_tx.send(LogicEvent::AuthError { message: e });
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
            let recovery_words = encode(master_seed.recovery_bytes());
            self.master_seed = Some(master_seed);
            self.open_websocket();
            self.reply(id, RestEvent::AccountCreated { recovery_words });
        }
    }

    pub(crate) fn handle_change_passphrase(
        &mut self,
        current_passphrase: SecretBytes,
        new_passphrase: SecretBytes,
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
        new_passphrase: SecretBytes,
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
}

/// The user-facing refusal of a provider name that is not one.
const PROVIDER_REFUSED: &str = "Sign-in failed: unknown provider";

/// Whether `provider` may name a provider in a relay path: lowercase ASCII
/// letters and digits only. Anything else could reach another route, so it
/// is refused before a request is built.
pub(crate) fn is_provider_name(provider: &str) -> bool {
    !provider.is_empty()
        && provider.len() <= 32
        && provider
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::http::{SecretToken, TokenPair};

    fn worker() -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        (SyncState::new(sink, String::new(), None), rx)
    }

    fn signed_in(keys_stored: bool) -> SyncEvent {
        SyncEvent::ProviderSignedIn {
            id: RequestId::next(),
            user_id: UserId::new(),
            display_name: "Ada".into(),
            email: "ada@example.org".into(),
            keys_stored,
            tokens: TokenPair {
                access_token: SecretToken::from("access-s3cr3t".to_owned()),
                refresh_token: SecretToken::from("refresh-s3cr3t".to_owned()),
                access_expires_in: 60,
            },
        }
    }

    /// A provider sign-in leaves its tokens on the worker (`CHROME-096`). What
    /// Logic hears of it names the person, and carries no byte of a token.
    /// No key is held yet, so no socket opens.
    #[test]
    fn a_provider_sign_in_keeps_its_tokens_on_the_worker() {
        let (mut state, rx) = worker();
        state.handle_sync_event(signed_in(true));

        let stored = state.tokens.borrow();
        let stored = stored.as_ref().expect("the worker holds the session");
        assert_eq!(stored.access_token.expose(), "access-s3cr3t");
        assert_eq!(stored.refresh_token.expose(), "refresh-s3cr3t");
        assert_eq!(state.socket_opens, 0);
        assert!(!state.setup_owed);

        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(told.iter().any(|e| matches!(
            e,
            LogicEvent::Rest {
                evt: RestEvent::ProviderSignedIn { keys_stored: true, display_name, .. },
                ..
            } if display_name == "Ada"
        )));
        let heard = format!("{told:?}");
        assert!(!heard.contains("s3cr3t"), "a token reached Logic: {heard}");
    }

    /// An account that owes its keys is marked so, which keeps the socket
    /// closed until the setup stores them.
    #[test]
    fn a_provider_sign_in_without_keys_marks_the_setup_owed() {
        let (mut state, _rx) = worker();
        state.handle_sync_event(signed_in(false));
        assert!(state.setup_owed);
        state.handle_sync_event(SyncEvent::ReconnectNow);
        assert_eq!(state.socket_opens, 0);
    }

    /// A provider name that could reach another relay route is refused before
    /// any request is built (`CHROME-095`).
    #[test]
    fn a_provider_name_outside_its_alphabet_is_refused() {
        assert!(is_provider_name("google"));
        assert!(is_provider_name("entra2"));
        for bad in ["", "../keys/master", "Google", "google?x=1", "a/b", "é"] {
            assert!(!is_provider_name(bad), "{bad:?} was accepted");
        }
        let (mut state, rx) = worker();
        state.handle_rest(
            RequestId::next(),
            RestCommand::ProviderSignIn {
                provider: "../keys/master".into(),
            },
        );
        state.handle_rest(
            RequestId::next(),
            RestCommand::ProviderReturn {
                provider: "../keys/master".into(),
                code: "c".into(),
                state: "s".into(),
            },
        );
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert_eq!(told.len(), 2);
        assert!(told.iter().all(|e| matches!(
            e,
            LogicEvent::AuthError { message } if message == PROVIDER_REFUSED
        )));
    }
}
