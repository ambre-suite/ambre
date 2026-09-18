# AMBRE -- FEATURE: THE SEALED ENVELOPE

**Type:** Horizontal (framework)
**Implementation:** partial -- the three message types take one pipeline. The document name and a
media blob take a bare cipher beside it.

> Nothing a user writes leaves the browser readable. What the relay carries is a sealed frame. It
> can route it, count it and drop it. It can never open it, move it to another document, or replay
> it.

Every frame the Sync worker sends takes this path, and every frame it accepts comes back through
the inverse. This document owns the envelope. What a payload means is the document that produced
it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SYNC`, shared with
`ambre-spec-sync.md`, `ambre-spec-delta-order.md`, `ambre-spec-membership.md`,
`ambre-spec-key-rotation.md` and `ambre-spec-media.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-088` | `INV` | The per-document map of remote ephemeral counters MUST be bounded. | this spec section 2; `SYNC-008` | `ambre_sync::ephemeral_tests::author_map_is_bounded` -- a hundred authors past the cap, and the map never exceeds it, and a peer can mint signing keys at will, so without the cap one document's presence state is an unbounded allocation | none | verified |
| `SYNC-001` | `INV` | The Sync worker MUST NOT hand the socket a payload that has not passed through the sealing pipeline. | `ZK-001`; this spec section 2 | none | unheld -- the rule cannot be stated as a test at all: the wire ciphertext is a public byte vector that accepts any bytes from anywhere, so the fix is a private field with a constructor reachable only from the seal, section 4; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-018` | `INV` | Every payload sealed under a document key MUST carry the author signature and the commitment tag. | `ZK-001`; this spec section 2 | none | fails -- the document name and every media blob take the bare cipher with no signature and no tag, so the relay can substitute one for another inside a document nobody has to be a member of (`SYNC-068`, `SYNC-051`); lands with the `The document name is sealed like every other payload` Story (`ambre-roadmap.md` E7) | failing |

