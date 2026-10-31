# AMBRE -- FEATURE: DOCUMENT MACROS

**Type:** Horizontal (framework)
**Implementation:** design-only -- no document carries code today. Macros land with the "Document
macros", "Macro triggers", "Macro network access" and "Macro marketplace" Stories
(`ambre-roadmap.md` post-V1).

> A user adds a small program to a sheet, a board, a text or a conversation, and a button runs it. It
> runs on the machine of the user who asks, only after that user approves it, in a sandbox the
> browser seals. The relay stores it sealed.

A macro is TypeScript or JavaScript stored in a document and run by a member client, never by the
relay. Its publisher wrote it; its integrator is the member who put this version in this document.
It runs at full browser speed in a worker the browser isolates, and it edits through the commands of
the interface. A function a cell calls is not a macro: Trame owns it, as a named formula.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MACRO`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACRO-001` | `INV` | A macro MUST be stored sealed, like any other content of the document. | `ZK-001` | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-002` | `INV` | A macro MUST run only in a member client, never on the relay. | `ZK-004`; this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-053` | `INV` | A macro MUST reach only the commands its application declares open to macros. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A list of what is open fails closed on a new command; a list of what is barred, as `MACRO-005` alone would be, fails open | unimplemented |
| `MACRO-003` | `INV` | A macro MUST edit the document only through the commands the interface offers. | `1-Foundations/ambre-spec-commands.md section 2` (`UI-THREAD-019`) | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A second write path would bypass the role check and the undo history that the commands already carry | unimplemented |
| `MACRO-004` | `INV` | A macro MUST act with the lower of two roles: its integrator's and its running user's. | `SYNC-016`; `1-Foundations/ambre-spec-membership.md section 3`; this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). With the running user's role alone, an editor's macro run by an owner would act as an owner, which its integrator never was | unimplemented |
| `MACRO-005` | `INV` | A macro MUST NOT reach the commands that manage members, roles, sharing or keys, whatever it is granted. | `1-Foundations/ambre-spec-membership.md section 3`; derived | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). These commands change who can read the document, and no approval of a macro is informed enough to grant that | unimplemented |
| `MACRO-006` | `INV` | A macro MUST NOT delete the document. | derived | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-007` | `INV` | A macro MUST reach no key. | `1-Foundations/ambre-spec-architecture.md section 2` (`ARCH-004`) | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-008` | `INV` | A macro MUST reach no storage of the Ambre client. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). The local store holds every document the user opened, and a macro from one document must not read another | unimplemented |
| `MACRO-009` | `INV` | A macro MUST reach another document only through a picker its user operates. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A macro that named a document by id could copy, into a shared one, a document its integrator cannot open | unimplemented |
| `MACRO-010` | `INV` | The browser MUST block every network request of a macro, except to a domain its user approved for that version. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). Removing the network functions from the sandbox in code is no boundary: one function forgotten is a leak, and the browser has dozens | unimplemented |
| `MACRO-046` | `INV` | The sandbox MUST be served from a registrable domain other than the client's. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). Browsers split processes by site, and a subdomain of the client is the same site: the sandbox could share the process of the Sync worker and its keys | unimplemented |
| `MACRO-047` | `INV` | The client MUST NOT delegate cross-origin isolation to the sandbox. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). Without it the sandbox has no shared memory and only coarse timers, which is the defence against a timing attack on the keys where processes are shared | unimplemented |
| `MACRO-048` | `INV` | The sandbox MUST serve the embedder and resource policies that a cross-origin isolated page requires of a frame. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). The client is cross-origin isolated for its shared memory (`ARCH-012`), and it refuses to load a frame that lacks them | unimplemented |
| `MACRO-011` | `INV` | A dialog a macro opens MUST offer no masked input field. | this spec section 4 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A macro that drew a passphrase prompt would harvest the one secret that opens every document. The rule slows a lure and no more: a plain field that asks for the passphrase still works, and `MACRO-012` is the defence that holds | unimplemented |
| `MACRO-012` | `OBS` | A dialog a macro opens MUST name the macro and its integrator. | this spec section 4 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-045` | `OBS` | A message a macro posts in a conversation MUST show that a macro posted it. | this spec section 4 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A message that looked typed would let a macro speak for its user | unimplemented |
| `MACRO-013` | `OBS` | A macro MUST show its publisher and its integrator. | this spec section 4 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-014` | `INV` | An approval MUST bind one exact version of a macro, in one document. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). An approval that survived an edit would let any editor swap the code under a trust already given | unimplemented |
| `MACRO-015` | `INV` | The code that runs MUST be derived from the approved source alone. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A document that stored a compiled form beside the source could show one and run the other | unimplemented |
| `MACRO-016` | `INV` | A version published after an integration MUST NOT replace the integrated one on its own. | this spec section 3 | not built | unbuilt -- lands with the `Macro marketplace` Story (`ambre-roadmap.md` post-V1). An update pushed to every document at once is how one stolen publisher key reaches every user | unimplemented |
| `MACRO-017` | `INV` | A secret a macro declares MUST hold one value per user, kept in that user's settings. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6). A token pasted into the source would be read by every member of the document | unimplemented |
| `MACRO-044` | `CONC` | Two versions of a macro saved concurrently MUST leave one of them whole, never a merge of both. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A source merged character by character is code that no integrator signed | unimplemented |
| `MACRO-018` | `INV` | A secret value MUST NOT reach the document. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.2 Integrate a macro

