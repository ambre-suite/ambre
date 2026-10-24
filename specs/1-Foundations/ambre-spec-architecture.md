# AMBRE -- FEATURE: THE THREE-THREAD STACK

**Type:** Horizontal (framework)
**Implementation:** partial -- the two instances, the six queues and the crate graph all ship.
The failure path and the tab model are held by construction alone.

> How the client is split, and why. Three execution contexts, two WebAssembly instances, one crate
> graph that carries the separation and one page that boots them.

This document owns the stack. What travels on it is `ambre-spec-message-bus.md`, what may never
leave it is `ambre-spec-zero-knowledge.md`, and what each part costs is `ambre-spec-budgets.md`.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `ARCH`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ARCH-002` | `INV` | The crate graph MUST stay acyclic. | this spec section 2 | crate-graph -- a cycle between two workspace crates fails to resolve, so the edit that would create one never builds | none | verified |
| `ARCH-004` | `INV` | Every cryptographic operation MUST run in the Sync worker. | README pillar "Zero-Knowledge"; `ambre-spec-zero-knowledge.md`, `ZK-002` | cargo-deny -- `deny.toml` grants `ambre-crypto` to `ambre-sync`, and to `ambre-logic` for one host-only fixture hash, so every other crate fails the CI job; `ambre_logic::dispatch::tests::a_sealed_export_hands_the_plaintext_to_sync_and_nothing_to_the_ui` -- the one Logic gesture that needs a seal hands it to the Sync worker | none | verified |
| `ARCH-012` | `INV` | The client MUST build against a shared memory with atomics enabled. | this spec section 2 | target-build -- the flags are scoped to the wasm target in `.cargo/config.toml`, and the threading runtime fails to link without them | none | verified |
| `ARCH-001` | `INV` | The client MUST run as two WebAssembly instances: one holding the UI and Logic threads over a single shared heap, one holding the Sync worker with its own memory. A macro sandbox is a frame outside both, and it holds none of their memory. | this spec section 2; `1-Foundations/ambre-spec-macros.md section 3` | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a page that loads other than two Rust artefacts, an `ambre-sync` that builds no `cdylib`, a crate that depends on `ambre-sync`, a worker spawned outside `platform.rs` and a created frame with no `sandbox` | none | verified |
| `ARCH-003` | `INV` | The Logic thread MUST NOT declare a dependency on the render runtime or on the interface toolkit. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a line of `ambre-logic/Cargo.toml` that declares `ambre-canvas`, `wgpu`, `leptos` or another render or toolkit crate; cargo-deny -- `deny.toml` grants `wgpu` to `ambre-canvas` and `ambre-app`, and the CI `deny` job fails any other crate that takes it | none | verified |
| `ARCH-005` | `INV` | Every network call of the two instances MUST leave from the Sync worker. A macro sandbox calls on its own, held by the browser to the domains its user approved. | this spec section 2; `ZK-001`; `1-Foundations/ambre-spec-macros.md section 3` (`MACRO-010`) | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a transport crate declared, or a network call written, outside the worker and the relay. The launcher's two reads of `UI-THREAD-004` are its stated exception | none | verified |
| `ARCH-006` | `INV` | The wire protocol types MUST have one definition, shared by the worker and the relay. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a type of `ambre-protocol` redeclared in `ambre-sync` or `ambre-server` | none | verified |
| `ARCH-007` | `INV` | The Main thread MUST drain the Logic to Sync queue once per frame, before it draws. | this spec section 2 | `ambre_canvas::frame::tests::each_tick_relays_once_before_it_draws`; ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a frame loop that does not run `pre_frame` as the relay of `drive_frame`, and a `pre_frame` that does not drain `logic_to_sync_rx` | none | verified |
| `ARCH-010` | `INV` | The engine MUST be created once per session. | this spec section 2 | `ambre_app::route::tests::path_doc_roundtrip`; ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a launcher that starts the Logic runtime other than once | none | verified |
| `ARCH-016` | `INV` | The engine MUST survive every route change. | this spec section 2 | `ambre_app::route::tests::a_route_walk_reuses_the_one_engine` -- each route change is a close, an open and a show; `::cross_kind_open_needs_reload` -- the one change allowed to end it | none | verified |
| `ARCH-011` | `INV` | Each browser tab MUST run a complete client of its own. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a line that names a `SharedWorker` or a service worker, and a worker spawned outside `platform.rs` | none | verified |
| `ARCH-014` | `INV` | The client MUST NOT rely on a queue disconnection to detect a thread failure. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a panic hook in `ambre-app/src/lib.rs` that does not call `ambre_canvas::signal_fatal`, and a Sync worker whose error event in `platform.rs` does not call it | none | verified |
| `ARCH-018` | `INV` | The launcher MUST hold integration and the shell alone: the logic of an application, what one of its commands does included, MUST live in that application's crate. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, which refuses a line of `ambre-app` naming a variant of an application's command or event, or one of its command ids | none | verified |
| `ARCH-017` | `INV` | The Logic thread MUST answer a view once per pass over the commands waiting on its inbound queues, not once per command. | this spec section 2 | `ambre_logic::pass_tests::a_burst_of_keystrokes_is_answered_once` | none | verified |
| `ARCH-027` | `INV` | The Logic thread MUST hand local persistence the delta an edit wrote, and a whole snapshot only when no snapshot of this session holds the log, the log grew past its bound, or the document closes. | this spec section 2 | `ambre_logic::pass_tests::a_burst_of_keystrokes_is_answered_once`; `ambre_logic::pass_tests::the_first_save_is_a_snapshot`; `ambre_logic::pass_tests::a_long_log_is_folded_into_a_snapshot`; `ambre_logic::pass_tests::a_document_closed_in_the_pass_keeps_its_last_edit` -- the close | none | verified |
| `ARCH-030` | `INV` | A document whose engine holds unfinished work MUST take one bounded step of it per pass, without waiting for a message. | this spec section 2; `TOILE-IO-028` | `ambre_logic::pass_tests::a_large_import_lands_over_several_passes` | none | verified |
| `ARCH-031` | `INV` | The Logic thread MUST finish a document's unfinished work before it merges a remote delta into that document or closes it. | this spec section 2; `TOILE-IO-028` | `ambre_logic::pass_tests::a_board_closed_mid_import_saves_it_whole`; `ambre_logic::pass_tests::a_remote_delta_mid_import_lands_the_import_first` | none | verified |
| `ARCH-020` | `INV` | The Main thread MUST NOT take a lock on a queue it shares with the Logic thread. | this spec section 2 | cargo-deny -- `deny.toml` bans the queue crate that locks from every client crate; lint-wall -- `crates/ambre-bus/clippy.toml` refuses a mutex, a read-write lock, a condition variable and a barrier in the crate that holds the queues | none | verified |
| `ARCH-021` | `INV` | A queue end the Main thread holds MUST NOT wait. | this spec section 2 | type-level -- `ambre_bus::conduit::Sender` and `ambre_bus::conduit::Receiver`, the two ends the launcher keeps, declare only operations that try, and the one end that parks goes to the Logic thread at spawn | none | verified |
| `ARCH-032` | `INV` | Every crate of the workspace MUST inherit the workspace lint table. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a root manifest without `[workspace.lints]` and a member that does not set `[lints]` `workspace = true` | none | verified |
| `ARCH-028` | `INV` | Code outside a test MUST NOT hold a path that panics: no unwrap, expect, panic, unreachable, todo, unimplemented, index or slice that can fail. | this spec section 2 | lint-wall -- the workspace lint table denies `unwrap_used`, `expect_used`, `panic`, `unreachable`, `todo`, `unimplemented` and `indexing_slicing`, and every crate inherits it (`ARCH-032`). `just lint` reads the host with every feature on, and `just lint-wasm` the wasm-only code, in the CI `lint` and `test-wasm` jobs. A boot site that cannot recover carries an `expect` attribute with its reason | none | verified |
| `ARCH-033` | `INV` | A field, parameter, return, constant, static or alias outside a test whose type holds a bare primitive MUST be typed, or named by one line of the primitive allowlist with one category of its closed set. The set MUST NOT hold a category for a value still to type. | `ambre-spec-testing.md section 2`, the primitive gate | `ambre_primitives::allowlist::tests::a_site_no_line_names_fails_the_gate`; `::a_category_outside_the_closed_set_is_refused`; `::a_debt_line_is_refused`; `ambre_primitives::scan::tests::every_kind_of_site_is_read`; `::a_module_file_resolves_where_rustc_finds_it`; `::a_file_no_module_reaches_fails_the_scan`; ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job. A site a macro writes, and a local binding, are outside its reach | none | verified |
| `ARCH-034` | `INV` | The primitive allowlist MUST NOT hold a line that names no site. | `ambre-spec-testing.md section 2`, the primitive gate | `ambre_primitives::allowlist::tests::a_line_that_names_no_site_is_stale`; `::a_prune_removes_the_stale_lines_and_nothing_else`; `::a_prune_adds_no_line_for_a_new_site`; ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job | none | verified |
| `ARCH-035` | `INV` | Each line of the primitive allowlist MUST carry the category true of its site. A `local` line MUST meet the four conditions of `ai/ambre-rust-style.md`. | `ambre-spec-testing.md section 2`, the primitive gate | human-review -- the reviewer reads every line a change adds to `specs/1-Foundations/ambre-primitives.tsv`, against the site its key names | unheld -- `domain-meaning` -- the gate checks that the word is one of the set, never that it is true of the site | human-review |
| `ARCH-013` | `CONC` | Two tabs on one document MUST converge as two users do. | this spec section 2 | `ambre_logic::authorship_tests::two_tabs_of_one_user_converge_as_two_users_do` -- two sessions of one user, one cell edited at once; `ambre_sync::updates::tests::each_open_draws_a_new_peer_and_a_repeated_open_keeps_it` -- each session its own PeerID; `ambre_server::ws::pubsub_hub::tests::a_frame_from_one_instance_reaches_a_subscriber_of_the_other` -- the relay fans out per connection, never per user | none | verified |

### 1.2 A thread fails

Something unrecoverable happens on one of the three. The two rules below are what stands between
that and a page that looks alive and answers nothing.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ARCH-009` | `POST` | A thread that detects an unrecoverable condition MUST emit a fatal event before it exits. | this spec section 2 | `ambre_logic::a_logic_thread_that_cannot_wait_emits_a_fatal_event`; `ambre_toile::ui::view::view_tests::a_fatal_event_latches_the_page_and_silences_the_view` | none | verified |
| `ARCH-008` | `POST` | A thread failure MUST reach the user out of band. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_fatal_event_latches_the_page_and_silences_the_view`; `::a_vanished_logic_thread_latches_the_page` -- the page's fatal latch is set; `ambre_canvas::frame::tests::a_fatal_process_paints_its_screen_once_then_idles` -- the terminal screen | none | verified |
| `ARCH-029` | `FAIL` | A reader fed by the relay or by a user's file MUST refuse a malformed input as a typed error, never a panic. | this spec section 2; `SRV-100` | cargo-fuzz -- the seven targets under `fuzz/fuzz_targets/`, run by the CI `fuzz` job: the wire decode, the `.ambre` container, the formula parser, the Markdown reader, the sealed open path, and the Trame and Toile file readers | none | verified |
| `ARCH-015` | `FRAME` | A failing thread MUST NOT write to the document on its way out. | this spec section 2; derived | `ambre_logic::a_logic_thread_that_cannot_wait_emits_a_fatal_event` -- nothing reaches the relay or local persistence; ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a client build that does not abort on a panic, and a destructor in the Logic thread, an engine or the Sync worker | none | verified |

### 1.3 A message crosses a queue

The UI thread sends a command while the Logic thread is parked. Or the Logic thread answers into a
queue the relay has not drained yet, or the Sync worker awaits its own.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ARCH-022` | `POST` | A message sent to the Logic thread MUST wake it when it is parked. | this spec section 2 | `ambre_bus::conduit::tests::a_send_wakes_the_parked_worker` | none | verified |
| `ARCH-023` | `POST` | A send from the Logic thread on a full queue MUST complete once the Main thread takes a message, in order and with nothing dropped. | this spec section 2 | `ambre_bus::conduit::tests::a_full_send_resumes_when_the_receiver_takes_one` | none | verified |
| `ARCH-024` | `POST` | The last sender of a queue into the Logic thread, once dropped, MUST wake it and read as a disconnection after the messages it queued. | this spec section 2 | `ambre_bus::conduit::tests::the_last_sender_dropped_wakes_the_parked_worker`; `::the_last_sender_dropped_disconnects_after_its_messages` | none | verified |
| `ARCH-026` | `FRAME` | A message crossing a queue MUST NOT overtake one queued before it. | this spec section 2 | `ambre_bus::conduit::tests::messages_arrive_in_order` | none | verified |
| `ARCH-025` | `POST` | A message pushed onto a queue of the Sync worker MUST wake the task awaiting it. | this spec section 2 | `ambre_bus::conduit::tests::a_push_wakes_the_awaiting_task` | none | verified |

