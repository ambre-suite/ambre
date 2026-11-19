# AMBRE -- SECURITY AUDIT: THE WORKSPACE PRE-AUDIT OUTSIDE THE CALL PATH, PASSES A TO D

> An addendum to [`security-preaudit-workspace-2026-11-19.md`](security-preaudit-workspace-2026-11-19.md),
> whose first pass it completes, and to [`security-audit.md`](security-audit.md).

This addendum holds the four passes of the MVP Story A security pre-audit that followed the first
pass, each split by scope and run read-only on `main` at `ed31f88`. Pass A holds `W100` and on, pass
B `W200`, pass C `W300` and pass D `W400`. It is an audit alone: it fixes nothing and ticks no box.
The roadmap carries one MVP story per group of findings. Each pass ends with what it did not read,
which carries no verdict.


## Security pre-audit, pass A: `ambre-crypto`, `ambre-protocol`, `ambre-core` secrets

Tree: `main` at `ed31f88`, read on 2026-11-19. Read-only: nothing was built, run or modified. Every finding comes from reading the code; none was executed.

### Summary

| Files read whole | |
| :-- | :-- |
| `ambre-crypto/src` | `lib.rs`, `aead.rs`, `envelope.rs`, `ecdh.rs`, `commitment.rs`, `signature.rs`, `keys.rs`, `keys/recovery.rs`, `passphrase.rs`, `strength.rs`, `hash.rs`, `peer.rs`, `random.rs`, `error.rs` |
| `ambre-protocol/src` | `lib.rs`, `messages.rs`, `public_data.rs`, `membership.rs`, `key_link.rs`, `master_rotation.rs`, `snapshot_confirm.rs`, `snapshot_report.rs`, `transcript.rs`, `ws.rs`, `version.rs`, `error.rs` |
| `ambre-core/src` | `secret.rs`, `fixed_bytes.rs`, `payload.rs`, `encoded.rs`, `envelope.rs`, `key_id.rs`, `public_key.rs`, `authorship.rs`, `version.rs`, `fingerprint.rs`, `hash.rs`, `credential.rs` |
| Specs and audits | `README.md`, `ambre-spec-zero-knowledge.md` (whole), `security-audit-blind-2026-11-17.md` (whole), `security-audit.md` (the 2026-11-14 to 2026-11-18 addenda and the crypto notes), `ambre-spec-op-authorship.md` (sections 3, 5, 6) |
| Read by excerpt, to trace a path | `ambre-sync/src/snapshot_trust.rs` (whole), `docfile.rs` (whole), `auth.rs:850-916`, `pipeline.rs:420-500` and `:900-957`, `updates.rs:340-430`, `lib.rs:1270-1330` |

| Severity | Count | Findings |
| :-- | :-- | :-- |
| high | 1 | W101 |
| medium | 2 | W100, W102 |
| low | 0 | |
| info | 2 | W103, W104 |

---

### W101 — high — A confirmation signs no byte of the snapshot it confirms

**Where.** `crates/ambre-protocol/src/snapshot_confirm.rs:20-35` (`confirm_transcript`); `crates/ambre-sync/src/snapshot_trust.rs:80-100` (`confirmation_holds`) and `:56-60`; `crates/ambre-sync/src/pipeline.rs:947-956` (`SnapshotDraft::seal`).

**Rule broken.** `AUTHOR-030` and `AUTHOR-031`, and the closure of audit `N31`. `ambre-spec-op-authorship.md` section 5 rejects "the relay's status alone" because "the relay and one writer would make a forged snapshot read as checked, and every cold open would adopt it". That pair of attackers still succeeds.

**Relation to earlier findings.** `P40` (open, medium) reaches the same end through the presigned link, with no help from the relay, inside 15 minutes. This is the root under it: the transcript covers the document, the snapshot id, the horizon and the author key, and no digest of the blob. A relay-side fix of `P40` leaves this path open, since the relay is the attacker here.

**Attack path.**
1. Mallory, a writer, builds an honest snapshot `S` under id `X` and horizon `h`, and uploads it.
2. Bob holds the deltas `S` covers, finds a match, and signs `confirm_transcript(doc, X, h, mallory_key)` (`updates.rs:399-408`). The relay stores the signature.
3. Mallory seals a second blob `S'` under the same `X` and `h`. `seal` takes the id as an argument, and the public data carries both, so her own signature covers them. `S'` holds operations under a PeerID of Alice, or drops operations of Alice.
4. The relay serves `S'` as the latest snapshot, with Bob's genuine confirmation.
5. Carol opens cold. `confirmation_holds` verifies Bob's signature over `(doc, X, h, mallory_key)`. The blob verifies under Mallory, its sealed id is `X` (`SYNC-257`) and its horizon is `h` (`SYNC-259`). `open_start` returns `start: Some(latest), held: None`, so the after-replay check of `AUTHOR-022` never runs on it.

**Impact.** Every cold open adopts a state Mallory wrote and Bob never saw, with operations attributed to other members. One honest confirmation of any snapshot by Mallory is enough, and it is reusable for as long as the relay serves that id.

**Fix, in words.** The confirmer signs a value that names the blob it checked: the envelope signature of the snapshot, or a digest of its nonce, ciphertext and commitment tag. The opener recomputes that value from the blob it downloaded and refuses a confirmation over another. The relay already sees each of these fields (`ZK-007`), so it learns nothing new. The transcript domain moves to `v2`.

---

### W100 — medium — The strength estimator credits well-known passwords above the floor

**Where.** `crates/ambre-crypto/src/strength.rs:343-397` (`Run::shape`), `:431-442` (`digits`), `:93-96` (`SEQUENCES`), `:244-246` (`said_before`).

**Rule broken.** `ZK-035` and `ZK-042` in what they are for. `ambre-spec-zero-knowledge.md` section 2 states that the model's "errors fall on the side of a refusal", and section 1 states that "a passphrase an offline guess finds seals no seed". Both are false for the inputs below.

**How the estimate goes wrong.** Three rules each over-credit:
- A run of letters reads as random, at 4.7 bits a letter, whenever fewer than a quarter of its letters are vowels or it holds five consonants in a row. Many dictionary words and keyboard walks meet that test.
- A row of `SEQUENCES` is matched only up to its own length. The digit row is 11 characters, so a longer run of digits is random digits at 3.32 bits each.
- A repetition is detected between runs, never inside one run.

**Inputs admitted, traced by hand through the code (floor 4 800 centibits).**

| Passphrase | Estimate | Why |
| :-- | :-- | :-- |
| `christchurch` | 5 640 | 2 vowels in 12 letters, so random letters |
| `qazwsxedcrfv` | 5 640 | a keyboard column walk; 2 vowels in 12 |
| `qwertyasdfgh` | 5 640 | two keyboard rows joined; `sdfgh` is five consonants |
| `qwertyuiopasdfgh` | 7 520 | same, and longer than any single row |
| `123456789012345` | 4 980 | 15 digits, longer than the 11-digit row |
| `123123123123123` | 4 980 | one run; the period inside it is not seen |
| `strengths12` | 4 894 | one English word (1 vowel in 9) and two digits |
| `a b c d e f g h i j k l m n o p q r s` | 4 950 | 19 single letters at 2.5 bits each, plus one separator |

**Attack path.** A user sets one of these at account setup, change or recovery. `ChosenPassphrase::check` admits it, `derive_sealing_key` runs, and the relay stores the salt and the sealed seed. The relay, or a thief of its database, runs a standard guess list offline at one Argon2id derivation per guess. Each of these inputs sits in the first million entries of such a list.

**Impact.** The Master Seed of that account, so every key and every document it reaches. The user was told the passphrase passed a strength check.

