# AMBRE -- FEATURE: THE SEQUENCE, AND WHAT SURVIVES A CRASH

**Type:** Horizontal (framework)

> The relay cannot read a change, so the one thing it can promise is order. Every accepted change
> gets a number, and the numbers never repeat. A client that was away gets exactly what it missed.

This document owns what happens to a sealed delta between its arrival and the next client's
request for it. Three things: the number it is given, the two stores it lands in, and the replay
that hands it back.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-server.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-006` | `INV` | Per document and per author the accepted clock MUST increase by exactly one. | this spec section 3; `ambre-spec-delta-order.md`, `SYNC-024` | `ambre_server::redis_ops::lua_tests::sequential_clock_succeeds` -- an accepted value; `::first_update_succeeds` -- the first one; `::duplicate_clock_rejected` -- the replayed refusal; `::clock_mismatch_rejected` -- the out-of-step refusal; `::independent_clients_have_independent_clocks` -- one author's clock does not move another's, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-150` | `INV` | The deltas a document holds inline MUST stay within the inline cap of the budget register, passed by no more than the writes in flight when it is reached. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_server::hot_tail::db_tests::a_full_tail_takes_no_more_inline` -- a tail at its count and one at its bytes each refuse the next delta, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`); `ambre_server::hot_tail::db_tests::the_tail_count_skips_the_moved_rows` -- after a move, the next insert counts only the rows still inline, on the same tier | none | verified |
| `SRV-132` | `INV` | The deltas a document keeps past its latest confirmed snapshot MUST stop growing while a single author writes. | this spec section 3; `SRV-130` | `ambre_server::compaction::db_tests::a_self_snapshot_trims_its_author_alone` -- a snapshot its author alone confirmed drops that author's deltas past the reference, up to its horizon, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-263` | `INV` | The durable index MUST hold at most one delta per number of a document. | this spec section 3; `SRV-033` | `ambre_server::hot_tail::db_tests::a_number_names_one_delta_of_its_document` -- a second delta under a taken number is refused, inline and with an object of its own, the same delta again stays one row, and another document reuses the number, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-274` | `INV` | An insert of a delta MUST NOT lock the row of its document. | this spec section 3; `SRV-259`; `ambre-spec-server.md`, `SRV-273` | `ambre_server::hot_tail::db_tests::an_insert_locks_no_document_row` -- an inline delta and a delta with an object of its own leave the document row with no locker in its `xmax`, where the check of a foreign key wrote one, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |

### 1.2 A delta is numbered

The one thing a blind relay decides. Two counters -- the author's clock and the document's number --
move together or neither moves.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-032` | `POST` | An ordering state the ephemeral store has lost MUST be rebuilt from the durable history before any delta is validated. | this spec section 3 | `ambre_server::redis_ops::redis_integration_tests::seed_clock_state_unblocks_the_next_update_after_a_redis_reset` -- the author's next delta is accepted after a reset that would otherwise refuse it for ever, the rebuild raising and never lowering, so a live writer cannot be undone by it | none | verified |
| `SRV-288` | `POST` | A rebuilt ordering state MUST number the next delta past every delta the trim removed and past every snapshot horizon. | this spec section 3; `SRV-032`; `SRV-127` | `ambre_server::db::tests::a_trimmed_document_reseeds_past_its_horizon` -- database tier: once every delta is trimmed under a confirmed horizon, the rebuild numbers the next delta past it, and a document holding a horizon alone seeds its counter | none | verified |
| `SRV-300` | `POST` | A rebuilt ordering state MUST keep the clock of each author whose deltas the trim removed. | this spec section 3; `SRV-032`; `SRV-288` | `ambre_server::db::tests::a_trimmed_document_reseeds_past_its_horizon` -- database tier: the author whose every delta was trimmed sends its next clock, and the rebuilt state accepts it | none | verified |
| `SRV-289` | `FAIL` | Given a snapshot whose horizon is past the last delta number the ephemeral store holds, the relay MUST refuse it, a lost counter included. | this spec section 3; `SRV-288` | `ambre_server::routes::tests::handler_tests::a_cold_counter_refuses_every_horizon_past_zero` -- database tier: a horizon past a lost counter is refused, and lands once the counter is rebuilt | none | verified |
| `SRV-080` | `POST` | The clock check and the number MUST be one indivisible step. | this spec section 3; `SRV-006` | `ambre_server::redis_ops::redis_integration_tests::one_clock_sent_at_once_is_numbered_once` -- one author's clock sent at once over sixteen connections is numbered once and refused fifteen times, and the counter moves by one | none | verified |
| `SRV-037` | `FAIL` | A refused clock MUST reach its author as a typed conflict, naming the document and the refused delta, and carrying the expected and the received value. | `ambre-spec-server.md`, `SRV-020`; `ambre-spec-delta-order.md`, `SYNC-147` | `ambre_server::redis_ops::tests::the_script_refusal_becomes_the_typed_conflict`; `ambre_server::ws::handler::message::refusal_tests::a_refused_clock_carries_the_expected_and_the_received_value` -- the script refusal is parsed into the typed conflict, and the refusal carries the document, the signature and both values | none | verified |
| `SRV-169` | `FRAME` | After a clock refusal, the relay MUST NOT number a later delta of that document on that socket until the author answers the refused delta. | this spec section 3; `SRV-037`; `ambre-spec-delta-order.md`, `SYNC-147` | `ambre_server::ws::handler::message::refusal_tests::a_refused_clock_fences_the_socket_until_its_answer` -- the fence holds one document, an answer naming another delta lifts nothing, and the answer lifts it, as does forgetting the document; `ambre_server::ws::handler::message::hot_tail_tests::a_delta_behind_a_refused_clock_waits_for_the_answer` -- of two sockets of one key, the refused one drops its next delta unanswered and unnumbered, and numbers it after the answer, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-081` | `FRAME` | A refusal MUST NOT carry a store's own name or error text out to the client. | `ZK-006`; `ambre-spec-server.md`, `SRV-020` | `ambre_server::ws::handler::message::refusal_tests::a_store_failure_reaches_the_client_without_its_name_or_text` -- any failure other than the typed conflict reaches the client as a bare internal code | none | verified |

### 1.3 A numbered delta is made durable

