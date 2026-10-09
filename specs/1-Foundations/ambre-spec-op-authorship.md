# AMBRE -- FEATURE: OPERATION AUTHORSHIP

**Type:** Horizontal (framework)
**Implementation:** partial -- every delta is checked against its signer before import. Nothing yet
checks a snapshot against the deltas it covers.

> Who wrote each operation of a document. A delta is signed, and the signature names its author.
> The operations inside it carry a Loro PeerID. This document binds the two, so a member cannot
> write an operation under the PeerID of another member.

This document owns the binding between a signing key and a PeerID. What an application allows a
user to do with an operation is that application's rule, and it reads the binding from here.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `AUTHOR`, and it belongs to this document alone.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-001` | `INV` | A PeerID MUST be derived from the signing key of its author and a session nonce. | this spec section 3 | `ambre_crypto::peer::tests::a_peer_id_derives_from_the_key_and_the_nonce` -- another key or another nonce derives another PeerID, and one pair is pinned to a value computed outside Rust | none | verified |
| `AUTHOR-002` | `INV` | A session nonce MUST be drawn fresh each time a client opens a document. | this spec section 3 | `ambre_sync::updates::tests::each_open_draws_a_new_peer_and_a_repeated_open_keeps_it` -- an open after a close derives another PeerID, and a second open of an open document keeps it; `ambre_crypto::peer::tests::two_drawn_nonces_differ` -- the draw | none | verified |
| `AUTHOR-003` | `INV` | The derivation domain of a PeerID MUST stay frozen at `ambre-peer-v1`. | this spec section 3; `ZK-009` | `ambre_crypto::peer::tests::a_peer_id_derives_from_the_key_and_the_nonce` -- the domain is asserted, and the pinned value goes red on any change to the derivation | none | verified |
| `AUTHOR-004` | `INV` | A document MUST carry a peer registry that maps each PeerID to its user and its session nonce. | this spec section 3 | `ambre_logic::authorship_tests::the_first_delta_carries_the_registry_entry_and_this_sessions_operations_alone` -- the entry lands in the `"peers"` map under its PeerID; `ambre_core::authorship::tests::an_entry_reads_back_and_a_truncated_one_does_not` -- the entry holds the user and the nonce | none | verified |
| `AUTHOR-005` | `INV` | A registry entry MUST resolve to a user only when its PeerID derives from that user key and its nonce. | this spec section 3 | `ambre_crypto::peer::tests::an_entry_resolves_to_the_user_who_signed_it_alone` -- an entry Mallory writes for Alice over a PeerID of her own resolves to nobody | none | verified |
| `AUTHOR-019` | `INV` | A registry entry MUST carry the signature of its user over its document, its PeerID and its nonce. | this spec section 3 | `ambre_crypto::peer::tests::an_entry_resolves_to_the_user_who_signed_it_alone` -- the entry verifies under its document, and under no other | none | verified |
| `AUTHOR-020` | `INV` | A registry entry MUST resolve to a user only when its signature verifies under that user's key. | this spec section 3; `AUTHOR-019` | `ambre_crypto::peer::tests::an_entry_resolves_to_the_user_who_signed_it_alone` -- a PeerID Mallory derives from Alice's public key, under a signature of her own, resolves to nobody | none | verified |
| `AUTHOR-006` | `INV` | An operation whose PeerID has no resolving registry entry MUST resolve to no user. | this spec section 3 | `ambre_crypto::peer::tests::an_entry_resolves_to_the_user_who_signed_it_alone` -- an absent entry resolves to nobody; `ambre_core::authorship::tests::an_entry_reads_back_and_a_truncated_one_does_not` -- an entry that does not decode reads as absent | none | verified |
| `AUTHOR-007` | `CONC` | Two peers that receive the same deltas MUST accept and refuse the same deltas. | this spec section 3; derived | `ambre_logic::authorship_tests::two_peers_given_the_same_deltas_keep_the_same_ones` -- two documents fed an honest, a forged and a mixed delta keep the same operations | none | verified |

