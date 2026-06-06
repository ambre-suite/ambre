# AMBRE — SPEC: ARCHITECTURE GÉNÉRALE
**Type :** Horizontal (Foundation)
**Statut :** Draft

> Architecture technique d'AMBRE : modèle **3 threads à mémoire partagée** (un seul binaire WASM, `wasm_thread` + `SharedArrayBuffer`), découpage en crates et frontières de dépendance, transport IPC inter-thread **typé** (canaux `flume`, valeurs Rust de bout en bout), plan de données à trois plans (contenu / service / réseau), sécurité zero-knowledge, budgets de performance, et roadmap d'intégration.

---

## 1. OBJECTIFS & CONTRAINTES

* **Périmètre :** Cette spec définit l'architecture technique client-serveur d'Ambre : structure du workspace Rust, responsabilités de chaque crate, flux de données entre les trois threads, transport IPC, et règles de dépendance. Elle ne décrit pas l'implémentation interne des composants (ces détails sont dans les specs spécifiques comme `ambre-spec-sync.md`).

> **Note de modèle (lire en premier).** La première version de cette spec décrivait le **Modèle A** : 3 binaires WASM séparés (`shared-nothing`), communication par `postMessage` + transfert d'`ArrayBuffer` + sérialisation `postcard`. **Le code réel est en Modèle B** : **un seul binaire WASM**, trois threads `wasm_thread` partageant une **mémoire linéaire** (`SharedArrayBuffer`), communiquant par des **canaux `flume`** qui transportent des **valeurs Rust typées** de bout en bout. Toutes les justifications historiques du type « telle dépendance lourde ne doit pas polluer tel binaire » sont caduques — il n'y a qu'un binaire. **Les frontières sont désormais garanties par le graphe de dépendances des crates** (vérifié par le compilateur), pas par la séparation des binaires. Le Modèle A et ses corollaires (`postMessage`, Web Workers JS séparés) sont consignés en **§6** pour ne pas être re-proposés.

* **Dépendances (règles globales) :**

| Règle | Justification |
|-------|---------------|
| `ambre-server` dépend UNIQUEMENT de `ambre-core` + `ambre-protocol` + `ed25519-dalek` (direct) | Le Blind Relay ne doit pas connaître la structure des documents, ni les types crypto applicatifs (`DocumentKey`, `MasterSeed`). La seule opération crypto du serveur est la vérification de signature Ed25519 sur les updates — `ed25519-dalek` est tiré directement sans passer par `ambre-crypto`. |
| `ambre-bus` dépend de `ambre-core` **uniquement** | Les 4 enveloppes IPC voyagent en **valeurs Rust sur `flume`** — rien n'est sérialisé. Plus aucune dépendance `serde`/`postcard`/`thiserror`/`flume`/`tracing`. Les payloads applicatifs sont **typés** (`AppCommand`/`AppEvent`) ; seuls les vrais octets réseau (deltas CRDT Loro, messages de canaux) restent `Vec<u8>`. Cette feuille en amont garantit qu'`ambre-bus` reste sans dépendances crypto/réseau. |
| `ambre-draw` dépend de `kurbo`, `peniko`, `parley` uniquement (pas de `wgpu`, pas de `krilla`) | Trait `DrawSurface` et ré-exports de types géométriques/typographiques. Dépendances de types légers (maths, couleurs, text layout) — aucune dépendance GPU. Les implémentations concrètes (`WgpuSurface` côté UI, `KrillaSurface` côté Logic) vivent dans les crates consommatrices. |
| `ambre-commands` ne dépend de **rien côté UI** (feuille ; `ambre-core` pour ids/`Locale` au besoin) | Registre de commandes partagé : `CommandId`, `Command`, `CommandState`, `CommandSource`, keybindings. Dépendue par **chaque crate d'app** (qui définit ses commandes) ET par `ambre-chrome` (qui les rend). Distinct du `AppCommand`/`LogicCommand` IPC d'`ambre-bus` : une `Command` invoquée *produit* un `LogicCommand`. |
| `ambre-canvas` dépend de `ambre-draw` (+ wgpu/swash + accesskit ; + web-sys/js-sys/wasm-bindgen(-futures)/tracing en deps **target `cfg(wasm32)`** pour `host`) | **Runtime de rendu canvas** du thread UI : `WgpuSurface`/`QuadRenderer`/`GlyphAtlas`, `TextCache`, `FrameHost`/`DocView`, `theme.rs` généré. **NE dépend PAS de `leptos`.** `accesskit` est scopé à l'a11y du **contenu** canvas. **Aucune dépendance applicative** (couplage `DocView` + injection par closure). Nommé `ambre-canvas` (medium, symétrique de `ambre-chrome`) ; spec `ambre-spec-canvas.md`. |
| `ambre-chrome` dépend de `leptos` + `ambre-commands` | **Toolkit chrome Leptos PARTAGÉ et app-agnostique** : `AppShell`, `CommandPalette`, primitives `Dialog`/`Menu`/`Toolbar`/`Toast`/`ContextMenu`/`Sidebar`/`StatusBar`, keybinding manager. Rend des vues sur le registre — **ne nomme aucune app** (même inversion que `DocView`). Le chrome **spécifique** à une app vit dans la crate de l'app, **pas ici**. |
| Chaque crate applicative (`ambre-trame`, …) dépend de `ambre-bus` + `ambre-draw` + `ambre-canvas` + `ambre-chrome` + `ambre-commands` + `leptos` (PAS de `ambre-sync` ni `ambre-logic`) | Une crate par app, **monolithe assumé**, **sans features** : `logic` (`*State` + handlers, thread Logic) + `ui` (`*View` impl `DocView`, rendu contenu sur `ambre-canvas`) + `chrome` (composants Leptos **spécifiques** bâtis sur `ambre-chrome`) + définition des `Command` (`CommandSource`). Types IPC (`TrameCommand`/`TrameEvent`/`DisplayCell`) ré-exportés depuis `ambre-bus`. |
| `ambre-logic` dépend de `ambre-bus` + la/les crate(s) app + `flume` + `tracing` | Runtime du thread Logic. **100 % Rust pur, zéro glue wasm** : les types fondamentaux arrivent par ré-export d'`ambre-bus` (pas de dép directe sur `ambre-core`). Le hook panic + le subscriber tracing sont posés dans `ambre-app`. |
| `ambre-sync` dépend de `ambre-bus` + `ambre-protocol` + `flume` + `tracing` + `wasm-bindgen` + `web-sys` (+ placeholders `zeroize`/`gloo-net`/`ruzstd`, balisés) | Runtime du thread Sync. L'encodage wire vient d'`ambre-protocol` ; le crypto viendra d'`ambre-crypto`. |
| `ambre-app` dépend de `ambre-bus` + `ambre-canvas` + `ambre-chrome` + `ambre-trame` + `ambre-logic` + `ambre-sync` + `flume` + `wasm-bindgen` + `wasm_thread` | LAUNCHER cdylib. **Monte l'`AppShell` Leptos** (`ambre-chrome`), crée les 4 canaux, spawn `ambre_logic::run` + `ambre_sync::run`, bootstrappe le canvas et injecte la vue contenu de l'app active. NE dépend pas de wgpu/postcard/ambre-core en direct. |

* **Invariants :**
- **Aucun cycle de dépendance** : le graphe est acyclique et vérifiable par `cargo tree`. C'est lui qui porte les frontières d'architecture (et non plus la séparation des binaires).
- **Le thread Logic ne touche ni GPU ni DOM** : `ambre-logic` n'a **aucune dépendance directe** sur `ambre-canvas`/`ambre-chrome`, et son **chemin runtime** n'appelle jamais wgpu/leptos. (Transitivement, `ambre-logic → crate d'app → ambre-canvas/ambre-chrome` rend wgpu/leptos *atteignables* dans le graphe — sans conséquence : un seul binaire, la frontière est portée par les **deps directes + le runtime**, pas par le graphe transitif.) Le thread Sync ne compile ni UI ni CRDT en deps directes.
- **Règle d'or — Séparation stricte des préoccupations entre threads :**
  - **UI Thread** (thread principal de la page) : rendu (wgpu + QuadRenderer + GlyphAtlas + Parley/Swash), capture d'input, accessibilité (AccessKit). **Zéro appel API**, **zéro WebSocket**, **zéro crypto**, **zéro compression**. Les données lui arrivent déjà prêtes à l'affichage (valeurs `AppEvent` typées). Exception unique : le chargement de **fonts** (assets statiques, même origine). **Invariant nouveau : le thread UI ne doit JAMAIS bloquer** (il tourne sur le main thread de la page).
  - **Logic Thread** : logique métier (CRDT Loro, pagination, moteur formules Trame, dispatch typé). **Zéro réseau**, **zéro crypto**, **zéro compression**. Aucun type `DocumentKey`, `ciphertext`, `WsClientMsg`, `WebSocket` dans les crates applicatives. Les `SyncCommand` ne portent que des `document_id` et des payloads CRDT bruts (`Vec<u8>`).
  - **Sync Thread** : **tout** le réseau (HTTP, WebSocket, presigned URLs S3), **toute** la crypto (`ambre-crypto`), **toute** la compression (`ruzstd`). Seul consommateur d'`ambre-crypto` et d'`ambre-protocol` côté client.