The number is assigned before the delta is stored, so every failure below is a failure after the
sequence has already moved.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-036` | `FRAME` | Two instances MUST NOT drain one document's queued deltas at the same time. | this spec section 3; `SRV-021` | `ambre_server::redis_ops::redis_integration_tests::flush_lock_is_mutually_exclusive` -- the second acquirer is refused while the first holds it, on the database tier | none | verified |
| `SRV-033` | `POST` | A delta given a number MUST reach the durable index, unless a delete has fenced its document. | audit `H1`; this spec section 3; `SRV-335` | `ambre_server::ws::handler::message::delivery_tests::a_refused_index_write_is_retried_until_the_row_lands` -- a trigger refuses the first two inserts, and the row lands on the third attempt with nothing dead-lettered | none | verified |
| `SRV-034` | `POST` | A delta given a number MUST reach durable storage, unless a delete has fenced its document: its index row, a batch object or an object of its own. | audit `H1`; this spec section 3; `SRV-335` | `ambre_server::ws::handler::message::hot_tail_tests::a_small_delta_lives_in_its_row_and_a_large_one_in_its_own_object` -- a small delta in its row, a large one in an object the row names; `ambre_server::ws::handler::message::delivery_tests::a_refused_object_leaves_the_frame_in_its_row` -- a large delta whose object the store refuses takes its bytes into its index row, the refusal is counted, and a replay delivers it | none | verified |
| `SRV-262` | `FAIL` | Given a dirty mark the store refuses in a sweep, the relay MUST keep the delta queued, log the miss and count it in its failures metric. | this spec section 3; `SRV-034`; `SRV-257`; `ZK-030` | `ambre_server::s3::tests::a_refused_mark_in_a_sweep_keeps_the_list_and_is_counted` -- a user whose keys stop at the document runs the sweep over a queued list: the list keeps its entry, and the `dirty_mark` failure grows. Database tier. The CI `test-db-cluster` job runs it over three primaries | none | verified |
| `SRV-082` | `POST` | A durable write that failed MUST leave the whole delta where an operator can replay it. | audit `H1`; this spec section 3 | `ambre_server::ws::handler::message::delivery_tests::an_index_write_that_never_lands_leaves_the_whole_frame` -- past every retry, the dead letter holds the number, the object key and the signed delta byte for byte, and its signature verifies | none | verified |
| `SRV-035` | `FRAME` | A flush MUST NOT drop an entry it did not store. | this spec section 3 | `ambre_server::s3::tests::a_partial_flush_trims_only_what_it_stored` -- a batch failing at its fourth entry trims three, the unreadable one among them, and keeps the rest | none | verified |
| `SRV-148` | `POST` | A delta under the inline size MUST be stored in its index row, in the same write as the row. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_server::ws::handler::message::hot_tail_tests::a_small_delta_lives_in_its_row_and_a_large_one_in_its_own_object` -- a small signed delta leaves its bytes in its row and no pending entry, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-149` | `POST` | A delta over the inline size MUST be stored as an object of its own, which its index row names. | this spec section 3; `ambre-spec-object-store.md` | `ambre_server::ws::handler::message::hot_tail_tests::a_small_delta_lives_in_its_row_and_a_large_one_in_its_own_object` -- a delta past the inline size gets a row naming its object, the object exists, and no pending entry holds it | none | verified |
| `SRV-307` | `FAIL` | Given a loss of the ephemeral store, every delta the relay acknowledged MUST stay readable. | this spec section 3; `SRV-034`; `SRV-149` | `ambre_server::ws::handler::message::delivery_tests::a_large_delta_survives_a_loss_of_the_ephemeral_store` -- the object of a large delta exists once the relay handled it, no pending entry holds it, and a replay after every ephemeral key of the document is dropped delivers it; `ambre_server::ws::handler::message::delivery_tests::a_refused_object_leaves_the_frame_in_its_row` -- a refused object leaves the frame in the durable index. Database tier | none | verified |
| `SRV-335` | `FRAME` | The relay MUST NOT write the object, the index row or the broadcast of a delta once it finds its document fenced. | this spec section 3; `SRV-034`; `ambre-spec-server.md`, `SRV-017`, `SRV-273`; `ambre-spec-object-store.md`, `SRV-057` | `ambre_server::ws::handler::message::delivery_tests::a_fenced_document_stores_and_sends_no_delta` -- a tombstone set after the number was given leaves an inline delta and a large one with no object, no row and no broadcast, and the author hears that the document is not found; `ambre_server::ws::handler::message::delivery_tests::a_delete_begun_during_the_object_put_takes_the_row_and_the_broadcast` -- a tombstone set while the object of a large delta is put leaves that object to the second sweep, and no row and no broadcast. Database tier | none | verified |
| `SRV-336` | `FRAME` | The object write of a delta MUST take one request, given up past the wait of `ambre-spec-budgets.md section 3`. | this spec section 3; `SRV-034`; `SRV-307`; `ambre-spec-budgets.md section 3` | `ambre_server::ws::handler::message::delivery_tests::an_object_put_past_its_wait_leaves_the_frame_in_its_row` -- the object of a large delta is written once, and a write the store holds for twice the wait is given up within a second past the wait, its frame in the index row and the refusal counted. Database tier | none | verified |
| `SRV-151` | `POST` | Past the inline cap, the relay MUST move the oldest inline deltas of the document into one batch object, and point their rows at it. | this spec section 3; `SRV-150` | `ambre_server::hot_tail::db_tests::a_move_takes_the_oldest_and_keeps_the_newest` -- of two hundred inline deltas the oldest 136 move to one batch their rows name, and the newest 64 stay; `ambre_server::hot_tail::tests::a_move_starts_at_three_quarters_of_either_bound` -- the move starts before the cap; `ambre_server::hot_tail::tests::a_move_keeps_a_quarter_of_the_cap` -- what stays | none | verified |
| `SRV-152` | `POST` | A document with no write for the idle delay MUST have its inline deltas moved into one batch object. | this spec section 3; `ambre-spec-budgets.md section 8` | `ambre_server::hot_tail::db_tests::an_idle_tail_moves_whole` -- a tail written a day ago moves whole, and one written since stays | none | verified |
| `SRV-153` | `FRAME` | Moving deltas into a batch object MUST NOT change their numbers or their bytes. | this spec section 3 | `ambre_server::hot_tail::db_tests::a_move_takes_the_oldest_and_keeps_the_newest` -- the batch holds each number with its bytes unchanged; `ambre_server::hot_tail::tests::a_batch_keeps_each_number_and_frame` -- the encoding round trip; `ambre_server::ws::handler::message::hot_tail_tests::a_replay_reads_the_rows_and_each_batch_once` -- a replay after a move sends the bytes it sent before | none | verified |
| `SRV-276` | `POST` | The move of an inline delta into its batch object MUST write no index entry. | this spec section 3; `SRV-151` | `ambre_server::hot_tail::db_tests::a_move_is_a_heap_only_update` -- of two hundred inline deltas of 256 bytes, the 136 moved count as 136 heap-only updates in the table statistics, and the tail count past the move reads the 64 kept | none | verified |
| `SRV-259` | `POST` | Making one accepted delta durable, its move and its trim included, MUST write no more WAL than the ceiling of the delta index row of `ambre-spec-budgets.md section 3`. | this spec section 3; `ambre-spec-budgets.md section 3` | `ambre_server::hot_tail::write_cost::write_cost_short_run` -- 4 000 deltas over four documents of a steady index, the WAL of each step read through `pg_walinspect`, the records of another writer left out (`ambre_server::hot_tail::write_cost::a_record_of_another_writer_is_foreign`), under the ceiling of `scripts/write_cost.py` in the CI `test-db` job (`just write-cost`) | none | verified |

### 1.4 A history is folded and trimmed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-039` | `POST` | A delta a retained snapshot already contains MUST be removed from the index. | this spec section 3 | `ambre_server::db::tests::purge_superseded_updates_trims_covered_deltas` -- ten deltas and a snapshot covering six leave exactly the uncovered tail, correct and unreachable while `SRV-038` is unbuilt, since with no snapshot there is no horizon | none | verified |
| `SRV-083` | `FRAME` | A trim MUST NOT remove a delta the oldest retained snapshot does not cover. | this spec section 3 | `ambre_server::db::tests::purge_horizon_is_min_covered_seq` -- two snapshots covering 3 and 8 stop the trim at 3, not 8; `::create_snapshot_roundtrips_covered_seq` -- the horizon a snapshot records; `ambre_logic::authorship_tests::a_built_snapshot_names_the_last_delta_held_without_a_gap` -- a client never claims a delta past a gap | none | verified |
| `SRV-154` | `FRAME` | A trim MUST NOT drop a batch object that still holds a delta no retained snapshot covers. | this spec section 3; `SRV-083` | `ambre_server::hot_tail::db_tests::a_trim_drops_a_batch_only_once_no_row_names_it` -- a snapshot covering seven of ten deltas drops the batch of the first five and keeps the one still naming eight to ten | none | verified |
| `SRV-038` | `POST` | The relay MUST ask for a snapshot once a document's unfolded history passes its threshold. | this spec section 3; `ambre-spec-delta-order.md`; `ambre-spec-budgets.md section 6` | `ambre_server::compaction::db_tests::the_writer_is_asked_once_past_the_threshold` -- the writer is asked at the first check past the threshold, and not again while the ask stands, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`); `ambre_server::compaction::tests::the_history_past_the_served_snapshot_decides` -- the threshold counts from the served horizon | none | verified |
| `SRV-126` | `POST` | A snapshot MUST stay pending until another member client confirms it, or until its author confirms it. | this spec section 3; `AUTHOR-016`, `AUTHOR-021` | `ambre_server::routes::tests::a_snapshot_is_served_once_confirmed` -- an upload is pending, its author makes it `self` and another member `confirmed`; `ambre_server::compaction::db_tests::one_author_alone_stands_past_the_reference` -- a second author may not confirm alone past the reference, and another member still may, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-284` | `FAIL` | Given a confirmation whose signature does not verify under the confirmer's key, the relay MUST refuse it and keep the snapshot's standing. | this spec section 3; `AUTHOR-030`, `AUTHOR-031` | `ambre_server::routes::tests::a_confirmation_stands_only_under_its_confirmers_signature` -- a signature by another key, over another horizon, or absent is refused and nothing is served, and the signature kept is served with the snapshot, and with the reference beside a later `self` one, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-127` | `FRAME` | A trim MUST NOT count a pending snapshot. | this spec section 3; `SRV-083` | `ambre_server::compaction::db_tests::a_pending_snapshot_moves_no_trim` -- nothing goes before a confirmation, and the covered deltas go after it, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-128` | `POST` | A cold open MUST be served the latest confirmed snapshot. | this spec section 3; `AUTHOR-015` | `ambre_server::routes::tests::a_snapshot_is_served_once_confirmed` -- a pending snapshot is not served, and one confirmed is, with its standing | none | verified |
| `SRV-129` | `POST` | A pending snapshot MUST be offered for confirmation to a member client, other than its author, that has the document open. | this spec section 3; `AUTHOR-016` | `ambre_server::routes::tests::a_snapshot_is_served_once_confirmed` -- the document's topic hears the upload with its horizon | none | verified |
| `SRV-130` | `FRAME` | A trim after a confirmation by the author MUST NOT drop a delta another user signed past the reference snapshot. | this spec section 3; `AUTHOR-022` | `ambre_server::compaction::db_tests::a_self_snapshot_trims_its_author_alone` -- the other author's deltas past the reference all stay, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-131` | `FRAME` | A trim MUST NOT drop the reference snapshot: the latest one a member other than its author confirmed. | this spec section 3; `AUTHOR-022` | `ambre_server::compaction::db_tests::the_keep_pass_holds_the_reference_and_the_latest_self` -- a keep of one still holds the reference and the latest snapshot its author confirmed alone, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-133` | `POST` | A mismatch report MUST drop the pending snapshot it names. | this spec section 3; `AUTHOR-023` | `ambre_server::routes::tests::a_signed_report_drops_a_pending_snapshot_alone` -- the row and its object go, and a confirmed snapshot outlives a report | none | verified |
| `SRV-134` | `POST` | After a drop, the relay MUST ask a member client other than the author of the dropped snapshot for the next one. | this spec section 3; `SRV-038` | `ambre_server::routes::tests::a_signed_report_drops_a_pending_snapshot_alone` -- the reporter's own topic hears the request | none | verified |
| `SRV-200` | `POST` | A report that drops the final snapshot of a segment MUST clear its finalization in the same transaction, and keep its lock and its horizon. | this spec section 3; `SRV-114`; `5-Murmure/features/ambre-spec-murmure-segments.md` | `ambre_server::routes::channels::tests::a_dropped_final_snapshot_is_finalized_again` -- database tier, another snapshot's drop changes nothing, the final one's grants a lease at the same horizon at once, and the new commit holds | none | verified |
| `SRV-135` | `FRAME` | Dropping a snapshot MUST NOT drop a delta. | this spec section 3 | `ambre_server::routes::tests::a_signed_report_drops_a_pending_snapshot_alone` -- the three deltas the snapshot covered all stay | none | verified |
| `SRV-264` | `POST` | A compaction pass MUST keep at most the two most recent snapshots of a document, besides the ones `SRV-131` holds. | this spec section 3; `ambre-spec-budgets.md section 6`; `SRV-083` | `ambre_server::compaction::db_tests::a_pass_keeps_the_deltas_past_the_older_kept_snapshot` -- five snapshots confirmed at the threshold over 2 750 deltas leave the two latest snapshots and the 750 deltas past the older one, under the bound of the budget row, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-277` | `POST` | The compaction pass MUST run once each pass period of `ambre-spec-budgets.md section 6`, start to start: the snapshot threshold over the design delta rate. | this spec section 3; `ambre-spec-budgets.md section 6`; `SRV-264` | `ambre_server::compaction::tests::the_pass_period_lets_one_threshold_in_at_the_design_rate` -- at the design rate, a document adds one threshold of deltas between two passes, and no more; `::the_loop_runs_one_pass_each_period` -- on a paused clock, the loop runs no pass before the first period, then one pass each period over three periods | none | verified |

