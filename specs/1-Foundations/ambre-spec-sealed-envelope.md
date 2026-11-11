# AMBRE -- FEATURE: THE SEALED ENVELOPE

**Type:** Horizontal (framework)

> Nothing a user writes leaves the browser readable. What the relay carries is a sealed envelope. It
> can route it, count it and drop it. It can never open it, move it to another document, or replay
> it.

Every payload the Sync worker sends takes this path, and every envelope it accepts comes back through
the inverse. This document owns the envelope. What a payload means is the document that produced
it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SYNC`, shared with
`ambre-spec-sync.md`, `ambre-spec-delta-order.md`, `ambre-spec-membership.md`,
`ambre-spec-key-rotation.md` and `ambre-spec-media.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-088` | `INV` | The per-document map of remote ephemeral counters MUST be bounded. | this spec section 2; `SYNC-008` | `ambre_sync::ephemeral_tests::author_map_is_bounded` -- a hundred authors past the cap, and the map never exceeds it, and a peer can mint signing keys at will, so without the cap one document's presence state is an unbounded allocation | none | verified |
| `SYNC-001` | `INV` | The Sync worker MUST NOT hand the socket a payload that has not passed through the sealing pipeline. | `ZK-001`; this spec section 2 | `ambre_sync::seal_tests::a_payload_frame_built_outside_the_seal_never_reaches_the_socket` -- a payload frame reaches the socket only as a `SealedFrame`, which the seal alone builds | none | verified |
| `SYNC-018` | `INV` | Every payload sealed under a document key MUST carry the author signature and the commitment tag. | `ZK-001`; this spec section 2 | `ambre_sync::pipeline::tests::a_sealed_object_carries_the_signature_and_the_tag` -- a name or a blob carries both, a flipped signature byte and a re-signed wrong tag each refuse; structural -- no Sync path calls the bare cipher under a document key | none | verified |
| `SYNC-125` | `INV` | Every sealed payload MUST name, in its public metadata, the identifier of the key that sealed it. | this spec section 2; `SYNC-115` | `ambre_sync::pipeline::tests::a_frame_names_the_key_that_sealed_it` -- an update names its key, and a seal or an open under another key is refused; `ambre_sync::pipeline::tests::an_object_names_the_key_that_sealed_it` -- a name or a blob names its key in its header, and a header rewritten to name another fails to open. The snapshot and the ephemeral take the same check, which no test drives | none | verified |

### 1.2 A payload is sealed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-004` | `POST` | A message MUST NOT be replayable into another document: the commitment tag MUST bind the ciphertext to its document. | this spec section 2 | `ambre_sync::pipeline::tests::tampered_document_id_rejected` -- a sealed message whose document is forged on its public metadata fails to open; `ambre_crypto::tests::commitment_tag_wrong_aad_rejected` -- the same one layer down | none | verified |
| `SYNC-021` | `POST` | A payload at or below the compression threshold MUST NOT be compressed. | this spec section 2 | `ambre_sync::pipeline::tests::compression_threshold` -- a payload exactly at the threshold comes back marked raw, and one byte over comes back compressed and round-trips | none | verified |
| `SYNC-003` | `POST` | Every sealed message MUST use a fresh 192-bit nonce drawn from the operating system generator. | this spec section 2 | `ambre_crypto::tests::every_seal_draws_a_fresh_nonce` -- one payload sealed sixty-four times under one key repeats no nonce and no ciphertext. The seal takes no nonce argument, so none can be passed in | none | verified |
| `SYNC-022` | `POST` | A snapshot MUST pass the wire checks before it is uploaded, because no relay ever sees it. | this spec section 2 | `ambre_sync::pipeline::tests::a_snapshot_that_fails_a_wire_check_is_never_sealed` -- a parent clock list naming one author twice draws the check's own refusal and no snapshot. The empty-cipher refusal cannot arise from a seal, which always appends a tag | none | verified |
| `SYNC-112` | `POST` | The payload of an update MUST carry the session nonce of its author inside the ciphertext. | this spec section 2; `ambre-spec-op-authorship.md` (`AUTHOR-010`) | `ambre_sync::pipeline::tests::the_session_nonce_is_sealed_inside_the_ciphertext` -- one delta sealed under two nonces shows the relay the same readable part, and each opens back to its own nonce | none | verified |
| `SYNC-113` | `POST` | An update that carries an activity summary MUST carry it inside the ciphertext. | this spec section 2; `ambre-spec-residency.md` | `ambre_sync::pipeline::tests::the_activity_summary_is_sealed_inside_the_ciphertext`; `ambre_sync::tests::a_summary_handed_with_a_delta_leaves_sealed_beside_it` -- the relay's readable part is the same with or without a summary, and the summary the Logic thread hands opens from inside the ciphertext | none | verified |
| `SYNC-020` | `FRAME` | The seal MUST NOT place the compression version outside the ciphertext, where the relay would read it. | `ZK-007`; this spec section 2 | `ambre_sync::pipeline::tests::the_compression_version_is_sealed_inside_the_ciphertext` -- the version byte is read back from the opened plaintext and from no public field | none | verified |