- **`ambre-protocol` est la source de vérité unique** pour tous les types du protocole réseau. Ni `ambre-sync` ni `ambre-server` ne les redéfinissent.
- **Le serveur ne voit jamais le plaintext** : tous les types transitant par `ambre-protocol` sont des blocs opaques (ciphertext + metadata publiques).
- **postcard reste sur le seul chemin réseau** (`ambre-protocol`, wire WebSocket). **Le chemin inter-thread ne porte aucun postcard** : les enveloppes `flume` voyagent en valeurs Rust typées. JSON uniquement pour l'API REST.
- **Toolchain :** nightly pinné + `-Z build-std` (rebuild de `std` avec `+atomics`), `target-feature=+atomics,+bulk-memory,+mutable-globals`, link-args `--shared-memory/--max-memory/--import-memory` + exports TLS. Voir `rust-toolchain.toml` + `.cargo/config.toml`.

---

## 2. CONCEPTION TECHNIQUE & ÉTAT (STATE)

### A. Structure du Workspace

```
ambre/
├── Cargo.toml                    (workspace racine — members ci-dessous)
├── rust-toolchain.toml           (nightly pinné, rust-src, wasm32 target)
├── .cargo/config.toml            (rustflags wasm32 : +atomics, --shared-memory, …, build-std)
├── crates/
│   ├── ambre-core/               Types fondamentaux (Uuid/DocumentId/UserId/…, DocType, PublicKey, Role). Partagé client + serveur.
│   ├── ambre-crypto/             Primitives crypto. ⚠️ STUB (lib.rs ~6 lignes, aucun consommateur réel).
│   ├── ambre-protocol/           Types réseau (Update/Snapshot/Ephemeral, Ws*Msg, Signaling*, encode/decode). SOURCE DE VÉRITÉ (sync + serveur).
│   ├── ambre-bus/                Contrat IPC client. Dépend de ambre-core UNIQUEMENT. Aucun serde/postcard/flume/tracing.
│   │     ├── lib.rs              (ré-exports + IPC_TEST_DOC_ID)
│   │     ├── logic.rs            (LogicCommand + AppCommand + TrameCommand)
│   │     ├── ui.rs               (UiEvent + AppEvent + TrameEvent + DisplayCell)
│   │     ├── command.rs          (SyncCommand)
│   │     ├── event.rs            (LogicEvent)
│   │     └── rest.rs             (RestCommand + RestEvent + RequestId + types métier partagés)
│   ├── ambre-draw/               Trait DrawSurface + ré-exports kurbo/peniko/parley. Aucune dép GPU.
│   ├── ambre-commands/           Registre de commandes partagé (CommandId/Command/CommandState/CommandSource/keybindings). Feuille, zéro dép UI.
│   ├── ambre-canvas/                 Runtime de rendu canvas du thread UI (PAS de leptos) :
│   │     ├── render/             (QuadRenderer, GlyphAtlas, WgpuSurface, modèle Layer 2-couches)
│   │     ├── text_cache.rs       (TextCache Parley)
│   │     ├── view.rs             (DocView trait + RepaintPlan + FrameStats + InputEvent)        [GPU-agnostique]
│   │     ├── host.rs             (FrameHost<V> + GpuState + bootstrap_gpu + start_raf_loop + run) [cfg(wasm32)]
│   │     └── theme.rs            (constantes tokens côté canvas, générées depuis tokens.ron ; accesskit scopé contenu)
│   ├── ambre-chrome/             Toolkit chrome Leptos PARTAGÉ, app-agnostique : AppShell, CommandPalette,
│   │                             Dialog/Menu/Toolbar/Toast/ContextMenu/Sidebar/StatusBar, keybindings.
│   │                             Dépend de leptos + ambre-commands. Le chrome SPÉCIFIQUE vit dans les crates d'app.
│   ├── ambre-trame/              TOUTE l'app Trame (monolithe, pas de features) :
│   │     ├── logic.rs            (TrameState + handlers, thread Logic)
│   │     ├── ui/{grid,renderer,view}  (TrameView impl DocView, rendu contenu sur ambre-canvas)
│   │     └── chrome/             (composants Leptos SPÉCIFIQUES sur ambre-chrome ; Command via ambre-commands)
│   ├── ambre-prose / ambre-toile / ambre-murmure   Stubs (doc only, pas de logique).
│   ├── ambre-logic/              Runtime thread Logic : run(from_ui, to_ui, to_sync, from_sync),
│   │                             LogicState { docs: HashMap<Uuid, DocState>, … }, dispatch par match. 100 % Rust pur.
│   ├── ambre-sync/               Runtime thread Sync : run(from_logic, to_logic), pipeline WS/crypto (placeholder).
│   ├── ambre-app/                LAUNCHER cdylib : #[wasm_bindgen(start)], monte l'AppShell Leptos (ambre-chrome),
│   │                             crée les 4 canaux, spawn logic + sync, bootstrappe le canvas + injecte la vue active.
│   └── ambre-server/             Blind Relay natif (Axum). HORS workspace members (cible native, pas wasm32).
└── specs/
```

> **`ambre-server` hors `members` du workspace racine** : choix volontaire. Le serveur est une cible **native** (Axum/Tokio) ; le workspace racine cible `wasm32` avec des rustflags incompatibles (`+atomics`, `--shared-memory`). Le sortir des members évite que `cargo build` du workspace tente de le compiler en wasm.

> **Build workflow :** un seul build cdylib (`ambre-app`). Les crates `*-worker` du Modèle A (`ambre-logic-worker`, `ambre-sync-worker`) **n'existent plus** ; plus de `make workers` ni d'étape wasm-pack séparée. Trunk (ou équivalent) build `ambre-app` et copie les assets (`fonts/`).

### B. Frontières, Flux de Données et Modèle 3 threads (mémoire partagée)

L'architecture client est organisée en **3 threads `wasm_thread`** au sein d'**un seul module WASM**, partageant une **unique mémoire linéaire** (`WebAssembly.Memory` `shared`, backée par `SharedArrayBuffer`). `wasm_thread::spawn` instancie un Web Worker sous le capot, mais tous chargent **le même module** et **importent la même mémoire** — d'où le heap commun. La communication inter-thread passe par des **canaux `flume`** transportant des **valeurs Rust typées par move** (zéro sérialisation, zéro copie de frontière) :

```
┌─────────────────────────────────────────────────────────────────┐
│  UI THREAD (thread principal de la page)                          │
│  Chrome : Leptos DOM (ambre-chrome) — AppShell/menus/dialogs/…    │
│  Contenu : ambre_canvas::run → FrameHost<V> dans le <canvas>          │
│  wgpu (QuadRenderer / GlyphAtlas / WgpuSurface), Parley/Swash     │
│  HTMLCanvasElement, GPUDevice — !Send, confinés ici              │
│  requestAnimationFrame → DocView::{update, plan_repaint, paint*}  │
│  ⚠️ ne bloque JAMAIS (try_send / try_recv côté UI)               │
└───────────────┬──────────────────────────────▲──────────────────┘
   flume bounded │ LogicCommand (UI → Logic)    │ UiEvent (Logic → UI)
                 ▼                              │
┌───────────────────────────────────────────────────────────────────┐
│  LOGIC THREAD (wasm_thread — heap partagé)                         │
│  ambre_logic::run → LogicState { docs: HashMap<Uuid, DocState> }   │
│  ambre-trame (TrameState) — LoroDoc, dispatch par match            │
│  Selector::wait sur (from_ui, from_sync)                           │
└───────────────┬──────────────────────────────▲──────────────────┘
   flume bounded │ SyncCommand (Logic → Sync)   │ LogicEvent (Sync → Logic)
                 ▼                              │
┌───────────────────────────────────────────────────────────────────┐
│  SYNC THREAD (wasm_thread — heap partagé)                          │
│  ambre_sync::run → SyncState                                       │
│  web_sys::WebSocket — !Send, confiné ici                          │
│  Pipeline crypto / compression (placeholder)                      │
└───────────────┬───────────────────────────────────────────────────┘
                │  WebSocket binaire  ([version_byte] [postcard(WsServerMsg)])
                │  ambre-protocol : WsClientMsg / WsServerMsg
┌───────────────▼───────────────────────────────────────────────────┐
│  BLIND RELAY (ambre-server — natif, Axum/Tokio)                    │
│  Dépendances : ambre-core + ambre-protocol + ed25519-dalek         │
└───────────────────────────────────────────────────────────────────┘
```

