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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `BUS-001` | A message on the UI to Logic path MUST travel as a typed Rust value by move: nothing on that path MUST be encoded. | this spec section 2; `ambre-spec-architecture.md section 2` | none. The path is a `flume` channel over the shared heap and no encoder is called on it, which is construction rather than a check. Nothing refuses the call that would add one | unverified |
| `BUS-002` | A document command MUST reach the Logic thread as a variant of a closed sum, never as an opaque payload. | this spec section 2 | the compiler: `AppCommand` and `AppEvent` are closed enums, so a missing document type fails the exhaustive `match` in the dispatcher | verified |
| `BUS-003` | The Logic to Sync hop MUST return every field it was given: the encode and the decode MUST agree. | this spec section 2 | `ambre_bus::host::sync_command_roundtrips_through_postcard`, `::logic_event_roundtrips_through_postcard`. They cover 5 of the 12 `SyncCommand` variants and 4 of the 19 `LogicEvent` variants, so a variant added tomorrow is covered by nothing | unverified |
| `BUS-004` | A service request identity MUST be allocated by the UI thread and echoed unchanged by every hop. | this spec section 2 | `ambre_bus::host::request_id_is_strictly_monotonic`, `::request_id_is_unique_under_concurrency`, `::request_id_echo_is_preserved_across_the_hop` | verified |
| `BUS-005` | A command for a document absent from the registry MUST be dropped, never synthesised. | this spec section 2 | `ambre_logic::dispatch::tests::app_command_on_unknown_doc_is_dropped`, `::a_command_for_an_unopened_document_is_dropped`, `::a_late_response_after_close_is_dropped_without_panic` | verified |
| `BUS-006` | The document registry MUST stay flat: an embedded child MUST be addressed by its own identity, in one lookup. | this spec section 2; `ambre-spec-toile-embeds.md`, `TOILE-084` | `ambre_logic::dispatch::tests::an_embedded_child_is_gated_by_its_own_role_not_the_boards`, `::owning_the_board_grants_nothing_on_a_document_embedded_in_it` | verified |
| `BUS-007` | The Logic dispatcher MUST drop a mutating command when the caller's role on that document is not writable. | `ambre-spec-membership.md`, `SYNC-016` | `ambre_logic::dispatch::tests::viewer_edit_is_dropped_and_emits_no_update`, `::viewer_read_only_command_still_passes`, `::editor_edit_passes_through` | verified |
| `BUS-008` | The role the relay reports MUST supersede the role a freshly opened document assumed. | this spec section 2 | `ambre_logic::dispatch::tests::document_ready_supersedes_the_optimistic_owner_role` | verified |
| `BUS-009` | A service request and its response MUST be relayed field for field: no hop MUST rewrite either. | this spec section 2 | `ambre_logic::dispatch::tests::rest_command_forwards_to_sync_with_id`, `::rest_event_forwards_to_ui_with_id`, `::create_document_preserves_fields`, `::an_unrouted_rest_event_still_forwards_to_the_ui` | verified |
| `BUS-010` | A command handler MUST NOT send on a channel: it returns an outcome and the dispatcher fans it out. | this spec section 2 | each engine holds it over its own handlers (`TOILE-089`; the Prose row is unbuilt). Nothing holds it for an engine added later: a handler that captures a sender compiles | unverified |
| `BUS-011` | A lossless command MUST NOT be dropped: a full channel MUST reach the user as a busy signal. | this spec section 2 | none, and both applications fail it. Toile logs a warning inside `send_edit` and continues; Trame returns the error and every call site logs it. No path stops accepting input | unverified |
| `BUS-012` | A lossy command MUST be coalesced to one send per frame, and a stale answer MUST be discarded by its version. | this spec section 2 | none. Each view keeps one pending viewport request and compares the version on the way back; no test drives a full channel, so the coalescing is held by reading the code | unverified |

* **Budget.** two rows of `ambre-spec-budgets.md section 1`: the `flume` send and receive at
  < 1 us, and the Logic to Sync `postMessage` hop at < 1 ms. Neither is measured.
* **Boundary.** none of its own. The bus carries plaintext and reaches no relay: every byte that
  leaves does so through the Sync worker, sealed. `ZK-002` is what forbids a key type here, and the
  worker hop itself is the accepted internal boundary of `ambre-spec-zero-knowledge.md`.
* **Verification.** 7 `verified`, 5 `unverified`. What holds is what the dispatcher can be driven
  through on host: an unknown document, a viewer's edit, a child's own role, a service round trip.
  What is loose is every rule about a path nobody exercises -- the encode of a variant no test
  names, a full channel, and a handler that captures a sender.

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

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A closed sum per direction, one variant per document type | the compiler proves the dispatch exhaustive, so adding a document type is one edit that fails to build until it is finished. A trait object would also re-create a crate cycle, because the sum resolves downstream of every application crate | a trait object plus a registry of boxed controllers, or an opaque byte payload decoded by the receiver |
| The service family is defined once and carried by a thin envelope | modelling each service operation as a variant in each of the four directions is about three hundred lines of one-to-one copying, and the failure mode is a field copied into the wrong place. Defined once, the whole service dispatch is a single arm | four nearly identical enums, one per direction |
| The request identity is allocated by the UI thread | the caller is the one that needs to recognise its own answer, and shared memory makes a single atomic counter legal across the two threads. Allocating it further down would force a second correlation table | allocating the identity in the Logic thread, or correlating by operation shape |
| The registry is flat and nesting is a view concern | a recursive registry needs a depth bound, an unwrapping path and a rule for a cycle. A flat map needs one lookup, and the nesting the user sees is the view owning a child view | a tree of documents, with a child reached through its parent |
| The role gate is comfort, and the relay stays the authority | the client is the attacker's own machine, so a gate there proves nothing. It exists so the interface stops offering an edit the relay would refuse | trusting the client gate, or offering every edit and letting the relay refuse |
| Nothing is encoded between the UI and the Logic threads | that path carries scroll and typing, where latency is the whole budget, and the two threads share one heap. Encoding it would buy isolation the crate graph already gives | one encoding for every hop, so that all four channels look alike |
| The envelopes group by direction, the content plane by application | an envelope belongs to one hop, so direction is its natural axis. A document type's command and its event share the whole value vocabulary, so splitting them by direction scatters each application over two modules | grouping every type by direction, including the per-application value types |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A lost edit is displayed as applied.** Trame's cell commit sends the text, logs the failure when
  the channel is full, and then writes the same text into its own view state. The user sees the
  edit; the document never received it (`BUS-011`).
* **Toile drops an edit with a warning.** `send_edit` treats a full channel as a logging event, so
  the lossless class exists in Trame alone and neither application surfaces a busy signal.
* **An undo group boundary is discarded outright.** The command that closes the group after a cell
  exit is sent with its result ignored, next to a comment stating that it must not be dropped. A
  dropped boundary leaves the group open and lets a later edit join it.
* **Nothing refuses an encoder on the UI to Logic path.** `BUS-001` is held by nobody calling one.
* **The postcard round trip is a sample.** Twelve variants of one envelope and nineteen of the other
  are covered by nine cases between them, and no test enumerates the variants.
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
