# AMBRE -- FEATURE: THE CLIENT MESSAGE BUS

**Type:** Horizontal (framework)
**Implementation:** partial -- the planes, the envelopes, the role gate, the outboxes on one
lane and the lossy gate ship. The lossy class coalesces by habit and no test drives the
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
| `BUS-001` | `INV` | A message on the UI to Logic path MUST travel as a typed Rust value by move: nothing on that path MUST be encoded. | this spec section 2; `ambre-spec-architecture.md section 2` | `ambre_bus::host::the_ui_to_logic_path_carries_nothing_an_encoder_takes` -- `LogicCommand` and `UiEvent` implement no `Serialize`, whatever they carry; `ambre_logic::app::tests::the_sums_on_the_ui_to_logic_path_carry_nothing_an_encoder_takes` -- nor do the two sums they carry | none | verified |
| `BUS-044` | `INV` | An event MUST reach each view of its document once. | this spec section 2; `ambre-spec-toile-embeds.md`, `TOILE-UI-340` | `ambre_app::switchboard::tests::two_views_of_one_document_never_take_an_event_twice` -- of two views of one document, the one that took an event first is not offered it again when the other reads; type-level -- each view queue holds a backlog of its own | none | verified |
| `BUS-029` | `INV` | A message field that carries an identifier MUST carry a type of its own, never a bare string or integer. | this spec section 2; `ai/ambre-rust-style.md`; `UI-THREAD-060`, the same rule for a shell signal | ci-gate -- `scripts/crate_boundaries.py` refuses an `id`, `*_id` or `*_ids` field of a bare primitive type in `ambre-bus` above its tests; type-level -- the sheet, filter view, conditional format and paragraph style ids each carry a newtype, so a swapped name and id fail to compile | none | verified |
| `BUS-032` | `INV` | An application's content commands and events MUST be declared in that application's crate, never in the crate that carries the envelopes. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-018` | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses a command or event enum of `ambre-bus` other than the four envelopes and the service family, a macro-written one included (`ambre_primitives::driver::bounds::the_bus_and_the_worker_hold_no_application_message_behind_a_macro_or_a_re_export`); type-level -- `LogicCommand` and `UiEvent` take the document command and event as a parameter and the channel types through a `ChannelPlane`, so the bus names none of an application's | none | verified |
| `BUS-027` | `INV` | A Murmure command MUST reach the Logic thread as a variant of the content plane, addressed to its channel. | this spec section 2; `BUS-002` | exhaustive-match -- `LogicCommand::Channel` carries the `Murmure` plane's `MurmureCommand`, a closed enum matched in `ambre_logic::channels::handle_channel_command`, so a new command does not build until its arm is written; `ambre_logic::channels::tests::a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event` | none | verified |
| `BUS-028` | `INV` | No envelope MUST carry a channel message as opaque bytes. | this spec section 2 | type-level -- `LogicCommand::Channel` and `UiEvent::Channel` hold the typed `MurmureCommand` and `MurmureEvent` of the `Murmure` plane, and no `SyncCommand` or `LogicEvent` variant names a channel message: a channel delta is the `SendUpdate` of its document | none | verified |
| `BUS-033` | `INV` | A view MUST write to the Logic thread through its outbox alone. | this spec section 2; `BUS-011` | type-level -- `Port` keeps its sends private to `ambre-bus` and implements no `Clone`, so a view moves its one port into one `Outbox`. The outbox's `send` and `send_lossy` are the two sends a view can name. The launcher builds each view's port from its own outbox and keeps no queue sender to hand a view (`BUS-035`) | none | verified |
| `BUS-035` | `INV` | A launcher sender that owns no frame MUST write to the Logic thread through the launcher's one outbox. | this spec section 2; `BUS-011` | type-level -- the raw end of the UI to Logic queue moves into `ShellOutbox::new` where it is made, so the compiler refuses a later use of it; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads the type of each expression. It refuses above its tests a launcher file other than `outbox.rs` that holds a `Sender<LogicCommand>` under any alias, past the one move into `ShellOutbox::new`, or builds a port over one (`ambre_primitives::driver::wiring::the_raw_logic_sender_under_an_alias_is_refused_past_the_outbox`); `ambre_app::outbox::tests::two_senders_on_a_full_queue_hold_their_commands_in_call_order` -- wasm tier: two clones share one hold queue and its order | none | verified |
| `BUS-037` | `INV` | A command MUST NOT reach the Logic thread before an older command that any outbox on its queue holds. | this spec section 2; `BUS-005`, `BUS-011` | `ambre_bus::outbox::tests::a_close_never_overtakes_an_edit_another_outbox_holds` -- a view holds an edit and the launcher's close joins the lane behind it; `::an_edit_never_overtakes_a_command_the_launcher_holds`; `::two_outboxes_on_one_lane_share_its_hold_bound`; `ambre_bus::lossy::tests::a_lossy_request_never_overtakes_a_held_command`; type-level -- a `Port` holds through a `Lane`, and `Lane::port` and `Port::direct` are its only constructors; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses above its tests a launcher file other than `outbox.rs` that builds a lane, and the raw sender moves into the one `ShellOutbox::new` that builds the queue's lane (`BUS-035`, `ambre_primitives::driver::wiring::the_raw_logic_sender_under_an_alias_is_refused_past_the_outbox`) | none | verified |

### 1.2 The interface sends a command

A view holds a bounded sender and one frame's worth of intent. What the class of the command decides
is what a full queue means.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-012` | `POST` | A lossy command MUST be coalesced to one send per frame. | this spec section 2 | `ambre_toile::ui::view::view_tests::two_camera_moves_in_one_frame_send_one_viewport_request`; `ambre_trame::ui::view::tests::two_scrolls_in_one_frame_flush_as_one_request` | none | verified |
| `BUS-017` | `POST` | A stale answer to a lossy request MUST be dropped by its version. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_stale_viewport_answer_is_dropped_by_its_version`; `ambre_trame::ui::grid::tests::group2::a_stale_viewport_answer_is_dropped_by_its_version` | none | verified |
| `BUS-011` | `POST` | A lossless command MUST NOT be dropped. | this spec section 2 | `ambre_bus::outbox::tests::a_full_channel_holds_the_command_instead_of_dropping_it`; `::a_flush_after_the_reader_drains_sends_the_held_command`; `::a_held_queue_keeps_the_order_the_user_typed`; `::a_send_while_busy_never_overtakes_a_held_command`; `ambre_murmure::view::tests::a_full_channel_holds_a_channel_command_and_the_page_takes_no_input` -- a channel command, through the outbox the two regions of a page share; type-level -- a view sends through its outbox alone (`BUS-033`), and so does a launcher sender with no frame (`BUS-035`); `ambre_app::outbox::tests::two_senders_on_a_full_queue_hold_their_commands_in_call_order` -- wasm tier: a launcher command held, then sent by the flush; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, records each `flush` on a `ShellOutbox`, and the judge refuses a switchboard in `lib.rs` that does not flush the launcher's outbox (`ambre_primitives::boundaries::tests::a_second_start_or_a_missing_fact_of_the_wiring_is_refused`); `ambre_canvas::frame::tests::a_hidden_page_relays_on_its_timer_and_a_shown_one_does_not` -- the switchboard that flushes it runs on a hidden page's timer too (`CANVAS-075`) | none | verified |
| `BUS-016` | `POST` | A full command queue MUST reach the user as a busy signal. | this spec section 2 | `ambre_canvas::busy::tests::only_the_first_frame_of_an_episode_reports_a_change` -- the publisher reports the edge once per episode, so the shell raises one notice and clears it once; `ambre_bus::outbox::tests::a_full_channel_holds_the_command_instead_of_dropping_it` -- the level the publisher carries; `ambre_belt::busy::tests::a_full_queue_reaches_the_user_as_a_busy_notice` -- browser tier: the Logic worker paused, a flood of edits raises the notice, and its resume clears it. The same flood with the worker running raises none | none | verified |
| `BUS-022` | `POST` | A view MUST stop accepting input while its outbox is busy. | this spec section 2; derived | `ambre_trame::ui::view::view_tests::a_busy_grid_takes_no_input`; `ambre_toile::ui::view::view_tests::a_busy_board_takes_no_input`; `ambre_app::host::prose::tests::a_busy_page_takes_no_input`; `ambre_murmure::view::tests::a_full_channel_holds_a_channel_command_and_the_page_takes_no_input` | none | verified |
| `BUS-024` | `POST` | A lossy command the queue dropped MUST be offered again on a later frame. | this spec section 2; derived | `ambre_trame::ui::view::tests::a_dropped_request_is_re_offered_not_doubled` -- the slot Trame re-offers from; `ambre_trame::ui::view::view_tests::a_dropped_scroll_request_is_offered_again`; `ambre_trame::ui::view::view_tests::a_refused_stats_request_fills_the_band_on_a_later_frame` -- the status band's request; `ambre_toile::ui::view::view_tests::a_dropped_viewport_request_is_offered_again`; `ambre_toile::ui::view::view_tests::a_dropped_media_request_is_offered_again`; `ambre_murmure::view::tests::a_full_channel_drops_a_typing_beat_and_holds_nothing` -- the beat the next key offers again | none | verified |
| `BUS-018` | `FRAME` | A command that failed to send MUST NOT be reflected in the view state. | this spec section 2; derived | `ambre_trame::ui::view::view_tests::a_refused_clear_leaves_the_cell`; `ambre_trame::ui::view::view_tests::a_refused_resize_leaves_the_width`; `ambre_trame::ui::view::view_tests::a_refused_paste_leaves_the_cut_source` | none | verified |
| `BUS-036` | `POST` | A launcher command its outbox refused MUST reach the user as a notice. | this spec section 2; `BUS-018` | `ambre_app::outbox::tests::a_refusal_past_the_bound_is_kept_for_the_relay_to_tell` -- wasm tier: the refusal waits for the switchboard, and is told once; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, records each `take_refusal` on a `ShellOutbox`, and the judge refuses a relay in `lib.rs` that never takes that refusal (`ambre_primitives::boundaries::tests::a_second_start_or_a_missing_fact_of_the_wiring_is_refused`) | none | verified |
| `BUS-023` | `FRAME` | A command that changes the document MUST NOT travel on the lossy class. | this spec section 2; derived | `ambre_bus::lossy::tests::a_command_that_changes_the_document_never_reaches_the_channel` -- one insert, one move and one resize, and the queue stays empty; `::a_command_of_another_plane_is_refused_with_the_mutations`; `::a_mutation_named_lossy_is_still_refused`; `::an_open_channel_takes_a_request_that_changes_nothing` -- the other side of the gate | none | verified |
| `BUS-034` | `FRAME` | A command its application does not name lossy MUST NOT travel on the lossy class. | this spec section 2; derived | `ambre_bus::lossy::tests::a_command_that_changes_nothing_but_is_not_named_lossy_is_refused` -- a selection changes nothing and still stays off the class; `::a_channel_command_takes_the_class_its_application_names`; `ambre_prose::ui::tests::a_lossless_command_never_leaves_as_a_request`; exhaustive-match -- `Mutation::traffic` has no default body, and `MurmureCommand` answers it with a `match` that names every command | none | verified |

### 1.3 Logic routes a content command

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-007` | `FAIL` | The Logic dispatcher MUST drop a mutating command when the caller's role on that document is not writable. | `ambre-spec-membership.md`, `SYNC-016` | `ambre_logic::dispatch::tests::viewer_edit_is_dropped_and_emits_no_update` -- no UI event and no outbound update; `::viewer_read_only_command_still_passes`; `::editor_edit_passes_through` -- the two sides the gate must not confuse | none | verified |
| `BUS-005` | `FAIL` | A command for a document absent from the registry MUST be dropped, never synthesised. | this spec section 2 | `ambre_logic::dispatch::tests::a_command_for_an_unopened_document_is_dropped` -- no update reaches Sync and no event reaches the UI; `::app_command_on_unknown_doc_is_dropped` -- the same for a read-only command, and both read the registry still empty | none | verified |
| `BUS-008` | `POST` | The role the relay reports MUST supersede the role a freshly opened document assumed. | this spec section 2 | `ambre_logic::dispatch::tests::document_ready_supersedes_the_optimistic_owner_role` -- it reads the optimistic `Owner` off the slot, feeds a `DocumentReady` carrying `Viewer`, and asserts a following edit no longer reaches Sync | none | verified |
| `BUS-010` | `FRAME` | A command handler MUST NOT send on a queue: it returns an outcome and the dispatcher fans it out. | this spec section 2 | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves. It refuses above its tests a queue of the conduit, a sender or an outbox, a `try_send` or a `send_to_` call in the `src/logic` directory of any engine, under any name (`ambre_primitives::driver::wiring::a_queue_under_a_neutral_name_is_refused_in_a_command_handler`) | none | verified |