An editor writes a macro, or installs one from a publisher. Either way, the editor signs this version
into this document. Nothing runs yet.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACRO-019` | `PRE` | A member without the right to edit MUST NOT integrate a macro. | `SYNC-016`; derived | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-020` | `POST` | An integrated macro MUST carry its integrator's signature over the document it lands in and its digest. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A signature that named no document would let an editor copy a macro from another document, under the name and the role of a member who never put it here | unimplemented |
| `MACRO-021` | `POST` | An installed macro MUST be copied whole into the document. | this spec section 3 | not built | unbuilt -- lands with the `Macro marketplace` Story (`ambre-roadmap.md` post-V1). A macro fetched at each run would tell its source which documents use it, and when | unimplemented |
| `MACRO-022` | `FRAME` | Integrating a macro MUST NOT run it. | derived | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-023` | `FAIL` | A source that uses TypeScript syntax the stripper cannot erase MUST be refused at integration. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). Refusing here tells the integrator; refusing at run tells every other member instead | unimplemented |
| `MACRO-054` | `FAIL` | A macro whose source is a compiled module MUST be refused at integration. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A user can read a short source before approving it, and cannot read a binary | unimplemented |
| `MACRO-024` | `FAIL` | A package whose publisher signature fails MUST be refused at integration. | this spec section 3 | not built | unbuilt -- lands with the `Macro marketplace` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.3 Approve a macro

A user reads what a macro asks for, and allows it on their own client.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACRO-025` | `POST` | The approval prompt MUST list every capability the macro asks for. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-026` | `FRAME` | An approval MUST NOT reach any other member. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). Trust is personal: one member's approval is no reason for another's code to run on their machine | unimplemented |
| `MACRO-051` | `POST` | Revoking an approval MUST stop every later run of that version for that user. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-027` | `FAIL` | A macro whose integrator signature fails, or names another document, MUST NOT be offered for approval. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.4 Run a macro

