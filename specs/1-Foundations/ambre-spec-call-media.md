# AMBRE -- FEATURE: CALL MEDIA

**Type:** Horizontal (framework)

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
| `CALL-001` | `INV` | A chunk of a call MUST leave the client sealed under a sender key of that call. | README pillar "E2EE"; this spec section 3 | `ambre_call::tests::a_chunk_leaves_the_worker_sealed_under_the_sender_key_or_not_at_all` -- no key, no chunk; `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier, under Chrome and under Firefox: each client decodes the others, and a member out of the keys decodes nothing the SFU forwards; `ambre_call::tests::a_camera_chunk_leaves_sealed_and_opens_on_a_receiver_of_the_camera_alone` -- the camera too; `ambre_belt::murmure::tests::three_members_decode_each_others_video_at_the_size_the_sfu_chooses` -- browser tier: the camera of each sender too | none | verified |
| `CALL-054` | `INV` | The seal of a chunk MUST encrypt it under the key that the sender key its identifier names derives. | this spec section 3, the chunk format; `CALL-001` | `ambre_crypto::call::tests::the_sealed_bytes_of_a_known_key_follow_rfc_9605_and_stay_pinned` -- a second path written from RFC 9605 alone; `::a_sealed_audio_chunk_opens_whole_under_its_sender_key` | none | verified |
| `CALL-160` | `CONC` | Two starts of a call in one channel at once MUST seat both sessions in one call, and each admission answer MUST name that call. | this spec section 3, signalling; `MURMURE-594` | `ambre_server::ws::handler::call::tests::two_starts_at_once_make_one_call_that_holds_both_starters` -- database tier: one room, a roster of both starters, and a new call once the last left; `ambre_sync::call_link::tests::a_start_seated_in_the_live_call_of_its_channel_joins_it` -- the client of a start seated in another call joins that call | none | verified |
| `CALL-002` | `INV` | A sender key MUST NOT derive from a channel key or a document key. | this spec section 3; `MURMURE-157` | type-level -- `ambre_crypto::call::SenderKey` has four doors: `generate`, which draws from the generator of the OS, `ratcheted`, `unwrap_sender_key`, and `KeyHandoff::from_port`, which reads back on the port of the call worker the bytes `KeyHandoff::to_port` wrote from a held key. None takes a channel key or a document key; `ambre_crypto::call::tests::a_malformed_handoff_builds_no_key`; `ambre_crypto::call::tests::a_sender_key_is_drawn_fresh_and_a_ratchet_hides_the_key_before_it` | none | verified |
| `CALL-003` | `INV` | The seal and the open of every chunk MUST run in the call worker. | this spec section 3; `ARCH-004` | cargo-deny -- `deny.toml` grants `ambre-crypto` to `ambre-sync`, `ambre-call` and `ambre-logic` alone, so the Main instance links no seal; `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier: the transform of each sender and receiver is a transform of the call worker, and a client decodes the others through it alone | none | verified |
| `CALL-004` | `INV` | The call worker MUST hold no key beyond the sender keys of its current call. | this spec section 3; `ZK-011` | type-level -- `ambre_call::CallWorker` holds a `ChunkSealer` and a `ReceiveKeys` alone, which `CallWorker::holds_no_key` destructures with no `..`, so a new field does not compile until a writer weighs it here, and its one door in is a `KeyHandoff`; `ambre_call::tests::the_end_of_a_call_erases_every_key_of_it`; ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses a crate that depends on the call worker (`ambre_primitives::boundaries::tests::a_dependency_on_the_call_worker_and_a_call_worker_with_no_artefact_are_refused`) | none | verified |
| `CALL-005` | `INV` | One sender key MUST NOT seal two chunks under one counter. | this spec section 3 | type-level -- `ambre_crypto::call::ChunkSealer` issues the counter of each seal, and no caller hands it one; `ambre_crypto::call::tests::one_key_never_seals_two_chunks_under_one_counter` | none | verified |
| `CALL-006` | `INV` | The clear prefix of a sealed chunk MUST be authenticated by its seal. | this spec section 3 | `ambre_crypto::call::tests::a_changed_byte_of_the_clear_prefix_drops_the_video_chunk` -- each byte of the prefix, under a bit that keeps its length; `ambre_crypto::call::tests::the_sealed_bytes_of_a_vp8_keyframe_follow_rfc_9605_with_the_prefix_in_the_additional_data` -- a second path written from RFC 9605 alone | none | verified |
| `CALL-147` | `INV` | Over one call, two chunks that one participant session seals MUST NOT share a key and a nonce, through any number of leaves and joins. | this spec section 3, the key schedule; `CALL-005` | `ambre_sync::call::tests::no_key_and_nonce_pair_repeats_over_a_long_call` -- property, over 800 to 1 600 joins, leaves, removals, delays and chunks: no two chunks share a keystream, the oracle the cipher itself gives, and each key handoff moves the generation up; `ambre_call::tests::a_seal_at_or_below_the_held_generation_restarts_no_counter` -- a handoff of the held generation, or of a lower one, leaves the counter running | none | verified |
| `CALL-148` | `INV` | The AEAD of a chunk, under the key and the nonce of the chunk format, MUST agree with the ChaCha20-Poly1305 vectors of Project Wycheproof. | this spec section 3, the chunk format | `ambre_crypto::call::tests::the_chunk_aead_passes_the_wycheproof_vectors` -- the vendored vectors, `fixtures/THIRD_PARTY.md`: each valid vector seals to its ciphertext and its tag and opens back, and each forged tag opens nothing | none | verified |
| `CALL-007` | `INV` | Every audio chunk of one call MUST be sealed at one fixed size. | this spec section 3; `ZK-021` | `ambre_crypto::call::tests::every_audio_chunk_has_one_size` -- one size in every call; `::a_clear_chunk_past_the_fixed_size_is_refused` | none | verified |
| `CALL-096` | `INV` | A client MUST send its camera as three VP8 encodings of three sizes, on one sender. | this spec section 3, the camera; `ambre-spec-budgets.md section 1`, the call video row | `ambre_belt::murmure::tests::three_members_decode_each_others_video_at_the_size_the_sfu_chooses` -- browser tier, under Chrome and under Firefox: each sender asks three encodings scaled down by 4, 2 and 1 under the caps of the call video row, from a camera opened at 1280 pixels across. It encodes the three at sizes in the ratio 1 to 2 to 4, and took the camera after its transform | none | verified |
| `CALL-099` | `OBS` | While one member of a call alone speaks, each other member MUST decode its video at the top size, and the video of every other member at the lowest. | this spec section 3, the SFU; `CALL-041`; `ambre-spec-budgets.md section 1` | `ambre_belt::murmure::tests::three_members_decode_each_others_video_at_the_size_the_sfu_chooses` -- browser tier, under Chrome and under Firefox: while Ada alone speaks, Bob and Carol each decode one video at the top layer and one at the lowest, each at the width its sender encodes that layer at, through the open transform. Once Carol alone speaks, the top size moves to her video at Bob. The top layer is the highest layer the sender encodes a frame on during the wait, at the last width it encoded at; `ambre_belt::murmure::tests::a_top_layer_short_of_cpu_keeps_the_width_it_last_encoded_at` -- on host: a Chrome short of CPU still matches; `ambre_belt::murmure::tests::a_top_layer_that_encodes_nothing_hands_the_top_to_the_next_layer` -- on host: a top layer with no frame is not sent; `ambre_belt::murmure::tests::a_size_the_sfu_did_not_choose_never_matches` -- on host: a size the SFU did not choose fails the wait | none | verified |
| `CALL-077` | `INV` | Each audio slot a client receives MUST play through an audio element of its own. | this spec section 3, the playout; `CALL-045` | `ambre_belt::murmure::tests::a_chosen_speaker_plays_every_slot_and_gives_way_when_it_goes` -- browser tier, under Chrome and under Firefox: Bob plays each audio receiver through an element of its own, and no two elements share a track | none | verified |
| `CALL-091` | `INV` | A gain set by hand MUST keep the echo canceller and the noise filter of the capture on. | this spec section 3, the gain; `CALL-077` | `ambre_belt::murmure::tests::a_gain_set_by_hand_scales_the_sound_at_one_rate` -- browser tier, under Chrome and under Firefox: the capture of a gain set by hand asks the echo canceller and the noise filter on; structural -- the graph reads the capture track, which the browser cancels the echo of inside the capture | none | verified |
| `CALL-008` | `INV` | A client MUST carry the audio and video of a call over one peer connection to the SFU. | this spec section 3 | `ambre_belt::murmure::tests::three_members_decode_each_others_video_at_the_size_the_sfu_chooses` -- browser tier, under Chrome and under Firefox: each page sends its audio and its camera over one peer connection, and decodes the others through it. The audio half is `CALL-066` | none | verified |
| `CALL-066` | `INV` | A client MUST carry the audio of a call over one peer connection to the SFU. | this spec section 3; `CALL-008` | `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier, under Chrome and under Firefox: each page holds one peer connection, which sends and decodes the audio of the call | none | verified |
| `CALL-067` | `INV` | The seal MUST drop no audio chunk of a sender for its size. | `ambre-spec-budgets.md section 1`, the call audio bitrate row; `CALL-007` | `ambre_sfu::tests::the_sfu_asks_each_sender_for_opus_at_the_constant_bitrate_of_a_call` -- the answer of the SFU asks Opus at the target of the row, constant; `ambre_crypto::call::tests::a_frame_at_the_constant_audio_bitrate_fits_a_chunk`; `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier, under Chrome and under Firefox: the call worker tells the page of each frame it drops for its size, and no page hears one over the call | none | verified |
| `CALL-009` | `INV` | Two clients of one call MUST NOT connect to each other. | this spec section 3; `ZK-021` | `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier: the candidate pair each page selects reaches the media port of the SFU | none | verified |
| `CALL-010` | `INV` | The peer connection of a call MUST NOT open a data channel. | this spec section 3 | `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier: no page calls `createDataChannel`, and no local description holds an application section | none | verified |
| `CALL-011` | `INV` | A client MUST attach the chunk transform to a sender before it attaches a track to it. | this spec section 3; `CALL-001` | `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier: the page records each track a sender takes, by `replaceTrack` or `addTrack`, and each one found its transform there; `ambre_belt::murmure::tests::three_members_decode_each_others_video_at_the_size_the_sfu_chooses` -- browser tier: the video sender too | none | verified |
| `CALL-012` | `INV` | The SFU MUST NOT link the crate that opens a chunk. | this spec section 3; `ZK-004` | cargo-deny -- `deny.toml` grants `ambre-crypto`, which opens a chunk, to `ambre-sync`, `ambre-call` and `ambre-logic` alone, so a dependency of `ambre-sfu` on it fails the CI `deny` job | none | verified |
| `CALL-013` | `INV` | The SFU MUST NOT link a C cryptography library. | README pillar "Rust-first"; this spec section 3 | ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads the graph `cargo metadata` resolves and refuses a C cryptography library on any edge it resolves from `ambre-sfu` (`ambre_primitives::boundaries::tests::a_c_cryptography_library_under_the_sfu_is_refused`) | none | verified |
| `CALL-014` | `INV` | The relay MUST NOT hold the secret that mints the credentials of the TURN server. | `ambre-spec-server.md`, `SRV-012`; this spec section 3 | ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses a crate that depends on `ambre-sfu`, whose secrets module alone holds the TURN secret (`ambre_primitives::boundaries::tests::a_crate_that_depends_on_the_sfu_is_refused`); `ambre_sfu::secrets::tests::a_turn_password_is_the_hmac_coturn_checks`; `ambre_belt::call::tests::a_browser_on_tcp_443_alone_decodes_the_call_through_turn` -- browser tier: coturn admits the credential the SFU minted, under the secret the two share | none | verified |
| `CALL-015` | `OBS` | The render runtime MUST draw a remote video, never a page element over the canvas. | this spec section 3, video on the canvas; `CANVAS-001` | `ambre_belt::murmure::tests::a_member_draws_the_video_of_another_on_its_canvas` -- browser tier, under Chrome and under Firefox: Bob's canvas draws Ada's video, two readbacks two seconds apart differ in one band, and no video element enters either page; `ambre_canvas::video::tests::a_picture_keeps_its_aspect_inside_its_tile` | none | verified |
| `CALL-031` | `INV` | A participant MUST complete the DTLS handshake with the SFU through the RustCrypto backend alone. | this spec section 3; `CALL-013` | `ambre_sfu::tests::a_participant_completes_dtls_with_the_sfu_over_rust_crypto` -- an in-memory link between a browser connection and the SFU reaches the connected state, and a wrong fingerprint fails it | none | verified |
| `CALL-032` | `INV` | The SFU MUST forward each RTP packet of a participant to every other participant of its room whose slots carry its track, its payload unchanged. | this spec section 3, the SFU | `ambre_sfu::forward::tests::each_packet_reaches_every_other_participant_of_the_room_unchanged` | none | verified |
| `CALL-036` | `INV` | The SFU MUST serve its signalling on a port of its own, apart from the relay. | `ambre-spec-broadcast.md`, `SRV-049`; this spec section 3, signalling | `ambre_belt::call::tests::two_browsers_decode_each_other_through_the_sfu` -- browser tier | none | verified |
| `CALL-037` | `OBS` | Two browsers in one room MUST each decode the audio and the video of the other through the SFU. | this spec section 3, the stack | `ambre_belt::call::tests::two_browsers_decode_each_other_through_the_sfu` -- browser tier, under Chrome and under Firefox | none | verified |
| `CALL-039` | `INV` | The SFU MUST switch a receiver to another layer of a track only on a keyframe of that layer. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_receiver_switches_layer_only_on_a_keyframe_of_the_new_layer` | none | verified |
| `CALL-040` | `INV` | The sequence numbers of a video track forwarded to one receiver MUST stay continuous across a layer switch. | this spec section 3, the SFU; `CALL-032` | `ambre_sfu::sim::tests::the_sequence_numbers_stay_continuous_across_a_layer_switch` | none | verified |
| `CALL-041` | `INV` | The SFU MUST fill the video slots that the choice of a receiver leaves with the most recent speakers, up to the count of videos that choice names. | this spec section 3, the SFU; `CALL-126`; `ambre-spec-budgets.md section 1` | `ambre_sfu::sim::tests::a_receiver_gets_the_video_of_the_most_recent_speakers_alone` -- a receiver of no choice; `ambre_sfu::sim::tests::the_most_recent_speakers_fill_the_slots_a_choice_leaves` -- a choice of two leaves nine slots to the speakers; `ambre_sfu::sim::tests::a_receiver_that_names_fewer_videos_gets_that_many` -- a count of five beside a receiver of no choice | none | verified |
| `CALL-042` | `OBS` | A browser whose network opens TCP on port 443 alone MUST decode the audio and the video of the call through the TURN relay within the delay of `ambre-spec-budgets.md section 1`. | this spec section 3, the stack | `ambre_belt::call::tests::a_browser_on_tcp_443_alone_decodes_the_call_through_turn` -- browser tier, coturn over TCP behind port 443 at 150 ms of round trip; `ambre_belt::murmure::tests::a_call_on_the_self_hosted_stack_reaches_the_sfu_through_turn_on_443` -- browser tier, against the stack of `deploy/compose/` that `just selfhost-test` starts: a page that gathers relay candidates alone joins through port 443, where the TLS front sends what is no TLS hello to coturn, and both pages decode the other | none | verified |
| `CALL-154` | `INV` | A TURN credential MUST expire within the TURN lifetime of `ambre-spec-budgets.md section 1`. A renewal that keeps its seat while the last credential of the seat expires within the margin of that row MUST bring a fresh credential under the same pseudonym. Any other renewal MUST bring none. | `security-audit-blind-2026-11-17.md`, finding `P60`; this spec section 3, the TURN credential; `CALL-120` | `ambre_sfu::turn::tests::a_credential_expires_one_turn_lifetime_after_it_is_minted`; `ambre_sfu::turn::tests::a_renewal_mints_a_fresh_credential_within_the_margin_alone` -- at the production lifetime and at a short one; `ambre_sfu::serve::tests::a_renewal_near_the_expiry_of_the_credential_answers_a_fresh_one` -- over the socket: a renewal before the margin, a renewal of another seat in the margin and a renewal right after a fresh credential read nothing | none | verified |
| `CALL-155` | `OBS` | A browser relayed through TURN MUST keep decoding the audio and the video of the call past the expiry of its first credential. | `security-audit-blind-2026-11-17.md`, finding `P60`; this spec section 3, the TURN credential; `CALL-042` | `ambre_belt::call::tests::a_relayed_call_flows_past_the_expiry_of_its_first_credential` -- browser tier, coturn over TCP alone, a credential of 30 s renewed 10 s before its expiry: the page restarts ICE onto a new relay allocation, and both pages decode more past the expiry; `ambre_sfu::forward::tests::an_ice_restart_moves_the_connection_to_its_fresh_fragment` -- the audio flows across the restart, and a check of the new allocation reaches its connection | none | verified |
| `CALL-045` | `INV` | The SFU MUST send each receiver the media of a call over the fixed count of audio and video slots of `ambre-spec-budgets.md section 1`. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_receiver_holds_a_fixed_count_of_slots` -- a call of twelve | none | verified |
| `CALL-047` | `INV` | The SFU MUST forward a receiver the audio of the most recent speakers alone, up to the count of `ambre-spec-budgets.md section 1`. | this spec section 3, the SFU | `ambre_sfu::sim::tests::a_receiver_gets_the_audio_of_the_three_most_recent_speakers_alone` | none | verified |
| `CALL-048` | `INV` | The sequence numbers of a slot MUST stay continuous when the slot moves to another sender. | this spec section 3, the SFU; `CALL-040` | `ambre_sfu::sim::tests::the_sequence_numbers_of_a_slot_stay_continuous_across_a_change_of_sender` | none | verified |
| `CALL-049` | `INV` | The SFU MUST move a video slot to another sender only on a keyframe of that sender. | this spec section 3, the SFU; `CALL-039` | `ambre_sfu::sim::tests::a_slot_moves_to_another_sender_only_on_a_keyframe` -- 150 ms of round trip | none | verified |
| `CALL-056` | `INV` | The relay MUST give each join of a call a sender index that no earlier join of that call held. | this spec section 3, signalling; `CALL-005` | `ambre_server::ws::handler::call::tests::each_join_of_a_call_gets_a_sender_index_no_earlier_join_held` -- database tier: a join after a leave gets a fresh index, and a join past the last index a key identifier carries reads a notice, since one script takes the seat and the index (`SRV-248`) | none | verified |
| `CALL-064` | `INV` | A sender key MUST reach the call worker from the Sync worker on a port of their own, never through the Main instance. | this spec section 3, the client of a call; `CALL-004`; `ARCH-004` | cargo-deny -- `deny.toml` grants `ambre-crypto` to no crate of the Main instance, so no code there names a key; `ambre_crypto::call::tests::a_handoff_crosses_the_port_whole`; `ambre_sync::call_link::tests::a_join_command_seats_the_call_it_names_and_hands_the_keys_on_its_port` -- the keys leave on the port of the command alone; `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier | none | verified |
| `CALL-101` | `INV` | The SFU MUST expose its metrics in the Prometheus text format, on a port that the media listener does not serve. | this spec section 3; `ZK-029` | `ambre_sfu::serve::tests::the_metrics_answer_on_their_own_port_under_closed_labels_alone` -- a scrape of the metrics port answers the text format, version 0.0.4, and the signalling port answers `404` on the same route. The media socket is UDP, so it serves no HTTP route | none | verified |
| `CALL-153` | `INV` | A metrics listener of the SFU off the loopback MUST answer only a scrape that presents the token of `SFU_METRICS_TOKEN`, and the SFU MUST NOT start there without one. | this spec section 3, the metrics of the SFU; `CALL-101`; `SRV-318` | `ambre_sfu::serve::tests::a_scrape_behind_a_token_presents_it` -- a served SFU answers `200` to the token and `401` with no metric to a missing or wrong one; `ambre_metrics::tests::a_listener_off_the_loopback_needs_the_token` -- an address off the loopback refuses to start without a token | none | verified |
| `CALL-115` | `INV` | A client MUST send its screen as one VP8 encoding, on a screen sender apart from the camera, capped at the call screen row of `ambre-spec-budgets.md section 1`. | this spec section 3, the screen; `CALL-096` | `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Ada's screen sender holds one encoding, capped at 30 fps and 1 500 kbit/s, and sends VP8 at most 1 920 pixels across | none | verified |
| `CALL-102` | `INV` | A chunk of the screen MUST be sealed under the key identifier of the screen track. | this spec section 3, the screen; `CALL-001` | `ambre_crypto::call::tests::a_screen_chunk_is_sealed_under_the_key_identifier_of_the_screen` -- the header names the screen track, and the screen seals under a key of its own; `ambre_call::tests::a_screen_chunk_leaves_sealed_and_opens_on_a_receiver_of_the_screen_alone`; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Bob decodes Ada's screen through the transform of his screen slot | none | verified |
| `CALL-110` | `INV` | A chunk of the screen MUST open through the open transform of the screen alone. | this spec section 3, the screen; `CALL-022` | `ambre_crypto::call::tests::a_screen_chunk_opens_through_the_door_of_the_screen_alone` -- the camera door and the audio door refuse a screen chunk, and the screen door refuses a camera chunk; `ambre_call::tests::a_screen_chunk_leaves_sealed_and_opens_on_a_receiver_of_the_screen_alone` | none | verified |
| `CALL-104` | `INV` | The SFU MUST forward a screen over the screen slot of each receiver, never over a video slot. | this spec section 3, the SFU; `CALL-045` | `ambre_sfu::forward::tests::a_screen_reaches_each_receiver_over_its_screen_slot_alone` -- the screen slot follows the nine video slots, and no video slot carries a packet of the screen | none | verified |
| `CALL-105` | `INV` | The SFU MUST forward a screen at its one encoding, whatever the downlink estimate of the receiver and whoever speaks. | this spec section 3, the SFU; `CALL-038`; `CALL-041` | `ambre_sfu::sim::tests::a_screen_keeps_its_one_size_under_a_cap_and_a_change_of_speaker` -- eight seeded runs at 150 ms of round trip and 2 % loss, under a cap of 900 kbit/s and through a change of speaker | none | verified |
| `CALL-106` | `INV` | The SFU MUST forward each other receiver of the room the screen of the participant whose share started last. | this spec section 3, one share at a time | `ambre_sfu::forward::tests::the_screen_of_the_last_share_reaches_each_other_receiver_and_none_reaches_its_sharer`; `ambre_sfu::forward::tests::a_still_screen_that_changes_again_takes_no_screen_back` -- Ada's screen holds still for 3 s, then changes, and Bob's share keeps the screen; `ambre_sync::call_link::tests::a_share_start_reaches_the_sfu_alone_and_never_the_relay` -- the page's start reaches the SFU; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: once Bob shares, Ada decodes his screen | none | verified |
| `CALL-111` | `INV` | The SFU MUST forward no screen to the participant whose share started last. | this spec section 3, the SFU; `CALL-033` | `ambre_sfu::forward::tests::the_screen_of_the_last_share_reaches_each_other_receiver_and_none_reaches_its_sharer`; `ambre_sfu::sim::tests::a_screen_keeps_its_one_size_under_a_cap_and_a_change_of_speaker` -- the sharer reads no screen; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Bob decodes no screen while his share is the last one | none | verified |
| `CALL-141` | `INV` | The chart of the repository MUST give each SFU node a public address that no load balancer fronts. | this spec section 3; `SRV-309` | `ambre_belt::murmure::tests::a_call_on_the_self_hosted_stack_reaches_the_sfu_over_udp` -- browser tier, on the stack that `just chart-test` deploys on a kind cluster, in the CI `selfhost` job: the one peer of each page is the media port at the address of the node; ci-gate -- `scripts/chart.py`, run by `just chart-check` in the CI `lint` job, refuses an SFU that is not one pod on the network of its node, and a Service that selects it with its media port or exposes it outside the cluster | none | verified |
| `CALL-142` | `INV` | A chunk a participant seals once the switch delay of a removal passed MUST stay secret from the removed member. | this spec section 3, the leave steps; `CALL-028`; `CALL-030` | `proverif` -- `crates/ambre-crypto/proofs/call_sender_keys.pv`, the query on `chunk_after` and the witness that reaches its seal; `ambre_crypto::call_model_labels::a_chunk_seals_under_the_sframe_labels_of_the_model`; `::a_ratchet_derives_under_the_ratchet_label_of_the_model`; `::a_wrap_opens_under_the_wrap_label_of_the_model`; `::a_signed_join_covers_the_join_label_of_the_model`; `::the_model_declares_and_uses_each_label_of_the_code` | none | verified |
| `CALL-143` | `INV` | A chunk that opens under a sender key drawn after a removal MUST name the participant who sealed it. | this spec section 3, the key identifier; `CALL-025`; audit `N11` | `proverif` -- `crates/ambre-crypto/proofs/call_sender_keys.pv`, the two correspondences on `Accepted` and the witness that reaches each; `ambre_crypto::call_model_labels::a_chunk_seals_under_the_sframe_labels_of_the_model`; `::a_ratchet_derives_under_the_ratchet_label_of_the_model`; `::a_wrap_opens_under_the_wrap_label_of_the_model`; `::a_signed_join_covers_the_join_label_of_the_model`; `::the_model_declares_and_uses_each_label_of_the_code` | none | verified |

### 1.2 Join a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-016` | `POST` | A join MUST hand the joiner the current sender key of each participant, wrapped to the joiner alone. | this spec section 3, the join steps | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` -- the key model: each participant opens every chunk of the others; `ambre_crypto::call::tests::a_wrap_opens_for_its_recipient_and_binding_alone` | none | verified |
| `CALL-017` | `POST` | A join MUST move each participant to the next generation of its sender key before the wrap. | this spec section 3, the join steps | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` -- a joiner opens nothing sealed before its join, under any key it holds or a ratchet of one | none | verified |
| `CALL-018` | `FRAME` | A join MUST NOT show a sender key to the relay or to the SFU. | `ZK-002`; this spec section 3 | type-level -- a join hands the relay a `SignedJoin`, which holds no key, and wraps that open under the X25519 key of their recipient alone. No wire type names a `SenderKey` (`ZK-002`); `ambre_crypto::call::tests::a_wrap_opens_for_its_recipient_and_binding_alone` | none | verified |
| `CALL-019` | `FAIL` | Given a joiner absent from the membership log of the channel, a client MUST wrap no key for it. | this spec section 3, the join steps; `ambre-spec-membership.md` | `ambre_sync::call::tests::a_joiner_outside_the_channel_gets_no_wrap`; `::each_member_opens_exactly_what_it_should` -- an outsider and a removed member are refused, and open nothing; `ambre_sync::call_client::tests::a_joiner_outside_the_channel_gets_no_key_through_the_client` -- through the client and a model of the call socket | none | verified |
| `CALL-020` | `FAIL` | Given a signed ephemeral key the client has seen already, the client MUST refuse it. | this spec section 3, the join steps | `ambre_sync::call::tests::a_join_seen_already_is_refused`; `::each_member_opens_exactly_what_it_should` -- a join delivered again is refused | none | verified |
| `CALL-124` | `FAIL` | Given a signed join that arrives after the client's own and is older than its own by more than the join clock skew of `ambre-spec-budgets.md section 8`, the client MUST wrap no key for it. | this spec section 3, the join steps; `CALL-020` | `ambre_sync::call::tests::a_join_older_than_its_own_is_refused` -- a join of an earlier session, replayed after the skew, is refused, and a join that lags by the skew alone gets its wrap | none | verified |
| `CALL-133` | `FAIL` | Given a join that a client refuses, the client MUST still admit a later join that carries the same exchange key under another identity. | this spec section 3, the join steps; `CALL-020` | `ambre_sync::call::tests::a_join_claiming_another_exchange_key_evicts_nobody` -- a stranger's claim on the exchange key of a later joiner is refused, and a member's claim under its own identity is admitted. The later joiner is admitted after both, and refused when delivered again (audit `N28`) | none | verified |
| `CALL-134` | `FAIL` | Given a membership log of the channel that stands on no root of the client, the client MUST admit no joiner against it. | this spec section 3, the client of a call; `CALL-019`; `SYNC-205`; `SYNC-210`; `SYNC-235` | `ambre_sync::sharing::tests::a_call_log_the_relay_alone_roots_admits_no_one` -- a log the relay creates, with a copy it wraps to the client, reads as no log until a root of the session signed it; `ambre_sync::call_link::tests::a_log_that_stands_on_no_root_admits_no_one` -- a first such log ends the join before any socket opens, and a later one leaves the held log in place (audit `N27`) | none | verified |
| `CALL-057` | `POST` | The relay MUST hand the signed join of a participant, unchanged, to every other participant of the call and to each later joiner. | this spec section 3, signalling; `CALL-016` | `ambre_server::ws::handler::call::tests::a_signed_join_reaches_every_other_seat_unchanged` -- database tier: a seat in the call hears the join, a later seat reads it in its roster, and a second announce changes nothing; `ambre_protocol::tests::the_relay_reads_only_the_listed_fields_of_a_call_frame` -- the join crosses as opaque bytes | none | verified |
| `CALL-058` | `FRAME` | The relay MUST NOT forward a wrap to a participant session other than the one its sender index names. | this spec section 3, signalling; `CALL-030` | `ambre_server::ws::handler::call::tests::a_wrap_reaches_its_recipient_alone_on_the_call_socket` -- database tier: a second session of the same member hears no wrap | none | verified |
| `CALL-059` | `POST` | Two clients that join one call through the relay MUST each hold the sender key of the other. | this spec section 3, the join steps; `CALL-016` | `ambre_sync::call_client::tests::two_clients_over_the_relay_each_hold_the_other_sender_key` -- host tier: two clients, each with its own keys, over a model of the call socket whose rules the database tier holds on the relay (`CALL-056` to `CALL-058`), and a chunk one seals the other opens; `::a_late_client_holds_every_key_from_its_join_on`; `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier: three Sync workers join through the relay and the SFU, and each client decodes the audio of the two others | none | verified |
| `CALL-061` | `FAIL` | Given a signed join whose sender index is not the one the relay gave its seat, a client MUST wrap no key for it. | this spec section 3, signalling; `CALL-056` | `ambre_sync::call_client::tests::a_join_signed_under_another_index_than_its_seat_gets_no_wrap` -- the same join under its own seat gets the wrap | none | verified |
| `CALL-062` | `POST` | A client admitted to a call MUST open the socket of the SFU that its admission names, with the admission as the first frame. | this spec section 3, signalling | `ambre_sync::call_client::tests::an_admission_opens_the_socket_of_the_sfu_with_the_admission_first`; `ambre_sync::call_link::tests::a_join_reads_the_log_then_opens_the_sockets_and_hands_the_key_to_the_port` -- host tier: the worker reads the log, opens the call socket with the token, then the socket of the SFU with the admission, and its key leaves on the port. The browser transport of both sockets runs in the belt of the Call worker and call client Story (`ambre-roadmap.md` E5) | none | verified |
| `CALL-065` | `POST` | A command of the bus that joins a call MUST seat the session in the call and the channel it names, and hand the keys of that call on the port it carries. | this spec section 3, the client of a call; `CALL-062` | `ambre_sync::call_link::tests::a_join_command_seats_the_call_it_names_and_hands_the_keys_on_its_port`; `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier: each page joins by the command, with no gesture | none | verified |
| `CALL-149` | `FAIL` | Given a join of a call, the Sync worker MUST seat it under the `meta` the Logic thread names from its own channel state, never under a document the relay names. | this spec section 3, a join from the page; `CALL-065` | `ambre_sync::call_link::tests::a_join_command_seats_the_call_it_names_and_hands_the_keys_on_its_port` -- the join asks the Logic thread, and its answer alone seats it; `ambre_sync::call_link::tests::a_join_the_logic_thread_names_no_meta_for_ends` -- no `meta` named, no socket, and the page hears the end; `ambre_logic::channels::tests::a_join_takes_the_meta_its_channel_state_names` -- the answer is the `meta` of the open or closed channel, and none for a channel never opened; `ambre_logic::channels::tests::a_join_asked_while_its_channel_opens_waits_for_the_open` -- a channel still opening answers once its open ends, with its `meta` or none | none | verified |
| `CALL-021` | `FAIL` | Given a call at the participant cap of `ambre-spec-budgets.md section 1`, a join MUST be refused with a notice. | this spec section 3 | `ambre_server::ws::handler::call::tests::a_join_at_the_participant_cap_is_refused_with_a_notice` -- database tier: the last seat is admitted, the next join reads the notice, then the socket closes | none | verified |
| `CALL-117` | `FAIL` | Given an admission that seated a connection already, the SFU MUST seat no other connection under it, after that connection leaves too. | audit `N12`; this spec section 3, the SFU seats | `ambre_sfu::serve::tests::an_admission_seats_no_connection_after_its_own_left` -- an admission whose connection closed reads no TURN servers again, and the SFU reads no offer of it | none | verified |
| `CALL-150` | `FAIL` | Given a signalling message longer than the bound of this spec section 3, the SFU MUST close its socket. | audit `N39`; this spec section 3, the SFU seats | `ambre_sfu::serve::tests::a_message_past_the_bound_closes_the_socket` -- one byte past the bound closes the socket before its admission and after it, and a socket under the bound is seated | none | verified |
| `CALL-151` | `FAIL` | Given an admitted socket that sends no offer within the deadline of this spec section 3, the SFU MUST close it. | audit `N39`; this spec section 3, the SFU seats | `ambre_sfu::serve::tests::an_admitted_socket_with_no_offer_closes_at_the_deadline` -- the socket stays open up to the deadline, and closes past it | none | verified |
| `CALL-152` | `FAIL` | Given an admission that opened a signalling socket already, the SFU MUST send another socket under it no TURN credential, and read nothing more of it. | audit `N39`; this spec section 3, the SFU seats; `CALL-117` | `ambre_sfu::serve::tests::a_second_socket_under_one_admission_reads_no_credential` -- three more sockets under the admission of a socket that waits for its offer read nothing, and the first is seated | none | verified |
| `CALL-119` | `FAIL` | Given a seat whose last admission expired with no renewal, the SFU MUST end the seat and close its signalling socket. | audit `N25`; this spec section 3, the SFU seats; `CALL-117` | `ambre_sfu::serve::tests::a_seat_whose_admission_expires_with_no_renewal_ends` -- a seat under an admission of two seconds, never renewed, closes | none | verified |
| `CALL-120` | `FAIL` | Given a renewal that names another room, participant, member or sender index than its seat, the SFU MUST NOT keep the seat past its last admission. The token of a renewal MUST seat no second connection while it is valid. | audit `N25`; this spec section 3, the SFU seats; `CALL-117` | `ambre_sfu::serve::tests::a_renewal_keeps_its_own_seat_alone` -- a renewal of the same seat keeps it past its first expiry, a renewal that changes the participant, the member or the sender index ends the seat at its first expiry, and the token of a renewal seats nothing | none | verified |
| `CALL-121` | `POST` | A join under an admission whose member holds a seat in the room MUST end that older seat, so a member holds one seat of a room. | audit `N25`; this spec section 3, the SFU seats | `ambre_sfu::serve::tests::a_join_of_a_member_ends_its_older_seat` -- the older seat of a member closes at its second join, and the seat of another member stays | none | verified |
| `CALL-122` | `POST` | Each heartbeat of a seat whose member is in the channel MUST send the client a renewed admission that names the room, the participant, the member and the sender index of the first. A heartbeat that finds the member out of the channel MUST send no renewal and end the seat. The client MUST hand each renewal to the SFU. | audit `N25`; this spec section 3, signalling; `SRV-248` | `ambre_server::ws::handler::call::tests::a_heartbeat_renews_the_admission_of_a_member_in_the_channel_alone` -- database tier: a renewal names the seat of the first admission, and once the member leaves the channel, the socket closes and the seat leaves the roster; `ambre_sync::call_link::tests::a_renewal_from_the_relay_reaches_the_sfu_as_an_admission` -- a seated client hands each renewal to the SFU, and a renewal before the seat reaches nothing | none | verified |
| `CALL-156` | `POST` | Each fresh TURN credential the SFU sends during a call MUST reach the page, and the page MUST restart ICE under it. | `security-audit-blind-2026-11-17.md`, finding `P60`; this spec section 3, a join from the page; `CALL-154` | `ambre_sync::call_link::tests::a_fresh_credential_from_the_sfu_reaches_the_page_each_time`; `ambre_belt::murmure::tests::the_page_restarts_ice_with_each_fresh_turn_credential` -- browser tier: two pages of the application on credentials of 30 s, renewed by the heartbeats of the relay, each take a fresh ICE fragment and still decode each other past the first expiry | none | verified |
| `CALL-068` | `FAIL` | Given a join that starts no call, for a call whose state the relay holds none of, the relay MUST refuse the join with a notice that the call ended. | this spec section 3, signalling; `SRV-249` | `ambre_server::ws::handler::call::tests::a_join_of_a_call_the_relay_holds_no_state_of_is_refused_as_ended` -- database tier; `ambre_server::ws::handler::call::tests::a_call_whose_state_is_lost_ends` -- database tier: a rejoin of a lost call is refused | none | verified |
| `CALL-161` | `FAIL` | Given a live call whose state the relay lost with no last leave, the relay MUST let a start in its channel create a new call. | this spec section 3, signalling; `CALL-160` | `ambre_server::ws::handler::call::tests::a_start_takes_the_channel_from_a_call_the_store_lost` -- database tier | none | verified |
| `CALL-076` | `FAIL` | Given a microphone that the browser refuses or does not have, a client MUST send no start and no join of a call. | this spec section 3, a join from the page; `5-Murmure/features/ambre-spec-murmure-calls.md section 3` (`MURMURE-601`) | `ambre_belt::murmure::tests::a_refused_microphone_starts_no_call_and_says_why` -- browser tier, under Chrome and under Firefox: no call worker, no peer connection and no call socket follow a refusal | none | verified |

