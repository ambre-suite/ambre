//! The session a device keeps across a page reload (`ambre-spec-session.md`
//! section 3, `SRV-323` to `SRV-331`; `ambre-spec-zero-knowledge.md`,
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
//!   account on the device no longer opens (`SRV-325`).
//! * Every renewal runs under one lock of the origin, which each tab shares.
//!   It sends the refresh token the record holds at that instant, and writes
//!   the next one before the lock is released (`SRV-327`, `SRV-330`).
//! * A sign-out erases the record and its key (`SRV-324`).
//! * At boot the page asks for a resume. The worker renews the kept refresh
//!   token, then reads the account answer (`SRV-329`). A relay that refuses
//!   the token for good erases the record (`SRV-326`). Any other failure
//!   keeps it, and the worker tries again after a delay (`SRV-331`).
//! * A development session keeps nothing (`SRV-328`).

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;

use ambre_core::{
    Confidential, Count, Failures, HttpStatus, LocalFailure, Ms, Plaintext, RecoveryWords, TimeSpan,
};

use super::*;
use crate::http::HttpError;

/// The delay before the first retry of a resume that failed short of a
/// refusal, doubled at each failure (`ambre-spec-budgets.md` section 8).
const RESUME_RETRY_BASE: TimeSpan<Ms> = TimeSpan::<Ms>::new(1_000);

/// The failures in a row a resume retries. One more leaves the page on the
/// sign-in screen, with the record kept for the next boot
/// (`ambre-spec-budgets.md` section 8, `SRV-331`).
const RESUME_RETRY_LIMIT: Count<Failures> = Count::new(5);

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
    /// The key the vault holds since the last keep or load, or a new one
    /// when it holds none.
    Held,
}

/// What a session ready to run keeps of itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keeping {
    /// Nothing: a development session (`SRV-328`).
    #[cfg_attr(
        all(not(feature = "dev-login"), not(test)),
        expect(dead_code, reason = "only the development sign-in keeps nothing")
    )]
    Nothing,
    /// The session, sealed under `DeviceKey`.
    Under(DeviceKey),
    /// The session the record holds already: a resume wrote it under the
    /// lock. A second write would land outside the lock, over a token another
    /// tab may have renewed since (`SRV-330`).
    #[cfg_attr(
        all(not(target_arch = "wasm32"), not(test)),
        expect(dead_code, reason = "only the worker resumes a kept session")
    )]
    Current,
}

/// What the vault read of the record.
pub(crate) enum Loaded {
    /// The device keeps no record.
    Absent,
    /// A record is kept, and its seal did not open.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(dead_code, reason = "only a WebCrypto seal fails to open")
    )]
    Unopened,
    /// The record, opened.
    Opened(Confidential<Plaintext>),
}

/// Whether a refused renewal ends the refresh token for good. The relay
/// answers `401` alone for a token it no longer honours: revoked, expired or
/// claimed already (`SRV-009`). A rate limit, another refusal, a fault of
/// the relay and a network failure say nothing of the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// The token opens nothing any more.
    Final,
    /// The same token may still renew later.
    Transient,
}

impl Verdict {
    /// The verdict a failed renewal `e` carries (`SRV-326`, `SRV-331`).
    fn of(e: &HttpError) -> Self {
        match e.failure() {
            ApiFailure::Relay { status, .. } if status == HttpStatus::new(401) => Self::Final,
            _ => Self::Transient,
        }
    }
}

/// What one renewal under the lock of the record came to.
enum Rotation {
    /// The relay renewed the token into `tokens`. `kept` is the record the
    /// renewal read and wrote again, `None` when the device keeps no record
    /// of the session.
    Renewed {
        tokens: http::TokenPair,
        kept: Option<Kept>,
    },
    /// The device keeps no record, and the caller holds no token: nothing
    /// was sent.
    Absent,
    /// The record does not open, or opens as no session. It was erased.
    Unreadable,
    /// The relay refused the token for good. A record that held it was
    /// erased.
    Refused(HttpError),
    /// The relay did not renew the token, and may later.
    Failed(HttpError),
}