### 1.4 A document closes during a pass

The user types, then closes the document before the Logic thread has answered the keystrokes.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ARCH-019` | `FRAME` | Closing a document MUST NOT drop the snapshot an edit earlier in the same pass is owed. | this spec section 2 | `ambre_logic::pass_tests::a_document_closed_in_the_pass_keeps_its_last_edit` | none | verified |

* **Budget.** the whole of `ambre-spec-budgets.md`: this stack is what every row of that register
  measures. Its own two rows, the queue hop and the worker hop of `section 1`, are not measured.
* **Boundary.** none of its own: `ARCH-004` and `ARCH-005` are this document's side of `ZK-002` and
  `ZK-001`.
* **Verification.** 34 `verified`, 1 `human-review`. The build refuses a cycle, a misplaced
  cryptographic dependency, a missing atomics flag and a lock on a client queue. It refuses
  application logic in the launcher too, a failure path that waits on a queue, and a write on a
  failing thread's way out. Host tests hold the Logic pass, its unfinished work, the wake, the full
  queue, each queue's disconnection and two tabs of one user. Seven fuzz targets read random bytes
  from the relay or a file, a gate ties each primitive to a line, and a reviewer its category.

---

## 2. MODEL

**CRDT containers.** none. The stack carries documents; it stores none.

**Two instances, three threads.**

| Context | Where it runs | What it owns |
|---|---|---|
| UI thread | the page's own thread | drawing, input, accessibility, the interface, and the relay between Logic and Sync |
| Logic thread | a worker sharing the Main instance's heap | the documents, the engines, the dispatch |
| Sync worker | a second instance, with its own memory | the socket, the requests, the cryptography, the compression |

A document with macros adds a fourth context: a sandboxed frame on a domain of its own, outside both
instances. It speaks to Logic by message only (`ambre-spec-macros.md section 3`). It holds no key,
no document and no memory of the client, so the split above is unchanged.

```mermaid
flowchart TB
    subgraph MAIN["MAIN INSTANCE (one shared heap)"]
        direction TB
        UI["UI THREAD<br/>page main thread"]
        LOGIC["LOGIC THREAD<br/>worker, shared heap"]
        UI -->|"queue: LogicCommand"| LOGIC
        LOGIC -->|"queue: UiEvent"| UI
    end
    SYNC["SYNC WORKER<br/>second instance, own memory"]
    RELAY["BLIND RELAY<br/>native server"]
    UI -->|"postMessage, relay: SyncCommand"| SYNC
    SYNC -->|"postMessage, relay: LogicEvent"| UI
    SYNC -->|"binary WebSocket"| RELAY
    RELAY -->|"server messages"| SYNC