### 1.3 Send and receive audio and video

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-022` | `POST` | A received chunk MUST reach the decoder only after it opens under the key its identifier names. | this spec section 3, the chunk steps | `ambre_call::tests::a_received_chunk_reaches_the_decoder_only_once_it_opens`; `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier: a member out of the keys decodes nothing; `ambre_call::tests::a_camera_chunk_leaves_sealed_and_opens_on_a_receiver_of_the_camera_alone` -- a video chunk, on a receiver of the camera alone; `ambre_belt::murmure::tests::three_members_decode_each_others_video_at_the_size_the_sfu_chooses` -- browser tier: each video receiver opens through the transform of the camera | none | verified |
| `CALL-146` | `FAIL` | Given a received track whose open transform does not attach, the page MUST play none of it and stop it. | audit `N40`; this spec section 3, the join steps; `CALL-022` | `ambre_belt::murmure::tests::a_receiver_with_no_transform_plays_nothing` -- browser tier: on a page whose receive transforms all fail to attach, every track the peer connection hands over ends and no media element plays one, while the other member decodes the call | none | verified |
| `CALL-055` | `POST` | The open of a chunk MUST hand out its clear bytes only once its tag verifies under the key its identifier names. | this spec section 3, the chunk format; `CALL-022` | `ambre_crypto::call::tests::a_changed_byte_drops_the_chunk`; `::a_chunk_that_names_another_sender_is_dropped`; `::a_sealed_audio_chunk_opens_whole_under_its_sender_key` | none | verified |
| `CALL-023` | `FRAME` | The seal of a chunk MUST NOT change the clear prefix the SFU reads. | this spec section 3, the chunk steps | `ambre_crypto::call::tests::the_seal_of_a_vp8_chunk_keeps_its_prefix_and_adds_a_fixed_overhead` -- the prefix of any VP8 chunk reads the same sealed; `ambre_call::tests::a_camera_chunk_leaves_sealed_and_opens_on_a_receiver_of_the_camera_alone` | none | verified |
| `CALL-097` | `FAIL` | Given a clear VP8 chunk shorter than its clear prefix, or whose keyframe bit differs from the kind the encoder names, the call worker MUST seal nothing of it. | this spec section 3, the chunk steps; `CALL-023` | `ambre_crypto::call::tests::a_vp8_chunk_whose_bit_and_kind_differ_or_shorter_than_its_prefix_seals_nothing` -- no counter spent; `ambre_call::tests::a_camera_chunk_leaves_sealed_and_opens_on_a_receiver_of_the_camera_alone` -- the call worker refuses it | none | verified |
| `CALL-098` | `FAIL` | Given a sealed VP8 chunk whose clear prefix is not the length its keyframe bit names, the receiver MUST drop it. | this spec section 3, the chunk steps; `CALL-006` | `ambre_crypto::call::tests::a_sealed_vp8_chunk_whose_prefix_is_another_length_is_dropped` -- a flipped keyframe bit, a chunk cut inside its prefix or its header, and a chunk of the other track | none | verified |
| `CALL-024` | `FAIL` | Given a chunk that does not open, the receiver MUST drop it. | this spec section 3, the chunk steps | `ambre_crypto::call::tests::a_changed_byte_drops_the_chunk`; `::a_chunk_that_does_not_open_moves_no_window` | none | verified |
| `CALL-025` | `FAIL` | Given a chunk whose key identifier names a sender whose key did not seal it, the receiver MUST drop it. | this spec section 3, the key identifier; audit `N11` | `ambre_crypto::call::tests::a_chunk_that_names_another_sender_is_dropped` -- a chunk Mallory seals under her key with the identifier of Bob, and one of hers she relabels. A member who holds Bob's key seals a chunk that opens under it: the SFU refuses that one (`CALL-116`) | none | verified |
| `CALL-026` | `FAIL` | Given a chunk whose counter falls outside the replay window of its key, the receiver MUST drop it. | this spec section 3, the chunk steps | `ambre_crypto::call::tests::a_chunk_outside_the_replay_window_is_dropped`; `::the_replay_window_agrees_with_a_model_of_the_counters_opened` -- against a model of the counters opened; `::a_chunk_of_a_dropped_generation_does_not_open_again` -- the window of a dropped opener; `::a_replayed_chunk_of_a_fresh_epoch_is_dropped` -- the opener a fresh epoch made survives the retirement it starts | none | verified |
| `CALL-125` | `FAIL` | Given a chunk of a sender whose epoch is older than one a chunk of that sender opened under, the receiver MUST drop it. | this spec section 3, the chunk steps; `CALL-026`, `CALL-116` | `ambre_crypto::call::tests::a_chunk_of_an_older_epoch_does_not_open_once_a_newer_one_opened` -- after the first chunk under the fresh key, a late chunk of the sender and three chunks a removed participant seals under the older key reach no decoder: the same step, a step up and a camera chunk. The older epoch opens until then | none | verified |
| `CALL-135` | `FAIL` | Given a chunk of a sender whose epoch is older than a key of that sender held for the switch delay of `ambre-spec-budgets.md section 1`, the receiver MUST drop it, whether a chunk of the newer epoch opened or not. | this spec section 3, the chunk steps; `CALL-125`, `CALL-027`; audit `N36` | `ambre_crypto::call::tests::an_older_epoch_closes_at_the_switch_delay_with_no_fresh_chunk` -- no chunk of the fresh epoch reaches the receiver, and three chunks a removed participant seals under the older key open before the deadline alone: the same step, a step up and a camera chunk; `::a_second_fresh_key_keeps_the_deadline_of_the_first`; `::an_older_key_after_a_fresh_one_still_closes_at_the_deadline`; `ambre_call::tests::the_worker_closes_an_older_epoch_at_the_switch_delay` -- through the handoffs of the call worker; `::a_fresh_key_after_a_spent_deadline_waits_a_whole_delay` -- the first chunk of a fresh epoch spends its deadline; `::a_step_of_the_current_epoch_keeps_the_deadline_a_late_key_set` | none | verified |
| `CALL-157` | `FAIL` | Given a chunk under a key that its sender drew before a removal the receiver heeded, the receiver MUST drop it once the switch delay of `ambre-spec-budgets.md section 1` passed after the removal, whether a fresh key of that sender arrived or not. | this spec section 3, the chunk steps and the leave steps; `CALL-135`, `CALL-028` | `proverif` -- `crates/ambre-crypto/proofs/call_sender_keys.pv`, the correspondence on `AcceptedLate` and the witness that reaches it; `ambre_sync::call::tests::a_removal_closes_the_older_key_of_a_sender_whose_fresh_wrap_is_withheld` -- the relay withholds the fresh wrap: the removal alone closes the older key, a replayed wrap of it is refused, and the late fresh wrap opens; `ambre_crypto::call::tests::a_removal_closes_the_older_epochs_with_no_fresh_key` -- the same step, a step up and a camera chunk; `::a_second_removal_keeps_the_time_of_the_first_close`; `::the_departures_keep_the_newest_indexes`; `ambre_call::tests::the_worker_closes_an_older_epoch_at_a_removal_with_no_fresh_key` -- through the handoffs of the call worker | none | verified |
| `CALL-100` | `FRAME` | Turning the camera on or off during a call MUST NOT renegotiate the session or replace the transform of the video sender. | this spec section 3, a camera during a call; `CALL-011` | `ambre_belt::murmure::tests::a_member_draws_the_video_of_another_on_its_canvas` -- browser tier, under Chrome and under Firefox: Ada's video sender takes her camera, then gives it up, under its first transform, and her page sets no local description | none | verified |