A user presses the button of an approved macro. Its edits land as one change.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACRO-028` | `PRE` | A macro MUST NOT run before its user approves its current version. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-029` | `PRE` | A macro whose integrator has left the document MUST NOT run until a member integrates it again. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). An integrator who left holds no role, so the lower of the two roles is none | unimplemented |
| `MACRO-030` | `POST` | A run MUST read the document as it stood when the run began, plus its own edits. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). Peers keep editing during a run, and a macro that read their edits halfway would compute on two states at once | unimplemented |
| `MACRO-031` | `POST` | A run MUST undo as one step. | `1-Foundations/ambre-spec-undo.md section 2` (`UNDO-001`) | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-049` | `PRE` | A run MUST commit only if the lower of the two roles still allows each of its commands. | `SYNC-016`; this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A role lowered during the run would otherwise be read from before the change | unimplemented |
| `MACRO-032` | `FRAME` | A run MUST NOT block the interface. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A loop that never ends is the first macro most users write | unimplemented |
| `MACRO-033` | `FAIL` | A run that exceeds its time budget MUST be stopped and leave the document unchanged. | `1-Foundations/ambre-spec-budgets.md section 1`; this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-050` | `FAIL` | A run whose replay fails on one command MUST leave the document unchanged. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1). A peer can delete, during the run, the row a command targets; half a run applied is an edit nobody asked for | unimplemented |
| `MACRO-034` | `FAIL` | A run that raises an error MUST leave the document unchanged and report the error. | this spec section 3 | not built | unbuilt -- lands with the `Document macros` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.5 Trigger a macro on an edit

A macro declares that it runs after a kind of edit, such as a new row.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACRO-035` | `PRE` | An edit trigger MUST run only for a user who granted automatic runs. | this spec section 3 | not built | unbuilt -- lands with the `Macro triggers` Story (`ambre-roadmap.md` post-V1). A macro that runs without a click is one the user must know about | unimplemented |
| `MACRO-036` | `POST` | An edit trigger MUST run only in the client that made the edit. | this spec section 3 | not built | unbuilt -- lands with the `Macro triggers` Story (`ambre-roadmap.md` post-V1). If every client ran it, a row would be stamped once per member online | unimplemented |
| `MACRO-037` | `FRAME` | An edit that arrives from another peer MUST trigger no macro. | this spec section 3 | not built | unbuilt -- lands with the `Macro triggers` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-038` | `FRAME` | The edits of a macro MUST trigger no macro. | this spec section 3 | not built | unbuilt -- lands with the `Macro triggers` Story (`ambre-roadmap.md` post-V1). Two macros that trigger each other would loop until the budget stops them, on every edit | unimplemented |
| `MACRO-052` | `POST` | A trigger run MUST undo in one step with the edit that fired it, unless a local edit came between them. | `1-Foundations/ambre-spec-undo.md section 2` (`UNDO-001`); this spec section 3 | not built | unbuilt -- lands with the `Macro triggers` Story (`ambre-roadmap.md` post-V1). The user sees the effect as part of the edit; undoing the row must not leave its stamp behind | unimplemented |
| `MACRO-039` | `FAIL` | A trigger whose macro the editing user has not approved MUST NOT run, and MUST tell that user. | this spec section 3 | not built | unbuilt -- lands with the `Macro triggers` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.6 Call the network from a macro

A macro calls an address its user approved. The browser refuses any other.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MACRO-040` | `PRE` | A macro MUST call only a domain its user approved for that version. | this spec section 3 | not built | unbuilt -- lands with the `Macro network access` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MACRO-041` | `POST` | A macro call MUST leave from the sandbox, never from the Sync worker. | this spec section 3 | not built | unbuilt -- lands with the `Macro network access` Story (`ambre-roadmap.md` post-V1). Through Sync, the domain check would be code, the content policy of the whole client would open to every approved domain, and the worker that holds the keys would parse a third party's answers | unimplemented |
| `MACRO-042` | `FRAME` | A macro call MUST carry no credential of the user. | this spec section 3 | not built | unbuilt -- lands with the `Macro network access` Story (`ambre-roadmap.md` post-V1). A call that carried the session token would let the macro act on the relay as its user | unimplemented |
| `MACRO-043` | `FAIL` | A call to a domain not approved MUST be refused before any byte leaves. | this spec section 3 | not built | unbuilt -- lands with the `Macro network access` Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** the macro rows of `ambre-spec-budgets.md section 1`. Each run has a ceiling, one for a
  gesture and one for a trigger, and the fork a run reads has a target.
* **Boundary.** a macro is sealed like content (`ZK-001`) and runs on no relay (`ZK-004`). A call a
  user approves sends what the macro chooses to a third party, and section 6 names that exit.
