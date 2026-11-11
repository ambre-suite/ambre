# AMBRE -- FEATURE: MACHINE ACCESS

**Type:** Horizontal (framework)

> A program reads or writes a sheet, a board, a text or a conversation only as a sealed client. The
> owner sees each program in the member list, with where it runs. The relay never opens what a
> program reads or writes.

Trame, Toile, Prose and Murmure share one answer to "give a program access". The relay cannot read a
document (`ZK-004`), so no server API can serve one. A program is a client, in one of three classes.
A member reads everything, a writer reads nothing, and an addressed machine reads only what a user
sends it. This document owns the three classes; each application maps a post onto its own
content.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MACH`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACH-001` | `INV` | A machine member MUST hold the document key through the same wrap as a human member. | `1-Foundations/ambre-spec-membership.md section 3` (`SYNC-010`) | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-002` | `INV` | A machine MUST reach a document only through the client library that seals every envelope. | `ZK-001`; this spec section 3 | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1). A library that let a caller skip the seal would put one plaintext message on the relay, and that message is the leak | unimplemented |
| `MACH-003` | `OBS` | The member list MUST mark each machine as a machine. | this spec section 4 | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-004` | `OBS` | The member list MUST show the host each machine declares. | this spec section 4 | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1). The declaration is the operator's word, and section 6 says why nothing can check it | unimplemented |
| `MACH-005` | `INV` | The relay MUST bucket the requests of a machine apart from those of the human members. | `1-Foundations/ambre-spec-server.md section 1` (`SRV-019`) | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1). A machine that polls in a loop would otherwise drain the bucket its owner types into | unimplemented |
| `MACH-006` | `INV` | An inbox writer MUST hold no key that opens the document. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-007` | `CONC` | Two members merging one inbox concurrently MUST apply each post once. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1). Both clients are online, both see the post, and neither knows the other merges | unimplemented |
| `MACH-008` | `INV` | An addressed machine MUST hold no key that opens an unaddressed envelope. | this spec section 3 | not built | unbuilt -- lands with the `Addressed machines` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-009` | `INV` | An addressed key MUST derive from the current document key. | this spec section 3 | not built | unbuilt -- lands with the `Addressed machines` Story (`ambre-roadmap.md` post-V1). A key derived from a retired document key would survive the rotation that removed a member | unimplemented |
| `MACH-010` | `INV` | An addressed machine MUST answer through the document inbox, as an inbox writer. | this spec section 3 | not built | unbuilt -- lands with the `Addressed machines` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Add a machine member

The owner invites a program as a member. It receives the document key, bound by its role.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACH-011` | `POST` | Adding a machine member MUST ask for the same fingerprint confirmation as a human invite. | `1-Foundations/ambre-spec-key-verification.md section 3` (`CHROME-073`) | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-012` | `POST` | A machine member MUST join as a viewer unless the owner picks another role. | derived | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1). A program that only reads, to export or index, is the common case, and a write role is a choice the owner states | unimplemented |
| `MACH-013` | `FRAME` | Adding a machine MUST change no other member's role. | derived | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-014` | `PRE` | The owner MUST see the declared host before any key is wrapped. | this spec section 4 | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1). The host is the operator's word, so refusing an empty one proves nothing; showing it before the key leaves is the one moment it informs a choice | unimplemented |

### 1.3 Remove a machine member

The owner removes the program. It keeps what it already read, and it opens nothing written after.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACH-015` | `POST` | Removing a machine member MUST rotate the document key. | `1-Foundations/ambre-spec-key-rotation.md section 1` (`SYNC-041`) | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-016` | `FRAME` | Removing a machine MUST NOT remove what it wrote. | derived | not built | unbuilt -- lands with the `Machine members` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.4 Post to an inbox