### 1.4 Leave a call

A participant leaves, or the channel removes a member who is in the call.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-027` | `POST` | A leave MUST move each remaining participant to a fresh sender key within the delay of `ambre-spec-budgets.md section 1`. | this spec section 3, the leave steps | `ambre_sync::call::tests::a_leave_switches_each_remaining_sender_within_the_delay` -- the switch at 5 s, the target of the budget row; `::each_member_opens_exactly_what_it_should` -- a member who left opens nothing after the switch; `ambre_sync::call_client::tests::a_leave_switches_the_remaining_clients_and_the_leaver_opens_nothing_after` -- through the client and a model of the call socket | none | verified |
| `CALL-118` | `FAIL` | Given a tick that finds the switch to a fresh sender key not yet due, the client MUST ask another tick. | audit `N22`; this spec section 3, the leave steps; `CALL-027` | `ambre_sync::call_client::tests::a_tick_before_the_switch_asks_another` -- a tick the wall clock reads before the switch hands no key and asks another, and the switch then shuts the removed participant out | none | verified |
| `CALL-028` | `POST` | A removal entry in the membership log of the channel MUST move each remaining participant to a fresh sender key. | this spec section 3, the leave steps; `SYNC-132` | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` -- a revoke entry of the owner, read from the walked log, switches every remaining sender | none | verified |
| `CALL-029` | `POST` | Leaving a call MUST erase every sender key of that call from the call worker. | this spec section 3, the leave steps | `ambre_call::tests::the_end_of_a_call_erases_every_key_of_it`; `ambre_sync::call_link::tests::a_join_command_seats_the_call_it_names_and_hands_the_keys_on_its_port` -- a leave hands the call worker the end of the call; `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier: the call worker of a member who leaves tells the page it erased its keys | none | verified |
| `CALL-030` | `FRAME` | A leave MUST NOT send a fresh sender key to the participant who left. | this spec section 3, the leave steps | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` -- no wrap reaches a member out of the call, and a member who left keeps every key and opens nothing after the switch | none | verified |
| `CALL-158` | `FRAME` | A removal MUST depart every session of the removed member before the client wraps a fresh sender key. | this spec section 3, the leave steps; `CALL-028`, `CALL-030` | `ambre_sync::call::tests::a_removal_departs_every_session_of_the_member_before_a_fresh_key` -- Eve sits in the call from two sessions, and the one fresh key reaches Roger alone; `proverif` -- `crates/ambre-crypto/proofs/call_sender_keys.pv`, phase 1 wraps one fresh key to the members of the log alone, and the query on `chunk_after` holds | none | verified |
| `CALL-159` | `FAIL` | Given a key that a participant who joined after a removal the receiver heeded drew under a membership log behind that removal, the receiver MUST drop its chunks once the switch delay of `ambre-spec-budgets.md section 1` passed after the removal. | this spec section 3, the chunk steps; `CALL-157`, `CALL-138` | `proverif` -- `crates/ambre-crypto/proofs/call_sender_keys.pv`, the correspondence on `AcceptedLate` with a late joiner under the rolled back log, and the witness on `g_late` that a late joiner under the current log reaches; `ambre_sync::call::tests::a_joiner_under_a_rolled_back_log_has_its_key_closed_at_the_removal` -- the joiner wraps its key to Eve, the receiver hands its close inside the delay and refuses it past the delay, and a joiner under the current log opens with no close; `ambre_sync::call::tests::a_rolled_back_joiner_opens_again_once_it_heeds_the_removal` -- the fresh key of the joiner names the current log, opens past the delay and reaches no session of Eve; `ambre_crypto::call::tests::a_wrap_opens_for_its_recipient_and_binding_alone` -- the wrap carries the place; `ambre_crypto::call_model_labels::a_wrap_opens_under_the_wrap_label_of_the_model` -- the place closes the plaintext of the wrap | none | verified |
| `CALL-060` | `POST` | A client in a call MUST read the membership log of the channel again when the members of the channel change, and switch its key away from a member the log no longer holds. | this spec section 3, the leave steps; `CALL-028` | `ambre_sync::call_link::tests::a_change_of_the_members_of_the_channel_reads_the_log_again` -- a change of the members, or a rotation of the key, of the channel reads the log; `ambre_sync::call_client::tests::a_member_removed_from_the_channel_holds_no_key_of_the_next_generation` -- the relay still carries the removed member's socket, and it opens nothing sealed after the switch, under any key it held or a ratchet of one; `ambre_belt::murmure::tests::three_clients_decode_each_others_sealed_audio_through_the_sfu` -- browser tier: a member leaves the channel during the call, through the relay. Past the switch, the SFU still forwards her the audio of the others, and she decodes none of it | none | verified |
| `CALL-136` | `POST` | A client in a call MUST read the membership log of the channel again once it signs a revoke on that channel. | this spec section 3, the client of a call; `CALL-060`, `CALL-028`; audit `N35` | `ambre_sync::call_link::tests::a_revoke_this_client_signs_reads_the_log_of_the_call_again` -- the relay pushes neither a change of the members nor a rotation | none | verified |
| `CALL-137` | `POST` | A client in a call MUST read the membership log of the channel again when a frame of the channel carries a head past the held log, and a member that log allows signed the entry the head names. | this spec section 3, the client of a call; `CALL-060`; `ambre-spec-membership.md`, `SYNC-219`; audit `N35` | `ambre_sync::call_link::tests::a_signed_head_past_the_log_of_the_call_reads_it_again` -- the owner's revoke on a presence sample reads the log, while a held head and an entry Eve signs read nothing | none | verified |
| `CALL-138` | `FAIL` | Given a membership log of the channel that does not pass through the head of the log a client in a call holds, the client MUST keep the held log. | this spec section 3, the client of a call; `CALL-134`; audit `N35` | `ambre_sync::call_link::tests::an_older_log_does_not_replace_the_log_the_call_holds` -- the older genuine log, which still holds the revoked member, leaves the held log in place; `ambre_sync::call_link::tests::a_newer_log_replaces_the_log_the_call_holds` -- a log through the held head replaces it | none | verified |
| `CALL-063` | `POST` | A seat that leaves its call with no close of its socket MUST reach the other participants as a leave. | this spec section 3, signalling; `SRV-251`; `CALL-027` | `ambre_server::ws::handler::call::tests::a_seat_that_stops_its_heartbeat_leaves_the_roster_and_frees_its_place` -- database tier: the heartbeat that finds the expired seat announces its leave, once | none | verified |
| `CALL-069` | `POST` | The relay MUST answer a leave with whether its seat was the last of the call. | this spec section 3, signalling; `5-Murmure/features/ambre-spec-murmure-calls.md section 3` (`MURMURE-591`) | `ambre_server::ws::handler::call::tests::a_leave_answers_whether_its_seat_was_the_last` -- database tier | none | verified |
| `CALL-073` | `FAIL` | Given a call socket that answers no ping of the relay for the time to live of `ambre-spec-budgets.md section 1`, the relay MUST close it and take its seat out of the call. | this spec section 3, signalling; `SRV-251`; `MURMURE-592` | `ambre_server::ws::handler::call::tests::a_call_socket_silent_for_the_time_to_live_leaves_its_seat` -- database tier | none | verified |

