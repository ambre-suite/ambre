//! The session a device keeps across a page reload (`ambre-spec-session.md`
//! section 3, `SRV-322` to `SRV-327`; `ambre-spec-zero-knowledge.md`,
//! `ZK-051`, `ZK-052`).
//!
//! A provider session keeps one record on the device. The record holds the
//! refresh token, the seed in its 24-word form and the profile of the person.
//! The Sync worker seals it under an AES-GCM key of WebCrypto, which it
//! generates as non-extractable, and stores both in IndexedDB. No page code
//! can read the key out. The access token, the passphrase and every document
//! key stay in worker memory alone.
//!
//! * A sign-in keeps the record under a fresh key, so the record of another
//!   account on the device no longer opens (`SRV-324`).
//! * A renewal keeps the next refresh token under the key held (`SRV-326`).
//! * A sign-out erases the record and its key (`SRV-323`).
//! * At boot the page asks for a resume. The worker renews the kept refresh
//!   token. A relay that refuses it erases the record (`SRV-325`).
//! * A development session keeps nothing (`SRV-327`).

use std::cell::RefCell;
use std::rc::Rc;

use ambre_core::{Confidential, Plaintext, RecoveryWords};

use super::*;

/// The form of a kept record. A record of another form opens nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum KeptForm {
    /// The first form: the fields of [`KeptRecord`].
    First,
}

/// What the device keeps of a session, before its seal.
#[derive(serde::Serialize, serde::Deserialize)]
struct KeptRecord {
    form: KeptForm,
    user_id: Id<User>,
    display_name: ambre_core::Validated<ambre_core::DisplayName>,
    email: ambre_core::Email,
    refresh_token: http::Credential<http::RefreshToken>,
    /// The seed, in the one form a seed leaves its type in.
    words: Confidential<RecoveryWords>,
}

/// Who a kept session belongs to, as the page shows the person.
#[derive(Debug, Clone)]
pub(crate) struct KeptProfile {
    pub(crate) display_name: ambre_core::Validated<ambre_core::DisplayName>,
    pub(crate) email: ambre_core::Email,
}

/// A kept session, opened.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "only the worker resumes a kept session")
)]
pub(crate) struct Kept {
    pub(crate) user_id: Id<User>,
    pub(crate) profile: KeptProfile,
    pub(crate) refresh_token: http::Credential<http::RefreshToken>,
    pub(crate) master_seed: MasterSeed,
}

/// The plaintext of the record of a session: `user_id` under `profile`, with
/// its `refresh_token` and its `seed`. The vault seals it. `None` when the
/// record does not encode, so nothing is kept.
fn encode(
    user_id: Id<User>,
    profile: &KeptProfile,
    refresh_token: &http::Credential<http::RefreshToken>,
    seed: &MasterSeed,
) -> Option<Confidential<Plaintext>> {
    let record = KeptRecord {
        form: KeptForm::First,
        user_id,
        display_name: profile.display_name.clone(),
        email: profile.email.clone(),
        refresh_token: refresh_token.clone(),
        words: seed.recovery_words(),
    };
    postcard::to_allocvec(&record)
        .ok()
        .map(Confidential::<Plaintext>::new)
}

/// The session the opened record `plaintext` holds, or `None` for a record
/// of another form, or one whose words spell no seed.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "only the worker resumes a kept session")
)]
fn decode(plaintext: &Confidential<Plaintext>) -> Option<Kept> {
    let record: KeptRecord = plaintext
        .with_plaintext(|bytes| postcard::from_bytes(bytes))
        .ok()?;
    if record.form != KeptForm::First {
        return None;
    }
    let master_seed = MasterSeed::from_recovery_words(&record.words).ok()?;
    Some(Kept {
        user_id: record.user_id,
        profile: KeptProfile {
            display_name: record.display_name,
            email: record.email,
        },
        refresh_token: record.refresh_token,
        master_seed,
    })
}

/// Which key a keep seals under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceKey {
    /// A new key: a sign-in. No record sealed before it opens under it.
    Fresh,
    /// The key the vault holds since the last keep or resume, or a new one
    /// when it holds none.
    Held,
}