A registered writer drops a post into the inbox of a document. It cannot read the document or the
other posts.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACH-034` | `PRE` | A writer MUST take the inbox public key from the owner who registers it, never from the relay. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1). A relay that served a key of its own would open every post | unimplemented |
| `MACH-017` | `POST` | A post MUST be sealed to the inbox public key. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-018` | `POST` | A post MUST carry the signature of its writer. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-019` | `FRAME` | A post MUST NOT change the document before a member merges it. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1). The writer holds no document key, so it cannot seal a delta, and a post is a request, not an edit | unimplemented |
| `MACH-020` | `FAIL` | The relay MUST refuse a post signed by no registered writer of that inbox. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1). An open inbox is a spam channel that every member client must open and drop. The relay's registry is its own word, so this check filters spam and `MACH-033` is the one that holds | unimplemented |

### 1.5 Revoke an inbox writer

The owner revokes a writer. The writer never read anything, so nothing is re-keyed.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACH-021` | `POST` | The relay MUST refuse the next post of a revoked writer. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-022` | `FRAME` | Revoking a writer MUST NOT rotate the document key. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.6 Merge an inbox

A member client opens the pending posts and writes them into the document as ordinary edits.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACH-023` | `PRE` | A member without the right to edit MUST NOT merge a post. | `SYNC-016`; this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1). A viewer who merged would write through the inbox what its role forbids it to write directly | unimplemented |
| `MACH-024` | `POST` | A merged post MUST keep the identity of its writer in the document. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-025` | `FRAME` | A member MUST NOT delete a post before the relay acknowledges the merged edit. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1). A crash between the two steps would otherwise lose the post | unimplemented |
| `MACH-033` | `FAIL` | A post whose writer is missing from the sealed machine records MUST be set aside, whatever the relay accepted. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1). A relay that added a writer of its own to its registry would otherwise write into the document | unimplemented |
| `MACH-026` | `FAIL` | A post that fails to open or to parse MUST be set aside and reported, never merged. | this spec section 3 | not built | unbuilt -- lands with the `Write-only inboxes` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.7 Address an envelope to a machine

A user sends a command or a message to one machine. Only that envelope reaches it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACH-027` | `POST` | An envelope addressed to a machine MUST be sealed with the addressed key of that machine. | this spec section 3 | not built | unbuilt -- lands with the `Addressed machines` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-028` | `FRAME` | Addressing an envelope MUST seal no other envelope with the addressed key. | this spec section 3 | not built | unbuilt -- lands with the `Addressed machines` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-029` | `FRAME` | Addressing an envelope MUST NOT send the document key to the machine. | this spec section 3 | not built | unbuilt -- lands with the `Addressed machines` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-030` | `FAIL` | Addressing a removed machine MUST be refused before any sealing. | this spec section 3 | not built | unbuilt -- lands with the `Addressed machines` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.8 Remove an addressed machine

The owner removes the machine. It keeps the envelopes it already opened, and it receives no other.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACH-031` | `POST` | Removing an addressed machine MUST revoke its inbox writer registration. | this spec section 3 | not built | unbuilt -- lands with the `Addressed machines` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACH-032` | `FRAME` | Removing an addressed machine MUST NOT rotate the document key. | this spec section 3 | not built | unbuilt -- lands with the `Addressed machines` Story (`ambre-roadmap.md` post-V1). The machine never held the document key, and its addressed key opens no unaddressed frame | unimplemented |

* **Budget.** none of its own: a machine client carries the sync costs of any client. The request
  rate of a machine has no row in `ambre-spec-budgets.md` yet, and section 6 names it.
* **Boundary.** every path is sealed on the client (`ZK-001`), and no relay endpoint reads content
  (`ZK-004`). The machine flag, the declared host and the inbox registry are new relay knowledge,
  which section 6 names.
* **Verification.** 0 `verified`, 34 `unimplemented`. Nothing here is built.

---

## 2. WHAT EACH USER SEES

A machine is a peer like any other. The scenarios below turn on the class it belongs to.

| Alice does | A machine does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds an export program as a viewer | reads the sheet and writes a file outside Ambre | nothing new (`MACH-012`) | Alice sees the program in the member list, with its host (`MACH-003`, `MACH-004`) | none -- unbuilt |
| removes the export program | tries to read the next edit | the edit, under a rotated key (`MACH-015`) | the program opens nothing written after its removal | none -- unbuilt |
| registers a form as an inbox writer | posts a new row | the row, once Alice's client merges it (`MACH-019`) | Alice and Bob see one row, signed by the form (`MACH-007`, `MACH-024`) | none -- unbuilt |
| addresses a command to a bot in Murmure | answers the command | the answer, merged from the inbox (`MACH-010`) | the bot reads the command and nothing else (`MACH-008`) | none -- unbuilt |

---

## 3. MODEL

**Containers.** none of its own. The inbox private key and the machine records live in the metadata
of the document, sealed under the document key (`ambre-spec-sealed-envelope.md`).

**The three classes.**

| Class | Holds | Reads | Writes | Removal costs |
| :--- | :--- | :--- | :--- | :--- |
| machine member | the wrapped document key | the whole document, its history included | what its role allows | a rotation (`MACH-015`) |
| inbox writer | its own signing key | nothing | posts, merged by a member | a registry update (`MACH-022`) |
| addressed machine | an addressed key, wrapped for it | the envelopes addressed to it | posts, as an inbox writer | a registry update (`MACH-032`) |

**The shapes.**

```rust
/// How a machine reaches one document. The class decides which key it holds.
pub enum MachineAccess {
    Member { role: Role },
    InboxWriter,
    Addressed,
}