* **Verification.** 0 `verified`, 54 `unimplemented`. Nothing here is built.

---

## 2. WHAT EACH USER SEES

A macro is content, so it syncs like any other edit, but as one value: a concurrent save keeps one
version whole (`MACRO-044`). Its runs do not sync: each user runs it alone.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| installs a published macro | opens the document | the macro, signed by its publisher and by Alice (`MACRO-020`) | Bob sees both names, and an approval prompt before any run (`MACRO-013`, `MACRO-028`) | none -- unbuilt |
| runs the macro | edits another cell at the same time | both edits, merged | the run undoes as one step, for Alice only (`MACRO-031`) | none -- unbuilt |
| is an owner, and runs a macro Bob integrated as an editor | -- | the edits an editor may make, and no more | the macro could not share the document (`MACRO-004`, `MACRO-005`) | none -- unbuilt |
| leaves the document | presses the button of the macro Alice integrated | the macro, unchanged | Bob is asked to integrate it again before it runs (`MACRO-029`) | none -- unbuilt |
| adds a row, which has a trigger | is online | the row, and one stamp from Alice's client | one stamp, not two (`MACRO-036`, `MACRO-037`) | none -- unbuilt |

---

## 3. MODEL

**Containers.** one per document: a map of macros in the document metadata, sealed like content
(`MACRO-001`). Each entry is one value that a save replaces whole, never a text that merges
(`MACRO-044`). Approvals and secret values live in the user's own settings, never in the document.
Those settings land with the `User settings` Story (`ambre-roadmap.md` E6).

**Two parties to a macro.**

| Party | Who | Role in the document | What the signature proves |
|---|---|---|---|
| publisher | whoever wrote the code: a member, or a third party behind a marketplace | none | where the code comes from |
| integrator | the member who put this version in this document | editor or owner | who answers for it here; it bounds the run (`MACRO-004`) |

A macro a member writes has one person in both parties. A published macro has a publisher key and a
member who installed it.

**The shapes.**

```rust
/// One macro, as the document stores it, and replaced whole on each save.
/// `digest` covers the name, the source, the API version, the asks and the
/// trigger; `integrator_signature` covers the document id and the digest.
pub struct Macro {
    pub name: String,
    pub source: String,
    pub api_version: u16,
    pub digest: [u8; 32],
    pub asks: Vec<Capability>,
    pub trigger: Option<Trigger>,
    pub publisher: Option<PublisherSignature>,
    pub integrator: PublicKey,
    pub integrator_signature: Signature,
}

/// A third party's signature over the source and its manifest.
pub struct PublisherSignature {
    pub publisher: PublicKey,
    pub version: String,
    pub signature: Signature,
}

/// What a macro may reach beyond the document's commands.
pub enum Capability {
    Network { domain: String },
    AutoRun,
    ClipboardRead,
    SaveFile,
    Secret { name: String },
}

/// When a macro runs without a button.
pub enum Trigger {
    OnEdit { kind: EditKind },
}

/// One approval, kept in the running user's settings.
pub struct Approval {
    pub document: Id<Document>,
    pub digest: [u8; 32],
    pub granted: Vec<Capability>,
    pub source: ApprovalSource,
}

/// Who approved. Only `User` exists until an organization can sign a catalog.
pub enum ApprovalSource {
    User,
    OrganizationCatalog { organization: OrganizationId },
}
```

**The capabilities.** Four tiers, and each capability sits in one.

| Tier | What it covers | How it is granted |
|---|---|---|
| always | the commands of the document within the role of `MACRO-004`, the selection, dialogs Ambre draws | nothing to grant |
| listed at approval | a network domain, automatic runs, reading the clipboard, saving a file, a named secret | the approval of that version (`MACRO-025`) |
| per gesture | opening another document, importing a local file | a picker, each time (`MACRO-009`) |
| never | members, roles, sharing, keys, deletion; camera, microphone, location | nothing grants them (`MACRO-005`, `MACRO-006`) |

