# AMBRE -- FEATURE: CALL MEDIA

**Type:** Horizontal (framework)
**Implementation:** partial -- the relay admits, the SFU forwards RTP; the seal and the keys run on the host

> The audio and video of a call, encrypted end to end. The browser captures and encodes, a Rust
> worker seals each chunk, and a blind selective forwarding unit (SFU) in Rust forwards what it
> cannot open.

A call starts from a Murmure channel, and that gesture belongs to
`5-Murmure/features/ambre-spec-murmure-calls.md`. This document owns the sound and the picture once
a member joins. It owns the chunk seal, the sender keys, the worker that holds them, the SFU and the
path of a chunk to the canvas.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CALL`, and it belongs to
this document alone.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-001` | `INV` | A chunk of a call MUST leave the client sealed under a sender key of that call. | README pillar "E2EE"; this spec section 3 | not built | unbuilt -- lands with the `Call worker and call client` Story (`ambre-roadmap.md` E5). The seal itself is `CALL-054` | unimplemented |
| `CALL-054` | `INV` | The seal of a chunk MUST encrypt it under the key that the sender key its identifier names derives. | this spec section 3, the chunk format; `CALL-001` | `ambre_crypto::call::tests::the_sealed_bytes_of_a_known_key_follow_rfc_9605_and_stay_pinned` -- a second path written from RFC 9605 alone; `::a_sealed_audio_chunk_opens_whole_under_its_sender_key` | none | verified |
| `CALL-002` | `INV` | A sender key MUST NOT derive from a channel key or a document key. | this spec section 3; `MURMURE-157` | type-level -- `ambre_crypto::call::SenderKey` has three doors: `generate`, which draws from the generator of the OS, `ratcheted` and `unwrap_sender_key`. None takes a key or bytes; `ambre_crypto::call::tests::a_sender_key_is_drawn_fresh_and_a_ratchet_hides_the_key_before_it` | none | verified |
| `CALL-003` | `INV` | The seal and the open of every chunk MUST run in the call worker. | this spec section 3; `ARCH-004` | not built | unbuilt -- lands with the `Call worker and call client` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-004` | `INV` | The call worker MUST hold no key beyond the sender keys of its current call. | this spec section 3; `ZK-011` | not built | unbuilt -- lands with the `Call worker and call client` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-005` | `INV` | One sender key MUST NOT seal two chunks under one counter. | this spec section 3 | type-level -- `ambre_crypto::call::ChunkSealer` issues the counter of each seal, and no caller hands it one; `ambre_crypto::call::tests::one_key_never_seals_two_chunks_under_one_counter` | none | verified |
| `CALL-006` | `INV` | The clear prefix of a sealed chunk MUST be authenticated by its seal. | this spec section 3 | not built | unbuilt -- lands with the `Video calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-007` | `INV` | Every audio chunk of one call MUST be sealed at one fixed size. | this spec section 3; `ZK-021` | `ambre_crypto::call::tests::every_audio_chunk_has_one_size` -- one size in every call; `::a_clear_chunk_past_the_fixed_size_is_refused` | none | verified |
| `CALL-008` | `INV` | A client MUST carry the audio and video of a call over one peer connection to the SFU. | this spec section 3 | not built | unbuilt -- lands with the `Call worker and call client` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-009` | `INV` | Two clients of one call MUST NOT connect to each other. | this spec section 3; `ZK-021` | not built | unbuilt -- lands with the `Call worker and call client` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-010` | `INV` | The peer connection of a call MUST NOT open a data channel. | this spec section 3 | not built | unbuilt -- lands with the `Call worker and call client` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-011` | `INV` | A client MUST attach the chunk transform to a sender before it attaches a track to it. | this spec section 3; `CALL-001` | not built | unbuilt -- lands with the `Call worker and call client` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-012` | `INV` | The SFU MUST NOT link the crate that opens a chunk. | this spec section 3; `ZK-004` | cargo-deny -- `deny.toml` grants `ambre-crypto`, which opens a chunk, to `ambre-sync` and `ambre-logic` alone, so a dependency of `ambre-sfu` on it fails the CI `deny` job | none | verified |
| `CALL-013` | `INV` | The SFU MUST NOT link a C cryptography library. | README pillar "Rust-first"; this spec section 3 | ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads the graph `cargo metadata` resolves and refuses a C cryptography library on any edge it resolves from `ambre-sfu` (`ambre_primitives::boundaries::tests::a_c_cryptography_library_under_the_sfu_is_refused`) | none | verified |
| `CALL-014` | `INV` | The relay MUST NOT hold the secret that mints the credentials of the TURN server. | `ambre-spec-server.md`, `SRV-012`; this spec section 3 | ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses a crate that depends on `ambre-sfu`, whose secrets module alone holds the TURN secret (`ambre_primitives::boundaries::tests::a_crate_that_depends_on_the_sfu_is_refused`); `ambre_sfu::secrets::tests::a_turn_password_is_the_hmac_coturn_checks`; `ambre_belt::call::tests::a_browser_on_tcp_443_alone_decodes_the_call_through_turn` -- browser tier: coturn admits the credential the SFU minted, under the secret the two share | none | verified |
| `CALL-015` | `OBS` | The render runtime MUST draw a remote video, never a page element over the canvas. | this spec section 3; `CANVAS-001` | not built | unbuilt -- lands with the `Video calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-031` | `INV` | A participant MUST complete the DTLS handshake with the SFU through the RustCrypto backend alone. | this spec section 3; `CALL-013` | `ambre_sfu::tests::a_participant_completes_dtls_with_the_sfu_over_rust_crypto` -- an in-memory link between a browser connection and the SFU reaches the connected state, and a wrong fingerprint fails it | none | verified |
| `CALL-032` | `INV` | The SFU MUST forward each RTP packet of a participant to every other participant of its room whose slots carry its track, its payload unchanged. | this spec section 3, the SFU | `ambre_sfu::forward::tests::each_packet_reaches_every_other_participant_of_the_room_unchanged` | none | verified |
| `CALL-036` | `INV` | The SFU MUST serve its signalling on a port of its own, apart from the relay. | `ambre-spec-broadcast.md`, `SRV-049`; this spec section 3, signalling | `ambre_belt::call::tests::two_browsers_decode_each_other_through_the_sfu` -- browser tier | none | verified |
| `CALL-037` | `OBS` | Two browsers in one room MUST each decode the audio and the video of the other through the SFU. | this spec section 3, the stack | `ambre_belt::call::tests::two_browsers_decode_each_other_through_the_sfu` -- browser tier, under Chrome and under Firefox | none | verified |
| `CALL-039` | `INV` | The SFU MUST switch a receiver to another layer of a track only on a keyframe of that layer. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_receiver_switches_layer_only_on_a_keyframe_of_the_new_layer` | none | verified |
| `CALL-040` | `INV` | The sequence numbers of a video track forwarded to one receiver MUST stay continuous across a layer switch. | this spec section 3, the SFU; `CALL-032` | `ambre_sfu::sim::tests::the_sequence_numbers_stay_continuous_across_a_layer_switch` | none | verified |
| `CALL-041` | `INV` | The SFU MUST forward a receiver the video of the most recent speakers alone, up to the count of `ambre-spec-budgets.md section 1`. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_receiver_gets_the_video_of_the_most_recent_speakers_alone` | none | verified |
| `CALL-042` | `OBS` | A browser whose network opens TCP on port 443 alone MUST decode the audio and the video of the call through the TURN relay within the delay of `ambre-spec-budgets.md section 1`. | this spec section 3, the stack | `ambre_belt::call::tests::a_browser_on_tcp_443_alone_decodes_the_call_through_turn` -- browser tier, coturn over TCP behind port 443 at 150 ms of round trip | none | verified |
| `CALL-045` | `INV` | The SFU MUST send each receiver the media of a call over the fixed count of audio and video slots of `ambre-spec-budgets.md section 1`. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_receiver_holds_a_fixed_count_of_slots` -- a call of twelve | none | verified |
| `CALL-047` | `INV` | The SFU MUST forward a receiver the audio of the most recent speakers alone, up to the count of `ambre-spec-budgets.md section 1`. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_receiver_gets_the_audio_of_the_three_most_recent_speakers_alone` | none | verified |
| `CALL-048` | `INV` | The sequence numbers of a slot MUST stay continuous when the slot moves to another sender. | this spec section 3, the SFU; `CALL-040` | `ambre_sfu::sim::tests::the_sequence_numbers_of_a_slot_stay_continuous_across_a_change_of_sender` | none | verified |
| `CALL-049` | `INV` | The SFU MUST move a video slot to another sender only on a keyframe of that sender. | this spec section 3, the SFU; `CALL-039` | `ambre_sfu::sim::tests::a_slot_moves_to_another_sender_only_on_a_keyframe` -- 150 ms of round trip | none | verified |
| `CALL-056` | `INV` | The relay MUST give each join of a call a sender index that no earlier join of that call held. | this spec section 3, signalling; `CALL-005` | `ambre_server::ws::handler::call::tests::each_join_of_a_call_gets_a_sender_index_no_earlier_join_held` -- database tier: a join after a leave gets a fresh index, and a join past the last index a key identifier carries reads a notice, since one script takes the seat and the index (`SRV-248`) | none | verified |