/// What a session ready to run keeps of itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keeping {
    /// Nothing: a development session (`SRV-327`).
    #[cfg_attr(
        all(not(feature = "dev-login"), not(test)),
        expect(dead_code, reason = "only the development sign-in keeps nothing")
    )]
    Nothing,
    /// The session, sealed under `DeviceKey`.
    Under(DeviceKey),
}

/// What the vault read of the record.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "only the worker reads a kept record")
)]
pub(crate) enum Loaded {
    /// The device keeps no record.
    Absent,
    /// A record is kept, and its seal did not open.
    Unopened,
    /// The record, opened.
    Opened(Confidential<Plaintext>),
}

/// What the boot of a page found of a kept session.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "only the worker resumes a kept session")
)]
pub(crate) enum Resumed {
    /// The device keeps no record.
    Absent,
    /// The record does not open, or opens as no session. The worker erases
    /// it, as it erases a refused one.
    Unreadable,
    /// The relay refused the kept refresh token. The record is erased.
    Refused,
    /// The relay did not answer, or failed. The record stays for a later
    /// boot.
    Unreachable,
    /// The relay renewed the kept refresh token into `tokens`.
    Ready { kept: Kept, tokens: http::TokenPair },
}

/// Resumes the session `record` keeps, through `transport` to the relay at
/// `api` (`SRV-322`, `SRV-325`). The kept refresh token is the one
/// credential sent. A refusal of the relay is final: that token opens
/// nothing any more.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "only the worker resumes a kept session")
)]
pub(crate) async fn resume_via(
    transport: &impl http::Transport,
    api: &http::Url<http::ApiRoot>,
    loaded: Loaded,
) -> Resumed {
    let record = match loaded {
        Loaded::Absent => return Resumed::Absent,
        Loaded::Unopened => return Resumed::Unreadable,
        Loaded::Opened(record) => record,
    };
    let Some(kept) = decode(&record) else {
        return Resumed::Unreadable;
    };
    match http::redeem_via(transport, api, &kept.refresh_token).await {
        Ok(tokens) => Resumed::Ready { kept, tokens },
        Err(e) => match e.failure() {
            ApiFailure::Relay { status, .. } if status.is_client_error() => Resumed::Refused,
            _ => Resumed::Unreachable,
        },
    }
}

/// The vault of a host build: the record in memory. It keeps nothing past
/// the process, and a host test reads what a keep chose.
#[derive(Default)]
pub(crate) struct MemoryVault {
    record: Option<Confidential<Plaintext>>,
    /// The key each keep sealed under, oldest first.
    keys: Vec<DeviceKey>,
}

impl MemoryVault {
    /// The session the vault keeps, opened.
    #[cfg(test)]
    pub(crate) fn kept(&self) -> Option<Kept> {
        self.record.as_ref().and_then(decode)
    }

    /// The key each keep sealed under, oldest first.
    #[cfg(test)]
    pub(crate) fn keys(&self) -> &[DeviceKey] {
        &self.keys
    }
}

/// Where the record lives.
pub(crate) enum Vault {
    /// In memory, on a host build.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the worker keeps its session in IndexedDB")
    )]
    Memory(Rc<RefCell<MemoryVault>>),
    /// In IndexedDB, sealed by WebCrypto, in the worker.
    #[cfg(target_arch = "wasm32")]
    Web(web::WebVault),
}

impl Default for Vault {
    fn default() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            Self::Web(web::WebVault::default())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self::Memory(Rc::default())
        }
    }
}

impl Vault {
    /// Keeps `plaintext` under `key`, in place of any record.
    fn keep(&self, plaintext: Confidential<Plaintext>, key: DeviceKey) {
        match self {
            Self::Memory(memory) => {
                let mut memory = memory.borrow_mut();
                memory.record = Some(plaintext);
                memory.keys.push(key);
            }
            #[cfg(target_arch = "wasm32")]
            Self::Web(web) => web.enqueue(web::Op::Keep(plaintext, key)),
        }
    }