### 1.2 Send a delta

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-008` | `POST` | A delta MUST carry the operations of the sending session alone. | this spec section 3 | `ambre_logic::authorship_tests::the_first_delta_carries_the_registry_entry_and_this_sessions_operations_alone` -- a document holding Alice's operations sends its own PeerID alone, on the first delta and the next | none | verified |
| `AUTHOR-009` | `POST` | The first delta of a session MUST write the registry entry of its PeerID. | this spec section 3 | `ambre_logic::authorship_tests::the_first_delta_carries_the_registry_entry_and_this_sessions_operations_alone` -- an unedited session sends nothing, and its first delta adds its entry | none | verified |
| `AUTHOR-010` | `POST` | The session nonce MUST travel inside the sealed payload of the delta. | this spec section 3; `ZK-007`; `SYNC-112` | `ambre_sync::pipeline::tests::the_session_nonce_is_sealed_inside_the_ciphertext` -- two nonces leave the readable part unchanged, and each opens back from the ciphertext | none | verified |
| `AUTHOR-011` | `FRAME` | Sending a delta MUST NOT write a registry entry for another PeerID, nor add a field the relay reads. | this spec section 3; `ZK-007` | `ambre_logic::authorship_tests::the_first_delta_carries_the_registry_entry_and_this_sessions_operations_alone` -- the first delta adds one entry and leaves Alice's as it was; `ambre_sync::pipeline::tests::the_session_nonce_is_sealed_inside_the_ciphertext` -- the readable part does not move with the nonce | none | verified |

### 1.3 Receive a delta

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-012` | `FAIL` | Given a delta that carries an operation whose PeerID does not derive from its signer and nonce, the client MUST refuse the whole delta before import. | this spec section 3 | `ambre_logic::authorship_tests::a_delta_carrying_an_operation_not_its_signers_is_refused_whole` -- a forged and a mixed delta, alone and in a batch, never land; `ambre_logic::authorship_tests::a_delta_its_signer_wrote_reaches_the_document` -- the honest delta lands, and a batch drops only the forged one | none | verified |
| `AUTHOR-013` | `POST` | An accepted delta MUST reach the Logic thread with the user identity of its signer. | this spec section 3; `SYNC-014` | `ambre_sync::updates::tests::an_opened_update_reaches_logic_with_its_nonce_its_peer_and_its_signer` -- the delta carries the member its signer resolves to, and a stranger's carries none | none | verified |
| `AUTHOR-014` | `FRAME` | Refusing a delta MUST NOT change the document, its registry or its export cursor. | this spec section 3 | `ambre_logic::authorship_tests::a_delta_carrying_an_operation_not_its_signers_is_refused_whole` -- the document's version vector is unchanged, nothing is sent, and the next edit carries this session alone | none | verified |

### 1.4 Open from a snapshot

