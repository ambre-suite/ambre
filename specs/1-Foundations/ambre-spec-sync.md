# AMBRE — SPEC: AMBRE-SYNC (MODULE SECSYNC)
**Type :** Horizontal (Layer / Engine)
**Statut :** Draft

> Couche de synchronisation chiffrée bout-en-bout entre les clients WASM et le Blind Relay. Ce module implémente le protocole SecSync en Rust pur : il transforme les deltas CRDT produits par le thread Logic en messages cryptographiquement scellés, les achemine via WebSocket, et réintroduit les updates distants dans le document Loro — sans jamais exposer le contenu en clair hors du process client.
>
> `ambre-sync` tourne exclusivement dans le **thread Sync** (un `wasm_thread`, c.-à-d. un Web Worker partageant la mémoire linéaire). Il n'a aucun accès au `LoroDoc` (thread Logic) ni au canvas WebGPU (thread UI).
>
> **Note de modèle.** Le code est en **Modèle B** (1 binaire WASM, 3 threads `wasm_thread` à mémoire partagée, canaux `flume`) — voir `ambre-spec-architecture.md`. Pour `ambre-sync` : (1) la communication Logic ↔ Sync passe par des **canaux `flume`** transportant des **valeurs Rust typées** (`SyncCommand`/`LogicEvent`), **plus de `postMessage`/`MessagePort`/postcard** sur ce chemin ; (2) le point d'entrée est `ambre_sync::run(from_logic, to_logic)` ; (3) **postcard ne subsiste que sur le wire réseau** (`ambre-protocol`). Le pipeline crypto, la quarantaine, le batching et la sécurité (ci-dessous) sont **inchangés** par ce passage.

---

## 1. OBJECTIFS & CONTRAINTES

**Périmètre :** `ambre-sync` est responsable de l'intégralité du pipeline entre le thread Logic et le réseau. Il gère : le chiffrement/déchiffrement des messages SecSync, la gestion du cycle de vie WebSocket, le batching et la compaction, et la numérotation `clock`/`seq`. Il ne connaît pas la sémantique CRDT (déléguée au thread Logic / aux apps), ni le rendu (délégué au thread UI).

**Dépendances :**

| Crate | Rôle | Justification |
|---|---|---|
| `ambre-core` | Types fondamentaux (`Uuid`, `DocType`) | Fondation commune |
| `ambre-crypto` | Primitives crypto (`MasterSeed`, `DocumentKey`, AEAD, sign, Argon2) | Seul consommateur de crypto côté client |
| `ambre-protocol` | Types réseau (`Update`, `Snapshot`, `Ephemeral`, `WsClientMsg`, `WsServerMsg`) | Source de vérité unique — pas de redéfinition locale |
| `ambre-bus` | `SyncCommand`, `LogicEvent` (+ `RestCommand`/`RestEvent`/`RequestId`, ré-exports `ambre-core`) | Interface **typée** avec le thread Logic — les valeurs voyagent par move sur `flume`, **aucune (dé)sérialisation**. `ambre-sync` ne dépend pas directement d'`ambre-core` (types via ré-export d'`ambre-bus`). |
| `chacha20poly1305` (RustCrypto) | XChaCha20-Poly1305 AEAD | Via `ambre-crypto` — listé ici pour la clarté des dépendances transitives |
| `ed25519-dalek` | Signature Ed25519 | Via `ambre-crypto` |
| `blake2` + `hmac` + `sha2` | Hash et commitment tag | Via `ambre-crypto` |
| `ruzstd` | Compression ET décompression zstd | Pure Rust, `wasm32` compatible — une seule crate pour les deux sens |
| `rand_core` + `getrandom` | Génération de nonces | `getrandom` avec feature `wasm_js` pour WASM |
| `lru` | Cache LRU pour `old_keys` et quarantaine | Borné, allocation minimale |
| `flume` | Canaux inter-thread Logic ↔ Sync | Transport **typé** (`SyncCommand`/`LogicEvent`) par move sur la mémoire partagée. Point d'entrée `run(from_logic, to_logic)`. Utilisé en WASM **et** en natif (tests). |
| `postcard` | Encodage wire réseau | **Uniquement** via `ambre-protocol` (WebSocket) : `[version_byte] [postcard(WsServerMsg)]`. Pas de postcard sur le chemin inter-thread. |
| `gloo-net` (features `websocket` + `http`) | WebSocket + HTTP fetch WASM | `web_sys::WebSocket` + `web_sys::fetch` wrappés — **`!Send`, confiné au thread Sync**. (Placeholder forward-looking aujourd'hui ; le code utilise `web_sys::WebSocket` directement.) |
| `web-sys` + `js-sys` + `wasm-bindgen` | `WebSocket` + glue async | Le socket et ses closures `onmessage`/`onclose` vivent dans le thread Sync. |
| `wasm-bindgen-futures` | `spawn_local` pour opérations async | Bridge futures Rust → event loop JS (fetch HTTP, WebSocket connect, rotation) |
| `tokio-tungstenite` (cfg `not(wasm32)`) | WebSocket natif pour tests | Backend de test headless uniquement |

> **Note transport (Modèle B) :** `ambre-sync` reçoit ses commandes en bloquant sur un `flume::Receiver<SyncCommand>` (`from_logic.recv()`) et émet des `LogicEvent` sur un `flume::Sender<LogicEvent>` (`to_logic`). Les valeurs voyagent **par move** dans le heap partagé — aucune sérialisation. Plus de `postMessage`, de `MessagePort`, ni de trait `SyncTransport`. ⚠️ **Limitation `Atomics.wait`** : bloquer sur `flume` dans un Worker WASM gèle l'event-loop JS du Worker (WebSocket/`setTimeout`). C'est toléré tant que le pipeline WS est un placeholder ; **dès que le vrai WebSocket sera câblé, la boucle devra devenir non-bloquante** (`recv_async` + WS piloté par `JsFuture`/`spawn_local`, branche WS désactivée quand `to_logic` est plein). Voir §2.C et `ambre-spec-architecture.md §7 ARCH P1`.

> **Note sur `ambre-bus` :** `ambre-sync` dépend d'`ambre-bus` pour `SyncCommand`/`LogicEvent` (+ la famille service `RestCommand`/`RestEvent`/`RequestId`) — pas pour `LogicCommand`, `UiEvent` ni les types UI. Les anciens traits `AppController`/`CommandSink` n'existent plus.

**Invariants :**

- **Zero plaintext on wire** : aucun octet du payload CRDT n'atteint le réseau sans être chiffré et signé. Propriété vérifiable statiquement : `PlaintextPayload` n'est jamais transmis à `WsTransport`.
- **Ordre causal garanti** : les updates sont toujours appliqués dans l'ordre `(snapshot_id, clock)`. Un update dont le `snapshot_id` est inconnu est mis en quarantaine jusqu'à réception du snapshot parent, puis demande automatique via `WsClientMsg::RequestMissing`.
- **Idempotence** : la ré-application d'un update CRDT déjà intégré est sans effet (propriété Loro héritée). Les doublons réseau sont tolérés.
- **Pas d'`unsafe`** : aucun `unsafe` dans le code applicatif. Les crates RustCrypto sont auditées.
- **Nonce unicité** : nonce XChaCha20 (192 bits) généré via CSPRNG par message. Probabilité de collision sur 2^96 messages cryptographiquement négligeable.
- **Le `WebSocket` (`web_sys`/`gloo-net`) est `!Send`** : créé et utilisé exclusivement dans le thread Sync.
- **Le thread Sync ne compile ni l'UI ni le CRDT** : garanti par le graphe de crates (`ambre-sync` ne dépend ni d'`ambre-canvas` ni de `loro`).

