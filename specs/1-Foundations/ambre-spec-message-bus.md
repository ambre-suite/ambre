# AMBRE -- FEATURE: THE CLIENT MESSAGE BUS

**Type:** Horizontal (framework)
**Implementation:** partial -- the three planes, the four envelopes and the role gate all ship; the
lossless class is designed and both applications drop an edit instead.

> The contract the three threads talk over. It fixes what a message may carry, who allocates its
> identity, and what happens to it when nobody is listening.

The threads themselves, the channels and the crate graph are `ambre-spec-architecture.md`. This
document owns what travels on them.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `BUS`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `BUS-002` | `INV` | A document command MUST reach the Logic thread as a variant of a closed sum, never as an opaque payload. | this spec section 2 | the compiler: `AppCommand` and `AppEvent` are closed enums, so a missing document type fails the exhaustive `match` in `ambre_logic::data::handle_app_command` | verified |
| `BUS-006` | `INV` | The document registry MUST stay flat: an embedded child MUST be addressed by its own identity, in one lookup. | this spec section 2; `ambre-spec-toile-embeds.md`, `TOILE-084` | `ambre_logic::dispatch::tests::an_embedded_child_is_gated_by_its_own_role_not_the_boards`, `::owning_the_board_grants_nothing_on_a_document_embedded_in_it` -- each seeds a parent and a child side by side and asserts the child's role decides, which only holds if the child is its own entry | verified |
| `BUS-001` | `INV` | A message on the UI to Logic path MUST travel as a typed Rust value by move: nothing on that path MUST be encoded. | this spec section 2; `ambre-spec-architecture.md section 2` | none. The path is a `flume` channel over the shared heap and `LogicCommand` derives no `Serialize`, so an encoder has nothing to call today. Nothing refuses the derive that would add one | unverified |

### 1.2 The interface sends a command

A view holds a bounded sender and one frame's worth of intent. What the class of the command decides
is what a full channel means.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `BUS-012` | `POST` | A lossy command MUST be coalesced to one send per frame. | this spec section 2 | none. Each view keeps one `pending_viewport` and flushes it from its frame hook, so the coalescing is held by reading the code; no test drives two requests inside one frame | unverified |
| `BUS-017` | `POST` | A stale answer to a lossy request MUST be discarded by its version. | this spec section 2 | none, and only one application holds it. `TrameUiState::on_viewport_data` compares against `accepted_version`; every test that drives it feeds an increasing version, so the discard branch runs in no test. Toile's viewport carries no version at all | unverified |
| `BUS-011` | `POST` | A lossless command MUST NOT be dropped. | this spec section 2 | not built -- lands with `A lost edit is never shown as applied` (`ambre-roadmap.md` E7). Toile's `send_edit` treats a full channel as a warning; Trame returns the error and both call sites log it, one of them with the group boundary discarded outright | unimplemented |
| `BUS-016` | `POST` | A full command channel MUST reach the user as a busy signal, and the view MUST stop accepting input until it clears. | this spec section 2 | not built -- lands with `A lost edit is never shown as applied` (`ambre-roadmap.md` E7). No path in either application surfaces the condition or stops accepting input | unimplemented |
| `BUS-018` | `FRAME` | A command that failed to send MUST NOT be reflected in the view state. | this spec section 2; derived | not built -- lands with `A lost edit is never shown as applied` (`ambre-roadmap.md` E7). `commit_edit` logs the full channel and then writes the same text into its own grid state unconditionally, so the user reads an edit the document never received | unimplemented |