/// Renews the session the record keeps, under the lock of the record that
/// every tab of the origin shares (`SRV-330`).
///
/// The renewal re-reads the record under the lock. When it holds a session
/// of `owner`, or of anyone when `owner` is `None`, the renewal sends its
/// refresh token: another tab may have renewed the one `held` names. It then
/// writes the next token before the lock is released (`SRV-327`). Without
/// such a record, the renewal sends `held` and writes nothing, so a record
/// another tab erased or replaced stays as that tab left it.
async fn rotate_under(
    transport: &impl http::Transport,
    api: &http::Url<http::ApiRoot>,
    vault: &Vault,
    owner: Option<Id<User>>,
    held: Option<http::Credential<http::RefreshToken>>,
) -> Rotation {
    vault
        .exclusive(async {
            let read = match vault.load().await {
                Loaded::Absent => None,
                Loaded::Unopened => Some(None),
                Loaded::Opened(record) => Some(decode(&record)),
            };
            let (mine, unreadable) = match read {
                None => (None, false),
                Some(None) => (None, true),
                Some(Some(kept)) if owner.is_none_or(|owner| owner == kept.user_id) => {
                    (Some(kept), false)
                }
                Some(Some(_)) => (None, false),
            };
            let sent = match (&mine, held) {
                (Some(kept), _) => kept.refresh_token.clone(),
                (None, Some(held)) => held,
                (None, None) if unreadable => {
                    vault.erase_now().await;
                    return Rotation::Unreadable;
                }
                (None, None) => return Rotation::Absent,
            };
            match http::redeem_via(transport, api, &sent).await {
                Ok(tokens) => {
                    if let Some(kept) = &mine {
                        match encode(
                            kept.user_id,
                            &kept.profile,
                            tokens.refresh_token(),
                            &kept.master_seed,
                        ) {
                            Some(plaintext) => vault.put(plaintext).await,
                            None => {
                                crate::log::warning!("Sync: the renewed session did not encode");
                            }
                        }
                    }
                    Rotation::Renewed { tokens, kept: mine }
                }
                Err(e) if Verdict::of(&e) == Verdict::Final => {
                    if mine.is_some() {
                        vault.erase_now().await;
                    }
                    Rotation::Refused(e)
                }
                Err(e) => Rotation::Failed(e),
            }
        })
        .await
}

/// The record of a running session, as a renewal reaches it.
#[derive(Clone)]
pub(crate) struct KeptLink {
    vault: Vault,
    user_id: Id<User>,
}

impl KeptLink {
    /// Renews the session through `transport` to the relay at `api`, under
    /// the lock of the record (`SRV-330`). `held` is the refresh token the
    /// session holds in memory. It is sent only when the record holds no
    /// session of this account.
    pub(crate) async fn rotate_via(
        &self,
        transport: &impl http::Transport,
        api: &http::Url<http::ApiRoot>,
        held: http::Credential<http::RefreshToken>,
    ) -> Result<http::TokenPair, HttpError> {
        match rotate_under(transport, api, &self.vault, Some(self.user_id), Some(held)).await {
            Rotation::Renewed { tokens, .. } => Ok(tokens),
            Rotation::Refused(e) | Rotation::Failed(e) => Err(e),
            // A held token is always sent, so neither is reached.
            Rotation::Absent | Rotation::Unreadable => {
                Err(HttpError::Local(LocalFailure::NotSignedIn))
            }
        }
    }
}

/// What the boot of a page found of a kept session.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "only the worker resumes a kept session")
)]
pub(crate) enum Resumed {
    /// The device keeps no record.
    Absent,
    /// The record does not open, or opens as no session. It was erased.
    Unreadable,
    /// The relay refused the kept refresh token for good. The record was
    /// erased.
    Refused,
    /// The relay did not answer, or failed, past the last retry. The record
    /// stays for a later boot.
    Unreachable,
    /// The relay renewed the kept refresh token into `tokens`, and `me` is
    /// its account answer.
    Ready {
        kept: Kept,
        tokens: http::TokenPair,
        me: http::AuthMeResponse,
    },
}

/// Resumes the session the record of `vault` keeps, through `transport` to
/// the relay at `api` (`SRV-323`, `SRV-326`, `SRV-329`, `SRV-331`).
///
/// The kept refresh token is the one credential the renewal sends. The
/// access token it brings then reads the account answer, so the session
/// learns whether a provider sign-in reclaimed its address. A failure short
/// of a refusal waits through `pause`, then tries again, up to the limit.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "only the worker resumes a kept session")
)]
pub(crate) async fn resume_via<F: Future<Output = ()>>(
    transport: &impl http::Transport,
    api: &http::Url<http::ApiRoot>,
    vault: &Vault,
    pause: impl Fn(TimeSpan<Ms>) -> F,
) -> Resumed {
    let mut failures = Count::<Failures>::new(0);
    let (kept, tokens) = loop {
        match rotate_under(transport, api, vault, None, None).await {
            Rotation::Renewed {
                tokens,
                kept: Some(kept),
            } => break (kept, tokens),
            Rotation::Renewed { kept: None, .. } | Rotation::Absent => return Resumed::Absent,
            Rotation::Unreadable => return Resumed::Unreadable,
            Rotation::Refused(_) => return Resumed::Refused,
            Rotation::Failed(e) if failures < RESUME_RETRY_LIMIT => {
                crate::log::warning!("Sync: the kept session did not renew, retrying: {e}");
                pause(RESUME_RETRY_BASE.doubled(failures)).await;
                failures = failures.incremented();
            }
            Rotation::Failed(_) => return Resumed::Unreachable,
        }
    };
    let url = api.route("/auth/me");
    let me = loop {
        match http::get_json_bearer_via::<http::AuthMeResponse>(
            transport,
            &url,
            tokens.access_token(),
        )
        .await
        {
            Ok(me) => break me,
            Err(e) if failures < RESUME_RETRY_LIMIT => {
                crate::log::warning!("Sync: the account answer did not read, retrying: {e}");
                pause(RESUME_RETRY_BASE.doubled(failures)).await;
                failures = failures.incremented();
            }
            // The record already holds the renewed token, so a later boot
            // resumes from it.
            Err(_) => return Resumed::Unreachable,
        }
    };
    if me.user_id != kept.user_id {
        // The relay answers for another account than the record names: the
        // record opens no session of its own.
        vault.exclusive(vault.erase_now()).await;
        return Resumed::Unreadable;
    }
    Resumed::Ready { kept, tokens, me }
}