---

## 2. CONCEPTION TECHNIQUE & ÉTAT (STATE)

### A. Modélisation des Données

Les types de messages réseau sont **importés depuis `ambre-protocol`**, pas redéfinis ici.

```rust
// Imports depuis ambre-protocol — source de vérité unique
use ambre_protocol::{
    Update, Snapshot, Ephemeral,
    UpdatePublicData, SnapshotPublicData, EphemeralPublicData,
    WsClientMsg, WsServerMsg,
};
```

**Types internes au thread Sync (non exportés) :**

```rust
/// Byte de version encodé dans le plaintext avant chiffrement.
/// Opaque pour le serveur (dans le ciphertext AEAD).
#[repr(u8)]
enum CompressionVersion {
    Raw  = 0x00,
    Zstd = 0x01,
}

/// Payload avant chiffrement. Ne quitte JAMAIS ce module en clair.
/// Zeroize-able manuellement après usage.
struct PlaintextPayload {
    version: CompressionVersion,
    data: Vec<u8>, // données CRDT brutes ou compressées
}
```

**État interne du thread Sync :**

```rust
/// État global du thread Sync — un seul par tab navigateur.
/// Gère N sessions de sync en parallèle (une par document ouvert,
/// y compris les documents embarqués en read-only dans Toile).
/// (Dans le code : `SyncState`.)
struct SyncState {
    /// Master Seed déchiffrée au login (SyncCommand::Unlock) — ZeroizeOnDrop
    /// Source unique des keypairs signing + exchange.
    master_seed: Option<MasterSeed>,

    /// Clé de signature Ed25519 — dérivée de master_seed au login
    signing_key: Option<ed25519_dalek::SigningKey>,

    /// Clé d'échange X25519 — dérivée de master_seed au login
    exchange_key: Option<x25519_dalek::StaticSecret>,

    /// Sessions de sync actives — une par document ouvert/embarqué
    documents: HashMap<Uuid, SyncDocumentState>,

    /// WebSocket multiplexé unique — ouvert au Unlock, fermé au Logout.
    /// Tous les documents et channels partagent cette connexion via Subscribe/Unsubscribe.
    /// `web_sys::WebSocket` (ou gloo-net), !Send, confiné au thread Sync.
    ws: Option<web_sys::WebSocket>,
    /// Closures JS attachées au WebSocket (onmessage/onclose), gardées vivantes.
    _ws_closures: Vec<Closure<dyn FnMut(MessageEvent)>>,

    /// Cache des DocumentKeys déchiffrées (post-MVP).
    /// Peuplé par OpenDocument et ListDocuments — évite de re-faire X25519 ECDH
    /// à chaque ListDocuments pour N documents (~150µs par ECDH sur mobile).
    /// ZeroizeOnDrop sur chaque entrée. Invalidé au logout.
    /// MVP : pas de cache, ECDH recalculé à chaque ListDocuments (acceptable pour < 20 docs).
    // doc_key_cache: HashMap<Uuid, DocumentKey>,

    /// Canal sortant vers le thread Logic (valeurs `LogicEvent` typées, par move).
    logic_tx: flume::Sender<LogicEvent>,
}

/// État de sync pour un document individuel.
/// Créé par SyncCommand::OpenDocument, détruit par SyncCommand::CloseDocument.
struct SyncDocumentState {
    /// Clé symétrique du document courante — ZeroizeOnDrop
    document_key: DocumentKey,

    /// Dernier snapshot connu (pour construire le `parent_snapshot_proof`)
    current_snapshot_id:   Option<Uuid>,
    current_snapshot_hash: Option<[u8; 32]>,

    /// Compteur d'update local (clock), strictement monotone, jamais remis à zéro
    local_clock: u64,

    /// Compteur anti-replay des messages Ephemeral SORTANTS, monotone par session
    ephemeral_counter: u64,

    /// Anti-replay des messages Ephemeral ENTRANTS (des autres clients).
    /// Clé = pub_key de l'émetteur, valeur = dernier counter accepté.
    /// Un message Ephemeral avec counter <= last_counter est DROP (replay).
    ephemeral_remote_counters: HashMap<[u8; 32], u64>,

    /// Updates en quarantaine : snapshot_id parent inconnu → (snapshot_id → Vec<Update>)
    /// Borné à 1000 updates (LRU drop au-delà + RequestMissing automatique)
    quarantine: BTreeMap<Uuid, Vec<Update>>,

    /// Dernier `seq` global connu (delta-sync au reconnect)
    last_known_seq: Option<u64>,

    /// Verrou local de compaction (évite double compaction depuis la même instance)
    compaction_lock: bool,

    /// Cache des anciennes Document Keys post-rotation
    /// Borné à 10 entrées (LRU). Clé = snapshot_id de référence.
    /// Éviction : entrée la plus ancienne supprimée au-delà de 10.
    old_keys: lru::LruCache<Uuid, DocumentKey>,
}
```

> **Plus de trait `SyncTransport`.** Le Modèle A abstrayait le transport (MessagePort WASM vs flume natif) derrière un trait. En Modèle B, `flume` est le transport **dans les deux cas** : `ambre-sync` émet directement sur un `flume::Sender<LogicEvent>` nu (`send_to_logic`), sans abstraction.