### 1.5 A client asks for what it missed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-040` | `POST` | A replay MUST answer every request it accepts, including one with nothing to send. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::empty_replay_still_sends_missing_updates` -- a document with no rows still emits one `MissingUpdates`, which is what releases the client's loading overlay; `ambre_server::ws::handler::message::delivery_tests::a_replay_over_a_broken_index_is_still_answered` -- an index that cannot be read is answered with a stop naming the caller's own cursor, not silence | none | verified |
| `SRV-041` | `POST` | A replay MUST deliver every delta the caller asked for that the relay holds, wherever it holds it. | this spec section 3 | `ambre_server::ws::handler::message::refusal_tests::a_replay_page_stops_at_the_first_unreadable_delta` -- a page ends at the first delta the relay cannot read, and the replay stops there; `ambre_server::ws::handler::message::hot_tail_tests::a_replay_reads_the_rows_and_each_batch_once` -- deltas in their rows and in a batch object; `ambre_server::ws::handler::message::delivery_tests::a_page_read_out_of_order_is_sent_in_order` -- deltas still pending, missed by the object store | none | verified |
| `SRV-084` | `POST` | A replay MUST deliver its deltas in the order of their numbers. | this spec section 3 | `ambre_server::ws::handler::message::delivery_tests::a_page_read_out_of_order_is_sent_in_order` -- the object store holds each read so the three finish in reverse order, and the page still carries them by number | none | verified |
| `SRV-155` | `POST` | A replay MUST read the inline deltas from the index, and fetch each batch object it needs once. | this spec section 3; `SRV-148`, `SRV-151` | `ambre_server::ws::handler::message::hot_tail_tests::a_replay_reads_the_rows_and_each_batch_once` -- six deltas, four of them moved, replay from their rows and one read of the batch | none | verified |
| `SRV-075` | `FRAME` | A page of the delta index served over a request MUST NOT exceed the cap the relay declares. | audit `M16`; this spec section 3 | `ambre_server::routes::tests::handler_tests::a_delta_page_never_exceeds_the_declared_cap` -- over 1,005 rows, a limit of 5,000 is served a thousand, and a limit of zero or below is served one; type-level -- every page query takes a `Bounded<QueryLimit>`, which holds no count outside one to a thousand; `ambre_server::routes::sync::tests::a_delta_page_query_clamps_its_limit` -- the route's own query clamps instead of refusing | none | verified |
| `SRV-085` | `FAIL` | A replay MUST be refused, and answered with nothing, when the caller's membership no longer holds. | this spec section 3; `ambre-spec-membership.md` | `ambre_server::ws::handler::message::access_tests::a_replay_after_a_removal_is_answered_with_nothing` -- a membership removed mid-session, then a replay asked past the mark: no update comes back | none | verified |
| `SRV-086` | `FAIL` | A second replay of one document within the throttle window MUST be refused, and answered with nothing. | this spec section 3 | `ambre_server::ws::handler::message::delivery_tests::a_second_replay_inside_the_window_is_answered_with_nothing` -- the first call is served, and the same call at once after it sends nothing | none | verified |
| `SRV-184` | `POST` | A replay that stops before its end MUST name the last delta it delivered. | this spec section 3; `ambre-spec-sync.md`, `SYNC-153` | `ambre_server::ws::handler::message::delivery_tests::a_replay_stopped_at_a_lost_delta_names_the_last_one_delivered` -- three deltas, the second lost: the first is delivered and the stop names it; `ambre_server::ws::handler::message::delivery_tests::a_replay_over_a_broken_index_is_still_answered` -- nothing delivered, so the stop names the caller's cursor | none | verified |

### 1.6 A segment closes into its successor