**Where it runs.** In a worker the browser compiles and optimizes like any page script. The worker
lives in a frame served from an origin of its own, sandboxed without its parent's origin. That
origin shares no storage with Ambre (`MACRO-008`). The frame comes in two variants, and the browser
holds the network of each (`MACRO-010`).

| Variant | Served for | Header policy | What the macro reaches |
|---|---|---|---|
| closed | a macro granted no domain | `connect-src 'none'` | nothing |
| network | a macro granted one domain or more | `connect-src https:` | the approved domains only, once narrowed |

In the network variant, Ambre's bootstrap adds a policy in a `meta` element before it creates the
worker. That policy lists the approved domains, and the browser enforces the intersection of both.
The worker starts from a `blob:` address, so it inherits the policy of the frame. The server that
serves the frame never learns which domains a macro calls.

The frame speaks to the Logic thread by message only. It is outside the two instances of the client
(`ARCH-001`), and it holds none of their memory. Its permissions policy denies the camera, the
microphone and the location.

The sandbox domain is a registrable domain of its own, not a subdomain of the client (`MACRO-046`).
Its frame serves `Cross-Origin-Embedder-Policy` and `Cross-Origin-Resource-Policy: cross-origin`,
which the isolated client demands of it (`MACRO-048`). The client never grants it
`cross-origin-isolated`, so the macro gets no shared memory and coarse timers (`MACRO-047`). A
deployment without a second domain has no macros.

**The language.** TypeScript limited to syntax that erases, or plain JavaScript. The document stores
the source alone. Before a run, the sandbox erases the types with a stripper of a pinned version.
The code that runs thus follows from the source that was approved (`MACRO-015`).

**The interface.** One global object, `ambre`, injected into the sandbox; nothing is fetched. It is
asynchronous and works on ranges, never cell by cell, because each call crosses a message.

```text
// Common to every application
await ambre.ui.alert("Done");                       // dialogs drawn by Ambre, not by the sandbox
const token = await ambre.secrets.get("API_KEY");   // this user's value, never the document's
const res = await ambre.net.fetch(url, { token });  // approved domains only, held by the browser

// One namespace per application
const r = ambre.trame.sheet("Q3").range("A2:D500");
await r.offset(0, 4).setValues((await r.values()).map(margin));
await ambre.prose.replace(await ambre.prose.find(/TODO/g), "DONE");
await ambre.murmure.post(thread, "Summary: ...");
```

The type declarations are generated from the Rust command definitions, and published for editors.
Every value that crosses the boundary is plain data, so a WebAssembly interface can describe the same
calls later.

**An installation, in steps.**

1. an editor picks a package; its publisher signature is checked (`MACRO-024`);
2. the source and its manifest are copied whole into the document (`MACRO-021`);
3. the editor signs the document id and the digest as integrator (`MACRO-020`);
4. a later version of the package changes nothing until an editor installs it (`MACRO-016`).

**A run, in steps.**

1. the client checks the approval of the current digest, and that the integrator is a member
   (`MACRO-028`, `MACRO-029`);
2. the Logic thread forks the document; the run reads and writes that fork alone (`MACRO-030`);
3. the sandbox erases the types and starts the code under a time budget (`MACRO-032`);
4. each command the macro calls is checked against the open list of its application and the lower
   of the two roles (`MACRO-053`, `MACRO-004`). It is then applied to the fork, and recorded;
5. a network call leaves the sandbox, and the browser refuses a domain not approved (`MACRO-040`,
   `MACRO-043`);
6. at the end, the roles are checked again (`MACRO-049`);
7. the recorded commands replay on the document as local edits, in one undo step (`MACRO-031`).
   One command that fails drops the whole replay (`MACRO-050`);
8. a run past its budget is terminated; it and a run that raises commit nothing (`MACRO-033`,
   `MACRO-034`).

The fork serves reads only. Its edits come from a peer of their own, and the undo history keeps
local edits alone. Imported, they would make the run impossible to undo. The replay costs each
command twice, and the fork costs one export and one import of the document.