**Modèle d'exécution (Modèle B, à venir avec le vrai WebSocket) :**

Le thread Sync est un `wasm_thread` qui boucle sur `from_logic.recv()` (bloquant via `Atomics.wait`). C'est valable tant que le WebSocket est un placeholder. **Quand le vrai WS sera câblé**, la boucle bloquante gèlerait l'event-loop JS du Worker (donc les callbacks WS) : il faudra passer à un `select` non-bloquant — `select { cmd = from_logic.recv_async() => …, frame = ws.next() if to_logic_not_full => … }` — où les I/O réseau (fetch HTTP presigned URLs, rotation, WS connect) sont pilotées par `wasm_bindgen_futures::spawn_local` / `JsFuture`. La branche WS est **désactivée quand `to_logic` est plein** (backpressure : Sync arrête de lire la socket → backpressure TCP ; voir `ambre-spec-architecture.md §7 ARCH P1`, « deadlock cyclique »).

### B. Pipeline Cryptographique

Le pipeline est identique pour les trois types de messages (Snapshot, Update, Ephemeral).

**Chiffrement :**

```
Données CRDT (Vec<u8>)
  │
  ├─ Si taille > 256 bytes → ruzstd::encoding::compress_to_vec(data, CompressionLevel::Fastest)
  │                           version = 0x01
  └─ Sinon → version = 0x00 (raw, pas de compression)
  │
  ▼
PlaintextPayload = [version_byte | data]
  │
  │  XChaCha20-Poly1305 (AEAD)
  │  key   = document_key (256 bits)
  │  nonce = getrandom::<[u8; 24]>()
  │  aad   = postcard::to_stdvec(&public_data)  ← lie le ciphertext aux métadonnées
  ▼
ciphertext (tag Poly1305 intégré — authenticité garantie)
  │
  │  HMAC-SHA256(document_key, nonce || ciphertext || public_data_bytes)
  │  → commitment_tag: [u8; 32]  ← key-commitment (XChaCha20-Poly1305 n'est pas key-committing)
  │                                 Champ explicite dans ambre_protocol::{Update|Snapshot|Ephemeral}
  │
  │  Ed25519::sign(signing_key, nonce || ciphertext || public_data_bytes || commitment_tag)
  │  → signature [u8; 64]         ← couvre le commitment_tag pour empêcher sa modification
  ▼
ambre_protocol::{Update|Snapshot|Ephemeral} prêt à envoyer via WsClientMsg
```

> **Note commitment_tag :** Le tag HMAC-SHA256 est distinct du tag Poly1305 de l'AEAD. Il sert de preuve de key-commitment : XChaCha20-Poly1305 n'est pas key-committing (un même ciphertext peut théoriquement être déchiffré par deux clés différentes). Le HMAC(document_key, ...) lie cryptographiquement le message à une clé spécifique, empêchant un serveur compromis de réinjecter un ciphertext dans un autre contexte de document.

> **Note Snapshots :** Les `Snapshot` chiffrés ne transitent pas par WebSocket — ils sont uploadés/téléchargés via S3 presigned URLs (voir `ambre-spec-server.md` §2.B). Le pipeline crypto est identique, seul le transport diffère.

**Déchiffrement (path symétrique) :**

```
Réception WsServerMsg::{Update|Ephemeral|...} (ou blob S3 pour Snapshot)
  │
  │  1. Ed25519::verify(public_data.pub_key, nonce || ciphertext || public_data_bytes || commitment_tag)
  │     → Err(InvalidSignature) → DROP + SyncWarning
  │
  │  2. HMAC-SHA256::verify(document_key, nonce || ciphertext || public_data_bytes) == commitment_tag
  │     → Err → DROP + SyncWarning (possible substitution inter-documents ou mauvaise clé)
  │
  │  3. XChaCha20-Poly1305::decrypt(document_key, nonce, aad=public_data_bytes, ciphertext)
  │     → Err(AuthenticationFailed) → DROP + SyncWarning (corruption ou attaque)
  ▼
PlaintextPayload = [version_byte | data]
  │
  ├─ version == 0x01 → ruzstd::decode_all(data) → Vec<u8>
  ├─ version == 0x00 → data tel quel
  └─ version inconnu → Err(UnknownCompressionVersion) → DROP
  ▼
CRDT bytes → logic_tx.send_to_logic(LogicEvent::RemoteUpdate { .. })
```

**Clé à utiliser lors du déchiffrement :**
- Si `public_data.snapshot_id` correspond à un snapshot chiffré avec une ancienne clé (post-rotation), lookup dans `old_keys` avant d'utiliser `document_key`.
- Si absent de `old_keys` → `Err(UnknownKey)` → quarantaine + `SyncWarning`.

### C. Contrat d'Interface (API)

**Point d'entrée (Modèle B — WASM et natif) :**

```rust
/// Point d'entrée du thread Sync. Spawné par `ambre-app` via wasm_thread (ou
/// appelé directement en test natif). Boucle bloquante tant que le WS est un
/// placeholder ; à rendre non-bloquante au câblage du vrai WebSocket (cf. note
/// « Atomics.wait » plus haut). Sort quand le thread Logic disparaît.
pub fn run(
    logic_rx: flume::Receiver<SyncCommand>,
    logic_tx: flume::Sender<LogicEvent>,
) {
    let mut state = SyncState::new(logic_tx, /* api_base */ String::new());
    while let Ok(cmd) = logic_rx.recv() {
        state.handle_command(cmd);
    }
    // logic_rx déconnecté ⇒ Logic a fermé ⇒ teardown.
}

impl SyncState {
    /// Émet un événement vers le thread Logic (valeur typée, par move).
    fn send_to_logic(&self, event: LogicEvent) { let _ = self.logic_tx.send(event); }

    /// Répond à une requête service en échoant son RequestId.
    fn reply(&self, id: RequestId, evt: RestEvent) {
        self.send_to_logic(LogicEvent::Rest { id, evt });
    }
}
```

**Dispatch des commandes (deux niveaux : enveloppe puis service) :**

Le `SyncCommand` se range en trois groupes (cf. `ambre-bus/src/command.rs`) : l'enveloppe **service** `Rest { id, cmd }` (relais corrélable), le **contrôle/content-send** document/channel (`OpenDocument`, `SendUpdate`, …), et `Shutdown`. La famille service est routée vers `handle_rest`, qui répond via `reply(id, …)` en échoant l'`id`.

