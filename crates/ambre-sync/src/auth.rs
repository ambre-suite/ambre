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
            // through the SyncEvent channel into handle_sync_event.
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("{url_scheme}://{api_base}/api/keys/master");
                match http::get_json_auth::<http::MasterKeyResponse>(&url, auth.clone()).await {
                    Ok(resp) => {
                        // User-facing AuthError messages are deliberately generic:
                        // they never carry crypto detail (algorithm names, key/salt
                        // lengths, decrypt-failure specifics) that would leak the
                        // account's cryptographic shape to whatever renders the
                        // error. The concrete cause is logged locally instead.
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
                        if master_key_enc.len() < 24 {
                            tracing::warn!(
                                "Sync: master_key_enc too short ({} bytes)",
                                master_key_enc.len()
                            );
                            let _ = logic_tx.send(LogicEvent::AuthError {
                                message: CORRUPT.into(),
                            });
                            return;
                        }
                        let nonce: &[u8; 24] = master_key_enc[..24].try_into().expect("nonce len");
                        let ciphertext = &master_key_enc[24..];
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
                                            access_token: String::new(),
                                            refresh_token: String::new(),
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
            // Host-test / offline path: generate keys locally.
            let master_seed = MasterSeed::generate();
            self.signing_key = Some(master_seed.derive_signing_key());
            self.exchange_key = Some(master_seed.derive_exchange_key());
            self.master_seed = Some(master_seed);
            self.open_websocket();
            self.reply(
                id,
                RestEvent::SessionReady {
                    user_id: UserId::new(),
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
                let salt = {
                    let mut buf = [0u8; 32];
                    getrandom::fill(&mut buf).expect("CSPRNG");
                    buf
                };
                let user_key = ambre_crypto::passphrase::derive_user_key(&passphrase, &salt)
                    .expect("key derivation");
                let user_doc_key = DocumentKey::from_array(user_key);
                let (nonce, master_key_enc) = master_seed
                    .wrap_for_storage(&user_doc_key, b"ambre-master-seed")
                    .expect("wrap master seed");
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
        // wiped when they leave scope here: `SecretBytes` zeroizes its bytes on
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
    /// `LoggedOut` response — when there is a request to answer — is sent by the
    /// caller (`Shutdown` performs the teardown without replying).
    pub(crate) fn handle_logout(&mut self) {
        if let Some(ws) = self.ws.take() {
            let _ = ws.close();
        }
        self._onmessage = None;
        self._onclose = None;
        self._onerror = None;
        self.documents.clear();
        self.master_seed = None;
        self.signing_key = None;
        self.exchange_key = None;
        *self.tokens.borrow_mut() = None;
        tracing::info!("Sync: logged out, all keys zeroized");
    }
}