### 1.2 Join a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-016` | `POST` | A join MUST hand the joiner the current sender key of each participant, wrapped to the joiner alone. | this spec section 3, the join steps | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` -- the key model: each participant opens every chunk of the others; `ambre_crypto::call::tests::a_wrap_opens_for_its_recipient_and_binding_alone` | none | verified |
| `CALL-017` | `POST` | A join MUST move each participant to the next generation of its sender key before the wrap. | this spec section 3, the join steps | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` -- a joiner opens nothing sealed before its join, under any key it holds or a ratchet of one | none | verified |
| `CALL-018` | `FRAME` | A join MUST NOT show a sender key to the relay or to the SFU. | `ZK-002`; this spec section 3 | type-level -- a join hands the relay a `SignedJoin`, which holds no key, and wraps that open under the X25519 key of their recipient alone. No wire type names a `SenderKey` (`ZK-002`); `ambre_crypto::call::tests::a_wrap_opens_for_its_recipient_and_binding_alone` | none | verified |
| `CALL-019` | `FAIL` | Given a joiner absent from the membership log of the channel, a client MUST wrap no key for it. | this spec section 3, the join steps; `ambre-spec-membership.md` | `ambre_sync::call::tests::a_joiner_outside_the_channel_gets_no_wrap`; `::each_member_opens_exactly_what_it_should` -- an outsider and a removed member are refused, and open nothing; `ambre_sync::call_client::tests::a_joiner_outside_the_channel_gets_no_key_through_the_client` -- through the client and a model of the call socket | none | verified |
| `CALL-020` | `FAIL` | Given a signed ephemeral key the client has seen already, the client MUST refuse it. | this spec section 3, the join steps | `ambre_sync::call::tests::a_join_seen_already_is_refused`; `::each_member_opens_exactly_what_it_should` -- a join delivered again is refused | none | verified |
| `CALL-057` | `POST` | The relay MUST hand the signed join of a participant, unchanged, to every other participant of the call and to each later joiner. | this spec section 3, signalling; `CALL-016` | `ambre_server::ws::handler::call::tests::a_signed_join_reaches_every_other_seat_unchanged` -- database tier: a seat in the call hears the join, a later seat reads it in its roster, and a second announce changes nothing; `ambre_protocol::tests::the_relay_reads_only_the_listed_fields_of_a_call_frame` -- the join crosses as opaque bytes | none | verified |
| `CALL-058` | `FRAME` | The relay MUST NOT forward a wrap to a participant session other than the one its sender index names. | this spec section 3, signalling; `CALL-030` | `ambre_server::ws::handler::call::tests::a_wrap_reaches_its_recipient_alone_on_the_call_socket` -- database tier: a second session of the same member hears no wrap | none | verified |
| `CALL-059` | `POST` | Two clients that join one call through the relay MUST each hold the sender key of the other. | this spec section 3, the join steps; `CALL-016` | `ambre_sync::call_client::tests::two_clients_over_the_relay_each_hold_the_other_sender_key` -- host tier: two clients, each with its own keys, over a model of the call socket whose rules the database tier holds on the relay (`CALL-056` to `CALL-058`), and a chunk one seals the other opens; `::a_late_client_holds_every_key_from_its_join_on` | none | verified |
| `CALL-061` | `FAIL` | Given a signed join whose sender index is not the one the relay gave its seat, a client MUST wrap no key for it. | this spec section 3, signalling; `CALL-056` | `ambre_sync::call_client::tests::a_join_signed_under_another_index_than_its_seat_gets_no_wrap` -- the same join under its own seat gets the wrap | none | verified |
| `CALL-062` | `POST` | A client admitted to a call MUST open the socket of the SFU that its admission names, with the admission as the first frame. | this spec section 3, signalling | `ambre_sync::call_client::tests::an_admission_opens_the_socket_of_the_sfu_with_the_admission_first`; `ambre_sync::call_link::tests::a_join_reads_the_log_then_opens_the_sockets_and_hands_the_key_to_the_port` -- host tier: the worker reads the log, opens the call socket with the token, then the socket of the SFU with the admission, and its key leaves on the port. The browser transport of both sockets runs in the belt of the Call worker and call client Story (`ambre-roadmap.md` E5) | none | verified |
| `CALL-021` | `FAIL` | Given a call at the participant cap of `ambre-spec-budgets.md section 1`, a join MUST be refused with a notice. | this spec section 3 | `ambre_server::ws::handler::call::tests::a_join_at_the_participant_cap_is_refused_with_a_notice` -- database tier: the last seat is admitted, the next join reads the notice, then the socket closes | none | verified |

### 1.3 Send and receive audio and video

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-022` | `POST` | A received chunk MUST reach the decoder only after it opens under the key its identifier names. | this spec section 3, the chunk steps | not built | unbuilt -- lands with the `Call worker and call client` Story (`ambre-roadmap.md` E5). The open itself is `CALL-055` | unimplemented |
| `CALL-055` | `POST` | The open of a chunk MUST hand out its clear bytes only once its tag verifies under the key its identifier names. | this spec section 3, the chunk format; `CALL-022` | `ambre_crypto::call::tests::a_changed_byte_drops_the_chunk`; `::a_chunk_that_names_another_sender_is_dropped`; `::a_sealed_audio_chunk_opens_whole_under_its_sender_key` | none | verified |
| `CALL-023` | `FRAME` | The seal of a chunk MUST NOT change the clear prefix the SFU reads. | this spec section 3, the chunk steps | not built | unbuilt -- lands with the `Video calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-024` | `FAIL` | Given a chunk that does not open, the receiver MUST drop it. | this spec section 3, the chunk steps | `ambre_crypto::call::tests::a_changed_byte_drops_the_chunk`; `::a_chunk_that_does_not_open_moves_no_window` | none | verified |
| `CALL-025` | `FAIL` | Given a chunk whose key identifier names a sender other than the owner of its track, the receiver MUST drop it. | this spec section 3, the key identifier | `ambre_crypto::call::tests::a_chunk_that_names_another_sender_is_dropped` -- a chunk Mallory seals under her key with the identifier of Bob, and one of hers she relabels | none | verified |
| `CALL-026` | `FAIL` | Given a chunk whose counter falls outside the replay window of its key, the receiver MUST drop it. | this spec section 3, the chunk steps | `ambre_crypto::call::tests::a_chunk_outside_the_replay_window_is_dropped` | none | verified |