```rust
impl SyncState {
    fn handle_command(&mut self, cmd: SyncCommand) {
        match cmd {
            // ── Service plane (requête/réponse corrélable) ────────────────
            SyncCommand::Rest { id, cmd } => self.handle_rest(id, cmd),

            // ── Contrôle / content-send (document) ────────────────────────
            SyncCommand::OpenDocument { document_id } => { /* GET members → ECDH → DocumentKey ;
                crée SyncDocumentState ; GET snapshot/deltas S3 → RemoteSnapshot/RemoteUpdate ;
                WsClientMsg::Subscribe { document_id } */ }
            SyncCommand::CloseDocument { document_id } => { /* Unsubscribe ; zeroize DocumentKey */ }
            SyncCommand::SendUpdate { document_id, payload } => { /* compress → encrypt → WsClientMsg::Update */ }
            SyncCommand::SendSnapshot { document_id, payload } => { /* encrypt → presigned PUT S3 → POST metadata */ }
            SyncCommand::SendEphemeral { document_id, payload } => { /* encrypt → WsClientMsg::Ephemeral */ }
            SyncCommand::Reconnect { document_id } => { /* Subscribe + RequestMissing { after_seq } */ }

            // ── Contrôle (channel) ────────────────────────────────────────
            SyncCommand::OpenChannel { channel_id }  => { /* SubscribeChannel */ }
            SyncCommand::CloseChannel { channel_id } => { /* UnsubscribeChannel */ }
            SyncCommand::SendChannelUpdate { channel_id, payload }    => { /* route vers segment actif */ }
            SyncCommand::SendChannelEphemeral { channel_id, payload } => { /* typing indicator */ }

            // ── Lifecycle ─────────────────────────────────────────────────
            SyncCommand::Shutdown => self.handle_logout(), // cleanup, zeroize, close WS
        }
    }

    /// Famille service relayée. Chaque bras répond via `reply(id, RestEvent::…)`.
    fn handle_rest(&mut self, id: RequestId, cmd: RestCommand) {
        match cmd {
            // ── Session ──────────────────────────────────────────────────
            RestCommand::Unlock { passphrase } => {
                // spawn_local: GET /api/keys/master + GET /api/auth/me
                // Argon2id(passphrase, salt) → User Key → déchiffre master_key_enc → MasterSeed
                // Dérive signing_key + exchange_key ; ouvre le WS multiplexé (GET /api/ws)
                // WS onmessage → handle_ws_message ; onclose → reconnect+resubscribe ; zeroize passphrase
                // reply(id, SessionReady{..}) | AccountSetupRequired (404) | ou LogicEvent::AuthError
            }
            RestCommand::SetupAccount { passphrase } => { /* … → reply(id, AccountCreated { recovery_words }) */ }
            RestCommand::Logout => { self.handle_logout(); self.reply(id, RestEvent::LoggedOut); }

            // ── Documents CRUD ───────────────────────────────────────────
            RestCommand::ListDocuments => { /* GET /api/documents (doc_key_enc joint) → ECDH → reply(id, DocumentList) */ }
            RestCommand::CreateDocument { doc_type, name } => { /* gen DocumentKey → POST → reply(id, DocumentCreated{document_id, doc_type}) */ }
            // … DeleteDocument / RenameDocument

            // ── Partage ──────────────────────────────────────────────────
            RestCommand::InviteToDocument { document_id, invitee_email, role } => { /* search → ECDH → POST /share → reply(id, InviteSent) */ }
            RestCommand::RevokeFromDocument { document_id, user_id } => {
                // séquence atomique : DELETE /share/{uid} → gen DocumentKey → GET members →
                // X25519 par membre → POST /rotate-key (serveur broadcast KeyRotationRequired) →
                // swap key (ancienne → old_keys, zeroize) → reply(id, RevocationComplete)
            }

            // ── Médias ───────────────────────────────────────────────────
            RestCommand::UploadMedia { document_id, data, mime_type } => { /* hash → encrypt → presigned PUT S3 → POST → reply(id, MediaUploaded) */ }
            RestCommand::DownloadMedia { document_id, content_hash } => { /* presigned GET → decrypt → reply(id, MediaDownloaded) */ }

            // Channels (CRUD/partage), Users, Meetings : même schéma (reply(id, …)).
            _ => { /* … */ }
        }
    }
}
```

> Le `RequestId` est **alloué par l'UI**, échoé par Logic, puis échoé par Sync sur la réponse — `ambre-sync` ne fait que le réémettre via `reply`. Les **erreurs** ne sont pas dans `RestEvent` : elles arrivent en `LogicEvent::AuthError`/`ApiError` (peuvent être réponse **ou** spontanées).

Les types `SyncCommand`/`LogicEvent` (+ `RestCommand`/`RestEvent`/`RequestId`) sont définis dans `ambre-bus` et importés ici — voir `ambre-spec-architecture.md §2.D` (plan de données typé, trois plans, et types métier `DocumentSummary`/`MemberInfo`/`UserInfo`/…).

### D. Flux de données

```
thread Logic                        thread Sync                    Blind Relay / S3
─────────────────                  ──────────────────────          ─────────────────
     │                                      │                           │
     │── SyncCommand::Unlock ──────────────▶│                           │
     │                                      │── GET /keys/master ──────▶│
     │                                      │── GET /auth/me ──────────▶│
     │                                      │── Argon2 + déchiffrement  │
     │                                      │── ouvre WS /api/ws ──────▶│
     │◀─ LogicEvent::SessionReady ─────────│                           │
     │                                      │                           │
     │── SyncCommand::ListDocuments ───────▶│── GET /documents ────────▶│
     │                                      │── déchiffre name_enc      │
     │◀─ LogicEvent::DocumentList ─────────│                           │
     │                                      │                           │
     │── SyncCommand::OpenDocument ────────▶│── GET /members ──────────▶│
     │                                      │── déchiffre DocumentKey   │
     │                                      │── GET snapshot S3 ───────▶│
     │                                      │── Subscribe {doc_id} ────▶│
     │◀─ LogicEvent::RemoteSnapshot ───────│                           │
     │◀─ LogicEvent::ConnectionStatus ─────│                           │
     │                                      │                           │
     │── SyncCommand::SendUpdate ──────────▶│── compress → encrypt ────▶│
     │                                      │── WsClientMsg::Update     │
     │                                      │◀─ WsServerMsg::Update ────│
     │◀─ LogicEvent::RemoteUpdate ─────────│                           │
     │                                      │                           │
     │── SyncCommand::SendSnapshot ────────▶│── encrypt → PUT S3 ──────▶│
     │                                      │── POST metadata ─────────▶│
     │                                      │                           │
     │── SyncCommand::InviteToDocument ────▶│── GET /users/search ─────▶│
     │                                      │── chiffre DocumentKey     │
     │                                      │── POST /share ───────────▶│
     │◀─ LogicEvent::InviteSent ───────────│                           │
     │                                      │                           │
     │── SyncCommand::RevokeFromDocument ──▶│── DELETE /share/{uid} ───▶│
     │                                      │── génère nouvelle clé     │
     │                                      │── POST /rotate-key ──────▶│
     │◀─ LogicEvent::RevocationComplete ───│                           │
     │                                      │                           │
     │── SyncCommand::UploadMedia ─────────▶│── chiffre → PUT S3 ──────▶│
     │◀─ LogicEvent::MediaUploaded ────────│                           │
```