### 1.5 Forward a packet

A participant sends a packet to the SFU, and the SFU forwards it within the room.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-033` | `FRAME` | The SFU MUST NOT send a packet to its own sender, or to a participant of another room. | this spec section 3, the SFU; `CALL-009` | `ambre_sfu::forward::tests::no_packet_goes_back_to_its_sender_or_into_another_room` | none | verified |
| `CALL-116` | `FAIL` | Given a chunk whose key identifier names a sender other than the index the relay signed into the admission of its connection, the SFU MUST NOT forward it. | audit `N11`; this spec section 3, the SFU seats; `CALL-025` | `ambre_sfu::forward::tests::a_chunk_under_the_identifier_of_another_sender_never_leaves_the_sfu` -- an audio chunk and the first packet of a camera keyframe that Mallory sends under Bob's index reach nobody, and her own chunk passes; `ambre_belt::call::tests::a_chunk_under_another_sender_index_reaches_nobody` -- browser tier, through the SFU binary: Mallory writes another index into the header of each frame she sends, and Ada decodes no picture and receives no audio packet of hers, while Mallory decodes Ada | none | verified |
| `CALL-139` | `FAIL` | Given a track in a codec other than Opus or VP8, the SFU MUST NOT forward a packet of it. | this spec section 3, the SFU seats; `CALL-116`; audit `N37` | `ambre_sfu::forward::tests::a_camera_in_a_codec_other_than_vp8_never_leaves_the_sfu` -- Mallory offers H264 alone for her camera, and her packets under Bob's index reach nobody | none | verified |
| `CALL-140` | `FAIL` | Given a packet that starts a frame and holds no whole chunk header where the receiver reads one, the SFU MUST NOT forward it. | this spec section 3, the SFU seats; `CALL-116`; audit `N37` | `ambre_sfu::forward::tests::a_frame_start_cut_inside_its_chunk_header_never_leaves_the_sfu` -- a keyframe start cut inside its header, the rest under Bob's index in the next packet, reaches nobody, and her own keyframe passes; `ambre_sfu::forward::tests::a_loud_packet_that_names_no_seat_takes_no_speaker_slot` -- an audio packet with no header reaches nobody | none | verified |
| `CALL-123` | `FAIL` | Given an audio packet whose chunk header does not name the sender index of its seat, the SFU MUST NOT count it as speech for the choice of the audio slots. | audit `N25`; this spec section 3, the playout; `CALL-045` | `ambre_sfu::forward::tests::a_loud_packet_that_names_no_seat_takes_no_speaker_slot` -- loud packets of Mallory with no chunk header take no slot from three speakers, and her latest one reaches nobody | none | verified |
| `CALL-144` | `FAIL` | Given a datagram of a new source, the SFU MUST hand it to no connection, unless it is an ICE check. A check MUST reach the one connection its username fragment names. | audit `N41`; this spec section 3, the routes of the SFU | `ambre_sfu::forward::tests::a_new_source_reaches_a_connection_by_its_check_alone` -- an RTP packet of a stranger reaches no connection and leaves no route, and a check replayed from the stranger reaches the connection of its client alone | none | verified |
| `CALL-145` | `FAIL` | Given a participant that reaches the SFU from more sources than the cap of this spec section 3, the SFU MUST forget its oldest source first. | audit `N41`; this spec section 3, the routes of the SFU | `ambre_sfu::forward::tests::a_participant_holds_no_more_routes_than_the_cap` -- a check from 20 sources leaves the cap of routes, the newest kept and the oldest forgotten | none | verified |
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
| `CALL-162` | `POST` | Given a camera layer that sends no packet for the silence of the SFU layer expiry row of `ambre-spec-budgets.md section 3`, the SFU MUST forward each of its receivers a layer that still sends, within the delay of that row. | this spec section 3, the SFU; `CALL-039` | `ambre_sfu::sim::tests::a_top_layer_that_stops_hands_its_receivers_the_layer_below` -- eight seeded runs at 150 ms of round trip and 2 % loss: the speaker stops its top layer, and both receivers get its middle layer; `ambre_sfu::layer::tests::a_silent_layer_has_no_live_rate_until_it_sends_again` -- the meter alone | none | verified |

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

### 1.9 Ask who is in a call

A member of the channel, in the call or not, asks the relay for the seats of a call.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-070` | `POST` | The relay MUST answer a member of the channel with the signed join of each seat of the call, or with no state once it holds none. | this spec section 3, the roster; `ZK-022` | `ambre_server::ws::handler::call::tests::a_member_out_of_the_call_reads_the_signed_join_of_each_seat` -- database tier | none | verified |
| `CALL-071` | `FAIL` | Given an asker who is no member of the channel, or a call of another channel, the relay MUST answer as for a call it holds no state of. | this spec section 3, the roster; `CALL-019` | `ambre_server::ws::handler::call::tests::a_stranger_or_another_channel_reads_a_running_call_as_gone` -- database tier | none | verified |
| `CALL-072` | `FRAME` | An ask MUST NOT change the state of the call in the ephemeral store. | this spec section 3, the roster; `ZK-024` | `ambre_server::ws::handler::call::tests::an_ask_changes_nothing_of_the_store` -- database tier: the keys of the call, their fields and their expiry read the same | none | verified |

### 1.10 Mute a call

A participant mutes its microphone, and unmutes it later.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-074` | `POST` | A mute MUST stop the sound of its sender: each receiver decodes silence alone from it, until the unmute. | this spec section 3, mute | `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` -- browser tier, under Chrome and under Firefox: under the mute, Bob decodes a frame every 20 ms and reads no audio energy from Ada, and the unmute brings it back | none | verified |
| `CALL-075` | `FRAME` | A mute MUST NOT change the rate or the size of the chunks its sender sends to the SFU. | this spec section 3, mute; `ZK-025` | `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` -- browser tier, under Chrome and under Firefox: in each window, Ada sends payload bytes that make a whole number of sealed chunks of `ambre_core::AUDIO_CHUNK_SIZE`. Over the live window, over the span of the mute, its press included, and over the mute window, she sends one chunk per 20 ms frame of the encoder, to one chunk at each edge of a window | none | verified |


### 1.11 Change a device

A participant picks another microphone or another speaker during a call, or a device goes away.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-078` | `POST` | A change of microphone during a call MUST NOT stop the chunks of its sender: each receiver opens a chunk of every frame across the change. | this spec section 3, devices | `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` -- browser tier, under Chrome and under Firefox: with a microphone that takes 500 ms to open, Bob conceals no more samples of Ada across the change than Ada conceals of Bob over the same window | none | verified |
| `CALL-079` | `FRAME` | A change of microphone MUST NOT renegotiate the session, replace the transform of the sender or draw a sender key. | this spec section 3, devices; `CALL-011` | `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` -- browser tier, under Chrome and under Firefox: across the change, Ada keeps one peer connection, one call worker and the same transform object, and sets no local description; structural -- the change of microphone writes no command to the Sync worker, which alone draws a sender key (`CALL-027`, `CALL-028`) | none | verified |
| `CALL-080` | `POST` | The track of a new microphone MUST take the mute state of the track it replaces. | this spec section 3, devices; `CALL-074` | `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` -- browser tier, under Chrome and under Firefox: Ada changes the microphone muted, Bob reads silence, and her unmute brings the new microphone | none | verified |
| `CALL-081` | `FAIL` | Given a chosen microphone that the browser no longer has, the client MUST send the sound of the default microphone in its place. | this spec section 3, devices | `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` -- browser tier, under Chrome and under Firefox: the track ends and the browser lists the device no more, and Bob hears the default microphone; `ambre_murmure::ui::call_devices::tests::a_chosen_device_that_went_away_resolves_to_the_default_one` | none | verified |
| `CALL-082` | `POST` | A chosen speaker MUST play every audio slot of the call, a slot that arrives after the choice included. | this spec section 3, devices; `CALL-077` | `ambre_belt::murmure::tests::a_chosen_speaker_plays_every_slot_and_gives_way_when_it_goes` -- browser tier, under Chrome: a speaker chosen before the call routes each slot that arrives, and one chosen during the call moves every slot. Headless Firefox lists an output only over an audio server | none | verified |
| `CALL-083` | `FAIL` | Given a chosen speaker that the browser no longer has, the client MUST play the call on the default output. | this spec section 3, devices | `ambre_belt::murmure::tests::a_chosen_speaker_plays_every_slot_and_gives_way_when_it_goes` -- browser tier, under Chrome: the browser lists the speaker no more, and every slot plays on the default output; `ambre_murmure::ui::call_devices::tests::a_chosen_device_that_went_away_resolves_to_the_default_one` | none | verified |