### 1.4 Leave a call

A participant leaves, or the channel removes a member who is in the call.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-027` | `POST` | A leave MUST move each remaining participant to a fresh sender key within the delay of `ambre-spec-budgets.md section 1`. | this spec section 3, the leave steps | `ambre_sync::call::tests::a_leave_switches_each_remaining_sender_within_the_delay` -- the switch at 5 s, the target of the budget row; `::each_member_opens_exactly_what_it_should` -- a member who left opens nothing after the switch; `ambre_sync::call_client::tests::a_leave_switches_the_remaining_clients_and_the_leaver_opens_nothing_after` -- through the client and a model of the call socket | none | verified |
| `CALL-028` | `POST` | A removal entry in the membership log of the channel MUST move each remaining participant to a fresh sender key. | this spec section 3, the leave steps; `SYNC-132` | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` -- a revoke entry of the owner, read from the walked log, switches every remaining sender | none | verified |
| `CALL-029` | `POST` | Leaving a call MUST erase every sender key of that call from the call worker. | this spec section 3, the leave steps | not built | unbuilt -- lands with the `Call worker and call client` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-030` | `FRAME` | A leave MUST NOT send a fresh sender key to the participant who left. | this spec section 3, the leave steps | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` -- no wrap reaches a member out of the call, and a member who left keeps every key and opens nothing after the switch | none | verified |
| `CALL-060` | `POST` | A client in a call MUST read the membership log of the channel again when the members of the channel change, and switch its key away from a member the log no longer holds. | this spec section 3, the leave steps; `CALL-028` | `ambre_sync::call_link::tests::a_change_of_the_members_of_the_channel_reads_the_log_again` -- a change of the members, or a rotation of the key, of the channel reads the log; `ambre_sync::call_client::tests::a_member_removed_from_the_channel_holds_no_key_of_the_next_generation` -- the relay still carries the removed member's socket, and it opens nothing sealed after the switch, under any key it held or a ratchet of one | none | verified |
| `CALL-063` | `POST` | A seat that leaves its call with no close of its socket MUST reach the other participants as a leave. | this spec section 3, signalling; `SRV-251`; `CALL-027` | `ambre_server::ws::handler::call::tests::a_seat_that_stops_its_heartbeat_leaves_the_roster_and_frees_its_place` -- database tier: the heartbeat that finds the expired seat announces its leave, once | none | verified |

### 1.5 Forward a packet

A participant sends a packet to the SFU, and the SFU forwards it within the room.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-033` | `FRAME` | The SFU MUST NOT send a packet to its own sender, or to a participant of another room. | this spec section 3, the SFU; `CALL-009` | `ambre_sfu::forward::tests::no_packet_goes_back_to_its_sender_or_into_another_room` | none | verified |
| `CALL-034` | `POST` | A participant who joins a room MUST receive each track the room already carries. | this spec section 3, the SFU | `ambre_sfu::forward::tests::a_late_joiner_receives_the_tracks_already_in_the_room` | none | verified |
| `CALL-035` | `POST` | A keyframe request from a receiver MUST reach the sender of its track. | this spec section 3, the SFU | `ambre_sfu::forward::tests::a_keyframe_request_reaches_the_sender_of_the_track` | none | verified |
| `CALL-046` | `FRAME` | A change of speaker MUST NOT renegotiate the session of any participant. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_change_of_speaker_renegotiates_nothing` -- a call of twelve | none | verified |