### 1.4 A service request crosses the hops

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-013` | `POST` | The request identity allocator MUST yield a strictly increasing value, unique under concurrency. | this spec section 2 | `ambre_bus::host::request_id_is_strictly_monotonic`; `::request_id_is_unique_under_concurrency` -- eight threads at a thousand ids each, and every one is new | none | verified |
| `BUS-014` | `POST` | A service request identity MUST be echoed unchanged by every hop. | this spec section 2 | `ambre_bus::host::request_id_echo_is_preserved_across_the_hop` -- the id survives the `postcard` codec byte for byte; `ambre_logic::dispatch::tests::rest_command_forwards_to_sync_with_id`; `::rest_event_forwards_to_ui_with_id` -- the dispatcher echoes it in both directions | none | verified |
| `BUS-009` | `POST` | A service request MUST be relayed to the Sync worker field for field. | this spec section 2 | `ambre_logic::dispatch::tests::create_document_preserves_fields` -- both fields survive the arm, and the relay is one arm for the whole family, so what the test pins for one variant is structural for the rest of it | none | verified |
| `BUS-015` | `POST` | A service response MUST be relayed to the interface field for field, unless a recorded route claims it for the document that issued it. | this spec section 2 | `ambre_logic::dispatch::tests::an_unrouted_rest_event_still_forwards_to_the_ui` -- the unclaimed case; `::routed_media_uploaded_lands_in_engine_not_the_ui` -- the claimed case reaches the engine and no stray event reaches the interface | none | verified |
| `BUS-030` | `POST` | A document that could not open MUST reach the interface as a failed open naming it, never as an authentication error. | this spec section 2; `ambre-spec-sync.md`, `SYNC-157` | `ambre_logic::dispatch::tests::a_failed_open_reaches_the_ui_naming_its_document` -- a plain document's failed open reaches the UI as one failed open naming it | none | verified |
| `BUS-020` | `FAIL` | A service response whose document was closed MUST be dropped, never delivered. | this spec section 2 | `ambre_logic::dispatch::tests::a_late_response_after_close_is_dropped_without_panic` -- the route is gone on close and the late response repaints nothing; `::a_correlated_upload_failure_aborts_the_import_without_a_raw_api_error` -- the same for the error direction | none | verified |
| `BUS-004` | `PRE` | A service request identity MUST be allocated in the Main instance. | this spec section 2 | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each call with the instance rustc infers. It refuses in `ambre-sync` above its tests a `next_request`, and an associated function or constant of `Seq`, `Issuer` or `SharedIssuer` whose instance returns the `Request` family, the family left to inference included (`ambre_primitives::driver::wiring::a_request_id_built_with_an_inferred_family_is_refused_in_the_worker`) | none | verified |
| `BUS-019` | `FRAME` | Relaying a service request MUST NOT touch the document registry, save for the two commands that remove a document from it. | this spec section 2 | `ambre_logic::dispatch::tests::a_relayed_request_leaves_the_registry_alone` -- every other variant, listed by an exhaustive `match`; `::logout_clears_docs_and_forwards` -- the exception that clears everything | none | verified |

### 1.5 A message crosses the worker hop

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-003` | `POST` | The Logic to Sync hop MUST return every field it was given: the encode and the decode MUST agree. | this spec section 2 | `ambre_bus::host::every_envelope_variant_roundtrips_through_postcard` -- every `SyncCommand` and `LogicEvent` variant, named by an exhaustive match; `::sync_command_roundtrips_through_postcard`; `::logic_event_roundtrips_through_postcard` | none | verified |
| `BUS-031` | `POST` | A fixed-width byte value -- a key, a nonce, a signature, a digest -- MUST cross the worker hop as its bytes in order, with no length prefix. | this spec section 2; `BUS-003` | `ambre_core::fixed_bytes::tests::a_binary_format_writes_the_bytes_with_no_length_prefix` -- a 64-byte value between two bytes of a frame encodes as those 64 bytes alone, reads back, and a frame one byte short is refused; type-level -- every fixed-width value is a `FixedBytes<R>`, whose one hand-written serde writes that form | none | verified |
| `BUS-025` | `POST` | An event from the Sync worker MUST reach the Logic thread in arrival order. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-021` | `ambre_bus::conduit::tests::a_backlog_holds_past_a_full_channel_and_keeps_the_order`; `::a_backlog_never_lets_a_new_value_overtake_a_held_one` | none | verified |
| `BUS-026` | `POST` | An event from the Sync worker MUST NOT be dropped on a full queue. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-021` | `ambre_bus::conduit::tests::a_backlog_holds_past_a_full_channel_and_keeps_the_order` | none | verified |
| `BUS-021` | `FRAME` | The worker hop MUST NOT decode an application payload: a CRDT delta MUST cross it as opaque bytes. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-002` | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses a path of `ambre-sync`, above its tests, that reaches the application module of `ambre-bus` other than its `murmure` channel types of the service plane, or an app command or event (`ambre_primitives::driver::bounds::the_bus_and_the_worker_hold_no_application_message_behind_a_macro_or_a_re_export`), and the worker links no crate that declares an application message (`ARCH-070`); cargo-deny -- `deny.toml` grants `loro` to the application crates, and the CI `deny` job fails a worker that takes it; `ambre_bus::host::every_envelope_variant_roundtrips_through_postcard` -- the bytes come back identical | none | verified |

### 1.6 An event reaches a view

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `BUS-038` | `POST` | An event for a view MUST reach that view in the order the Logic thread sent it, past a full queue. | this spec section 2; `ambre-spec-toile-embeds.md`, `TOILE-UI-340` | `ambre_app::switchboard::tests::a_refused_main_view_event_waits_and_arrives_in_order` -- three events on a queue of one reach the main view in send order; `::the_main_view_backlog_drains_on_the_next_frame` -- the next frame's flush hands over what waited, ahead of that frame's fresh event; `::a_refused_event_waits_and_arrives_in_order` -- the same for an embedded view | none | verified |
| `BUS-039` | `FAIL` | An event that a view's full queue refuses MUST NOT be dropped in silence. | this spec section 2; `ambre-spec-toile-embeds.md`, `TOILE-UI-340` | `ambre_app::switchboard::tests::an_overflowing_main_view_backlog_says_what_it_dropped` -- past its bound the backlog drops its oldest event and logs one error per drop; `::an_overflowing_backlog_says_what_it_dropped` -- the same for an embedded view | none | verified |
| `BUS-040` | `FRAME` | An event waiting for one view MUST NOT hold back the events of another view. | this spec section 2; `BUS-038` | `ambre_app::switchboard::tests::a_full_view_holds_back_no_other_view` -- a full main view leaves a child its events, and a full child leaves the main view its own | none | verified |
| `BUS-041` | `POST` | A view that lost an event MUST have its document resent once its queue takes events again. | this spec section 2; `BUS-039`; `ambre-spec-budgets.md section 6` | `ambre_app::switchboard::tests::a_main_view_that_lost_an_event_has_its_document_resent_once_it_reads` -- once, and not while the view still lags; `::an_embedded_view_that_lost_an_event_has_its_document_resent_once_it_reads`; `::a_view_gone_after_a_loss_asks_for_nothing` | none | verified |
| `BUS-042` | `POST` | A resend of an open Trame or Toile document MUST hand its views the events that draw it whole. | this spec section 2; `BUS-041` | `ambre_logic::dispatch::tests::a_resend_hands_a_trame_view_its_whole_state` -- the re-fetch of the window, the tab strip, the cell toggles and the undo state; `::a_resend_hands_a_toile_view_its_whole_state` -- the viewport, the board and deck changes and the undo state; `::a_resend_of_a_document_not_open_answers_nothing` | none | verified |
| `BUS-045` | `FRAME` | A resend MUST NOT reach the relay. | this spec section 2; `BUS-042` | `ambre_logic::dispatch::tests::a_resend_hands_a_trame_view_its_whole_state` -- the Sync worker hears nothing; `::a_resend_hands_a_toile_view_its_whole_state`; `::a_resend_of_a_document_not_open_answers_nothing` | none | verified |
| `BUS-043` | `POST` | A Prose view that lost an event MUST come back to the text the Logic thread holds. | this spec section 2; `BUS-041`; `ambre-spec-prose-text-editing.md section 3` | not built | unbuilt -- lands with the `A Prose view that lost an event catches up` Story (`ambre-roadmap.md` E7). The view reconciles its text change by change, numbered, so a resent layout leaves a lost change out of its count | unimplemented |

* **Budget.** two rows of `ambre-spec-budgets.md section 1`: the queue send and receive at
  < 1 us, and the Logic to Sync `postMessage` hop at < 1 ms. Neither is measured.
* **Boundary.** none of its own. The bus carries plaintext and reaches no relay: every byte that
  leaves does so through the Sync worker, sealed. `ZK-002` is what forbids a key type here, and the
  worker hop itself is the accepted internal boundary of `ambre-spec-zero-knowledge.md`.
* **Verification.** 44 `verified`, 1 `unimplemented`. Host tests drive the Logic dispatcher: a role,
  a service round trip, the lossy gate, a full queue and a resend. Native views drive the input gate
  and the re-offer. A gate refuses a handler that sends on a queue, and the browser tier reads the
  busy notice. The native tier drives the view fan-out past its bound, and the Prose resend is not
  built.
---

## 2. MODEL

**CRDT containers.** none. The bus carries commands and events; the document lives on the Logic
thread.

**Three planes, and each is routed differently.**

| Plane | Carried by | Payload | Logic routing |
|---|---|---|---|
| **Content** | `LogicCommand::App` / `LogicCommand::Channel` / `UiEvent::{Viewport,Presence,Channel}` | `AppCommand` / `AppEvent`, and `MurmureCommand` / `MurmureEvent`, typed and closed | by document identity, or by channel identity to that channel's documents |
| **Service** | the `Rest { id, .. }` variant of all four envelopes | `RestCommand` / `RestEvent`, defined once | relayed verbatim, plus a state hook for the few that change the registry. Two are served inside Sync and reach no relay: the seal and the open of a file (`DOCFILE-014`) |
| **Network** | `SyncCommand::Send*` / `LogicEvent::Remote*` and the pushes | opaque bytes: CRDT deltas | applied to the document, or forwarded |

**Four envelopes, one per hop.** The direction is carried by the type name, so a `Command` always
travels towards the downstream worker and an `Event` always flows back.

| Flow | Type | Module | Variants |
|---|---|---|---|
| UI to Logic | `LogicCommand` | `logic.rs` | 20 |
| Logic to UI | `UiEvent` | `ui.rs` | 24 |
| Logic to Sync | `SyncCommand` | `command.rs` | 39 |
| Sync to Logic | `LogicEvent` | `event.rs` | 55 |
| shared service plane | `RestCommand` / `RestEvent` / `Request` | `rest.rs` | 31 / 34 |

```rust
pub enum LogicCommand<A = Infallible, P: ChannelPlane = NoChannels> {
    App { document_id: Id<Document>, command: A },   // content, the view's own command
    Channel { channel_id: Id<Channel>, command: P::Command }, // content, a channel
    Rest { id: Seq<Request>, cmd: RestCommand },     // service, relayed
    OpenDocument { document_id: Id<Document>, doc_type: DocType },
    // control and local restore variants, one per fire-and-forget operation
}
pub enum Request {}  // a family of `ambre_core::Seq`; `None` answers no request
// in `ambre-logic`, downstream of every application:
pub enum AppCommand { Trame(TrameCommand), Toile(ToileCommand), Prose(ProseCommand) }
pub enum AppEvent { Trame(TrameEvent), Toile(ToileEvent), Prose(ProseEvent) }
pub type LogicCommand = ambre_bus::LogicCommand<AppCommand, Murmure>;
```

The four envelopes and the service family are the contract. The variant lists are not restated
here, because a second copy of two hundred variants is a copy that drifts. They live in
`crates/ambre-bus/src/`, one module per row of the table above.

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

**Each application owns its vocabulary.** The transport envelopes stay split by direction: each is
bound to one hop. The two UI to Logic envelopes are generic over the document command and event
they carry. Each application declares its command enum, event enum and value types in its own crate
(`BUS-032`). The Logic runtime declares the sums over them, since it sits downstream of every
application.

A view holds a `Port` and a `Feed` typed by its own vocabulary. The launcher builds them over the
shared queues: `Embed` wraps a view's command into the sum, and unwraps its events. So the Sync
worker builds against no application's vocabulary (`ARCH-070`). A `Port` has no send outside the
bus and no `Clone`: the view moves it into its one outbox (`BUS-033`).

**Murmure's content rides a channel plane.** Its commands and events are addressed to a channel or
a space, never to a document (`BUS-027`), so they keep variants of their own. The envelopes take
their types through a second parameter, a `ChannelPlane` whose associated types the `Murmure`
marker fixes. Every other sender carries `NoChannels`, whose types are uninhabited. `PlaneOf`
embeds a view's plane into the launcher's, as `Embed` does for the document command. What stays
in `ambre-bus` is shared by nature: the grid, the find query, and the channel types of the service
plane, which the worker reads.

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

Each class is one send of the view's outbox, and the send decides what a full queue does. The
outbox is the view's one way to write (`BUS-033`). `Outbox::send` holds a lossless command for a
later frame (`BUS-011`). `Outbox::send_lossy` drops a lossy one. It refuses any other command,
because the lossy class carries no intent the user gave (`BUS-023`, `BUS-034`). The caller reads
the outcome: it writes nothing into the view for a command that did not leave (`BUS-018`), and
re-offers a dropped request (`BUS-024`).

**The launcher writes through one outbox of its own.** The shell, the landing, the route poll, the
switchboard, the local store and the vault send commands no view owns. None of them has a frame, so
they share one `ShellOutbox` over one `Outbox`, and their commands leave in call order (`BUS-035`).

**Every outbox on one queue holds through one lane.** A `Lane` is the hold queue of the UI to Logic
queue, and each port holds through it. So a view's outbox and the launcher's keep one order, the
order of the calls (`BUS-037`). A close sent after an edit joins the lane behind that edit. The
Logic thread then applies the edit before the document leaves its registry (`BUS-005`).

* **The busy signal is shared.** A command any sender holds makes every outbox on the lane busy,
  since the queue is full for each of them alike (`BUS-016`).
* **The bound is shared.** The lane refuses past its bound, whichever sender fills it (`BUS-018`).
* **Any flush drains the lane.** The switchboard's flush sends a view's held edit too, oldest first.
* **A lossy request never passes a held command.** It drops while the lane holds anything, and the
  next frame offers it again (`BUS-024`).

The switchboard flushes it before anything newer leaves, on every frame and on a hidden page's timer
(`ambre-spec-frame.md`, `CANVAS-075`). A held command raises the busy notice (`BUS-016`). A refusal
raises a notice once (`BUS-036`), and a shell control reads the outcome before it changes its state.

The raw end of the queue moves into that outbox where it is made, which builds the queue's one
lane. The views take their ports from that lane. A Leptos `StoredValue` with local storage carries
it across the router's `Send` boundary.
Every call runs on the main thread, so no lock guards it (`ARCH-020`).

The worker hop is not this queue, so the Logic to Sync drain and the Sync backlog need no outbox.

**The application names its lossy commands.** `Mutation::traffic` answers lossy or lossless for each
command, and has no default. A command that changes nothing is not lossy for that reason alone: a
selection or a read mark carries intent. A request the view records as asked before it leaves is
lossless too, since nothing would offer it again.

| Application | Lossy commands |
|---|---|
| Trame | the viewport fetch, the selection aggregates |
| Toile | the viewport fetch, the media fetch, the presence beat |
| Prose | the marks query, the word count |
| Murmure | the tick, the typing beat |

Two requests that change nothing stay lossless, because no sender offers them again:

| Application | Request | Why it stays lossless |
|---|---|---|
| Toile | the selection aggregates | the view records the request as asked before it leaves: the selection the band last asked for, a pending fit, a pending focus |
| Prose | the caret sample | no sender exists yet (`ambre-spec-presence.md`), so nothing offers a lost sample again |

The other read-only requests of Trame and Toile answer one user action each, so they stay lossless
for the same reason.

**The Sync events are lossless too.** A delta or a service answer arrives through the worker's
`postMessage` handler on the Main thread, which may not wait for room (`ambre-spec-architecture.md`,
`ARCH-021`). What the queue to the Logic thread refuses waits in a backlog, in arrival order
(`BUS-025`, `BUS-026`). The switchboard offers it again each frame before anything newer. Past a
high mark the switchboard asks the worker to stop reading the relay, and at a low mark to start again
(`ambre-spec-sync.md`, `SYNC-059`). The relay holds what was not read, so the backlog stays bounded
and loses nothing.

**A view's events wait for room too.** The switchboard writes every view queue through one view
fan-out, which holds the main view's sender and the sender of each embedded view. What a full
queue refuses waits in a backlog per view, in send order (`BUS-038`). The switchboard offers it
again on the next frame, before anything newer. Past its bound the backlog drops its oldest event
and logs an error (`BUS-039`). Each view keeps a backlog of its own, so it holds back no other view
(`BUS-040`) and takes each event once (`BUS-044`).

**A view that lost an event is resent its document.** The bound is reachable: a hidden page relays
its events on a timer, and its views read nothing until it shows again
(`ambre-spec-budgets.md section 6`). The backlog notes the document of each event it drops. Once
the backlog is empty again, the switchboard asks the Logic thread to resend that document
(`BUS-041`). It waits for that moment, since a view that reads nothing would lose the answer too.

The Logic thread answers from the state it holds, with the events a whole import answers with
(`BUS-042`). Nothing reaches the relay, and nothing is written (`BUS-045`). A Trame view
re-fetches its window and reads its tabs, its toggles and its undo state again. A Toile view takes
a viewport and re-reads its panels. A Prose view is resent nothing yet (`BUS-043`).

**A fixed-width value crosses as its bytes.** A key, a nonce, a signature or a digest has a width
its role states once. The worker hop encodes it as that many bytes, in order, with no length
prefix, since the type already fixes the length (`BUS-031`). A readable codec, such as the JSON of
the relay, spells the same value as lowercase hexadecimal (`ambre-spec-server.md`, `SRV-242`).
One generic type writes both forms, so no role chooses its form by an attribute.

These forms replaced older ones with no version bump and no migration, since nothing had shipped.
A key, a nonce or a signature once carried a length prefix, and a salt once travelled as base64.
So the update blobs, the snapshots and the membership log a development database stored before the
change do not read after it. The break is deliberate, not a defect.

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
| A view that lost an event is resent its document once it reads again | a resend asked the moment the event drops | the view reads nothing then: a hidden page or a stalled frame is what filled it. The answer would wait behind the backlog, or be dropped in turn |
| A view that lost an event is resent its document once it reads again | a backlog with no bound | a page hidden for an hour holds every event of that hour, in the heap the Logic thread shares (`ambre-spec-budgets.md section 6`) |
| A view that lost an event is resent its document once it reads again | the Logic thread importing its own snapshot, or a close and an open | the import reseeds a Trame formula engine whole: at 10k formula cells, about a second and a half on the target phone (`ambre-spec-budgets.md section 1`). A close and an open reach the relay and the store for a state this thread already holds |
| Each view keeps a backlog of its own | one backlog per document, shared by its views | a full view holds back the event the other view took, and both are offered it again: that view applies it twice |
| A closed sum per direction, one variant per document type | a trait object plus a registry of boxed controllers | it re-creates a crate cycle, because the sum resolves downstream of every application crate, and it gives up the exhaustive dispatch the compiler proves today |
| A closed sum per direction, one variant per document type | an opaque byte payload decoded by the receiver | the dispatch becomes a run-time match on bytes: adding a document type still builds, and the missing arm is found when a message arrives rather than at compile time |
| The service family is defined once and carried by a thin envelope | four nearly identical enums, one per direction | modelling each service operation as a variant in each of the four directions is about three hundred lines of one-to-one copying, and the failure mode is a field copied into the wrong place. Defined once, the whole service dispatch is a single arm |
| The request identity is allocated in the Main instance, by the caller | allocating the identity in the Sync worker | the worker holds its own memory and so its own counter, and an id it allocated would collide in silence with one the Main instance gave out. In the Main instance the UI and Logic threads share one atomic counter, and the caller is the one that recognises its own answer |
| The request identity is allocated in the Main instance, by the caller | correlating by operation shape | two identical requests in flight are indistinguishable, and the pairing is wrong exactly when the interface is busiest |
| The registry is flat and nesting is a view concern | a tree of documents, with a child reached through its parent | it needs a depth bound, an unwrapping path and a rule for a cycle. A flat map needs one lookup, and the nesting the user sees is the view owning a child view |
| The role gate is comfort, and the relay stays the authority | trusting the client gate | the client is the attacker's own machine, so a gate there proves nothing |
| The role gate is comfort, and the relay stays the authority | offering every edit and letting the relay refuse | the interface would offer an edit the relay rejects, and the user learns it after the act rather than before |
| Murmure's content rides a second parameter, a channel plane | folding the channel and space variants into the document parameter `A` | `App` is addressed to a document, and a channel is several (`BUS-027`). The role gate and the lossy class reason per document, so every channel site and both gates would be rewritten |
| Murmure's content rides a second parameter, a channel plane | a Murmure sum in the Logic runtime, carried by the existing variants | the envelope would still need a parameter to name it, and every channel site would wrap and unwrap a sum of one application |
| A view's own command is embedded into the sum on the one UI to Logic queue | one queue per application into the Logic thread, each typed by that application's command | the launcher's own commands and a view's commands would travel on two queues, and nothing would order them. A close sent after an edit could overtake it, which is the loss `ARCH-019` forbids |
| The launcher's senders share one outbox, flushed by the switchboard | one outbox per launcher sender | two hold queues on one queue reorder it: a route change held in one would leave after an edit held in the other, and the close could overtake its edit |
| The launcher's senders share one outbox, flushed by the switchboard | an outbox shared through a mutex, so that it crosses the router's boundary | the Main thread takes no lock on a queue it shares with the Logic thread (`ARCH-020`). Every call runs on that one thread, so thread-local storage costs nothing and waits on nothing |
| Every outbox on one queue holds through one lane | a close that waits until the view's outbox is empty, the launcher flushing the view first or holding the close back while the view is busy | the launcher reaches no view's outbox, and the level the view publishes lags an input that arrives between frames. It also orders the close alone: an edit could still overtake a held open |
| Every outbox on one queue holds through one lane | the Logic thread keeping an edit for a document that is closing, instead of dropping it | Logic cannot tell a late edit from one sent to a document never opened, so `BUS-005` would turn into a tombstone per closed document. An edit applied after the close would also reach no view |
| The launcher's senders share one outbox, flushed by the switchboard | each sender flushing on a timer of its own | a timer per sender is one more clock to drive, and the switchboard already runs on every frame and on a hidden page's timer |
| Nothing is encoded between the UI and the Logic threads | one encoding for every hop, so that all four queues look alike | that path carries scroll and typing, where latency is the whole budget, and the two threads share one heap. Encoding it would buy isolation the crate graph already gives |
| The envelopes group by direction, the content plane by application | grouping every type by direction, including the per-application value types | a document type's command and its event share the whole value vocabulary, so splitting them by direction scatters each application over two modules. An envelope belongs to one hop, which is why direction is the right axis for it and the wrong one below it |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The lane has a bound, and past it the command is refused.** Sixty-four commands across every
  outbox on the queue, after which the outbox reports a refusal rather than growing without limit.
  The input gate closes on the first held command, so reaching the bound takes a saturation no test
  has produced.
* **A channel is not a document, so it has its own content variant.** `AppCommand` is addressed to
  one document, and a channel is several (`MURMURE-021`). `LogicCommand::Channel` carries its
  commands through the `Murmure` plane, and a channel delta leaves as the `SendUpdate` of the
  document it names.
* **An event that names no document is lost for good.** The backlog bound trades a stalled
  view's memory for a lost event, which the log names (`BUS-039`). A view event names its document,
  so the document is resent (`BUS-041`). A service answer or a channel event names none, so nothing
  resends it. The busy signal of `BUS-016` covers the command direction alone, so the user sees
  nothing.
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