**Symétrie des trois runtimes :**
```
ambre_canvas::run(canvas_id, make_view)                  ← thread UI (générique sur DocView)
ambre_logic::run(from_ui, to_ui, to_sync, from_sync) ← thread Logic
ambre_sync::run(from_logic, to_logic)                ← thread Sync
```
Le seul morceau app-aware côté UI (construire `TrameView`) est **injecté par closure** depuis `ambre-app` → pas de cycle (`ambre-canvas` ne dépend pas d'`ambre-trame`).

**Justification de la séparation Logic/Sync :** le déchiffrement + décompression d'un update distant (~0.5–2 ms sur mobile) ne doit pas interrompre la réponse aux events P1 (scroll) du thread Logic. Avec deux threads séparés et un heap partagé, ces opérations s'exécutent en parallèle réel **sans recopie de payload** : un `Vec<u8>` passe d'un thread à l'autre **par move** sur le canal `flume`, sans memcpy de frontière.

**Justification du modèle à mémoire partagée (vs shared-nothing — §6) :** le transport `flume` déplace des valeurs Rust typées (y compris les payloads applicatifs) sans aucune sérialisation ni recopie — par opposition au `postMessage` du Modèle A qui imposait `postcard` + deux memcpy WASM↔JS par hop. Contrepartie : la **toolchain** (nightly + `-Z build-std` + flags `+atomics`) et les **headers HTTP COOP/COEP** requis par `SharedArrayBuffer`. La séparation des préoccupations reste garantie — non plus par l'isolation mémoire des binaires, mais par le **graphe de dépendances des crates**.

**Backpressure — canaux bornés.** Les 4 canaux `flume` sont `bounded(CHANNEL_CAPACITY = 1024)` (créés dans `ambre_app::main`). Un canal non borné laisserait un consommateur momentanément lent accumuler une file illimitée dans le heap partagé (ex. key-repeat de scroll empilant des `ScrollViewport` plus vite que Logic ne draine). Les **senders des threads worker (Logic, Sync) peuvent bloquer sur canal plein** = c'est la backpressure voulue. **Le thread UI ne bloque jamais** : `TrameView::send_command` utilise `try_send` et **droppe sur `Full`** (une requête viewport est superseded par la suivante via `version` ; aucune commande non-idempotente ne transite par ce chemin chaud aujourd'hui). De plus, les `ScrollViewport` sont **coalescés à ≤ 1 envoi par frame** (`handle_input` accumule dans `pending_viewport`, `update` flushe une fois par frame).

> ⚠️ **Limitation connue (`Atomics.wait`).** Bloquer sur `flume::Selector::wait` / `recv` dans un Worker WASM utilise `Atomics.wait`, ce qui **gèle l'event-loop JS du Worker** (WebSocket/`setTimeout`). C'est légal **hors** du thread principal (Logic et Sync sont des workers). OK tant que le pipeline WS est un placeholder ; **quand le vrai WebSocket sera câblé, le thread Sync devra passer à une boucle non-bloquante** (`recv_async` + WS piloté par `JsFuture`/`spawn_local`). Voir §7 ARCH P1.

**Modèle « thread failure ».** Deux chemins :
- **Gracieux** : un thread qui détecte une condition irrécupérable émet `UiEvent::FatalError { message }` **avant** de sortir. `TrameView::update` bascule alors en état `fatal` (plus aucun envoi, log unique).
- **Non gracieux (panic)** : sous WASM multi-thread, `panic=abort` (via `build-std` + `panic_abort`). ⚠️ **Correctif d'une affirmation antérieure :** `panic=abort` **ne déroule pas la pile**, donc **aucun `Drop` ne s'exécute** et **aucun `Sender` n'est droppé** — le `Sender<UiEvent>` du thread Logic vit dans la mémoire **partagée** et est **fuité** sur abort. L'UI voit donc `Empty` pour toujours, **jamais `Disconnected`** : on **ne peut pas** s'appuyer sur la déconnexion de canal pour détecter un panic. La détection doit passer par un signal **hors-bande** : le `panic::set_hook` (déjà posé dans `ambre-app`) doit, en plus de logger, appeler un hook JS `ambreFatal(message)` (symétrique d'`ambreReady`) **avant** l'abort. *Reste à câbler (§7 ARCH P1).* La déconnexion de canal n'est, au mieux, qu'un signal de **shutdown propre** (teardown volontaire).

**Isolation par tab :** chaque tab instancie sa propre application complète (3 threads + 1 heap partagé). Deux tabs ouvrant le même document sont deux clients CRDT indépendants — convergence par le Blind Relay (même flow que deux utilisateurs). Loro gère cela nativement. Optimisation post-MVP : `BroadcastChannel` inter-tabs.

**Sync multi-document :** le thread Sync gère **N sessions de sync en parallèle** (`HashMap<DocumentId, SyncDocumentState>`) sur un **unique WebSocket multiplexé**. Nécessaire pour les documents embarqués (une Toile peut inclure un Prose + un Trame, chacun avec sa `DocumentKey` et son `LoroDoc`). Démultiplexage par `document_id`. `OpenDocument` → `Subscribe`, `CloseDocument` → `Unsubscribe`. WS ouvert au `Unlock`, fermé au `Logout` ; reconnexion → resubscribe automatique.

**Documents embarqués (Toile → Prose/Trame) :** MVP read-only live (le thread Logic ouvre un état viewer, reçoit les `RemoteUpdate`, n'émet jamais de `SendUpdate`) ; post-MVP editable in-place (routage d'input par focus, spec dédiée). La relation d'imbrication est stockée dans le `LoroDoc` de la Toile (contenu chiffré, invisible serveur). Aucune table Postgres d'imbrication.

### C. Transport inter-thread — canaux `flume` (`ambre-app::main`)

`ambre-app/src/lib.rs` (`#[wasm_bindgen(start)]`) est le **seul point de création et de distribution** des canaux. Aucune autre crate n'en crée. Il n'existe plus de `worker.rs`/`bridge.rs`/`bridge_sync.rs` (Modèle A supprimé).

```rust
// ambre-app/src/lib.rs (extrait)
const CHANNEL_CAPACITY: usize = 1024;

#[wasm_bindgen(start)]
pub fn main() {
    // tracing_wasm + console_error_panic_hook posés ici (hook panic ⇒ ambreFatal, à câbler).

    // 4 canaux bornés (backpressure). Worker senders peuvent bloquer ; l'UI jamais.
    let (ui_to_logic_tx, ui_to_logic_rx) = flume::bounded::<LogicCommand>(CHANNEL_CAPACITY);
    let (logic_to_ui_tx, logic_to_ui_rx) = flume::bounded::<UiEvent>(CHANNEL_CAPACITY);
    let (logic_to_sync_tx, logic_to_sync_rx) = flume::bounded::<SyncCommand>(CHANNEL_CAPACITY);
    let (sync_to_logic_tx, sync_to_logic_rx) = flume::bounded::<LogicEvent>(CHANNEL_CAPACITY);

    wasm_thread::Builder::new().name("logic".into()).spawn(move || {
        ambre_logic::run(ui_to_logic_rx, logic_to_ui_tx, logic_to_sync_tx, sync_to_logic_rx);
    }).expect("spawn Logic thread");

    wasm_thread::Builder::new().name("sync".into()).spawn(move || {
        ambre_sync::run(logic_to_sync_rx, sync_to_logic_tx);
    }).expect("spawn Sync thread");

    // UI tourne sur CE thread (le principal), pilotée par la closure qui injecte la vue.
    ambre_canvas::run("ambre-canvas", move |w, h| {
        TrameView::new(ui_to_logic_tx, logic_to_ui_rx, w, h)
    });
}
```

**Le thread Logic** boucle sur `flume::Selector::wait` (attend la première des deux entrées `from_ui`/`from_sync`), dispatche, recommence ; sort quand **les deux** canaux entrants sont déconnectés. **Le thread Sync** boucle sur `from_logic.recv()` (bloquant) jusqu'à déconnexion. **Injection de dépendance par closure** : `ambre_canvas::run<V, F>(canvas_id, make_view)` reçoit `F: FnOnce(f64, f64) -> V` ; `ambre-canvas` ne nomme jamais `TrameView` — c'est ainsi qu'il reste en amont des apps sans cycle (cf. `DocView` vs `AppView`, §6).

