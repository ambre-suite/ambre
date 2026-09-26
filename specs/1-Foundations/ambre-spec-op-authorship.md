# AMBRE -- FEATURE: OPERATION AUTHORSHIP

**Type:** Horizontal (framework)
**Implementation:** design-only

> Who wrote each operation of a document. A delta is signed, and the signature names its author.
> The operations inside it carry a Loro PeerID that nothing checks. This document binds the two, so
> a member cannot write an operation under the PeerID of another member.

This document owns the binding between a signing key and a PeerID. What an application allows a
user to do with an operation is that application's rule, and it reads the binding from here.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `AUTHOR`, and it belongs to this document alone.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-001` | `INV` | A PeerID MUST be derived from the signing key of its author and a session nonce. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-002` | `INV` | A session nonce MUST be drawn fresh each time a client opens a document. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-003` | `INV` | The derivation domain of a PeerID MUST stay frozen at `ambre-peer-v1`. | this spec section 3; `ZK-009` | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-004` | `INV` | A document MUST carry a peer registry that maps each PeerID to its user and its session nonce. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-005` | `INV` | A registry entry MUST resolve to a user only when its PeerID derives from that user key and its nonce. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-019` | `INV` | A registry entry MUST carry the signature of its user over its document, its PeerID and its nonce. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-020` | `INV` | A registry entry MUST resolve to a user only when its signature verifies under that user's key. | this spec section 3; `AUTHOR-019` | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-006` | `INV` | An operation whose PeerID has no resolving registry entry MUST resolve to no user. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-007` | `CONC` | Two peers that receive the same deltas MUST accept and refuse the same deltas. | this spec section 3; derived | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Send a delta

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-008` | `POST` | A delta MUST carry the operations of the sending session alone. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-009` | `POST` | The first delta of a session MUST write the registry entry of its PeerID. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-010` | `POST` | The session nonce MUST travel inside the sealed payload of the delta. | this spec section 3; `ZK-007`; `SYNC-112` | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-011` | `FRAME` | Sending a delta MUST NOT write a registry entry for another PeerID, nor add a field the relay reads. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Receive a delta

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-012` | `FAIL` | Given a delta that carries an operation whose PeerID does not derive from its signer and nonce, the client MUST refuse the whole delta before import. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-013` | `POST` | An accepted delta MUST reach the Logic thread with the user identity of its signer. | this spec section 3; `SYNC-014` | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-014` | `FRAME` | Refusing a delta MUST NOT change the document, its registry or its export cursor. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Open from a snapshot

A snapshot is not signed operation by operation, so its author could place any operation in it.
Another member checks it against the signed deltas before anyone opens from it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `AUTHOR-015` | `POST` | A client that opens a document MUST start from its latest confirmed snapshot, and replay every delta after it. | this spec section 3; `SRV-126` | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-016` | `POST` | A client that holds the deltas a pending snapshot covers MUST check the snapshot against them, and confirm it when they match. | this spec section 3; `SRV-126` | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-017` | `FAIL` | Given a pending snapshot that differs from the deltas it covers, the client MUST withhold its confirmation and report the mismatch. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-021` | `POST` | A client MUST confirm its own snapshot only when every operation of another user past the reference snapshot comes from a delta that user signed. | this spec section 3; `SRV-126`, `SRV-130` | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-022` | `FAIL` | Given a snapshot its author confirmed, whose operations of another user past the reference snapshot differ from the deltas that user signed, the client MUST refuse it and open from the reference snapshot. | this spec section 3; `SRV-131` | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `AUTHOR-018` | `FRAME` | Checking a snapshot MUST NOT change the document. | this spec section 3 | not built | unbuilt -- lands with the `Operations bound to their signer` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: one hash per PeerID of a received delta, below any row of
  `ambre-spec-budgets.md`. An application check on top of it cites its own row. Checking a
  snapshot costs one import and one comparison. A self-confirmed one compares only what others wrote.
* **Boundary.** the session nonce rides inside the sealed payload (`AUTHOR-010`), so the relay
  learns nothing beyond the `ZK-007` set. The registry is document content, sealed like the rest.
* **Verification.** 22 `unimplemented`: no code derives a PeerID today. Every row lands with one
  Story, and none holds until it ships.

---

## 2. WHAT EACH USER SEES

| Alice does | Mallory does | The document | What each user sees | Test |
|---|---|---|---|---|
| sends a message | nothing | one message, under the PeerID of Alice's session | the message, by Alice | none |
| nothing | signs a delta that edits Alice's message under the PeerID of Alice's session | unchanged: every client refuses the delta (`AUTHOR-012`) | the message as Alice wrote it | none |
| opens the document on a second device | nothing | a second PeerID and registry entry for Alice | both sessions resolve to Alice | none |
| nothing | writes a registry entry that maps a PeerID of Mallory to Alice | the entry is present and resolves to nobody (`AUTHOR-005`, `AUTHOR-020`) | Mallory's operations stay Mallory's | none |

---

## 3. MODEL

**CRDT containers.** one per document, the peer registry.

- `"peers"` (Map) -- from a `PeerId` to a `PeerEntry`. An absent key means the PeerID resolves to no user.

**The derivation.**

```text
nonce   = 128 random bits, drawn when the client opens the document
peer_id = first 64 bits of BLAKE2b(b"ambre-peer-v1" || signing_pubkey || nonce)
```

```rust
pub struct SessionNonce([u8; 16]);
pub struct PeerId(u64);                  // the Loro PeerID, derived and never drawn
pub struct PeerEntry {
    pub user: UserId,
    pub nonce: SessionNonce,
    pub signature: Signature,  // by `user`, over the document, the PeerID and the nonce
}
impl PeerId {
    pub fn derive(signing_key: &PublicKey, nonce: &SessionNonce) -> PeerId;
}
```

The client sets the PeerID on the Loro document before its first operation. A second device, a
second tab and a reopen each draw a new nonce, so each writes under its own PeerID. Two writers
never share one, which is the invariant a Loro document needs.

**Send a delta.**

1. The Logic thread exports the operations of its own session since the last export.
2. On the first delta of the session, it writes the registry entry of its PeerID in the same delta.
3. The Sync worker seals the nonce and the delta together, and signs the frame.

**Receive a delta.**

1. The Sync worker checks the signature and opens the payload (`ambre-spec-sealed-envelope.md`).
2. It forwards the delta, the nonce and the user its signer resolves to (`SYNC-014`).
3. The Logic thread reads the PeerIDs of the delta from the Loro blob metadata, with no import.
4. It recomputes each PeerID from the signer key and the nonce. One mismatch refuses the delta.
5. It imports the delta. An application check on the operations, when one exists, runs just before.
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

**Code.** not built. The export cursor it constrains is `crates/ambre-trame/src/logic/sync.rs`.

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
| A snapshot is confirmed by another member | trusting the snapshot whole | its author could write any operation under the PeerID of another member, and every cold open would believe it |
| A snapshot is confirmed by another member | a signature on each operation | it would sign every keystroke, and the delta already carries one signature |
| A registry entry is signed by its user | dating each operation against the membership log | an operation carries no trusted time, and a log position per operation would weigh on every delta |
| An author alone confirms its own snapshot, checked after the fact | no snapshot until another member confirms | a document shared once and then edited alone would pile up deltas for ever, and replay them all at each open |
| An author alone confirms its own snapshot, checked after the fact | a purge after a delay, the snapshot marked unverified | a forged snapshot would stay the only record, and a mark does not tell the reader what was forged |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** Every client draws a random PeerID, and no check reads the PeerIDs of a
  delta.
* **The Loro calls are not proven.** The design assumes that Loro reads the PeerIDs of a blob with
  no import. A prototype confirms it before the first row ships.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a frame is sealed and signed, and what its payload carries | `ambre-spec-sealed-envelope.md` |
| Which user a signing key resolves to | `ambre-spec-membership.md` |
| What the relay may learn | `ambre-spec-zero-knowledge.md` |
| What a user may do with an operation, for example edit only their own message | the application that owns the gesture |