**A trigger, in steps.** A local edit that matches the kind of a trigger queues its macro, if the
user granted automatic runs (`MACRO-035`). A remote delta queues nothing (`MACRO-037`), and the
commit of a run queues nothing (`MACRO-038`). The run joins the undo step of its edit, unless a
local edit came between them; then it takes a step of its own (`MACRO-052`).

**Code.** not built -- the model lands with its Stories (`ambre-roadmap.md` post-V1).

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| "Macros" in the tools menu | the list of macros, each with its publisher, its integrator and its state for this user (`MACRO-013`) |
| the macro editor | edit the source and the capabilities it asks for; saving signs it as integrator (`MACRO-020`) |
| "Install a macro" | pick a package; its publisher is checked and its source copied in (`MACRO-021`, `MACRO-024`) |
| a button, a slash command, or a command palette entry bound to a macro | run it, or ask for approval first (`MACRO-028`) |
| the approvals list, in the user's settings | each approved version, with its document and its grants; revoke stops its next run (`MACRO-051`) |
| the approval prompt | the publisher, the integrator, the source, the capabilities asked, and the refusing action first (`MACRO-025`) |
| a dialog a macro opens | framed as the macro's, with its name and its integrator, and no masked field (`MACRO-011`, `MACRO-012`) |
| a run that stops | a notice names the macro and the reason; the document is unchanged (`MACRO-034`) |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A macro runs in a member client | a macro run by the relay, as Apps Script runs on the server | the relay cannot open the document (`ZK-004`). A relay able to run a macro would be a relay able to read |
| A worker the browser optimizes | an interpreter embedded in the Logic instance | far slower than the browser's compiler on a loop over a sheet, and several megabytes more in the Logic instance. Its one gain, counting each instruction, is not needed: a worker stops when terminated |
| A registrable domain for the sandbox | a frame with an opaque origin, served from the client's own page | no second domain to deploy, but the frame is always the same site as the client, so it always shares the process of the keys |
| The sandbox is enforced by the browser | a worker whose network functions are removed in code | the global scope of a worker holds dozens of ways out, and a sandbox that lists them by hand fails at the first one it forgets |
| The lower of the integrator's and the running user's role | the running user's role alone | an editor could integrate a macro that shares the document, and wait for an owner to press its button |
| The lower of the integrator's and the running user's role | the publisher's role | a marketplace publisher is never a member, so its role is none and no published macro could write |
| Another document through a picker only | a document named by id, under a capability | the integrator chooses the id and the running user holds the key. The macro would read, for its integrator, what only the running user may open |
| The document stores the source alone | the source and its compiled form, side by side | an editor could store a harmless source and a harmful compiled form; the user would approve one and run the other |
| An installed macro is copied into the document | a macro fetched from the marketplace at each run | the marketplace would learn which documents run which macro, and when; the sandbox has no network to fetch with |
| An update needs a new integration | updates applied as the publisher releases them | one stolen publisher key would push code into every document at once, past every approval |
| A secret has one value per user | a secret stored in the document | every member reads the document, so every member would hold the token |
| TypeScript that erases, or JavaScript | full TypeScript compiled by `tsc` | the compiler is too large to load for a run, and its output is a second form to trust |
| TypeScript that erases, or JavaScript | a language of Ambre's own | a user who writes one macro a year will not learn a new language for it |
| TypeScript that erases, or JavaScript | WebAssembly modules in any language | a user can read a short source before approving it, and cannot read a binary (`MACRO-054`). The interface stays plain data, so the refusal is a rule and not a wall |
| A cell calls a named formula | a macro function called from a cell | a cell must compute the same value on every client, inside a recompute that cannot wait on a worker. Trame's named formulas give custom functions without a second language |
| The recorded commands replay on the document | the fork's edits imported into the document | they arrive as a remote peer's edits, and the undo history keeps local ones alone: the run could not be undone (`MACRO-031`) |
| The recorded commands replay on the document | edits written straight into the document as the run goes | peers would see half a run, and a run that failed would have to be undone in front of them (`MACRO-033`) |
| An approval lasts until revoked | an approval that lapses after a period | it binds one version, so the code a user approved is the code that runs a year later. A lapse would train users to approve again without reading |
| A trigger run joins the undo step of its edit | a trigger run as its own undo step | undoing a new row would leave its stamp behind, and the user would undo twice for one gesture |
| An approval binds one version, in one document | an approval per author, or shared across documents | an editor could change the code after the approval. Approving a network call approves sending this document, not every document the macro lands in |
| An edit trigger runs in the editing client only | every client that sees the edit runs it | each online member would apply the effect once more |
| No timed trigger | a trigger that runs every hour | no client is sure to be online at the hour. A timed job is a machine member (`ambre-spec-machine-access.md`) |
| No ambient network | network open to every macro, as in VBA | one approved click would send the whole document in clear to any address |
| A call leaves from the sandbox | a call relayed by the Sync worker | the check would be code, the policy of the whole client would open to every approved domain, and the worker holding the keys would parse hostile answers |
| The approved domains narrow the policy on the client | a list of domains fixed in the deployment's policy | macros could reach only the interfaces the host picked in advance |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Every row is unbuilt.** No document carries code today.
* **Process isolation is partial on mobile.** A distinct site gets its own process on desktop
  browsers. Chrome on Android isolates only some sites, Firefox on Android none, and Safari is
  still rolling it out. There, the macro can share the process of the Sync worker. Coarse timers
  and no shared memory make a timing attack on the keys hard; nothing makes it impossible.
