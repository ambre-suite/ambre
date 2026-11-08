# AMBRE -- FEATURE: CALL MEDIA

**Type:** Horizontal (framework)
**Implementation:** partial -- the SFU forwards RTP between the participants of a room; no call exists

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
| `CALL-001` | `INV` | A chunk of a call MUST leave the client sealed under a sender key of that call. | README pillar "E2EE"; this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-002` | `INV` | A sender key MUST NOT derive from a channel key or a document key. | this spec section 3; `MURMURE-157` | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-003` | `INV` | The seal and the open of every chunk MUST run in the call worker. | this spec section 3; `ARCH-004` | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-004` | `INV` | The call worker MUST hold no key beyond the sender keys of its current call. | this spec section 3; `ZK-011` | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-005` | `INV` | One sender key MUST NOT seal two chunks under one counter. | this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-006` | `INV` | The clear prefix of a sealed chunk MUST be authenticated by its seal. | this spec section 3 | not built | unbuilt -- lands with the `Video calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-007` | `INV` | Every audio chunk of one call MUST be sealed at one fixed size. | this spec section 3; `ZK-021` | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-008` | `INV` | A client MUST carry the audio and video of a call over one peer connection to the SFU. | this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-009` | `INV` | Two clients of one call MUST NOT connect to each other. | this spec section 3; `ZK-021` | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-010` | `INV` | The peer connection of a call MUST NOT open a data channel. | this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-011` | `INV` | A client MUST attach the chunk transform to a sender before it attaches a track to it. | this spec section 3; `CALL-001` | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-012` | `INV` | The SFU MUST NOT link the crate that opens a chunk. | this spec section 3; `ZK-004` | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-013` | `INV` | The SFU MUST NOT link a C cryptography library. | README pillar "Rust-first"; this spec section 3 | ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads the graph `cargo metadata` resolves and refuses a C cryptography library on any edge it resolves from `ambre-sfu` (`ambre_primitives::boundaries::tests::a_c_cryptography_library_under_the_sfu_is_refused`) | none | verified |
| `CALL-014` | `INV` | The relay MUST NOT hold the secret that mints the credentials of the TURN server. | `ambre-spec-server.md`, `SRV-012`; this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
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

### 1.2 Join a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-016` | `POST` | A join MUST hand the joiner the current sender key of each participant, wrapped to the joiner alone. | this spec section 3, the join steps | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-017` | `POST` | A join MUST move each participant to the next generation of its sender key before the wrap. | this spec section 3, the join steps | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-018` | `FRAME` | A join MUST NOT show a sender key to the relay or to the SFU. | `ZK-002`; this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-019` | `FAIL` | Given a joiner absent from the membership log of the channel, a client MUST wrap no key for it. | this spec section 3, the join steps; `ambre-spec-membership.md` | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-020` | `FAIL` | Given a signed ephemeral key the client has seen already, the client MUST refuse it. | this spec section 3, the join steps | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-021` | `FAIL` | Given a call at the participant cap of `ambre-spec-budgets.md section 1`, a join MUST be refused with a notice. | this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Send and receive audio and video

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-022` | `POST` | A received chunk MUST reach the decoder only after it opens under the key its identifier names. | this spec section 3, the chunk steps | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-023` | `FRAME` | The seal of a chunk MUST NOT change the clear prefix the SFU reads. | this spec section 3, the chunk steps | not built | unbuilt -- lands with the `Video calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-024` | `FAIL` | Given a chunk that does not open, the receiver MUST drop it. | this spec section 3, the chunk steps | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-025` | `FAIL` | Given a chunk whose key identifier names a sender other than the owner of its track, the receiver MUST drop it. | this spec section 3, the key identifier | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-026` | `FAIL` | Given a chunk whose counter falls outside the replay window of its key, the receiver MUST drop it. | this spec section 3, the chunk steps | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Leave a call

A participant leaves, or the channel removes a member who is in the call.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CALL-027` | `POST` | A leave MUST move each remaining participant to a fresh sender key within the delay of `ambre-spec-budgets.md section 1`. | this spec section 3, the leave steps | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-028` | `POST` | A removal entry in the membership log of the channel MUST move each remaining participant to a fresh sender key. | this spec section 3, the leave steps; `SYNC-132` | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-029` | `POST` | Leaving a call MUST erase every sender key of that call from the call worker. | this spec section 3, the leave steps | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-030` | `FRAME` | A leave MUST NOT send a fresh sender key to the participant who left. | this spec section 3, the leave steps | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |

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

* **Budget.** the call rows of `ambre-spec-budgets.md section 1` and section 3. They cover the
  chunk transform, the worker load, the key switch, the cap, the latency and the SFU load.
* **Boundary.** every chunk leaves sealed under a sender key that neither the relay nor the SFU
  holds (`CALL-001`, `CALL-018`). What each of them learns of a call is bounded by `ZK-021` and
  `ZK-022`.
* **Verification.** 23 `verified`, 29 `unimplemented`. The SFU links no C cryptography,
  completes DTLS, and forwards RTP within a room (`CALL-013`, `CALL-031` to `CALL-037`). It
  chooses the layer of each receiver, and a browser on TCP 443 alone reaches it through TURN
  (`CALL-038` to `CALL-043`). Its congestion control holds under random loss and climbs back
  after a false drop (`CALL-044`, `CALL-050` to `CALL-052`). Each receiver holds fixed slots
  (`CALL-045` to `CALL-049`), and no call code exists beyond the SFU.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The call holds | Each user sees | Test |
|---|---|---|---|---|
| joins | is in the call | two participants, one sender key each | each the video of the other (`CALL-016`) | none |
| joins late | talks | Bob's key, one generation up | Alice hears Bob from her join on, and nothing before (`CALL-017`) | none |
| leaves | stays | Bob's fresh key | Bob goes on alone, and Alice opens nothing after the switch (`CALL-027`) | none |
| is removed from the channel | stays | Bob's fresh key | Alice's client opens nothing after the switch (`CALL-028`) | none |
| sends chunks under the track of Bob, as Mallory | listens | two sender keys | nothing of the forged chunks (`CALL-025`) | none |

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

**Who admits whom.** The membership log of the channel decides whom a client wraps a key for. The
relay keeps the roster of a call and forwards the wraps; it orders nothing and stores nothing.

**Signalling.** The relay route issues a call token, admits, keeps the roster and forwards the
opaque wraps. The session description goes over a socket to the SFU, which a token of the relay
admits under a pseudonymous identifier per call. The SFU mints the TURN credentials (`CALL-014`).

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
| `src/serve.rs` | the UDP socket and the signalling WebSocket |
| `src/sim/` | tests alone: the impairment relay, the synthetic participants, the load run |

**A simulated run is seeded.** The seed sets the losses of each link and the random numbers str0m
draws on its thread. The patched str0m keeps its streams in a sorted map, and the simulated SFU
runs its connections on one thread. So a seed gives one path, on any machine and under any load.

The simulated clock steps by 1 ms. The SFU wakes str0m at the timeout it asks, and tokio rounds
that timeout to 1 ms. The pacer of str0m sends one padding packet per wake. A coarser step would
starve a probe of padding that the SFU sends.

**Dependencies.** Each one the SFU adds, and why:

| Crate | Why |
|---|---|
| `str0m`, vendored | the seam of the feedback and the estimate, which str0m 0.25 lacks |
| `fastrand`, tests alone | the generator str0m draws from; a test seeds it, and adds no crate to the lockfile |

Nothing seals a chunk yet. `crates/ambre-protocol/src/signaling.rs` carries a shape
for a mesh, with plaintext session descriptions sent to a named user: it is replaced, not
extended.

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
| A probe pads up to the next layer step | a faster growth of the target near the known capacity | the target would grow on a capped downlink too, and the receiver would climb into overuse, then drop again |
| A probe pads up to the next layer step | the probe clusters of str0m | str0m starts none while the SFU sets the estimate, and its result feeds the estimate of str0m, which `CALL-044` rejects |
| A drop to a lower layer stops the current layer | the current layer flows until the lower keyframe | its packets queue in the pacer of the congested downlink. Over 40 seeds on 2026-11-08, the worst drop took 2.52 s, against 0.85 s |
| The video of the nine most recent speakers | every video of the room to every receiver | 49 videos at the lowest layer exceed the downlink of a phone, and the SFU cost grows with the count |
| str0m drives the SFU | the `rtc` crate of webrtc-rs | ring or aws-lc-rs seal its DTLS records, not only its certificate |
| str0m drives the SFU | LiveKit | a Go server with a frame format of its own |
| Every call goes through the SFU | a direct connection between two clients | two topologies to test, and each peer would see the address of the other |
| The texture copy from a video element | a copy of each picture through WebAssembly memory | one readback per picture. Page elements over the canvas are the fallback, and no overlay could cover them |
| coturn serves TURN | turn-rs | aws-lc-rs seals its TLS records, and one person maintains it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing seals a chunk.** No call worker and no relay route exist. The SFU forwards with no
  admission token, and it mints no TURN credential yet (`CALL-014`).
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