A full Murmure segment closes. The relay registers its successor, and names the one it kept to a
close that came later.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-194` | `POST` | A close by a writer of the channel MUST register the caller's successor as the active segment, and mark the old one `closing`. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-segments.md` | `ambre_server::db::channels::tests::a_close_registers_the_successor_and_marks_the_segment_closing`; `ambre_server::routes::channels::tests::a_writer_closes_a_segment_and_a_late_close_learns_the_winner`; `::a_close_needs_a_writer_and_its_own_successor` -- database tier, a viewer, a successor of another member and a segment closed into itself are refused | none | verified |
| `SRV-196` | `FAIL` | Given a close while a segment of the channel is still `closing`, or of a segment no longer active, the relay MUST refuse it. | this spec section 3 | `ambre_server::db::channels::tests::a_close_while_a_segment_is_closing_is_refused` -- database tier, both closes answer a conflict and nothing is registered | none | verified |
| `SRV-198` | `POST` | A close asked again with the successor it registered MUST be answered that successor, as the caller's own. | this spec section 3; `ambre-spec-sync.md`, `SYNC-164` | `ambre_server::db::channels::tests::a_close_asked_again_answers_its_own_successor` -- database tier, the second close answers the same successor and registers nothing | none | verified |
| `SRV-197` | `FRAME` | A close MUST NOT change the segments of any other channel. | this spec section 3 | `ambre_server::db::channels::tests::a_close_registers_the_successor_and_marks_the_segment_closing` -- database tier, a second channel keeps its one active segment | none | verified |

### 1.7 Two closes race

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-195` | `CONC` | Two closes of one active segment MUST register one successor, and each MUST be answered that successor. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md`, `MURMURE-011` | `ambre_server::db::channels::tests::two_racing_closes_register_one_successor`; `ambre_server::routes::channels::tests::a_writer_closes_a_segment_and_a_late_close_learns_the_winner` -- database tier, two closes at once answer one successor, and the late one registers nothing | none | verified |

### 1.8 A segment is finalized

A Murmure segment is a document the relay closes to deltas for good. The client writes the final
snapshot, and the relay arbitrates who writes it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-113` | `POST` | The relay MUST grant the finalize lease of a segment to one client at a time. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md` | `ambre_server::routes::channels::tests::a_lease_waits_out_the_grace_then_goes_to_one_writer_at_a_time` -- database tier, a second writer is answered the time left on the live lease; `::a_lease_needs_a_writer_and_a_closing_segment` -- a viewer and the active segment take none | none | verified |
| `SRV-114` | `POST` | Granting a finalize lease MUST lock the segment against deltas for good. | this spec section 3 | `ambre_server::routes::channels::tests::a_lease_waits_out_the_grace_then_goes_to_one_writer_at_a_time` -- database tier, the register records the lock at the last delta number, and the store mirrors it; `ambre_server::redis_ops::lua_tests::a_locked_segment_refuses_every_later_delta` -- a pending delta holds the lock back | none | verified |
| `SRV-290` | `POST` | The horizon a lock records MUST NOT be lower than the highest delta number the durable store holds for the segment. | this spec section 3; `SRV-114`; `SRV-288` | `ambre_server::routes::channels::tests::a_lease_after_a_store_reset_takes_its_horizon_from_the_register` -- database tier: after the ephemeral store loses the counter of a segment of five deltas, the finalize lease and the register both hold horizon 5 | none | verified |
| `SRV-115` | `FAIL` | Given a delta for a locked segment, the relay MUST refuse it with an answer that names the lock. | this spec section 3 | `ambre_server::redis_ops::lua_tests::a_locked_segment_refuses_every_later_delta` -- database tier, the delta takes no number and consumes no clock; `ambre_server::ws::handler::message::refusal_tests::a_delta_for_a_locked_segment_is_refused_with_the_lock` -- the refusal names the delta and the lock's code | none | verified |
| `SRV-116` | `FAIL` | Given a finalize commit from a client that holds no live lease, the relay MUST refuse it. | this spec section 3 | `ambre_server::routes::channels::tests::a_commit_needs_the_live_lease_and_a_snapshot_of_the_horizon` -- database tier, another lease and a snapshot off the horizon or of another document are refused; `::an_expired_lease_goes_to_another_writer` -- an expired lease commits nothing | none | verified |
| `SRV-117` | `POST` | An accepted finalization MUST record the finalization time of the segment. | this spec section 3; `ZK-015` | `ambre_server::routes::channels::tests::a_commit_needs_the_live_lease_and_a_snapshot_of_the_horizon` -- database tier, the segment is `final` with its time, and a retry is answered alike | none | verified |
| `SRV-118` | `POST` | The deltas of a final segment MUST be purged only once a member client other than its author confirms the finalization. | this spec section 3; `SRV-126` | `ambre_server::routes::channels::tests::a_final_segment_keeps_its_deltas_until_another_member_confirms` -- the author's confirmation leaves the final snapshot pending and the deltas in place, and another member's purges them | none | verified |
| `SRV-119` | `POST` | A lease that expires MUST let another client take the lease of the same segment. | this spec section 3; `ambre-spec-budgets.md section 8` | `ambre_server::routes::channels::tests::an_expired_lease_goes_to_another_writer` -- database tier, the second writer takes a new lease at the horizon the lock fixed | none | verified |
| `SRV-120` | `FRAME` | Finalizing a segment MUST NOT touch a delta of any other document. | this spec section 3 | `ambre_server::routes::channels::tests::a_commit_needs_the_live_lease_and_a_snapshot_of_the_horizon` -- database tier, the deltas of the segment and of `meta` all stay, and `meta` is not finalized | none | verified |
| `SRV-147` | `POST` | A rotation of a channel key MUST lock every segment of the channel sealed under the previous key. | this spec section 3; `SRV-114`, `SYNC-125` | `ambre_server::routes::channels::tests::a_rotation_locks_the_closing_segment_at_once` -- database tier, the register and the store hold the lock at the last delta number, and the lease waits out no grace; `::a_rotation_locks_the_active_segment_at_its_close` -- the active segment stays open until its close locks it; `ambre_server::redis_ops::lua_tests::a_rotation_locks_a_segment_with_a_delta_still_pending` -- the lock waits for no pending delta | none | verified |
| `SRV-199` | `FAIL` | Given a lease asked before the grace period has passed since the segment closed and since its latest delta, the relay MUST refuse it. | this spec section 3; `ambre-spec-budgets.md section 8` | `ambre_server::routes::channels::tests::a_lease_waits_out_the_grace_then_goes_to_one_writer_at_a_time` -- database tier, a fresh close and a delta an hour old each answer a wait, and lock nothing | none | verified |

### 1.9 A locked segment is amended

A late edit of a locked segment goes to an amendments document beside it. The relay keeps one per
segment, and names it to every writer that asks.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-201` | `POST` | A registration by a writer of the channel MUST link the caller's document to a locked segment of the channel as its one amendments document. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md`, `MURMURE-013` | `ambre_server::db::channels::tests::a_locked_segment_registers_one_amendments_document` -- database tier, a second ask answers the same document; `::an_unlocked_or_foreign_segment_registers_no_amendments` -- database tier, both answer a conflict and register nothing; `ambre_server::routes::channels::tests::a_writer_registers_the_amendments_and_a_late_one_learns_them` -- database tier, a viewer and a document of another member are refused | none | verified |
| `SRV-203` | `FRAME` | A registration MUST NOT change the state, the lock or the horizon of the segment it amends. | this spec section 3 | `ambre_server::db::channels::tests::a_locked_segment_registers_one_amendments_document` -- database tier, the segment keeps its state and its horizon | none | verified |