/// One machine, as the members see it. `identity` is the machine's signing key.
pub struct MachineRecord {
    pub identity: PublicKey,
    pub access: MachineAccess,
    pub host: HostDeclaration,
}

/// Where the machine runs, as its operator declares it. Nothing verifies it.
pub struct HostDeclaration {
    pub operator: String,
    pub third_parties: Vec<String>,
}

/// One post, as the relay stores it: sealed to the inbox, signed by its writer.
pub struct InboxPost {
    pub post_id: [u8; 16],
    pub writer: PublicKey,
    pub sealed: Vec<u8>,
    pub signature: Signature,
}
```

**The client.** One Rust library, `ambre-client`, with no interface. It seals every envelope, and it
exposes no path that skips the seal (`MACH-002`). One feature per class keeps each build small:

| Feature | Pulls in | Serves |
| :--- | :--- | :--- |
| `inbox` | `ambre-core`, `ambre-crypto`, `ambre-protocol`, an HTTP client | an inbox writer |
| `addressed` | `inbox`, and a native socket | an addressed machine |
| `member` | `addressed`, Loro, and the crate of each application | a machine member |

An inbox writer and an addressed machine never open the document, so neither links Loro. Python and
TypeScript reach the same core through bindings; no other language implements the cryptography.

**A post, in steps.**

1. the writer holds the inbox public key the owner gave it at registration (`MACH-034`);
2. it builds the payload the application declares for the inbox;
3. it seals the payload to the inbox X25519 public key, with a fresh ephemeral key (`MACH-017`);
4. it signs the post id and the sealed bytes with its Ed25519 key (`MACH-018`);
5. the relay checks the signature against the registry of the inbox, and stores the post (`MACH-020`).

**A merge, in steps.**

1. a member client with the right to edit opens the inbox private key with the document key (`MACH-023`);
2. it checks each writer against the sealed machine records, and sets aside a stranger (`MACH-033`);
3. it opens each pending post, and sets aside one that fails to open or to parse (`MACH-026`);
4. it writes the post into the document, keyed by post id, with the writer identity (`MACH-024`);
5. it deletes the post once the relay acknowledges the edit (`MACH-025`).

A second merge of the same post finds its id in the document and writes nothing (`MACH-007`).

**The addressed key, in steps.**

1. a member derives the key with HKDF-SHA256 over the document key;
2. the info string is `ambre-addressed-v1`, followed by the signing key of the machine;
3. the member wraps the result for the exchange key of the machine, as a membership wrap;
4. after a rotation, a member derives and wraps again from the new document key (`MACH-009`).

An addressed envelope is not an edit. It travels on a stream of its own beside the oplog, and the
document never holds it. The oplog is sealed under the document key, which the machine lacks.

**Code.** not built -- the three classes land with their Stories (`ambre-roadmap.md` post-V1).

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| "Add a machine" in the share dialog | the class, the role and the host are asked; the host is shown and the fingerprint confirmed (`MACH-011`, `MACH-014`) |
| the member list | a machine badge and the declared host beside each machine (`MACH-003`, `MACH-004`) |
| the inbox settings of a document | register or revoke a writer by its public key (`MACH-021`) |
| a Murmure slash command, or a command palette entry in an editor | the envelope is addressed to one machine (`MACH-027`) |
| a post set aside | a notice names the writer and the reason (`MACH-026`) |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A machine is a sealed client | a REST API on the relay | the relay links no crate that can open a payload (`ZK-004`), so the API could serve only ciphertext. Giving it a key makes the relay a reader, which ends the zero-knowledge pillar |
| A machine is a sealed client | a bridge service that holds the document key and exposes webhooks | the bridge is a member that every integration trusts at once. One breach reads every document it bridges, and the owner sees one member where many programs read |
| A machine is a sealed client | scheduled scripts that run on the relay | the relay cannot run a script on content it cannot open |
| The client library seals every envelope | a library where the seal is optional | one caller that skips the seal writes plaintext to the relay, and the relay cannot tell a sealed envelope from plaintext |
| Three classes, the member class opt-in | every machine a full member | a program that only posts, or only answers a command, would read every edit and its whole history. Each such program would also cost a rotation on removal |
| The inbox is asymmetric | a symmetric inbox key shared with the writers | a writer holding the key could open every other post. Removing it would then cost a rotation of the inbox key |
| Access is per document | a key per range, per sheet or per page | a Loro document is one oplog under one key. Splitting it into several keys means several documents, and every cross-range formula or link breaks |
| The addressed machine answers through the inbox | an answer sealed with the addressed key | members would need a second merge path for the same kind of post |
| Filtering happens by key | filtering on the relay, by event kind | the relay cannot read the kind of a sealed envelope. Exposing the kind to filter on it is a new leak |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Every row is unbuilt.** No machine reaches a document today.
* **No budget row for the request rate of a machine.** `MACH-005` separates the buckets, and no figure
  sizes the machine one.
* **A third derivation domain.** The addressed key needs `ambre-addressed-v1` beside the two domains
  that `ZK-009` freezes. That row must widen before the Addressed machines Story lands.
* **New relay knowledge.** The machine flag, the declared host and the inbox registry lie outside the
  set `ZK-007` allows. Each needs a row in the boundary table, or a sealed form the relay stores blind.
* **A machine member reads the history.** The document key opens the whole oplog, edits made before
  the machine joined included. A shallow snapshot that would hide the older edits is not ruled.
* **The stream of addressed envelopes is not ruled.** Where the relay keeps it, and for how long,
  has no rule yet.
* **The host declaration is unverifiable.** It records what the operator says; nothing proves where
  the program runs.
* **The content a post maps to is per application.** A row in Trame, a card in Toile, a paragraph in
  Prose and a message in Murmure are four schemas. No application declares its inbox schema yet.
* **No native client exists.** `ambre-sync` runs in the browser worker only: its sockets and its HTTP
  calls go through the browser. A transport seam, with one browser and one native side, comes first.
* **The bindings and the publication are not ruled.** Publishing `ambre-client` publishes the crates it
  depends on. Their wire format then becomes a public contract, and no rule versions it yet.
* **The licence of the client is not ruled.** The workspace is `AGPL-3.0-only`, and few integrators
  link a copyleft library into a private program. A permissive licence for the client crates is open.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a document key is wrapped for a member, and what a role allows | `ambre-spec-membership.md` |
| When and how a document key rotates | `ambre-spec-key-rotation.md` |
| How a fingerprint is shown and confirmed | `ambre-spec-key-verification.md` |
| How an envelope is sealed | `ambre-spec-sealed-envelope.md` |
| What the relay may learn | `ambre-spec-zero-knowledge.md` |
| How the relay limits the request rate | `ambre-spec-server.md` |
| The figure for any rate or cost | `ambre-spec-budgets.md` |
| How a bot shows in a Murmure conversation | `5-Murmure/features/ambre-spec-murmure-bots.md` |
| What a post becomes in a sheet, a board or a text | each application's own feature document |