/// The event a resumed session answers: the session of `kept`, under the
/// renewed `tokens`, as the account answer `me` names the person. The record
/// holds it already, so the worker writes nothing more.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "only the worker resumes a kept session")
)]
fn resumed_session(
    id: Seq<Request>,
    kept: Kept,
    tokens: http::TokenPair,
    me: http::AuthMeResponse,
) -> SyncEvent {
    SyncEvent::SessionReady {
        id: Some(id),
        user_id: kept.user_id,
        display_name: me.display_name,
        email: me.email,
        // The same flag a sign-in reads, so the launcher names a reclaim
        // after a reload too (`SRV-329`, `CHROME-146`).
        address: me.address_reclaimed,
        signing_key: kept.master_seed.derive_signing_key(),
        exchange_key: kept.master_seed.derive_exchange_key(),
        master_seed: kept.master_seed,
        tokens,
        kept: Keeping::Current,
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

/// The memory vault, and the lock that stands in for the lock of the origin.
/// Two workers of a host test that share it act as two tabs.
#[derive(Default)]
pub(crate) struct MemoryShared {
    pub(crate) vault: RefCell<MemoryVault>,
    gate: futures_util::lock::Mutex<()>,
}

/// Where the record lives.
#[derive(Clone)]
pub(crate) enum Vault {
    /// In memory, on a host build.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the worker keeps its session in IndexedDB")
    )]
    Memory(Rc<MemoryShared>),
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
    /// Keeps `plaintext` under `key`, in place of any record. The write is
    /// queued: a sign-in waits for nothing.
    fn keep(&self, plaintext: Confidential<Plaintext>, key: DeviceKey) {
        match self {
            Self::Memory(memory) => {
                let mut memory = memory.vault.borrow_mut();
                memory.record = Some(plaintext);
                memory.keys.push(key);
            }
            #[cfg(target_arch = "wasm32")]
            Self::Web(web) => web.enqueue(web::Op::Keep(plaintext, key)),
        }
    }

    /// Keeps `plaintext` under the key held, and returns once the write
    /// landed. A renewal calls it under the lock, so the next tab reads it.
    async fn put(&self, plaintext: Confidential<Plaintext>) {
        match self {
            Self::Memory(_) => self.keep(plaintext, DeviceKey::Held),
            #[cfg(target_arch = "wasm32")]
            Self::Web(web) => {
                web.landed(web::Op::Keep(plaintext, DeviceKey::Held)).await;
            }
        }
    }

    /// Erases the record and the key it was sealed under. The write is
    /// queued.
    fn erase(&self) {
        match self {
            Self::Memory(memory) => memory.vault.borrow_mut().record = None,
            #[cfg(target_arch = "wasm32")]
            Self::Web(web) => web.enqueue(web::Op::Erase),
        }
    }

    /// Erases the record, and returns once the write landed.
    async fn erase_now(&self) {
        match self {
            Self::Memory(_) => self.erase(),
            #[cfg(target_arch = "wasm32")]
            Self::Web(web) => web.landed(web::Op::Erase).await,
        }
    }

    /// The record the device keeps, opened. The vault holds its key for the
    /// next keep.
    async fn load(&self) -> Loaded {
        match self {
            Self::Memory(memory) => match &memory.vault.borrow().record {
                Some(record) => Loaded::Opened(record.clone()),
                None => Loaded::Absent,
            },
            #[cfg(target_arch = "wasm32")]
            Self::Web(web) => web.load().await,
        }
    }

    /// Runs `critical` under the lock of the record, which every tab of the
    /// origin shares (`SRV-330`). The device keeps one record, whatever the
    /// account, so the lock is one per origin: a sign-in of another account
    /// and a renewal never write the record at once.
    async fn exclusive<R>(&self, critical: impl Future<Output = R>) -> R {
        match self {
            Self::Memory(memory) => {
                let _turn = memory.gate.lock().await;
                critical.await
            }
            #[cfg(target_arch = "wasm32")]
            Self::Web(_) => web::exclusive(critical).await,
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
        match keeping {
            Keeping::Nothing => {}
            Keeping::Current => self.kept.profile = Some(profile),
            Keeping::Under(key) => {
                self.kept.profile = Some(profile);
                self.keep_session(key);
            }
        }
    }

    /// The record a renewal of the running session reads and writes, or
    /// `None` when the device keeps no session (`SRV-330`).
    pub(crate) fn kept_link(&self) -> Option<KeptLink> {
        self.kept.profile.as_ref()?;
        Some(KeptLink {
            vault: self.kept.vault.clone(),
            user_id: self.user_id?,
        })
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

    /// Erases the kept session (`SRV-324`). The record and its key go
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
    /// keeps (`SRV-323`). A ready session answers `SessionReady`. Anything
    /// else answers `NoKeptSession`, and the page stays on the sign-in
    /// screen (`SRV-326`).
    pub(crate) fn handle_resume(&mut self, id: Seq<Request>) {
        crate::log::info!("Sync: ResumeSession");
        #[cfg(target_arch = "wasm32")]
        if let (Some(tx), Vault::Web(_)) = (self.event_tx.clone(), &self.kept.vault) {
            use http::Transport as _;

            let vault = self.kept.vault.clone();
            let logic_tx = self.logic_tx.clone();
            let api = http::Url::<http::ApiRoot>::of(self.url_scheme, &self.api_base);
            crate::spawn(async move {
                let pause = |span| async move { http::Fetch.elapse(span).await };
                match resume_via(&http::Fetch, &api, &vault, pause).await {
                    Resumed::Ready { kept, tokens, me } => {
                        let _ = tx.send(resumed_session(id, kept, tokens, me));
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

    /// The one write a queue entry holds, and the resolver of the promise
    /// its writer waits on, if any.
    type Entry = (Op, Option<js_sys::Function>);

    /// The database, its one store and the one key of the record.
    const DATABASE: StaticKey<StorageKeys> = StaticKey::new("ambre-session");
    const STORE: StaticKey<StorageKeys> = StaticKey::new("session");
    const RECORD: StaticKey<StorageKeys> = StaticKey::new("kept");
    /// The Web Lock every tab of the origin takes before it renews the kept
    /// session (`SRV-330`). Like the database, it is a name of the origin.
    const LOCK: StaticKey<StorageKeys> = StaticKey::new("ambre-session-renewal");

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
        queue: Rc<RefCell<VecDeque<Entry>>>,
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
            self.push((op, None));
        }

        /// Queues `op`, and returns once it landed, after every write queued
        /// before it. A renewal waits on it under the lock of the record, so
        /// the next tab reads the token it wrote (`SRV-330`).
        pub(crate) async fn landed(&self, op: Op) {
            let mut done = None;
            let landed = js_sys::Promise::new(&mut |resolve, _reject| done = Some(resolve));
            self.push((op, done));
            let _ = JsFuture::from(landed).await;
        }

        fn push(&self, entry: Entry) {
            self.queue.borrow_mut().push_back(entry);
            if self.drain.get() == Drain::Busy {
                return;
            }
            self.drain.set(Drain::Busy);
            let vault = self.clone();
            crate::spawn(async move {
                loop {
                    let next = vault.queue.borrow_mut().pop_front();
                    let Some((op, done)) = next else { break };
                    if let Err(e) = vault.apply(op).await {
                        crate::log::warning!("Sync: the kept session was not written: {e:?}");
                    }
                    if let Some(done) = done {
                        let _ = done.call0(&JsValue::NULL);
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

    /// Runs `critical` under the exclusive Web Lock of the record. Every tab
    /// of the origin asks for the same lock, so one renewal runs at a time
    /// across them (`SRV-330`). A scope with no lock manager runs `critical`
    /// alone, with a warning: an older browser loses the order across tabs,
    /// and still renews.
    pub(crate) async fn exclusive<R>(critical: impl std::future::Future<Output = R>) -> R {
        let held = match acquire().await {
            Ok(held) => Some(held),
            Err(e) => {
                crate::log::warning!("Sync: no lock across tabs, renewing alone: {e:?}");
                None
            }
        };
        let out = critical.await;
        drop(held);
        out
    }

    /// Runs `critical` under the Web Lock of the record, or fails when the
    /// scope has no lock manager. The wasm-tier test of `SRV-330` reads it,
    /// since the fallback of [`exclusive`] would hide a missing lock.
    #[doc(hidden)]
    pub async fn strictly_exclusive<R>(
        critical: impl std::future::Future<Output = R>,
    ) -> Result<R, JsValue> {
        let held = acquire().await?;
        let out = critical.await;
        drop(held);
        Ok(out)
    }

    /// A Web Lock granted. Dropping it settles the promise the lock waits
    /// on, which releases it, even when the critical future never ends.
    struct HeldLock {
        release: js_sys::Function,
    }

    impl Drop for HeldLock {
        fn drop(&mut self) {
            let _ = self.release.call0(&JsValue::NULL);
        }
    }

    /// Asks the lock manager of the scope for [`LOCK`], exclusive, and waits
    /// until it is granted. `web-sys` puts the Web Locks API behind its
    /// unstable flag, so the call goes through the object itself.
    async fn acquire() -> Result<HeldLock, JsValue> {
        let navigator = js_sys::Reflect::get(&js_sys::global(), &"navigator".into())?;
        let locks = js_sys::Reflect::get(&navigator, &"locks".into())?;
        if locks.is_undefined() || locks.is_null() {
            return Err(JsValue::from_str("no lock manager"));
        }
        let request: js_sys::Function =
            js_sys::Reflect::get(&locks, &"request".into())?.dyn_into()?;
        let mut release = None;
        let held = js_sys::Promise::new(&mut |resolve, _reject| release = Some(resolve));
        let mut grant = None;
        let granted = js_sys::Promise::new(&mut |resolve, _reject| grant = Some(resolve));
        let (Some(release), Some(grant)) = (release, grant) else {
            return Err(JsValue::from_str("a promise gave no resolver"));
        };
        // The manager calls this once the lock is free, and holds the lock
        // until the promise it returns settles.
        let on_grant = wasm_bindgen::closure::Closure::once_into_js(move |_lock: JsValue| {
            let _ = grant.call0(&JsValue::NULL);
            JsValue::from(held)
        });
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &"mode".into(), &"exclusive".into())?;
        let guard = HeldLock { release };
        request.call3(&locks, &LOCK.as_str().into(), &options, &on_grant)?;
        JsFuture::from(granted).await?;
        Ok(guard)
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
    use std::cell::Cell;

    use super::*;
    use crate::http::scripted::{Scripted, renewed};
    use crate::http::{Credential, HttpBody, Incoming, Outgoing, TokenPair, TransportError};
    use ambre_core::Payload;

    fn profile(name: &str) -> KeptProfile {
        KeptProfile {
            display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse(name).unwrap(),
            email: format!("{name}@example.org").into(),
        }
    }

    fn pair(access: &str, refresh: &str) -> TokenPair {
        TokenPair::new(
            Credential::new(access.to_owned()),
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

    /// The record of `user`, named `name`, under `seed`, holding `refresh`.
    fn record_of(
        user: Id<User>,
        name: &str,
        refresh: &str,
        seed: [u8; 32],
    ) -> Confidential<Plaintext> {
        encode(
            user,
            &profile(name),
            &Credential::new(refresh.to_owned()),
            &MasterSeed::from_array(seed),
        )
        .unwrap()
    }

    /// A memory vault that keeps the record of `user` holding `refresh`.
    fn vault_with(user: Id<User>, refresh: &str) -> (Vault, Rc<MemoryShared>) {
        let shared = Rc::new(MemoryShared::default());
        shared.vault.borrow_mut().record = Some(record_of(user, "Ada", refresh, [7; 32]));
        (Vault::Memory(Rc::clone(&shared)), shared)
    }

    /// The refresh token the vault keeps, if any.
    fn kept_token(shared: &MemoryShared) -> Option<String> {
        let kept = shared.vault.borrow().kept()?;
        ["refresh-1", "refresh-2", "refresh-3"]
            .into_iter()
            .find(|t| {
                kept.refresh_token
                    .matches(&Credential::new((*t).to_owned()))
            })
            .map(str::to_owned)
    }

    fn signing_key_of(seed: &MasterSeed) -> [u8; 32] {
        seed.derive_signing_key().verifying_key().to_bytes()
    }

    /// The account answer of `user`, with its reclaim flag when `reclaimed`
    /// is set.
    fn me(user: Id<User>, reclaimed: Option<bool>) -> serde_json::Value {
        let mut answer = serde_json::json!({
            "user_id": user.to_string(),
            "display_name": "Ada",
            "email": "ada@example.org",
        });
        if let Some(reclaimed) = reclaimed {
            answer["address_reclaimed"] = serde_json::Value::Bool(reclaimed);
        }
        answer
    }

    /// The refresh token the body of a renewal `request` carries.
    fn sent_token(request: &Outgoing) -> String {
        let body = request.body.as_ref().map(|(_, b)| b.as_bytes().to_vec());
        let body: serde_json::Value = serde_json::from_slice(&body.unwrap()).unwrap();
        body["refresh_token"].as_str().unwrap().to_owned()
    }

    /// The delays a pause was asked for, in milliseconds, oldest first.
    type Waited = Rc<RefCell<Vec<u64>>>;

    /// A pause that records each delay it was asked for, and waits none.
    fn recorder() -> (Waited, impl Fn(TimeSpan<Ms>) -> std::future::Ready<()>) {
        let waited = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&waited);
        (waited, move |span: TimeSpan<Ms>| {
            log.borrow_mut().push(span.get());
            std::future::ready(())
        })
    }

    /// `SRV-323`: a kept record opens as the same person, the same refresh
    /// token and the same seed. A record of no known form opens nothing.
    #[test]
    fn a_kept_record_opens_as_the_same_person_token_and_seed() {
        let user = Id::<User>::mint();
        let kept = decode(&record_of(user, "Ada", "refresh-1", [7; 32])).unwrap();
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

    /// `SRV-323`, `SRV-327`: the relay renews the kept refresh token, and the
    /// session resumes under the kept seed with the new pair. The renewal
    /// carries the kept token alone, and the record keeps the next one.
    #[tokio::test]
    async fn a_kept_session_resumes_with_the_renewed_pair() {
        let user = Id::<User>::mint();
        let (vault, shared) = vault_with(user, "refresh-1");
        let transport = Scripted::new([Ok((200, renewed("access-2"))), Ok((200, me(user, None)))]);
        let (waited, pause) = recorder();
        let Resumed::Ready { kept, tokens, .. } =
            resume_via(&transport, &api(), &vault, pause).await
        else {
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
        assert_eq!(kept_token(&shared).as_deref(), Some("refresh-2"));
        assert!(waited.borrow().is_empty());
        let sent = transport.sent.borrow();
        assert_eq!(sent[0].url.reveal(), "https://relay.test/api/auth/refresh");
        assert!(sent[0].bearer.is_none(), "no access token at boot");
        assert_eq!(sent_token(&sent[0]), "refresh-1");
    }

    /// `SRV-329`: a resumed session reads the account answer under the
    /// renewed access token, and carries its reclaim flag into the session
    /// event, as a sign-in does. The worker then tells the reclaim after the
    /// session (`CHROME-146`).
    #[tokio::test]
    async fn a_resumed_session_reads_the_reclaim_of_its_address() {
        for (flag, standing) in [
            (Some(true), ambre_core::AddressStanding::Reclaimed),
            (Some(false), ambre_core::AddressStanding::Held),
            (None, ambre_core::AddressStanding::Held),
        ] {
            let user = Id::<User>::mint();
            let (vault, _) = vault_with(user, "refresh-1");
            let transport =
                Scripted::new([Ok((200, renewed("access-2"))), Ok((200, me(user, flag)))]);
            let (_, pause) = recorder();
            let Resumed::Ready { kept, tokens, me } =
                resume_via(&transport, &api(), &vault, pause).await
            else {
                panic!("the session did not resume");
            };
            assert_eq!(
                transport.calls()[1],
                (
                    http::Method::Get,
                    "https://relay.test/api/auth/me".to_owned()
                )
            );
            assert!(
                transport.sent.borrow()[1]
                    .bearer
                    .as_ref()
                    .is_some_and(|b| b.matches(&Credential::new("access-2".to_owned())))
            );

            let (mut state, rx) = worker();
            let id = ambre_bus::next_request();
            let event = resumed_session(id, kept, tokens, me);
            let SyncEvent::SessionReady { address, kept, .. } = &event else {
                panic!("a resume answers a session");
            };
            assert_eq!(*address, standing);
            assert_eq!(*kept, Keeping::Current);
            state.handle_sync_event(event);
            let reclaim_told = rx.try_iter().any(|e| {
                matches!(
                    e,
                    LogicEvent::Rest {
                        evt: RestEvent::AddressReclaimed,
                        ..
                    }
                )
            });
            assert_eq!(
                reclaim_told,
                standing == ambre_core::AddressStanding::Reclaimed
            );
        }
    }

    /// `SRV-326`: a kept refresh token the relay refuses at boot with `401`,
    /// the one answer it gives a dead token, erases the record. A record whose
    /// form does not open is erased too. No record sends nothing.
    #[tokio::test]
    async fn a_refused_or_unreadable_record_is_erased_and_resumes_nothing() {
        let user = Id::<User>::mint();
        let (vault, shared) = vault_with(user, "refresh-1");
        let refused = Scripted::new([Ok((401, serde_json::json!({"error": "UNAUTHORIZED"})))]);
        let (waited, pause) = recorder();
        let resumed = resume_via(&refused, &api(), &vault, pause).await;
        assert!(matches!(resumed, Resumed::Refused));
        assert!(shared.vault.borrow().kept().is_none(), "the record stayed");
        assert!(waited.borrow().is_empty(), "a refusal is never retried");

        shared.vault.borrow_mut().record = Some(Confidential::<Plaintext>::new(vec![1; 16]));
        let silent = Scripted::new([]);
        let (_, pause) = recorder();
        let resumed = resume_via(&silent, &api(), &vault, pause).await;
        assert!(matches!(resumed, Resumed::Unreadable));
        assert!(shared.vault.borrow().record.is_none());

        let (_, pause) = recorder();
        let resumed = resume_via(&silent, &api(), &vault, pause).await;
        assert!(matches!(resumed, Resumed::Absent));
        assert!(silent.sent.borrow().is_empty(), "no record sends nothing");
    }

    /// `SRV-331`: a rate limit, any other refusal, a fault of the relay and a
    /// network failure keep the record. The resume waits, then renews the
    /// same token again.
    #[tokio::test]
    async fn a_transient_failure_keeps_the_record_and_retries() {
        let transient = [
            Ok((429, serde_json::json!({}))),
            Ok((400, serde_json::json!({}))),
            Ok((403, serde_json::json!({}))),
            Ok((422, serde_json::json!({}))),
            Ok((500, serde_json::json!({}))),
            Ok((503, serde_json::json!({}))),
            Err("the network is down".to_owned()),
        ];
        for failure in transient {
            let user = Id::<User>::mint();
            let (vault, shared) = vault_with(user, "refresh-1");
            let transport = Scripted::new([
                failure,
                Ok((200, renewed("access-2"))),
                Ok((200, me(user, None))),
            ]);
            let (waited, pause) = recorder();
            let resumed = resume_via(&transport, &api(), &vault, pause).await;
            assert!(matches!(resumed, Resumed::Ready { .. }));
            assert_eq!(*waited.borrow(), [1_000]);
            let sent = transport.sent.borrow();
            assert_eq!(sent_token(&sent[0]), "refresh-1");
            assert_eq!(sent_token(&sent[1]), "refresh-1", "the same token again");
            assert_eq!(kept_token(&shared).as_deref(), Some("refresh-2"));
        }
    }

    /// `SRV-331`: past the last retry, the page stays on the sign-in screen
    /// and the record stays for the next boot. Each delay doubles the last.
    #[tokio::test]
    async fn a_relay_down_past_the_last_retry_keeps_the_record() {
        let user = Id::<User>::mint();
        let (vault, shared) = vault_with(user, "refresh-1");
        let transport = Scripted::new([]);
        let (waited, pause) = recorder();
        let resumed = resume_via(&transport, &api(), &vault, pause).await;
        assert!(matches!(resumed, Resumed::Unreachable));
        assert_eq!(*waited.borrow(), [1_000, 2_000, 4_000, 8_000, 16_000]);
        assert_eq!(transport.sent.borrow().len(), 6);
        assert_eq!(kept_token(&shared).as_deref(), Some("refresh-1"));
    }

    /// A relay that honours each refresh token once, as `SRV-009` states.
    /// It yields before it answers, so two renewals in flight interleave.
    struct OnceRelay {
        live: RefCell<String>,
        minted: Cell<u32>,
        sent: RefCell<Vec<Outgoing>>,
    }

    impl OnceRelay {
        fn new(live: &str, minted: u32) -> Self {
            Self {
                live: RefCell::new(live.to_owned()),
                minted: Cell::new(minted),
                sent: RefCell::new(Vec::new()),
            }
        }
    }

    impl http::Transport for OnceRelay {
        async fn send(&self, request: &Outgoing) -> Result<Incoming, TransportError> {
            self.sent.borrow_mut().push(request.clone());
            tokio::task::yield_now().await;
            let token = sent_token(request);
            let (status, body) = if token == *self.live.borrow() {
                let n = self.minted.get() + 1;
                self.minted.set(n);
                *self.live.borrow_mut() = format!("refresh-{n}");
                let body = serde_json::json!({
                    "access_token": format!("access-{n}"),
                    "refresh_token": format!("refresh-{n}"),
                    "access_expires_in": 900,
                });
                (200, body)
            } else {
                (401, serde_json::json!({"error": "UNAUTHORIZED"}))
            };
            Ok(Incoming {
                status: HttpStatus::new(status),
                body: Payload::<HttpBody>::new(serde_json::to_vec(&body).unwrap()),
            })
        }

        async fn elapse(&self, _span: TimeSpan<Ms>) {
            std::future::pending().await
        }
    }

    /// One tab of a session of `user` kept in `vault`: its own pair and its
    /// own gate, the record and its lock shared.
    fn tab(vault: &Vault, user: Id<User>, access: &str) -> http::AuthContext {
        let mut tab = http::scripted::session(access, "refresh-1");
        tab.kept = Some(KeptLink {
            vault: vault.clone(),
            user_id: user,
        });
        tab
    }

    /// `SRV-330`: two tabs renew one kept session at the same instant. The
    /// second waits for the lock, re-reads the record and sends the token the
    /// first one kept, so the relay refuses neither and the record holds the
    /// last token.
    #[tokio::test]
    async fn two_tabs_renew_one_kept_token_in_turn() {
        let user = Id::<User>::mint();
        let (vault, shared) = vault_with(user, "refresh-1");
        let first = tab(&vault, user, "access-a");
        let second = tab(&vault, user, "access-b");
        let relay = OnceRelay::new("refresh-1", 1);
        let (stale_a, stale_b) = (
            Credential::new("access-a".to_owned()),
            Credential::new("access-b".to_owned()),
        );
        let (a, b) = futures_util::join!(
            first.renew_via(&relay, &stale_a),
            second.renew_via(&relay, &stale_b),
        );
        assert_eq!(a, Ok(()));
        assert_eq!(b, Ok(()));
        let sent: Vec<String> = relay.sent.borrow().iter().map(sent_token).collect();
        assert_eq!(sent, ["refresh-1", "refresh-2"]);
        assert_eq!(kept_token(&shared).as_deref(), Some("refresh-3"));
    }

    /// `SRV-330`: a tab whose pair holds a token another tab renewed since
    /// sends the token the record keeps, never its own.
    #[tokio::test]
    async fn a_tab_behind_the_record_sends_the_kept_token() {
        let user = Id::<User>::mint();
        let (vault, shared) = vault_with(user, "refresh-2");
        let behind = tab(&vault, user, "access-a");
        let relay = OnceRelay::new("refresh-2", 2);
        let renewed = behind
            .renew_via(&relay, &Credential::new("access-a".to_owned()))
            .await;
        assert_eq!(renewed, Ok(()));
        assert_eq!(sent_token(&relay.sent.borrow()[0]), "refresh-2");
        assert_eq!(kept_token(&shared).as_deref(), Some("refresh-3"));
    }

    /// `SRV-330`: a renewal finds no record of its account, because another
    /// tab signed out or another account signed in. It sends its own token
    /// and writes nothing over what that tab left.
    #[tokio::test]
    async fn a_renewal_writes_no_record_another_tab_erased_or_replaced() {
        let user = Id::<User>::mint();
        let (vault, shared) = vault_with(user, "refresh-2");
        shared.vault.borrow_mut().record = None;
        let relay = OnceRelay::new("refresh-1", 1);
        let alone = tab(&vault, user, "access-a");
        let renewed = alone
            .renew_via(&relay, &Credential::new("access-a".to_owned()))
            .await;
        assert_eq!(renewed, Ok(()));
        assert!(shared.vault.borrow().record.is_none());

        let other = Id::<User>::mint();
        let (vault, shared) = vault_with(other, "refresh-3");
        let relay = OnceRelay::new("refresh-1", 1);
        let foreign = tab(&vault, user, "access-a");
        let renewed = foreign
            .renew_via(&relay, &Credential::new("access-a".to_owned()))
            .await;
        assert_eq!(renewed, Ok(()));
        assert_eq!(sent_token(&relay.sent.borrow()[0]), "refresh-1");
        let kept = shared.vault.borrow().kept().unwrap();
        assert_eq!(kept.user_id, other);
        assert_eq!(kept_token(&shared).as_deref(), Some("refresh-3"));
    }

    fn worker() -> (SyncState, ambre_bus::conduit::Receiver<LogicEvent>) {
        let (sink, rx) = LogicSink::channel();
        (
            SyncState::new(sink, http::Url::<http::RelayHost>::new(String::new())),
            rx,
        )
    }

    fn memory(state: &SyncState) -> Rc<MemoryShared> {
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
            address: ambre_core::AddressStanding::Held,
            signing_key: master_seed.derive_signing_key(),
            exchange_key: master_seed.derive_exchange_key(),
            master_seed,
            tokens: pair("access-1", "refresh-1"),
            kept,
        });
        user_id
    }

    /// `SRV-323`, `SRV-324`: a provider session is kept as it starts, and a
    /// sign-out erases the record, so nothing on the device opens.
    #[test]
    fn a_session_is_kept_and_a_sign_out_erases_it() {
        let (mut state, _rx) = worker();
        let user = ready(&mut state, "Ada", [7; 32], Keeping::Under(DeviceKey::Fresh));
        let vault = memory(&state);
        assert_eq!(vault.vault.borrow().kept().map(|k| k.user_id), Some(user));

        state.handle_sign_out(ambre_bus::next_request());
        assert!(
            vault.vault.borrow().kept().is_none(),
            "the sign-out left a record"
        );
        state.handle_sync_event(SyncEvent::TokensRenewed);
        assert!(
            vault.vault.borrow().kept().is_none(),
            "a late renewal kept the session again"
        );
        assert!(state.kept_link().is_none(), "a renewal reaches no record");
    }

    /// `SRV-325`: the sign-in of another account on the device keeps its own
    /// session under a fresh key, in place of the first. The first record no
    /// longer opens: its key is gone with it.
    #[test]
    fn another_account_keeps_its_session_under_a_fresh_key() {
        let (mut state, _rx) = worker();
        ready(&mut state, "Ada", [7; 32], Keeping::Under(DeviceKey::Fresh));
        let bob = ready(&mut state, "Bob", [8; 32], Keeping::Under(DeviceKey::Fresh));
        let vault = memory(&state);
        let kept = vault.vault.borrow().kept().unwrap();
        assert_eq!(kept.user_id, bob);
        assert_eq!(
            signing_key_of(&kept.master_seed),
            signing_key_of(&MasterSeed::from_array([8; 32]))
        );
        assert_eq!(
            vault.vault.borrow().keys(),
            [DeviceKey::Fresh, DeviceKey::Fresh]
        );
    }

    /// `SRV-327`: a renewal of the running session keeps the next refresh
    /// token under the key held, since the relay honours the previous one
    /// once. The renewal writes it, under the lock.
    #[tokio::test]
    async fn a_renewal_keeps_the_next_refresh_token() {
        let (mut state, _rx) = worker();
        ready(&mut state, "Ada", [7; 32], Keeping::Under(DeviceKey::Fresh));
        let auth = state.auth_context();
        assert!(
            auth.kept.is_some(),
            "a kept session renews through its record"
        );
        let relay = OnceRelay::new("refresh-1", 1);
        let renewed = auth
            .renew_via(&relay, &Credential::new("access-1".to_owned()))
            .await;
        assert_eq!(renewed, Ok(()));
        let vault = memory(&state);
        assert_eq!(kept_token(&vault).as_deref(), Some("refresh-2"));
        assert_eq!(
            vault.vault.borrow().keys(),
            [DeviceKey::Fresh, DeviceKey::Held]
        );
        state.handle_sync_event(SyncEvent::TokensRenewed);
        assert_eq!(
            vault.vault.borrow().keys(),
            [DeviceKey::Fresh, DeviceKey::Held],
            "the renewed event writes nothing outside the lock"
        );
    }

    /// `SRV-328`: a development session keeps nothing, and its renewals keep
    /// nothing either.
    #[test]
    fn a_development_session_keeps_nothing() {
        let (mut state, _rx) = worker();
        ready(&mut state, "Ada", [7; 32], Keeping::Nothing);
        state.handle_sync_event(SyncEvent::TokensRenewed);
        let vault = memory(&state);
        assert!(vault.vault.borrow().kept().is_none());
        assert!(vault.vault.borrow().keys().is_empty());
        assert!(state.kept_link().is_none());
    }

    /// `SRV-330`: a resumed session holds the record the resume wrote, and
    /// writes it no second time.
    #[test]
    fn a_resumed_session_writes_no_second_record() {
        let (mut state, _rx) = worker();
        ready(&mut state, "Ada", [7; 32], Keeping::Current);
        let vault = memory(&state);
        assert!(vault.vault.borrow().keys().is_empty());
        assert!(state.kept_link().is_some());
    }

    /// `SRV-326`: with no relay, a resume answers that no session is kept.
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