A snapshot is not signed operation by operation, so its author could place any operation in it.
Another member checks it against the signed deltas before anyone opens from it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-015` | `POST` | A client that opens a document MUST start from its latest confirmed snapshot, and replay every delta after it. | this spec section 3; `SRV-126` | `ambre_sync::updates::tests::a_held_snapshot_is_checked_once_the_replay_is_done` -- the open starts from the reference and replays before anything else; `ambre_logic::authorship_tests::an_adopted_snapshot_moves_the_horizon` -- the adopted snapshot answers for the deltas below its horizon | none | verified |
| `AUTHOR-016` | `POST` | A client that holds the deltas a pending snapshot covers MUST check the snapshot against them, and confirm it when they match. | this spec section 3; `SRV-126` | `ambre_logic::authorship_tests::a_pending_snapshot_is_answered_and_never_imported` -- the snapshot holding the covered delta matches; `ambre_logic::snapshot_check::tests::one_history_cut_two_ways_holds_the_same_operations` -- the same operations match however Loro cut them; `ambre_logic::snapshot_check::tests::a_gap_in_the_covered_deltas_leaves_the_snapshot_unchecked` -- a gap is never read as a match | partial -- the confirmation is posted by a browser-only helper no host test drives; lands with the `The Sync request policy on the host` Story (`ambre-roadmap.md` E9) | partial |
| `AUTHOR-017` | `FAIL` | Given a pending snapshot that differs from the deltas it covers, the client MUST withhold its confirmation and report the mismatch. | this spec section 3 | `ambre_logic::authorship_tests::a_pending_snapshot_is_answered_and_never_imported` -- a forged operation is a mismatch; `ambre_logic::snapshot_check::tests::a_snapshot_omitting_a_covered_delta_is_a_mismatch` -- so is a covered delta left out | partial -- the report is posted by a browser-only helper no host test drives; lands with the `The Sync request policy on the host` Story (`ambre-roadmap.md` E9) | partial |
| `AUTHOR-021` | `POST` | A client MUST confirm its own snapshot only when every operation of another user past the reference snapshot comes from a delta that user signed. | this spec section 3; `SRV-126`, `SRV-130` | `ambre_logic::authorship_tests::a_delta_carrying_an_operation_not_its_signers_is_refused_whole` -- no unsigned operation reaches the document a snapshot is built from; `ambre_sync::updates::tests::a_held_snapshot_is_adopted_only_when_its_extras_resolve_to_its_author` -- nor through a snapshot it adopted | partial -- the confirmation is posted by a browser-only helper no host test drives; lands with the `The Sync request policy on the host` Story (`ambre-roadmap.md` E9) | partial |
| `AUTHOR-022` | `FAIL` | Given a snapshot its author confirmed, whose operations of another user past the reference snapshot differ from the deltas that user signed, the client MUST refuse it and open from the reference snapshot. | this spec section 3; `SRV-131` | `ambre_sync::updates::tests::a_held_snapshot_is_adopted_only_when_its_extras_resolve_to_its_author` -- a mismatch, or an extra signed by another key, leaves the document on its reference; `ambre_logic::snapshot_check::tests::operations_only_the_snapshot_holds_are_named_with_their_entry` -- each extra reaches Sync with its registry entry; `ambre_sync::updates::tests::a_held_snapshot_is_checked_once_the_replay_is_done` -- the held snapshot is checked only once the replay is done | none | verified |
| `AUTHOR-023` | `POST` | A mismatch report MUST be signed by its reporter, and name the snapshot and its author. | this spec section 3; `SRV-133` | `ambre_server::routes::tests::a_signed_report_drops_a_pending_snapshot_alone` -- the relay refuses a report signed over another author, and acts on one signed over the right one; `ambre_protocol::snapshot_report::tests::the_transcript_names_the_document_the_snapshot_and_the_author` -- what is signed changes with each of the three | none | verified |
| `AUTHOR-018` | `FRAME` | Checking a snapshot MUST NOT change the document. | this spec section 3 | `ambre_logic::snapshot_check::tests::the_check_leaves_the_document_unchanged` -- the version and the value are unchanged; `ambre_logic::authorship_tests::a_pending_snapshot_is_answered_and_never_imported` -- neither verdict reaches the document | none | verified |

### 1.5 The operations no session signs

The scaffolding of a new document and each schema migration write under fixed PeerIDs, which
derive from no key. No delta ever carries them, so every client must write them alike.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-024` | `INV` | A client MUST write the scaffolding and each migration of a schema version as the operations every other build writes for it. | this spec section 3; `SCHEMA-020` | `ambre_trame::logic::schema_shape::the_scaffold_a_build_writes_is_the_scaffold_frozen_for_its_version` -- the scaffolding's identity against `schema/scaffold.txt`; `ambre_toile::logic::state::schema_shape::the_scaffold_a_build_writes_is_the_scaffold_frozen_for_its_version` -- the same for Toile; `ambre_prose::logic::schema_shape::the_scaffold_a_build_writes_is_the_scaffold_frozen_for_its_version` -- the same for Prose; ci-gate -- `scripts/schema_freeze.py`, run by `just schema-freeze` in the CI `lint` job, which holds a published section of `scaffold.txt` append-only | partial -- what each migration step writes is not frozen; lands with the `Each migration frozen with its version` Story (`ambre-roadmap.md` E5) | partial |