### 1.10 Two registrations race

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-202` | `CONC` | Two registrations for one segment MUST link one document, and each MUST be answered that document. | this spec section 3 | `ambre_server::db::channels::tests::two_racing_registrations_keep_one_amendments_document` -- database tier, both answer the one linked document; `ambre_server::routes::channels::tests::a_writer_registers_the_amendments_and_a_late_one_learns_them` -- database tier, the late caller reads `created: false` | none | verified |

### 1.11 Amendments fold into their segment

A writer folds the amendments of a final segment into a new snapshot of it. The relay arbitrates
who folds, and erases the amendments once another member confirms the fold.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-204` | `POST` | The relay MUST grant the fold lease of a final segment to one writer of the channel at a time. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-segments.md`, `MURMURE-075` | `ambre_server::routes::channels::tests::a_fold_lease_waits_for_a_fold_owed_and_its_delay` -- database tier, a second writer waits out the live lease, and a viewer takes none | none | verified |
| `SRV-205` | `FAIL` | Given a fold lease asked while the amendments hold no delta past the last fold, the relay MUST refuse it. | this spec section 3 | `ambre_server::routes::channels::tests::a_fold_lease_waits_for_a_fold_owed_and_its_delay` -- database tier, a segment without amendments and empty amendments are refused; `::a_fold_commit_needs_the_lease_and_a_snapshot_of_the_horizon` -- a fold of every delta refuses the next lease | none | verified |
| `SRV-206` | `FAIL` | Given a fold lease asked before the fold delay has passed since the last fold, or since the amendments were created, the relay MUST refuse it. | this spec section 3; `ambre-spec-budgets.md section 8` | `ambre_server::routes::channels::tests::a_fold_lease_waits_for_a_fold_owed_and_its_delay` -- database tier, fresh amendments answer a wait of about the delay; `::a_fold_commit_needs_the_lease_and_a_snapshot_of_the_horizon` -- the delay restarts at the fold | none | verified |
| `SRV-207` | `FAIL` | Given a fold commit without the live lease, with a snapshot off the horizon of the segment, or reaching no further than the last fold, the relay MUST refuse it. | this spec section 3 | `ambre_server::routes::channels::tests::a_fold_commit_needs_the_lease_and_a_snapshot_of_the_horizon` -- database tier, another lease and a short snapshot are refused, and a retry of the accepted commit is answered alike | none | verified |
| `SRV-208` | `POST` | A fold confirmed by a member other than its author MUST become the final snapshot of the segment. | this spec section 3; `SRV-118`; `5-Murmure/features/ambre-spec-murmure-segments.md`, `MURMURE-076` | `ambre_server::routes::channels::tests::a_confirmed_fold_becomes_final_and_erases_the_amendments` -- database tier, each confirmed fold is the final snapshot after it | none | verified |
| `SRV-209` | `POST` | A settled fold MUST delete every other snapshot of the segment. | this spec section 3; `ambre-spec-budgets.md section 8` | `ambre_server::routes::channels::tests::a_confirmed_fold_becomes_final_and_erases_the_amendments` -- database tier, the replaced final snapshot goes | none | verified |
| `SRV-210` | `FRAME` | A fold commit MUST NOT change the final snapshot of the segment. | this spec section 3 | `ambre_server::routes::channels::tests::a_fold_commit_needs_the_lease_and_a_snapshot_of_the_horizon` -- database tier, the final snapshot stays after the commit | none | verified |
| `SRV-211` | `POST` | A settled fold MUST delete the amendments only when it holds their last delta. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-segments.md`, `MURMURE-076` | `ambre_server::routes::channels::tests::a_confirmed_fold_becomes_final_and_erases_the_amendments` -- database tier, a delta past the fold keeps them, and a fold of them all erases them; `ambre_server::redis_ops::lua_tests::a_fold_seals_only_the_amendments_it_holds` -- a pending or a later delta holds the seal back | none | verified |
| `SRV-212` | `POST` | A report that drops a fold snapshot MUST leave the fold owed at once. | this spec section 3; `SRV-133` | `ambre_server::routes::channels::tests::a_fold_commit_needs_the_lease_and_a_snapshot_of_the_horizon` -- database tier, the drop grants the next lease with no delay | none | verified |

### 1.12 A root message takes one thread

The first reply to a message creates a thread document. The relay keeps one per root, and names
it to every writer that asks.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-213` | `POST` | A registration by a writer of the channel MUST link the caller's document to a root message of the channel as its one thread document. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-threads.md`, `MURMURE-105` | `ambre_server::db::channels::tests::a_root_registers_one_thread_document` -- database tier, a second ask answers the same document, and another root takes its own; `ambre_server::routes::channels::tests::a_writer_registers_a_thread_and_a_late_one_learns_it` -- database tier, a viewer and a document of another member are refused | none | verified |
| `SRV-215` | `FRAME` | A thread registration MUST NOT change any other document of the channel. | this spec section 3 | `ambre_server::db::channels::tests::a_root_registers_one_thread_document` -- database tier, every other row of the channel reads the same columns after both registrations | none | verified |