> **Évolution hybride (UI Chrome — voir `ambre-spec-chrome.md`).** Le modèle threads/canaux ci-dessus est **inchangé** ; le chrome Leptos est **additif sur le thread UI**. Le bootstrap évolue : `ambre-app` monte d'abord l'`AppShell` Leptos (`ambre-chrome`) qui possède la cellule `<canvas id="ambre-canvas">` ; `ambre_canvas::run` s'attache à ce canvas pour le **contenu**, la closure `make_view` injectant toujours la `DocView` de l'app active. La coquille Leptos et la boucle rAF du canvas cohabitent sur le même thread. **Conséquence :** la coquille devenant propriétaire du canvas et décidant *quelle* vue est montée (onglets/multi-document), elle **rapproche l'échéance** de la bascule `DocView → AppView` (§6) — encore différée, mais plus pour longtemps. (Bootstrap exact à figer au POC, `ambre-spec-chrome.md §7`.)

### D. Modélisation des données — plan de données typé (3 plans) & contrats d'interface

**Plus aucun `Vec<u8>` postcard sur le chemin UI ↔ Logic.** Les payloads applicatifs sont des **valeurs Rust typées**, dans un **monde fermé** (sommes sur les types de documents) défini **dans `ambre-bus`** (pas de nouvelle crate). L'IPC client se décompose en **trois plans** :

| Plan | Porté par | Payload | Routage Logic |
|---|---|---|---|
| **Contenu** | `LogicCommand::App` / `UiEvent::{Viewport,Presence}` | `AppCommand` / `AppEvent` (typés, fermés) | par `DocumentId` vers `DocState` |
| **Service** | `…::Rest { id, … }` (4 directions) | `RestCommand` / `RestEvent` (**définis une fois**, `rest.rs`) | relais verbatim + hooks d'état |
| **Réseau** | `SyncCommand::Send*` / `LogicEvent::Remote*` + push | `Vec<u8>` (octets réseau Loro / chiffrés) | appliqué au CRDT / forwardé |