### 1.6 Lower a layer

The downlink estimate of a receiver falls below what the SFU forwards it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-038` | `POST` | Given a receiver whose downlink estimate falls below what it receives, the SFU MUST forward it a lower layer within the delay of `ambre-spec-budgets.md section 3`. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_capped_receiver_drops_a_layer_within_the_budget` -- eight seeded runs at 150 ms of round trip and 2 % loss; `ambre_sfu::congestion::tests::the_target_falls_under_a_new_cap_within_a_second` -- the controller alone | none | verified |
| `CALL-043` | `FRAME` | A layer drop for one receiver MUST NOT change the layer another receiver gets. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_layer_drop_for_one_receiver_leaves_another_receiver_on_its_layer` -- eight seeded runs | none | verified |
| `CALL-044` | `FRAME` | A random loss of 10 % or less on the downlink of a receiver MUST NOT lower the layer it gets. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_receiver_of_twenty_keeps_the_top_layer_up_to_ten_percent_loss` -- a call of twenty at 150 ms of round trip, 2 % and 10 % of loss; `ambre_sfu::congestion::tests::the_target_holds_under_random_loss_up_to_ten_percent` -- a modelled link at 2 %, 5 % and 10 % | none | verified |

### 1.7 Raise a layer again

The downlink of a receiver carries more than its target, after a drop with no lasting congestion.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-050` | `POST` | Given a receiver that dropped a layer on a delay step with no lasting congestion, the SFU MUST forward it the top layer again within the delay of `ambre-spec-budgets.md section 3`. | this spec section 3, the SFU; `CALL-038` | `ambre_sfu::sim::tests::a_receiver_climbs_back_to_the_top_layer_after_a_delay_step` -- eight seeded runs at 150 ms of round trip and 2 % loss, a step of 40 ms | none | verified |
| `CALL-051` | `FRAME` | A probe MUST NOT raise the target of a receiver to a layer step its downlink cannot carry. | this spec section 3, the SFU; `CALL-050` | `ambre_sfu::sim::tests::a_receiver_capped_below_the_top_layer_never_climbs_to_it` -- eight seeded runs at 150 ms of round trip and 2 % loss, under a cap of 1.2 Mbit/s | none | verified |
| `CALL-052` | `POST` | Given a probe that raises overuse, the SFU MUST stop its padding at the feedback that reads the overuse. | this spec section 3, the SFU; `CALL-050` | `ambre_sfu::congestion::tests::a_probe_that_raises_overuse_stops_at_once` -- the controller alone, a modelled link at 20 ms of round trip and 2 % loss | none | verified |

### 1.8 Simulate a call

A test runs the SFU and synthetic participants in simulated time, from a seed.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-053` | `INV` | A simulated run of the SFU MUST follow one path per seed, in any process and under any load. | this spec section 3, the SFU | `ambre_sfu::sim::paths::each_seed_follows_one_path_in_separate_processes` -- every test of the simulation in three concurrent processes | none | verified |

* **Budget.** the call rows of `ambre-spec-budgets.md section 1` and section 3. They cover the
  chunk transform, the worker load, the key switch, the cap, the latency and the SFU load.
* **Boundary.** every chunk leaves sealed under a sender key that neither the relay nor the SFU
  holds (`CALL-001`, `CALL-018`). What each of them learns of a call is bounded by `ZK-021` and
  `ZK-022`.