### 1.12 Set the volume of a participant

A member turns another participant of the call up or down, on its own device.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-084` | `POST` | The call worker MUST tell the page the sender of an audio slot or a video slot at the first chunk it opens from a new sender in that slot, and at no other chunk. | this spec section 3, the volumes; `CALL-045` | `ambre_call::tests::a_slot_tells_the_page_of_each_new_sender_once` -- the sender comes from the key identifier of the opened chunk; `ambre_call::tests::a_video_slot_tells_the_page_of_each_new_sender_once` -- a video slot names its sender the same way; `ambre_core::call::tests::a_slot_sender_reads_back_from_its_bytes`; `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: the slot that carries Bob after he joins again plays at his volume | none | verified |
| `CALL-085` | `POST` | A member who turns a participant down to nothing MUST play none of the sound of that participant. | this spec section 3, the volumes; `CALL-077` | `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: while Bob alone speaks, the slot his sent level reaches plays at no volume, and Ada plays no energy of him; `ambre_murmure::ui::call_volumes::tests::a_volume_follows_its_member_from_slot_to_slot` | none | verified |
| `CALL-086` | `POST` | A volume set for one participant MUST leave each other participant at its own volume. | this spec section 3, the volumes; `CALL-077` | `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: while Carol alone speaks, her slot plays at the full volume, and Ada plays her energy; `ambre_murmure::ui::call_volumes::tests::a_volume_follows_its_member_from_slot_to_slot` | none | verified |
| `CALL-087` | `POST` | The volume of a participant MUST apply to each audio slot that carries it, a slot it moves to after the volume was set included. | this spec section 3, the volumes; `CALL-047` | `ambre_murmure::ui::call_volumes::tests::a_volume_follows_its_member_from_slot_to_slot`; `::a_new_sender_index_of_one_member_keeps_its_volume`; `ambre_sync::call_client::tests::the_page_learns_the_member_of_each_admitted_sender_and_its_leave`; `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: Bob and Carol leave and join again in the other order, Bob reaches Ada through another slot, and Ada still plays nothing of him | none | verified |
| `CALL-088` | `POST` | A slot that changes sender MUST take the volume of the new sender as soon as the call worker tells the page of it, before the audio of the former sender plays out. | this spec section 3, the volumes | `ambre_murmure::ui::call_volumes::tests::a_slot_takes_the_volume_of_its_new_sender_at_the_note` -- the note itself answers the volume of the new sender, whatever the former one played at; structural -- the call worker posts the note at the transform, which the browser runs before its jitter buffer | none | verified |
| `CALL-095` | `FRAME` | A change of volume MUST NOT post anything to a worker, nor change the track or the encoder of the sender. | this spec section 3, the volumes; `ZK-027` | `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: setting a volume posts no message to a worker or a port; structural -- a volume writes the element and the local storage alone, and no command of the bus carries a volume | none | verified |

### 1.13 Set the gain of the microphone

A member sets the gain of its microphone by hand during a call.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-089` | `POST` | A gain set by hand MUST turn the automatic gain of the capture off. | this spec section 3, the gain | `ambre_belt::murmure::tests::a_gain_set_by_hand_scales_the_sound_at_one_rate` -- browser tier, under Chrome and under Firefox: a gain set by hand opens the microphone again with the automatic gain off, and the return to the automatic gain opens it with the automatic gain on | none | verified |
| `CALL-090` | `POST` | A gain set by hand MUST scale the sound its sender sends. | this spec section 3, the gain | `ambre_belt::murmure::tests::a_gain_set_by_hand_scales_the_sound_at_one_rate` -- browser tier, under Chrome and under Firefox: at a gain of nothing Bob reads silence from Ada, and at one and a half he hears her and decodes every frame | none | verified |
| `CALL-092` | `FRAME` | A change of gain MUST NOT change the rate or the size of the chunks its sender sends, renegotiate the session, replace the transform of the sender or draw a sender key. | this spec section 3, the gain; `ZK-027`; `CALL-079` | `ambre_belt::murmure::tests::a_gain_set_by_hand_scales_the_sound_at_one_rate` -- browser tier, under Chrome and under Firefox: at each gain, Ada sends at least nine tenths of the chunks of the window before, each of the one size, and keeps one peer connection, one call worker and the same transform, and sets no local description; structural -- a change of gain posts no command to a worker | none | verified |
| `CALL-093` | `POST` | Under a gain set by hand, a mute MUST stop the sound of its sender. | this spec section 3, the gain; `CALL-074` | `ambre_belt::murmure::tests::a_gain_set_by_hand_scales_the_sound_at_one_rate` -- browser tier, under Chrome and under Firefox: under a gain of one and a half, the mute brings Bob silence | none | verified |
| `CALL-094` | `POST` | A change of microphone MUST keep the gain set by hand. | this spec section 3, the gain; `CALL-078` | `ambre_belt::murmure::tests::a_gain_set_by_hand_scales_the_sound_at_one_rate` -- browser tier, under Chrome and under Firefox: a change of microphone under the gain opens the new one with the automatic gain off, the sender takes no new track, the panel keeps the gain and Bob hears Ada | none | verified |

### 1.14 Share a screen

A participant sends a screen or a window into the call, and stops it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-103` | `FRAME` | After the first share of a call, a start or a stop of a share MUST NOT renegotiate the session or replace the transform of the screen sender. | this spec section 3, the screen; `CALL-011`; `CALL-112` | `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: after their first share, Ada's take-over stop and Bob's end set no local description, under the first transform of each screen sender | none | verified |
| `CALL-107` | `POST` | A client that shares MUST stop its share once the SFU tells it that a later share took the screen. | this spec section 3, one share at a time; `CALL-106` | `ambre_sfu::forward::tests::a_later_share_tells_the_former_sharer_alone_each_time` -- Ada, then Bob, then Ada again: each former sharer hears it once, and no one else; `ambre_sync::call_link::tests::a_share_start_reaches_the_sfu_alone_and_never_the_relay` -- the word of the SFU reaches the page; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Bob's share stops Ada's, then Ada's second share stops Bob's | none | verified |
| `CALL-108` | `POST` | Given a screen track that the browser ends, the client MUST stop its share: the screen sender gives up its track. | this spec section 3, the screen | `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Bob's browser ends his track, and his screen sender gives it up | none | verified |
| `CALL-109` | `FAIL` | Given a screen capture that the browser refuses or the member cancels, the call MUST go on with its audio and its camera, and the screen sender MUST take no track. | this spec section 3, the screen; `5-Murmure/features/ambre-spec-murmure-calls.md section 3` (`MURMURE-634`) | `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Bob's browser refuses his capture, his screen sender takes no track, and he still decodes Ada | none | verified |
| `CALL-112` | `POST` | The first share of a call MUST add the screen sender by one offer of the page, which the SFU answers. | this spec section 3, the screen; `CALL-008` | `ambre_sfu::forward::tests::a_first_share_adds_the_screen_section_by_one_offer_and_the_camera_flows_across`; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Ada's first share and Bob's first share each set one local description | none | verified |
| `CALL-113` | `FRAME` | The offer of a first share MUST NOT stop the camera of its sharer. | this spec section 3, the screen; `CALL-096` | `ambre_sfu::forward::tests::a_first_share_adds_the_screen_section_by_one_offer_and_the_camera_flows_across` -- each receiver reads every camera packet across the offer; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Ada's three camera encodings send on across her first share, and Bob decodes her camera at the top size while her screen goes too | none | verified |
| `CALL-114` | `FAIL` | Given an offer of a participant that crosses an offer of the SFU still unanswered, the SFU MUST answer none of it. | this spec section 3, the screen | `ambre_sfu::forward::tests::an_offer_that_crosses_an_offer_of_the_sfu_gets_no_answer_until_it_is_answered` -- the next offer, once the participant answered, gets its answer | none | verified |

* **Budget.** the call rows of `ambre-spec-budgets.md section 1` and section 3. They cover the
  chunk transform, the worker load, the key switch, the cap, the latency and the SFU load.
* **Boundary.** every chunk leaves sealed under a sender key that neither the relay nor the SFU
  holds (`CALL-001`, `CALL-018`). What each of them learns of a call is bounded by `ZK-021` and
  `ZK-022`.
* **Verification.** 162 `verified`. The relay admits under the cap, and the SFU mints the TURN
  credentials (`CALL-021`, `CALL-014`). The SFU links no C cryptography, forwards RTP and reaches a
  browser through TURN (`CALL-013`, `CALL-031` to `CALL-044`, `CALL-050` to `CALL-052`). Each
  receiver holds fixed slots, and a seed gives one path (`CALL-045` to `CALL-053`). The seal and the
  keys hold on the host (`CALL-016` to `CALL-030`, `CALL-054`, `CALL-055`).

---

### 1.15 Choose the videos a receiver gets

A receiver names the senders whose video it wants, and one of them it shows large.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-126` | `POST` | The SFU MUST forward a receiver the video of each sender its choice names, up to the count of `ambre-spec-budgets.md section 1`. | this spec section 3, the choice of videos; `CALL-041` | `ambre_sfu::sim::tests::a_receiver_gets_the_video_of_the_senders_it_chose` -- a call of twelve: two senders outside the nine speakers | none | verified |
| `CALL-127` | `POST` | Given a chosen sender who leaves the call, the SFU MUST give its video slot to the most recent speaker the receiver does not get. | this spec section 3, the choice of videos; `CALL-041` | `ambre_sfu::sim::tests::a_chosen_sender_who_leaves_frees_its_slot_to_a_speaker` | none | verified |
| `CALL-128` | `POST` | The SFU MUST forward the sender a receiver marks large at the top layer the estimate holds, in place of the most recent speaker. | this spec section 3, the choice of videos; `CALL-099` | `ambre_sfu::sim::tests::a_large_sender_gets_the_top_layer_in_place_of_the_speaker` -- the speaker drops to the lowest layer at the chooser alone | none | verified |
| `CALL-129` | `POST` | The SFU MUST ask a keyframe of each sender that a choice brings to a receiver, within the chosen sender switch row of `ambre-spec-budgets.md section 3`. | this spec section 3, the choice of videos; `CALL-049` | `ambre_sfu::sim::tests::a_chosen_sender_is_asked_a_keyframe_and_shows_within_the_budget` -- the synthetic senders send a keyframe on request alone | none | verified |
| `CALL-130` | `FRAME` | The SFU MUST NOT ask one layer of a sender for a keyframe again before a keyframe of that layer passes or the keyframe retry interval of this spec section 3 ends. | this spec section 3, the choice of videos; `CALL-129` | `ambre_sfu::sim::tests::ten_receivers_who_choose_one_sender_ask_it_one_keyframe` -- ten receivers choose one sender 10 ms apart, before the keyframe of the first ask comes back, and each one decodes it | none | verified |
| `CALL-131` | `FAIL` | Given a choice that names more senders than the desktop count of `ambre-spec-budgets.md section 1`, a count of videos past that count or under its senders, a sender twice, or a large sender outside it, the SFU MUST read none of it. | this spec section 3, the choice of videos; `CALL-126` | type-level -- `ambre_protocol::signaling::VideoChoice` has one door, its constructor, and its deserialisation goes through it; `ambre_protocol::tests::a_choice_of_too_many_videos_or_a_stray_large_sender_reads_as_nothing` | none | verified |
| `CALL-132` | `POST` | A command of the bus that chooses videos MUST reach the socket of the SFU alone, never the relay. | this spec section 3, the choice of videos; `ZK-044` | `ambre_sync::call_link::tests::a_choice_of_videos_reaches_the_sfu_alone_and_never_the_relay`; `ambre_sync::call_link::tests::a_choice_before_the_offer_leaves_after_it` -- a choice the page posts before its first offer waits for it | none | verified |

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The call holds | Each user sees | Test |
|---|---|---|---|---|
| joins | is in the call | two participants, one sender key each | each the video of the other (`CALL-016`) | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` |
| joins late | talks | Bob's key, one generation up | Alice hears Bob from her join on, and nothing before (`CALL-017`) | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` |
| leaves | stays | Bob's fresh key | Bob goes on alone, and Alice opens nothing after the switch (`CALL-027`) | `ambre_sync::call::tests::a_leave_switches_each_remaining_sender_within_the_delay` |
| is removed from the channel | stays | Bob's fresh key | Alice's client opens nothing after the switch (`CALL-028`) | `ambre_sync::call::tests::each_member_opens_exactly_what_it_should` |
| sends chunks under the track of Bob, as Mallory | listens | two sender keys | nothing of the forged chunks (`CALL-025`, `CALL-116`) | `ambre_sfu::forward::tests::a_chunk_under_the_identifier_of_another_sender_never_leaves_the_sfu` |

---

## 3. MODEL

**CRDT containers.** none. A call message names the call in its channel, and
`5-Murmure/ambre-spec-murmure-schema.md section 3` declares it.

**The stack, layer by layer.**

| Layer | Choice | Runs on | Rust |
|---|---|---|---|
| capture | the browser capture of the camera, the microphone and the screen | the UI thread | no |
| echo, noise and gain | the three capture constraints; remote audio plays as a stream in a detached audio element | the browser | no |
| encode and decode | the browser codecs: Opus with in-band error correction at a constant rate, VP8 at three sizes for the camera, and VP8 at one size for the screen | the browser | no |
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
   audio. The keyframe bit of VP8, bit 0 of the first byte, names the length. A chunk shorter than
   its prefix, or whose bit differs from the kind the encoder names, seals nothing (`CALL-097`).
2. Pad an audio chunk to the fixed size of its call (`CALL-007`).
3. Write the header of RFC 9605: the key identifier, then the counter.
4. Derive the nonce from the counter by the key schedule of RFC 9605.
5. Seal the rest under ChaCha20-Poly1305 with a 16-byte tag, as private suite `0xF001`. The header
   and the clear prefix are the additional data (`CALL-006`).
6. On receive, read the length of the prefix from the same bit. Check the replay window of the
   key, then open, then hand the chunk to the decoder (`CALL-022`). A chunk that fails any step is
   dropped (`CALL-024`, `CALL-026`). A prefix of another length moves the header, so the parse or
   the tag fails (`CALL-098`).

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
| any | the clear prefix | none for audio. For VP8, 10 bytes on a keyframe and 1 byte otherwise, which the keyframe bit of the first byte names. The seal leaves it unchanged (`CALL-023`) |
| 1 | the config byte of RFC 9605 | `0xFF`: a key identifier and a counter of 8 bytes each. A parser refuses any other value |
| 8 | the key identifier, big-endian | the generation in bits 16 to 63, the sender index in bits 2 to 15, the track in bits 0 and 1 |
| 8 | the counter, big-endian | issued from 1 up by the one sealer of its key identifier (`CALL-005`) |
| 162 for audio | the ciphertext | audio: a big-endian `u16` length, the clear chunk, then zeros up to 162 bytes (`CALL-007`). Video: the clear chunk past its prefix, with no padding (`ZK-028`) |
| 16 | the Poly1305 tag | over the clear prefix and the header, the additional data |

* A sealed audio chunk is 195 bytes in every call. A clear audio chunk holds 160 bytes at most:
  Opus at a constant 64 kbit/s writes that many per 20 ms.