    /// Erases the record and the key it was sealed under.
    fn erase(&self) {
        match self {
            Self::Memory(memory) => memory.borrow_mut().record = None,
            #[cfg(target_arch = "wasm32")]
            Self::Web(web) => web.enqueue(web::Op::Erase),
        }
    }
}

/// The kept session of a worker: where it lives, and who it is for.
#[derive(Default)]
pub(crate) struct KeptSession {
    pub(crate) vault: Vault,
    /// The profile of the session kept now. `None` while nothing is kept,
    /// so a renewal keeps nothing either.
    profile: Option<KeptProfile>,
}

impl SyncState {
    /// A session is ready to run: keeps it as `keeping` says.
    pub(crate) fn adopt_kept(&mut self, keeping: Keeping, profile: KeptProfile) {
        let Keeping::Under(key) = keeping else {
            return;
        };
        self.kept.profile = Some(profile);
        self.keep_session(key);
    }

    /// The pair was renewed: keeps its refresh token, under the key held
    /// (`SRV-326`). The relay honours a refresh token once, so a record
    /// left on the previous one would open nothing at the next boot.
    pub(crate) fn rekeep_after_renewal(&mut self) {
        self.keep_session(DeviceKey::Held);
    }

    /// Seals and stores what the session holds now, under `key`. A session
    /// with no profile, no seed or no pair keeps nothing.
    fn keep_session(&self, key: DeviceKey) {
        let (Some(profile), Some(user_id), Some(seed)) =
            (&self.kept.profile, self.user_id, self.master_seed.as_ref())
        else {
            return;
        };
        let Some(refresh_token) = self
            .tokens
            .borrow()
            .as_ref()
            .map(|t| t.refresh_token().clone())
        else {
            return;
        };
        match encode(user_id, profile, &refresh_token, seed) {
            Some(plaintext) => self.kept.vault.keep(plaintext, key),
            None => crate::log::warning!("Sync: the session did not encode, nothing kept"),
        }
    }

    /// Erases the kept session (`SRV-323`). The record and its key go
    /// together, so nothing left on the device opens.
    pub(crate) fn erase_kept(&mut self) {
        self.kept.profile = None;
        self.kept.vault.erase();
    }

    /// Forgets who the kept session is for, and keeps the record. A teardown
    /// that is no sign-out ends the session in memory alone.
    pub(crate) fn forget_kept_profile(&mut self) {
        self.kept.profile = None;
    }

    /// The page booted with no other sign-in: resumes the session the device
    /// keeps (`SRV-322`). A ready session answers `SessionReady`. Anything
    /// else answers `NoKeptSession`, and the page stays on the sign-in
    /// screen (`SRV-325`).
    pub(crate) fn handle_resume(&mut self, id: Seq<Request>) {
        crate::log::info!("Sync: ResumeSession");
        #[cfg(target_arch = "wasm32")]
        if let (Some(tx), Vault::Web(web)) = (self.event_tx.clone(), &self.kept.vault) {
            let web = web.clone();
            let logic_tx = self.logic_tx.clone();
            let api = http::Url::<http::ApiRoot>::of(self.url_scheme, &self.api_base);
            crate::spawn(async move {
                let record = web.load().await;
                let resumed = resume_via(&http::Fetch, &api, record).await;
                if matches!(resumed, Resumed::Unreadable | Resumed::Refused) {
                    web.enqueue(web::Op::Erase);
                }
                match resumed {
                    Resumed::Ready { kept, tokens } => {
                        let _ = tx.send(SyncEvent::SessionReady {
                            id: Some(id),
                            user_id: kept.user_id,
                            display_name: kept.profile.display_name,
                            email: kept.profile.email,
                            signing_key: kept.master_seed.derive_signing_key(),
                            exchange_key: kept.master_seed.derive_exchange_key(),
                            master_seed: kept.master_seed,
                            tokens,
                            kept: Keeping::Under(DeviceKey::Held),
                        });
                    }
                    Resumed::Absent
                    | Resumed::Unreadable
                    | Resumed::Refused
                    | Resumed::Unreachable => {
                        let _ = logic_tx.send(LogicEvent::Rest {
                            id: Some(id),
                            evt: RestEvent::NoKeptSession,
                        });
                    }
                }
            });
            return;
        }
        // No relay, or no device store: nothing resumes.
        self.reply(id, RestEvent::NoKeptSession);
    }
}