### 1.2 A payload is sealed

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-004` | `POST` | A message MUST NOT be replayable into another document: the commitment tag MUST bind the ciphertext to its document. | this spec section 2 | `ambre_sync::pipeline::tests::tampered_document_id_rejected` -- a sealed message whose document is forged on its public metadata fails to open; `ambre_crypto::tests::commitment_tag_wrong_aad_rejected` -- the same one layer down | none | verified |
| `SYNC-021` | `POST` | A payload at or below the compression threshold MUST NOT be compressed. | this spec section 2 | `ambre_sync::pipeline::tests::compression_threshold` -- a payload exactly at the threshold comes back marked raw, and one byte over comes back compressed and round-trips | none | verified |
| `SYNC-003` | `POST` | Every sealed message MUST use a fresh 192-bit nonce drawn from the operating system generator. | this spec section 2 | none | unheld -- sealing is exercised (`ambre_crypto::tests::prop_roundtrip_encrypt_decrypt`), and nonce freshness is asserted nowhere: no test seals the same payload twice and compares the two nonces; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-022` | `POST` | A snapshot MUST pass the wire checks before it is uploaded, because no relay ever sees it. | this spec section 2 | none | unheld -- the seal calls the check and returns its error, and no test hands the seal a snapshot that fails one: the two refusals the check can raise are unreached from here; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-020` | `FRAME` | The seal MUST NOT place the compression version outside the ciphertext, where the relay would read it. | `ZK-007`; this spec section 2 | none | unheld -- `ambre_sync::pipeline::tests::roundtrip_snapshot_large_compresses` and `::roundtrip_update_small` recover the version through the seal, which shows where it is today; nothing refuses a version byte moved into the public metadata, and a byte there tells the relay how compressible a document is; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 An inbound frame is opened

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-019` | `POST` | An inbound frame MUST pass three checks -- the signature, the commitment tag and the cipher -- and each MUST refuse on its own. | this spec section 2 | `ambre_crypto::tests::invalid_signature_rejected` -- the signature refusal; `::commitment_tag_wrong_aad_rejected` -- the tag refusal; `::tampered_ciphertext_fails` -- the cipher refusal, so no check can be silently skipped without a test going red | none | verified |
| `SYNC-005` | `FAIL` | A message sealed under a different document key MUST be rejected, never applied. | this spec section 2 | `ambre_sync::pipeline::tests::wrong_document_key_rejected` -- a sealed update opened under a foreign key answers the error; `ambre_crypto::tests::tampered_ciphertext_fails`; `::prop_single_byte_mutation_fails`; `::invalid_signature_rejected` -- the byte-level refusals under it | none | verified |
| `SYNC-008` | `FAIL` | An ephemeral frame whose counter is not greater than the last one seen for its author and document MUST be dropped. | this spec section 2 | `ambre_sync::ephemeral_tests::monotonic_counter_accepted_replay_dropped` -- an equal and a lower counter are both refused, and forward progress resumes, and the check has one hole the rule does not state, an author evicted by `SYNC-088`'s cap losing the baseline the rule reads and replaying once | none | verified |
| `SYNC-009` | `FAIL` | Decompression MUST be refused past a size ceiling drawn from the compressed length. | audit `H9`; this spec section 2 | `ambre_sync::pipeline::tests::decompress_bomb_rejected` -- 8 MiB compressed answers the typed refusal; `::decompress_within_ceiling_ok` -- a legitimate payload is untouched by the bound, the half that keeps the ceiling from being a denial of service of its own | none | verified |
| `SYNC-090` | `FAIL` | An unknown compression version MUST be refused, never guessed. | audit `H9`; this spec section 2 | `ambre_sync::pipeline::tests::decompress_unknown_version_rejected` -- the version byte is echoed in a typed error rather than falling through to a default codec | none | verified |
| `SYNC-089` | `POST` | The signature MUST be verified before the cipher is opened. | this spec section 2; `ZK-001` | none | unheld -- the three refusals are each driven, and their order is not: `SYNC-019`'s tests would pass on any order, so nothing would go red if the open decrypted first and authenticated afterwards. The order is what keeps unauthenticated bytes out of the decoder; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-072` | `FAIL` | A signature MUST be accepted only in its canonical form. | audit `M13` | none | unheld -- the client verifier and the relay one both call the strict form, which refuses a non-canonical scalar and a small-order key, and no test presents a second valid signature over one message; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-091` | `POST` | A failed check MUST reach the user's client as a warning. | this spec section 2 | none | unheld -- every refusal returns a typed error and the caller raises one warning code for all of them, so a reader cannot tell a corrupt frame from a wrong key, and a wrong key is the one that means a re-key was missed. No test drives the path; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-023` | `FRAME` | A frame that fails any check MUST NOT be applied in part. | this spec section 2 | none | unheld -- each refusal returns before the payload reaches the engine, so the whole-frame drop is held by the checks running ahead of the apply; no test asserts the document is unchanged after a refused frame; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** four rows of `ambre-spec-budgets.md section 1`. The outbound seal of 1 KB is under
  200 us, the inbound open under 350 us, and the round trip under 600 us. A 100 KB snapshot
  compresses under 5 ms. `criterion` measures the primitives. No bench measures the
  pipeline.
* **Boundary.** this is the boundary `ambre-spec-zero-knowledge.md` draws, on the client side. Every
  byte of user content that leaves the client leaves through the seal here. `SYNC-001` and
  `SYNC-020` keep the metadata down to the whitelist `ZK-007` names.
* **Verification.** 8 `verified`, 1 `failing`, 8 `untested`. What holds is refusal: a tampered byte,
  a wrong document and a wrong key fail closed. So do a bad signature, a replayed counter, a
  decompression bomb and an unknown codec. What is loose is every rule stated as an absence. No unsealed
  send is forbidden, no nonce asserted fresh, no order established.
---

## 2. MODEL

**CRDT containers.** none. The envelope wraps a payload and reads none of it.

**One pipeline, three messages.** A delta, a snapshot and a presence frame differ by their public
metadata and by their transport. The seal and the open are the same code for all three.

| Message | Carries | Travels by | Metadata the relay reads |
|---|---|---|---|
| an update | one CRDT delta | the socket | the document, the author key, the parent snapshot, the clock |
| a snapshot | a full CRDT state | a presigned upload, straight to object storage | the same, plus the parent proof |
| an ephemeral frame | a presence sample | the socket | the document, the author key, the counter |

**The seal, in steps.** The output is one message of the three shapes above.

1. compress the payload when it is over the threshold, and record which of the two ways in one
   version byte (`SYNC-021`);
2. form the plaintext as the version byte followed by the body, so the version is sealed with it
   (`SYNC-020`);
3. encrypt with XChaCha20-Poly1305 under the document key, with a fresh 192-bit nonce
   (`SYNC-003`). The public metadata is the additional data that binds the ciphertext to it;
4. commit the key: an HMAC-SHA256 over the nonce, the ciphertext and the metadata bytes
   (`SYNC-004`);
5. sign the nonce, the ciphertext, the metadata bytes and the commitment tag with the author's
   Ed25519 key (`SYNC-018`).

**The open, in the inverse order.** The order is the rule, not an implementation detail. The
signature says who wrote the frame, the tag says under which key, and only then is the cipher run
(`SYNC-019`).

| Step | Refuses | On failure |
|---|---|---|
| 1. verify the signature against the author key the metadata names | a forged author | drop, and warn |
| 2. verify the commitment tag against the document key | a frame moved from another document, or a wrong key | drop, and warn |
| 3. open the cipher, with the metadata as the additional data | a mutated byte anywhere | drop, and warn |
| 4. read the version byte and decompress under the ceiling | an unknown version, and a bomb | drop, and warn |

**Why the commitment tag exists.** XChaCha20-Poly1305 is not key-committing: one ciphertext can in
theory open under two keys. The HMAC binds the frame to one key, which is what stops a compromised
relay re-injecting a ciphertext into another document (`SYNC-004`).

**The decompression ceiling.** The document key is shared with every collaborator. An authorized
peer can therefore craft a bomb that has already passed the signature, the tag and the cipher. The
ceiling is derived per message, and the decoder streams into it rather than allocating it.

| Bound | Value | Why |
|---|---|---|
| ratio over the compressed length | 200 | zstd reaches a thousandfold on crafted input |
| floor | 1 MiB | a few compressed bytes legitimately expand to a normal delta |
| absolute cap | 64 MiB | the client-memory pillar is 100 MB for a whole document |

**The ephemeral counter is a per-device replay guard**, kept apart from the identity the frame is
delivered under. It is keyed by the author's signing key, which is per session. The map is capped
so a peer minting keys cannot grow it without bound (`SYNC-008`). Who the author *is* is
`ambre-spec-membership.md`.

**Code.**

* `crates/ambre-sync/src/pipeline.rs` -- the compression, the three seals and the three opens;
* `crates/ambre-crypto/src/envelope.rs` -- the seal and the open of the envelope itself;
* `crates/ambre-crypto/src/commitment.rs` -- the commitment tag;
* `crates/ambre-protocol/src/messages.rs` -- the three message shapes and their wire checks.

---

## 3. SURFACE

The envelope has no surface. A user sees it only when it refuses something, and then only as the
warning `SYNC-023` describes. That is content which does not arrive, and a message that does not
say which of the four refusals fired.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One pipeline for the three message types | a seal per message type, each with its own additional data rule | the three differ by their metadata and their transport, never by their cryptography, so a pipeline per type triples the surface that has to be reviewed line by line -- which `ambre-spec-testing.md section 3` says is the mechanism here |
| A commitment tag beside the cipher's own tag | trusting the additional data alone | it binds the metadata to the ciphertext and says nothing about the key. The cipher's tag proves the ciphertext was not touched, not which key opened it, so without the commitment a relay can move a frame between two documents and let the receiver decide it is corrupt -- a silent failure rather than a refusal |
| The signature covers the commitment tag | signing the nonce, the ciphertext and the metadata alone | the tag is then the one field an attacker may rewrite, and rewriting it turns a refusal at step 2 into a refusal at step 3: the same drop, a different diagnosis, and one more thing a reader has to reason about |
| The version byte is sealed, not public | a version byte in the public metadata | it would let a relay reject an unknown version early, and it would let it read whether the payload compressed -- a property of the content. That tells a relay whether a delta was above or below the threshold, for every delta, which is a channel `ZK-007` does not allow |
| The ceiling is derived per message, with a floor and a cap | one absolute limit for every message | it is either too low for a large legitimate snapshot or too high to bound a small hostile frame. Three numbers hold both ends and the pillar figure |
| Making an unsealed payload unrepresentable is a type change, not a test | a test that walks every send site | it passes until the next send site is added. `SYNC-001` is the rule the product rests on and it has no test shape: any test asserts a path was taken, and the rule is that no other path exists. A private field with one constructor puts the compiler on it, permanently, which is the top of the ladder in `ambre-spec-testing.md section 2` |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Two payloads take a bare cipher beside this pipeline.** The document name and every media blob
  are sealed under the document key with a constant additional-data string, no commitment tag and
  no signature. So neither is bound to its document, neither names an author, and a rotation leaves
  both unreadable (`SYNC-018`, `SYNC-068`, `SYNC-051`, `ambre-spec-key-rotation.md`).
* **The wire ciphertext accepts any bytes.** It is a public byte vector, so `SYNC-001` cannot be
  stated as a test and is held today by reading the send sites.
* **No nonce is ever compared to another.** The generator is the operating system's, and nothing in
  the suite would notice a nonce derived from a counter.
* **The four refusals arrive as one warning code.** A user, and a developer reading a console, cannot
  separate a corrupt frame from a frame sealed under a key they no longer hold.
* **The parent proof of a snapshot is empty.** The chain that would make snapshot reordering
  detectable without decryption is designed and unbuilt: every snapshot this client uploads carries
  an empty proof (`ambre-spec-delta-order.md`).
* **No bench measures this pipeline.** The four budgeted operations are measured on the primitives
  in `ambre-crypto`, so the compression step and the metadata encoding are outside every figure.
* **Nothing fuzzes the open path.** It is fed by an untrusted relay, which is the case
  `ambre-spec-testing.md section 2` names as not optional, and no target exists.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Which key seals a frame, and who holds it | `ambre-spec-sync.md`, `SYNC-002` |
| What the relay may learn, and what counts as a leak | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| In what order sealed updates are applied, and what waits | `ambre-spec-delta-order.md` |
| Which user is behind an author key | `ambre-spec-membership.md`, `SYNC-014` |
| What a rotation does to a frame sealed under an older key | `ambre-spec-key-rotation.md` |
| How a media blob is addressed and transferred | `ambre-spec-media.md` |
| What the relay does with a frame it cannot open | `ambre-spec-sequencer.md`, `ambre-spec-broadcast.md` |
| What a presence payload holds, and what it paints | `ambre-spec-presence.md` |