### 1.3 An inbound envelope is opened

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-019` | `POST` | Each of the three checks an inbound sealed envelope passes -- the signature, the commitment tag and the cipher -- MUST refuse that envelope on its own. | this spec section 2 | `ambre_crypto::tests::invalid_signature_rejected` -- the signature refusal; `::commitment_tag_wrong_aad_rejected` -- the tag refusal; `::tampered_ciphertext_fails` -- the cipher refusal, so no check can be silently skipped without a test going red | none | verified |
| `SYNC-005` | `FAIL` | A message sealed under a different document key MUST be rejected, never applied. | this spec section 2 | `ambre_sync::pipeline::tests::wrong_document_key_rejected` -- a sealed update opened under a foreign key answers the error; `ambre_crypto::tests::tampered_ciphertext_fails`; `::prop_single_byte_mutation_fails`; `::invalid_signature_rejected` -- the byte-level refusals under it | none | verified |
| `SYNC-008` | `FAIL` | An ephemeral whose counter is not greater than the last one seen for its author and document MUST be dropped. | this spec section 2 | `ambre_sync::ephemeral_tests::monotonic_counter_accepted_replay_dropped` -- an equal and a lower counter are both refused, and forward progress resumes, and the check has one hole the rule does not state, an author evicted by `SYNC-088`'s cap losing the baseline the rule reads and replaying once; `ambre_sync::ephemeral_tests::below_the_cap_each_author_keeps_its_counter`; `::at_the_cap_a_known_author_evicts_nobody` -- the cap evicts for a new author alone, so a known author keeps the baseline its replay is refused against | none | verified |
| `SYNC-009` | `FAIL` | Decompression MUST be refused past a size ceiling drawn from the compressed length. | audit `H9`; this spec section 2 | `ambre_sync::pipeline::tests::decompress_bomb_rejected` -- 8 MiB compressed answers the typed refusal; `::decompress_within_ceiling_ok` -- a legitimate payload is untouched by the bound, the half that keeps the ceiling from being a denial of service of its own; `ambre_sync::pipeline::tests::an_output_at_the_ceiling_opens_and_one_byte_past_is_refused` -- the ceiling is inclusive, to the byte | none | verified |
| `SYNC-090` | `FAIL` | An unknown compression version MUST be refused, never guessed. | audit `H9`; this spec section 2 | `ambre_sync::pipeline::tests::decompress_unknown_version_rejected` -- the version byte is echoed in a typed error rather than falling through to a default codec | none | verified |
| `SYNC-138` | `FAIL` | A document name or a media blob of a layout the reader does not know MUST be refused by that layout, before any other part of it is read. | this spec section 2; `ambre-spec-media.md` | `ambre_sync::pipeline::tests::an_unknown_object_layout_is_refused_before_any_check` -- a later layout is named in a typed error by the open and by the key read, and one byte alone is enough to refuse | none | verified |
| `SYNC-089` | `POST` | The signature MUST be verified before the cipher is opened. | this spec section 2; `ZK-001` | `ambre_crypto::tests::the_signature_is_checked_before_the_tag_and_the_cipher` -- one envelope fails all three checks and draws the signature refusal. Re-signed it draws the tag refusal, and with a matching tag the cipher refusal | none | verified |
| `SYNC-072` | `FAIL` | A signature MUST be accepted only in its canonical form. | audit `M13` | `ambre_crypto::signature::tests::only_the_canonical_signature_is_accepted` -- the client; `ambre_server::ws::handler::message::ed25519_tests::only_the_canonical_signature_is_accepted` -- the relay. Each refuses the scalar raised by the group order, and a small-order key under a forged signature | none | verified |
| `SYNC-091` | `POST` | A failed check MUST reach the user's client as a warning. | this spec section 2 | `ambre_sync::updates::tests::every_refused_frame_reaches_logic_as_a_warning` -- an unsigned envelope, one under a key never held and one with a broken tag each raise one warning naming the document. The message names the failed check, so a missed re-key reads apart | none | verified |
| `SYNC-023` | `FRAME` | An envelope that fails any check MUST NOT be applied in part. | this spec section 2 | `ambre_sync::updates::tests::a_refused_frame_is_applied_in_no_part` -- a tampered and an unsigned envelope in one replay each deliver none of their payload, and their neighbours arrive whole | none | verified |
| `SYNC-135` | `FAIL` | A document name or a media blob whose author key belongs to no member of its document MUST be refused. | this spec section 2; `SYNC-018` | not built | unbuilt -- lands with the `The author of a sealed object is a member` Story (`ambre-roadmap.md` E7) | unimplemented |
| `SYNC-114` | `POST` | An opened update MUST reach the Logic thread with its session nonce, the PeerID it derives and the user of its signer beside the delta. | this spec section 2; `AUTHOR-013` | `ambre_sync::updates::tests::an_opened_update_reaches_logic_with_its_nonce_its_peer_and_its_signer` -- the delta arrives with the nonce its body carried, the PeerID its signer derives, and the member the directory names, or none | none | verified |
| `SYNC-144` | `POST` | An opened update that carries an activity summary MUST reach the Logic thread with it beside the delta. | this spec section 2; `ambre-spec-residency.md` | `ambre_sync::updates::tests::an_opened_update_hands_logic_its_summary_beside_the_delta` -- an update with a summary hands it beside its delta, and one with none hands none | none | verified |

* **Budget.** four rows of `ambre-spec-budgets.md section 1`. The outbound seal of 1 KB is under
  200 us, the inbound open under 350 us, and the round trip under 600 us. A 100 KB snapshot
  compresses under 5 ms. `criterion` measures the primitives. No bench measures the
  pipeline.
* **Boundary.** this is the boundary `ambre-spec-zero-knowledge.md` draws, on the client side. Every
  byte of user content that leaves the client leaves through the seal here. `SYNC-001` and
  `SYNC-020` keep the metadata down to the whitelist `ZK-007` names.
* **Verification.** 23 `verified`, 1 `unimplemented`. What holds is refusal: a tampered byte, a
  wrong document and a wrong key fail closed. So do a bad signature, a replayed counter, a
  decompression bomb and an unknown codec. The nonce is drawn fresh, and the signature is checked
  first.
---

## 2. MODEL

**CRDT containers.** none. The envelope wraps a payload and reads none of it.

**One pipeline, three messages.** A delta, a snapshot and a presence sample differ by their public
metadata and by their transport. The seal and the open are the same code for all three.

| Message | Carries | Travels by | Metadata the relay reads |
|---|---|---|---|
| an update | one CRDT delta | the socket | the document, the author key, the key identifier, the parent snapshot, the clock |
| a snapshot | a full CRDT state | a presigned upload, straight to object storage | the same, plus the parent proof |
| an ephemeral | a presence sample | the socket | the document, the author key, the key identifier, the counter |

**The seal, in steps.** The output is one message of the three shapes above.

1. compress the payload when it is over the threshold, and record which of the two ways in one
   version byte (`SYNC-021`);
2. form the plaintext as the version byte followed by the body, so the version is sealed with it
   (`SYNC-020`). The body of an update holds the session nonce of its author (`SYNC-112`). It holds
   the activity summary when the application writes one (`SYNC-113`), and then the delta;
3. encrypt with XChaCha20-Poly1305 under the document key, with a fresh 192-bit nonce
   (`SYNC-003`). The public metadata is the additional data that binds the ciphertext to it;
4. commit the key: an HMAC-SHA256 over the nonce, the ciphertext and the metadata bytes
   (`SYNC-004`);
5. sign the nonce, the ciphertext, the metadata bytes and the commitment tag with the author's
   Ed25519 key (`SYNC-018`).

**The open, in the inverse order.** The order is the rule, not an implementation detail. The
signature says who wrote the envelope, the tag says under which key, and only then is the cipher run
(`SYNC-019`).

| Step | Refuses | On failure |
|---|---|---|
| 1. verify the signature against the author key the metadata names | a forged author | drop, and warn |
| 2. verify the commitment tag against the key the envelope names (`SYNC-125`) | an envelope moved from another document, or a wrong key | drop, and warn |
| 3. open the cipher, with the metadata as the additional data | a mutated byte anywhere | drop, and warn |
| 4. read the version byte and decompress under the ceiling | an unknown version, and a bomb | drop, and warn |
| 5. split an update into its nonce, its summary and its delta | a body too short for its parts | drop, and warn |

```text
update body = nonce (16 bytes) || summary marker (1 byte) || summary, when the marker says so || delta
```

```rust
pub struct UpdateBody {
    pub nonce: SessionNonce,                 // ambre-spec-op-authorship.md
    pub summary: Option<ActivitySummary>,    // ambre-spec-residency.md
    pub delta: DeltaBytes,                   // a Loro update, opaque to the envelope
}
```

The marker is `0` for no summary and `1` for a summary of `ambre-spec-residency.md`, which then
precedes the delta. Any other value is refused, and an opened summary reaches the Logic thread
beside its delta (`SYNC-144`).

**Two standalone objects take the same envelope.** A document name and a media blob travel outside
any message. So they carry their own parts: the layout, the key identifier, the author key, the
signature, the commitment tag, the nonce, then the ciphertext. The layout and the key identifier
are also bound into the additional data, so a header cannot be made to name another of either.
Neither object is compressed.

**Why a layout byte.** The relay keeps a blob across releases. Without the byte, an object of an
older layout fails as forged or as sealed under a foreign key. With it, the reader refuses it by
name before reading anything else (`SYNC-138`). Every object of a release shares one layout, so the
byte tells the relay nothing about the content. A name's
additional data holds its document id, so it opens under its own document only (`SYNC-018`,
`SYNC-068`).

**Why the nonce and the summary ride inside.** The session nonce binds each operation to its signer
(`ambre-spec-op-authorship.md`). The activity summary tells a client what an update holds with no
document loaded (`ambre-spec-residency.md`). Either one outside the ciphertext would tell the relay
when a session starts, or what kind of change a member made.

**Why the commitment tag exists.** XChaCha20-Poly1305 is not key-committing: one ciphertext can in
theory open under two keys. The HMAC binds the envelope to one key, which is what stops a compromised
relay re-injecting a ciphertext into another document (`SYNC-004`).

**The decompression ceiling.** The document key is shared with every collaborator. An authorized
peer can therefore craft a bomb that has already passed the signature, the tag and the cipher. The
ceiling is derived per message, and the decoder streams into it rather than allocating it.

| Bound | Value | Why |
|---|---|---|
| ratio over the compressed length | 200 | zstd reaches a thousandfold on crafted input |
| floor | 1 MiB | a few compressed bytes legitimately expand to a normal delta |
| absolute cap | 64 MiB | the client-memory pillar is 100 MB for a whole document |

**The ephemeral counter is a per-device replay guard**, kept apart from the identity the envelope is
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
| A commitment tag beside the cipher's own tag | trusting the additional data alone | it binds the metadata to the ciphertext and says nothing about the key. The cipher's tag proves the ciphertext was not touched, not which key opened it, so without the commitment a relay can move an envelope between two documents and let the receiver decide it is corrupt -- a silent failure rather than a refusal |
| The signature covers the commitment tag | signing the nonce, the ciphertext and the metadata alone | the tag is then the one field an attacker may rewrite, and rewriting it turns a refusal at step 2 into a refusal at step 3: the same drop, a different diagnosis, and one more thing a reader has to reason about |
| The version byte is sealed, not public | a version byte in the public metadata | it would let a relay reject an unknown version early, and it would let it read whether the payload compressed -- a property of the content. That tells a relay whether a delta was above or below the threshold, for every delta, which is a channel `ZK-007` does not allow |
| The ceiling is derived per message, with a floor and a cap | one absolute limit for every message | it is either too low for a large legitimate snapshot or too high to bound a small hostile envelope. Three numbers hold both ends and the pillar figure |
| Making an unsealed payload unrepresentable is a type change, not a test | a test that walks every send site | it passes until the next send site is added. `SYNC-001` is the rule the product rests on and it has no test shape: any test asserts a path was taken, and the rule is that no other path exists. A private field with one constructor puts the compiler on it, permanently, which is the top of the ladder in `ambre-spec-testing.md section 2` |
| The key identifier is public metadata | the identifier inside the ciphertext | the relay could not refuse an envelope under a key a removal left behind (`SRV-123`), and a client would try every key before it knew which one to open with |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The author of a name or a blob is checked, not resolved.** The open verifies the signature
  against the key the object carries. Nothing yet asks whether that key belongs to a member
  (`SYNC-135`).
* **A rotation still leaves a name and every blob unreadable.** Each names the key that sealed it,
  and nothing brings that key back yet (`SYNC-042`, `ambre-spec-key-rotation.md`).
* **The wire ciphertext accepts any bytes.** It is a public byte vector, so `SYNC-001` cannot be
  stated as a test and is held today by reading the send sites.
* **No nonce is ever compared to another.** The generator is the operating system's, and nothing in
  the suite would notice a nonce derived from a counter.
* **The four refusals arrive as one warning code.** A user, and a developer reading a console, cannot
  separate a corrupt envelope from one sealed under a key they no longer hold.
* **The parent proof of a snapshot is empty.** The chain that would make snapshot reordering
  detectable without decryption is designed and unbuilt: every snapshot this client uploads carries
  an empty proof (`ambre-spec-delta-order.md`).
* **No bench measures this pipeline.** The four budgeted operations are measured on the primitives
  in `ambre-crypto`, so the compression step and the metadata encoding are outside every figure.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Which key seals an envelope, and who holds it | `ambre-spec-sync.md`, `SYNC-002` |
| What the relay may learn, and what counts as a leak | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| In what order sealed updates are applied, and what waits | `ambre-spec-delta-order.md` |
| Which user is behind an author key | `ambre-spec-membership.md`, `SYNC-014` |
| What a rotation does to an envelope sealed under an older key | `ambre-spec-key-rotation.md` |
| How a media blob is addressed and transferred | `ambre-spec-media.md` |
| What the session nonce binds | `ambre-spec-op-authorship.md` |
| What an activity summary holds, and who reads it | `ambre-spec-residency.md` |
| What the relay does with an envelope it cannot open | `ambre-spec-sequencer.md`, `ambre-spec-broadcast.md` |
| What a presence payload holds, and what it paints | `ambre-spec-presence.md` |