### 1.3 Logic routes a content command

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `BUS-007` | `FAIL` | The Logic dispatcher MUST drop a mutating command when the caller's role on that document is not writable. | `ambre-spec-membership.md`, `SYNC-016` | `ambre_logic::dispatch::tests::viewer_edit_is_dropped_and_emits_no_update` asserts no UI event and no outbound update; `::viewer_read_only_command_still_passes` and `::editor_edit_passes_through` pin the two sides the gate must not confuse | verified |
| `BUS-005` | `FAIL` | A command for a document absent from the registry MUST be dropped, never synthesised. | this spec section 2 | `ambre_logic::dispatch::tests::a_command_for_an_unopened_document_is_dropped` asserts no update reaches Sync and no event reaches the UI; `::app_command_on_unknown_doc_is_dropped` repeats it for a read-only command. Neither asserts that the registry stayed empty, so the synthesis half is held by the drop alone | verified |
| `BUS-008` | `POST` | The role the relay reports MUST supersede the role a freshly opened document assumed. | this spec section 2 | `ambre_logic::dispatch::tests::document_ready_supersedes_the_optimistic_owner_role` -- it reads the optimistic `Owner` off the slot, feeds a `DocumentReady` carrying `Viewer`, and asserts a following edit no longer reaches Sync | verified |
| `BUS-010` | `FRAME` | A command handler MUST NOT send on a channel: it returns an outcome and the dispatcher fans it out. | this spec section 2 | none. Each engine holds it over its own handlers (`TOILE-089`; the Prose row is unbuilt). Nothing holds it for an engine added later: a handler that captures a sender compiles | unverified |

### 1.4 A service request crosses the hops

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `BUS-013` | `POST` | The request identity allocator MUST yield a strictly increasing value, unique under concurrency. | this spec section 2 | `ambre_bus::host::request_id_is_strictly_monotonic`; `::request_id_is_unique_under_concurrency` drives eight threads at a thousand ids each and asserts every one is new | verified |
| `BUS-014` | `POST` | A service request identity MUST be echoed unchanged by every hop. | this spec section 2 | `ambre_bus::host::request_id_echo_is_preserved_across_the_hop` asserts the id survives the `postcard` codec byte for byte; `ambre_logic::dispatch::tests::rest_command_forwards_to_sync_with_id` and `::rest_event_forwards_to_ui_with_id` assert the dispatcher echoes it in both directions | verified |
| `BUS-009` | `POST` | A service request MUST be relayed to the Sync worker field for field. | this spec section 2 | `ambre_logic::dispatch::tests::create_document_preserves_fields` asserts both fields survive the arm. The relay is one arm for the whole family, so what the test pins for one variant is structural for the other thirty | verified |
| `BUS-015` | `POST` | A service response MUST be relayed to the interface field for field, unless a recorded route claims it for the document that issued it. | this spec section 2 | `ambre_logic::dispatch::tests::an_unrouted_rest_event_still_forwards_to_the_ui`; `::routed_media_uploaded_lands_in_engine_not_the_ui` asserts the claimed case reaches the engine and no stray event reaches the interface | verified |
| `BUS-020` | `FAIL` | A service response whose document was closed MUST be dropped, never delivered. | this spec section 2 | `ambre_logic::dispatch::tests::a_late_response_after_close_is_dropped_without_panic` asserts the route is gone on close and that the late response repaints nothing; `::a_correlated_upload_failure_aborts_the_import_without_a_raw_api_error` pins the same for the error direction | verified |
| `BUS-004` | `PRE` | A service request identity MUST be allocated by the UI thread. | this spec section 2 | none, and nothing could refuse it: `RequestId::next` is a free function over a process-wide counter, so the Sync worker -- which holds its own memory and therefore its own counter -- would allocate a colliding id in silence. There is no failure to define, only a caller that owes the rule | unverified |
| `BUS-019` | `FRAME` | Relaying a service request MUST NOT touch the document registry, save for the two commands that remove a document from it. | this spec section 2 | none. `ambre_logic::dispatch::tests::logout_clears_docs_and_forwards` pins the one exception that clears everything; nothing pins that the other twenty-nine variants leave the registry alone, and the hook is one `match` arm away from any of them | unverified |