### E. Gestion de la Quarantaine

Un update dont le `snapshot_id` parent est inconnu est mis en quarantaine :

```
Réception Update avec snapshot_id inconnu
  │
  ├─ Ajout dans quarantine[snapshot_id].push(update)
  │
  ├─ Si quarantine.total_len() >= 900 (seuil d'alerte)
  │   └─ Envoyer WsClientMsg::RequestMissing { after_seq: last_known_seq }
  │
  ├─ Si quarantine.total_len() >= 1000 (limite dure)
  │   └─ Drop des updates les plus anciens (LRU par clock)
  │   └─ LogicEvent::SyncWarning { code: QUARANTINE_OVERFLOW }
  │
  Réception Snapshot avec snapshot_id correspondant
  │
  └─ Libérer quarantine[snapshot_id] dans l'ordre clock ASC
      └─ Appliquer chaque update via LogicEvent::RemoteUpdate
```

### F. Batching et Compaction

```
Réception SyncCommand::SendUpdate
  │
  ├─ Chiffrement immédiat → WsClientMsg::Update envoyé (temps réel)
  └─ Incrémenter local_clock

Réception SyncCommand::SendSnapshot
  │
  ├─ Vérifier compaction_lock == false
  │   └─ Si true → LogicEvent::SyncWarning { code: COMPACTION_IN_PROGRESS } → skip
  │
  ├─ compaction_lock = true
  ├─ Chiffrement du snapshot complet
  ├─ POST vers Object Storage (via presigned URL depuis le Blind Relay)
  ├─ Signal serveur : purger deltas seq <= current_seq
  └─ compaction_lock = false
```

---

## 3. SÉCURITÉ & CONFIDENTIALITÉ (ZERO-KNOWLEDGE)

### Audit des Frontières

| Donnée | Sort du client ? | Forme sur le wire | Garantie |
|---|---|---|---|
| Contenu CRDT (texte, données) | Oui | XChaCha20-Poly1305 ciphertext | Blind Relay voit `ciphertext` + `nonce`, jamais le plaintext |
| Document Key | Non | Reste dans `SyncState` — ZeroizeOnDrop | Jamais dans un message réseau |
| Signing Key (privée) | Non | Idem | Seule `pub_key` dans `public_data` |
| Version de compression | Non | Encodée dans le ciphertext AEAD | Opaque pour le serveur |
| `document_id`, `pub_key`, `clock`, `seq` | Oui | En clair dans `public_data` | Métadonnées minimales pour le routage — ne révèlent pas le contenu |
| Curseurs / présence | Oui | Chiffrés dans payload `Ephemeral` | Le serveur ne voit jamais curseur ni nom en clair |

**Fuite de métadonnées acceptée (MVP) :** le Blind Relay infère "un client ayant la clé publique X a modifié le document Y à l'instant T". Cette information est jugée acceptable pour le MVP.

### Prévention des Attaques

- **Replay (Updates)** : le `clock` par client est vérifié séquentiellement par le serveur (script Lua Redis). Un update avec `clock <= dernier_clock_connu` est rejeté côté serveur.
- **Replay (Ephemeral)** : le `counter` est monotone par **(document_id, session)**. Un récepteur client rejette tout message Ephemeral avec `counter <= dernier_counter_connu` pour ce `(pub_key, document_id)`.
- **Substitution inter-documents** : le `commitment_tag` HMAC(document_key, nonce || ciphertext || public_data) lie le ciphertext à son `document_id`. Un message ne peut pas être réinjecté dans un autre document.
- **Réordonnancement des snapshots** : le `parent_snapshot_proof` (chaîne BLAKE2b) garantit l'intégrité de la chaîne sans déchiffrement.
- **Double Compaction** : verrou Redis `SET NX TTL 30s` côté serveur + `compaction_lock` côté client.
- **Présence en clair** : les payloads Ephemeral (curseur, sélection, nom) sont **chiffrés** avec la Document Key avant envoi. Le serveur ne voit que `document_id`, `pub_key`, `counter`.

### Rotation de Document Key — Flow Complet

Deux déclencheurs : (A) le client initiateur révoque un membre via `SyncCommand::RevokeFromDocument`, (B) les autres clients reçoivent `WsServerMsg::KeyRotationRequired` via WebSocket.

La rotation est **entièrement orchestrée par le thread Sync** — le thread Logic ne fait ni crypto ni réseau. La DocumentKey ne quitte jamais le thread Sync.

```
[Client initiateur — thread Logic]
1. L'utilisateur révoque un membre via l'UI
   thread Logic émet SyncCommand::RevokeFromDocument { document_id, user_id }

[Client initiateur — thread Sync (async via spawn_local)]
2. DELETE /api/documents/{id}/share/{uid}
3. Génère nouvelle DocumentKey via CSPRNG
4. GET /api/documents/{id}/members → liste des membres restants + pubkeys
5. Pour chaque membre restant :
   X25519(own_exchange_key, member.exchange_pubkey) → chiffre nouvelle clé
6. POST /api/documents/{id}/rotate-key avec encrypted_keys_per_member
   → serveur remplace atomiquement tous les doc_key_enc (transaction Postgres)
   → serveur broadcast WsServerMsg::KeyRotationRequired à CE moment
     (les nouvelles clés sont en base — pas de race condition)
7. Achève les messages en cours de chiffrement avec l'ancienne clé
8. Stocke ancienne clé dans old_keys (LRU, max 10 entrées)
9. Remplace document_key par new_key, zeroize() l'ancienne
10. Envoie LogicEvent::RevocationComplete { document_id } au thread Logic

[Client initiateur — thread Logic]
11. Déclenche SyncCommand::SendSnapshot { document_id, payload }
    → snapshot chiffré avec la nouvelle clé → scelle l'état post-révocation

[Autres clients — thread Sync (réception WsServerMsg::KeyRotationRequired)]
12. GET /api/documents/{id}/members → récupère nouveau doc_key_enc
13. X25519 ECDH → déchiffre nouvelle DocumentKey
14. Swap document_key, ancienne → old_keys, zeroize
15. Envoie LogicEvent::MemberRevoked { document_id, revoked_user_id } au thread Logic
    (le thread Logic est informé du départ, peut mettre à jour l'UI)
```