### 1.6 A delta that waits on an operation no session signs

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-025` | `FAIL` | Given a delta that waits on a scaffolding or migration operation the client never wrote, the client MUST report the document as unreadable, never hold the delta for ever. | this spec section 3 | `ambre_logic::authorship_tests::a_delta_waiting_on_a_foreign_scaffold_reports_the_document` -- reported once, after the replay, and never for this build's scaffolding; `ambre_logic::authorship_tests::a_delta_past_a_gap_is_not_reported` -- a gap below the delta may still fill; `ambre_logic::waiting::tests::a_delta_waiting_on_one_not_yet_held_is_not_reported` -- a dependency that arrives settles the delta; `ambre_sync::updates::tests::a_held_snapshot_is_checked_once_the_replay_is_done` -- the open is ready only after the held snapshot's verdict | none | verified |

### 1.7 The open, before and after the session identity

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-026` | `POST` | An edit made before the session identity arrives MUST be applied under that identity once it arrives. | this spec section 3; `AUTHOR-008` | `ambre_logic::authorship_tests::an_edit_before_the_identity_is_applied_under_it` -- the held edit travels once, carrying the session's operations alone | none | verified |
| `AUTHOR-027` | `POST` | A document restored from the local cache MUST be shown before the session identity arrives. | this spec section 3 | `ambre_logic::authorship_tests::a_cached_document_shows_before_the_identity_and_is_rebuilt_under_it` -- a workbook and an emptied page show before the identity and save nothing, and then hold the cache under its own PeerIDs and the session's alone | none | verified |
| `AUTHOR-028` | `POST` | The edits a closed session left unsent MUST travel at the next open restored from the local store, under the PeerID that wrote them. | this spec section 3; `AUTHOR-002` | `ambre_logic::authorship_tests::an_earlier_sessions_unsent_edits_travel_with_its_nonce` -- after the replay, the span past the relay's copy travels with that session's nonce and its operations alone, and the new session is saved at its identity; `ambre_logic::authorship_tests::an_earlier_session_the_relay_holds_whole_is_forgotten` -- a session the relay holds whole sends nothing and is dropped from the store; `ambre_sync::updates::tests::an_unsent_delta_is_sealed_with_its_sessions_nonce` -- Sync seals it with the nonce it was given, and a resend keeps it; `ambre_logic::persistence::wasm_tests::sessions_are_kept_until_forgotten` -- wasm tier, the store keeps each session per document until it is forgotten | none | verified |
| `AUTHOR-029` | `FRAME` | An edit held until the session identity arrives MUST NOT reach the document under any other PeerID. | this spec section 3; `AUTHOR-008` | `ambre_logic::authorship_tests::an_edit_before_the_identity_is_applied_under_it` -- before the identity, the document and Sync see nothing of the edit | none | verified |

* **Budget.** one row of `ambre-spec-budgets.md section 1`, the check of one received delta. It
  decodes the blob metadata without importing it, and compares one PeerID per author. An
  application check on top of it cites its own row. Checking a snapshot costs one import and one
  comparison.
* **Boundary.** the session nonce rides inside the sealed payload (`AUTHOR-010`), so the relay
  learns nothing beyond the `ZK-007` set. The registry is document content, sealed like the rest.
* **Verification.** 25 `verified`, 4 `partial`. Every delta is checked, and every
  row on it holds by a test. Nothing checks a snapshot yet, so its author can still place any
  operation in it.

---

## 2. WHAT EACH USER SEES

| Alice does | Mallory does | The document | What each user sees | Test |
|---|---|---|---|---|
| sends a message | nothing | one message, under the PeerID of Alice's session | the message, by Alice | none |
| nothing | signs a delta that edits Alice's message under the PeerID of Alice's session | unchanged: every client refuses the delta (`AUTHOR-012`) | the message as Alice wrote it | `ambre_logic::authorship_tests::a_delta_carrying_an_operation_not_its_signers_is_refused_whole` |
| opens the document on a second device | nothing | a second PeerID and registry entry for Alice | both sessions resolve to Alice | none |
| nothing | writes a registry entry that maps a PeerID of Mallory to Alice | the entry is present and resolves to nobody (`AUTHOR-005`, `AUTHOR-020`) | Mallory's operations stay Mallory's | `ambre_crypto::peer::tests::an_entry_resolves_to_the_user_who_signed_it_alone` |

---

## 3. MODEL

**CRDT containers.** one per document, the peer registry.

- `"peers"` (Map) -- from a PeerID, as 16 hexadecimal digits, to a `PeerEntry` as bytes. An absent
  key means the PeerID resolves to no user.

**The derivation.**

```text
nonce   = 128 random bits, drawn when the client opens the document
peer_id = first 8 bytes, little-endian, of BLAKE2b-512(b"ambre-peer-v1" || signing_pubkey || nonce)
entry   = Ed25519 signature over b"ambre-peer-entry-v1" || document || peer_id || nonce
```

