# AMBRE -- FEATURE: THE CLIENT MESSAGE BUS

**Type:** Horizontal (framework)
**Implementation:** partial -- the three planes, the four envelopes, the role gate, the lossless
outbox and the lossy gate all ship. The lossy class coalesces by habit and no test drives the
coalescing.

> The contract the three threads talk over. It fixes what a message may carry, who allocates its
> identity, and what happens to it when nobody is listening.

The threads themselves, the queues and the crate graph are `ambre-spec-architecture.md`. This
document owns what travels on them.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `BUS`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-002` | `INV` | A document command MUST reach the Logic thread as a variant of a closed sum, never as an opaque payload. | this spec section 2 | exhaustive-match -- `AppCommand` and `AppEvent` are closed enums, so a missing document type fails the exhaustive `match` in `ambre_logic::data::handle_app_command` | none | verified |
| `BUS-006` | `INV` | The document registry MUST stay flat: an embedded child MUST be addressed by its own identity, in one lookup. | this spec section 2; `ambre-spec-toile-embeds.md`, `TOILE-084` | `ambre_logic::dispatch::tests::an_embedded_child_is_gated_by_its_own_role_not_the_boards`; `::owning_the_board_grants_nothing_on_a_document_embedded_in_it` -- each seeds a parent and a child side by side and asserts the child's role decides, which only holds if the child is its own entry | none | verified |
| `BUS-001` | `INV` | A message on the UI to Logic path MUST travel as a typed Rust value by move: nothing on that path MUST be encoded. | this spec section 2; `ambre-spec-architecture.md section 2` | none | unheld -- the path is a `conduit` queue over the shared heap and `LogicCommand` derives no `Serialize`, so an encoder has nothing to call today. Nothing refuses the derive that would add one; lands with the `The thread-separation belts` Story (`ambre-roadmap.md` E9) | untested |
| `BUS-027` | `INV` | A Murmure command MUST reach the Logic thread as a variant of the content plane, addressed to its channel. | this spec section 2; `BUS-002` | not built | unbuilt -- lands with the `ARCH P7` -- the Murmure engine core Story (`ambre-roadmap.md` E5) | unimplemented |
| `BUS-028` | `INV` | No envelope MUST carry a channel message as opaque bytes. | this spec section 2 | not built | unbuilt -- lands with the `ARCH P7` -- the Murmure engine core Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 The interface sends a command

A view holds a bounded sender and one frame's worth of intent. What the class of the command decides
is what a full queue means.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-012` | `POST` | A lossy command MUST be coalesced to one send per frame. | this spec section 2 | none | unheld -- each view keeps one `pending_viewport` and flushes it from its frame hook, so the coalescing is held by reading the code, and no test drives two requests inside one frame; lands with the `The thread-separation belts` Story (`ambre-roadmap.md` E9) | untested |
| `BUS-017` | `POST` | A stale answer to a lossy request MUST be dropped by its version. | this spec section 2 | none | unheld -- only one application holds it: `TrameUiState::on_viewport_data` compares against `accepted_version`, and every test that drives it feeds an increasing version, so the drop branch runs in no test. Toile's viewport carries no version at all; lands with the `The thread-separation belts` Story (`ambre-roadmap.md` E9) | untested |
| `BUS-011` | `POST` | A lossless command MUST NOT be dropped. | this spec section 2 | `ambre_bus::outbox::tests::a_full_channel_holds_the_command_instead_of_dropping_it`; `::a_flush_after_the_reader_drains_sends_the_held_command`; `::a_held_queue_keeps_the_order_the_user_typed`; `::a_send_while_busy_never_overtakes_a_held_command` | none | verified |
| `BUS-016` | `POST` | A full command queue MUST reach the user as a busy signal. | this spec section 2 | `ambre_canvas::busy::tests::only_the_first_frame_of_an_episode_reports_a_change` -- the publisher reports the edge once per episode, so the shell raises one notice and clears it once; `ambre_bus::outbox::tests::a_full_channel_holds_the_command_instead_of_dropping_it` -- the level the publisher carries; qa-manual -- DOM glue held by using the application (`1-Foundations/ambre-spec-testing.md section 2`), the shell's per-frame read of that level and the notice it raises, reached by typing into a document whose Logic thread is saturated | none | qa-manual |
| `BUS-022` | `POST` | A view MUST stop accepting input while its outbox is busy. | this spec section 2; derived | qa-manual -- DOM glue held by using the application (`1-Foundations/ambre-spec-testing.md section 2`), one early return at the head of each `handle_input`, reached by typing into a document whose Logic thread is saturated. The level it reads is `BUS-016` | none | qa-manual |
| `BUS-024` | `POST` | A lossy command the queue dropped MUST be offered again on a later frame. | this spec section 2; derived | qa-manual -- DOM glue held by using the application (`1-Foundations/ambre-spec-testing.md section 2`), each frame hook putting the dropped request back: Trame into `pending_viewport`, Toile into the same slot and into the set of media already asked for, reached by scrolling a document whose Logic thread is saturated; `ambre_trame::ui::view::tests::a_dropped_request_is_re_offered_not_doubled` -- the slot Trame re-offers from | none | qa-manual |
| `BUS-018` | `FRAME` | A command that failed to send MUST NOT be reflected in the view state. | this spec section 2; derived | qa-manual -- DOM glue held by using the application (`1-Foundations/ambre-spec-testing.md section 2`), one branch at each Trame commit path, read back off the cell after a refused send. The outcome it branches on is `BUS-011` or `BUS-023` | none | qa-manual |
| `BUS-023` | `FRAME` | A command that changes the document MUST NOT travel on the lossy class. | this spec section 2; derived | `ambre_bus::lossy::tests::a_command_that_changes_the_document_never_reaches_the_channel` -- one insert, one move and one resize, and the queue stays empty; `::a_command_of_another_plane_is_refused_with_the_mutations`; `::an_open_channel_takes_a_request_that_changes_nothing` -- the other side of the gate | none | verified |