* The SFU asks each sender for Opus at the constant bitrate of the call audio bitrate row of
  `ambre-spec-budgets.md section 1`, through `maxaveragebitrate` and `cbr`. The sender caps
  its encoder at the same bitrate. A frame past the fixed size is dropped, and the call worker
  tells the page of each one (`CALL-067`).
* A sealed video chunk is its clear chunk and 33 bytes: the header and the tag. The SFU reads the
  activity of each picture from its size, as the zero-knowledge spec states (`ZK-028`).
* RFC 9605 writes the shortest form of each field. The fixed lengths here give every header one size.
* The key schedule is the one of RFC 9605 section 4.4.2, over HKDF-SHA256. From the sender key `K`,
  `HKDF-Extract("", K)` gives a secret. Under the label `SFrame 1.0 Secret key `, the key
  identifier and `0xF001`, it expands to the 32-byte key; under `SFrame 1.0 Secret salt `, to a
  12-byte salt.
* The nonce is the salt XOR the counter, left-padded to 12 bytes. So each track of a sender seals
  under a key and a nonce of its own.
* A sealer issues its counters from 1. So the call worker takes a key handoff only above the
  generation it seals under (`CALL-147`). A handoff at or below it starts no counter again. The
  Sync worker moves its generation up at each handoff, a join and a switch alike.
* The AEAD agrees with the ChaCha20-Poly1305 vectors of Project Wycheproof, run through the key
  and the nonce of this format (`CALL-148`).
* A generation is an epoch in bits 32 to 47, then a step in bits 0 to 31. A ratchet expands
  `HKDF-Extract("", K)` under `ambre-call-ratchet-v1` to the key one step up. A fresh key opens the
  next epoch at step zero, so a ratchet and a fresh key never share a generation.
* A receiver ratchets a held key up to 64 steps inside its epoch, never into another. It keeps a
  window of 128 counters per key identifier, marked once a tag verifies. It keeps the 8 newest key
  identifiers of each sender, and refuses a generation below one it dropped.
* A chunk of a sender opens under a newer epoch. The receiver then drops every key and window of
  the older epochs of that sender (`CALL-125`). A removed participant holds those keys, and a
  malicious SFU forwards her chunks. A chunk of the sender still in flight under them is lost.
* A malicious SFU also forwards no chunk of the fresh epoch. So a fresh key of a sender sets a
  deadline: the switch delay of the budget row, on the monotonic clock of the call worker. At
  the deadline the receiver drops the older epochs of that sender, chunk or not (`CALL-135`). A
  second fresh key inside the delay keeps the deadline of the first, as the sender does.
* A malicious relay may also withhold the wrap of the fresh key, so no deadline starts. A
  removal therefore sets a deadline of its own: the switch delay, from the removal the
  receiver heeds. At it, the receiver drops each key a sender drew before the removal, fresh
  key or not (`CALL-157`). That sender goes silent rather than open chunks Eve can forge.
* Each wrap names the sessions its sender had departed when it drew the key: its departures.
  A key that names a removed session as departed was drawn after the removal. A key that
  names none was drawn before it, and the Sync worker hands its close with it. Past the
  delay, the Sync worker refuses the wrap of such a key.
* A participant who joins after the removal chooses whom it wraps to from its own log. The
  relay may serve that joiner the log before the removal, and the joiner then wraps to Eve.
  So each wrap also names the place of the membership log its sender walked when it drew the
  key. The departures of a joiner name none of the removed sessions, so the receiver reads
  the place instead. A key of a joiner counts as drawn after the removal only at the place of
  the log of the removal or past it (`CALL-159`). Below it, the key closes as a key drawn
  before the removal does.
* The joiner opens again once it reads the current log and draws a fresh key, as its own
  removal of Eve does.
* A removal that the log of the space alone makes moves no place of the log of the channel.
  The place then tells a receiver nothing about a joiner, which `CALL-137` alone bounds.

**The join and the wrap, byte by byte.**

| Item | Bytes |
|---|---|
| the signed transcript of a join | `ambre-call-join-v1`, the call, the channel, the member, the Ed25519 identity, the time in ms and the sender index as big-endian `u64`, the X25519 key of the join |
| the key that wraps | HKDF-SHA256 over the X25519 secret of the two join keys, under `ambre-call-wrap-v1`, the call, the two sender indexes and the two join keys |
| a wrap | a 24-byte nonce, then the generation as a big-endian `u64`, the 32-byte key, the count of departures in one byte and 16 slots of a big-endian `u64` sender index, then the place of the membership log its sender walked as a big-endian `u64`, under XChaCha20-Poly1305 with `ambre-call-wrap-v1` as additional data: 217 bytes. The slots hold the 16 newest departures, then zeros |

**A join, in steps.** The Sync worker holds the identity; the call worker receives symmetric keys
alone (`CALL-004`).

1. The joiner draws an X25519 key for this call, and signs the call, the channel, the time and the
   key with its Ed25519 identity.
2. Each participant checks the joiner against the membership log of the channel (`CALL-019`) and
   refuses a signed key it has seen (`CALL-020`). Only an admitted join marks its key, under the
   identity that signed it (`CALL-133`).
3. Each participant moves its sender key one generation up by HKDF (`CALL-017`).
4. Each participant wraps that key to the joiner alone (`CALL-016`). The wrap binds the call, the
   sender, the recipient and the generation.

**A leave, in steps.** A leave or a signed removal starts the same switch (`CALL-028`).

1. Each remaining participant departs the session that left. A removal departs every session
   of the removed member at once, before any fresh key (`CALL-158`).
2. Each remaining participant draws one fresh sender key, which names every session departed
   so far.
3. It wraps the key to each other remaining participant, never to the one who left (`CALL-030`).
4. It seals with the fresh key once the switch delay of the budget row has passed (`CALL-027`).
5. On a removal, it closes each key of another participant drawn before the removal, at the end
   of the same delay (`CALL-157`).

The fresh key takes the next epoch above the current key and any key still waiting. A second leave
inside the delay draws another key and keeps the time of the first, so each leave switches in time.
A join while a key waits ratchets both keys, and wraps both to the joiner.

**Who admits whom.** The membership log of the channel decides whom a client wraps a key for. The
relay keeps the roster of a call and forwards the wraps; it orders nothing and stores nothing.

**Signalling, in steps.** Two sockets carry it, and neither is the socket of the document plane
(`SRV-049`).

1. The client opens the call socket of the relay, `/api/calls/{call}/socket`. Its first frame
   holds the access token, the channel of the call, and whether the join starts the call. Only a
   start creates the state of a call: a join finds it, or is refused as ended (`CALL-068`).
   A start in a channel whose live call runs joins that call (`CALL-160`).
2. The relay checks the membership of the channel. Under the cap, it adds the session to the
   roster, or it sends a notice and closes (`CALL-021`).
3. The relay names the call of the seat. It answers an admission, the address of the SFU, a
   sender index and the signed join of each participant already in the call. No earlier join of
   the call held that index (`CALL-056`).
4. The admission names the room and the participant by two pseudonyms of 128 random bits. The
   relay draws the room once per call, and the participant once per join (`ZK-023`).
5. The client opens the socket of the SFU and sends the admission as the first frame. The SFU
   verifies it with the relay's public key alone.
6. The SFU answers the TURN servers, with a credential under the participant's pseudonym, then
   reads the offer. A renewal near the expiry of that credential brings a fresh one
   (`CALL-154`).
7. The client signs its join under that index and announces it. The relay hands that signed join,
   unchanged, to every other participant (`CALL-057`). It forwards each opaque wrap to the
   session of its sender index alone (`CALL-058`), and announces each leave. A closed call
   socket is the leave, and the last leave ends the call.
8. A client that leaves sends a leave frame first. The relay answers whether the seat was the
   last, then closes (`CALL-069`). The last leave writes the end of its call message
   (`MURMURE-591`).

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

Each heartbeat also pings the socket. A participant whose network died closes nothing. So a
socket that answered nothing for one time to live, a pong included, closes, and its seat leaves
(`CALL-073`). Without the ping, the seat of a silent peer would hold its call open for as long
as the relay sends it nothing.

A socket whose call state the store lost gets a notice, then closes (`SRV-249`). A rejoin is
no start, so the relay refuses it as ended (`CALL-068`). So a call whose state is lost ends, and
no two senders share an index under one call. Whether a call ended is no fact the relay keeps:
the call message of the channel holds it.

**The roster.** `GET /api/call-rosters/{call}?channel={channel}` answers a member of the channel
with the signed join of each seat, or with no state (`CALL-070`). An asker who is no member, and
a call of another channel, read the same answer as a call the relay holds none of (`CALL-071`).
The route reads the state of the call and writes nothing (`CALL-072`, `ZK-024`). The reader
checks each join against the membership log of the channel, as a participant does
(`MURMURE-596`).

**The client of a call** runs in the Sync worker. It reads the membership log of the channel,
then opens the call socket. The log counts only once it stands on a root of the client, as an
open of the `meta` checks it. That root is the anchor, the pinned inviter or a root of the
session, and a wrapper the log verifies (`CALL-134`). A first log with no root ends the join.

A change of the members of the channel reads the log again. The
client then switches its key away from a member the log no longer holds (`CALL-060`).

The relay may push no frame of a removal. So two more events read the log again. The first is a
revoke this client signs on the channel (`CALL-136`). The second is a signed head of the channel
past the held log, whose entry a member that log allows signed (`CALL-137`). A log that does not
pass through the head of the held log is an older one, and the client keeps the held log
(`CALL-138`).

It wraps no
key to a join signed under another index than its seat (`CALL-061`). A join that arrives after
its own must be no older than its own, within the join clock skew (`CALL-124`). The roster
names joins older by right, so the client checks no time there. It opens the socket of the
SFU that the admission names, with the admission first (`CALL-062`). It hands each key it
draws, ratchets or unwraps to the call worker as a handoff, on a port of their own.

**A join, from the page.** A command of the bus joins a call, so no gesture is needed to test
one (`CALL-065`). The UI thread runs these steps:

1. It opens the microphone. A refusal, or no capture device, ends the steps here: no worker, no
   command and no seat (`CALL-076`).
2. It spawns the call worker, which tells it once it listens.
3. It creates one message channel, and gives one end to the call worker. The other end goes to
   the Sync worker in the transfer list of the join command.
4. The Sync worker asks the Logic thread for the `meta` document of the channel, then joins as
   above. The Logic thread names the `meta` of the channel state it opened, whose sealed header
   names the channel. A channel still opening answers once its open ends. A channel it never
   opened has none, and the join ends (`CALL-149`). Each handoff crosses the channel as 49
   bytes in a moved buffer (`CALL-064`). The call worker copies them into wiped memory, then
   zeroes the buffer.
5. Once the SFU answers its TURN servers, the UI thread opens one peer connection to the SFU,
   with no data channel. It carries an audio sender and a video sender (`CALL-008`, `CALL-010`).
6. Each sender takes the seal transform of the call worker, then its track (`CALL-011`). Each
   receiver takes the open transform of its kind as it arrives. A receiver whose transform does
   not attach plays nothing, and the page stops its track (`CALL-146`, audit `N40`). A detached
   audio element plays an audio slot.
7. The session descriptions cross the worker hop, and the Sync worker carries them on the socket
   of the SFU. The UI thread reads each one on arrival, and none reaches the Logic thread.

A leave closes both sockets. The Sync worker hands the call worker the end of the call, which
drops every key (`CALL-029`). The call worker tells the UI thread, which ends the instance. A
chunk the call worker cannot seal or open is dropped, never passed on in the clear (`CALL-001`,
`CALL-022`).

The Main instance moves the two ends of the channel and reads nothing on it. A key does cross
the JavaScript heap of each worker as the bytes of the moved buffer. Neither engine wipes the
memory it moved, as no engine wipes a buffer it frees.

**The camera.** Every call carries one video sender, with three VP8 encodings (`CALL-096`):

* The camera opens at the top size of the call video row of `ambre-spec-budgets.md section 1`.
* The three encodings scale it down by 4, 2 and 1, each capped at the bitrate of its size in that
  row.
* The sender exists with no camera too. So a camera that comes later takes the track by a replace,
  and the session is not renegotiated.
* A join that asks for the camera opens it after the microphone. A camera that does not open
  leaves the call on audio alone.

The transform of the video sender sees the chunks of the three encodings. They share one key
identifier, and its sealer issues one counter sequence across them (`CALL-005`). A receiver gets
one encoding at a time, so its replay window sees the counters of one encoding, with gaps.

The SFU forwards each video slot at the layer its choice sets (`CALL-041`). While one member alone
speaks, each other member decodes that member at the top size, and every other member at the
lowest (`CALL-099`).

**A camera during a call.** The camera button of the header opens the camera, then the video
sender takes its track by a replace. A second press replaces the track with none, then stops the
camera. Neither press renegotiates the session or touches the transform of the sender
(`CALL-100`). A camera that does not open leaves the call on its audio, and the header says why
(`5-Murmure/features/ambre-spec-murmure-calls.md`).

**The screen.** The first share of a call adds a screen sender beside the camera, with one VP8
encoding (`CALL-115`):

* The page adds the screen transceiver, then offers once, and the SFU answers (`CALL-112`). The
  camera goes on across that offer (`CALL-113`).
* An offer of the page that crosses an offer of the SFU gets no answer (`CALL-114`). The page
  rolls it back, answers the SFU, then offers again.
* From then on, a share takes the track by a replace, and a stop gives it up, with the encoding
  inactive between two shares (`CALL-103`).
* The share button of the header asks the browser for a screen or a window, from the gesture of
  its press. The browser shows its own picker.
* The capture is capped at the size and the frame rate of the call screen row of
  `ambre-spec-budgets.md section 1`. The encoding is capped at its bitrate and its frame rate.
* The track carries the content hint `detail`. So under a low bitrate the encoder drops pictures and
  keeps the size of the text.
* The sender seals each chunk as a camera chunk, under the key identifier of the screen track
  (`CALL-102`). That key identifier gives the screen a chunk key and a counter sequence of its own.
* A receiver opens the screen slot through the open transform of the screen alone. A camera chunk
  on that slot does not open, and a screen chunk on a video slot does not open (`CALL-110`).
* A capture that the browser refuses, or a picker the member cancels, leaves the call as it was
  (`CALL-109`). The header says why (`5-Murmure/features/ambre-spec-murmure-calls.md`).
* The browser ends the track when the member stops the share in the browser bar, or the shared
  window closes. The page then stops the share as a second press does (`CALL-108`).

No sender of the screen exists before the first share. On 2026-11-13, the belt of three members read
the effect of an idle second send-only video section. With it, Firefox 160 kept its camera on the
lowest layer for 30 s in 16 runs of 20. That layer ran capped at 100 kbit/s. Without that section,
no run of 11 failed.

Firefox exposes no estimate of its send bandwidth. The SFU sends that sender no REMB, and its
transport-wide TWCC acknowledges packets alone, so an idle section changes none of it. The cameras
of the SFU take the estimate less the screen only while a screen goes to that receiver.

The screen gets no simulcast. Text needs the full size of the capture, and a smaller size makes it
unreadable. Under the hint `detail`, the encoder spends the bitrate on sharpness first, and on frame
rate only after. So a still page stays sharp, and a scroll reaches the frame rate cap. The bitrate
cap sets what the SFU forwards, whatever the frame rate.