* **Verification.** 51 `verified`, 12 `unimplemented`. The relay admits under the cap, and the SFU
  mints the TURN credentials (`CALL-021`, `CALL-014`). The SFU links no C cryptography, forwards RTP
  and reaches a browser through TURN (`CALL-013`, `CALL-031` to `CALL-044`, `CALL-050` to
  `CALL-052`). Each receiver holds fixed slots, and a seed gives one path (`CALL-045` to
  `CALL-053`). The seal and the keys hold on the host (`CALL-016` to `CALL-030`, `CALL-054`,
  `CALL-055`).

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The call holds | Each user sees | Test |
|---|---|---|---|---|
| joins | is in the call | two participants, one sender key each | each the video of the other (`CALL-016`) | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` |
| joins late | talks | Bob's key, one generation up | Alice hears Bob from her join on, and nothing before (`CALL-017`) | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` |
| leaves | stays | Bob's fresh key | Bob goes on alone, and Alice opens nothing after the switch (`CALL-027`) | `ambre_sync::call::tests::a_leave_switches_each_remaining_sender_within_the_delay` |
| is removed from the channel | stays | Bob's fresh key | Alice's client opens nothing after the switch (`CALL-028`) | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` |
| sends chunks under the track of Bob, as Mallory | listens | two sender keys | nothing of the forged chunks (`CALL-025`) | `ambre_crypto::call::tests::a_chunk_that_names_another_sender_is_dropped` |

---

## 3. MODEL

**CRDT containers.** none. A call message names the call in its channel, and
`5-Murmure/ambre-spec-murmure-schema.md section 3` declares it.

**The stack, layer by layer.**

| Layer | Choice | Runs on | Rust |
|---|---|---|---|
| capture | the browser capture of the camera, the microphone and the screen | the UI thread | no |
| echo, noise and gain | the three capture constraints; remote audio plays as a stream in a detached audio element | the browser | no |
| encode and decode | the browser codecs: Opus with in-band error correction at a constant rate, and VP8 at three sizes | the browser | no |
| jitter, loss, congestion | the browser peer connection | the browser | no |
| chunk seal and open | the chunk format below, in `ambre-crypto` | the call worker | yes |
| sender keys: identity, checks, wraps | the steps below | the Sync worker | yes |
| signalling | a relay route of its own (`SRV-049`), then a socket to the SFU | the Sync worker | yes |
| forwarding | `ambre-sfu`, a process apart from the relay, over str0m | the server | yes |
| fallback when UDP is blocked | coturn, a TURN relay (Traversal Using Relays around NAT) over TLS on port 443 | the server, isolated | no |
| video on the canvas | a detached video element, copied into a GPU texture each paint | the UI thread | yes |

**Why the browser keeps half of it.** The capture, the hardware codecs and the echo canceller exist
only in the browser. The peer connection is the one real-time path whose audio every engine
cancels, and iOS offers no other with end-to-end encryption.

**A chunk, sealed.** The browser hands each encoded chunk to a transform that runs in the call
worker.

1. Read the clear prefix the SFU needs: 10 bytes on a VP8 keyframe, 1 byte otherwise, none for
   audio.
2. Pad an audio chunk to the fixed size of its call (`CALL-007`).
3. Write the header of RFC 9605: the key identifier, then the counter.
4. Derive the nonce from the counter by the key schedule of RFC 9605.
5. Seal the rest under ChaCha20-Poly1305 with a 16-byte tag, as private suite `0xF001`. The header
   and the clear prefix are the additional data (`CALL-006`).
6. On receive, check the replay window of the key, then open, then hand the chunk to the decoder
   (`CALL-022`). A chunk that fails any step is dropped (`CALL-024`, `CALL-026`).

**The key identifier.** Three fields, one key each:

* the track: audio 0, camera 1, screen 2;
* the sender index, given to one participant session for one call. A reload is a new join, and an
  index is never resumed;
* the generation, which strictly increases.

**The chunk format, byte by byte.** One layout serves every track, and a field a track does not use
is empty. The Call crypto review Story (`ambre-roadmap.md` E5) reads it, with
`crates/ambre-crypto/src/call.rs`.

| Bytes | Field | Rule |
|---|---|---|
| any | the clear prefix | none for audio. A video track seals nothing until the Video calls Story (`ambre-roadmap.md` E5) sets its rule |
| 1 | the config byte of RFC 9605 | `0xFF`: a key identifier and a counter of 8 bytes each. A parser refuses any other value |
| 8 | the key identifier, big-endian | the generation in bits 16 to 63, the sender index in bits 2 to 15, the track in bits 0 and 1 |
| 8 | the counter, big-endian | issued from 1 up by the one sealer of its key identifier (`CALL-005`) |
| 162 for audio | the ciphertext | a big-endian `u16` length, the clear chunk, then zeros up to 162 bytes (`CALL-007`) |
| 16 | the Poly1305 tag | over the clear prefix and the header, the additional data |

* A sealed audio chunk is 195 bytes in every call. A clear audio chunk holds 160 bytes at most:
  Opus at a constant 64 kbit/s writes that many per 20 ms.
* RFC 9605 writes the shortest form of each field. The fixed lengths here give every header one size.
* The key schedule is the one of RFC 9605 section 4.4.2, over HKDF-SHA256. From the sender key `K`,
  `HKDF-Extract("", K)` gives a secret. Under the label `SFrame 1.0 Secret key `, the key
  identifier and `0xF001`, it expands to the 32-byte key; under `SFrame 1.0 Secret salt `, to a
  12-byte salt.
* The nonce is the salt XOR the counter, left-padded to 12 bytes. So each track of a sender seals
  under a key and a nonce of its own.
* A generation is an epoch in bits 32 to 47, then a step in bits 0 to 31. A ratchet expands
  `HKDF-Extract("", K)` under `ambre-call-ratchet-v1` to the key one step up. A fresh key opens the
  next epoch at step zero, so a ratchet and a fresh key never share a generation.
* A receiver ratchets a held key up to 64 steps inside its epoch, never into another. It keeps a
  window of 128 counters per key identifier, marked once a tag verifies. It keeps the 8 newest key
  identifiers of each sender, and refuses a generation below one it dropped.

**The join and the wrap, byte by byte.**

| Item | Bytes |
|---|---|
| the signed transcript of a join | `ambre-call-join-v1`, the call, the channel, the member, the Ed25519 identity, the time in ms and the sender index as big-endian `u64`, the X25519 key of the join |
| the key that wraps | HKDF-SHA256 over the X25519 secret of the two join keys, under `ambre-call-wrap-v1`, the call, the two sender indexes and the two join keys |
| a wrap | a 24-byte nonce, then the generation as a big-endian `u64` and the 32-byte key, under XChaCha20-Poly1305 with `ambre-call-wrap-v1` as additional data: 80 bytes |

**A join, in steps.** The Sync worker holds the identity; the call worker receives symmetric keys
alone (`CALL-004`).

1. The joiner draws an X25519 key for this call, and signs the call, the channel, the time and the
   key with its Ed25519 identity.
2. Each participant checks the joiner against the membership log of the channel (`CALL-019`) and
   refuses a signed key it has seen (`CALL-020`).
3. Each participant moves its sender key one generation up by HKDF (`CALL-017`).
4. Each participant wraps that key to the joiner alone (`CALL-016`). The wrap binds the call, the
   sender, the recipient and the generation.

**A leave, in steps.** A leave or a signed removal starts the same switch (`CALL-028`).

1. Each remaining participant draws a fresh sender key.
2. It wraps the key to each other remaining participant, never to the one who left (`CALL-030`).
3. It seals with the fresh key once the switch delay of the budget row has passed (`CALL-027`).

The fresh key takes the next epoch above the current key and any key still waiting. A second leave
inside the delay draws another key and keeps the time of the first, so each leave switches in time.
A join while a key waits ratchets both keys, and wraps both to the joiner.

**Who admits whom.** The membership log of the channel decides whom a client wraps a key for. The
relay keeps the roster of a call and forwards the wraps; it orders nothing and stores nothing.

**Signalling, in steps.** Two sockets carry it, and neither is the socket of the document plane
(`SRV-049`).

1. The client opens the call socket of the relay, `/api/calls/{call}/socket`. Its first frame
   holds the access token and the channel of the call.
2. The relay checks the membership of the channel. Under the cap, it adds the session to the
   roster, or it sends a notice and closes (`CALL-021`).
3. The relay answers an admission, the address of the SFU, a sender index and the signed join
   of each participant already in the call. No earlier join of the call held that index
   (`CALL-056`).
4. The admission names the room and the participant by two pseudonyms of 128 random bits. The
   relay draws the room once per call, and the participant once per join (`ZK-023`).
5. The client opens the socket of the SFU and sends the admission as the first frame. The SFU
   verifies it with the relay's public key alone.
6. The SFU answers the TURN servers, with a credential under the participant's pseudonym, then
   reads the offer.
7. The client signs its join under that index and announces it. The relay hands that signed join,
   unchanged, to every other participant (`CALL-057`). It forwards each opaque wrap to the
   session of its sender index alone (`CALL-058`), and announces each leave. A closed call
   socket is the leave, and the last leave ends the call.

**The state of a call** lives in the ephemeral store, never in the durable register
(`SRV-250`). It holds the channel, the room, the next sender index, and the key and the signed
join of each seat. One script takes a seat and its index at once. The relay passes it a room from
the generator of the OS, which the first join keeps.

Every key of a call,
and each field of a seat, lives under the time to live of `ambre-spec-budgets.md section 1`. The
heartbeat of each open socket refreshes the keys and its own seat (`SRV-248`). A seat no
heartbeat holds leaves the roster and the cap (`SRV-251`). The next heartbeat of another seat
finds it gone and announces its leave, so the others switch their keys (`CALL-063`). The last
leave deletes every key of the call.

A socket whose call state the store lost gets a notice, then closes (`SRV-249`). A rejoin then
starts a fresh call under a new room, from the first sender index. So no two senders share an
index under one call state. Whether a call ended is no fact the relay keeps: the call message
of the channel holds it.

**The client of a call** runs in the Sync worker. It reads the membership log of the channel,
then opens the call socket. A change of the members of the channel reads the log again. The
client then switches its key away from a member the log no longer holds (`CALL-060`).

It wraps no
key to a join signed under another index than its seat (`CALL-061`). It opens the socket of the
SFU that the admission names, with the admission first (`CALL-062`). It hands each key it
draws, ratchets or unwraps to the call worker as a handoff, on a port of their own.

**The admission token.** Its text is hex: the encoded admission, then an Ed25519 signature.

| Part | Holds |
|---|---|
| claims | the room pseudonym, the participant pseudonym, the expiry |
| expiry | 60 s after the relay issues it, in Unix seconds |
| audience | the SFU: the signature covers the domain `ambre-call-admission-v1` first |
| key | `CALL_ADMISSION_SEED` on the relay, its public key in `SFU_ADMISSION_KEY` on the SFU |

The admission key serves one purpose. It is never the access-token key, so either one rotates
alone, and a leak of one forges nothing under the other. The SFU then holds no key that verifies
a session.

**The TURN credential.** The SFU mints it under the REST API of coturn. The username is the
expiry, then the pseudonym. The password is the HMAC-SHA1 of the username under a secret. The
SFU and coturn alone share that secret (`CALL-014`). No crate depends on the SFU, so the relay
cannot name it.

**The SFU seats.** One admission seats one connection, and the SFU holds the cap too.

**The SFU.** It forwards packets and never decodes them.

* str0m drives it, with its DTLS on RustCrypto. A patched copy of its RustCrypto backend builds
  the DTLS certificate from RustCrypto too, so no C library generates one (`CALL-013`).
* A congestion control of Ambre's own runs on the downlink of each receiver (`CALL-044`). Its
  delay half is the GCC design of libwebrtc, ported from the estimator of str0m.
* A patched str0m hands it the transport-wide feedback of the receiver, and paces and pads at its
  target (`vendor/str0m/PATCH.md`). str0m starts no probe cluster of its own.
* The target is the lower of two bounds. A growing queue delay lowers the delay bound. A loss
  above 10 % lowers the loss bound, as the GCC draft states, and 10 % or less holds it.
* The loss bound falls only when the loss of the last second is above 10 % by two standard
  errors. A random loss of exactly 10 % reads above 10 % in half of its windows.
* The SFU pads up to the rate of the next layer step. The target grows by 8 % per second, and by
  100 % per second before its first decrease, never above 1.5 times the acknowledged rate.
* Under overuse, the SFU pads nothing. The padding would hold the queue of the bottleneck full,
  and the keyframe of the lower layer would wait behind it.
* A decrease sets the target under the acknowledged rate, and the target grows back by about one
  packet per round trip. So a delay step with no lasting congestion would hold a lower layer for
  seconds. The SFU probes instead (`CALL-050`).
* A probe starts while the delay bound reads no overuse, no loss bounds the target, and a layer
  step lies above the target. For 300 ms, the SFU pads at 1.6 times that step. The padding fills
  the gaps between the media alone, so the link sees less than that pace.
* The target takes the step when the acknowledged rate reaches 95 % of it, with no overuse. The
  layer choice then climbs.
* A probe that raises overuse stops its padding at once (`CALL-052`). Until that overuse ends,
  or for one round trip and 600 ms at most, no decrease goes under the target the probe found.
  The queue is the probe's, and it drains.
* A failed probe waits 1 s before the next one, and each further failure doubles the wait up to
  30 s. So a capped receiver sees a probe rarely, and never takes a step it cannot carry
  (`CALL-051`).
* Each receiver holds three audio slots and nine video slots (`CALL-045`). The SFU offers them
  once, when the room first carries a track of another participant.
* A change of speaker moves a slot to another sender, and renegotiates nothing (`CALL-046`). The
  pacer of str0m reads every send stream at each packet, so the slots also bound its cost.
* Each receiver gets the video of the nine most recent speakers (`CALL-041`), and the audio of the
  three most recent speakers (`CALL-047`). The audio level of each packet, which the seal leaves
  in clear, names who speaks.
* The layer choice runs per receiver, every 100 ms, and at once when a target falls or overuse
  starts or ends. Each of the nine gets its lowest layer while
  the estimate holds it. Then the most recent speaker moves up to the highest layer the rest
  holds. A move up needs a margin, so a receiver near a boundary stays on its layer.
* A switch to another layer waits for a keyframe of that layer (`CALL-039`). The SFU shifts the
  sequence numbers, the timestamps and the VP8 picture numbers of the new layer to continue the
  old one (`CALL-040`). The VP8 payload descriptor stays in clear, so the SFU reads it.
* A move of a video slot to another sender waits for a keyframe of that sender (`CALL-049`). An
  audio slot moves at the next packet. The SFU shifts the fields the same way (`CALL-048`).
* The receiver reads the sender of each chunk from its key identifier. So no signal names the
  sender of a slot.
* A switch to a lower layer of the same track stops the current layer at once. Its packets would
  queue in the pacer of the congested downlink, ahead of the lower layer, for seconds.
* The stopped layer resumes on a keyframe alone, when the switch no longer goes down. It missed
  pictures, and a resume between two keyframes would also break the sequence numbers
  (`CALL-040`).
* The receiver sees a freeze until the keyframe of the lower layer arrives. The SFU asks for that
  keyframe again every 300 ms, the shortest interval libwebrtc keeps between two keyframes.
* It writes nothing to a receiver before its connection is up. str0m would hold the packets and
  send them in one burst at connect, and the jitter buffer of the receiver would hold seconds.
* Its signalling is a WebSocket on a port of its own (`CALL-036`). axum and tokio serve it, the
  stack of the relay, so the SFU adds no crate to the workspace.
* It forwards in RTP mode: it copies the payload and the header fields of each packet
  (`CALL-032`). The sequence numbers of each receiver start at a rollover count of zero.
* One task owns the SFU core. Within one poll, the connections run on several threads, since
  they share nothing, and the forwarding between them runs on one.

**What the design does not protect.** These limits are accepted, and stated here so that no reader
assumes them away.

| Limit | Why it stays |
|---|---|
| Mallory, working with the SFU, can forge the chunks of another member | a signature per chunk costs more than the budget of the call worker allows |
| a member compromised during a call stays compromised until the call ends | that device already holds the camera and the plaintext |
| the SFU sees when each member speaks, through the packet rate of silence | the codec stops sending in silence, and padding does not hide an absent packet |
| a member who leaves can open chunks until the switch completes | the switch waits for the delay of the budget row, so that late wraps arrive |

**Code.** The crate `ambre-sfu` holds the SFU:

| File | Holds |
|---|---|
| `src/lib.rs` | the connection settings of a participant |
| `src/forward.rs` | the forwarding core, without I/O |
| `src/layer.rs` | the layer choice |
| `src/congestion.rs` | the congestion control of each receiver: the loss bound, the acknowledged rate and the probe |
| `src/congestion/delay.rs` | its delay bound, ported from str0m under the MIT license, with its notice |
| `src/rewrite.rs` | the header fields of a slot across a switch of layer or of sender |
| `src/serve.rs` | the UDP socket and the signalling WebSocket, which admits a participant |
| `src/admit.rs` | the verification of an admission |
| `src/turn.rs` | the TURN servers and the credential of a participant |
| `src/secrets/` | the TURN secret and the read of an admission, the one module that reads a secret |
| `src/sim/` | tests alone: the impairment relay, the synthetic participants, the load run, the guard of one path per seed |

**A simulated run is seeded.** The seed sets the losses of each link and the random numbers str0m
draws on its thread. The patched str0m keeps its streams in a sorted map, and the simulated SFU
runs its connections on one thread. So a seed gives one path, on any machine and under any load
(`CALL-053`). The guard compares the runs of three processes in the native tier, and
`just sim-paths` compares 20.

The simulated clock starts at a fixed offset from the origin of the clock of str0m. The test
exports of the patched str0m pin the wall clock of that origin. str0m reads the round trip from
the NTP times of a sender report, at a resolution of 1/65536 s. Started from the clock of the
process, that round trip rounds another way in each process. It times the retries of a NACK. On
2026-11-08, one retry 1 ms apart gave seed 1 of `CALL-051` 0 or 4 layer switches.

The keys of a simulated DTLS handshake come from the generator of the OS. The length of an ECDSA
signature then varies by a byte or two between processes. The guard of `CALL-053` leaves the size
of a DTLS record out of its trace, and keeps its time and its link.

The simulated clock steps by 1 ms. The SFU wakes str0m at the timeout it asks, and tokio rounds
that timeout to 1 ms. The pacer of str0m sends one padding packet per wake. A coarser step would
starve a probe of padding that the SFU sends.

**Dependencies.** Each one the SFU adds, and why:

| Crate | Why |
|---|---|
| `str0m`, vendored | the seam of the feedback and the estimate, which str0m 0.25 lacks |
| `fastrand`, tests alone | the generator str0m draws from; a test seeds it, and adds no crate to the lockfile |
| `ambre-protocol` | the admission, which the relay signs and the SFU reads |
| `ed25519-dalek`, `hex` | the verification of the relay's signature over an admission, in pure Rust |
| `hmac`, `sha1`, `base64` | the TURN credential, as coturn checks it; RustCrypto, already in the lockfile |

**The chunk seal and the sender keys** run on the host and in the Sync worker. No call worker
reads a handoff yet:

| File | Holds |
|---|---|
| `crates/ambre-core/src/call.rs` | the track, the sender index, the generation, the counter, the key identifier and the two pseudonyms of the SFU |
| `crates/ambre-crypto/src/call.rs` | the chunk format, the sender key, the receiver, the signed join and the wrap |
| `crates/ambre-sync/src/call.rs` | the keys of one participant: the admission, the wraps and the switch |
| `crates/ambre-sync/src/call_client.rs` | the client of a call: each frame of the two sockets, and the steps it answers |
| `crates/ambre-sync/src/call_link.rs` | the transport of a call in the Sync worker: the two sockets, the log, the switch timer and the port |

**The signalling** of a call: `crates/ambre-protocol/src/signaling.rs` holds the frames of the
call socket, the admission and the messages of the socket of the SFU.
`crates/ambre-server/src/ws/handler/call.rs` serves the call socket, and
`crates/ambre-server/src/redis_ops.rs` keeps the state of a call.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| join, from a call message | the joiner's keys and the peer connection come up; the gesture is `ambre-spec-murmure-calls.md`'s |
| a participant joins or leaves | the remaining participants switch their sender keys |
| a removal entry in the membership log | the same switch, for every call of that channel |
| a chunk arrives | it opens and plays, or it is dropped |
| the call ends | the call worker erases its keys |

---

## 5. REJECTED ALTERNATIVES

Versions and bug numbers read on 2026-10-18, from the sources in the session research.

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| WebRTC carries the media | Media over QUIC, through WebTransport and WebCodecs, behind `moq-relay` | WebKit bug 319818 stalls long sessions, and the reference client turns WebTransport off on WebKit. WebKit cancels no echo from audio played outside a stream. WebCodecs conceals no lost packet. Revisit when an iOS release fixes the bug and holds a 30-minute call |
| The call worker holds the transform | the Sync worker | a sealed file blocks the Sync worker for up to 2 s (`ambre-spec-budgets.md section 1`). The transform would also parse untrusted chunks beside the Master Seed |
| One sender key per participant | one key for the call, drawn by an elected rotator | it needs an election, an answer to two leaves at once and an epoch order on the relay. A key per sender needs none of the three |
| One sender key per participant | MLS through OpenMLS | it adds about 0.63 MB gzipped to the worker. Its WebAssembly build is untested upstream, and no full audit exists |
| One sender key per participant | a key derived from the channel key | every member of the channel could open a call they never joined (`MURMURE-157`) |
| The chunk format is our own code | the `sframe` crate | its cipher suites are a closed enum, so it cannot hold the private suite |
| ChaCha20-Poly1305 | AES-128-GCM, the suite of RFC 9605 | interoperability buys nothing: no browser ships the standard transform. Software AES runs 5 to 8 times slower in WebAssembly, on a desktop |
| ChaCha20-Poly1305 | the AES-GCM of the browser crypto API | one asynchronous call per chunk, and the key leaves Rust |
| str0m drives the SFU | mediasoup | its worker is C++ over OpenSSL. It stays the fallback if the SFU proof fails |
| str0m drives the SFU | Signal-Calling-Service as it ships | it has no DTLS |
| A congestion control of Ambre's own controls each downlink | the estimate of str0m 0.25 | under 2 % of random loss it fell: on 2026-11-08 a simulated call of twenty held its receiver on the middle layer, and the load run of fifty held every receiver on the lowest. The patched str0m paces at the SFU's target, so no second estimator runs |
| A congestion control of Ambre's own controls each downlink | the loss-based controller of str0m 0.25 | the estimate of str0m fell under 2 % of random loss, as the row above states, which `CALL-044` forbids |
| A congestion control of Ambre's own controls each downlink | a congestion control under a license Ambre does not hold | Ambre keeps the option of a second license, so no code under a copyleft license of another owner enters the tree |
| Fixed slots per receiver | a media section per track of the room | each join renegotiated every receiver, and the pacer of str0m read 98 send streams at each packet of a call of fifty |
| The audio of the three most recent speakers | every audio track of the room | a slot count bounds the pacer and the downlink; a listener follows no fourth voice at once |
| A seeded simulation | a run isolated from the load of the machine | the spread came from the random numbers of str0m, not from the load: on 2026-11-08, seven runs alone on the machine dropped a layer from 0.87 s to 3.42 s |
| The simulated clock starts from the pinned origin of str0m | a simulated clock that starts from the clock of the process | the round trip str0m reads from a sender report rounds another way in each process, as section 3 states |
| The keys of a simulated DTLS handshake come from the generator of the OS | a DTLS handshake seeded in full | dimpl draws its key share from the generator of the OS inside its own crypto provider, which the workspace does not patch. The size of a handshake record moved no path: on 2026-11-08, four processes traced the same datagram times over every seeded run |
| A probe pads up to the next layer step | a faster growth of the target near the known capacity | the target would grow on a capped downlink too, and the receiver would climb into overuse, then drop again |
| A probe pads up to the next layer step | the probe clusters of str0m | str0m starts none while the SFU sets the estimate, and its result feeds the estimate of str0m, which `CALL-044` rejects |
| A drop to a lower layer stops the current layer | the current layer flows until the lower keyframe | its packets queue in the pacer of the congested downlink. Over 40 seeds on 2026-11-08, the worst drop took 2.52 s, against 0.85 s |
| The video of the nine most recent speakers | every video of the room to every receiver | 49 videos at the lowest layer exceed the downlink of a phone, and the SFU cost grows with the count |
| str0m drives the SFU | the `rtc` crate of webrtc-rs | ring or aws-lc-rs seal its DTLS records, not only its certificate |
| str0m drives the SFU | LiveKit | a Go server with a frame format of its own |
| Every call goes through the SFU | a direct connection between two clients | two topologies to test, and each peer would see the address of the other |
| The texture copy from a video element | a copy of each picture through WebAssembly memory | one readback per picture. Page elements over the canvas are the fallback, and no overlay could cover them |
| The fields of the chunk header have a fixed length | the shortest form of each field, as RFC 9605 writes it | the header would grow at the counter 256, and the size of an audio chunk would vary (`CALL-007`) |
| A second leave inside the delay keeps the time of the first | a second leave starts the delay again | the member who left first would open chunks past the delay of its own leave (`CALL-027`) |
| coturn serves TURN | turn-rs | aws-lc-rs seals its TLS records, and one person maintains it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No call worker seals a chunk.** The Sync worker drives the keys over the call socket, and
  no command of the bus joins a call yet. So the SFU still sees the media of the test pages in
  clear (`ZK-021`).
* **The browser syncs no audio slot to a video slot.** Each slot is a stream of its own, and a
  slot changes sender. The audio and the video of one sender drift by the difference of their
  jitter buffers.
* **Above 10 % of random loss, the loss bound lowers the target.** No rule binds the SFU there.
* **The receive sizes rest on no measurement.** One 720p and eight 180p videos on a phone, and the
  software VP8 decoder of an iPhone, are untested. H.264 for every sender is the fallback.
* **iOS audio may crackle with the echo canceller on.** WebKit bug 326286 reports it on iOS 26.2 to
  27, and Ambre has no fix of its own.
* **The SFU builds on two patched str0m crates.** The upstream backend builds its certificate
  through rcgen, which links aws-lc-sys. Upstream str0m offers no seam for an external congestion
  control. The workspace patches both until str0m ships the changes.
* **The time of a signed join is not checked.** A client refuses a join it has seen. A join of an
  earlier session of a member, which this client never saw, still gets a wrap while the log holds
  that member.
* **No rule caps the calls of one channel.** Whether a channel holds one live call or several is
  not ruled.
* **A network that allows only TCP on port 443** reaches the SFU through TURN alone. coturn serves
  it over TCP. TURN over TLS waits for a certificate the deployment holds.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a member starts or joins a call from a channel | `5-Murmure/features/ambre-spec-murmure-calls.md` |
| Who is a member of the channel, and how a removal is signed | `ambre-spec-membership.md` |
| What the relay and the SFU may learn of a call | `ambre-spec-zero-knowledge.md` |
| The threads and the instances of the client | `ambre-spec-architecture.md` |
| Every figure a call is held to | `ambre-spec-budgets.md` |
| Where the signalling travels | `ambre-spec-broadcast.md` |