### 1.5 A message crosses the worker hop

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `BUS-003` | `POST` | The Logic to Sync hop MUST return every field it was given: the encode and the decode MUST agree. | this spec section 2 | `ambre_bus::host::sync_command_roundtrips_through_postcard`, `::logic_event_roundtrips_through_postcard` compare the `Debug` rendering before and after. They cover 5 of the 12 `SyncCommand` variants and 4 of the 19 `LogicEvent` variants, so a variant added tomorrow is covered by nothing | unverified |
| `BUS-021` | `FRAME` | The worker hop MUST NOT decode an application payload: a CRDT delta and a channel message MUST cross it as opaque bytes. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-002` | none. The envelope types carry `Vec<u8>` and the round trip pins that the bytes come back identical, which says the codec does not touch them; nothing refuses the arm that would decode one before forwarding | unverified |

* **Budget.** two rows of `ambre-spec-budgets.md section 1`: the `flume` send and receive at
  < 1 us, and the Logic to Sync `postMessage` hop at < 1 ms. Neither is measured.
* **Boundary.** none of its own. The bus carries plaintext and reaches no relay: every byte that
  leaves does so through the Sync worker, sealed. `ZK-002` is what forbids a key type here, and the
  worker hop itself is the accepted internal boundary of `ambre-spec-zero-knowledge.md`.
* **Verification.** 10 `verified`, 8 `unverified`, 3 `unimplemented`. What holds is the Logic
  dispatcher: it can be driven on host through an unknown document, a viewer's edit, a child's own
  role, a service round trip in both directions and a response that arrives after its document
  closed. What is loose is every path nobody drives -- the encode of a variant no test names, a full
  channel, a stale version, and a handler that captures a sender. Four conjunctions came apart, and
  each hid an uncovered half behind a covered one. `BUS-004` claimed both who allocates an identity
  and that every hop echoes it: the echo is pinned three times over, and who allocates is a `PRE` no
  code could refuse, because the worker holds a counter of its own and a collision would be silent.
  `BUS-009` claimed the request and its response, which are different arms with different tests, and
  the response has a documented exception the old row denied outright. `BUS-011` claimed the edit
  survives and the user learns it did not, and neither is built. `BUS-012` claimed the coalescing and
  the stale discard, and only one application has a version to discard by. Three rows are new rather
  than split: `BUS-018` is the frame condition the lost-edit Story is really about -- the view writes
  the text it just failed to send -- and `BUS-019` and `BUS-021` name what the two relays must leave
  alone, the registry and the payload.

---

## 2. MODEL

**CRDT containers.** none. The bus carries commands and events; the document lives on the Logic
thread.

**Three planes, and each is routed differently.**

| Plane | Carried by | Payload | Logic routing |
|---|---|---|---|
| **Content** | `LogicCommand::App` / `UiEvent::{Viewport,Presence}` | `AppCommand` / `AppEvent`, typed and closed | by document identity, to that document's state |
| **Service** | the `Rest { id, .. }` variant of all four envelopes | `RestCommand` / `RestEvent`, defined once | relayed verbatim, plus a state hook for the few that change the registry |
| **Network** | `SyncCommand::Send*` / `LogicEvent::Remote*` and the pushes | opaque bytes: CRDT deltas, channel messages | applied to the document, or forwarded |

**Four envelopes, one per hop.** The direction is carried by the type name, so a `Command` always
travels towards the downstream worker and an `Event` always flows back.

| Flow | Type | Module | Variants |
|---|---|---|---|
| UI to Logic | `LogicCommand` | `logic.rs` | 9 |
| Logic to UI | `UiEvent` | `ui.rs` | 16 |
| Logic to Sync | `SyncCommand` | `command.rs` | 12 |
| Sync to Logic | `LogicEvent` | `event.rs` | 19 |
| shared service plane | `RestCommand` / `RestEvent` / `RequestId` | `rest.rs` | 31 / 30 |

```rust
pub enum LogicCommand {
    App { document_id: DocumentId, command: AppCommand },   // content
    Rest { id: RequestId, cmd: RestCommand },               // service, relayed
    OpenDocument { document_id: DocumentId, doc_type: DocType },
    // control and local restore variants, one per fire-and-forget operation
}
pub enum AppCommand { Trame(TrameCommand), Toile(ToileCommand) }
pub enum AppEvent { Trame(TrameEvent), Toile(ToileEvent) }
pub struct RequestId(pub u64);
```

The four envelopes and the service family are the contract; the variant lists are not restated here,
because a second copy of thirty variants is a copy that drifts. They live in `crates/ambre-bus/src/`,
one module per row of the table above.

**What stays on an envelope, and what joins the service family.**

| Shape | Where it goes |
|---|---|
| a correlated request with a success answer: session, document and channel management, sharing, media | `RestCommand` and `RestEvent`, one variant each |
| a control operation with no answer: open, close, send a channel message | an explicit envelope variant |
| a server push: a remote change, a connection status, a revocation, a meeting event | an explicit envelope variant |
| a failure: an authentication error, an interface error | an explicit envelope variant, correlated by identity only where a route recorded one |

**The content plane is a closed world.** Adding a document type is one variant in `AppCommand` and
one in `AppEvent`, and the dispatcher stops compiling until the new arm is written. Nesting adds
nothing: an embedded child is an ordinary entry of the flat registry, addressed by its own identity
(`BUS-006`), so the view layer nests while the data layer stays flat. What crosses into a focused
child is a raw input event, never a command, because a parent view cannot name a child's view type.

**Per-app modules, envelopes by direction.** The transport envelopes stay split by direction: each
is bound to one hop. The typed content plane is grouped one module per document type under `app/`,
so a document type's command enum, event enum and value types sit together. Everything is
re-exported flat at the crate root, so a downstream crate never names the internal layout.

**The role gate.** Each open document carries this client's role on **that** document. The
dispatcher drops a mutating command when the role is not writable (`BUS-007`). A freshly opened
document assumes the owner role so edits route before the members answer arrives, and the real role
supersedes it on `DocumentReady` (`BUS-008`). The gate is comfort, never security: the relay
re-checks every write, and the client is the attacker's own machine.

**Two traffic classes.** They share the same bounded channels and differ in what a full channel
means.

| Class | Examples | On a full channel |
|---|---|---|
| lossy | a viewport request, a hit test | drop it: the next frame resends the current state, and a stale answer is discarded by its version |
| lossless | an edit, a selection, a paste, an undo group boundary | never drop it: surface a busy signal and stop accepting input |

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
| a saturated Logic thread | an edit is lost with a log line, which is what `BUS-011` exists to forbid |

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
| Nothing is encoded between the UI and the Logic threads | one encoding for every hop, so that all four channels look alike | that path carries scroll and typing, where latency is the whole budget, and the two threads share one heap. Encoding it would buy isolation the crate graph already gives |
| The envelopes group by direction, the content plane by application | grouping every type by direction, including the per-application value types | a document type's command and its event share the whole value vocabulary, so splitting them by direction scatters each application over two modules. An envelope belongs to one hop, which is why direction is the right axis for it and the wrong one below it |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A lost edit is displayed as applied.** Trame's cell commit sends the text, logs the failure when
  the channel is full, and then writes the same text into its own view state. The user sees the
  edit; the document never received it (`BUS-018`).
* **Toile drops an edit with a warning.** `send_edit` treats a full channel as a logging event, so
  the lossless class exists in Trame alone and neither application surfaces a busy signal.
* **An undo group boundary is discarded outright.** The command that closes the group after a cell
  exit is sent with its result ignored, next to a comment stating that it must not be dropped. A
  dropped boundary leaves the group open and lets a later edit join it.
* **Nothing refuses an encoder on the UI to Logic path.** `BUS-001` is held by nobody calling one.
* **The postcard round trip is a sample.** Twelve variants of one envelope and nineteen of the other
  are covered by nine cases between them, and no test enumerates the variants (`BUS-003`).
* **A stale viewport answer is discarded in one application.** Trame compares an incoming version
  against the last one it accepted; Toile's viewport request carries no version, so a late answer is
  applied whenever it lands (`BUS-017`).
* **No rule covers a channel message payload.** The channel plane still carries opaque bytes on both
  envelopes, which is the `ARCH P7` Story, and the bus states nothing about what those bytes are.
* **Nothing bounds how long a service request may stay outstanding.** A request whose answer never
  arrives leaves its route recorded, and nothing times it out.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The threads, the channels and the crate graph | `ambre-spec-architecture.md` |
| What the UI thread may hold, and what it may open | `ambre-spec-ui.md` |
| What the Sync worker does with a command once it arrives | `ambre-spec-sync.md` |
| How a payload is sealed before it leaves | `ambre-spec-sealed-envelope.md` |
| Where a role comes from, and who may change it | `ambre-spec-membership.md` |
| How each engine handles a command and builds its outcome | each application's engine document |
| What a document contains, and how it is versioned | `ambre-spec-schema-version.md` |
| What each hop is budgeted at | `ambre-spec-budgets.md` |