* **A bug in the bootstrap fails open.** If the `meta` policy is not added, a network variant reaches
  any `https:` address. The closed variant never does.
* **Only interfaces open to a browser answer.** The sandbox sends a null origin, so an interface
  that does not answer every origin refuses it. A proxy would fix that, and the relay cannot be one:
  it would read the call in clear.
* **An approved call is an exit.** A macro with an approved domain sends what it chooses in clear.
  The approval names the domain, and nothing checks what the macro sends there.
* **User settings do not exist.** Approvals and secret values need a sealed store that follows a user
  across devices. It lands with the `User settings` Story (`ambre-roadmap.md` E6).
* **The marketplace is not ruled.** Who hosts it, who vets a publisher, and how a stolen publisher key
  is revoked all belong to the `Macro marketplace` Story (`ambre-roadmap.md` post-V1).
* **`MACRO-004` inherits the relay's authority on roles.** A lying relay can raise the bound of a
  macro to its running user's role, which it could already grant. The approval, held on the client,
  still bounds the macro. That is why an organization catalog needs a trust anchor and this row does
  not.
* **Organization catalogs are not ruled.** An organization could approve macros for its members, and
  restrict them to that list. Its signature is worth nothing while the relay decides who its admins
  are. The catalog waits on the `Organization trust anchor` Story (`ambre-roadmap.md` post-V1).
* **A policy binds a client, not a person.** The relay cannot see a macro, so only the client enforces
  a catalog. It stops a mistake or a lure, and it does not stop a member who changes their client.
* **A run computes on a state that can be stale.** A total read from the fork is written after a peer
  may have changed a value it summed. An offline edit has the same limit, and nothing here treats it.
* **No application declares its macro commands yet.** `MACRO-053` opens none until one does.
* **An approval has no end date.** It binds one version, so time adds no risk the code did not
  have. A user revokes it by hand (`MACRO-051`).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The commands a macro calls, and their chords | `ambre-spec-commands.md` |
| What one undo step holds | `ambre-spec-undo.md` |
| Which thread holds what, and where a network call leaves | `ambre-spec-architecture.md` |
| What a role allows | `ambre-spec-membership.md` |
| What an organization is, and who administers it | `ambre-spec-organizations.md` |
| A program that runs without a user online | `ambre-spec-machine-access.md` |
| The figure for any budget | `ambre-spec-budgets.md` |
| A custom function a cell calls, as a named formula | `2-Trame/features/ambre-spec-trame-function-library.md` |