**One share at a time.** The SFU forwards one screen per room, the screen of the share that started
last (`CALL-106`), as its page marks it. Its sharer gets no screen (`CALL-111`). So a member who
starts a share takes the screen of the room from the member who shared before.

1. The SFU reads the start of the new share, and moves the screen of the room to it.
2. It writes `"unshared"` to the former sharer alone, on the socket of the SFU (`CALL-107`).
3. The page of the former sharer stops its own share, and its header says that another member
   shares now.

The SFU alone knows the order of the starts, so it tells the former sharer. The screen slot of
that page cannot tell it. On 2026-11-13, Firefox 160 read the screen of another member with no
unmute of the track. A chunk of a new sharer cannot tell it either. A sharer seen before, who
shares again, reads as no new sharer. A chunk in flight when the page starts its own share would
stop that share.

**The start of a share.** The page marks it. Once the screen sender takes a track, the page
posts one command with no field to the Sync worker. The Sync worker writes `"share"` on the socket
of the SFU, and on no socket of the relay. The SFU takes that instant as the start of the share of
that participant (`CALL-106`). A start that comes before the screen section holds until it opens.

The packets cannot mark a start. A still screen sends none for seconds, and a pause would read as
a new start. Measured on 2026-11-13 under Chrome for Testing 157: a still track with the hint
`detail` encoded one picture, then sent no packet for 12 s. Firefox 160 encoded the still screen
of `xvfb-run` at 5 fps, with no pause. A start read from a pause would let a still screen take
the screen back from a later share once it changes (`ambre_sfu::forward::tests`).

**Video on the canvas.** The render runtime draws each received video, and no page element shows
one (`CALL-015`):

1. Each video receiver plays into a detached video element of its own, muted. No element enters
   the page.
2. The page hands each element to the render runtime under the slot of its receiver. The call
   worker names the sender of the slot at its first chunk, as for an audio slot (`CALL-084`).
3. A slot whose element holds a picture, and whose track carries media, counts as live.
4. Each paint of a region that shows the videos copies the picture of each live element into a
   GPU texture of its slot.
5. The paint then draws each texture as a textured quad.
6. The quad keeps the aspect of the picture inside the box the view gives it.

The copy runs inside the GPU device, so no picture crosses WebAssembly memory. The texture stays on
the UI thread (`CANVAS-001`). A video is screen state, so no primitive of the drawing surface
takes one, and a page export draws none (`CANVAS-056`). The region draws its videos on its overlay,
which repaints at each frame while it shows a video, and the list keeps its paint. The time of such
a paint is the call video paint row of `ambre-spec-budgets.md section 1`.

**Mute.** The UI thread disables the track of the microphone, and keeps it on its sender. The
browser then feeds the encoder silence. So the sender goes on with one chunk per frame, of the
fixed size (`CALL-007`), and each receiver decodes silence (`CALL-074`). An unmute enables the
track again. No frame of either socket carries a mute, and the Sync worker hears of none.

The SFU reads a mute as a silence of that sender. It reads the same chunk rate and the same
padded size (`CALL-075`), and the audio level of silence in the clear extension. The SFU
already ranks speakers by that level (`CALL-047`), so a mute adds nothing to `ZK-021` (`ZK-025`).
The SFU cannot tell a member who mutes from a member who stays quiet.

**The padding of the congestion control.** The browser also sends packets with no payload. Its
pacer sends this padding to probe the uplink, below the track and below the transform. So the
call worker never sees it, and no chunk rides it. The SFU reads the time and the size of such a
packet, as of any packet (`ZK-021`). A padding packet is transport metadata, and it follows the
probes of the browser, never the microphone (`ZK-025`).

Measured on 2026-11-14, by `getStats()` every 20 ms and a capture of the loopback during the mute
test of the browser tier:

* Firefox 160.0 sends a burst of two to four padding packets on the audio stream every 5 s. It
  sends them muted or not, and the member who never mutes sends the same bursts.
* Chrome for Testing 157 sends none on the audio stream. Its probes ride the video stream, at the
  start of the call.

A test that counts packets reads a burst inside a mute window as a change of rate. So the browser
tier counts chunks: the payload bytes over the fixed size (`CALL-075`).

**The playout.** The SFU forwards three audio slots to each receiver (`CALL-045`), and a slot
moves to another sender at the next packet. So a participant has no track of its own on the
receiver: a slot carries it while it is among the three most recent speakers. Only a packet
whose chunk header names the sender index of its seat counts as speech (`CALL-123`).

Each slot plays through a detached audio element of its own (`CALL-077`). The element plays the stream of the
slot, so the browser cancels its echo. The element has a volume and an output of its own, and
no other path offers both on Chrome and Firefox.

**The volumes.** A member sets a volume for each other participant, from silence to the full
level of an element. The page keeps it by member and applies it to the element of the slot that
carries that member. An element plays at most the level of the system, so a volume scales the
system volume and never replaces it. The volume keys of a phone set the system volume, and no page
receives them. So the call shows no volume for the whole call.

A slot carries one sender, then another (`CALL-047`). The page tells the call worker the slot of
each receive transform it attaches. The call worker reads the sender index of each chunk it opens
from its key identifier. At the first chunk of a new sender in a slot, it tells the page the slot
and the sender (`CALL-084`).

The Sync worker admits each sender index once its signed join passed
the membership log (`CALL-019`). It then tells the page the member of that index. The page then
sets the element of the slot to the volume of that member (`CALL-087`).

The member of a sender index is no news to the Main instance. The Logic thread already shows who
is in the call, and the relay already holds each index with its signed join (`ZK-022`). The slot
and the index stay on the client. The page posts no volume and no slot to a worker that writes a
socket (`ZK-027`, `CALL-095`).

**The change of sender of a slot.** The call worker opens a chunk before the jitter buffer of the
browser reads it. So the note reaches the page while the buffer still holds the last frames of the
former sender. The slot takes the volume of the new sender at once, and those frames play at it
(`CALL-088`). The reasons, in order:

1. The new sender speaks: the SFU gave it the slot because it spoke last (`CALL-047`). A hold of
   the former volume would cut its first words, or play them at the volume of another member.
2. The former sender is the least recent speaker of the slots. Its last frames are mostly the
   silence after its speech.
3. The volume of the new sender is in place before its own first frame plays. The note crosses one
   message between two threads, and the jitter buffer holds at least one frame.

So a member turned down to nothing leaks only when four or more members speak at one instant. The
leak lasts the depth of one jitter buffer at most.

**Where a volume cannot be set.** Safari on iOS keeps the volume of an element read-only. A Web
Audio graph would carry a gain, but WebKit cancels no echo of audio played outside a stream, as
section 5 states. So the page reads whether the volume of an element takes a value, and offers no
volume for a participant where it does not. That test reads the behaviour, never the name of the
browser.

**The gain.** A member sets the gain of its microphone by hand, from silence to the cap of
`ambre-spec-budgets.md section 1`. The automatic gain of the browser would undo it, so a gain set by
hand opens the microphone again with the automatic gain off (`CALL-089`). The echo canceller and
the noise filter stay on (`CALL-091`).

No capture constraint sets a gain, so a Web Audio graph carries it on the send side alone:

1. The capture track feeds a source node, then a gain node.
2. The gain node feeds a stream destination node, whose track the sender takes, by the same
   replace as a change of microphone.
3. The sender keeps its transform, and no session description crosses the hop (`CALL-092`).

The browser cancels the echo inside the capture, before any page reads the track. So the gain
scales a sound already free of echo, and the playout keeps its elements (`CALL-077`). A mute
disables the capture track, so the graph sends silence (`CALL-093`). A change of microphone feeds
the new capture track to the same graph (`CALL-094`). A return to the automatic gain opens the
microphone with the automatic gain on, and the sender takes the capture track again.

The sender encodes at a constant rate, so a gain changes the size and the rate of no chunk
(`CALL-092`). The SFU reads a louder or a quieter voice, as from a member who speaks so.

**The meter.** The panel shows the level the microphone sends, read at the interval of
`ambre-spec-budgets.md section 1`. An analyser node reads the capture, after the gain when a gain is
set by hand. It reads and plays nothing, so it touches no playout path.

**Devices.** The page opens the microphone the member chose, or the default one. A change during a
call opens the new microphone first. The new track takes the mute state of the call before it
replaces the old one, so a muted member stays muted (`CALL-080`). The sender then replaces its
track in one step, and the old track stops after.

The encoder runs through the change, so the sender sends a chunk of every frame (`CALL-078`). The
sender keeps its transform, the call worker keeps its keys, and no session description crosses the
hop (`CALL-079`). The page writes no command to a worker for a change of device.

A chosen speaker routes each audio element of the call to that output, the elements of the slots
that arrive later included (`CALL-082`).

A device can go away during a call: a headset unplugged,
a Bluetooth link lost. The track of a lost microphone ends, and the browser fires a change of the
device list. On either, a microphone the browser no longer lists gives way to the default one, by
the same replace (`CALL-081`). A lost speaker gives way to the default output (`CALL-083`). The
header then says which device the call uses (`5-Murmure/features/ambre-spec-murmure-calls.md`).

The page reads the device list from the browser and keeps the choice on the device. No device
identifier and no label leaves the page (`ZK-026`).

What each engine offers, read on 2026-11-11:

| Engine | Microphone | Speaker | Volume of an element |
|---|---|---|---|
| Chrome | each input, by its identifier | each output, through `setSinkId` | writable |
| Firefox | each input, by its identifier | each output, through `setSinkId`, listed once the microphone is granted | writable |
| Safari on macOS | each input, by its identifier | `setSinkId` exists; no run reads its routing of a call | writable |
| Safari on iOS | each input the system lists | none: the system picks the output | read-only |

The speaker choice shows where the element has `setSinkId` and the browser lists an output by its
identifier. So a browser that cannot route shows no control.

**The admission token.** Its text is hex: the encoded admission, then an Ed25519 signature.

| Part | Holds |
|---|---|
| claims | the room pseudonym, the participant pseudonym, the sender index of the seat, the expiry |
| expiry | 60 s after the relay issues it, in Unix seconds |
| audience | the SFU: the signature covers the domain `ambre-call-admission-v2` first |
| key | `CALL_ADMISSION_SEED` on the relay, its public key in `SFU_ADMISSION_KEY` on the SFU |

The admission key serves one purpose. It is never the access-token key, so either one rotates
alone, and a leak of one forges nothing under the other. The SFU then holds no key that verifies
a session.

**The TURN credential.** The SFU mints it under the REST API of coturn. The username is the
expiry, then the pseudonym. The password is the HMAC-SHA1 of the username under a secret. The
SFU and coturn alone share that secret (`CALL-014`). No crate depends on the SFU, so the relay
cannot name it.

**The SFU seats.** One admission seats one connection, and the SFU holds the cap too. A token
whose connection left seats nobody again before it expires (`CALL-117`).

One socket holds a bounded share of the SFU (audit `N39`):

* A message, or a frame, longer than twice the cap of a session description closes the socket
  (`CALL-150`). That bound holds an offer at its cap, each byte of it escaped once in JSON.
* A socket waits for its admission, then for its offer, ten seconds each. Past either, it closes
  (`CALL-151`).
* A socket spends its admission before the SFU mints its TURN credential. A second socket
  under the same admission reads nothing, so one admission mints one credential (`CALL-152`).

**The lifetime of a TURN credential** is the TURN row of `ambre-spec-budgets.md section 1`
(audit `P60`). coturn reads the expiry when an allocation first authenticates. It then keeps the
key of the allocation for each refresh, so an allocation outlives its credential. Read on
2026-11-20 in the coturn source of that day, `ns_turn_server.c`, `check_stun_auth`.

So the lifetime bounds when a credential opens a new allocation, never how long one lasts:

* A member removed from the call opens no allocation once its last credential expired.
* A page that opens an allocation later, at an ICE restart, needs a credential still valid.

A renewal of the seat that the SFU keeps brings a fresh credential, once the last one expires
within the margin (`CALL-154`). The margin holds three heartbeats, so one late beat still
brings it in time. The page sets it on its peer connection and restarts ICE under it
(`CALL-156`). The fresh allocation carries the call past the first expiry (`CALL-155`).

An offer that restarts ICE gives the connection on the SFU a fresh fragment. The SFU indexes
the connection under it, and the old fragment names nothing (`CALL-155`).

**The routes of the SFU.** One task reads every datagram, so a datagram costs that task
little (audit `N41`):

* A known source names its participant in one lookup.
* A new source reaches a connection by an ICE check alone. The check names the username
  fragment of the connection, and the SFU finds that connection in one lookup. The check
  then passes its integrity there, or reaches nothing (`CALL-144`).
* Any other datagram of a new source reaches no connection.
* One participant holds eight sources at most: a host, a reflexive and a relayed candidate on
  each address family. A ninth forgets the oldest (`CALL-145`).

A seat lives while the relay renews its admission (audit `N25`). A heartbeat renews the
admission of a seat whose member is in the channel. The client hands each renewal to the SFU
on its socket (`CALL-122`). The SFU ends a seat whose last admission expired (`CALL-119`).
So a member who closes the call socket, or leaves the channel, loses its SFU seat within one
admission lifetime. A renewal keeps the seat it names alone (`CALL-120`).

The admission also names the member by a pseudonym of the call. The relay derives it from the
room and the member's key under the admission key, so the SFU cannot derive it from a key. A
join of a member ends its older seat in the room (`CALL-121`). So a member who rejoins many
times holds one seat, and stale seats never fill the cap.

**The SFU binds a seat to its sender.** Every member holds the key of every sender. A key alone
then names no author: Mallory holds Bob's key and seals a chunk that opens as Bob's. The relay
signs the sender index of the seat into the admission.

The SFU reads the key identifier of each chunk as the receiver reads it, and forwards no chunk
under another index (`CALL-116`). For VP8 it reads the first packet of each frame, which holds the
header of the whole frame. The SFU already sees each index in each key identifier (`ZK-021`), so
the admission tells it nothing new.

A receiver joins the packets of a frame before it reads the header. So the SFU drops a frame start
that holds no whole header, since the next packet could complete it under another index
(`CALL-140`). The SFU offers Opus and VP8 alone, the two codecs whose header it reads (`CALL-139`).

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
* Each receiver holds three audio slots, nine video slots and one screen slot (`CALL-045`). The
  SFU offers them once, when the room first carries a track of another participant.
* The SFU reads the second video section of the offer of a participant as its screen. A screen
  travels over the screen slot alone, never over a video slot (`CALL-104`).
* The screen slot carries the share that started last, at its one encoding. The layer choice of
  the camera never lowers or drops it (`CALL-105`). The cameras share what the estimate leaves.
* A move of the screen slot to another sharer waits for a keyframe of that sharer, as a video
  slot does (`CALL-049`).
* A change of speaker moves a slot to another sender, and renegotiates nothing (`CALL-046`). The
  pacer of str0m reads every send stream at each packet, so the slots also bound its cost.
* Each receiver gets the video of the senders it chose (`CALL-126`), then of the most recent
  speakers up to the count its choice names (`CALL-041`). That is nine at most. It
  gets the audio of the three most recent speakers (`CALL-047`). The audio level of each packet,
  which the seal leaves in clear, names who speaks.
* The layer choice runs per receiver, every 100 ms, and at once when a target falls or overuse
  starts or ends. Each video it gets takes its lowest layer while
  the estimate holds it. Then the most recent speaker moves up to the highest layer the rest
  holds. A move up needs a margin, so a receiver near a boundary stays on its layer.
