# AMBRE -- FEATURE: CALL MEDIA

**Type:** Horizontal (framework)
**Implementation:** design-only

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
| `CALL-013` | `INV` | The SFU MUST NOT link a C cryptography library. | README pillar "Rust-first"; this spec section 3 | not built | unbuilt -- lands with the `Call SFU proofs` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-014` | `INV` | The relay MUST NOT hold the secret that mints the credentials of the TURN server. | `ambre-spec-server.md`, `SRV-012`; this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `CALL-015` | `OBS` | The render runtime MUST draw a remote video, never a page element over the canvas. | this spec section 3; `CANVAS-001` | not built | unbuilt -- lands with the `Video calls` Story (`ambre-roadmap.md` E5) | unimplemented |

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

* **Budget.** the call rows of `ambre-spec-budgets.md section 1` and section 3. They cover the
  chunk transform, the worker load, the key switch, the cap, the latency and the SFU load.
* **Boundary.** every chunk leaves sealed under a sender key that neither the relay nor the SFU
  holds (`CALL-001`, `CALL-018`). What each of them learns of a call is bounded by `ZK-021` and
  `ZK-022`.
* **Verification.** 30 `unimplemented`: no call code exists.

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

* str0m drives it, with its DTLS on RustCrypto. The SFU makes its certificate at boot, so no C
  library generates one (`CALL-013`).
* The congestion control, the layer choice and the parser of the dependency descriptor come from
  Signal-Calling-Service, under AGPL-3.0 like Ambre.
* It sends a phone one video at 720p and eight at 180p, and the few loudest audio tracks.

**What the design does not protect.** These limits are accepted, and stated here so that no reader
assumes them away.

| Limit | Why it stays |
|---|---|
| Mallory, working with the SFU, can forge the chunks of another member | a signature per chunk costs more than the budget of the call worker allows |
| a member compromised during a call stays compromised until the call ends | that device already holds the camera and the plaintext |
| the SFU sees when each member speaks, through the packet rate of silence | the codec stops sending in silence, and padding does not hide an absent packet |
| a member who leaves can open chunks until the switch completes | the switch waits for the delay of the budget row, so that late wraps arrive |

**Code.** not built. `crates/ambre-protocol/src/signaling.rs` carries a shape for a mesh, with
plaintext session descriptions sent to a named user: it is replaced, not extended.

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
| str0m drives the SFU | Signal-Calling-Service as it ships | it has no DTLS. Its congestion control is ported instead |
| str0m drives the SFU | the `rtc` crate of webrtc-rs | ring or aws-lc-rs seal its DTLS records, not only its certificate |
| str0m drives the SFU | LiveKit | a Go server with a frame format of its own |
| Every call goes through the SFU | a direct connection between two clients | two topologies to test, and each peer would see the address of the other |
| The texture copy from a video element | a copy of each picture through WebAssembly memory | one readback per picture. Page elements over the canvas are the fallback, and no overlay could cover them |
| coturn serves TURN | turn-rs | aws-lc-rs seals its TLS records, and one person maintains it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No call worker, no SFU and no route exist.
* **The receive sizes rest on no measurement.** One 720p and eight 180p videos on a phone, and the
  software VP8 decoder of an iPhone, are untested. H.264 for every sender is the fallback.
* **iOS audio may crackle with the echo canceller on.** WebKit bug 326286 reports it on iOS 26.2 to
  27, and Ambre has no fix of its own.
* **The SFU still links a C library.** str0m generates its certificate through rcgen, which pulls
  aws-lc-sys (`CALL-013`).
* **No rule caps the calls of one channel.** Whether a channel holds one live call or several is
  not ruled.
* **A network that allows only TCP on port 443** reaches the SFU through TURN alone. No rule bounds
  the latency it adds.

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