A PeerID of `u64::MAX` is moved to the value below it, because Loro reserves that one.

```rust
pub struct SessionNonce(pub [u8; 16]);
pub struct PeerEntry {
    pub user: UserId,
    pub nonce: SessionNonce,
    pub signature: [u8; 64],   // by `user`, over the document, the PeerID and the nonce
}
pub fn derive_peer_id(signing_pub: &[u8; 32], nonce: &SessionNonce) -> u64;
```

**Sync derives, Logic compares.** The Sync worker holds the signing key and the directory, so it
derives every PeerID. The Logic thread holds the document, so it compares. Neither needs the
other's state.

**Open a document.**

1. The Sync worker draws the nonce of the session, or keeps the one an open document holds.
2. It derives the PeerID, signs the registry entry, and sends both to the Logic thread.
3. The Logic thread sets the PeerID on the document and writes the entry at once.
4. Until the PeerID arrives, the Logic thread holds each edit, applied under the PeerID once it lands
   (`AUTHOR-026`).
5. A local restore in that wait is shown at once and kept. A restore may write, and a write under
   the provisional PeerID never travels. So the PeerID rebuilds the document from a fresh state and
   runs the restore again (`AUTHOR-027`). The Sync worker sends the PeerID before it fetches, so
   the rebuild drops nothing from the relay.
6. The local store saves the PeerID and its nonce at the identity, before any operation under it.
7. Once the relay's replay is done, each earlier session in the store sends what it wrote past what
   the relay is known to hold. Its own nonce seals it. A session the relay holds whole
   is dropped from the store (`AUTHOR-028`). What the relay holds is a lower bound, so an operation
   may travel twice, and an import skips it.

A second device, a second tab and a reopen each draw a new nonce, so each writes under its own
PeerID. Two writers never share one, which is the invariant a Loro document needs.

**Send a delta.**

1. The Logic thread exports the operations of its own PeerID since the last export.
2. The first delta of the session carries the registry entry, written at the open. The entry alone
   never sends, and the undo stack never sees it.
3. The Sync worker seals the nonce and the delta together, and signs the frame.

**Receive a delta.**

1. The Sync worker checks the signature and opens the payload (`ambre-spec-sealed-envelope.md`).
2. It derives the PeerID from the signer key and the nonce of the body.
3. It forwards the delta, the nonce, that PeerID and the user its signer resolves to (`SYNC-114`).
4. The Logic thread reads the PeerIDs of the delta from the Loro blob metadata, with no import.
5. One PeerID other than the derived one refuses the whole delta, and so does a blob that is not
   an update. The log names the document and the user, never the payload.
6. It imports the delta. An application check on the operations, when one exists, runs just before.
   Murmure carries its deltas as Loro JSON updates, so its check reads them with no document.

**Resolving a PeerID to a user.** A reader looks the PeerID up in the registry and recomputes the
hash from the entry. A match resolves to the user of the entry. A mismatch or an absent entry
resolves to nobody. The registry survives a snapshot, where the signed deltas do not.

**Open from a snapshot, in steps.** A snapshot is pending until a member other than its author
confirms it (`SRV-126`).

1. A cold open starts from the latest confirmed snapshot, and replays every delta after it
   (`AUTHOR-015`). Every one of those deltas passes the check above.
2. A client that now holds the deltas a pending snapshot covers imports the snapshot on the side.
3. It compares the operations of the snapshot with its own, up to the horizon of the snapshot.
4. It confirms a match (`AUTHOR-016`), and reports a mismatch (`AUTHOR-017`).

The next member to open cold is the natural confirmer: it has just replayed the very deltas the
snapshot covers.

**A registry entry is signed by its user** (`AUTHOR-019`). The PeerID is a public hash, so anyone
could write an entry that derives correctly for another user. The signature is what only that user
can make. A snapshot can therefore attribute an operation only to a user who wrote in the document
(`AUTHOR-020`). So an author cannot forge the work of a member invited later.

**An author may confirm its own snapshot** (`AUTHOR-021`). The reference is the latest snapshot
another member confirmed, which the relay keeps (`SRV-131`). Past it, the snapshot may hold
operations of another user only where that user signed the delta, and the relay keeps those
deltas (`SRV-130`).