**Fix, in words.** Make each rule fail closed:
- Treat a letter run that no rule recognizes as words-rate rather than random, unless it mixes case or classes inside the run.
- Match a sequence by walking the row cyclically and across rows, at any length.
- Detect a period inside a run and charge one unit plus the log of the count.
- Cap a digit-only passphrase.
- Add each row of the table above to `a_known_weak_password_is_refused_with_its_weakness`, and a property test that no concatenation of `COMMON` or `SEQUENCES` fragments reaches the floor.

---

### W102 — medium — A PeerID is 64 bits, and its second preimage is within the budget the spec gives its own attacker

**Where.** `crates/ambre-crypto/src/peer.rs:37-51` (`derive_peer_id`); `crates/ambre-sync/src/lib.rs:1311-1315` (`open_signed_update`).

**Rule broken.** `AUTHOR-005` ("a registry entry MUST resolve to a user only when its PeerID derives from that user key and its nonce") holds, but the property it serves does not: one PeerID is meant to belong to one author. `ambre-spec-op-authorship.md` does not state the 64-bit margin anywhere, in section 5 or section 6.

**Attack path.**
1. Alice wrote in a document under PeerID `P`. Her signed entry for `P` sits in the registry.
2. Mallory, a writer of the same document, searches a 128-bit nonce `n'` with `derive_peer_id(mallory_key, n') == P`. The cost is 2^64 BLAKE2b blocks, divided by the number of PeerIDs Alice holds in that document (one per open that wrote).
3. Mallory seals a delta with `n'` in its body and operations under `P`, and signs it with her own key.
4. Each receiver derives `P` from Mallory's key and `n'` (`lib.rs:1314`). Per section 3 of the spec, the Logic thread accepts a delta whose operations all sit under the derived PeerID.
5. The registry entry for `P` is still Alice's, and it still verifies. A reader that resolves `P` after a snapshot reads Alice.

**Cost.** Section 2 of the zero-knowledge spec names an attacker with 100 GPUs for one year. At roughly 5 x 10^9 BLAKE2b a second per card (an order of magnitude from memory, not measured), that farm computes about 2^64 hashes in the year. With a few hundred target PeerIDs, the search takes days.

**Impact.** Operations written by Mallory are attributed to Alice once the signed deltas are folded into a snapshot, or the snapshot check of `AUTHOR-021` refuses every honest snapshot of the document. Either way one member acts on the identity of another.

**Limit of this trace.** The Logic-side compare lives in `ambre-logic`, outside this scope. Step 4 is read from the spec, not from that code.

**Fix, in words.** The 64 bits are Loro's, so the binding must not rest on them alone. Two options:
- A receiver refuses a delta whose derived PeerID already has a registry entry that resolves to another user.
- The first delta of a session must carry its registry entry, and the receiver keeps the first signer it saw for each PeerID.

Either way, state the 64-bit margin in section 6 of the spec.

---

### W103 — info — The sealed Master Seed carries no layout or parameter version, and its additional data is a constant

**Where.** `crates/ambre-crypto/src/passphrase.rs:3-5` and `:21-23` (the claim); `crates/ambre-crypto/src/keys.rs:137-146` (`wrap_for_storage` writes `nonce || ciphertext`); `crates/ambre-protocol/src/master_rotation.rs:20-24` (`SealedSeed` holds the sealed seed and the salt alone); `crates/ambre-sync/src/auth.rs:872` and `:907` (`Aad::of(b"ambre-master-seed")`).

**Rule.** None today. The module states that "the salt and version travel with the stored key material", and `Argon2Cost::V1` says the lane count "can be raised without breaking existing accounts". No version is stored, and the seal binds none. A local-store row has a layout byte (`RowLayout`) and a sealed file binds its header; the seed has neither.

**Why it matters later.** The first change of the Argon2id cost needs a version field. If that field is added beside the blob and not inside the additional data, the relay chooses which parameters a client derives with, and serves the weaker set. The additional data also binds no account, so the seal does not tell two accounts apart.

**Fix, in words.** Before a second parameter set exists, give the sealed seed a version byte and bind that byte, the salt and the user id as additional data. Correct the two comments now.

---

### W104 — info — The estimator leaves partial copies of the passphrase on the heap

**Where.** `crates/ambre-crypto/src/strength.rs:186-188` (`Chars::of`).

**Rule.** `ZK-010` and `ZK-019` in spirit. The module doc says "the one copy of the passphrase the estimator makes is a `Secret`, wiped when the estimate ends".

**Path.** `chars().collect()` builds the `Vec<char>` from an iterator whose lower size bound is a quarter of the byte length. For an ASCII passphrase the vector grows by reallocation, and each freed buffer holds a prefix of the passphrase, unwiped, in the linear memory of the Sync worker. A 28-character passphrase leaves two such buffers. `folded` does not have the defect: its source has an exact length.

**Fix, in words.** Reserve the byte length of the text inside `Secret::filled` and push the characters into it, so the buffer never moves.

---

### Checked and found sound

- **Nonces.** Every seal draws 24 fresh bytes from `getrandom` in one private function (`aead.rs:249`). No caller supplies a nonce. A failed entropy source panics; it never yields a key or a nonce.
- **Order of an open.** Strict signature, then commitment tag, then AEAD (`envelope.rs:115-137`). The tag compares through `verify_slice`.
- **Signatures.** `verify_strict` on every path, including `peer.rs`. The envelope transcript is length-framed. Each whole-message transcript opens with its own domain, and no domain is a prefix of another or can equal the first bytes of an envelope transcript.
- **Key wrap.** A low-order peer key is refused. The KEK binds both exchange keys in sender-then-recipient order, and both sides agree. `KeyWrapper` wipes each KEK on drop.
- **Seed derivation and recovery words.** Domains are distinct and frozen. The recovery reader refuses length, word and checksum before the seed leaves the function, and a refusal wipes the partial seed.
- **Secret hygiene.**
  - `Secret` has no `Clone`, serde, `PartialEq` or `Display`.
  - `Confidential` and `Credential` roles are sealed where a serde door exists.
  - `FixedBytes` roles are sealed, and none holds a secret.
  - `Debug` of every payload and encoded role prints a length.
  - The dalek crates keep their default `zeroize` feature.
- **Wire decoding.**
  - No `unwrap` or index reaches hostile bytes in the files read.
  - Fixed widths are enforced by type.
  - `Encoded` enforces the wrapped-key and master-key lengths.
  - `MembershipEntry::decode` is canonical.
  - The version byte is checked before the body.
  - Byte strings decode as one bounded block. This relies on `postcard` and `serde_bytes` behaviour that was not re-read.
- **Commitment tag.** It concatenates ciphertext and additional data with no framing, unlike the signature. No path uses that: the framed signature covers the same fields and the tag, and only a holder of the document key computes a tag. Framing it at the next layout change would remove the dependence on the signature.
- **Known and unchanged, not re-reported.** The wrap binds no document (`P59`), the three frames share one shape of additional data (`P25`), the document key doubles as HMAC key (`P11`), the registry entry does not sign its user (`P12`), the fingerprint covers the exchange key alone (`P44`). The sealed file takes no strength floor, which `ambre-spec-docfile.md` section 5 already states.

### Not read

- `ambre-crypto`: `call.rs`, `call_tests.rs` (call path, out of scope); `tests.rs` (grep hits only); `benches/`, `tests/phone_bench.rs`; `Cargo.toml` beyond the dependency lines.
- `ambre-protocol`: `signaling.rs` (call path, out of scope); `tests.rs`.
- `ambre-core`: every file not listed in the summary, among them `opaque.rs`, `redacted.rs`, `id.rs`, `seq.rs`, `bounded.rs`, `validated.rs`, `email.rs`, `serde_id.rs`, `object_key.rs`, `media.rs`, `error.rs`, `lib.rs`. Whether `Opaque` and `Redacted` hold their `Debug` promise was not checked.
- `ambre-sync` and `ambre-logic` beyond the excerpts named above. The Logic-side PeerID compare (W102, step 4) and the code that adopts a trusted snapshot whole (W101, step 5) are taken from the spec and from module docs.
- The specs that define the `SYNC-*`, `SRV-*` and `ARCH-*` rows these crates cite (`ambre-spec-sealed-envelope.md`, `ambre-spec-membership.md`, `ambre-spec-key-rotation.md`, `ambre-spec-newtypes.md`), `deny.toml`, and the first 400 lines and the remediation waves of `security-audit.md`.
- No third-party crate source.
## Security pre-audit, pass B: `crates/ambre-sync`

Read-only pass on `main` at `ed31f88`. Nothing was built or run: every finding below comes from reading the code, and none was reproduced.

### Summary

| Severity | Count | Findings |
| :-- | :-- | :-- |
| high | 2 | W200, W201 |
| medium | 2 | W202, W203 |
| low | 3 | W204, W205, W206 |
| info | 3 | W207, W208, W209 |

**Files read whole (production part, tests excluded):** `membership_log.rs`, `key_vouch.rs`, `pipeline.rs`, `chain.rs`, `snapshot_trust.rs`, `local_store.rs`, `log.rs`, `docfile.rs`, `renewal.rs`, `withheld.rs`, `rekey.rs`, `updates.rs`, `websocket.rs`, `lib.rs` (224 to 2266).

**Read in part:** `auth.rs`, `sharing.rs`, `http.rs`, `members.rs`, `channels.rs`, `space_rotation.rs`, `light.rs`. The "Not read" section gives the ranges.

**Specs:** `ambre-spec-zero-knowledge.md` (lines 1 to 437), `ambre-spec-membership.md` sections 3 to 6, the blind audit whole, and `security-audit.md` from the 2026-11-14 addendum on.

W200 and W201 share one shape, the one the blind pass named for `P26`: a path reads the members again and trusts the fresh answer. `N32` and `N33` closed it for the four paths that post to the relay. The paths that only change local state still walk from the anchor of the fresh answer and never read the footing the session holds.

---

### W200 (high): a fetched rotation and a reopen walk a cut log, so a removed member's key becomes current

**Where:** `websocket.rs:1912-2003` (`fetch_rotated_key`, walk at `:1970`, check at `:1986`); `updates.rs:1100-1141` (`opened_from_members`, walk at `:1110`); `websocket.rs:844-853` (a reopen replaces the whole document state); `key_vouch.rs:393-399` (check 4).

**Rule broken:** `SYNC-267` in intent (a log that does not pass through the latest head this client walked writes nothing), `SYNC-221`, `SYNC-225`, and the README guarantee that a removed member reads nothing written after the removal. `N33` reads fixed, and this path keeps the defect.

**Attack path (Eve removed, with the relay):**
1. Alice owns D; Bob and Eve are editors under K1. Alice revokes Eve at place p and installs K2. Bob is online, installs K2, and his footing stands past p.
2. Eve draws K3. She signs a link from K3 back to K1 at the head just before her revoke, where she still held a writing role. She wraps K3 for Bob under her exchange key, with the true anchor.
3. The relay sends Bob `KeyRotationRequired`. On the members read it serves Eve's wrap on Bob's row, Eve as wrapper, the genuine log cut before place p, and no debt.
4. `fetch_rotated_key` walks that log from the anchor of the served wrap. It never calls `Footing::walk`, so the cut is not seen.
5. Every check passes on the cut log:
   - Eve's exchange key is signed, so the wrapper is a member.
   - Her link is signed inside her writing stretch.
   - No entry names another key at or past the link's head, so check 4 does not fire.
   - The cut log holds no revoke.
   - The chain walks from K3 back to K1, the latest key the cut log names.
6. `install_key` accepts K3: it is not a key the cache or the chain reaches, so `SYNC-213` does not fire. Bob now seals under K3.

A second path reaches the same state without `KeyRotationRequired`. Bob closes D and opens it again in the same session. The open walks from the anchor of the served wrap and replaces the document state whole. The session footing in `log_footings` is not read, so the cut log plus Eve's wrap, or the old K1 wrap, is accepted.

**Impact:** Eve reads everything Bob writes from then on, with no warning. A later honest listing does not repair it: K2 still sits in Bob's cache, so the `SYNC-224` wait never starts. `P43` holds the same roll back across sessions as a known limit; here the footing exists and is not consulted.

**Fix:** walk the served log through `Footing::walk` in `fetch_rotated_key` and in an open of a document the session already holds a footing for, and refuse on `LogDiverged`. Keep the latest revoke this client walked, as `knowing` keeps the latest departure, so a log served without it still fails checks 4 and 6.

---

### W201 (high): a members refresh takes its directory, writers and space word from a log the relay rooted

**Where:** `sharing.rs:1693-1710` (`list_members_task`); `websocket.rs:1371-1381` (the handler); `membership_log.rs:164-170` (`grows_into`), `:830-832` (`binds`), `:108-112` (`admits_joiner`); `sharing.rs:259-306` (`rotate_entries_for`), `:1599-1605`; `space_rotation.rs:136`.

**Rule broken:** `SYNC-215` (audit `N19`), `SYNC-214` (audit `N18`), `SYNC-211`, `SYNC-227` (audit `N24`), and `ZK-001` through the last effect. `SYNC-266` names a settle, a revoke, an invite and a role change; the listing has the same shape and is not covered.

The task walks the log from the anchor of the wrap this very answer serves. The footing is passed in, and is used only to move itself forward: a `LogDiverged` there refuses nothing. No pin and no root is checked. Three effects follow.

**Effect 1: the directory is replaced with no condition (`N19` through the refresh).**
1. Bob holds D with Alice and Mallory, an editor who works with the relay.
2. The relay sends `MembersChanged`. In the answer, Bob's own row does not unwrap, and Alice's row carries Mallory's signing key.
3. No anchor means an unanchored log, and `binds` then lets every row stand.
4. The handler calls `set_signing_keys` before the `grows_into` test and whatever its result. The writers are kept; the directory is not.
5. Mallory's key is still a writer. Her next delta opens and resolves to Alice, so the Murmure author check and each cursor read as Alice's.

**Effect 2: a rooted superset passes `grows_into` (`N18` in part).**
1. The relay wraps any key for Bob under an exchange key of its own, with an anchor of its own.
2. It serves a log it signed as creator, which invites every signing key the held log granted, plus keys it chooses. An invite needs the author's signature alone.
3. The held writers are a subset, so the writers and the word are replaced.
4. A viewer who holds the current key now writes through the relay, and each key is bound to the member the relay names.

**Effect 3: a space word of the relay yields a channel key (relay alone).**
1. The relay does effect 2 on a space document. Its word lists a ghost member present, under a signing key and an exchange key of the relay.
2. It appends a self-join of that ghost to the genuine log of a public channel. `walk_under` admits it, because the word binds those keys.
3. At the next re-key of that channel (a settle, a revoke, or the space rotation after a leave), the write walks the genuine channel log from its footing under that word. `rotate_entries_for` wraps the fresh key for the ghost's exchange key.
4. The relay unwraps the channel key. It then reads the channel and writes into it as an editor.

**Impact:** one member acts as another with the relay's help (effects 1 and 2), and the relay alone obtains a document key (effect 3).

**Fix:** have the listing walk through `Footing::walk`, as the four writes do, and change nothing on a divergence: directory, writers, word, heads, departure and revoke alike. An unanchored answer for a document whose open was anchored changes nothing either.

---

### W202 (medium): served heads that no walk validated clear a head under check

**Where:** `membership_log.rs:413-425` (`ServedHeads::of`), `:459-469` (`holds`); `withheld.rs:162-183` (`served`); `websocket.rs:1406-1426`.

**Rule broken:** `SYNC-207`, `SYNC-220`, `SYNC-224`.

**Attack path:**
1. Alice revokes Eve with entry R and rotates to K2. The relay hides both from Bob.
2. A frame of Alice reaches Bob with the head after R, and R as its proof. Bob asks for the log and holds his edits.
3. The relay answers with the old log, one entry that fails the walk, then R at its true index. A row of Bob's that does not unwrap does the same job.
4. The walk stops before R, so it holds no revoke and nothing is awaited.
5. The served heads are the digests of every decoded entry, valid or not. They hold the head, so the check clears and the held edits leave under K1.

**Impact:** the relay keeps forwarding frames to Bob, so no partition shows, and it still clears the one check meant to catch it. Eve reads Bob's edits. This is not the limit "A held edit can leave under a retired key": there the revoke is walked and the `SYNC-224` wait starts.

**Fix:** clear a head under check only against heads the valid walk passes through (`walked_through`). Keep the served heads for the head this client signs into its own frames.

---

### W203 (medium): the author of a snapshot can mint its own confirmer

**Where:** `membership_log.rs:189-197` (`Confirmers::of`), `:971-978` (an editor invites any absent user); `snapshot_trust.rs:80-100`.

**Rule broken:** the intent of `AUTHOR-030` and audit `N31`. The row as written is met; the design leaves this open.

**Attack path (one editor, honest relay):**
1. Mallory, an editor, invites a second account of hers as viewer. The log binds that key to a member.
2. She seals a snapshot that holds operations under Alice's PeerID, uploads it and confirms it as author.
3. Her second account signs the confirmation. It is a key the log bound, and it is not the author's key.
4. Every cold open adopts that snapshot whole.

A removed member's key confirms too, since the set keeps every key ever bound; the spec states that choice.

**Impact:** forged operations read as Alice's on every cold open, with no help from the relay.

**Fix:** do not count a confirmer whose binding the snapshot's author signed. Better, count only a confirmer that a member other than the author added, or that held a writing role before the snapshot's horizon.

---

### W204 (low): a sign-out leaves session trust and keys in the worker

**Where:** `auth.rs:547-565` (`handle_logout`); fields at `lib.rs:1460`, `:1504-1508`, `:1523`, `:1433`.

**Rule broken:** `SYNC-235` and `SYNC-237` (roots "for the session"), `ZK-019` hygiene. The log line says "all keys zeroized".

**What stays:**
- `murmure_roots` and `pinned_inviters`.
- `listed_keys`, which holds a `DocumentKey` per listed channel.
- `pending_pre_session_updates`, which holds plaintext deltas, and `held_outbox`.
- `self_join_logs`, `listed_channels`, `roster_logs`, `typed_opens` and `user_id`.

A task in flight also lands after the sign-out: `OpenDocumentReady`, `KeyRotated` and `DebtSettled` each put a document key back, since no session generation makes them stale.

**Path:** A signs out and B signs in on the same worker, which `handle_sign_out` plans for. A Murmure open of B with no link passes `SYNC-235` on a root A's session vouched. A buffered delta of A is sealed under B's signing key if B opens that document.

**Fix:** clear every session map in `handle_logout`, and stamp spawned tasks with a session generation that the event handler checks.

---

### W205 (low): presence keeps sealing under a key the worker already holds edits for

**Where:** `updates.rs:1989-2034` (`handle_send_ephemeral`).

**Rule broken:** the intent of `SYNC-128`, `SYNC-220` and `SYNC-224`.

**Path:** a departure or a hidden revoke is known, so each edit waits unsealed. A presence sample reads only the log gate. It is still sealed under the current key, which the leaver holds, and the relay forwards it.

**Impact:** the leaver keeps reading cursors and selections.

**Fix:** drop a sample while `rekey.owes()` or `head_watch.holds_edits()` holds.

---

### W206 (low): no size bound on what the relay or the object store sends

**Where:** `http.rs:1314-1321` (`fetch_bytes`); `websocket.rs:192-199` (the frame is copied whole before decode); `lib.rs:234-237` (the quarantine counts frames, not bytes; the same holds for `unvouched` and `refused`).

**Path:** a snapshot or media answer of several gigabytes, or large frames parked in the 1 000 quarantine slots, exhausts the worker's memory before any signature is read. I found no bound by search in `http.rs` and `media.rs`; I did not read the transport body.

**Fix:** refuse a body past a stated ceiling from its length header and while reading, cap a socket frame before the copy, and count the quarantine in bytes.

---

### W207 (info): an echo is trusted before its signature

**Where:** `updates.rs:79-115`.

A frame that names this client's key moves `local_clock`, confirms a sent delta by its signature, and can settle a release, all before any check. The relay can forge that frame. Each effect is a loss the relay can already cause, and the clock saturates rather than panics.

### W208 (info): a 404 on the members read opens a local-only owner copy

**Where:** `updates.rs:833-839`.

By design (`SYNC-040`, membership section 3). A relay that answers 404 for a shared document makes the client draw a fresh key and later sign a creation for that identifier. Nothing leaks: the relay holds no key. An open that carries a pinned inviter could refuse instead.

### W209 (info, outside this scope): the release gate reads the Trunk profile

**Where:** `scripts/trunk_post_build.py:420-422`; `justfile:340`.

In `ambre-sync` the development sign-in is correctly behind a non-default feature, and the build without it refuses `DevLogin`. The post-build gate fires only when `TRUNK_PROFILE` is `release`. `proof_build` builds the optimised `wasm-release` profile with `dev-login` and without `--release`, so the gate does not run on it. Whoever owns the build scope should rule on this.

---

### Checked and found sound

- **Order of an open:** the key identifier, then the signature, the commitment tag and the cipher; decompression is bounded and runs after the AEAD.
- **Snapshot blob:** held to its document, identifier and horizon before decryption, and to the author its trust names.
- **Key links:** bound to the document, the previous key and the head. A forged link costs an open and installs nothing; a closed-stretch link raised by a forged successor is then refused as superseded.
- **Log walk:** the anchor, position, digest chain, role clamp and owner-only re-invite hold. An unanchored copy is refused at an open.
- **Writing paths:** invite, revoke, settle, role change and channel re-key all walk from the footing.
- **Replay batch:** an update filed under another document is dropped (`SYNC-203`).
- **Presence:** the counter moves only once a sample opens.
- **Rotation roll back:** a rotation back to a key the cache or the chain reaches is refused (`SYNC-213`).
- **Passphrase and seed:** the strength floor runs before any derivation, the seed is sealed before it leaves, and a re-seal carries the signed proof.
- **Local rows:** sealed under a key derived per request, bound to their place.
- **`.ambre` file:** failures are uniform.
- **Logs:** no payload, key or token reaches a log line in the files read; the passphrase never sits in a request type.

### Not read

- `sharing.rs` 520 to 1290 (the invite, revoke and leave tasks) and its tests.
- `http.rs` outside 1200 to 1429: the transport, the token renewal, the response types.
- `auth.rs` 190 to 225 and 637 to 880 (`open_stored_seed`).
- `channels.rs` outside 224 to 265; `members.rs` and `space_rotation.rs` outside the excerpts cited.
- `documents.rs`, `media.rs`, `light.rs` (two excerpts only), `finalize.rs`, `fold.rs`, `cursors.rs`, `lifecycle.rs`, `converter.rs`, `worker.rs`, `error.rs`, `generation.rs`, `lib.rs` 1 to 223.
- Every test module, `tests/integration.rs`, `mock.rs`, `mock_relay.rs`, `fuzz.rs`.
- The call files, out of scope: `call.rs`, `call_client.rs`, `call_link.rs`, `call_roster.rs`.
- `ambre-spec-sync.md`, `ambre-spec-key-rotation.md`, `ambre-spec-sealed-envelope.md` and `ambre-spec-op-authorship.md` were read by search for the rows cited, not whole.
- The Logic side of the author check and of the snapshot verdict, which W201 effect 1 and W203 rely on, belongs to another scope; I took the audit's own description of `N19` and `N31` for what it does with the directory and an adopted snapshot.
## Security pre-audit, pass C: parsers of foreign files (`main` at `ed31f88`)

Nothing was run and nothing was modified: every finding is a read of the code, traced from a crafted file to the failure. The scope was **not** read whole; the "Not read" list at the end is long and matters as much as the findings.

### Summary

| Severity | Count | Findings |
| :-- | :-- | :-- |
| high | 0 | none |
| medium | 5 | W300, W301, W302, W303, W304 |
| low | 1 | W305 (eleven quadratic sites) |
| info | 2 | W306, W307 |

No finding crosses the zero-knowledge boundary or reaches the UI with unescaped content. All are denial of service against the session that imports the file, so none is rated high.

**Files read whole (26):**

| Crate | Files |
| :-- | :-- |
| `ambre-opc` | `lib.rs`, `xml.rs`, `odf.rs` |
| `ambre-docx` | `xml.rs`, `read.rs`, `rel.rs`, `time.rs`, `lib.rs` |
| `ambre-odt` | `xml.rs`, `read.rs`, `lib.rs` |
| `ambre-prose-form` | `link.rs`, `word_read.rs` |
| `ambre-pptx` | `read/mod.rs`, `read/xml.rs`, `read/graphic.rs`, `read/style.rs`, `read/odp.rs` |
| `ambre-xlsx` | `read.rs`, `xml.rs`, `lib.rs`, `formula.rs`, `ods/mod.rs`, `ods/read.rs` |
| `ambre-convert` | `engine/mod.rs`, `engine/html.rs` |

Read in part: `ambre-xlsx/src/styles.rs` (lines 330 to 424), `engine/prose/html_read.rs` (one grep). Specs: `ambre-spec-interop.md` whole, the blind audit whole, the other documents by grep for their rows.

### Findings

#### W300 (medium): a tracked deletion that names itself recurses without bound

- **Where:** `crates/ambre-odt/src/read.rs:913` (`removed`), reached from `:761` (`inlines`) and `:478` (`blocks_into`).
- **Rule broken:** `PROSE-IO-092` (a hostile OpenDocument text MUST fail as a typed error); `INTEROP-002`.
- **Path:**
  1. `content.xml` declares `<text:changed-region text:id="x"><text:deletion><office:change-info/><text:p><text:change text:change-id="x"/></text:p></text:deletion></text:changed-region>`.
  2. `read_changes` (`:362`) clones that paragraph into `Change.removed`.
  3. The body holds `<text:p><text:change text:change-id="x"/></text:p>`.
  4. `inlines` meets `text:change` and calls `removed`, which clones `change.removed` and calls `paragraph` on each block.
  5. That paragraph holds the same `text:change`, so `inlines` calls `removed` again. No guard marks a change as open.
- **Why the depth bound does not help:** `MAX_DEPTH` of `xml.rs` bounds the tree, and the cycle goes through the id map, not through nesting.
- **Impact:** a file under 1 KB overflows the stack of the Office module on every import. No error is answered. The blind audit states for `P66` that a lost converter thread is never ended and the fourth loss leaves the session with none.
- **Fix:** keep the set of change ids being expanded and treat a re-entry as an unknown element; or expand each deletion at most once.

#### W301 (medium): one deleted region is copied once per reference to it

- **Where:** `crates/ambre-odt/src/read.rs:928` (`change.removed.clone()`), `:481`, `:763`.
- **Rule broken:** `PROSE-IO-092` ("before it holds the memory it asks for"); the client memory row of `ambre-spec-budgets.md section 6`.
- **Path:**
  1. One `text:deletion` holds a paragraph of 1 MiB of text.
  2. The body holds 800 000 elements `<text:change text:change-id="x"/>`, about 36 bytes each, 29 MiB.
  3. Each reference clones the subtree and appends its paragraphs to the output.
- **Impact:** the reader asks for about 800 GB from a part under the 32 MiB bound; the pattern deflates to a few tens of kilobytes. The module runs out of memory instead of answering `TooLarge`.
- **Fix:** expand a deletion at its first reference only, or count output bytes against one budget for the whole read.

#### W302 (medium): the repeat bound of an ODT table holds per attribute, not per table

- **Where:** `crates/ambre-odt/src/read.rs:1177` to `:1185` (rows), `:1217` to `:1265` (cells), `:33` (`MAX_REPEAT`), `:722` to `:729` (`text:s`).
- **Rule broken:** `PROSE-IO-092`; the client memory row of `ambre-spec-budgets.md section 6`.
- **Path:**
  1. `<table:table-row table:number-rows-repeated="1024">` holds `<table:table-cell table:number-columns-repeated="1024">`.
  2. `table_parts` calls `row` 1024 times; `row` calls `self.blocks(cell, …)` inside the repeat loop, 1024 times.
  3. One table of about 150 bytes yields 1 048 576 cells, each with its own block list.
  4. A table nested in that cell multiplies again. Three levels cost 9 of the 256 XML levels and ask for 1024^6 cells.
- **Same defect, smaller factor:** `<text:s text:c="1024"/>` is 23 bytes and writes 1024 spaces, so a 32 MiB part yields about 1.5 GB of run text.
- **Impact:** a file under 1 KB holds the module until it runs out of memory or passes its deadline.
- **Fix:** read a repeated row or cell once and copy the result under a cell budget per document, as `ambre-xlsx` counts cells against `MAX_CELLS`; bound the total of `text:s`.

#### W303 (medium): a flattened deck bounds its items and its picture bytes, not its text

- **Where:** `crates/ambre-pptx/src/read/mod.rs:130` to `:145` (slide list), `:283` to `:295` (master drawn per slide), `:630` (`text`), `:61` and `:66` (the two bounds); `read/odp.rs:394` to `:404` for the OpenDocument twin.
- **Rule broken:** `TOILE-IO-162` (a deck whose masters, flattened onto every slide, pass the bound MUST be refused whole).
- **Path:**
  1. The master holds one plain shape, no placeholder, whose `a:t` is 1 MiB of text. `shared` reads the master once, so it counts once against the 64 MiB inflated bound.
  2. `p:sldIdLst` lists 2 000 `p:sldId`. They may all name one relationship id: nothing dedupes `slide_parts`. Each read of the slide costs a few hundred bytes of the inflated bound.
  3. Each `slide` calls `tree_items` on the master; `shape` builds a fresh `TextBody.text` of 1 MiB and counts one item.
- **Impact:** 2 000 items, far under `MAX_ITEMS` of 100 000, hold 2 GB of text. The comment on `MAX_PICTURE_BYTES` names this very multiplication for a logo; text has no such bound. `read_deck` then serializes the deck with postcard, a second copy.
- **Fix:** count the bytes of text (and of names, and of freeform path segments) of the flattened deck against a bound beside `MAX_PICTURE_BYTES`.

#### W304 (medium): each run copies the list of every comment thread still open

- **Where:** `crates/ambre-prose-form/src/word_read.rs:433` to `:438` (`plain` clones `self.open`), `:389` to `:394` (`CommentStart`).
- **Rule broken:** the client memory row of `ambre-spec-budgets.md section 6`, which `PROSE-IO-072` holds the package bounds under.
- **Path, as a `.docx`:**
  1. `comments.xml` holds 20 000 `<w:comment w:id="n"/>`, about 0.5 MiB. Each becomes a thread.
  2. `document.xml` holds 20 000 `<w:commentRangeStart w:id="n"/>` and no end, then 500 000 runs `<w:r><w:t>a</w:t></w:r>`, about 12 MiB.
  3. `runs` pushes each thread into `self.open`; nothing closes it.
  4. Every `IrRun` then carries a clone of the 20 000 entries.
- **Impact:** 10^10 entries, 40 to 80 GB by the size of `Pos<IrThread>`, from parts well under their bounds. An `.odt` reaches the same code with `office:annotation` elements that carry a name and no `annotation-end`.
- **Fix:** bound the threads open at once (Word nests a handful), or share the open set between consecutive runs instead of cloning it per run.

#### W305 (low): quadratic walks on counts the bounds leave free

Each site holds a module past its answer deadline from a part inside the 32 MiB bound; the effect is the lost converter thread of `P66`. None is one of the three sites `P66` names. No spec row was found that bounds reader time; the nearest are the conversion rows of `ambre-spec-budgets.md section 1`.

| Site | Crafted input | Cost |
| :-- | :-- | :-- |
| `ambre-docx/src/read.rs:898` | 200 000 fields opened and separated, never ended, then millions of `w:t` | `fields.contains` scans the whole stack per run child |
| `ambre-prose-form/src/word_read.rs:169` | a chain of n replies, each the child of the one before | `root_of` walks the chain per comment |
| `ambre-prose-form/src/word_read.rs:391` | n comment starts left open | `open.contains` per start |
| `ambre-pptx/src/read/mod.rs:1191` | n empty placeholders on a slide, m on its layout | `placeholder_shapes` is rebuilt per shape; an empty placeholder returns before `count()`, so `MAX_ITEMS` does not bound n |
| `ambre-pptx/src/read/mod.rs:153` | n `p:pic` whose `r:embed` names no relationship, m relationships | a full scan each; a dropped picture is not counted either |
| `ambre-pptx/src/read/mod.rs:645` to `:676` | a master whose `a:lvl1pPr` holds a large subtree | the subtree is cloned per text shape |
| `ambre-pptx/src/read/graphic.rs:201` | a chart part of n categories and n values | a linear search per value |
| `ambre-xlsx/src/read.rs:209` | n `xf`, each naming a distinct font | `dedupe_styles` compares each style with all before it; `styles.rs:379` bounds no count |
| `ambre-xlsx/src/read.rs:760` and `:789` | a shared formula master of 16 MiB, then 450 000 cells that name it | `formula::shift` lexes the whole master per cell; the 8 192 character cap (`formula.rs:104`) applies after, and a cell left empty is not counted against `MAX_CELLS` |
| `ambre-xlsx/src/ods/read.rs:880` | 500 000 `table-column`, then 500 000 empty cells in one row | `inherited_style` scans the column runs per cell |
| `ambre-odt/src/read.rs:1157` and `:1249` | covered by `P66` | not re-reported |

The `dedupe_styles` row rests on `read_font`, which was not read: it assumes two fonts of different sizes give two different styles.

**Fix:** index what each loop searches (a map by relationship id, by placeholder index, by category index, by style), cap the open fields, and refuse a shared master past the formula cap before it is stored.

#### W306 (info): unchecked arithmetic on numbers a file spells

- **Where:** `ambre-docx/src/time.rs:117` (`hours * 3600` on an offset such as `-9999999999999999:00`) and `:139` to `:145` (`era * 146_097` on a year near `i64::MAX`); `ambre-xlsx/src/read.rs:968` (the same on a `t="d"` cell); `ambre-xlsx/src/ods/read.rs:915` and `:916` (`table.row + cell.matrix.1 - 1` with a matrix span near `u64::MAX`).
- **Impact:** `Cargo.toml:419` to `:436` sets no `overflow-checks` on the release and wasm profiles, so a shipped build wraps and reads a wrong date or range. A debug or test build panics on the same file.
- **Fix:** `checked_mul` and `checked_add` at these sites, answering `None`.

#### W307 (info): the spreadsheet readers skip a document type where the three others refuse it

- **Where:** `ambre-xlsx/src/xml.rs:124`, `ambre-xlsx/src/ods/mod.rs:137`; the two streamed readers (`read.rs:567`, `ods/read.rs:706`) ignore it the same way.
- **Rule:** `TOILE-IO-163` and `PROSE-IO-092` make a declared document type a refusal. My grep found no such row in the Trame import spec; the lines it omitted as too long were not read.
- **Impact:** none found. `quick-xml` expands no declared entity, and `push_reference` refuses any entity past the five predefined ones.
- **Fix:** refuse `Event::DocType` in the four readers, so the five codecs hold one rule.

### Checked and found sound

- **Zip bombs.** `Archive::part` reads one byte past the cap while it inflates and never trusts the header size; the 32 MiB and 64 MiB bounds count every read, a repeated read included; `MAX_PARTS` holds on open.
- **Package names.** `PartName::resolve_from` refuses a climb out of the package, reads a backslash as a slash and compares in lower case. A duplicate entry resolves to the last one, in one place.
- **Encrypted and legacy files.** Refused before any read.
- **XML.** `ambre-opc`, `ambre-docx`, `ambre-odt` and `ambre-pptx` refuse a document type, resolve the five predefined entities alone, and build their trees without recursion under a depth of 256 (64 in `ambre-xlsx`).
- **External references.** A relationship marked external stays text; no reader fetches one. A linked picture is reported and dropped. ODT picture paths with a scheme, a leading slash or `..` are refused.
- **Macros.** Named in the report, never inflated.
- **Links.** A Word or ODT link keeps its address as a `LinkUrl`; `ambre_prose_form::link::openable` passes http, https and mailto alone, reading the scheme as a browser does.
- **Slicing.** The direct slices of `placed` in `odp.rs:1334` to `:1377` cut at ASCII parentheses. `decode_name`, `decode_ooxml_escapes`, `points` and the formula lexer use `get` or split at found ASCII offsets.
- **Bounded chains.** Word style chains stop at 16, ODP style chains at 32, a comment parent loop at the comment count.
- **Formulas.** Both dialects cap at 8 192 characters; the lexer is linear.
- **CSV reader.** One record buffer, a room in records and fields, and a guard against a reader that does not advance.

### Not read

- **`ambre-pdf`:** the whole crate.
- **`ambre-convert`:** `engine/board.rs`, `json.rs`, `html_sink.rs`, `markdown.rs`, `svg.rs`, `webp.rs`, `png.rs`, `refusal.rs`, `spell.rs`, `picture/*`, `prose/*` (so the HTML walk, the Markdown reader and both writers), and every file outside `engine/`: `loader.rs`, `lib.rs`, `exports.rs`, `records.rs`, `images.rs`, `raster.rs`, `jpeg.rs`, `markup.rs`, `boards.rs`, `spelling.rs`.
- **`ambre-prose-form`:** `lib.rs`, `clip.rs`, `kinds.rs`, `zone.rs`, `report.rs`, `word_write.rs`.
- **`ambre-xlsx`:** `styles.rs` outside lines 330 to 424, `cellref.rs`, `model.rs`, `report.rs`, `book/*`, `write.rs`, `ods/formula.rs`, `ods/number.rs`, `ods/styles.rs`, `ods/write.rs`.
- **`ambre-docx`, `ambre-odt`, `ambre-pptx`:** every writer, `model.rs`, `report.rs`, `construct.rs`, `pml.rs`, `read_back.rs`, and every test file.
- **The applications:** none of the CSV, HTML or plain-text paste and import paths of `ambre-prose`, `ambre-trame` and `ambre-toile`. This is a whole third of the scope.
- **Consequences of the gaps above:**
  - SVG, board (Figma, Miro), Markdown, picture decoding and the HTML walk carry no verdict here.
  - Whether the module traps cleanly on W300 to W304 was not checked: `ambre-logic/src/converter_thread.rs` and the `ARCH-099` and `ARCH-128` rows were not read, so the impact rests on what `P66` states.
  - `XlsxError::Malformed` carries a part name and the parser's words; whether `refusal::xlsx` keeps them out of the answer (`INTEROP-004`) was not checked.
- **Audits:** `security-audit.md` was read by its headings alone, so a finding above may overlap one it holds; the blind audit of 2026-11-17 was read whole.
## Security pre-audit, pass D — bus and threads, applications, rest of `ambre-server`

Tree `main` at `ed31f88`, read-only. Nothing was built, run or modified. Every finding below comes from a read of the code; none was executed.

The scope was far larger than one pass can read whole. The relay half was read almost completely. The client half was read where the brief pointed (the Murmure author check, the Loro import sites, the page policy, sign-in, attachments), and most of the application crates were not read at all — see "Not read".

### Summary

| Severity | Count | Ids |
| :-- | :-- | :-- |
| high | 1 | W400 |
| medium | 2 | W401, W402 |
| low | 3 | W403, W404, W405 |
| info | 2 | W406, W407 |

**Files read whole**

- `ambre-server`: `s3.rs`, `oidc.rs`, `payload.rs`, `hot_tail.rs`, `compaction.rs`, `supervise.rs`, `state.rs`, `error.rs`, `routes/answers.rs`, `ws/handler/signature.rs`, `ws/handler/token_bucket.rs`, `secrets/{mod,seed,oidc,token}.rs`, `db/members.rs`, `db/documents.rs`, `db/limit.rs`, `db/orgs.rs`
- `ambre-crdt`: `lib.rs`, `ledger.rs`
- `ambre-murmure`: `logic/delta.rs`, `logic/reaction.rs`, `logic/attachment.rs`
- `ambre-app`: `host/web_embed.rs`, `index.html`

**Files read in part**

- `ambre-server`: `redis_ops.rs` (1-330, 700-1140, 1282-1980), `routes/channels.rs` (255-750: close, lease, finalize, fold, amendments, thread), `db/channels.rs` (340-1160), `ws/handler/message.rs` (170-430)
- `ambre-logic`: `lib.rs` (440-770), `data.rs` (1100-1430), `snapshot_check.rs` (1-190)
- `ambre-murmure`: `logic/channel.rs` (1700-1840, 2190-2370), `channel/amending.rs` (262-530), `channel/folding.rs` (395-555), `logic/schema.rs` (510-580)
- `ambre-app`: `signin.rs` (1-215), `lib.rs` (1215-1310), `host/murmure.rs` (30-140)
- `scripts/trunk_post_build.py` (150-390)

### Findings

#### W400 — high — a duplicate message id lets a member edit or delete another member's message

- **Where:** `ambre-murmure/src/logic/delta.rs:249-291` (`check`, no rule on the `id` key or the list position), `:293-303` (an insert at any position passes), `:415-426` (`author_in`, first match in list order), `:367-375` (`check_amendments`); `ambre-murmure/src/logic/channel/amending.rs:486-514` (`overlay`); `ambre-murmure/src/logic/channel/folding.rs:496-548` (`fold_into`).
- **Rule broken:** `MURMURE-253` (an amendment of another author's message is refused), `MURMURE-089` and `MURMURE-085` (a delete or an edit of another author's message is refused), `MURMURE-002` (the identity is minted by the peer that creates the message).
- **Path:**
  1. Alice posts a message with identifier `M` in the active segment.
  2. Mallory, a writer of the channel, sends a signed delta that inserts a message map of her own at position 0 of `messages`, with `id = M`, `author = Mallory` and `deleted = true` to keep it out of sight. She can also write `id = M` on an earlier message of hers. The check passes: every operation is under her PeerID, the map names her as author, and nothing reads `id` or the position.
  3. The segment closes and is locked, and its amendments document is registered.
  4. Mallory sends an amendments delta that creates the entry `M`, with `deleted = true, by = author` or a body of her own. `check_amendments` asks `author_in(segment, M)`, which answers the first map of the list with that id: Mallory's. The delta passes at every receiver.
  5. `overlay` applies the entry to every live view whose id is `M`, so Alice's message reads deleted or carries Mallory's text, marked edited. `fold_into` later writes the same into the segment for good.
- **Impact:** any writer of a channel rewrites or deletes any other member's message once its segment locks, and the fold makes it permanent. Alice's own amendment of `M` is refused everywhere, because the check now names Mallory as its author. The same first-match lookup feeds `message_map` and `thread_in`.
- **Fix:** refuse a delta that writes an `id` the document already holds under another map, and refuse a write of `id` on a map the delta did not create. Bind an amendment to the creating operation of the message rather than to its id text. Have `overlay` and `fold_into` apply an entry only to the map whose author signed it.

#### W401 — medium — the author check admits any write to the `header` and `meta` roots, so one writer makes a channel document unopenable

- **Where:** `ambre-murmure/src/logic/delta.rs:307-308` (`check`: `ContainerID::Root { .. } => {}`), `:397-398` (the same in `check_amendments`); `ambre-murmure/src/logic/schema.rs:552-578` (`admit_with` checks kind and channel after the import); `ambre-murmure/src/logic/channel/amending.rs:301-305` (the amendments header must name its segment).
- **Rule broken:** `MURMURE-079` (two peers accept and refuse the same deltas), and `MURMURE-013` for the amendments case.
- **Path:**
  1. Mallory, a writer, sends a signed delta on a segment, a thread or an amendments document with one map operation on the root `header`: `channel` set to another id, `kind` changed, or `segment` changed or deleted on an amendments document.
  2. The check treats every root other than `messages`, `peers`, `amendments` and `threads` as holding nothing, so the delta passes and each live client imports it. No live client re-reads the header.
  3. Every later open imports the snapshot and the replay, then reads the header: `ForeignChannel`, `WrongKind` or `ForeignSegment`. The document is refused whole.
  4. An honest snapshot taken after the delta carries the poisoned header, so the refusal outlives the delta.
- **Impact:** one delta from any writer makes the active segment, a thread or an amendments document unreadable for every session that opens it afterwards. For an amendments document, later openers lose every edit and delete written after the lock, so a message its author deleted reads again in its original form. Clients that held the document before the delta keep working, so members see two different channels.
- **Not traced:** a write of `folded` and `folded_seq` in a segment header sends every later snapshot check through the fold branch (`channel/confirming.rs:48`). I did not trace the verdict of that branch. A write of the foundation `meta` schema version probably has the same effect as the header writes; I did not trace the version check.
- **Fix:** refuse in both checks any operation on a root the document kind does not list as member-writable. The header and the version are written once, by the creating delta, and never after.

#### W402 — medium — a finalize lease reads its horizon from the ephemeral store alone

- **Where:** `ambre-server/src/db/channels.rs:656-673` (`take_finalize_lease` stores what `lock` answers); `ambre-server/src/redis_ops.rs:1357-1380` (`lock_segment`: `tonumber(GET seq) or 0`), `:1385-1402` (`lock_segment_now`, the same); `ambre-server/src/routes/channels.rs:378-383` (`lock_retired_segments` → `record_lock`). `take_fold_lease` at `db/channels.rs:939-941` takes the larger of the store's number and `max(updates.seq)`; the finalize path does not.
- **Rule broken:** `SRV-114` (the register records the lock and its horizon for good), `SRV-116` and `SRV-117` (the final snapshot covers the horizon exactly).
- **Path:**
  1. A segment holds deltas 1 to N and is closing.
  2. The ephemeral store is reset, as in W68. The counter `doc:{id}:seq` is gone until a subscribe reseeds it (`message.rs:215-233`).
  3. A client asks the finalize lease before any socket subscribes to that segment again. The Murmure runtime asks from a timer (`runtime/segments.rs:117-126`); I did not confirm the order against a subscribe. A rotation's `lock_retired_segments` reaches the same code with no client at all.
  4. The script answers 0. The register writes `horizon = 0`, and no later request rewrites a horizon that is set.
  5. The client "holds every delta up to the horizon" at once, whatever it has loaded, and commits a snapshot at `covered_seq = 0`. The relay accepts it and the segment is `final`.
- **Impact:** a segment is finalized on a snapshot that claims to cover nothing, built from a copy that may hold only part of the segment, while the register holds N deltas. I did not trace what the trim then drops, so the loss is not proven; the wrong horizon is. If the client refuses to label its snapshot 0, the segment stays closing for ever, and `SRV-196` then refuses every later close of the channel.
- **Distinct from W68:** this is not a restart of numbering on a trimmed document. The delta rows are present and the relay still records the wrong number.
- **Fix:** take the horizon as the larger of the store's counter and `max(seq)` of the delta rows, inside the lease transaction, as the fold lease does. The same for `lock_segment_now`.

#### W403 — low — the sign-in follows any `https` address the relay names

- **Where:** `ambre-app/src/signin.rs:69-71` (`is_followable`: scheme only), `:76-96`; `ambre-app/src/lib.rs:1270-1275`.
- **Rule broken:** `CHROME-095` as the file header cites it; the rule text was not read.
- **Path:** the user clicks the provider sign-in. The Sync worker asks the relay for the provider's address. A malicious relay answers `https://accounts.google.com.evil.example/...`. The page stores the state and navigates the top window there.
- **Impact:** the relay sends the user to a page of its choice at the moment the user expects a provider password prompt. The set of providers is closed (`OidcProvider`), so the client knows the one host each may use and does not check it.
- **Fix:** pin the authorization host per provider in the client, and refuse any other.

#### W404 — low — two more sites put an object key in an error text logged at warn level (the shape of `P41`)

- **Where:** `ambre-server/src/hot_tail.rs:126` (`"batch put {key}: {e}"`), logged at `:139` and `:156` at the warn level.
- **Rule broken:** `ZK-030`. The key holds the document identifier, and the gate reads types, so a text hides it.
- **Path:** the object store refuses a batch put during a tail move; the warn line prints the document id.
- **Why reported:** `P41` lists `s3.rs:405`, `:477`, `media.rs:218` and `error.rs:197`. A fix at those four sites leaves this one.
- **Fix:** carry the key in a typed field the gate can see, or drop it from the text, at every `AppError::S3` construction.

#### W405 — low — a rotation clears the debt without a wrapped copy for every member (database layer)

- **Where:** `ambre-server/src/db/members.rs:740-797` (`rewrap_members`).
- **Rule broken:** `SRV-167` in spirit: it refuses a rotation that names too few members, and does not hold that each named member receives a copy.
- **Path:** a rotation names members `{A, B, C}` and carries keys for `{A}` alone. The check `current ⊆ members` passes, the link is appended, `rotation_owed` is cleared, and B and C keep a wrap of the retired key.
- **Impact:** B and C lose the document from that rotation on. The rotator is a writer, so this is a member-level denial, not a disclosure.
- **Caveat:** `routes/keys.rs` belongs to the first pass and was not read here. If the route already refuses a key set that does not cover the members, this is defence in depth only.
- **Fix:** refuse inside `rewrap_members` when the set of `keys` is not exactly the current members with keys.

#### W406 — info — a contained panic still prints its message

- **Where:** `ambre-server/src/supervise.rs:24-42`. No `panic::set_hook` exists in `ambre-server/src`.
- **Rule:** `ZK-006`, as the comment on `contained` states it.
- **Fact:** `contained` and `catch_panics` keep the panic message out of the `tracing` line. The default hook of the standard library has already written the message to stderr before the unwind is caught. A panic that formats a value therefore reaches the same log stream.
- **Fix:** install a panic hook at start that prints the location alone.

#### W407 — info — the provider client has no deadline and follows redirects

- **Where:** `ambre-server/src/oidc.rs:166` (`reqwest::Client::new()`).
- **Fact:** the discovery, the code exchange and the key fetch run with no timeout and with the default redirect policy. The provider crate's own documentation asks for no redirects. The only issuer is Google's fixed address, so no attacker path was found: a stalled provider holds callback requests open.
- **Fix:** build the client with a timeout and `redirect::Policy::none()`.

### Checked and found sound

- **Object store:** every key is built from typed ids; `snapshot_id_of_key` accepts the canonical form alone; the content hash is 64 lowercase hex; a presigned link is capped at 15 minutes and signs `Content-Length`; the public endpoint keeps the signed path and query.
- **Flusher:** per-document lock with a token; the trim counts stored and dead-lettered entries only; a poison entry is logged by length and number.
- **Payload type:** no `Debug`, no `Display`; a batch refuses a negative number.
- **Signature check:** length-framed transcript, strict Ed25519, one function for updates and ephemerals.
- **Secrets module:** refresh and sign-in state keys are hashes; the seed text is wiped; no sink returns a bare string.
- **OIDC:** PKCE, nonce, state taken once; the identity token is verified against the provider keys.
- **Errors:** infrastructure causes never reach the caller.
- **Members and documents:** last-owner checks run under row locks; the log position is checked under the document lock; `document_claim` holds `N38`; the idempotent create never moves an owner.
- **Segment, fold and thread handlers:** each checks a writing role on the channel's `meta`, the caller's ownership of the registered document, and the channel of the segment inside the transaction. A lease commit needs the live lease and a snapshot of that segment at the horizon.
- **Page policy:** no `unsafe-inline`, no `unsafe-eval`, digests for the inline texts, `object-src 'none'`, `base-uri 'none'`, `form-action 'none'`, and `frame-ancestors 'none'` by header. Workers take their policy from their own answers.
- **Web embeds:** sandbox without `allow-same-origin` or top navigation, empty permissions, no referrer.
- **Attachments:** a received file opens only as a known raster type, and otherwise downloads as an octet stream; none runs in the page's origin.
- **Sign-in return:** the state is compared in constant time against the one this tab stored, and the code leaves the address bar.
- **Bus:** no token and no key type appears in a UI-bound event; passphrases and recovery words travel in wiping types.
- **Delta author check (Trame, Toile, Prose):** update blobs only, every PeerID equal to the signer's, read before any import; a refused delta is recorded as carrying nothing.

### Not read

- **Applications:** all of `ambre-trame`, `ambre-toile`, `ambre-prose`, `ambre-canvas` and `ambre-draw`. Only search hits were looked at.
- **Murmure:** everything outside the files listed above — the runtime, threading, closing, finalizing, confirming, mentions, search, watchers, the UI, and `check_fold`.
- **Bus and threads:** `ambre-bus`, `ambre-chrome`, `ambre-commands`, `ambre-client-core` (searches only); `ambre-logic` outside the three ranges listed, including `dispatch.rs`, `persistence.rs`, `store_ledger.rs`, `converter_thread.rs`, `spaces.rs` and `channels.rs`. The role gate was confirmed to exist (`data.rs:270`), not audited command by command.
- **`ambre-crdt`:** `history.rs` and its submodules, `session.rs`.
- **Relay:** `redis_ops.rs` 330-700 (first pass), 1140-1282 and past 1980; `routes/channels.rs` outside 255-750 (listing, create, direct message, delete, join); `db/channels.rs` outside 340-1160; `secrets/tests.rs`; `hot_tail/write_cost.rs`.
- **Specs:** `ambre-spec-architecture.md` not read; `ambre-spec-message-bus.md` by its contract rows only; `ambre-spec-zero-knowledge.md` lines 438-526 not read.
- **Clipboard, local store, what stays on disk after a sign-out:** not audited.
- **Loro itself:** its source is not in this container. Two things rest on it and stay unverified:
  - whether the blob metadata `authored_by` reads always agrees with what the import applies;
  - whether a JSON update may name an existing container as an inserted value. If it may, `delta.rs:260-266` takes the `reactions` value of a new message without checking that the map is the one that operation creates, and a member could alias another member's container. This is a lead, not a finding.