**Plan contenu (typé, fermé).** `enum AppCommand { Trame(TrameCommand), … }` (UI → Logic) ; `enum AppEvent { Trame(TrameEvent), … }` (Logic → UI). Ajouter un type de document = ajouter un variant (un seul point d'édition, **pas de trait-objet, pas de `Box<dyn Any>`, pas de cycle de crates**). **Composition** (embed Trame-dans-Toile, partage dans un Rendez-Vous) = **récursion** sur ces enums (ex. `ToileCommand::Embedded { child, command: Box<AppCommand> }`), routée par `Uuid` dans le thread Logic. Plugins futurs = variant `Plugin(Box<dyn Plugin>)`. Restent en `Vec<u8>` (vrais octets **réseau**) : payloads CRDT / snapshots Loro et messages de canaux Murmure, sur `SyncCommand`/`LogicEvent`.

**Plan service (relais corrélable).** La quasi-totalité du trafic P3 est un pur relais : l'UI demande une opération, Logic la forwarde verbatim à Sync (REST/crypto/WS), la réponse remonte par le même chemin. Plutôt que de modéliser chaque étape comme un variant d'enum dans chacune des 4 directions (la « triplication P3 » : ~300 lignes de recopie 1:1), la **famille service est définie une seule fois** (`RestCommand`/`RestEvent` + `RequestId` dans `rest.rs`) et **portée par des enveloppes minces** `…::Rest { id, … }` :

```text
LogicCommand::Rest { id, cmd: RestCommand }   // UI    → Logic
SyncCommand ::Rest { id, cmd: RestCommand }   // Logic → Sync
LogicEvent  ::Rest { id, evt: RestEvent }     // Sync  → Logic
UiEvent     ::Rest { id, evt: RestEvent }     // Logic → UI
```

Le dispatch P3 du thread Logic se réduit à **un seul bras** (`Rest { id, cmd } => send_to_sync(Rest{…})`), précédé d'un petit `match &cmd` pour les rares ops à effet d'état (`Logout` vide `docs`, `DeleteDocument` retire l'entrée). Ajouter une op = un variant `RestCommand` + un variant `RestEvent`, édités au même endroit. **`RequestId`** (newtype `u64`) est alloué par l'**UI** via `RequestId::next()` (un `AtomicU64` partagé entre threads — légal grâce à la mémoire partagée), **échoé inchangé** par Logic et Sync. Domicile unique = l'enveloppe `Rest`.

**Convention de nommage** — par **destination + rôle** (`Command` = requête vers le worker en aval, `Event` = notification qui remonte) ; endpoints nommés par direction (`to_ui`/`from_ui`/…, plus de `tx`/`rx`). Suffixe identique aux deux niveaux : `Command` voyage dans `Command`, `Event` dans `Event`.

| Flux | Type | Fichier `ambre-bus/src/` |
|---|---|---|
| UI → Logic | `LogicCommand` | `logic.rs` (+ `AppCommand`, `TrameCommand`) |
| Logic → UI | `UiEvent` | `ui.rs` (+ `AppEvent`, `TrameEvent`, `DisplayCell`) |
| Logic → Sync | `SyncCommand` | `command.rs` |
| Sync → Logic | `LogicEvent` | `event.rs` |
| (partagé) | `RestCommand`/`RestEvent`/`RequestId` + types métier | `rest.rs` |

**Partition `Rest` vs explicite :**
- **Dans `RestCommand`/`RestEvent`** = opérations **requête/réponse corrélables**, réponses de **succès uniquement** : session, document CRUD, partage, users, media, channels CRUD+partage, unread, meetings.
- **Restent explicites sur l'enveloppe** : *contrôle/content-send* (`OpenDocument`/`CloseDocument`, `OpenChannel`/`CloseChannel`, `SendChannelMessage` — fire-and-forget) ; *push serveur* (`Remote*`, `ConnectionStatus`, `SyncWarning`, `MemberRevoked`, `Channel*`, `Meeting*Participant*`) ; *erreurs* (`AuthError`/`ApiError` — réponse **ou** spontanées ; corrélation par `id` = raffinement ultérieur).

#### Contrats d'interface

**`ambre-core` → toutes les crates**

```rust
pub enum DocType { Prose = 0, Trame = 1, Toile = 2, Murmure = 3 } // valeur SQL serveur identique
pub struct PublicKey(pub [u8; 32]);   // Ed25519 ou X25519 selon contexte
// Identifiants typés (newtypes sur Uuid v7) : DocumentId, UserId, ChannelId, MeetingId, SnapshotId.
pub enum Role { Owner, Editor, Viewer }
```

**Rendez-Vous n'est pas un DocType** (meetings éphémères, table dédiée).

**`ambre-draw`** — trait `DrawSurface` (source de vérité, détails en `ambre-spec-canvas.md §D`). Deps : kurbo/peniko/parley, aucune dép wgpu/krilla. Impls : `WgpuSurface` (`ambre-canvas::render`), `KrillaSurface` (PDF, futur, thread Logic, lazy-loaded).

**`ambre-protocol` → `ambre-sync` + `ambre-server` (SOURCE DE VÉRITÉ)** — `Update`/`Snapshot`/`Ephemeral` (avec `commitment_tag` HMAC-SHA256 + `signature` Ed25519), `*PublicData`, `WsClientMsg`/`WsServerMsg` (WS multiplexé : `Subscribe`/`Unsubscribe`/`Update`/`Ephemeral`/`RequestMissing` + variantes channel), `Signaling*`. Encodage wire `[version_byte: u8] [postcard(msg)]` (`encode_client_msg`/`decode_server_msg`) — **seul endroit où postcard subsiste côté client**.

```rust
pub struct Update {
    pub ciphertext: Vec<u8>, pub nonce: [u8; 24], pub public_data: UpdatePublicData,
    pub commitment_tag: [u8; 32], // HMAC-SHA256(doc_key, nonce||ciphertext||public_data) — anti key-commitment
    pub signature: [u8; 64],      // Ed25519 sur (nonce||ciphertext||public_data||commitment_tag)
}
pub struct UpdatePublicData { pub document_id: Uuid, pub pub_key: [u8;32], pub snapshot_id: Uuid, pub clock: u64 }
// Snapshot/SnapshotPublicData (parent_snapshot_proof BLAKE2b, parent_snapshot_update_clocks), Ephemeral/EphemeralPublicData : idem.
```

> **`commitment_tag`** : XChaCha20-Poly1305 n'est pas key-committing ; le HMAC lie le message à une clé spécifique, empêchant un serveur compromis de réinjecter un ciphertext dans un autre contexte. **Snapshots** : via S3 presigned URLs (pas par WS) ; `SnapshotAvailable` notifie. **Signaling** : le serveur voit les SDP (qui appelle qui) — fuite de métadonnées acceptée pour le MVP ; l'E2EE media passe par SFrame.

**`ambre-bus` → threads Logic / Sync / UI** — les **4 enveloppes** + le plan service partagé. **Rien n'y est sérialisé.** Les `Vec<u8>` ne subsistent que pour les vrais octets réseau.

```rust
// ── UI → Logic : logic.rs ──────────────────────────────────────────────────
pub enum LogicCommand {
    App { document_id: DocumentId, command: AppCommand },          // contenu (P2)
    Rest { id: RequestId, cmd: RestCommand },                      // service (P3) relayé
    OpenDocument { document_id: DocumentId },                      // contrôle (P3) fire-and-forget
    CloseDocument { document_id: DocumentId },
    OpenChannel { channel_id: ChannelId },
    CloseChannel { channel_id: ChannelId },
    SendChannelMessage { channel_id: ChannelId, payload: Vec<u8> }, // payload opaque (ambre-murmure)
}
pub enum AppCommand { Trame(TrameCommand) /* , Prose, Toile, … ; futur Plugin(Box<dyn Plugin>) */ }
pub enum TrameCommand {
    /// `version` monotone : l'UI jette toute réponse plus vieille que la dernière acceptée.
    ScrollViewport { version: u64, row_start: u32, row_end: u32, col_start: u32, col_end: u32 },
}

// ── Logic → UI : ui.rs ─────────────────────────────────────────────────────
pub enum UiEvent {
    Viewport { document_id: DocumentId, event: AppEvent },         // contenu
    Presence { document_id: DocumentId, event: AppEvent },
    Rest { id: RequestId, evt: RestEvent },                        // service
    ConnectionStatus { document_id: DocumentId, connected: bool }, // push & statut
    SyncWarning { document_id: DocumentId, code: u16, message: String },
    MemberRevoked { document_id: DocumentId, revoked_user_id: UserId },
    ChannelMessage { channel_id: ChannelId, payload: Vec<u8> },
    ChannelPresenceUpdate { channel_id: ChannelId, payload: Vec<u8> },
    ChannelConnectionStatus { channel_id: ChannelId, connected: bool },
    MeetingParticipantJoined { meeting_id: MeetingId, user_id: UserId },
    MeetingParticipantLeft { meeting_id: MeetingId, user_id: UserId },
    AuthError { message: String },
    ApiError { request: String, code: u16, message: String },
    /// Échec irrécupérable d'un thread (chemin GRACIEUX). Le panic, lui, est
    /// panic=abort et NE se manifeste PAS comme un disconnect (cf. §2.B).
    FatalError { message: String },
}
pub enum AppEvent { Trame(TrameEvent) }
pub enum TrameEvent { ViewportData { version: u64, row_start: u32, col_start: u32, cells: Vec<DisplayCell> } }
pub struct DisplayCell { pub row: u32, pub col: u32, pub text: String } // plat, data-oriented

// ── Logic → Sync : command.rs ──────────────────────────────────────────────
pub enum SyncCommand {
    Rest { id: RequestId, cmd: RestCommand },
    OpenDocument { document_id: DocumentId }, CloseDocument { document_id: DocumentId },
    SendUpdate   { document_id: DocumentId, payload: Vec<u8> },   // delta CRDT Loro brut
    SendSnapshot { document_id: DocumentId, payload: Vec<u8> },   // état complet → S3
    SendEphemeral{ document_id: DocumentId, payload: Vec<u8> },   // curseur/présence (chiffré par Sync)
    Reconnect    { document_id: DocumentId },
    OpenChannel  { channel_id: ChannelId }, CloseChannel { channel_id: ChannelId },
    SendChannelUpdate    { channel_id: ChannelId, payload: Vec<u8> },
    SendChannelEphemeral { channel_id: ChannelId, payload: Vec<u8> },
    Shutdown,
}

// ── Sync → Logic : event.rs ────────────────────────────────────────────────
pub enum LogicEvent {
    Rest { id: RequestId, evt: RestEvent },
    RemoteUpdate   { document_id: DocumentId, payload: Vec<u8> },  // déchiffré/décompressé → LoroDoc::import
    RemoteSnapshot { document_id: DocumentId, payload: Vec<u8> },
    RemoteEphemeral{ document_id: DocumentId, payload: Vec<u8> },
    ConnectionStatus { document_id: DocumentId, connected: bool },
    SyncWarning { document_id: DocumentId, code: u16, message: String },
    SnapshotNeeded { document_id: DocumentId },                   // → export_snapshot → SendSnapshot
    MemberRevoked { document_id: DocumentId, revoked_user_id: UserId },
    ChannelRemoteUpdate { channel_id: ChannelId, payload: Vec<u8> },
    ChannelRemoteEphemeral { channel_id: ChannelId, payload: Vec<u8> },
    ChannelConnectionStatus { channel_id: ChannelId, connected: bool },
    ChannelSnapshotNeeded { channel_id: ChannelId },
    MeetingParticipantJoined { meeting_id: MeetingId, user_id: UserId },
    MeetingParticipantLeft { meeting_id: MeetingId, user_id: UserId },
    AuthError { message: String }, ApiError { request: String, code: u16, message: String },
}

// ── Plan service partagé : rest.rs ─────────────────────────────────────────
pub struct RequestId(pub u64); // alloué par l'UI (RequestId::next, AtomicU64 partagé), échoé par Logic/Sync
pub enum RestCommand {
    // Session
    Unlock { passphrase: Vec<u8> }, SetupAccount { passphrase: Vec<u8> },
    ChangePassphrase { current_passphrase: Vec<u8>, new_passphrase: Vec<u8> },
    RecoverAccount { recovery_words: String, new_passphrase: Vec<u8> }, Logout,
    // Document CRUD
    ListDocuments, CreateDocument { doc_type: DocType, name: String },
    DeleteDocument { document_id: DocumentId }, RenameDocument { document_id: DocumentId, name: String },
    // Partage
    InviteToDocument { document_id: DocumentId, invitee_email: String, role: Role },
    RevokeFromDocument { document_id: DocumentId, user_id: UserId },
    ListDocumentMembers { document_id: DocumentId },
    UpdateDocumentMemberRole { document_id: DocumentId, user_id: UserId, role: Role },
    // Users / Media
    SearchUsers { query: String },
    UploadMedia { document_id: DocumentId, data: Vec<u8>, mime_type: String },
    DownloadMedia { document_id: DocumentId, content_hash: String },
    // Channels (CRUD & partage) + unread
    ListChannels, CreateChannel { name: String }, DeleteChannel { channel_id: ChannelId },
    InviteToChannel { channel_id: ChannelId, invitee_email: String, role: Role },
    RevokeFromChannel { channel_id: ChannelId, user_id: UserId },
    ListChannelMembers { channel_id: ChannelId },
    UpdateChannelMemberRole { channel_id: ChannelId, user_id: UserId, role: Role },
    UpdateReadCursor { channel_id: ChannelId, last_seq: u64 }, GetUnreadCounts,
    // Meetings
    CreateMeeting { name: String }, JoinMeeting { meeting_id: MeetingId },
    LeaveMeeting { meeting_id: MeetingId }, EndMeeting { meeting_id: MeetingId },
}
pub enum RestEvent { // réponses de SUCCÈS uniquement (échecs = AuthError/ApiError sur l'enveloppe)
    SessionReady { user_id: UserId, display_name: String, email: String },
    AccountSetupRequired, AccountCreated { recovery_words: String },
    PassphraseChanged, AccountRecovered, LoggedOut,
    DocumentList { documents: Vec<DocumentSummary> },
    DocumentCreated { document_id: DocumentId, doc_type: DocType }, // hook Logic: pending_doc_types
    DocumentDeleted { document_id: DocumentId }, DocumentRenamed { document_id: DocumentId },
    InviteSent { document_id: DocumentId, user_id: UserId, display_name: String },
    RevocationComplete { document_id: DocumentId },
    DocumentMembers { document_id: DocumentId, members: Vec<MemberInfo> },
    MemberRoleUpdated { document_id: DocumentId, user_id: UserId, role: Role },
    UserSearchResults { users: Vec<UserInfo> },
    MediaUploaded { document_id: DocumentId, content_hash: String },
    MediaDownloaded { document_id: DocumentId, content_hash: String, data: Vec<u8> },
    ChannelList { channels: Vec<ChannelSummary> }, ChannelCreated { channel_id: ChannelId },
    ChannelDeleted { channel_id: ChannelId }, ChannelInviteSent { channel_id: ChannelId, user_id: UserId },
    ChannelRevocationComplete { channel_id: ChannelId },
    ChannelMembers { channel_id: ChannelId, members: Vec<MemberInfo> },
    ChannelMemberRoleUpdated { channel_id: ChannelId, user_id: UserId, role: Role },
    UnreadCounts { counts: Vec<UnreadCount> },
    MeetingCreated { meeting_id: MeetingId }, MeetingJoined { meeting_id: MeetingId },
    MeetingLeft { meeting_id: MeetingId }, MeetingEnded { meeting_id: MeetingId },
}
// Types métier partagés (rest.rs) — zéro type crypto, noms déjà déchiffrés par Sync :
// DocumentSummary, MemberInfo, UserInfo, ChannelSummary, UnreadCount.
```

> **Plus de `AppController` ni de `CommandSink`** (traits du Modèle A supprimés). Le registre des documents est **typé** : `HashMap<DocumentId, DocState>` où `enum DocState { Trame(TrameState), … }`, et le dispatch est un **`match`** sur `(DocState, AppCommand)` — pas de trait-objet, pas de `downcast`, pas de `Box<dyn Any>`. La logique envoie sur un `&flume::Sender<SyncCommand>` nu.

**Dispatch du thread Logic (`ambre-logic::LogicState`)** : `handle_logic_command` route `App` vers `docs[document_id]` (drop si inconnu) → `UiEvent::Viewport` ; `Rest` → un seul bras forward `SyncCommand::Rest` (+ hooks `Logout`/`DeleteDocument`) ; contrôle → counterpart `SyncCommand`. `handle_logic_event` : `Rest` → forward `UiEvent::Rest` (+ hook `DocumentCreated` → `pending_doc_types`) ; `RemoteSnapshot` promeut un `pending_doc_type` en `DocState` vivant ; `SnapshotNeeded` → `export_snapshot` → `SendSnapshot` ; push/erreurs forwardés explicitement.

### E. Crates et Dépendances Externes

**Serveur (`ambre-server`, natif) :** `rustis`, `sqlx`, `aws-sdk-s3`, `axum`/`tower`/`hyper`, `postcard`/`serde`/`serde_json`, `thiserror`/`tracing`, `ed25519-dalek` (direct).

**Client WASM (un seul binaire, dépendances mutualisées) :**
- `wasm_thread` — spawn de threads WASM (Web Workers chargeant le même module + mémoire partagée).
- `flume` (feature `select`) — canaux MPMC inter-thread (Logic/Sync s'appuient sur `Selector`).
- `wgpu` (+ `webgl`) + `swash` — rendu GPU du **contenu** (thread UI). WebGL2 par défaut, WebGPU opt-in `?webgpu`.
- `parley` — layout typographique (UI + pagination Logic). Fonts via `fetch()` même origine.
- `leptos` — **chrome DOM** (thread UI ; `ambre-chrome` partagé + chrome spécifique des apps). `nucleo-matcher` — fuzzy palette.
- `accesskit` — a11y du **contenu** canvas (scopé ; le chrome DOM est accessible nativement). `kurbo`/`peniko` — types (deps `ambre-draw`).
- `loro` — CRDT (Logic). `petgraph` — DAG formules (Trame). `rstar` — R-Tree (Toile).
- `web-sys`/`js-sys`/`wasm-bindgen`/`wasm-bindgen-futures` — glue navigateur (UI `host` + Sync).
- `gloo-net` (websocket+http) — réseau (Sync). `ruzstd` — compression. `zeroize` — secrets.
- `tracing` + `tracing-wasm` + `console_error_panic_hook` — logging + hook panic (posés dans `ambre-app`).
- Crypto (futur, `ambre-crypto`) : `chacha20poly1305`/`ed25519-dalek`/`x25519-dalek`/`blake2`/`argon2`.
- `krilla`/`docx-rs`/`html5ever`/`pulldown-cmark` — interop import/export (lazy-loaded, Logic).
- `postcard` — **uniquement** le wire réseau (`ambre-protocol`).

---

## 3. SÉCURITÉ & CONFIDENTIALITÉ (ZERO-KNOWLEDGE)

**Frontières du Blind Relay :**

| Composant | Données vues par le serveur | Jamais vues |
|-----------|----------------------------|-------------|
| `ambre-server` | `document_id`, `pub_key` (Ed25519), `clock`, `seq`, `snapshot_id`, `nonce` | Contenu CRDT, Document Key, Master Seed, User Key, passphrase |

**Vérification statique par `cargo tree` :** `ambre-server` n'importe aucun type de `ambre-trame`/`ambre-bus`/`ambre-crypto` ; il importe `ed25519-dalek` directement (pas `ambre-crypto`). Tous les `ciphertext` dans `ambre-protocol` sont `Vec<u8>` opaques.

**Séparation des clés :**
- `DocumentKey` et `MasterSeed` (`ambre-crypto`) implémentent `ZeroizeOnDrop`. Jamais sérialisés dans `ambre-protocol` ni `ambre-bus`.
- Les canaux `flume` Logic ↔ Sync transportent des deltas CRDT **déchiffrés** (`Vec<u8>`, par move dans le heap partagé). Frontière interne considérée sûre : le heap n'est accessible que depuis l'intérieur du process navigateur de l'utilisateur. La mémoire **partagée** retire l'argument « isolation par binaire » du Modèle A — la confiance repose sur le sandbox du process et le graphe de crates (le thread UI ne compile pas la crypto, etc.).
- Données de présence éphémère **chiffrées** avant envoi (payload `Ephemeral`).

> **Transit des secrets par le thread Logic (mineur, documenté).** La passphrase voyage `UI → Logic → Sync`. Logic la **move** sans copie ni usage, mais ces octets résident un instant dans le heap **partagé** et `zeroize` n'est pas encore câblé. **Pas de correctif code** : zeroizer côté `ambre-bus` imposerait une dép `zeroize`, **violant l'invariant `ambre-bus → ambre-core` uniquement**. La zeroization reste à la charge de `ambre-sync` (§7 ARCH P1). Choix assumé.

**Cryptographie & gestion des clés.** Toutes les opérations crypto s'exécutent dans le **thread Sync** (seul consommateur d'`ambre-crypto`). Primitives : XChaCha20-Poly1305 (AEAD), Ed25519 (signature), X25519 (ECDH), BLAKE2b (dérivation/hash), Argon2id (KDF passphrase), HMAC-SHA256 (`commitment_tag`).

**Hiérarchie des clés E2E (côté client) :** les champs Postgres (`master_key_enc`, `master_key_salt`, `doc_key_enc`, `org_master_key_enc`) sont des blobs opaques pour le Blind Relay (voir `ambre-spec-server.md §2.A`).

```
Passphrase (saisie UI → transit canal flume → thread Sync, JAMAIS transmise au serveur)
  │  Argon2id(salt=users.master_key_salt)  [thread Sync]
  ▼
User Key (éphémère, dérivée à chaque login)
  │  XChaCha20-Poly1305 déchiffre users.master_key_enc
  ▼
Master Seed (256 bits, permanente, stockée chiffrée côté serveur)
  ├── BLAKE2b(b"ambre-signing-key-v1", seed)  → Signing Key Ed25519  → signing_pubkey (Postgres)
  └── BLAKE2b(b"ambre-exchange-key-v1", seed) → Exchange Key X25519   → exchange_pubkey (Postgres)
                                                  │  X25519 ECDH avec exchange_pubkey du owner
                                                  │  → déchiffre document_members.doc_key_enc
                                                  ▼
                                              Document Key (une par doc, 256 bits)
                                                  │  XChaCha20-Poly1305
                                                  ▼
                                              Contenu CRDT (snapshots + updates + ephemeral)
```

**TOFU :** à la première invitation, l'UI affiche le fingerprint BLAKE2b-256 (base32) de l'`exchange_pubkey`. Clés acceptées stockées dans `IndexedDB` (`trusted_keys`). Toute divergence ultérieure alerte. Limite : substitution possible au premier échange si non vérifiée out-of-band.

**Changement de passphrase :** nouvelle passphrase → Argon2id → nouvelle User Key → rechiffre `master_key_enc` uniquement (Master Seed inchangée) → `PUT /api/keys/master/rotate`. **Partage / rotation post-révocation** : `ambre-spec-sync.md §3` (15 étapes). **Multi-device, Recovery Key (24 mots BIP39), Org Recovery Key (post-MVP — escrow Shamir/Vault via trait `OrgKeyStore`)** : inchangés par le passage au Modèle B ; tables (`organizations`, `org_recovery_log`) et routes (`POST /api/organizations/{id}/recover/{uid}`) dans `ambre-spec-server.md §2`.

```rust
// ambre-crypto/src/key_store.rs (cible)
pub struct MasterSeed(pub [u8; 32]); // ZeroizeOnDrop, 24 mots BIP39 (Recovery Key)
impl MasterSeed {
    pub fn derive_signing_key(&self) -> ed25519_dalek::SigningKey { /* BLAKE2b ctx séparé */ }
    pub fn derive_exchange_key(&self) -> x25519_dalek::StaticSecret { /* BLAKE2b ctx séparé */ }
}
```

---

## 4. QUALITÉ & TESTABILITÉ

* **Tests unitaires / testabilité hôte.** Le thread Logic étant **100 % Rust pur** (zéro glue wasm), tout le routage commande/événement est exercé **sur l'hôte** (x86_64) avec des canaux `flume` mock : les rustflags wasm sont scopés `[target.wasm32-unknown-unknown]`, donc `cargo test -p ambre-logic` tourne nativement. **C'est un bénéfice de premier ordre du découpage** (pas seulement une propriété du graphe de deps). `ambre-logic` a **9 tests** (`#[cfg(test)]`) construisant `LogicState` avec deux receivers drainables et vérifiant : forward P3 → `SyncCommand` avec **écho de l'`id`** ; préservation des champs ; `App` routé / drop sur doc inconnu ; promotion `DocumentCreated` → `RemoteSnapshot` → état vivant ; `SnapshotNeeded` → `SendSnapshot` ; `Logout` vide `docs` ; `CloseDocument` drop + forward. Étendre à `ambre-sync` quand son pipeline sortira du placeholder.

* **Stratégie par crate :**

| Crate | Tests unitaires | Tests d'intégration | Prop-tests | Benchmarks |
|-------|----------------|---------------------|------------|------------|
| `ambre-core` | 100 % des types | — | — | — |
| `ambre-crypto` | Round-trip, tampering, zeroize | — | ✓ (proptest) | ✓ (criterion) |
| `ambre-protocol` | Sérialisation round-trip | — | ✓ (postcard roundtrip) | — |
| `ambre-bus` | Construction des enveloppes | — | — | — |
| `ambre-logic` | **Dispatch typé sur hôte (9 tests, flume mock)** | — | — | — |
| `ambre-sync` | Pipeline crypto, quarantaine, rotation | `MockBlindRelay` in-process | ✓ (clock monotonic) | ✓ (encrypt/decrypt 1KB) |
| `ambre-canvas` | TextCache, WgpuSurface (MockSurface), dirty/RepaintPlan | headless | — | frame budget |
| `ambre-commands` | Lookup registre, keybindings, état dérivé (`CommandState`) | — | — | — |
| `ambre-chrome` | Composants Leptos (patterns a11y ARIA), positionnement menus (flip) | `wasm-bindgen-test` (DOM) | — | — |
| `ambre-server` | Routes, ACL, Lua Redis | Docker Compose | ✓ (seq ordering) | ✓ (latency p99) |
| `ambre-trame` | `TrameState` handlers, formules, cycle detection | via `ambre-logic` | ✓ (déterminisme) | ✓ (recalcul 10k) |
| `ambre-app` | — | End-to-end headless | — | — |

* **Mocks & environnement :** tests natifs en `#[cfg(not(target_arch = "wasm32"))]` ; tests WASM (GPU/DOM) via `wasm-bindgen-test`.

---

## 5. PERFORMANCE & RESSOURCES

* **Budget par opération (target : hardware mobile 2024) :**

| Opération | Cible (P95 mobile) | Ceiling | Thread | Méthode |
|-----------|-------------------|---------|--------|---------|
| Envoi/réception `flume` (move d'enveloppe typée, heap partagé) | < 1 µs | — | tous | benchmark |
| Round-trip scroll UI → Logic → UI (P1) | < 2 ms | — | UI ↔ Logic | benchmark e2e |
| Chiffrement outbound 1KB (compress+AEAD+HMAC+Ed25519) | < 200 µs | < 500 µs | Sync | criterion |
| Déchiffrement inbound 1KB (verify+HMAC+AEAD+decompress) | < 350 µs | < 1 ms | Sync | criterion |
| Round-trip crypto complet (hors réseau) | < 600 µs | < 1.5 ms | Sync | criterion |
| Compression snapshot 100KB (ruzstd) | < 5 ms | — | Sync | criterion |
| `doc.import()` Loro update (1KB delta) | < 2 ms | — | Logic | criterion |
| Recalcul DAG formules Trame (10k cellules) | < 50 ms | — | Logic | benchmark custom |
| Réponse viewport scroll (P1, bout en bout) | < 15 ms | — | Logic → UI | benchmark e2e |

> Le `flume` à mémoire partagée **supprime** les coûts de frontière du Modèle A (sérialisation postcard + 2 memcpy WASM↔JS par hop) : un `Vec<u8>` change de thread **par move**. Coût dominant restant : scheduling du thread réveillé. Goulot crypto : Ed25519 (~80–250 µs). Smoke-test IPC : RTT ~1–17 ms (borné par la frontière rAF), overhead `flume` < 1 ms.

* **Impact WASM.** Modèle B = **un seul binaire** : dépendances dédupliquées une fois (plus de triple copie de RustCrypto/loro entre binaires comme le craignait le Modèle A). Budget total < 2 MB gzip ; « sans interop Prose » (~429 KB gzip) tient ; l'interop Prose (`docx-rs`/`krilla`/`html5ever`/`pulldown-cmark`) reste **lazy-loaded** (bundle splitting dans le thread Logic, pas un thread de plus). Heap : **partagé unique** (une seule `WebAssembly.Memory`, max 4 GiB) ; `SyncState` < 1 MB, `SyncDocumentState`/doc < 1 MB, `LoroDoc` < 10 MB cible, quarantaine bornée 1000 updates/doc, DAG Trame < 50 MB pour 100k cellules.

---

## 6. DÉCISIONS ARCHITECTURALES ÉCARTÉES

> ⚠️ **Inversion par rapport à la v1 de cette spec.** Le Modèle A (shared-nothing, `postMessage`, 3 binaires) **était** l'architecture retenue et listait `SharedArrayBuffer`/`flume`/`wasm_thread` comme rejetés. Le code a basculé en **Modèle B** : ces trois technologies sont désormais **adoptées** (voir §1/§2). La table ci-dessous documente **ce qui reste écarté** — Modèle A inclus — pour ne pas le re-proposer.

| Alternative | Rejetée car |
|-------------|-------------|
| **Modèle A — shared-nothing (3 binaires WASM, `postMessage` + transfert `ArrayBuffer` + `postcard`)** | Imposait une sérialisation `postcard` + **deux memcpy WASM↔JS par hop**, et **interdisait `flume`** (un `Arc` d'un Worker pointe vers une mémoire absente dans l'autre). Le Modèle B (mémoire partagée) déplace des **valeurs Rust typées par move**, sans sérialisation ni copie, payloads applicatifs inclus. Coût accepté du Modèle B : headers HTTP COOP/COEP (`SharedArrayBuffer`) + toolchain nightly `-Z build-std`. Ne pas revenir au shared-nothing : on perdrait le plan de données typé et la performance du transport. |
| **Web Workers JS séparés + `postMessage` (Modèle A) / `MessageChannel`/`MessagePort`** | Remplacés par `wasm_thread::spawn` (Web Workers chargeant le **même** module + important la **même** mémoire partagée) et canaux `flume`. Plus de `worker.rs`/`bridge*.rs`, plus de features `web-sys` `MessageChannel`/`MessagePort`/`Worker*`. |
| **`AppController` / `CommandSink` (trait-objets) + registry `HashMap<Uuid, Box<dyn AppController>>`** | Remplacés par le **plan de données typé** : `enum DocState` + `HashMap<Uuid, DocState>` + dispatch par `match`. Pas de trait-objet, pas de `downcast`. Un trait-objet réintroduirait un cycle de crates potentiel (l'enum fermé se résout en aval, dans `ambre-logic`) et empêcherait la testabilité hôte simple. |
| **`Box<dyn Any>` / `Vec<u8>` postcard pour les payloads applicatifs** | Remplacés par `AppCommand`/`AppEvent` (sommes fermées, typées). Le compilateur garantit l'exhaustivité du dispatch ; ajouter un type de doc = un variant. Les `Vec<u8>` ne subsistent que pour les **vrais octets réseau** (deltas/snapshots Loro, messages de canaux). |
| **Enums P3 par direction (4 enums quasi-identiques, « triplication »)** | ~300 lignes de recopie 1:1 sujettes au « mauvais champ recopié ». Remplacés par la famille `RestCommand`/`RestEvent` **définie une fois** et portée par l'enveloppe `…::Rest { id, … }` ; dispatch P3 = un seul bras. |
| **Nouvelle crate pour le plan de données typé** | Inutile : `AppCommand`/`AppEvent` vivent dans `ambre-bus` (feuille en amont). Une crate dédiée créerait un cycle ou une dépendance superflue. |
| **Effect-sink injecté pour l'outcome d'édition (vers Sync)** | Réintroduirait un trait-objet (l'inverse de la suppression de `CommandSink`). Choix : **retour outcome typé** (`TrameOutcome { display, update, ephemeral }`, octets domaine) éclaté par le dispatcher Logic — préserve la pureté + testabilité hôte. |
| **Makepad** | macro `script_mod!` contournant le système de types Rust, incompatible avec le pilier sécurité (pas d'`unsafe` maison). |
| **Crate unique (`ambre-all`)** | Temps de compilation excessif, violation séparation serveur/client. |
| **JSON pour le wire réseau** | `postcard` ~10× plus compact, parsing plus rapide. (Sur le chemin inter-thread, **rien n'est sérialisé** : valeurs Rust sur `flume`.) |
| **`ambre-crypto` dans `ambre-server`** | Moindre privilège : exposerait `DocumentKey`/`MasterSeed` au serveur. `ed25519-dalek` direct suffit. |
| **Types wire redéfinis dans `ambre-sync`/`ambre-server`** | Divergence garantie (`[u8; 32]` vs `Vec<u8>`). Source de vérité unique = `ambre-protocol`. |
| **`ambre-sync`/`ambre-logic` comme dépendance directe des apps** | Couplage fort logique↔infra. Les apps ne connaissent que les types IPC d'`ambre-bus`. |
| **Logic + Sync fusionnés (un seul thread)** | Le déchiffrement/décompression bloquerait la réponse aux events P1 (scroll). Séparation obligatoire (< 15 ms). |
| **`ambre-canvas` dépendant des apps (`AppView` enum fermé aujourd'hui)** | Créerait un cycle (`ambre-canvas` est en amont des apps). D'où `DocView` + injection par closure. La bascule vers `AppView` reste possible **mais impose de déplacer `FrameHost` hors d'`ambre-canvas`** — différée au multi-document (§7 ARCH P2). |
| **UI 100 % canvas (framework widget maison) + Vello/Masonry/Xilem** | Rejeté au profit de l'**UI hybride** (chrome DOM Leptos + contenu wgpu, web-only). Détail et rejets associés : modèle UI (Masonry, Xilem, winit, WebGPU-only) en `ambre-spec-ui.md §6`, chrome (egui, ribbon) en `ambre-spec-chrome.md §6`, contenu (Vello) en `ambre-spec-canvas.md §6`. Le pipeline GPU/texte d'`ambre-canvas` est **conservé** ; seul le framework widget est retiré. |
| **Un WebSocket par document** | Gaspillage (limites navigateur ~6 conns/origine, batterie). Un seul WS multiplexé, Subscribe/Unsubscribe par doc. |
| **Séparation `ambre-sync` / `ambre-network`** | La couche réseau est assez légère pour rester dans `ambre-sync`. Séparer créerait un cycle ou de la complexité. |
| **Framework DI complexe** | Le registre des documents est un `HashMap` ; l'injection (closure `make_view`, canaux) est explicite dans `ambre-app`. Pas de framework DI. |
| **4e thread pour l'import/export** | Inutile : pendant un import/export l'UI affiche un waiting screen ; le thread Logic bloqué (sur version figée du `LoroDoc`) est le comportement voulu. Lazy loading WASM, pas un thread de plus. |

---

## 7. PLAN D'INTÉGRATION (ROADMAP)

> Backlog détaillé de cette spec ; l'ordonnancement transversal (milestones) vit dans `specs/ambre-roadmap.md`.

### ARCH P0 — Foundation
- [x] Workspace + `rust-toolchain.toml` (nightly, `rust-src`, `wasm32`) + `.cargo/config.toml` (`+atomics`, `--shared-memory`, `-Z build-std`).
- [x] Prototype threading shared-memory validé : 3 threads `wasm_thread` + 4 canaux `flume` + heap partagé. Smoke-test IPC opérationnel.
- [x] `ambre-core` ; `ambre-protocol` (encode/decode, round-trip) ; `ambre-bus` (4 enveloppes + plan service, zéro serde) ; `ambre-draw`.
- [x] `ambre-crypto` (wrappers RustCrypto, `ZeroizeOnDrop`, prop-tests, benchmarks ; enveloppe `seal`/`open` + primitives).

### ARCH P1 — Threading & Sync
- [x] Runtimes découpés (`ambre_canvas`/`ambre_logic`/`ambre_sync::run`), launcher `ambre-app`.
- [x] Backpressure (canaux bornés), envoi UI non-bloquant + coalescing, `FatalError`, dispatch typé testé sur hôte (9 tests).
- [x] Pipeline `ambre-crypto` (+ `commitment_tag`) + quarantaine causale par `snapshot_id` (libération `clock` ASC, bornage 900/1000).
- [x] `RestCommand::Unlock` + `SetupAccount` ; `OpenDocument`/`CloseDocument` (multi-document).
- [ ] Vrai WebSocket multiplexé.
- [ ] Boucle Sync non-bloquante (`recv_async` + WS via `JsFuture`/`spawn_local`) : `recv()` bloquant gèle l'event-loop du Worker (`Atomics.wait`).
- [ ] Anti-deadlock Logic <-> Sync : Logic -> Sync bloquant (lossless), Sync -> Logic en `try_send` (sur `Full` : stop lecture WS -> backpressure TCP + `RequestMissing`).
- [ ] Séparer les classes de trafic UI -> Logic : viewport lossy (drop + coalescing) vs édition/service lossless (échec synchrone au call-site sur `Full`).
- [ ] Consommer `RequestId` côté UI : `RequestRegistry<Tag>` générique dans `ambre-canvas` (route `UiEvent::Rest`, timeout/retry ; `RequestId::next()` seul allocateur).
- [ ] Hook `ambreFatal(message)` dans `panic::set_hook` (avant l'abort) : surface UI du panic non gracieux.
- [ ] `SnapshotRequired` -> `SnapshotNeeded` -> `SendSnapshot`. `MockBlindRelay`.
- [ ] Retirer l'échafaudage smoke-test (`IPC_TEST_DOC_ID`, `DocState::Trame` pré-seedé, `pending_doc_types`) à l'arrivée du flux open-document réel.

### ARCH P2 — Applications pilotes
- [x] `ambre-trame` : `TrameState` + `TrameView` (impl `DocView`), grille virtual-scroll, smoke-test viewport.
- [x] Migration UI hybride : `ambre-commands` + `ambre-chrome` créés et montés par `ambre-app` ; framework widget maison retiré, `ambre-canvas` slimmé au runtime de rendu.
- [x] Édition Trame : `TrameState::handle_command` retourne `TrameOutcome { display, update, ephemeral }`, éclaté par le dispatcher Logic (chemin lossless ; subsume l'export snapshot hors chemin chaud).
- [ ] Extraire le chrome spécifique Trame d'`ambre-app` vers un module `chrome` d'`ambre-trame`.
- [ ] Bascule `DocView` -> `AppView` (déplacer `FrameHost` hors d'`ambre-canvas`), au multi-document/onglets.
- [ ] `ambre-prose` (LoroText).

### ARCH P3 — Blind Relay
- [ ] `ambre-server` : routes REST, WS upgrade, ACL, Lua Redis. PostgreSQL/Valkey/MinIO. Snapshot via S3. Tests e2e.

### ARCH P4 — Finalisation & Optimisation
- [ ] Moteur formules Trame complet (DAG, recalcul incrémental, cycle detection).
- [ ] Persistance Loro locale (IndexedDB) + durabilité offline : persister l'état Loro, compacter la file sortante hors-ligne, flush + `RequestMissing` au reconnect ; badge passif alimenté par `ConnectionStatus`.
- [ ] Rotation de Document Key (15 étapes), CRUD/partage/médias complets. Benchmarks hot paths. Audit ZK (`cargo tree`).
- [ ] Audit warnings wasm-only (`-A warnings` masque dead-code sur `ambre-app`/`ambre-canvas::host`/`ambre-sync`) ; confirmer morts `CanvasRenderingContext2d`/`ImageData`.

### ARCH P5 — Applications restantes
- [ ] `ambre-toile` (R-Tree + embarqués read-only), `ambre-murmure` (channels, segments gérés par Sync).
- [ ] Composition (récursion `AppCommand`/`TrameOutcome` routée par `Uuid`). Présence chiffrée. TOFU UI. Meetings (signaling WebRTC).