**Sécurité :** La DocumentKey ne traverse jamais le canal `flume` Logic ↔ Sync (elle reste confinée au thread Sync). Le broadcast `WsServerMsg::KeyRotationRequired` est émis par le serveur **après** que les nouvelles clés sont en base (étape 6) — les autres clients ne fetchent jamais une `doc_key_enc` périmée.

**Cache `old_keys` :** borné à 10 entrées LRU par document. Au-delà, les snapshots antérieurs à la 11e rotation ne sont plus déchiffrables localement → force un re-sync depuis S3.

---

## 4. QUALITÉ & TESTABILITÉ

### Tests Unitaires

```rust
// Round-trip encrypt/decrypt pour les trois types de messages
#[test]
fn roundtrip_update() { /* encrypt → decrypt, assert_eq payload */ }

// Détection de tampering
#[test]
fn tampered_ciphertext_fails() {
    let mut enc = encrypt_update(&data, &key, &nonce, &public_data);
    enc.ciphertext[0] ^= 0xFF;
    assert!(decrypt_update(&enc, &key).is_err());
}

// HMAC commitment : substitution inter-documents détectée
#[test]
fn commitment_tag_wrong_document_rejected() { /* modifier document_id → HMAC invalide */ }

// Signature invalide
#[test]
fn invalid_signature_rejected() { /* wrong pub_key → Err(InvalidSignature) */ }

// Byte de version
#[test]
fn version_byte_routing() {
    // 0x00 → raw, 0x01 → decompress, 0xFF → Err(UnknownCompressionVersion)
}

// Quarantaine : libération ordonnée
#[test]
fn quarantine_released_in_clock_order() { /* update avant snapshot → quarantaine → snapshot → libération */ }

// Quarantaine : RequestMissing déclenché à 900
#[test]
fn quarantine_triggers_request_missing_at_threshold() { }

// Anti-replay Ephemeral
#[test]
fn ephemeral_counter_replay_rejected() { /* counter <= dernier connu → DROP */ }
```

### Prop-Testing

```rust
proptest! {
    /// Propriété : encrypt(decrypt(m)) == m pour tout payload et toute clé.
    #[test]
    fn roundtrip_encrypt_decrypt(
        data in any::<Vec<u8>>(),
        key  in any::<[u8; 32]>(),
    ) {
        let encrypted = encrypt_update(&data, &key, /* ... */).unwrap();
        let decrypted = decrypt_update(&encrypted, &key).unwrap();
        prop_assert_eq!(data, decrypted);
    }

    /// Propriété : les clocks sont strictement croissants.
    #[test]
    fn clock_monotonically_increases(n_updates in 1usize..1000) {
        let mut state = SyncState::new_for_test();
        let clocks: Vec<u64> = (0..n_updates)
            .map(|_| state.next_clock())
            .collect();
        // windows(2) retourne &[u64] — pas de tuple
        prop_assert!(clocks.windows(2).all(|w| w[0] < w[1]));
    }

    /// Propriété : la compression puis décompression est une identité.
    #[test]
    fn ruzstd_roundtrip(data in any::<Vec<u8>>()) {
        use ruzstd::encoding::{compress_to_vec, CompressionLevel};
        use ruzstd::decoding::StreamingDecoder;
        let compressed = compress_to_vec(&data, CompressionLevel::Fastest);
        let mut decoder = StreamingDecoder::new(compressed.as_slice()).unwrap();
        let mut decompressed = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut decompressed).unwrap();
        prop_assert_eq!(data, decompressed);
    }
}
```

### Mocks & Environnement

```rust
/// Transport WebSocket en mémoire — channel flume bidirectionnel.
/// Permet de tester le pipeline complet sans WASM ni réseau.
pub struct MockWsTransport {
    pub client_tx: flume::Sender<WsClientMsg>,
    pub server_rx: flume::Receiver<WsClientMsg>,
    pub server_tx: flume::Sender<WsServerMsg>,
    pub client_rx: flume::Receiver<WsServerMsg>,
}

// Le canal Logic ↔ Sync EST déjà un `flume` en Modèle B : pas de mock de transport
// nécessaire. Un test construit `SyncState` avec une paire (Sender, Receiver) flume
// et draine le `Receiver<LogicEvent>` pour observer les réponses (même approche que
// les 9 tests hôte de `ambre-logic`, cf. ambre-spec-architecture.md §4).

/// Blind Relay in-process pour tests d'intégration multi-clients.
/// Simule : createSnapshot, createUpdate, getDocument, hasAccess, hasBroadcastAccess.
pub struct MockBlindRelay { /* ... */ }
```

Tous les tests compilent en mode natif (`#[cfg(not(target_arch = "wasm32"))]`) avec `tokio::test`. Les tests WASM sont réservés aux interactions `gloo-net` réelles et exécutés avec `wasm-bindgen-test`.

### Benchmarks (criterion)

| Benchmark | Cible (P95 mobile) | Ceiling (jamais dépasser) | Note |
|---|---|---|---|
| Chiffrement outbound 1 KB (compress+AEAD+HMAC+Ed25519 sign) | < 200 µs | < 500 µs | Goulot : Ed25519 sign (~80–150 µs) |
| Déchiffrement inbound 1 KB (Ed25519 verify+HMAC+AEAD+decompress) | < 350 µs | < 1 ms | Goulot : Ed25519 verify (~150–250 µs) |
| Round-trip complet crypto (hors réseau) | < 600 µs | < 1.5 ms | |
| Compression zstd snapshot 100 KB (`ruzstd`) | < 5 ms | — | |
| Libération quarantaine 100 updates | < 1 ms | — | |