A reader checks it on open. It reads the operations of each other user past the reference, and
compares them with the deltas that user signed. A difference is a forgery: the reader refuses the
snapshot and opens from the reference (`AUTHOR-022`). The deltas kept are only what others wrote,
so they stop growing while one author writes alone (`SRV-132`).

**Why the check is deterministic.** It reads the delta, its signer and its nonce, and nothing
else: no arrival order, no local state. So every peer reaches the same verdict on the same delta
(`AUTHOR-007`).

**Code.**

* `crates/ambre-crypto/src/peer.rs` -- the nonce, the derivation, the signed entry and its resolution;
* `crates/ambre-core/src/authorship.rs` -- the registry map and the entry bytes;
* `crates/ambre-sync/src/updates.rs` -- the session identity per open, and the derived PeerID;
* `crates/ambre-logic/src/data.rs` -- the gate on edits, the check before import, and the unsent
  edits of earlier sessions;
* `crates/ambre-logic/src/persistence.rs` -- the sessions the local store keeps;
* `crates/ambre-trame/src/logic/sync.rs` -- the entry write and the export cursor, as in Toile
  and Prose.

---

## 4. SURFACE

The binding has no control of its own.

| Trigger | Behaviour |
|---|---|
| a delta arrives with a forged PeerID | nothing changes on screen, and the client logs the document and the signer, never the payload (`ZK-006`) |
| a document opens | the client draws a new nonce and a new PeerID for the session |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A PeerID derives from a key and a nonce | a PeerID equal to a hash of the signing key alone | two sessions of one user, on two devices or two tabs, would write under one PeerID, and a Loro document forbids two concurrent writers on one PeerID |
| A PeerID derives from a key and a nonce | a PeerID bound on first sight, with no derivation | the binding would depend on which delta a client receives first, and two clients could disagree |
| The registry is document content | a registry held and signed by the relay | the relay would become the authority on who wrote what, which the key model per member refuses |
| A forged delta is refused before import | an import followed by a filter at display | the filter detects and prevents nothing: a last-writer-wins value is already overwritten when the display reads it |
| One check per delta | a signature on each operation | a signature per operation multiplies the cost by the operation count, and the delta signature already covers every operation it carries |
| Sync derives the PeerID, Logic compares | the Logic thread derives it | the Logic thread would need the signer key of every delta and the hash, and it holds neither today |
| The registry entry is written at the open | the entry written on the first export | the entry would split the undo group of the first edit, and two edits would undo as two steps |
| A snapshot is confirmed by another member | trusting the snapshot whole | its author could write any operation under the PeerID of another member, and every cold open would believe it |
| A snapshot is confirmed by another member | a signature on each operation | it would sign every keystroke, and the delta already carries one signature |
| A registry entry is signed by its user | dating each operation against the membership log | an operation carries no trusted time, and a log position per operation would weigh on every delta |
| An author alone confirms its own snapshot, checked after the fact | no snapshot until another member confirms | a document shared once and then edited alone would pile up deltas for ever, and replay them all at each open |
| An author alone confirms its own snapshot, checked after the fact | a purge after a delay, the snapshot marked unverified | a forged snapshot would stay the only record, and a mark does not tell the reader what was forged |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A local restore is not checked.** It imports with no check, so whoever wrote the local store
  can still place any operation in it. A snapshot from the relay is checked (section 1.4).
* **No reader resolves a PeerID yet.** The resolution is built and tested, and no surface shows an
  author. The first is the message of Murmure.
* **A migration step is held by convention alone.** The scaffolding's identity is frozen per
  version, and what a migration step writes is not (`AUTHOR-024`). A step edited after its version
  shipped leaves a delta waiting, and the document is then reported unreadable (`AUTHOR-025`).
* **An edit held past its cap is dropped.** The cap is a row of `ambre-spec-budgets.md section 6`,
  and only a session opened before its unlock reaches it. A held edit naming a block the shown
  restore grew finds no such block after the rebuild, since the block regrows under another id.
* **Unsent edits wait for an open from the local store.** Only the boot open restores the cache, so
  a document reopened inside the page sends an earlier session's edits at the next boot open
  (`AUTHOR-028`). Edits left unsent before the store kept sessions never travel: no nonce was kept.
* **The relay history of an older build no longer opens.** Its deltas carry random PeerIDs.
  Nothing has shipped, so no migration is planned.