* A switch to another layer waits for a keyframe of that layer (`CALL-039`). The SFU shifts the
  sequence numbers, the timestamps and the VP8 picture numbers of the new layer to continue the
  old one (`CALL-040`). The VP8 payload descriptor stays in clear, so the SFU reads it.
* A move of a video slot to another sender waits for a keyframe of that sender (`CALL-049`). An
  audio slot moves at the next packet. The SFU shifts the fields the same way (`CALL-048`).
* The receiver reads the sender of each chunk from its key identifier. So no signal names the
  sender of a slot.
* A camera layer that sends no packet for 1 s leaves the layer choice (`CALL-162`). A browser
  short of CPU stops its top layer, and a receiver on it would see a frozen picture. The layer
  comes back with its next packet, measured again from nothing. The screen keeps its one
  layer, since a still screen sends nothing for seconds.
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

**The choice of videos.** A receiver names the senders whose video it wants, by the sender index
of each, on the socket of the SFU (`CALL-132`). The page posts the choice to the Sync worker, which
writes `{"videos": {"senders": [..], "large": .., "count": ..}}` there, and on no socket of the
relay. A choice replaces the one before it.

1. The choice names the count of videos its receiver lays out, nine at most, and as many
   senders at most, each once. Its large sender is one of them. The SFU reads nothing of another
   choice (`CALL-131`). A choice with no count asks for nine.
2. The SFU serves the chosen senders that send a camera first. It then fills the slots left with
   the most recent speakers, up to the count (`CALL-126`, `CALL-041`). So the fill sends no
   video its receiver has no tile for. Under a tight estimate, a filler loses its video
   before a chosen sender does.
3. The large sender takes the top layer the estimate holds, in place of the most recent speaker
   (`CALL-128`). With no large sender, the most recent speaker served takes it. The estimate
   still sets the layer (`CALL-038`).
4. A chosen sender who leaves is in no track any more. Its slot falls to the next speaker at the
   next choice (`CALL-127`).
5. A slot that moves to a chosen sender asks it for a keyframe, as a change of speaker does
   (`CALL-129`). The SFU asks one layer of a sender once until a keyframe of it passes, or for
   300 ms at most. The count of receivers that wait for the layer does not matter: one keyframe
   serves them all (`CALL-130`).

The audio does not follow the choice. The three audio slots carry the most recent speakers alone
(`CALL-047`). So the SFU learns which senders each receiver watches (`ZK-044`).

**The metrics of the SFU** take a listener of their own, on the address of `SFU_METRICS_ADDR`, the
loopback by default (`CALL-101`). Neither the media socket nor the signalling serves them. An
address off the loopback needs the token of `SFU_METRICS_TOKEN`, or the SFU refuses to start
(`CALL-153`). The format and the closed labels are the relay's (`ambre-spec-server.md section 3`):

| Metric | Kind | Label |
|---|---|---|
| `ambre_sfu_participants`, `ambre_sfu_rooms` | gauge | none |
| `ambre_sfu_datagrams_total` | counter | `direction`: in, out, or dropped on a full send buffer |
| `ambre_sfu_admissions_total` | counter | `outcome`: seated, refused, or full |

**What the design does not protect.** These limits are accepted, and stated here so that no reader
assumes them away.

| Limit | Why it stays |
|---|---|
| Mallory, working with the SFU, can forge the chunks of another member | a signature per chunk costs more than the budget of the call worker allows. An honest SFU refuses the forgery alone (`CALL-116`) |
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
| `src/metrics.rs` | the metrics, their closed labels and their listener |
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
| `ambre-metrics` | the metrics under closed labels, shared with the relay |
| `ed25519-dalek`, `hex` | the verification of the relay's signature over an admission, in pure Rust |
| `hmac`, `sha1`, `base64` | the TURN credential, as coturn checks it; RustCrypto, already in the lockfile |

**The chunk seal and the sender keys** run in the call worker and in the Sync worker:

| File | Holds |
|---|---|
| `crates/ambre-core/src/call.rs` | the track, the sender index, the generation, the counter, the key identifier, the two pseudonyms of the SFU and the size of a sealed audio chunk |
| `crates/ambre-crypto/src/call.rs` | the chunk format, the sender key, the receiver, the signed join and the wrap |
| `crates/ambre-sync/src/call.rs` | the keys of one participant: the admission, the wraps and the switch |
| `crates/ambre-sync/src/call_client.rs` | the client of a call: each frame of the two sockets, and the steps it answers |
| `crates/ambre-sync/src/call_link.rs` | the transport of a call in the Sync worker: the two sockets, the log, the switch timer and the port |
| `crates/ambre-call/src/lib.rs` | the call worker: the sealer of the session, the keys of the others, the seal, the open and the erase |
| `crates/ambre-call/src/worker.rs` | its entry in the browser: the port of the Sync worker and the frames of each transform |
| `crates/ambre-app/src/call.rs` | the client of the UI thread: the microphone, the camera, the peer connection, the transforms and the two ends of the port |

**The signalling** of a call: `crates/ambre-protocol/src/signaling.rs` holds the frames of the
call socket, the admission and the messages of the socket of the SFU.
`crates/ambre-server/src/ws/handler/call.rs` serves the call socket,
`crates/ambre-server/src/routes/calls.rs` the roster, and
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
| mute, then unmute | the sender sends silence, then the microphone again, at one rate and one size |
| a refused microphone | no start and no join leave the client |
| another microphone, during a call | the sender sends the new microphone with no gap, under the same keys |
| another speaker | every audio slot of the call plays on it |
| a device that goes away | the call goes on with the default device |
| a volume for a participant | the element of each slot that carries it plays at that volume |
| a slot that changes sender | the element takes the volume of the new sender |
| a gain set by hand | the automatic gain turns off, and the sender sends the sound at that gain, at one rate and one size |
| the camera button, during a call | the video sender takes the camera, or gives it up, under the same session and transform |
| a received video | the render runtime draws it on the canvas at each paint |
| the share button, during a call | the screen sender takes a screen or a window the member picks, or gives it up |
| a refused or a cancelled capture of the screen | the call goes on with no share |
| the browser ends the screen track | the share stops |
| a share of another participant reaches a sharer | its own share stops |


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
| A count of videos in the choice | a fixed fill of nine for every receiver | the desktop count and the phone count are two figures of the budgets, and a desktop count that grows must leave a phone where it is with no change of protocol |
| str0m drives the SFU | the `rtc` crate of webrtc-rs | ring or aws-lc-rs seal its DTLS records, not only its certificate |
| str0m drives the SFU | LiveKit | a Go server with a frame format of its own |
| Every call goes through the SFU | a direct connection between two clients | two topologies to test, and each peer would see the address of the other |
| The texture copy from a video element | a copy of each picture through WebAssembly memory | one readback per picture. Page elements over the canvas are the fallback, and no overlay could cover them |
| The fields of the chunk header have a fixed length | the shortest form of each field, as RFC 9605 writes it | the header would grow at the counter 256, and the size of an audio chunk would vary (`CALL-007`) |
| A second leave inside the delay keeps the time of the first | a second leave starts the delay again | the member who left first would open chunks past the delay of its own leave (`CALL-027`) |
| A mute disables the track | the sender drops its track, through `replaceTrack(null)` | the sender stops sending, so the SFU reads a mute apart from a silence (`ZK-025`) |
| A mute disables the track | the UI thread stops the track of the microphone | the same gap in the packets, and an unmute asks the browser for the microphone again |
| A mute disables the track | the call worker seals an empty frame in place of each chunk | the call worker would hold a state of the call that the browser already holds |
| A mute disables the track | the sender takes the output of a Web Audio graph on every call, and a mute sets its gain to zero | the packets with no payload that a window of a mute showed under Firefox are padding of the pacer, below the track, sent muted or not (section 3). The graph hides none of them. On 2026-11-14, one hop of the graph added 23 ms to 32 ms and about 2 points of one core under Chrome for Testing 157, and nothing measurable under Firefox 160.0 |
| An audio element per slot plays the call | a Web Audio graph with a gain per slot | Chromium cancels the echo of what a media element plays, and of no Web Audio output unless a flag of the build turns it on (Chromium bug 933677). WebKit cancels no echo of audio played outside a stream, as the first row states |
| An audio element per slot plays the call | an audio element per participant | the SFU forwards slots, not participants: a participant has no track of its own on a receiver (`CALL-045`) |
| The slot takes the volume of its new sender at once | the slot keeps the lower of the two volumes until the jitter buffer drains | the new sender speaks at the change, so its first words would play at the volume of another member |
| The slot takes the volume of its new sender at once | the page waits until the slot plays the first frame of the new sender, read from the playout timestamps of the receiver | the page would poll each receiver, and the former sender is mostly silent by then |
| A volume follows its member from slot to slot | a volume per slot | a slot changes sender with who speaks, so a volume per slot would move from one member to another |
| A volume scales the system volume | a volume of the whole call | the volume keys of a phone set the system volume and never reach a page, so two controls would fight |
| A Web Audio graph carries a gain set by hand | the graph on every call | a context created with no gesture starts suspended, so the sender would send nothing until a gesture |
| A Web Audio graph carries a gain set by hand | a gain constraint of the capture | no engine offers one |
| A change of microphone replaces the track of the sender | a new transceiver for the new microphone | a renegotiation through the SFU, a new transform, and a gap in the chunks until the answer |
| A change of microphone opens the new microphone first | the page stops the old track, then opens the new one | the sender sends nothing between the two, so a receiver hears a gap, and the SFU reads a change of microphone apart from a silence |
| A change of microphone keeps the old track on the sender until the new one replaces it | the sender drops its track, through `replaceTrack(null)`, while the new microphone opens | the sender sends nothing until the open ends. On 2026-11-11, a microphone that took 500 ms to open left the receiver to conceal 0.74 s of audio in the belt under Chrome |
| A video chunk is not padded | each video chunk padded to a size class | a keyframe weighs several still pictures, so a class large enough to hide motion multiplies the downlink. A smaller class still shows motion against stillness |
| VP8 for every sender | H.264 | the SFU parses the VP8 descriptor alone, to switch a layer and to rewrite a picture number (`CALL-039`, `CALL-040`). H.264 takes another clear prefix rule and another rewrite. It stays the fallback of `Call client proofs on iOS` |
| One video sender in every call | a video sender added when the camera opens | a renegotiation through the SFU, as for a new transceiver of a microphone |
| The screen is one VP8 encoding at the full size of the capture | the three sizes of the camera | text at a quarter or half of its size is unreadable, and two encodings nobody reads cost the uplink of the sharer |
| The frame rate of the screen is capped at 30 | a cap of 5 frames per second | a scroll or a demonstration stutters at 5. Under the hint `detail`, the frame rate takes only the bitrate that sharpness leaves, so a cap of 30 blurs no text, and the bitrate cap holds the egress of the SFU |
| The first share adds the screen sender | a screen sender in every call from its start, with no renegotiation | an idle second video section kept the camera of Firefox on its lowest layer, as section 3 measures |
| The screen has a sender of its own | the camera sender takes the screen by a replace | a member could not send its camera and its screen at once, and the layer choice of the camera would lower the screen |
| One screen per room | a screen slot per sharer | each receiver would draw several large screens, and each slot adds a send stream the pacer reads at each packet (`CALL-045`) |
| The page marks the start of a share | the first packet after a pause of the screen track | a still screen sends no packet for seconds under Chrome, so a still screen that changes again would take the screen back from a later share, as section 3 measures |
| The page marks the start of a share | a keyframe request or a lowest frame rate that keeps every screen sending | a full picture of a still screen every second costs the uplink of each sharer and the downlink of each receiver, and a lost request still reads as a pause |
| The page marks the start of a share | a start read from a change of track: a new SSRC or a jump of the VP8 picture number | a replace of the track keeps the SSRC, and the encoder of each engine numbers its pictures in its own way across a replace |
| A new share takes the screen of the room | the SFU refuses a second share | a refusal needs a signal to the second sharer, and the SFU would learn and tell more of a share |
| The SFU tells the former sharer that a later share took the screen | the page reads the take-over from an unmute of its screen slot | Firefox 160 read the screen of another member with no unmute, so a share went on unseen |
| The SFU tells the former sharer that a later share took the screen | the call worker tells the page of the first chunk of a new sharer on the screen slot | a sharer seen before reads as no new sharer, and a chunk in flight at the start of a share would stop it |
| coturn serves TURN | turn-rs | aws-lc-rs seals its TLS records, and one person maintains it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Firefox encodes Opus at a variable bitrate.** It ignores `cbr`, and holds the average the SFU
  asks. Its frames read 152 bytes at most on 2026-11-10, under the 160 a chunk holds, from a fake
  microphone. Speech or music with sharper transients may peak higher, and no run measures it.
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
* **The time of a signed join in the roster is not checked.** A client refuses a join it has
  seen, and a later join older than its own (`CALL-124`). A join of an earlier session that the
  relay puts in the roster still gets a wrap while the log holds that member.
* **No rule caps the calls of one channel.** Whether a channel holds one live call or several is
  not ruled.
* **Safari on iOS offers no volume for a participant.** Its element volume is read-only, and no
  path that keeps the echo canceller carries one there. No run on a device reads it.
* **No belt records the sound an element plays.** A headless browser plays to no device it can
  record. Chrome measures the energy of a slot after the volume of its element, and Firefox before
  it. The belt reads both with the volume of each element, and the browser applies the volume.
* **No run measures the tail of a former sender.** The note crosses before the jitter buffer by
  design, and no belt times it against the playout.
* **Headless Firefox runs an audio context only over an audio server.** The meter and the gain need
  one, so its belt runs over a null sink of PulseAudio.
* **The tail of a former sender plays at the volume of the new one.** Four members who speak at
  one instant can leak a member turned down to nothing, for one jitter buffer at most.
* **WebKit may ignore `setSinkId` for a call.** WebKit bug 320087 reports it ignored for a stream
  of one WebRTC track on iOS 26.5. No belt runs Safari, so no run reads it on macOS either.
* **Headless Firefox offers one fake microphone, and an output only over an audio server.** Its
  belt changes the microphone to the same device by its identifier, which opens a new track and
  replaces it. The speaker choice runs on Chrome alone.
* **A share carries no sound.** The capture asks for no audio, so a shared video plays silent.
* **No rule binds a screen on a downlink that cannot carry it.** The SFU forwards the screen at
  any estimate, and the cameras take what remains.
* **No belt drives an offer that crosses an offer of the SFU.** The SFU offers its slots once, at
  the start of a call, so a first share rarely meets it. The SFU half holds in the host tier.
* **A start reaches the SFU apart from the media.** A start that comes before the first packet of
  its screen empties the screen slots of the room until that packet. The slot then waits for a
  keyframe of the new sharer (`CALL-049`).
* **Firefox calls no listener of `ended` on a captured track.** On 2026-11-12, Firefox 160 called
  the `onended` of a screen track and of a microphone track, and no listener added to either. So
  the page stops a share from `onended`.
* **No belt presses the bar of the browser.** The belt stops the track and fires `ended` on it, as
  the bar of the browser does, and reads the share stop.
* **Safari on iOS has no screen capture.** A press of the share button there reads the failure of
  the capture, and no run reads it on a device.
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