> **Alignement architecture :** ces cibles sont identiques à `ambre-spec-architecture.md` §5. Les cibles P95 sont calibrées sur les benchmarks RustCrypto WASM (sans SIMD) sur SoC mobile milieu de gamme 2024. Le benchmark `ambre-crypto` (§7 SYNC P0) valide ces cibles sur device mobile réel.

---

## 5. PERFORMANCE & RESSOURCES

**Budget performance :**

- Le thread Sync est un `wasm_thread` dédié (Web Worker partageant la mémoire linéaire) : il s'exécute **en parallèle réel** du thread Logic et du thread UI, sur son propre cœur si disponible.
- Les opérations crypto sont bloquantes dans le thread Sync mais n'affectent ni le thread Logic ni le thread UI (threads distincts, event loops distinctes). Le transport `flume` déplace les payloads **par move** dans le heap partagé — pas de recopie de frontière.
- Latence cible pour un update de 1 KB : chiffrement outbound < 200 µs (P95 mobile), ceiling < 500 µs.

**Taille binaire (contribution d'`ambre-sync` au binaire total) :**

| Composant | Raw estimé | Notes |
|---|---|---|
| Logique `ambre-sync` | ~50 KB | Hors dépendances |
| RustCrypto (chacha20 + ed25519 + blake2 + hmac + argon2) | ~140 KB | Partagé avec `ambre-crypto` (Argon2 ajouté pour login) |
| `ruzstd` (compression + décompression) | ~80 KB | |
| `gloo-net` (websocket + http) | ~40 KB | |
| `postcard` | ~15 KB | Partagé |
| **Total contribution** | **~325 KB raw** | Dépendances partagées déduites dans le binaire final |

**Mémoire (`SyncState` multi-document) :**

- Base (session) : < 1 MB (MasterSeed, SigningKey, ExchangeKey + overhead HashMap)
- Par document ouvert (`SyncDocumentState`) : < 1 MB (DocumentKey, compteurs, état sync, quarantine)
- `quarantine` par document : borné à 1000 updates × ~2 KB moyenne = ~2 MB max
- `old_keys` par document : borné à 10 × `DocumentKey` (32 bytes + overhead) = négligeable
- Scénario typique (1 doc hôte + 3 embarqués) : ~4–8 MB overhead Sync total

---

## 6. DÉCISIONS ARCHITECTURALES ÉCARTÉES

| Alternative | Rejetée car |
|---|---|
| **libsodium / sodiumoxide** | Dépendance C, incompatible `wasm32-unknown-unknown` sans shim. RustCrypto est équivalent, audité, pure Rust. |
| **TLS comme unique couche de sécurité** | TLS protège le transport mais le serveur verrait le plaintext. E2EE applicatif est non-négociable. TLS est complémentaire. |
| **`zstd` crate (bindings C) pour la compression** | Dépendance C non compatible WASM. `ruzstd` fournit compression ET décompression en pure Rust depuis la v0.8. |
| **Dual feature flag `ruzstd` (décompression) + `zstd` (compression natif)** | Complexité inutile. `ruzstd` couvre les deux sens avec des performances acceptables sur WASM mobile. |
| **Signal Protocol (Double Ratchet)** | Conçu pour la messagerie 1-to-1 avec forward secrecy par message. Notre modèle est documentaire (N collaborateurs, Document Key partagée). MLS V2. |
| **WebRTC DataChannel (P2P)** | Requiert STUN/TURN, complexifie NAT traversal, incompatible avec le modèle Blind Relay. Gain de latence marginal pour un usage documentaire. |
| **JSON sur le wire interne** | `postcard` est ~10x plus compact, parsing plus rapide, déjà dans le projet. |
| **Quarantaine non bornée** | Un snapshot manquant indéfiniment ferait grossir la quarantaine sans limite. LRU + RequestMissing automatique à 900 updates est le bon équilibre. |
| **Cache `old_keys` non borné** | En cas de rotations répétées rapides (attaque ou bug), le cache grossirait. LRU à 10 entrées est suffisant pour les cas réels (< 5 rotations simultanément actives). |
| **Types wire redéfinis dans `ambre-sync`** | `ambre-protocol` est la source de vérité. Redéfinir localement (`SecSyncUpdate`, etc.) crée une divergence garantie (ex: `[u8; 32]` vs `Vec<u8>` pour `pub_key`). |
| **Yrs à la place de Loro** | Hors scope. Les snapshots Yrs et Loro ont des ratios de compression comparables (3:1 à 8:1 sur du texte). Le choix de Loro n'impose aucune contrainte sur `ambre-sync`. |
| **Modèle A — shared-nothing (`postMessage` + `MessagePort` + postcard pour Logic ↔ Sync)** | ⚠️ **Inversion v1.** Le Modèle A imposait `postcard` + 2 memcpy WASM↔JS par hop et un trait `SyncTransport`. Remplacé par **canaux `flume`** transportant des valeurs Rust typées par move (mémoire partagée). Ne pas y revenir : on réintroduirait la sérialisation et l'abstraction de transport pour rien. Le coût accepté du Modèle B (toolchain nightly `-Z build-std`, headers COOP/COEP de `SharedArrayBuffer`) est documenté dans `ambre-spec-architecture.md §6`. |
| **`wasm_thread::spawn` + `SharedArrayBuffer` — RETENU (était rejeté en v1)** | Désormais **adopté** : le thread Sync est un `wasm_thread` partageant la mémoire linéaire. Requiert `+atomics,+bulk-memory,+mutable-globals` + `-Z build-std` + nightly + headers COOP/COEP — accepté pour bénéficier du transport `flume` typé sans copie. |
| **Un thread Sync par document ouvert** | Impliquerait N threads supplémentaires pour une Toile avec documents embarqués — chacun avec sa copie de RustCrypto. Inacceptable sur mobile. Un seul thread Sync multi-document (`HashMap<Uuid, SyncDocumentState>`) avec un seul WebSocket multiplexé (Subscribe/Unsubscribe par document). |
| **Boucle `flume` bloquante avec un vrai WebSocket** | `Atomics.wait` (sous `flume::recv`) gèle l'event-loop JS du Worker, donc les callbacks WS. À éviter dès le vrai WS : boucle non-bloquante `select { recv_async … , ws.next() … }`. Détail en `ambre-spec-architecture.md §7 ARCH P1`. |
| **Un WebSocket par document ouvert** | Gaspillage de connexions (5 WS pour une Toile avec 4 documents embarqués). Limites navigateur (~6 connexions par origine), keepalives coûteux en batterie mobile. Les messages portent déjà `document_id` dans `publicData` — le serveur démultiplexe. Un seul WS multiplexé par session avec Subscribe/Unsubscribe. |
| **Snapshot via WebSocket** | Les snapshots peuvent faire plusieurs MB — trop gros pour un message WebSocket. Upload/download via S3 presigned URLs. `WsServerMsg::SnapshotAvailable` notifie les clients. |

---

## 7. PLAN D'INTÉGRATION (ROADMAP)

> Backlog détaillé de cette spec ; l'ordonnancement transversal (milestones) vit dans `specs/ambre-roadmap.md`.

### SYNC P0 — Core Crypto (standalone)

- [x] Pipeline encrypt/decrypt pour les 3 types de messages (avec `commitment_tag` HMAC), en isolation complète. *`ambre-crypto` : enveloppe `seal`/`open` generique sur `aad` (les 3 types ne different que par leur `PublicData` serialise) ; primitives AEAD / HMAC-SHA256 / Ed25519 / X25519 / Argon2id / BLAKE2b.*
- [x] Tests unitaires exhaustifs + prop-tests (round-trip, tampering, commitment tag). *Le `version byte` reste a la couche Sync (compression), hors `ambre-crypto`.*
- [x] Benchmarks criterion de reference. *Desktop : seal 1 KB ~34 us, open 1 KB ~48 us (cibles P95 200/350 us). Mesure mobile a faire au cablage Sync.*
- [x] Validation `ruzstd` compress/decompress roundtrip *(couche Sync : `ambre-sync::pipeline`, version-byte 0x00/0x01, seuil 256 o, tests host)*
- [x] Implémentation `MasterSeed` → `derive_signing_key` / `derive_exchange_key` + tests *(derivation BLAKE2b domain-separated, deterministe)*
- [x] Pas de WebSocket, pas de thread Logic *(crate standalone, zero dep reseau/IPC)*

### SYNC P1 — Intégration thread Logic

- [x] Point d'entrée `ambre_sync::run(from_logic, to_logic)` (canaux `flume`), boucle `recv()` bloquante, squelette `SyncState` + dispatch `handle_command`/`handle_rest` (placeholders).
- [x] **Pipeline crypto cablé** (`ambre-sync::pipeline`) : compression version-byte + `ambre-crypto::envelope`, assemblage/desassemblage `Update`/`Snapshot`/`Ephemeral`, chiffrement sortant (`SendUpdate`/`SendEphemeral` → `WsClientMsg`, Snapshot scelle/S3 TODO) et dechiffrement entrant (`handle_ws_message` → `RemoteUpdate`/`RemoteEphemeral`, echec → `SyncWarning`). Cles de session derivees du `MasterSeed` (seed local placeholder en attendant Unlock reseau). Tests host. *Reseau/WebSocket reel hors scope (item suivant).*
- [ ] **Boucle non-bloquante** (`recv_async` + WS `JsFuture`) au câblage du vrai WebSocket — prérequis `Atomics.wait` + correctif deadlock cyclique (cf. `ambre-spec-architecture.md §7 ARCH P1`).
- [x] Implémentation `RestCommand::Unlock` (fetch GET /api/keys/master + Argon2 → MasterSeed → dérivation keypairs)
- [x] Implémentation `RestCommand::SetupAccount` (génération MasterSeed, upload clés)
- [x] Implémentation `SyncCommand::OpenDocument` / `CloseDocument` (multi-document `HashMap<Uuid, SyncDocumentState>`)
- [x] Implémentation CRUD documents (`RestCommand`) : `ListDocuments`, `CreateDocument`, `RenameDocument`, `DeleteDocument`
- [x] `MockWsTransport` pour tests end-to-end sans réseau
- [x] Quarantaine : implémentation + tests de libération ordonnée + RequestMissing automatique *(release par `clock` ASC sur `SnapshotAvailable`, bornage 900/1000 + `SyncWarning` overflow, 4 tests host)*

### SYNC P2 — Blind Relay & Réseau

- [ ] Connexion WebSocket réelle (`gloo-net` WASM, `tokio-tungstenite` natif)
- [ ] HTTP REST via `gloo-net::http` (CRUD, partage, médias, users search)
- [ ] Upload/download Snapshot via presigned URLs S3 (pas par WebSocket)
- [ ] Delta-sync au reconnect (`last_known_seq` → `RequestMissing`)
- [ ] Batching + compaction + verrou Redis (`compaction_lock`)
- [ ] Handling `WsServerMsg::SnapshotRequired` → `LogicEvent::SnapshotNeeded`
- [ ] `MockBlindRelay` in-process pour tests multi-clients

### SYNC P3 — Partage & Révocation

- [ ] Flow `InviteToDocument` complet (search user → fetch pubkeys → chiffre DocumentKey → POST /share)
- [ ] Flow `RevokeFromDocument` complet (DELETE /share → rotation clé → POST /rotate-key → 15 étapes, voir §3)
- [ ] Cache `old_keys` LRU 10 entrées par document
- [ ] Anti-replay Ephemeral (`ephemeral_remote_counters`)
- [ ] Flow médias : `UploadMedia` (chiffrement + presigned URL + PUT S3 + confirmation POST), `DownloadMedia` (GET S3 + déchiffrement)

### SYNC P4 — Channels, Meetings & Optimisation

- [ ] Channels Murmure : `OpenChannel`, `SendChannelUpdate`, `SendChannelEphemeral`, gestion segments interne (création, scellement, routage — le thread Logic ne voit pas les segment_id)
- [ ] Meetings : signaling WebRTC (`SignalingClientMsg` / `SignalingServerMsg`)
- [ ] Tuning seuil de compression (256 bytes par défaut — ajuster si benchmarks le justifient)
- [ ] Backpressure WebSocket : buffer bounded des messages en attente
- [ ] Audit de sécurité : revue des frontières ZK, vérification `zeroize` systématique sur tous les types sensibles

### SYNC Post-MVP — Optimisations

- [ ] Cache `doc_key_cache: HashMap<Uuid, DocumentKey>` dans `SyncState` — évite N × X25519 ECDH à chaque `ListDocuments` (peuplé par `OpenDocument` et `ListDocuments`, zeroize au logout)