```

**Why the split is not uniform.** The UI to Logic link carries scroll and typing, so it keeps shared
memory and moves typed values with no encoding. The Logic to Sync link carries network traffic,
where the network already dominates the latency. So it pays one encode and one copy, and buys the
worker a free event loop for its socket. A worker that shares the Main heap cannot have both: it
must either block its own stack to stay alive, or return and be collected.

**What the page creates at boot**, in order:

1. The Logic thread's doorbell, and six bounded queues, each of capacity 1024. Two run between
   UI and Logic and two between Logic and Sync. One carries document snapshots and deltas from Logic
   to the Main thread for the local store. One carries the content event stream on to the active view.
2. The Logic thread, as a worker over the same heap, holding the doorbell, the two receivers and
   the three senders it owns. It binds the doorbell before it first looks at a queue.
3. The Sync worker, as a separate artefact loaded through its own loader, so it is an ordinary
   browser worker rather than a thread.
4. The render loop on the Main thread. Each frame, before drawing, it drains the Logic to Sync
   queue and posts each message to the worker. Then it drains the events for the views, drains the
   persistence queue into the local store, and polls the route.

An embedded child adds one more queue, so its event stream never reaches its parent's view.

**The queues take no lock.** The browser refuses `Atomics.wait` on the page's main thread, and a
lock that finds itself contended ends in one. So every queue of the Main instance stands on a
bounded queue that takes no lock (`ARCH-020`), `crossbeam-queue`'s `ArrayQueue`. It is pure Rust,
with no dependency but its own utilities. The ends the Main thread holds only try, to push or to pop
(`ARCH-021`). A push that finds a slot still being written by the other thread spins for the few
instructions that write takes, and never sleeps.

**The Logic thread parks on a doorbell.** Every queue into it rings the bell on a push, and on
the drop of its last sender (`ARCH-022`, `ARCH-024`). Ringing unparks the thread, which is an
`Atomics.notify` and never waits. The thread parks only once both inbound queues are empty. A
ring that lands between its last look and its park is not lost: the park returns at once.

**The Sync worker awaits its queues.** It has one thread and an event loop, so it may not park. Its
queues have no bound, `crossbeam-queue`'s `SegQueue`, and its loop awaits them as a task. A push
wakes that task through an atomic waker, which takes no lock either (`ARCH-025`).

**Backpressure.** The Logic thread may block on a full outbound queue, which is the intended
signal: it parks, and the relay's next pop wakes it (`ARCH-023`). The UI thread never blocks: it
drops a lossy message and holds a lossless one for a later frame. The two classes and what each owes
are `ambre-spec-message-bus.md`.

**The Logic loop.** It parks while no message is queued and no work is unfinished (`ARCH-030`),
then applies every message already queued on both queues. Each document the pass wrote is owed
its layout and its save, and gets them once when the pass ends (`ARCH-017`). A burst of keystrokes
costs one layout, so the lag stays one pass whatever the typing rate. A pass takes only what waited
when it began, so a sender that never pauses cannot hold the answer back. A document closed during
the pass is answered first, so its last edit is saved (`ARCH-019`).

**The save.** The layout leaves first, since the keystroke waits on it. The save is the delta each
edit wrote, logged after the document's snapshot. A whole snapshot costs an export of the document.
So it is taken only to start the log, to fold it past 256 deltas, and at close (`ARCH-027`).

**Failure, in three paths.**

| Path | What happens | What the user gets |
|---|---|---|
| graceful | the thread emits a fatal event, then exits | the view latches an inert state and stops sending |
| a panic | the build aborts rather than unwinding, so no value is dropped and no queue ever reports a disconnection | the panic hook calls the page's fatal hook before the abort, and a terminal screen replaces the content |
| the Sync worker fails | its trap reaches the page as the worker's error event, and no queue closes | the error handler calls the page's fatal hook, and the same terminal screen replaces the content |

No path writes the document on its way out (`ARCH-015`). The abort runs no destructor, and the
graceful path emits its event before any state exists to flush.

The second path is why `ARCH-008` exists. A leaked sender lives in shared memory forever, so the
receiving side reads an empty queue rather than a closed one. A disconnection therefore means a
clean shutdown, and never a crash.

**One tab, one client.** Each tab instantiates the whole stack. Two tabs on one document are two
independent peers (`ARCH-013`). They converge through the relay, exactly as two users do. Each tab
sees its own edit at once, and the other tab's when the relay delivers it.

**No two-peer table here.** This document holds no document state of its own: what converges is each
application's, and the scenario tables are theirs.

**The crate graph is the boundary.** There is one binary per instance and no runtime check, so the
separation is carried by what each crate is allowed to declare.

| Crate | Depends on | Why the line is there |
|---|---|---|
| `ambre-core` | nothing of ours | the identifiers and the document kinds, shared by the client and the relay |
| `ambre-protocol` | `ambre-core` | the wire types, defined once (`ARCH-006`). Every payload in them is opaque |
| `ambre-crypto` | nothing of ours | the primitives and the key types, reachable from the Sync worker alone (`ARCH-004`) |
| `ambre-bus` | `ambre-core` | the four envelopes and the typed content plane (`ambre-spec-message-bus.md`) |
| `ambre-draw` | geometry, colour and text layout, plus `ambre-core` | the drawing abstraction, with no graphics and no document dependency |
| `ambre-commands` | `ambre-core` | the command identities, a leaf every application and the toolkit depend on |
| `ambre-canvas` | `ambre-draw` plus the graphics stack | the content runtime of the UI thread. It names no application |
| `ambre-chrome` | the page framework, plus `ambre-commands` and `ambre-core` | the interface toolkit. It names no application either |
| each application crate | `ambre-bus`, `ambre-draw`, `ambre-canvas`, `ambre-chrome`, `ambre-commands` | one crate per application, holding its state, its view and its own interface. It knows neither the Logic runtime nor the worker |
| `ambre-logic` | `ambre-bus` plus the application crates | the Logic runtime, pure Rust with no browser glue (`ARCH-003`) |
| `ambre-sync` | `ambre-bus`, `ambre-protocol`, `ambre-crypto`, the transport crates | the Sync worker, and its own entry point |
| `ambre-app` | everything above except `ambre-sync` | the launcher: it mounts the interface, creates the queues, spawns both workers and relays between them. It integrates the applications and holds none of their logic (`ARCH-018`) |
| `ambre-server` | `ambre-core`, `ambre-protocol`, and one signature crate | the relay. It links nothing that could read a document (`ZK-004`) |

**What an application hands the shell.** A `CommandHandler` of `ambre-chrome`, its panels and
dialogs, and a relay (`ARCH-018`). The handler says what a command id does. The relay fills the
panels from each event and turns a panel's intent into a command. The launcher only sends.

Two consequences follow and neither is obvious. The launcher sits **downstream** of every
application, which is what lets the closed view registry name an application view without a cycle.
And the transitive graph does reach the graphics stack from the Logic runtime, through an
application crate. The boundary is carried by the **direct** dependencies and by what the runtime
path calls, never by the transitive closure.

**The toolchain.** The wasm target builds against a shared memory with atomics, bulk memory and
mutable globals, which needs a pinned nightly and a rebuilt standard library. The flags are scoped
to that target, so the relay and the host tests are unaffected. The page must be served
cross-origin isolated for the shared memory to exist at all, and the Sync worker inherits that
property from the page.

**Code.** `crates/ambre-app/src/lib.rs` for the boot and the relay, and `crates/ambre-app/src/host/`
for the frame driver and the view registry. Then `crates/ambre-logic/src/lib.rs` for the Logic
runtime, `crates/ambre-sync/src/worker.rs` for the worker entry, `.cargo/config.toml` and
`Trunk.toml` for
the build.

---

## 3. SURFACE

The stack has no control of its own. What a user meets is what it guarantees them.

| Trigger | Behaviour |
|---|---|
| scrolling or typing | the answer comes from a thread that shares memory with the page, so no encoding sits in the path |
| a remote change arriving | it is opened and decompressed in a second instance, in parallel, so the interface never pauses for it |
| navigating from the home screen to a document | the page never reloads: the engine, its keys and its socket survive the move |
| a thread dying | the content is replaced by a terminal screen instead of freezing in place |
| opening the same document in two tabs | both edit, and both converge |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Shared memory to the Logic thread, message passing to the Sync worker | one uniform transport, every link by message passing and no shared memory anywhere | the two links have different budgets. Scroll and typing cannot afford an encode and a copy; network traffic already waits on the network. Pushing the first link onto message passing would pay serialization on the one path where it is the whole budget. |
| The Sync worker is its own instance | a shared-heap worker for the Sync side, with an asynchronous loop inside it | a shared-heap worker runs to completion: an asynchronous loop returns immediately, its handle is dropped and it is collected after the first poll. A worker cannot both block its stack to stay alive and keep an event loop free for a socket. A second instance has an ordinary event loop and no such conflict. |
| The Logic thread parks on a doorbell of its own | an asynchronous receive across a worker boundary | the asynchronous waker of a queue crate is scheduled on the sending worker, so it never wakes a consumer parked on another one. Parking the thread and unparking it from the sender crosses workers, and the main thread may unpark. |
| Every client queue is our own, over a lock-free queue | `flume`, or any queue crate that wakes its receiver under a lock | its queue sits behind a `std::sync::Mutex`. On the page's main thread a contended one spins briefly, then calls `Atomics.wait`, which the browser refuses: one keystroke probe run in three died on it. Its `spin` feature swaps the wait for a busy loop, which trades the crash for a main thread that can burn a frame on a lock the Logic thread holds. |
| The queues are `crossbeam-queue`'s `ArrayQueue` and `SegQueue` | a ring buffer written by hand over atomics | the bounded ring is the hard part, and it needs `unsafe`, which our code never carries. What is ours is the wake-up and the disconnection, which take atomics and `std::thread` alone. |
| The launcher owns the frame driver and the closed view registry | the view registry in the content runtime, upstream of the applications | a closed registry naming an application view must sit downstream of every application crate, so putting it upstream is the cycle this resolves. |
| One crate per application, deliberately monolithic | a crate per layer, with an application spread across three of them | the state, the view and the interface of one application share its whole vocabulary, and splitting them by layer is what produced the specification corpus this migration is undoing. |
| The Logic runtime is pure Rust with no browser glue | a Logic runtime that reaches the browser directly for storage or timing | it forfeits the whole engine being testable on the host under an ordinary test run, with no browser and no worker. That is a first-order property of the split, not a side effect. |
| The relay is in the same workspace as the client | a separate workspace for the server | the build flags are scoped by target, so a native member is unaffected, and one workspace gives one formatter, one lint pass and one CI job. |
| A panic aborts | unwinding on panic, and detecting the death through queue disconnection | unwinding across a shared heap with a leaked sender leaves the other threads reading a queue that will never close, so the disconnection the detection waits on never comes. Aborting is honest, and the fatal hook is what makes it visible. |
| One binary per instance, not one per application | a separate build per application | the applications share the render runtime, the interface toolkit and the engine, so splitting them would duplicate the heavy half of the download per application. |
| An application hands the shell one trait object | moving each command's logic into a free function of its crate, still called by the shell | the shell would still name each command it routes, so the next command would land in the shell again and nothing would stop it |
| No dependency-injection framework | a container that resolves the graph at run time | the registry is a map and the wiring is explicit in the launcher: two queues, a closure that builds a view, and a worker handle. |
| No fourth thread for import and export | a fourth thread for conversions | during an import the interface shows a waiting state, and a blocked Logic thread is the intended behaviour. The cost of a long conversion is paid by lazy loading, not by another thread. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The panic hook is driven by no test.** A fatal event and a dropped channel reach the page latch
  in a host test. A gate reads that the hook calls the latch, but no test panics a thread in a
  browser.
* **The relay cadence is unmeasured.** `ARCH-007` states one drain per frame ahead of the draw, and
  the budget rows for both hops read that no measurement exists.
* **The route poll runs every frame inside the launcher.** Its decision is `route::step`, which a
  test walks. The effects it applies, the document open and close and the render gate, run only in
  a browser. It exists because the interface framework requires a subtree that the engine handles
  cannot enter.
* **Three files at the workspace root cite a document the corpus does not have.** The build
  configuration, the workspace manifest and the toolchain file each name a migration note that was
  deleted. No resolver opens a file outside the corpus, the crates and the rules.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What travels on the queues, and what a full one means | `ambre-spec-message-bus.md` |
| What the relay may learn, and what may never leave sealed | `ambre-spec-zero-knowledge.md` |
| What every part of the stack is budgeted at | `ambre-spec-budgets.md` |
| How a document stays readable by a later binary | `ambre-spec-schema-version.md` |
| What the UI thread may hold, and what it may open | `ambre-spec-ui.md` |
| How the content is drawn, and what a frame costs | `ambre-spec-frame.md` |
| The interface components, and the shell around the surface | `ambre-spec-chrome.md` |
| The home screen, the routes and the sign-in | `ambre-spec-launcher.md` |
| What the Sync worker does once a message arrives | `ambre-spec-sync.md` |
| What the relay stores, and what it refuses | `ambre-spec-server.md` |
| How Ambre is verified, tier by tier and crate by crate | `ambre-spec-testing.md` |