### 1.13 Two thread registrations race

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-214` | `CONC` | Two registrations for one root MUST link one document, and each MUST be answered that document. | this spec section 3; `MURMURE-108` | `ambre_server::db::channels::tests::two_racing_registrations_keep_one_thread_document` -- database tier, both answer the one linked document; `ambre_server::routes::channels::tests::a_writer_registers_a_thread_and_a_late_one_learns_it` -- database tier, the late caller reads `created: false` | none | verified |

* **Budget.** three rows of `ambre-spec-budgets.md section 3`. The live receive and broadcast
  under 5 ms, the ordering check under 1 ms, the flush batch under 2 s for a thousand deltas.
  All three read **none exists**, as does the cold start of the same table.
* **Boundary.** the relay orders bytes it cannot read. What it learns is the routing set of
  `ZK-007`: which key wrote to which document, at which clock, and how large the delta was.
  `SRV-001` keeps the delta out of the logs.
* **Verification.** 85 `verified`. The ordering, the storage and the replay hold. So does each step
  of a segment's life, from its close to its fold, and the thread of a root. The recovery paths are
  driven through real failures: a register that refuses inserts, a queue that refuses pushes, an
  index that cannot be read.
---

## 2. WHAT EACH USER SEES

Two peers over one document. Every row is about order, which is the only thing the relay decides.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| edits twice | edits once | three deltas, three numbers, one order | the same document, in the same order, on both screens (`SRV-006`) | `redis_ops::lua_tests::independent_clients_have_independent_clocks` |
| edits, and her delta is replayed by an attacker | watches | one delta | the replayed delta is refused: its clock is already spent (`SRV-006`) | `redis_ops::lua_tests::duplicate_clock_rejected` |
| edits while Bob is offline | reconnects | every delta, indexed | Bob gets exactly what he missed, in order (`SRV-041`, `SRV-084`) | `ws::handler::message::delivery_tests::a_page_read_out_of_order_is_sent_in_order` |
| edits nothing while Bob is away | reconnects with a current snapshot | nothing new | Bob's document opens rather than spinning (`SRV-040`) | `ws::handler::message::delta_size_cap_tests::empty_replay_still_sends_missing_updates` |
| edits for a year | opens the document | the snapshot the relay asked for, and the deltas past it (`SRV-038`, `SRV-039`) | Bob replays the tail past the snapshot, never the whole year | `compaction::db_tests::the_writer_is_asked_once_past_the_threshold` |
| edits during a reset of the ephemeral store | watches | the durable history alone, then the rebuilt state | Alice's next edit is accepted rather than refused for ever (`SRV-032`) | `redis_ops::redis_integration_tests::seed_clock_state_unblocks_the_next_update_after_a_redis_reset` |
| edits in two tabs at once | watches | both edits, the second tab's under a later clock | Bob sees both edits, in the order Alice's tabs sent them (`SRV-169`) | `ws::handler::message::hot_tail_tests::a_delta_behind_a_refused_clock_waits_for_the_answer` |

---

## 3. MODEL

**CRDT containers.** none. What is ordered here is a sealed envelope; what converges inside it is the
client's business (`ambre-spec-delta-order.md`).

**Two counters, one indivisible step.** The relay keeps a clock per author and a number per
document, and moves both together or neither.

```text
given a document, an author key and an incoming clock:
1. current := the author's last accepted clock for this document, or zero
2. if incoming is not current + 1: refuse, and report the current value
3. record incoming as the author's last accepted clock
4. return the document's next number, incremented
```

The step runs inside the ephemeral store, so no two deltas can interleave between the check and the
increment (`SRV-080`). The exact script is in the source.

**Two sessions of one author.** Two tabs or devices of one user sign with one key, so they share one
clock. A delta each sends before it sees the other's echo is refused once. The refusal names the
delta by its signature, and goes to that socket alone (`SRV-037`). The relay then fences the
document on that socket: it drops each later delta unanswered, with no clock and no number.

The author answers with the refused delta's signature, and the answer lifts the fence (`SRV-169`).
The author then seals that delta and every later one again from the expected clock (`SYNC-147`).
Without the fence, a later delta would take the refused clock's place. Peers would then see one
author's deltas out of order.

**Why the author clocks are never pruned.** The hash grows one entry per authoring key, which is
bounded by the document's members. Expiring it would reset an author to zero, and a returning client
holding its own advanced clock would then be refused for ever. Removing one entry would reopen the
replay window the clock exists to close. The bound is the membership, and that is accepted.

**What survives a trim.** A rebuild reads the durable history, and the trim removes part of it. So
each delete of the trim raises a high-water mark per author of the rows it removes: the highest
clock and the highest number. A rebuild takes the larger of the rows and the marks (`SRV-300`), and
the counter also reaches the highest snapshot horizon (`SRV-288`). Until a subscribe rebuilds a lost counter,
it reads 0, and the relay refuses every horizon past it (`SRV-289`).

**What happens to an accepted delta**, in order. Each step's failure has its own answer, and none of
them un-assigns the number.

| Step | On failure |
|---|---|
| the number is assigned | the delta is refused, and nothing has moved (`SRV-006`) |
| the tombstone is read, before anything is stored | a delete has begun: the delta takes no object, no row and no broadcast, and its author hears that the document is not found (`SRV-335`). A store that does not answer reads as no tombstone |
| the durable index gains a row | tried three more times, a third of a second in all (`SRV-033`); past that, the whole delta goes to a dead-letter list for an operator (`SRV-082`) |
| a delta without an inline home gets its object, before its index row names it | its index row takes the frame and names no object; the relay logs and counts the refusal (`SRV-034`). A store that does not answer within the wait of the delta object row of `ambre-spec-budgets.md section 3` counts as a refusal (`SRV-336`). The tombstone is read again once the object is written, before the row |
| the delta is fanned out live | logged; a reconnecting client replays it from the index (`ambre-spec-broadcast.md`) |

**A delete can begin after the number.** The clock script refuses a document whose tombstone is set
(`ambre-spec-server.md`, `SRV-017`). A delete may set it once the script has run. So the relay reads
the tombstone again before it stores the delta. A delta without an inline home reads it once more
after its object write (`SRV-335`).

Each read guarantees one thing: a tombstone set before it stops every write after it. A delete that
begins after the last read still meets a row and a broadcast. The late-write sweeps remove the row
and the object (`SRV-273`, `SRV-057`). That broadcast reaches only the subscribers of the document,
who hold its key already. A reset of the ephemeral store loses the tombstone until the next pass
sets it again. The sweeps then hold alone.

**Where a delta lives.** A delta is a few hundred bytes, so one object per delta costs a request for
almost nothing. A delta under the inline size lives in its index row (`SRV-148`), and a replay
reads a page of rows in one query (`SRV-155`). An oversized delta keeps an object of its own
(`SRV-149`).

**No acknowledged delta lives in the ephemeral store alone** (`SRV-307`). The managed Redis of
Scaleway does not guarantee a recovery after an incident. So the relay writes the object of an
oversized delta before the row names it, and before the echo. The write costs one request to the
object store on that delta alone, which the delta object row of `ambre-spec-budgets.md section 3`
bounds (`SRV-336`). A refused write puts the frame in the row, which Postgres keeps.

**The index is keyed by the document and the number.** No query reads any other key of a row. The
number grows within a document, so an insert writes the last page of that document's range. A
random key wrote a random page of its own index instead, and that page went whole into the WAL
after each checkpoint. A number taken twice fails the insert, and the retry and the dead letter of
`SRV-033` and `SRV-082` take the delta (`SRV-263`). Before, two rows held one number, and a replay
from that number skipped one of them.

**An insert locks no document row.** The index carries no foreign key to the register. The check of
such a key locks the document row. That costs one more WAL record per insert, and a wait between
two writers of one document (`SRV-274`). The relay numbers a delta only for a member of a live
document, and a delete fences the document before any row goes. So the key held no rule the relay
does not hold already.

A trigger on the delete of a document row removes the rows of the index, the work the cascade did
(`ambre-spec-server.md`, `SRV-275`).

**Postgres keeps the hot tail alone.** A document holds at most the inline cap in its rows
(`SRV-150`). Past it, the oldest deltas move into one batch object (`SRV-151`), and so does the
whole tail of a document idle for a day (`SRV-152`). Compaction trims the rest. So the inline bytes
stay bounded by the documents in use at once, never by every document ever written. A batch
object is dropped only once a retained snapshot covers every delta in it (`SRV-154`).

**The move writes no index entry** (`SRV-276`). Postgres writes an update in place on its page,
with no index entry, when no index reads a column it changes. So no index reads the frame or the
batch key.

**What reads the primary key instead.** The tail count reads the range of the document past its
newest batch row. A frame put in a row whose object the store refused (`SRV-034`) falls out of that count. The
trim asks whether a batch is still named within the range of its own document. The idle pass
reads the whole table, once each pass period (`SRV-277`).

**Why half of each page stays free.** Inside the move, the old and the new version of each moved
row share the page. A frame under about 100 bytes,
or one stored out of line, is shorter than the batch key that replaces it. Its move can then leave
the page and write its index entries.

**Why the cap may be passed.** The move starts at three quarters of either bound and keeps a
quarter inline, so a writer seldom meets the cap. The count and the insert are one statement, so
writers racing at the cap may each see room, and pass it by one delta each. A lock per document
would close that race at the price of three more round trips on every write.

**The flush.** The write path queues no delta now. The flush drains the lists an older relay
queued.

One task per instance, every fifteen seconds, over the documents marked unflushed
rather than over the whole keyspace. Each document is drained under a lock of its own (`SRV-036`).
The drain advances a single front counter over every entry it has finished with, stored or
permanently unreadable. A trim therefore never passes an entry still owed a retry (`SRV-035`).

**The sweep.** Every twentieth cycle the whole keyspace is swept once, so a queue that missed its
mark is still found. On a cluster the sweep reads every primary in turn (`SRV-255`).

**Why the front counter counts two outcomes.** An entry that will never parse and an entry that was
stored are both finished with. Counting only the stored ones desynchronised the trim. A bad entry
ahead of a good one re-stored the good one every cycle, and a bad entry at the end was retried
for ever.

**A snapshot, and the horizon it moves.** A client seals its whole state and puts it in object
storage. It records it against the document with the highest delta number it had folded in, and
that number is the horizon.

A snapshot is pending until it is confirmed (`SRV-126`). The relay offers it at once to a member who
has the document open (`SRV-129`), so two active members confirm within seconds. An author may
confirm its own snapshot. The relay then keeps the reference snapshot (`SRV-131`) and every delta
another user signed past it (`SRV-130`), and drops the author's own. So the deltas kept stop
growing while one author writes alone (`SRV-132`).

A confirmation carries the confirmer's signature and the digest of the blob checked. The
signature covers the document, the snapshot, its horizon, its author and that digest. The relay
refuses one that does not verify (`SRV-284`). It keeps another member's signature and serves it
with the snapshot, since a cold open trusts nothing else (`AUTHOR-030`). The relay keeps no
digest. A cold open computes it again from the blob it downloads, so a digest of another blob
fails there (audit `W101`).

Only a confirmed snapshot is retained, and moves the trim (`SRV-127`). A mismatch report drops
the pending snapshot and keeps every delta (`SRV-133`, `SRV-135`). The next snapshot comes from
another member (`SRV-134`). The trim
removes every delta at or below the lowest horizon of the snapshots the document still keeps
(`SRV-039`, `SRV-083`). Each pass keeps the two most
recent, and never drops the reference nor the latest snapshot its author confirmed alone (`SRV-131`,
`SRV-264`).

**Why two snapshots.** No client is ever served an older snapshot: an open gets the latest one
(`SRV-128`). The older snapshot covers an open that was served it just before a newer one was
confirmed. That open still pages the deltas past the older horizon, and a pass that kept one
snapshot would trim them under it. So an active document keeps one threshold of deltas, plus the
ones past its latest snapshot: the delta rows kept row of `ambre-spec-budgets.md section 6`.

**When the pass runs.** Each instance runs one pass each pass period, start to start (`SRV-277`).
The period is the time a document at the design delta rate takes to cross the snapshot threshold.
So a document at that rate adds one threshold of deltas between two passes. The pass period row of
`ambre-spec-budgets.md section 6` gives the figure.

**A dropped snapshot** is announced on the document's topic, with its author and its reporter. A final
snapshot takes its finalization with it, and the segment keeps its lock and its horizon
(`SRV-200`).

**What triggers it.** Every thirty-second delta, the relay compares the document's number with the
horizon of its latest served snapshot. Past the threshold of `ambre-spec-budgets.md section 6`, it
asks the writer of that delta for a snapshot (`SRV-038`). One ask stands for five minutes, so a
client that never answers is asked again at a later crossing. The relay refuses a recorded horizon
past the last number it gave, since a snapshot cannot fold in a delta that does not exist yet.

**Why only one author confirms alone.** Two authors confirming alone past one reference would each
drop their own deltas. Neither snapshot then holds the other's, and the deltas that proved them are
gone. So an author's own confirmation waits while another author's stands past the reference.

**A close.** A writer creates the successor document, then asks the relay to close the active
segment into it. One transaction marks the old segment `closing` and registers the successor as
`active` (`SRV-194`). The database holds one of each per channel, so of two racing closes one
registers, and the other is answered the winner (`SRV-195`). A close while a segment is still
`closing` is refused (`SRV-196`): only a finalization ends that state.

**A finalization.** A Murmure segment ends as one immutable snapshot
(`5-Murmure/ambre-spec-murmure-schema.md`).

1. A writer of the channel asks for the finalize lease of a closing segment. The relay answers a
   wait until the grace period has passed since the close and since the latest delta (`SRV-199`).
2. The first lease locks the segment for good, once no delta of it waits to be flushed. The lock
   is set in the ephemeral store first, then recorded with the last delta number, the horizon
   (`SRV-114`). The horizon is never under the highest number the register holds, since a reset
   of the ephemeral store loses its counter (`SRV-290`). A later delta is refused with an answer
   that names the lock, so the client sends the edit elsewhere (`SRV-115`).
3. The relay grants the lease to one client at a time (`SRV-113`), and answers another the time
   the live lease has left. A lease that expires gives way to the next one asked (`SRV-119`).
4. The client uploads the final snapshot, covering exactly the horizon, and commits it under the
   lease. The relay refuses a commit without the live lease (`SRV-116`). An accepted one marks the
   segment `final` with the finalization time (`SRV-117`), and writes no other row (`SRV-120`).
5. The deltas stay until another member client confirms the snapshot against them (`SRV-118`).
   A false snapshot therefore loses nothing, however long the confirmation takes.

The lease names the segment by its `Id<Segment>`, and the commit carries the lease and its
`Id<Snapshot>`, whose horizon the relay checks. The durations are rows of
`ambre-spec-budgets.md section 8`.

**A rotation's lock.** A rotation of a channel key flags each open segment of the channel
(`SRV-147`). The relay locks the closing one at once, and the active one at its close. This lock
waits for no pending delta, so its horizon may name a delta not yet flushed. Its lease then waits
out no grace period. A lock the store missed is set again at the next close or lease request.

**An amendments link.** A writer creates a document, then asks the relay to link it to a locked
segment of the channel. One transaction, under the channel row lock, links it as that segment's
amendments document (`SRV-201`). Of two racing links one is kept, and the other caller is answered
it and deletes its own (`SRV-202`). A segment not yet locked takes none: its deltas still enter. The
relay learns that a segment was amended, never what changed.

**A thread link.** A writer creates a document, then asks the relay to link it to a root message
of the channel. One transaction, under the channel row lock, links it as that root's thread
(`SRV-213`), and touches no other document (`SRV-215`). Of two racing links one is kept, and the
other caller is answered it (`SRV-214`). The relay learns the identifier of the root, never its text.

**A fold.** A delete in the amendments erases nothing while the final snapshot still holds the
text. So a writer folds the amendments into a new snapshot of the segment.

1. A writer asks for the fold lease of a final segment. The relay refuses it while the amendments
   hold no delta past the last fold (`SRV-205`). It answers a wait until the fold delay has passed
   since the last fold, or since the amendments were created (`SRV-206`).
2. The relay grants the lease to one writer at a time (`SRV-204`). The lease names the amendments
   and their last delta number, which the fold holds. The amendments still take deltas meanwhile.
3. The writer uploads the fold snapshot at the horizon of the segment, and commits it under the
   lease (`SRV-207`). The final snapshot stays until a confirmation (`SRV-210`).
4. Another member's confirmation makes the fold the final snapshot (`SRV-208`). Every other
   snapshot of the segment goes (`SRV-209`).
5. The relay then seals the amendments, unless a delta came past the fold, and deletes them
   (`SRV-211`). The confirmation names them, so the confirmer drops them from `meta`. A later
   delta waits for the next fold.

A report that drops the fold snapshot leaves the fold owed at once (`SRV-212`). The delay is a row
of `ambre-spec-budgets.md section 8`.

**A replay.** A client asks for everything after a number it names.

1. refuse, and answer nothing, on three counts: no subscription, a replay under a second old
   (`SRV-086`), or a membership that no longer holds (`SRV-085`);
2. load the still-queued deltas once, as a map from number to delta;
3. read the index in pages, in order;
4. take each page's inline deltas from its rows, and fetch once each batch object and oversized
   delta it names (`SRV-155`). Fall back to the map for what has not flushed yet, and re-sort the
   page by number;
5. send one answer per page, and one empty answer when the first page is empty (`SRV-040`);
6. on an index or a delta that cannot be read, stop, and name the last delta delivered
   (`SRV-184`). The caller asks again from there (`ambre-spec-sync.md`, `SYNC-153`).

**Why the first empty page still answers.** A client treats the arrival of a replay as the end of
its hydration, so silence leaves a document loading for ever. A later empty page simply ends the
paging: the earlier pages already released the client.

**Code.**

* `crates/ambre-server/src/redis_ops.rs` -- the ordering script, the rebuild, the queue and the
  dead-letter list;
* `crates/ambre-server/src/ws/handler/message.rs` -- the write path and the paged replay;
* `crates/ambre-server/src/s3.rs` -- the flush, its lock and its front counter;
* `crates/ambre-server/src/db/content.rs` -- the index, the snapshot record and the trim;
* `crates/ambre-server/src/routes/sync.rs` -- the snapshot record and the delta page over requests;
* `crates/ambre-server/src/compaction.rs` -- when the relay asks for a snapshot;
* `crates/ambre-server/src/main.rs` -- the compaction pass, at the period of `compaction.rs`.

---

## 4. SURFACE

The sequencer has no surface of its own: it is reached by a delta or by a request. What a person
sees is how long a document takes to open.

| Trigger | Behaviour |
|---|---|
| a client sends a sealed delta | it is checked, numbered, indexed, queued and fanned out |
| a client reconnects and asks for what it missed | one paged replay, throttled to one per second per document |
| a client asks for the current snapshot record | the most recent one somebody confirmed, with its standing, or nothing |
| a member confirms a snapshot | it is served from then on, as `self` when its author confirmed it alone |
| a member reports a pending snapshot under its signature | the snapshot is dropped, every delta stays, and the reporter is asked for the next one |
| a document's history passes the threshold | the writer of the delta that crossed it is asked for a snapshot |
| a client asks for a page of the index over a request | up to a thousand rows, after a number it names (`SRV-075`) |
| a client records a snapshot it has uploaded | the row is written and every subscriber is told one is available |
| the hour turns | the oldest snapshots beyond ten are removed, and the index is trimmed to the horizon |
| a writer asks for the finalize lease of a closing segment | a wait, or the lease with its horizon, the segment then locked for good |
| a writer commits a final snapshot under its lease | the segment becomes `final`, and every delta stays |
| a writer links a document to a locked segment | the one amendments document of the segment, its own or an earlier one |
| a writer asks for the fold lease of a final segment | a refusal with nothing to fold, a wait, or the lease with the amendments and their reach |
| a writer commits a fold snapshot under its lease | the fold is recorded, and the final snapshot stays |
| another member confirms a fold snapshot | it becomes final, the older snapshots go, and the amendments are erased when the fold holds them all |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The ordering check and the numbering are one step in the ephemeral store | checking in the process and numbering in the store | it is the same code with a race in it: two round trips leave a window in which a second delta from the same author passes the same check, and the whole anti-replay property rests on that window not existing. One script closes it without a transaction on the durable store |
| The author clock hash is never expired | a lifetime on the hash | it trades an unbounded key for a permanent lockout: a reset to zero locks a legitimate author out of its own document for ever, because its own clock only moves forward. The hash is bounded by the membership, so the growth is bounded already |
| The number is assigned before the delta is durable | assigning after the durable write | it makes every store failure a client failure with no retry, because a client cannot retry at the same clock. Assigning first and dead-lettering a failure keeps the sequence intact and turns a durability failure into an operator's task |
| The object of an oversized delta is written before the echo | queueing it in the ephemeral store for the flush | the managed Redis may lose its data in an incident, and the queue held an acknowledged delta for up to fifteen seconds while its row already named the object |
| One flush task per instance, over a marked set | a task per active document | it is a thousand idle tasks on a thousand open documents |
| One flush task per instance, over a marked set | a keyspace scan every cycle | it costs a scan the store does not owe us, every cycle, whatever was written. A set of unflushed documents is one read |
| The drain takes a lock per document | letting both instances drain | it is safe until the second instance exists: two instances draining one queue both trim by their own count, which removes entries a client queued in between, so a delta already numbered and already acknowledged would be gone. The write path never takes the lock, so collaboration is untouched |
| Each pass keeps two snapshots per document | ten | no client reads a snapshot past the latest. The trim stops at the oldest kept, so an active document kept about 4 500 rows in the index, 108 MB of heap and 83 MB of indexes for 32 documents on 2026-11-13. The delete then wrote full pages of those cold rows |
| Each pass keeps two snapshots per document | one | an open served the previous snapshot a moment before the pass still pages the deltas past it. The trim would delete them under that replay |
| The pass runs once each pass period: the snapshot threshold over the design delta rate | once an hour | an active document adds the deltas of up to one hour between two passes on top of the delta rows kept: 18 000 rows at the design rate of `ambre-spec-budgets.md section 6` |
| The trim is three deletes, one per rule, in one transaction | one delete that joins the three rules with `OR` | the planner cannot use the key of `updates` under the `OR`: it read every row and ran three correlated subqueries for each. On 2026-11-16, over 2 000 documents of 1 000 rows, it took 10.8 s against 0.42 s, deleting nothing, by `EXPLAIN ANALYZE` on the database tier |
| The replay is paged, and each page is fetched together | one batch with a hard cap | it silently truncated any client further behind than the cap: it applied the batch, declared itself hydrated and never asked again |
| The replay is paged, and each page is fetched together | one object fetch at a time | the serial round trip dominated a cold open. Fetching a page's objects together removes it |
| A replay is throttled per document | no throttle | it makes the cheapest wire message the most expensive request: a request for everything since the first delta is one small message that costs the relay the whole history from object storage. One replay per reconnect is the real need, and a second within the same second is not one |
| A pending snapshot moves no trim | trimming as soon as a snapshot lands | a false snapshot would replace the signed deltas that could prove it false |
| The deltas of a final segment wait for a confirmation | purging them after a delay | a false snapshot nobody checked in time would become the only record, and every cold open would trust it |
| The relay offers a snapshot for confirmation | waiting for the next cold open | a document whose other members keep it open would never be confirmed, and its deltas would pile up |
| A report drops the snapshot at once | the relay weighing the report against the snapshot | the relay reads neither, and the deltas it keeps settle the question at the next snapshot |
| The index is keyed by the document and the number | a random key per row | no query reads it, and each insert writes a random page of its index. That page goes whole into the WAL after each checkpoint |
| An insert locks no document row | a foreign key from the index to the register | its check locks the document row on every insert, for a rule the membership check and the tombstone already hold. The lock writes WAL, and two writers of one document share it through a multixact |
| The inline tail count reads the primary key above the newest batched number | a partial index on the inline rows | it repeated the columns of the primary key and wrote one more leaf entry per inline insert: about 100 B of WAL per delta, 4.6 % of the short run of `just write-cost` on 2026-11-16. On the same day, on a document of 4 000 rows of which 200 inline, the count read 18 buffers with the index, 37 above the batched number without it, and 102 over every row of the document. The idle sweep took 40 ms with it and 57 ms without it, over 200 such documents |
| A small delta lives in its index row | one object per delta | a replay of 250 deltas costs 250 object requests for a few hundred bytes each |
| A small delta lives in its index row | batch objects for every delta | a partly trimmed batch must stay, and a live document rewrites its batch at every flush |
| The move writes no index entry | a partial index of the inline rows, and one of the batch rows | each index reads a column the move changes, so each move wrote a new row version and an entry in every index: the move cost as much as the insert |
| The move writes no index entry | a batch reference shorter than the object key | it saved about 60 bytes per moved row, and the fillfactor it allows rose by 10 points only, in a trial of 2026-11-16 |
| The hot tail is capped, and an idle one moves out | every delta inline, trimmed by compaction alone | a document shared and then left pending would keep its whole tail in the register |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A snapshot check reads the whole history of each peer.** It compares every operation from
  the first one, not only those past the last snapshot it adopted. Its cost grows with the
  document, and no budget row bounds it yet.
* **A delta parented on a snapshot between the reference and the latest may stay quarantined.**
  An open that refuses the latest snapshot stays on the reference, and never learns the snapshots
  in between (`AUTHOR-022`). No rule says how it releases what rests on them.
* **An offline open never checks the snapshot it held back.** With no replay, the check never
  runs, and the document stays on its reference until the next open.
* **A warm client below the trim cannot catch up.** A client that kept a document open offline
  asks on reconnect for the deltas past the last number it holds. Once the trim has passed that
  number, the replay stops at the first missing delta, and nothing sends the client to the
  snapshot instead (`SRV-041`). No rule says what it should do.
* **A lost delta cannot be replayed.** A page stops at the first delta the relay cannot read, and
  names the last one before it. Nothing restores a delta the object store lost after accepting it,
  so every retry meets the same gap. The client gives up after its budget (`SYNC-154`), and no
  rule says what it does with a hole that never closes.
* **The dead-letter list is written and never read.** It holds the whole signed delta, so a replay
  is possible (`SRV-082`). Nothing drains it, counts it or exposes it over a route, so that replay
  is an operator reading the ephemeral store by hand.
* **The snapshot chain proof is stored and never checked.** The record carries a parent hash and a
  chain proof, and the relay validates neither -- which it could not, being blind. Whether any
  client checks them is `ambre-spec-delta-order.md`.
* **A rotation's lock may precede a flush.** Its horizon names deltas still pending (`SRV-147`).
  A confirmation that purges before such a delta is flushed leaves its row behind the purge.
* **A delta may enter a segment past its grace check.** The grace is read, then the lock is set.
  A delta numbered between the two enters the final segment, before the grace has passed since it.
  The window is one transaction, and the delta is kept, so only the grace is shortened.
* **A store that loses the lock forgets it until a subscribe.** The register holds the lock for
  good, and a subscribe writes its mirror again. A delta sent between a store reset and the next
  subscribe is numbered, and the final snapshot does not cover it.
* **No test drives a failing store.** Every recovery path here is reached only when the index, the
  queue or object storage refuses a single operation. The suite has no way to make one refuse
  (`ambre-spec-testing.md section 9`).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Why a client waits for a parent, and what it holds meanwhile | `ambre-spec-delta-order.md` |
| What a client puts inside the delta this document numbers | `ambre-spec-sealed-envelope.md` |
| Where a sealed envelope is stored, and under which name | `ambre-spec-object-store.md` |
| Who receives the delta live, and over what | `ambre-spec-broadcast.md` |
| Who may write at all | `ambre-spec-server.md`, `SRV-008` |
| The signature the delta carries | `ambre-spec-server.md`, `SRV-007` |
| What the relay may learn from an ordered delta | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