### 1.3 Logic routes a content command

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-007` | `FAIL` | The Logic dispatcher MUST drop a mutating command when the caller's role on that document is not writable. | `ambre-spec-membership.md`, `SYNC-016` | `ambre_logic::dispatch::tests::viewer_edit_is_dropped_and_emits_no_update` -- no UI event and no outbound update; `::viewer_read_only_command_still_passes`; `::editor_edit_passes_through` -- the two sides the gate must not confuse | none | verified |
| `BUS-005` | `FAIL` | A command for a document absent from the registry MUST be dropped, never synthesised. | this spec section 2 | `ambre_logic::dispatch::tests::a_command_for_an_unopened_document_is_dropped` -- no update reaches Sync and no event reaches the UI; `::app_command_on_unknown_doc_is_dropped` -- the same for a read-only command | partial -- neither test asserts that the registry stayed empty, so the synthesis half is held by the drop alone: an arm that inserts an entry and then drops the command leaves both assertions green; lands with the `Undo and the thread failure paths held` Story (`ambre-roadmap.md` E9) | partial |
| `BUS-008` | `POST` | The role the relay reports MUST supersede the role a freshly opened document assumed. | this spec section 2 | `ambre_logic::dispatch::tests::document_ready_supersedes_the_optimistic_owner_role` -- it reads the optimistic `Owner` off the slot, feeds a `DocumentReady` carrying `Viewer`, and asserts a following edit no longer reaches Sync | none | verified |
| `BUS-010` | `FRAME` | A command handler MUST NOT send on a queue: it returns an outcome and the dispatcher fans it out. | this spec section 2 | none | unheld -- each engine holds it over its own handlers (`TOILE-089`, the Prose row being unbuilt). Nothing holds it for an engine added later: a handler that captures a sender compiles; lands with the `Undo and the thread failure paths held` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 A service request crosses the hops

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-013` | `POST` | The request identity allocator MUST yield a strictly increasing value, unique under concurrency. | this spec section 2 | `ambre_bus::host::request_id_is_strictly_monotonic`; `::request_id_is_unique_under_concurrency` -- eight threads at a thousand ids each, and every one is new | none | verified |
| `BUS-014` | `POST` | A service request identity MUST be echoed unchanged by every hop. | this spec section 2 | `ambre_bus::host::request_id_echo_is_preserved_across_the_hop` -- the id survives the `postcard` codec byte for byte; `ambre_logic::dispatch::tests::rest_command_forwards_to_sync_with_id`; `::rest_event_forwards_to_ui_with_id` -- the dispatcher echoes it in both directions | none | verified |
| `BUS-009` | `POST` | A service request MUST be relayed to the Sync worker field for field. | this spec section 2 | `ambre_logic::dispatch::tests::create_document_preserves_fields` -- both fields survive the arm, and the relay is one arm for the whole family, so what the test pins for one variant is structural for the rest of it | none | verified |
| `BUS-015` | `POST` | A service response MUST be relayed to the interface field for field, unless a recorded route claims it for the document that issued it. | this spec section 2 | `ambre_logic::dispatch::tests::an_unrouted_rest_event_still_forwards_to_the_ui` -- the unclaimed case; `::routed_media_uploaded_lands_in_engine_not_the_ui` -- the claimed case reaches the engine and no stray event reaches the interface | none | verified |
| `BUS-020` | `FAIL` | A service response whose document was closed MUST be dropped, never delivered. | this spec section 2 | `ambre_logic::dispatch::tests::a_late_response_after_close_is_dropped_without_panic` -- the route is gone on close and the late response repaints nothing; `::a_correlated_upload_failure_aborts_the_import_without_a_raw_api_error` -- the same for the error direction | none | verified |
| `BUS-004` | `PRE` | A service request identity MUST be allocated by the UI thread. | this spec section 2 | none | unheld -- nothing could refuse it: `RequestId::next` is a free function over a process-wide counter, so the Sync worker (which holds its own memory and therefore its own counter) would allocate a colliding id in silence. There is no failure to define, only a caller that owes the rule; lands with the `The thread-separation belts` Story (`ambre-roadmap.md` E9) | untested |
| `BUS-019` | `FRAME` | Relaying a service request MUST NOT touch the document registry, save for the two commands that remove a document from it. | this spec section 2 | none | unheld -- `ambre_logic::dispatch::tests::logout_clears_docs_and_forwards` pins the one exception that clears everything, and nothing pins that the other twenty-nine variants leave the registry alone: the hook is one `match` arm away from any of them; lands with the `Undo and the thread failure paths held` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 A message crosses the worker hop

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-003` | `POST` | The Logic to Sync hop MUST return every field it was given: the encode and the decode MUST agree. | this spec section 2 | `ambre_bus::host::sync_command_roundtrips_through_postcard`; `::logic_event_roundtrips_through_postcard` -- the `Debug` rendering is compared before and after | partial -- they cover 5 of the 12 `SyncCommand` variants and 4 of the 19 `LogicEvent` variants, so a variant added tomorrow is covered by nothing; lands with the `The thread-separation belts` Story (`ambre-roadmap.md` E9) | partial |
| `BUS-025` | `POST` | An event from the Sync worker MUST reach the Logic thread in arrival order. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-021` | `ambre_bus::conduit::tests::a_backlog_holds_past_a_full_channel_and_keeps_the_order`; `::a_backlog_never_lets_a_new_value_overtake_a_held_one` | none | verified |
| `BUS-026` | `POST` | An event from the Sync worker MUST NOT be dropped on a full queue. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-021` | `ambre_bus::conduit::tests::a_backlog_holds_past_a_full_channel_and_keeps_the_order` | none | verified |
| `BUS-021` | `FRAME` | The worker hop MUST NOT decode an application payload: a CRDT delta and a channel message MUST cross it as opaque bytes. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-002` | none | unheld -- the envelope types carry `Vec<u8>` and the round trip pins that the bytes come back identical, which says the codec does not touch them, and nothing refuses the arm that would decode one before forwarding; lands with the `The thread-separation belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** two rows of `ambre-spec-budgets.md section 1`: the queue send and receive at
  < 1 us, and the Logic to Sync `postMessage` hop at < 1 ms. Neither is measured.
* **Boundary.** none of its own. The bus carries plaintext and reaches no relay: every byte that
  leaves does so through the Sync worker, sealed. `ZK-002` is what forbids a key type here, and the
  worker hop itself is the accepted internal boundary of `ambre-spec-zero-knowledge.md`.
* **Verification.** 13 `verified`, 2 `partial`, 7 `untested`, 4 `qa-manual`, 2 `unimplemented`. What
  holds is the Logic dispatcher, driven on host through an unknown document, a viewer's edit and a
  child's role. A service round trip, a late response, the lossy gate and the backlog behind a full
  queue are driven too. Loose is the envelope discipline the other two threads owe.

  Five paths are loose because nobody drives them. Three sit in the codec and dispatcher: an encode
  no test names, a stale version, a handler capturing a sender. The fourth is the registry entry no
  test asserts was never created; the fifth is Toile's re-offer of a dropped request. Three rows
  live in the DOM glue, and the doctrine rules them `qa-manual`. They are the busy notice, the input
  gate and the call site that must not write what it failed to send.
---

## 2. MODEL

**CRDT containers.** none. The bus carries commands and events; the document lives on the Logic
thread.

**Three planes, and each is routed differently.**

| Plane | Carried by | Payload | Logic routing |
|---|---|---|---|
| **Content** | `LogicCommand::App` / `UiEvent::{Viewport,Presence}` | `AppCommand` / `AppEvent`, typed and closed | by document identity, to that document's state |
| **Service** | the `Rest { id, .. }` variant of all four envelopes | `RestCommand` / `RestEvent`, defined once | relayed verbatim, plus a state hook for the few that change the registry. Two are served inside Sync and reach no relay: the seal and the open of a file (`DOCFILE-014`) |
| **Network** | `SyncCommand::Send*` / `LogicEvent::Remote*` and the pushes | opaque bytes: CRDT deltas | applied to the document, or forwarded |

**Four envelopes, one per hop.** The direction is carried by the type name, so a `Command` always
travels towards the downstream worker and an `Event` always flows back.

| Flow | Type | Module | Variants |
|---|---|---|---|
| UI to Logic | `LogicCommand` | `logic.rs` | 9 |
| Logic to UI | `UiEvent` | `ui.rs` | 16 |
| Logic to Sync | `SyncCommand` | `command.rs` | 12 |
| Sync to Logic | `LogicEvent` | `event.rs` | 19 |
| shared service plane | `RestCommand` / `RestEvent` / `RequestId` | `rest.rs` | 33 / 34 |

```rust
pub enum LogicCommand {
    App { document_id: DocumentId, command: AppCommand },   // content
    Rest { id: RequestId, cmd: RestCommand },               // service, relayed
    OpenDocument { document_id: DocumentId, doc_type: DocType },
    // control and local restore variants, one per fire-and-forget operation
}
pub enum AppCommand { Trame(TrameCommand), Toile(ToileCommand) } // Murmure(MurmureCommand) is designed
pub enum AppEvent { Trame(TrameEvent), Toile(ToileEvent) }       // Murmure(MurmureEvent) is designed
pub struct RequestId(pub u64);
```

The four envelopes and the service family are the contract. The variant lists are not restated
here, because a second copy of thirty variants is a copy that drifts. They live in `crates/ambre-bus/src/`,
one module per row of the table above.

**What stays on an envelope, and what joins the service family.**

| Shape | Where it goes |
|---|---|
| a correlated request with a success answer: session, document and channel management, sharing, media, a local seal or open | `RestCommand` and `RestEvent`, one variant each |
| a control operation with no answer: open, close | an explicit envelope variant |
| a Murmure message, reaction or edit | a content-plane command addressed to its channel (`BUS-027`) |
| a server push: a remote change, a connection status, a revocation, a meeting event | an explicit envelope variant |
| a failure: an authentication error, an interface error | an explicit envelope variant, correlated by identity only where a route recorded one |

**The content plane is a closed world.** Adding a document type is one variant in `AppCommand` and
one in `AppEvent`, and the dispatcher stops compiling until the new arm is written. Nesting adds
nothing. An embedded child is an ordinary entry of the flat registry, addressed by its own identity
(`BUS-006`). The view layer nests while the data layer stays flat. What crosses into a focused
child is a raw input event, never a command, because a parent view cannot name a child's view type.

**Per-app modules, envelopes by direction.** The transport envelopes stay split by direction: each
is bound to one hop. The typed content plane is grouped one module per document type under `app/`.
A document type's command enum, event enum and value types sit together. Everything is
re-exported flat at the crate root, so a downstream crate never names the internal layout.

**The role gate.** Each open document carries this client's role on **that** document. The
dispatcher drops a mutating command when the role is not writable (`BUS-007`). A freshly opened
document assumes the owner role so edits route before the members answer arrives, and the real role
supersedes it on `DocumentReady` (`BUS-008`). The gate is comfort, never security: the relay
re-checks every write, and the client is the attacker's own machine.

**Two traffic classes.** They share the same bounded queues and differ in what a full queue
means.

| Class | Examples | On a full queue |
|---|---|---|
| lossy | a viewport request, a hit test | drop it: the next frame offers the request again, and a stale answer is dropped by its version |
| lossless | an edit, a selection, a paste, an undo group boundary | never drop it: surface a busy signal and stop accepting input |

Each class is one function, and the function decides what a full queue does. `Outbox::send` holds
a lossless command for a later frame (`BUS-011`). `send_lossy` drops a lossy one, and refuses a
command that changes the document, because the lossy class carries no intent the user gave
(`BUS-023`). The caller reads the outcome: it writes nothing into the view for a command that did
not leave (`BUS-018`), and re-offers a dropped request (`BUS-024`).

**The Sync events are lossless too.** A delta or a service answer arrives through the worker's
`postMessage` handler on the Main thread, which may not wait for room (`ambre-spec-architecture.md`,
`ARCH-021`). What the queue to the Logic thread refuses waits in a backlog, in arrival order
(`BUS-025`, `BUS-026`). The relay offers it again each frame before anything newer. Past a high
mark it asks the worker to stop reading the relay, and at a low mark to start again
(`ambre-spec-sync.md`, `SYNC-059`). The relay holds what was not read, so the backlog stays bounded
and loses nothing.

**Code.** `crates/ambre-bus/src/` for the contract, `crates/ambre-logic/src/dispatch.rs` for the
routing, `crates/ambre-logic/src/data.rs` for the role gate.

---

## 3. SURFACE

The bus has no control of its own. What a user meets is what it does under load.

| Trigger | Behaviour |
|---|---|
| scrolling faster than the Logic thread answers | requests are dropped and the last one wins; the view never lags behind by more than a frame |
| a document opened from the home screen | edits route immediately, and the permission arrives a beat later |
| editing a document shared read-only | the command is dropped locally, so no failed write reaches the relay |
| a saturated Logic thread | the edit is held rather than dropped, the view stops taking input, and a notice says the document is catching up until the backlog clears |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A closed sum per direction, one variant per document type | a trait object plus a registry of boxed controllers | it re-creates a crate cycle, because the sum resolves downstream of every application crate, and it gives up the exhaustive dispatch the compiler proves today |
| A closed sum per direction, one variant per document type | an opaque byte payload decoded by the receiver | the dispatch becomes a run-time match on bytes: adding a document type still builds, and the missing arm is found when a message arrives rather than at compile time |
| The service family is defined once and carried by a thin envelope | four nearly identical enums, one per direction | modelling each service operation as a variant in each of the four directions is about three hundred lines of one-to-one copying, and the failure mode is a field copied into the wrong place. Defined once, the whole service dispatch is a single arm |
| The request identity is allocated by the UI thread | allocating the identity in the Logic thread | it forces a second correlation table, where the caller is the one that needs to recognise its own answer and shared memory makes a single atomic counter legal across the two threads |
| The request identity is allocated by the UI thread | correlating by operation shape | two identical requests in flight are indistinguishable, and the pairing is wrong exactly when the interface is busiest |
| The registry is flat and nesting is a view concern | a tree of documents, with a child reached through its parent | it needs a depth bound, an unwrapping path and a rule for a cycle. A flat map needs one lookup, and the nesting the user sees is the view owning a child view |
| The role gate is comfort, and the relay stays the authority | trusting the client gate | the client is the attacker's own machine, so a gate there proves nothing |
| The role gate is comfort, and the relay stays the authority | offering every edit and letting the relay refuse | the interface would offer an edit the relay rejects, and the user learns it after the act rather than before |
| Nothing is encoded between the UI and the Logic threads | one encoding for every hop, so that all four queues look alike | that path carries scroll and typing, where latency is the whole budget, and the two threads share one heap. Encoding it would buy isolation the crate graph already gives |
| The envelopes group by direction, the content plane by application | grouping every type by direction, including the per-application value types | a document type's command and its event share the whole value vocabulary, so splitting them by direction scatters each application over two modules. An envelope belongs to one hop, which is why direction is the right axis for it and the wrong one below it |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The two view halves of the lossless class run in no test.** A host test drives the outbox. The
  branches that read it compile only under `wasm32`, where no test builds a view. Those are the
  input gate (`BUS-022`) and the commit that must not write a refused edit (`BUS-018`).
* **Nothing refuses a lossless send routed around the outbox.** Each view still holds a raw
  `ambre_bus::conduit::Sender`, which the lossy class and the session commands need. A call site
  added tomorrow can `try_send` a lossless command on it, and drop it silently. The
  outbox holds `BUS-011` for every caller that goes through it, and no type refuses the caller that
  does not.
* **The lossy gate refuses a mutation, and nothing more.** Presence and view state are not mutating,
  so a selection offered to `send_lossy` passes the gate and a full queue drops it. No call site
  does so today, and the class table is what says it must not (`BUS-023`).
* **The hold queue has a bound, and past it the command is refused.** Sixty-four commands, after
  which the outbox reports a refusal rather than growing without limit. The input gate closes on the
  first held command, so reaching the bound takes a saturation no test has produced.
* **Nothing refuses an encoder on the UI to Logic path.** `BUS-001` is held by nobody calling one.
* **The postcard round trip is a sample.** Twelve variants of one envelope and nineteen of the other
  are covered by nine cases between them, and no test enumerates the variants (`BUS-003`).
* **A stale viewport answer is dropped in one application.** Trame compares an incoming version
  against the last one it accepted. Toile's viewport request carries no version, so a late answer is
  applied whenever it lands (`BUS-017`).
* **The channel variants of the code carry opaque bytes.** Both envelopes still hold them. The design
  moves a channel onto the content plane (`BUS-027`, `BUS-028`). That is the
  `ARCH P7` Story (`ambre-roadmap.md` E5).
* **Nothing bounds how long a service request may stay outstanding.** A request whose answer never
  arrives leaves its route recorded, and nothing times it out.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The threads, the queues and the crate graph | `ambre-spec-architecture.md` |
| What the UI thread may hold, and what it may open | `ambre-spec-ui.md` |
| What the Sync worker does with a command once it arrives | `ambre-spec-sync.md` |
| How a payload is sealed before it leaves | `ambre-spec-sealed-envelope.md` |
| Where a role comes from, and who may change it | `ambre-spec-membership.md` |
| How each engine handles a command and builds its outcome | each application's engine document |
| What a document contains, and how it is versioned | `ambre-spec-schema-version.md` |
| What each hop is budgeted at | `ambre-spec-budgets.md` |