/// The vault of the worker: IndexedDB and WebCrypto.
#[cfg(target_arch = "wasm32")]
pub(crate) mod web {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::rc::Rc;

    use ambre_core::{Confidential, Plaintext, StaticKey, StorageKeys};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    use super::{DeviceKey, Loaded};

    /// The database, its one store and the one key of the record.
    const DATABASE: StaticKey<StorageKeys> = StaticKey::new("ambre-session");
    const STORE: StaticKey<StorageKeys> = StaticKey::new("session");
    const RECORD: StaticKey<StorageKeys> = StaticKey::new("kept");

    /// One write of the vault, applied in the order the worker asked.
    pub(crate) enum Op {
        Keep(Confidential<Plaintext>, DeviceKey),
        Erase,
    }

    /// Whether a task applies the writes now.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Drain {
        Idle,
        Busy,
    }

    /// The vault: the key it holds in memory, and the writes still to apply.
    #[derive(Clone)]
    pub(crate) struct WebVault {
        key: Rc<RefCell<Option<web_sys::CryptoKey>>>,
        queue: Rc<RefCell<VecDeque<Op>>>,
        drain: Rc<Cell<Drain>>,
    }

    impl Default for WebVault {
        fn default() -> Self {
            Self {
                key: Rc::default(),
                queue: Rc::default(),
                drain: Rc::new(Cell::new(Drain::Idle)),
            }
        }
    }

    impl WebVault {
        /// Queues `op`. One task applies the writes one after the other, so
        /// an earlier refresh token never lands over a later one.
        pub(crate) fn enqueue(&self, op: Op) {
            self.queue.borrow_mut().push_back(op);
            if self.drain.get() == Drain::Busy {
                return;
            }
            self.drain.set(Drain::Busy);
            let vault = self.clone();
            crate::spawn(async move {
                loop {
                    let next = vault.queue.borrow_mut().pop_front();
                    let Some(op) = next else { break };
                    if let Err(e) = vault.apply(op).await {
                        crate::log::warning!("Sync: the kept session was not written: {e:?}");
                    }
                }
                vault.drain.set(Drain::Idle);
            });
        }

        async fn apply(&self, op: Op) -> Result<(), JsValue> {
            match op {
                Op::Keep(plaintext, choice) => {
                    let held = self.key.borrow().clone();
                    let key = match (choice, held) {
                        (DeviceKey::Held, Some(key)) => key,
                        (DeviceKey::Fresh, _) | (DeviceKey::Held, None) => fresh_key().await?,
                    };
                    *self.key.borrow_mut() = Some(key.clone());
                    let (iv, sealed) = seal(&key, &plaintext).await?;
                    let record = js_sys::Object::new();
                    js_sys::Reflect::set(&record, &"key".into(), &key)?;
                    js_sys::Reflect::set(&record, &"iv".into(), &iv)?;
                    js_sys::Reflect::set(&record, &"sealed".into(), &sealed)?;
                    write(|store| store.put_with_key(&record, &RECORD.as_str().into())).await
                }
                Op::Erase => {
                    *self.key.borrow_mut() = None;
                    write(|store| store.delete(&RECORD.as_str().into())).await
                }
            }
        }

        /// The record the device keeps, opened, and its key held for the
        /// next keep.
        pub(crate) async fn load(&self) -> Loaded {
            let found = async {
                let db = open().await?;
                let store = db
                    .transaction_with_str(STORE.as_str())?
                    .object_store(STORE.as_str())?;
                settle(&store.get(&RECORD.as_str().into())?).await
            };
            let record = match found.await {
                Ok(record) if record.is_undefined() || record.is_null() => return Loaded::Absent,
                Ok(record) => record,
                Err(e) => {
                    crate::log::warning!("Sync: the kept session did not read: {e:?}");
                    return Loaded::Absent;
                }
            };
            match self.open_record(&record).await {
                Ok(plaintext) => Loaded::Opened(plaintext),
                Err(e) => {
                    crate::log::warning!("Sync: the kept session did not open: {e:?}");
                    Loaded::Unopened
                }
            }
        }

        async fn open_record(&self, record: &JsValue) -> Result<Confidential<Plaintext>, JsValue> {
            let key: web_sys::CryptoKey =
                js_sys::Reflect::get(record, &"key".into())?.dyn_into()?;
            let iv: js_sys::Uint8Array = js_sys::Reflect::get(record, &"iv".into())?.dyn_into()?;
            let sealed: js_sys::ArrayBuffer =
                js_sys::Reflect::get(record, &"sealed".into())?.dyn_into()?;
            let plaintext = open_sealed(&key, &iv, &sealed).await?;
            *self.key.borrow_mut() = Some(key);
            Ok(plaintext)
        }
    }

    /// The global scope of the worker.
    fn scope() -> Result<web_sys::WorkerGlobalScope, JsValue> {
        js_sys::global().dyn_into().map_err(JsValue::from)
    }

    fn subtle() -> Result<web_sys::SubtleCrypto, JsValue> {
        Ok(scope()?.crypto()?.subtle())
    }

    /// A promise that settles with the result of `request`.
    fn settle(request: &web_sys::IdbRequest) -> JsFuture {
        let request = request.clone();
        JsFuture::from(js_sys::Promise::new(&mut |resolve, reject| {
            let done = request.clone();
            let ok = wasm_bindgen::closure::Closure::once_into_js(move || {
                let _ = resolve.call1(&JsValue::NULL, &done.result().unwrap_or_default());
            });
            let failed = wasm_bindgen::closure::Closure::once_into_js(move || {
                let _ = reject.call0(&JsValue::NULL);
            });
            request.set_onsuccess(Some(ok.unchecked_ref()));
            request.set_onerror(Some(failed.unchecked_ref()));
        }))
    }

    /// Opens the database, creating its store on the first open.
    async fn open() -> Result<web_sys::IdbDatabase, JsValue> {
        let factory = scope()?
            .indexed_db()?
            .ok_or_else(|| JsValue::from_str("no IndexedDB"))?;
        let request = factory.open_with_u32(DATABASE.as_str(), 1)?;
        let upgrading = request.clone();
        let upgrade = wasm_bindgen::closure::Closure::once_into_js(move || {
            if let Ok(db) = upgrading
                .result()
                .and_then(|db| db.dyn_into::<web_sys::IdbDatabase>())
                && !db.object_store_names().contains(STORE.as_str())
            {
                let _ = db.create_object_store(STORE.as_str());
            }
        });
        request.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
        settle(&request).await?.dyn_into()
    }

    /// Runs `change` on the store in a transaction that writes, and waits
    /// until the transaction commits: a reload right after it finds the
    /// change.
    async fn write(
        change: impl FnOnce(&web_sys::IdbObjectStore) -> Result<web_sys::IdbRequest, JsValue>,
    ) -> Result<(), JsValue> {
        let db = open().await?;
        let tx = db.transaction_with_str_and_mode(
            STORE.as_str(),
            web_sys::IdbTransactionMode::Readwrite,
        )?;
        let committed = JsFuture::from(js_sys::Promise::new(&mut |resolve, reject| {
            let ok = wasm_bindgen::closure::Closure::once_into_js(move || {
                let _ = resolve.call0(&JsValue::NULL);
            });
            let failed = reject.clone();
            let error = wasm_bindgen::closure::Closure::once_into_js(move || {
                let _ = failed.call0(&JsValue::NULL);
            });
            let aborted = wasm_bindgen::closure::Closure::once_into_js(move || {
                let _ = reject.call0(&JsValue::NULL);
            });
            tx.set_oncomplete(Some(ok.unchecked_ref()));
            tx.set_onerror(Some(error.unchecked_ref()));
            tx.set_onabort(Some(aborted.unchecked_ref()));
        }));
        change(&tx.object_store(STORE.as_str())?)?;
        committed.await.map(|_| ())
    }

    /// A new AES-GCM key of 256 bits. It cannot be exported: no code of the
    /// page reads its bytes, and IndexedDB stores the key object alone.
    async fn fresh_key() -> Result<web_sys::CryptoKey, JsValue> {
        let params = web_sys::AesKeyGenParams::new("AES-GCM", 256);
        let usages = js_sys::Array::of2(&"encrypt".into(), &"decrypt".into());
        let key =
            JsFuture::from(subtle()?.generate_key_with_object(&params, false, &usages)?).await?;
        key.dyn_into()
    }

    /// Seals `plaintext` under `key`, with a fresh nonce of 96 bits. The
    /// copy handed to WebCrypto is wiped once WebCrypto took its own.
    async fn seal(
        key: &web_sys::CryptoKey,
        plaintext: &Confidential<Plaintext>,
    ) -> Result<(js_sys::Uint8Array, JsValue), JsValue> {
        let iv = js_sys::Uint8Array::new_with_length(12);
        scope()?
            .crypto()?
            .get_random_values_with_array_buffer_view(&iv)?;
        let params = web_sys::AesGcmParams::new("AES-GCM", &iv);
        // A copy on the JS heap, no view of the memory of this instance,
        // which WebCrypto may refuse. WebCrypto takes its own copy at the
        // call, so this one is wiped at once.
        let sealing = plaintext.with_plaintext(|bytes| {
            let data = js_sys::Uint8Array::new_with_length(bytes.len().try_into().unwrap_or(0));
            data.copy_from(bytes);
            let promise = subtle().and_then(|subtle| {
                subtle.encrypt_with_object_and_buffer_source(&params, key, &data)
            });
            data.fill(0, 0, data.length());
            promise
        })?;
        let sealed = JsFuture::from(sealing).await?;
        Ok((iv, sealed))
    }

    /// Opens `sealed` under `key` and `iv`. The JS copy of the plaintext is
    /// wiped once the worker holds its own.
    async fn open_sealed(
        key: &web_sys::CryptoKey,
        iv: &js_sys::Uint8Array,
        sealed: &js_sys::ArrayBuffer,
    ) -> Result<Confidential<Plaintext>, JsValue> {
        let params = web_sys::AesGcmParams::new("AES-GCM", iv);
        let opened =
            JsFuture::from(subtle()?.decrypt_with_object_and_buffer_source(&params, key, sealed)?)
                .await?;
        let view = js_sys::Uint8Array::new(&opened);
        let plaintext = Confidential::<Plaintext>::new(view.to_vec());
        view.fill(0, 0, view.length());
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::scripted::{Scripted, renewed};
    use crate::http::{Credential, TokenPair};

    fn profile(name: &str) -> KeptProfile {
        KeptProfile {
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse(name).unwrap(),
            email: format!("{name}@example.org").into(),
        }
    }

    fn pair(refresh: &str) -> TokenPair {
        TokenPair::new(
            Credential::new("access-s3cr3t".to_owned()),
            Credential::new(refresh.to_owned()),
            ambre_core::TimeSpan::<ambre_core::Sec>::new(900),
        )
    }

    fn api() -> http::Url<http::ApiRoot> {
        http::Url::<http::ApiRoot>::of(
            http::UrlScheme::Https,
            &http::Url::<http::RelayHost>::new("relay.test".to_owned()),
        )
    }

    /// The record of a session under `seed`, for the person `name`.
    fn record(name: &str, refresh: &str, seed: [u8; 32]) -> Confidential<Plaintext> {
        encode(
            Id::<User>::mint(),
            &profile(name),
            &Credential::new(refresh.to_owned()),
            &MasterSeed::from_array(seed),
        )
        .unwrap()
    }

    fn signing_key_of(seed: &MasterSeed) -> [u8; 32] {
        seed.derive_signing_key().verifying_key().to_bytes()
    }

    /// `SRV-322`: a kept record opens as the same person, the same refresh
    /// token and the same seed. A record of no known form opens nothing.
    #[test]
    fn a_kept_record_opens_as_the_same_person_token_and_seed() {
        let user = Id::<User>::mint();
        let plaintext = encode(
            user,
            &profile("Ada"),
            &Credential::new("refresh-1".to_owned()),
            &MasterSeed::from_array([7; 32]),
        )
        .unwrap();
        let kept = decode(&plaintext).unwrap();
        assert_eq!(kept.user_id, user);
        assert_eq!(kept.profile.display_name.as_str(), "Ada");
        assert!(
            kept.refresh_token
                .matches(&Credential::new("refresh-1".to_owned()))
        );
        assert_eq!(
            signing_key_of(&kept.master_seed),
            signing_key_of(&MasterSeed::from_array([7; 32]))
        );
        assert!(decode(&Confidential::<Plaintext>::new(vec![9; 40])).is_none());
    }

    /// `SRV-322`: the relay renews the kept refresh token, and the session
    /// resumes under the kept seed with the new pair. The one request is the
    /// renewal, and it carries the kept token alone.
    #[tokio::test]
    async fn a_kept_session_resumes_with_the_renewed_pair() {
        let transport = Scripted::new([Ok((200, renewed("access-2")))]);
        let resumed = resume_via(
            &transport,
            &api(),
            Loaded::Opened(record("Ada", "refresh-1", [7; 32])),
        )
        .await;
        let Resumed::Ready { kept, tokens } = resumed else {
            panic!("the session did not resume");
        };
        assert_eq!(
            signing_key_of(&kept.master_seed),
            signing_key_of(&MasterSeed::from_array([7; 32]))
        );
        assert!(
            tokens
                .refresh_token()
                .matches(&Credential::new("refresh-2".to_owned()))
        );
        let sent = transport.sent.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].url.reveal(), "https://relay.test/api/auth/refresh");
        assert!(sent[0].bearer.is_none(), "no access token at boot");
        let body = sent[0]
            .body
            .as_ref()
            .map(|(_, body)| body.as_bytes().to_vec());
        let body: serde_json::Value = serde_json::from_slice(&body.unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({ "refresh_token": "refresh-1" }));
    }

    /// `SRV-325`: a kept refresh token the relay refuses at boot erases the
    /// record, and the page stays on the sign-in screen. A record whose seal
    /// or whose form does not open is erased too. No record sends nothing. A relay that does not
    /// answer leaves the record for a later boot.
    #[tokio::test]
    async fn a_refused_or_unreadable_record_is_erased_and_resumes_nothing() {
        let refused = Scripted::new([Ok((401, serde_json::json!({})))]);
        let resumed = resume_via(
            &refused,
            &api(),
            Loaded::Opened(record("Ada", "refresh-1", [7; 32])),
        )
        .await;
        assert!(matches!(resumed, Resumed::Refused));

        let silent = Scripted::new([]);
        let resumed = resume_via(
            &silent,
            &api(),
            Loaded::Opened(record("Ada", "refresh-1", [7; 32])),
        )
        .await;
        assert!(matches!(resumed, Resumed::Unreachable));

        let garbage = Loaded::Opened(Confidential::<Plaintext>::new(vec![1; 16]));
        let resumed = resume_via(&silent, &api(), garbage).await;
        assert!(matches!(resumed, Resumed::Unreadable));
        let resumed = resume_via(&silent, &api(), Loaded::Unopened).await;
        assert!(matches!(resumed, Resumed::Unreadable));

        let none = Scripted::new([]);
        let resumed = resume_via(&none, &api(), Loaded::Absent).await;
        assert!(matches!(resumed, Resumed::Absent));
        assert!(none.sent.borrow().is_empty(), "no record sends nothing");
    }

    fn worker() -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        (
            SyncState::new(sink, http::Url::<http::RelayHost>::new(String::new())),
            rx,
        )
    }

    fn memory(state: &SyncState) -> Rc<RefCell<MemoryVault>> {
        match &state.kept.vault {
            Vault::Memory(memory) => Rc::clone(memory),
            #[cfg(target_arch = "wasm32")]
            Vault::Web(_) => panic!("a host worker keeps its session in memory"),
        }
    }

    /// A session of `name` under `seed` is ready, keeping as `kept` says.
    fn ready(state: &mut SyncState, name: &str, seed: [u8; 32], kept: Keeping) -> Id<User> {
        let user_id = Id::<User>::mint();
        let master_seed = MasterSeed::from_array(seed);
        let profile = profile(name);
        state.handle_sync_event(SyncEvent::SessionReady {
            id: None,
            user_id,
            display_name: profile.display_name,
            email: profile.email,
            signing_key: master_seed.derive_signing_key(),
            exchange_key: master_seed.derive_exchange_key(),
            master_seed,
            tokens: pair("refresh-1"),
            kept,
        });
        user_id
    }

    /// `SRV-322`, `SRV-323`: a provider session is kept as it starts, and a
    /// sign-out erases the record, so nothing on the device opens.
    #[test]
    fn a_session_is_kept_and_a_sign_out_erases_it() {
        let (mut state, _rx) = worker();
        let user = ready(&mut state, "Ada", [7; 32], Keeping::Under(DeviceKey::Fresh));
        let vault = memory(&state);
        assert_eq!(vault.borrow().kept().map(|k| k.user_id), Some(user));

        state.handle_sign_out(ambre_bus::next_request());
        assert!(
            vault.borrow().kept().is_none(),
            "the sign-out left a record"
        );
        state.handle_sync_event(SyncEvent::TokensRenewed);
        assert!(
            vault.borrow().kept().is_none(),
            "a late renewal kept the session again"
        );
    }

    /// `SRV-324`: the sign-in of another account on the device keeps its own
    /// session under a fresh key, in place of the first. The first record no
    /// longer opens: its key is gone with it.
    #[test]
    fn another_account_keeps_its_session_under_a_fresh_key() {
        let (mut state, _rx) = worker();
        ready(&mut state, "Ada", [7; 32], Keeping::Under(DeviceKey::Fresh));
        let bob = ready(&mut state, "Bob", [8; 32], Keeping::Under(DeviceKey::Fresh));
        let vault = memory(&state);
        let kept = vault.borrow().kept().unwrap();
        assert_eq!(kept.user_id, bob);
        assert_eq!(
            signing_key_of(&kept.master_seed),
            signing_key_of(&MasterSeed::from_array([8; 32]))
        );
        assert_eq!(vault.borrow().keys(), [DeviceKey::Fresh, DeviceKey::Fresh]);
    }

    /// `SRV-326`: a renewal keeps the next refresh token under the key held,
    /// since the relay honours the previous one once.
    #[test]
    fn a_renewal_keeps_the_next_refresh_token() {
        let (mut state, _rx) = worker();
        ready(&mut state, "Ada", [7; 32], Keeping::Under(DeviceKey::Fresh));
        *state.tokens.borrow_mut() = Some(pair("refresh-2"));
        state.handle_sync_event(SyncEvent::TokensRenewed);
        let vault = memory(&state);
        let kept = vault.borrow().kept().unwrap();
        assert!(
            kept.refresh_token
                .matches(&Credential::new("refresh-2".to_owned()))
        );
        assert_eq!(vault.borrow().keys(), [DeviceKey::Fresh, DeviceKey::Held]);
    }

    /// `SRV-327`: a development session keeps nothing, and its renewals keep
    /// nothing either.
    #[test]
    fn a_development_session_keeps_nothing() {
        let (mut state, _rx) = worker();
        ready(&mut state, "Ada", [7; 32], Keeping::Nothing);
        state.handle_sync_event(SyncEvent::TokensRenewed);
        let vault = memory(&state);
        assert!(vault.borrow().kept().is_none());
        assert!(vault.borrow().keys().is_empty());
    }

    /// `SRV-325`: with no relay, a resume answers that no session is kept.
    #[test]
    fn a_resume_with_no_relay_answers_no_kept_session() {
        let (mut state, rx) = worker();
        let id = ambre_bus::next_request();
        state.handle_resume(id);
        let told: Vec<LogicEvent> = rx.try_iter().collect();
        assert!(matches!(
            told.as_slice(),
            [LogicEvent::Rest {
                evt: RestEvent::NoKeptSession,
                ..
            }]
        ));
    }
}
