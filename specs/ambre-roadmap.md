# AMBRE -- ROADMAP

**Type:** Scheduling (one file for the whole project)
**Scope:** every epic and every story, for every application.

> **Role of this file.** Nothing schedules anywhere else. A spec says what the system does; this file
> says what work is left to reach it. A spec that carries its own backlog is a second schedule, and
> two schedules disagree.

---

## 1. THE MODEL

| Term | What it is | Where it lives |
| :--- | :--- | :--- |
| **Feature** | the smallest thing a user can name, ship alone and specify end to end | one spec document, always current |
| **Story** | one unit of production that moves one or more features forward | one row of this file |
| **Milestone** | a release scope: `MVP`, `V1` or `post-V1` | one `##` section of this file, repeated in the `Milestone` column |
| **Epic** | the stories of one milestone, one increment of value | the milestone section; a closed epic is a heading of `ambre-roadmap-journal.md` |
| **Area** | the stories of one milestone in one product area | one `###` heading of a milestone section |

**Eight rules, and each one exists because it was broken.**

* **A story is one row, and it says what, never how.** A checkbox, a name, its milestone, the
  features it touches. The code says how it works and git says how it got there. A row that explains
  an implementation is a row nobody reads, and it rots on the next commit while looking
  authoritative.
* **A story that needs a paragraph is two stories**, or it is design content sitting in the wrong
  file.
* **Finishing a story is ticking its box. Nothing else.** No note added, no wording changed, no
  account of what was done. The row said what the work was before it started, and it still says it.
* **Closing a milestone is cutting its section into `ambre-roadmap-journal.md`, verbatim, once
  every row of it is `[x]`.** Every row as written, the headings as written. No summary, no
  retrospective, no rewrite. A milestone is `[x]` only when every story under it is `[x]`: the status
  is derived, never set. While the milestone is open a finished story stays, ticked, because that is
  what lets a reader say "eight of eleven, these three remain".
* **A story states when it is done.** Its `Touches` cell ends with `Done when` and a check
  a reader can run. The check names rows that read `verified`, a gate that passes or a test. A story whose
  check did not pass is not ticked; the delivered part is renamed and the rest becomes a new row.
* **A milestone tags a story, never a feature.** A feature lives: it has an MVP form, then others.
  The `Milestone` cell equals the section that holds the row: `MVP` in section 3, `V1` in section 4.
  `contracts.py` refuses a row whose cell differs. A story no release scope names sits in section 5,
  which is why transverse work waits.
* **A milestone section holds no `post-V1` story.** One such row holds the section open for ever, so
  section 5 carries them, with no `Milestone` column. Each row sits in exactly one area.
* **A defect is fixed in the milestone that finds it.** A `failing` row is shipped code that breaks its
  rule today, or that no mechanism holds. Its story carries the milestone in progress: the earliest
  one with a story still open. A later milestone carries only what is not built. `contracts.py`
  refuses such a row bound to a story of any other milestone.

**Two milestones, each with its test.**

* **MVP is the release a team can switch to.** Each application does the daily work of its kind
  alone. Any office file a member receives opens in Ambre and leaves it again, with every loss named
  in a report. A story is MVP when a team that lacks it would keep its old suite.
* **V1 is the release that keeps that team.** The depth of each application, the models the
  applications share, and the checks that hold what MVP shipped by more than one oracle. A story is
  V1 when its absence is a cost, not a reason to leave.

**The milestone is the epic key.** A section's key is `MVP`, `V1` or `POST-V1`, and the order of the
sections orders the work. A story changes section only when its milestone changes. The keys `E0` to
`E9`, `ET` and `EF` name closed epics in the journal. On 2026-11-15 the epics `E5` to `E9` were
dissolved: their open rows moved into sections 3 to 5, and their ticked rows into the journal.

A story is cited by its name, and the name is the only join key. `contracts.py` fails on a citation
this file does not answer, or that names a ticked or closed story (`ambre-spec-template.md` section 1,
`ambre-spec-architecture.md` section 4). It ignores an epic key written in the citation.

---

## 2. SHIPPED

Closed epics live in `ambre-roadmap-journal.md`, whole. They are named here so a citation resolves.
`E5` to `E9` were dissolved on 2026-11-15, and the journal holds their ticked rows alone.

| Epic | What it delivered |
| :--- | :--- |
| `E0` | the three-thread stack, the GPU pipeline, the chrome, the crypto primitives |
| `E1` | the Trame vertical end to end: encrypted, synchronized, live through the Blind Relay |
| `E2` | the launcher, the sharing UI, the snapshot client legs |
| `E3` | the Trame MVP: a usable spreadsheet |
| `ET` | CI, the three test tiers, the convergence property test, the first UI belts |
| `E4` | the Toile MVP: an infinite collaborative whiteboard with a presentation mode |
| `EF` | the `.ambre` file, the schema register, and the fixture corpora with their gate |
| `E5` | the Prose engine and view, the Murmure core, and encrypted calls through the SFU |
| `E6` | the first V1 depth of the foundations, Trame and Toile, and the converter thread |
| `E7` | the correctness and security fixes the audits found in shipped code |
| `E8` | the shared models: one command registry and one document locale, and the launcher in modules |
| `E9` | the verification debt: the gates read by rustc, the belts, the budgets given a mechanism |

---

## 3. MVP -- THE RELEASE A TEAM CAN SWITCH TO  `[ ]`

The test of section 1: a team that lacks one of these stories keeps its old suite. Every row reads
`MVP`.

### Foundations: sync and relay

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` The delta index is keyed by document and number | MVP | each row of `updates` carries a random key that no query reads, beside an index on the document and the number. The random key writes to a random page of its index on every insert. After each checkpoint, that page goes whole into the WAL. Done when `updates` carries no random key, no index of `updates` repeats another, and the write cost per delta is measured again and lower |
| `[ ]` A delta moves to its batch object without an index write | MVP | the move clears the inline frame and names the batch, so the row leaves one partial index and enters another. Postgres then writes a new row version and an entry in every index, and the move costs as much as the insert. Done when the move of an inline delta is a heap-only update, counted by a database-tier test over the table statistics, and `SRV-151`, `SRV-154` and `SRV-155` still read `verified` |
| `[ ]` The partial inline index earns its write | MVP | since the delta index is keyed by document and number, `idx_updates_inline (document_id, seq) WHERE wire IS NOT NULL` carries the columns of the primary key, and serves only the inline tail count and the idle inline sweep. No run measures what it adds to the WAL of an accepted delta. Done when `just write-cost` reads the WAL with and without it, and the index is dropped for a query on the primary key, or kept with its figure in `ambre-spec-budgets.md section 3` |
| `[x]` Compaction keeps fewer delta rows | MVP | compaction keeps ten snapshots per document and trims below the lowest of them, so an active document keeps about 4 500 rows in `updates`: 108 MB of heap and 83 MB of indexes for 32 documents in the write-cost run of 2026-11-13. Most of the WAL a delete writes is full pages of those cold rows. Done when a row of `ambre-spec-budgets.md` bounds the rows kept per document, the sync rows still read `verified`, and the write cost per delta is measured again and lower |
| `[ ]` The write-cost gap between CI and a local run is explained | MVP | the short write-cost run reads 2 674 B to 2 676 B per delta in the CI `test-db` job and 2 632 B to 2 643 B in a cloud container, the same Postgres image: about 30 B more full-page images and 73 more distinct blocks in CI. The ceiling of 2 730 B sits on the CI figure, so a story that lowers the cost cannot read its gain locally against it. Done when the cause is named in `ambre-spec-budgets.md` section 3, and the short run reads within its margin on both machines |
| `[ ]` An insert takes no lock on its document row | MVP | each insert of a delta checks its foreign key to `documents`, which locks the document row: one WAL record per insert, and a wait between two writers of one document. Done when the insert writes no lock on the document row, the deletion of a document still removes its deltas under a row that reads `verified`, and the write cost per delta is measured again |
| `[x]` The relay cost of an average typist | MVP | the send window of a document is 25 ms, the client share the latency budget leaves. At five keys a second it groups no key, so each key still costs the relay an index row, an ordering script and a broadcast. Done when a row of `ambre-spec-budgets.md` section 3 states the relay load of N typists at five keys a second, measured, and either the database tier holds it or a story names the cut |
| `[ ]` The compaction pass runs often enough for the kept rows | MVP | the pass that deletes snapshots and trims `updates` runs once an hour, so an active document adds the deltas of up to one hour on top of the bound of the delta rows kept: at five deltas a second, 18 000 rows. Done when the trim runs at a period a row of `ambre-spec-budgets.md` states from the snapshot threshold and the delta rate, and the write cost per delta is measured again |
| `[ ]` The write-cost CI figure of two snapshots kept is recorded | MVP | the ceiling of 2 280 B is the dearest local run plus 2 %, with no CI allowance; the CI reading after two snapshots kept is not in the budget row. Done when the row quotes the CI figure of the run that merged two snapshots kept, and the ceiling is that figure plus 2 % |
| `[ ]` An open without its link keeps the pin it proved | MVP | the inviter pin lives in the Sync worker for one session, and an open from the document list or a share notice names none, so such an open is back on the anchor the relay chooses (`ambre-spec-membership.md section 6`). Done when the pin, or the anchor it proved, persists per document across sessions, every later open checks against it, and a row states it and reads `verified` |

### Foundations: security and zero knowledge

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` A key no member signed is shown as a relay fault | MVP | an open that meets a document key no member of the journal signed is refused, but the refusal reaches the view as `KeyNotReceived`, the answer of a key not yet delivered. A relay that serves a key of its choosing is the attack the zero-knowledge pillar exists to expose, and the user sees a wait. Done when the bus carries a refusal of its own for a key no member signed, the view shows it as a security fault, and a row reads `verified` |
| `[x]` A first open trusts no wrapper the relay names | MVP | the log anchor travels inside the copy, so a relay that wraps a key of its own also picks the anchor and serves a whole log rooted at a creator of its own; every key check then passes. Done when a row binding the first open of a document to an anchor or a wrapper key this client holds out of band (a pinned inviter, or an anchor kept from an earlier open) reads `verified` The owner chose on 2026-11-15: the invite link carries the fingerprint of the inviter's signing key, and the first open checks the log against it |
| `[ ]` A channel document accepts no current key the relay chose | MVP | a segment or an amendments document serves the rows and the links of its `meta` and a log of its own, so no log the open walks names its key and the key checks skip every Murmure document; the chain also drops the `meta`'s links, which name the `meta`. Done when the open of a channel document names its `meta` from the sealed channel state, and a row checking its key against the `meta`'s log and links reads `verified` |
| `[ ]` The wrapper check reads the exchange key the log signs | MVP | the open checks the wrapper of its copy against the exchange key the relay serves on the member's row (`SYNC-201`), which no signature covers, while the log now signs each member's exchange key (`SYNC-197`). Done when the check reads the key from the log, and a test where the relay swaps the key on a row is refused |
| `[x]` A member sees that the relay withheld a log entry | MVP | the relay can hide a revoke and its rotation from one member, then accept that member's deltas under the old key and forward them to the removed member. Only the relay refuses a stale key, so the removal guarantee holds against an honest relay alone. Done when a row binding a peer's detection of a log head it has not seen reads `verified` |
| `[ ]` A log head a member announces is signed by its author | MVP | an unsigned head lets a member forge a later head and have the relay blamed for withholding an entry it never had (`SYNC-207`). The owner wants the blame sound before the MVP. Done when a row binding the warning to a head whose signature checks reads `verified` |
| `[ ]` A presence sample carries the log head | MVP | an update alone carries the head its author read (`SYNC-207`), so a relay that drops every update of the up-to-date members towards one member hides the gap. Presence flows constantly. Done when a row binding detection to a head an ephemeral carries reads `verified` |
| `[ ]` A member warned of a withheld entry stops sealing under the old key | MVP | the warning stops nothing: the member's next deltas still reach the removed member through the relay. Done when a row binding held edits to a withheld head reads `verified` |
| `[x]` The page the relay serves matches the source | MVP | the operator serves the client code, and nothing lets a user or an auditor check it against the source. A changed bundle reads every key, so every zero-knowledge row assumes an honest server of code. Done when two clean builds of one commit give the same digest, the release publishes it, and a row of `ambre-spec-zero-knowledge.md` names the limit that remains |
| `[x]` Every answer of the relay carries the strict transport policy | MVP | `ambre-spec-server.md section 3` states that every answer carries a two-year strict transport policy, and the relay sets none. Done when a row binding the header to every answer reads `verified` |
| `[x]` A view sets no style attribute | MVP | the page policy refuses a `style` attribute, so a view written with `style="..."` loses its position in silence, and only the routes the browser test opens show a violation. Done when a gate refuses a `style` attribute built from a string in a view |
| `[ ]` A passphrase change or a recovery takes the strength floor | MVP | the change and recovery handlers of the Sync worker are placeholders that drop both passphrases, so the floor of `ZK-035` holds at account setup alone. Once either seals the Master Seed again, a weak new passphrase reopens the offline guess. Done when a row binding the floor to the new passphrase of a change and of a recovery reads `verified` |
| `[ ]` A security pre-audit of the cryptographic design | MVP | AI reviewers working apart audit the protocol the rest of the code builds on, before more of it does: the key hierarchy and its rotations, the sealed envelopes and the zero-knowledge boundary, the call chunks and sender keys. A flaw in the design costs a rewrite once the remaining stories stand on it, and a fix today. It covers the design and the code that carries it, and leaves the whole workspace to `A security pre-audit of the whole workspace`. Done when each finding is fixed or answered in writing, and each high finding is a contract row that reads `verified` |
| `[x]` A client refuses a delta whose signer holds no writing role | MVP | the client applies a delta or a snapshot from the holder of any key of the document, current or retired, whatever the signer's role (audit P1). A viewer, or a removed member with a key the cache or the chain still reaches, writes through a relay that forwards it. `ambre-spec-key-rotation.md section 5` leaves the refusal to the relay today. Done when a row binding each applied delta to a signer the verified log grants a writing role reads `verified` |
| `[ ]` A security pre-audit of the whole workspace | MVP | once every other MVP story is ticked, AI reviewers working apart audit the code and the specs of every application, the relay and the SFU: the zero-knowledge boundary, the key hierarchy, the call chunks and sender keys, the parsers of foreign files and the server routes. It comes before the external `Call crypto review`, and never replaces it: a reviewer of the same kind as the one that wrote the code shares its blind spots. Done when each finding is fixed or answered in writing, and each high finding is a contract row that reads `verified` |
| `[x]` The relay can depend on no application crate | MVP | `ambre-spec-zero-knowledge.md` section 2 says `deny.toml` keeps `ambre-server` from importing `ambre-trame`, and no ban names `ambre-trame`, `ambre-toile`, `ambre-prose` or `ambre-murmure`: only their CRDT dependency is banned. Done when `deny.toml` bans each application crate outside the client crates, a row binding it reads `verified`, and the section names the bans as they are |
| `[x]` Logout forgets every pending Murmure request | MVP | the logout clears neither the pending deletes, leaves and erases nor the call asks and preview fetches, so a later session retries the erases of the previous user under its own account and sends that user's metadata to the relay. Done when a test logs out with each one pending and no later session sends it |

### Foundations: platform and architecture

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` PNG and GIF decode on the converter thread | MVP | the converter thread decodes a JPEG alone. A PNG or a GIF still decodes whole on the Logic thread, and no one has measured it: a large one holds a key typed in another document, as a JPEG photograph did for 235 ms. Murmure also copies the bytes of a JPEG once on the Logic thread before its job leaves. The decode times of the module come from a scratch run that was deleted. Done when a PNG and a GIF decode on the converter thread under rows that read `verified`, Murmure hands the bytes on without a copy, a committed bench measures the decode of each format in the module, and the row "Prose picture decode, one picture" holds for every format |
| `[ ]` Every conversion runs off the Logic thread | MVP | the converter thread runs a picture decode alone. The PDF write, the DOCX, ODT, xlsx and HTML imports and exports, the Markdown read and write and the SVG read still call their module whole on the Logic thread: a key typed in another document waits behind each (`ambre-spec-architecture.md section 2`, the converter thread). Done when each conversion runs on the converter thread under rows that read `verified`, and the module call leaves the slice rows of `ambre-spec-budgets.md section 1` |
| `[ ]` Each conversion family has a time limit of its own | MVP | a conversion fails after 30 s and three tries, figures estimated and never measured. The PDF and DOCX exports bound for the converter thread take longer on a dense document, so a large export fails while it is still working. Done when each family of conversion, image, Office and PDF, holds a time limit measured on its dense fixture with its margin, in a row of `ambre-spec-budgets.md`, and a row reads `verified` |
| `[x]` A first converter spawn that fails is tried again | MVP | since `ARCH-112` the first job spawns the converter thread, and a first spawn that fails is never tried again, so the whole session loses its conversions until a reload. Done when the next job tries the spawn again, within the respawn limit of `ARCH-108`, and a row reads `verified` |
| `[x]` The respawn row names the browser spawn | MVP | `ARCH-108` says no browser test drives the spawner the page builds over `wasm_thread`. Since `ARCH-112`, the JPEG drop of the browser belt drives the first spawn from the Logic worker, so the sentence is half stale. Done when the `ARCH-108` row names what the belt holds and leaves only the respawn after a lost thread to `A converter thread spawned again runs in the browser` |
| `[ ]` Every converter module gives back its memory | MVP | a module instance is cached for the session, and a WebAssembly memory never shrinks. The images module drops an instance past 64 MiB after a job (`RESET_FOOTPRINT`); no other module does. The markup module keeps about 6.6 times the largest file it read, counted only in the derivation of the Markdown bound, and the text is copied once more for its request. Done when every module instance is held after a job to a row of `ambre-spec-budgets.md` that reads `verified`, and the request takes the text without a copy |
| `[x]` The architecture diagram shows the converter thread | MVP | the tables of `ambre-spec-architecture.md` section 2 give the Main instance three threads, and its diagram draws two. Done when the diagram draws the converter thread and its two queues, and renders |
| `[x]` A converter thread spawned again runs in the browser | MVP | the Logic thread spawns the next converter thread once one stops answering (`ARCH-108`), through a spawner the page builds over the platform's threads. No browser test drives that spawn from the Logic worker: only a native spawner is tested. Done when a browser test holds a thread lost, the next one is spawned from the Logic worker and lands the next job, and the row names that test |
| `[ ]` A long conversion holds a deadline of its own | MVP | the answer deadline of the converter thread is one figure, 30 s, and the respawn limit is 3, both estimates sized on a picture decode. A PDF or DOCX export of a large document, once it runs on the converter thread, may run past it and be failed while it still works. Done when each family of jobs holds a deadline measured on its largest admitted input, and the rows of `ambre-spec-budgets.md` read `verified` |
| `[x]` Main back under its ceiling | MVP | the `size` CI job is red on `main` since 2026-11-14: Main read 10 406 422 raw bytes on `fc5d798e`, then 10 416 059 after the font, Prose import, converter trap and open-key stories, and 10 446 761 after the sheet and deck PDF exports, against a 10,410,000 ceiling. Four cuts came first and took Main to 10 423 590: two map fills without the sort of a `collect`, and shared sorts by identifier in `ambre_core::order`. The ceiling of Main is 10,470,000, the cut figure plus 40 000 bytes for the stories that wait to merge; `ambre-spec-budgets.md` section 5 names each crate that grew, with `twiggy` figures. Done when `just size` passes for Main on `main` |
| `[x]` Main's temporary ceiling comes back down | MVP | the Main ceiling rose to 10,520,000 raw bytes on 2026-11-15 with no cut, so that the stories queued behind a red `size` job could land while `Main back under its ceiling` measures its cut. Done when `scripts/wasm_size.py` holds the figure that story measures, below 10,520,000, and `just size` passes for Main on `main` |
| `[x]` Sync has room for its next feature | MVP | Sync reads 1 309 891 raw bytes against a ceiling of 1 310 000 after the strength floor, so the next change to the worker fails `just size`. Two cuts took Sync from 1 315 088 to 1 257 356: a hand-written base32 for the key fingerprint, and a zstd read with no `std::io::Read` adapter. The ceiling of Sync is 1,270,000, with 9 313 bytes of room once the inviter pin lands; `ambre-spec-budgets.md` section 5 names each cut. Done when Sync holds 5 000 raw bytes of room, by a cut or by a raise its story row names |
| `[x]` A view holds no CRDT type | MVP | the view gate refuses an engine import in a view module, but a type of `ambre-crdt` passes, and a view that holds one can write the document outside the Logic thread. Done when the gate of `UI-THREAD-020` refuses each type of `ambre-crdt` in a view module, with a driver test, and its row reads `verified` |
| `[x]` The Logic runtime holds no Murmure logic | MVP | five modules of `ambre-logic` hold the Murmure logic of channels, spaces, direct messages, segments and unopened documents, as methods of the runtime state. A channel spans several documents, and the engine interface takes one document. So the runtime absorbed logic that belongs to an application crate (`ARCH-018`). Done when `ambre-logic` names no Murmure type outside its content sums and its dispatch, and a gate refuses one |
| `[x]` A Logic module reaches the view code through a listed path alone | MVP | the architecture lets the Logic code of an application call its own view code when the runtime path needs it, and no gate holds that half of the boundary. Toile Logic reaches five view modules today. Done when a gate refuses a Logic path to a view module or to `ambre-canvas` that no list names, with the reason for each entry |
| `[x]` The PDF export arms no font fetch on the Logic thread | MVP | the Toile PDF export builds a text cache on the Logic thread, and a missing face there arms a fetch through a window the thread does not have. The fetch never runs, and each export arms it again. Done when a row binding the export to the faces the Logic thread already holds reads `verified` |
| `[x]` The relay links no client-only module of the core | MVP | `ambre-core` carries the document file, the export file and the locale, which the relay, the wire and the SFU never use, and the relay compiles them all the same. Done when the three modules sit in a crate the relay does not link, and the crate gate refuses a relay that reaches it |
| `[x]` An emoji presentation selector in a base family | MVP | parley 0.10 counts a presentation selector `U+FE0F` as mapped in a cluster of one shaped character, so the first base face claims `U+2764 U+FE0F` whole and draws glyph 0 for the heart. The lazy fallback cannot reach a cluster the base face already claimed. Found on 2026-11-09 while building `Lazy faces in the sealed fallback, outside a presentation selector`. A spike on parley 0.12 on 2026-11-10 still drew `U+263A U+FE0F` wrong, regressed a bare `U+2764`, added 18 KB to Main and a second `skrifa`, so parley stays on 0.10. The text style of an emoji sequence names the emoji family before the base family, through one constructor of the ranged builder that a gate makes the only one, its cost read under the `paragraph_shape` bench. Done when `CANVAS-090` reads `verified` |
| `[x]` Sync's temporary ceiling comes back down | MVP | Sync read 1 315 076 raw bytes on `main` after the strength floor, over its ceiling of 1,310,000, and the inviter pin adds 3 331. The ceiling rose to 1,325,000 on 2026-11-15 with no cut, so that the queue behind a red `size` job can land while `Sync has room for its next feature` measures its cut. Done when `scripts/wasm_size.py` holds the figure that story measures, below 1,325,000 |
| `[x]` Main writes to no standard stream | MVP | the ICU warnings no longer reach the standard error of the Main instance, and the rule covers ICU alone. The debug Main wasm still links callers of std's print functions: `lyon_tessellation` debug logging, `loro` lock guards, `serde_columnar` error paths. Two threads that print at once contend for the lock of the stream, and the main thread's wait traps the page as the IME test did. Done when a gate reads the debug Main wasm and refuses any caller of std's print functions outside an allowlist, and a row states the rule |

### Build, CI and verification

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` The mechanism texts match the gates they name | MVP | four texts describe a gate the code no longer has: `ZK-002` and section 2 of the zero-knowledge spec omit the call worker, `PDF-008` names a paint call that tests alone make, and `ARCH-078` omits the office codecs. Done when each of the four texts names the gate the code holds |
| `[x]` The prose IME test passes on every Chrome run | MVP | `ime::tests::a_composition_in_a_prose_page_commits_its_text_once` of `ambre-belt` failed on a Chrome run and passed on the next with no change, so a red browser tier no longer says whether a change broke it. Done when the cause is named, the test waits on the event it asserts and not on time, and fifty runs in a row pass on Chrome |
| `[x]` No CI checkout keeps the job token on disk | MVP | each `actions/checkout` step leaves the job token in `.git/config` by default, where every later step of the job, a build script of a dependency included, can read it. Done when every checkout sets `persist-credentials: false`, the steps that push are named with the reason, and a gate refuses a checkout that omits it |
| `[x]` The newtype census reads a field typed by a bare type parameter | MVP | a generic struct outside `ambre-core` with one field of type `T` resolves to no primitive, so `S<u32>` holds a `u32` with no census line, while `ARCH-062` puts a generic struct outside `ambre-core` under the rule. The census reads each instance a compile of the lint gives the struct, or fails closed on a bare parameter. Done when a test of the gate refuses `struct S<T>(T)` used as `S<u32>` with no census line, and `ARCH-062` names no known gap |
| `[x]` A contract id is reserved before its branch lands | MVP | parallel branches took the same ids, `ARCH-107`, `ARCH-109`, `ARCH-112`, `SRV-259` and `SRV-260`, and each landing renumbered a rule and its citations by hand. Done when the contracts gate refuses a branch whose new id already names another rule on `main`, with the next free id in its message, and a test holds the refusal |
| `[x]` Media reachable from the phone test stack | MVP | the relay signs presigned links for `http://garage:3900`, a name of the stack network over plain HTTP, so a phone on the dev container stack fails every media upload and download. Done when the relay presigns against a public S3 endpoint served under the HTTPS origin of the stack, and the probe of the `devcontainer` workflow puts one object through a presigned link from the runner |
| `[ ]` The math lines read against the rule | MVP | about three hundred lines of the primitive allowlist file a value as `math`, and the notes several share allow it for a count, a size, an index or a limit: "count, size, limit, index or jitter local to an algorithm", "geometry, layout, GPU or count arithmetic". `ai/ambre-rust-style.md` says a count, a size or a quantity with a unit is never a multiplier, so widths, heights, cursors and counts pass there untyped. Each line is read against that rule: a value with a unit, a count or a size moves to a family of `ambre-core` or to a census line with its reason, a line whose category is wrong takes the true one, and a note that allows what the rule refuses is rewritten. Wire and store bytes stay unchanged. Done when each remaining `math` line names a scalar multiplier or a term inside a formula, and no note of the file names a count, a size or an index as `math` |
| `[x]` A branch push answers within its CI budget | MVP | every push ran the whole pipeline, about 60 minutes set by the instruction counts, and every job compiled every dependency again: `rust-cache` deleted the `std` that `-Z build-std` builds. A branch push runs a fast lane alone; `main` and a pull request run the full lane; the schedule adds the mutants and the coverage. Firefox, Chrome and geckodriver name their versions. Done when a branch run holds the fast-lane row and a run of `main` the full-lane row of `ambre-spec-budgets.md` section 7, and `ARCH-105` reads `verified` |
| `[x]` Size blocks on the product budgets alone | MVP | each converter module and worker holds a raw ceiling a few hundred bytes above its measure (`scripts/wasm_size.py`), so a feature that grows one fails `just size` and needs a raise story (`ARCH-095`): the ceilings rise one after another and builds break on growth no user sees, while the budget users feel is the first download. Done when `just size` fails only on the budgets of `ambre-spec-budgets.md` section 5 (the first download, Main plus Sync, under 4 MB brotli; each lazy module under a brotli budget of its own; Main raw under a ceiling with a margin of at least 2 %), every push prints the delta of each artefact against `origin/main`, the per-artefact raw ceilings become a report that fails nothing, `ARCH-095` and `ARCH-096` say so, and the row "Each wasm ceiling holds 1 % of room" is closed by it. The Main ceiling rises to 10 660 000 raw, 2 % above its measure of 10 442 998, with no cut tried: the raise buys the margin, not a growth |

### Trame

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` A hidden line leaves the cached axis after a sweep | MVP | the cached axis view of Trame keeps the concealed lines and the registers that a full read no longer holds, once `delete_selected_axis` hides lines and a sweep runs, so `ambre_trame::logic::tests::group14::four_peers::every_reference_reads_its_live_element_or_ref` panics with "the layout is the list's" in `crates/ambre-trame/src/model/axis.rs`. Its shrunk case: `[Push(0, 1), Delete(1, 0), Hide(2, 0, 0), Push(1, 1), Sync(1, 2), Sweep(1), Hide(1, 0, 0)]`, seed `1b866af92cb792acbe17904730f782d263495f1d94965ddf5261fada9214fe2e`. A second case hides lines before a sweep too: `[Hide(0, 0, 0), Hide(1, 0, 0), Push(0, 1), Sync(0, 1), Sweep(0), Delete(0, 0)]`, seed `f28c8a4cc317c8e3a2111d957103cffa96c9813f8efce86117a1a2f2f90f9035`. Done when both seeds sit in the proptest regressions of that test and the test passes |
| `[x]` A hidden row undone after a merge keeps the derived layout | MVP | a four-peer proptest fails on `main`: Hide(0,0,0), Push(0,1), Undo(0), Hide(3,0,0), Push(3,1), Sync(0,3), Undo(0) trips the debug assertion "the derived layout is the lists'" in `remote.rs`. The view and the stored lists disagree after an undo of a block delete merged with another peer's. Done when that case is a named regression test and `group14::four_peers::every_reference_reads_its_live_element_or_ref` passes |

### Toile

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` The board context menu from the keyboard | MVP | no key opens the context menu, so a keyboard user reaches none of its commands. Done when `TOILE-UI-422` reads `verified` |
| `[x]` The grid context menu from the keyboard | MVP | no key opens the Trame context menu, so a keyboard user reaches none of its commands, where Toile and Murmure open theirs from the context-menu key and Shift+F10. Done when both keys open the grid menu at the active cell, its entries run from the keyboard, and a row reads `verified` |
| `[x]` The page context menu from the keyboard | MVP | no key opens the Prose context menu, so a keyboard user reaches none of its commands, where Toile and Murmure open theirs from the context-menu key and Shift+F10. Done when both keys open the page menu at the caret, its entries run from the keyboard, and a row reads `verified` |

### Prose

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` A Prose document costs less than ten times its Markdown | MVP | a document holds about 16 times the Markdown it was imported from: the Loro state, its oplog and the mirror the undo history keeps, by the import memory harness on 2026-11-13. That floor, not the import, now sets the Markdown bound of 3.5 MiB under the client budget of 100 MB. Done when a row of `ambre-spec-budgets.md` splits the cost by part, a CI test holds a document under 10 times its Markdown, and the Markdown bound is derived again from the new figure |
| `[x]` A failed import keeps the undo history | MVP | a Prose import that fails rolls the document back to its state before the import, and the rollback clears the whole undo history of the document. Done when a failed import leaves the undo history as it stood before the import, and a row states it and reads `verified` |

### Murmure

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Murmure measured on a phone | MVP | the Murmure budgets measured in WASM on a mid-range phone, in place of the desktop figures and the ratio they assume: segment replay and snapshot import, the JSON authorship check, the envelope open, the rotation of a space leave, the color emoji font size, and the two residency memory budgets. The `segment` bench of `ambre-murmure` is the starting point. It also measures a cold open end to end, relay read included, against the cold start row |

### Import and export

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[x]` A dense PDF page holds the slice in CI | MVP | `print_slice_picture`, the slice that draws a page with a picture, reads 2 728 514 instructions, and no CI ceiling holds it. No case measures a page dense with text and pictures. Done when `scripts/instructions.py` holds a ceiling on the picture slice and on a dense page, each under the 3 200 000 instructions of the row "Prose PDF export, one slice" |
| `[x]` A failed import is taken back after the history is cleared | MVP | a Prose import that fails midway is taken back by its undo entry, but once the history is cleared during the import nothing takes it back, and its slices already reached every peer, so the half import stays on all of them. Done when an import that fails after a history clear leaves the document as it was on every peer, its slices held back until the import ends or undone by a snapshot, and a row reads `verified` |
| `[x]` Sheet page breaks across merges | MVP | a merged block that straddles a page break prints its part on each page, clipped, and no row rules it. Done when a row of `ambre-spec-trame-pdf-export.md` states where a merged block that crosses a page break prints, and a test holds it |
| `[x]` Persisted page setup | MVP | paper size, margins and orientation stored with the document instead of asked at export |
| `[x]` PPTX import | MVP | a PowerPoint file opens as frames ordered into the deck, one per slide, with its speaker notes. The master and layout inheritance is flattened onto each slide. SmartArt, charts, animations and video land as a picture or are named in the conversion report, and a transition Toile lacks falls back to its own |
| `[x]` PPTX import within the Office ceiling | MVP | the reader and the JPEG thumbnail decode put the Office module at 1 654 287 raw bytes on 2026-11-14, past its 1 480 000 ceiling; the cut tried first, without the JPEG decode, left 1 528 559. Done when `scripts/wasm_size.py` holds the Office module under a ceiling of 1 660 000 named here, and `just size` passes on `main` |
| `[x]` Tables and charts of an imported deck | MVP | a PowerPoint table or chart lands as nothing unless the file carries a fallback picture; a table could land as text boxes per cell, a chart as its cached picture. Done when each lands as editable text or as a picture and a row reads `verified` |
| `[x]` OpenDocument presentation import | MVP | an `.odp` file opens as frames ordered into the deck, one per slide, with its speaker notes, through a codec in the Office family module beside the PowerPoint codec. It lands as the same deck the same file saved from PowerPoint would, with the same report. Follows PPTX import, whose mapping onto the deck it shares. Done when a rule holding it to the PPTX import reads `verified` |
| `[x]` Figma and Miro import | MVP | open a board exported from either, reporting every construct it had to drop. The reader runs in a Boards module of its own, under a ceiling of 160,000 raw bytes and a budget of 100,000 brotli |
| `[x]` PNG export | MVP | a board or a frame rendered back to a raster image, without blocking the visible frame |
| `[x]` The PDF module carries the PNG raster | MVP | the PNG raster put the PDF module at 2 204 715 raw bytes on 2026-11-15, past its 1,830,000 ceiling. Each cut was tried before the raise: tiny-skia without its encoder, zeno in its place, a coverage rasterizer of the module's own, and the PNG chunks written over the zlib stream krilla links. They took the module to 1 953 835; the ceiling is 1,960,000, `ambre-spec-budgets.md` section 5 names each figure. Done when `just size` passes for the PDF module |
| `[ ]` The PDF export's request pass holds its budget | MVP | the request of a deck export derives every slide's view at once: 0.93 ms for 50 slides on desktop on 2026-11-14, about 6.5 ms on the phone, growing with the deck, and no CI job holds it. Done when a ceiling of `scripts/instructions.py` holds the pass of the request |
| `[x]` A deck exports at full resolution | MVP | a picture reaches the page from its inline thumbnail, capped at a 256 px edge, so a deck prints at screen quality. The full blob is ciphertext on the relay and the Logic thread has no path that fetches and opens one; give it that path, which the export must then be able to await. |
| `[x]` The standalone showcase bundle | MVP | a board exported as a self-contained page that renders without the relay. It leaves the zero-knowledge boundary, and says so |
| `[x]` An import that writes a comment reply keeps its undo step | MVP | the import pushes a map into the reply list of a comment, a commit the undo history cannot replay, so it clears the history: one undo no longer takes such an import back (`PROSE-IO-051`), and a refusal in its last slice has no step to withdraw and leaves the partial import on every peer (`PROSE-IO-112`). Done when an import of a Word file with a reply is one undo step, and a test of each row holds it |
| `[x]` Office interop held by its input fixtures | MVP | each codec reads its file, writes it and reads it back into its own model (`PROSE-IO-073`), but its reader is its only oracle. A fault the reader and the writer share passes: a Word comment date read as UTC and written as UTC comes back equal and still two hours off the time Word shows in Paris. Each input fixture of DOCX, xlsx and the OpenDocument formats is imported, exported and imported again, and the round trip fails on any difference the import report does not count. The comparison reads data, never displayed text: an instant in UTC, a number, a formula. An import that reads a local time takes the time zone as a parameter, never from the machine. Comparing the bytes of two files is rejected: zip order, timestamps, namespace prefixes and `rsid` differ with no defect. Split from the three oracles so no office format ships held by its own reader alone. PowerPoint has its own story, PowerPoint held by its input fixtures. Done when the fixture round trip of DOCX, xlsx and the OpenDocument formats runs in `cargo test` and its rows read `verified` |
| `[ ]` The share dialog shows the invite link | MVP | the link pinned to the inviter goes to the clipboard after the invite lands, outside the gesture that confirmed it; a browser that requires a fresh user activation (Firefox) refuses the write, and the inviter never sees the link. Done when the share dialog shows the last invite link with a copy button, and a row states it and reads `verified` |
| `[x]` PowerPoint held by its input fixtures | MVP | the fixture round trip of DOCX, xlsx and the OpenDocument formats runs (`PROSE-IO-114`, `TRAME-IO-080`), and PPTX was left out while its codec was rewritten. Each PPTX input fixture is imported into Toile, exported and imported again, and the round trip fails on any difference the import report does not count, compared as data. Split from Office interop held by its input fixtures. Done when the PPTX fixture round trip runs in `cargo test` and its rows read `verified` |

### Calls

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Call client proofs on Android | MVP | two proofs on a Galaxy A55 under Chrome, before a video call reaches users: the audio call worker is built before them, since they measure the load of video. `Video calls` and `Call videos on the canvas` build the canvas path and VP8 the specs already rule, and a failed proof reopens them. The call worker seals about 1 500 chunks per second at p99 5 ms and 25 % of one core, while the Sync worker seals a file; else a phone shows one 720p and four 180p videos. Nine videos reach the canvas at 30 fps for 4 ms of UI thread per paint; else page elements carry them. A desktop with a GPU first fills the `Call video paint` row of the budgets spec, which the cloud container measured without one |
| `[ ]` Call client proofs on iOS | MVP | four proofs on an iPhone under Safari. The two proofs of Android hold at the same thresholds. The iPhone decodes one 720p and eight 180p VP8 videos for 30 minutes at 25 fps and 15 % battery; else every sender uses H.264. iOS audio holds 30 minutes with echo cancellation on and no crackle; else the calls spec states it as degraded |
| `[ ]` Call video stall on real hardware | MVP | the stall budget measured with real browsers on a machine apart from the SFU and its synthetic clients, so that no receiver is starved of CPU. The SFU and 46 synthetic clients on one host; two Chrome and two Firefox receivers on another, at 150 ms of round trip and 2 % loss, for 10 minutes. The figure is `totalFreezesDuration` from `getStats()`, dated in the budget register with both machines. Done when each received video freezes 1 s per 10 minutes at most; else the SFU switches to mediasoup behind the same signalling |
| `[x]` The Murmure padding test holds on every Firefox run | MVP | the browser test bound to `ZK-025` failed on Firefox with 4 distinct padded sizes where it tolerates 2, on a run where nothing leaked: the tolerance is a guess, not a bound derived from the padding rule, so the browser lane goes red at random. Done when the test reads a bound derived from the padding rule of `ZK-025`, the cause of the extra sizes on Firefox is named, the test passes on 20 Firefox runs in a row, and its row reads `verified` |

---

## 4. V1 -- THE RELEASE THAT KEEPS THAT TEAM  `[ ]`

The test of section 1: the absence of one of these stories is a cost, not a reason to leave. Every
row reads `V1`.

### Foundations: sync and relay

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` A removed picture's blob is collected, to design | V1 | a picture removed from a document leaves its object in storage until the document is deleted, and no rule says who collects it, or when. Done when the collection is written as contract rows of `ambre-spec-media.md` and `ambre-spec-object-store.md`, and each reads `verified` |
| `[ ]` A new migration rebuilds the relay | V1 | `sqlx::migrate!` and `#[sqlx::test]` embed `migrations/` at compile time, and cargo tracks no file there. A migration added alone leaves a stale build, whose tests run on the old schema and can pass. Done when a build script of `ambre-server` reruns on any change under `migrations/`, and adding a migration rebuilds the tests without touching a source file |
| `[ ]` The delta index keeps its size under churn | V1 | each row of `updates` is inserted, moved, then deleted, so the table and its indexes collect dead row versions. The default autovacuum clears them later than the churn adds them, and a bloated index writes more pages per delta. Done when a soak run of one day at the measured delta rate holds the size of `updates` and of its indexes under a bound that a row of `ambre-spec-budgets.md` states |
| `[ ]` The write-cost ceiling rises only through a story | V1 | `ceiling_raise.py` reads the size ceilings of `wasm_size.py` alone, so `CEILING` of `write_cost.py` rises with no story row. Done when the gate holds `CEILING` of `write_cost.py` as it holds the size ceilings |
| `[ ]` A refused dead letter keeps its delta reachable | V1 | when the dead-letter push fails after an index or pending failure, the relay logs and counts it, but the delta survives only in its index row if any. Done when each path that ends in a refused dead letter is listed, the one with no copy left is refused upstream or stated as a known loss, and a row reads `verified` |
| `[ ]` A page hidden flushes the send window | V1 | a tab closed inside the 25 ms send window drops the held delta from the wire. It is saved locally and sent again at the next open (`AUTHOR-028`) only if the sealed store write landed. Done when a `pagehide` sends a flush command the Logic thread honours before the page goes, and a row reads `verified` under a browser test |
| `[ ]` The send window is measured on a phone | V1 | the 25 ms send window is the client share left by the sum of the P95 targets of `ambre-spec-budgets.md` section 1, most of which no run measures. Done when the keystroke probes read the client path of one key on the reference device, and the window row is set again from that measure |

### Foundations: security and zero knowledge

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` A provider account is unlocked or set up | V1 | the home screen has no passphrase field and no setup, so an account that holds its keys cannot open them and one that owes them is never asked. The setup refusal reaches a screen that ignores it. Needs `The provider sign-in reaches the client` first. Done when `CHROME-097` to `CHROME-100` read `verified` |
| `[ ]` A log line carries no identifier formatted into a text | V1 | the gate of `ZK-030` reads the type of each logged value, so an identifier first formatted into a `String` passes. This covers a foreign error that echoes a key, or an S3 key held as bare text. Done when the relay holds every store and object key in a type the gate refuses, and a test reads every info line of the db tier for no UUID text |
| `[ ]` The scrape of the relay and the SFU answers a token off the loopback | V1 | the metrics listener answers anyone who reaches its address, so a deployment that binds it to a shared network shows its load figures to that network. Done when a non-loopback address needs a token from the secrets module, and a row reads `verified` |
| `[ ]` A snapshot follows each rotation | V1 | no snapshot seals the state after a rotation, so the history of a document stays a chain of deltas across the key change |
| `[ ]` The author of a sealed object is a member | V1 | a name or a blob carries its author key, and the open checks the signature against that key without asking whether it belongs to a member. Needs the signed membership log first. Done when `SYNC-135` reads `verified` |
| `[ ]` The relay refuses an entry whose exchange key is not the member's | V1 | the relay checks that an entry names the signing key the member registered (`SRV-104`), and no check covers the exchange key the entry now signs (`SYNC-197`). An honest relay could then store an entry whose key differs from the member's own: that member gets no copy at the next rotation, and no one knows why until then. Done when a row binding the refusal of an entry naming another exchange key than the registered one reads `verified` |
| `[ ]` A member left out of a rotation gets the key back | V1 | a member whose row no entry signs, or whose key is malformed, is reported and then loses the document for good (`SYNC-043`, `SYNC-199`); a document older than the log loses every member but the rotator at its first rotation. Done when a row binding a path back for such a member, through a fresh signed invite, reads `verified` |
| `[ ]` A member's new exchange key enters the log | V1 | the exchange key of a member is fixed by the entry that adds them; a role change and a revoke carry it unchanged (`SYNC-197`). A user who changes exchange key, on a new device or after a recovery, gets no copy at the next rotation, and no path writes a new entry for it. Done when a signed entry replaces a member's exchange key, the rotation wraps for the new one, and a row states it and reads `verified` |
| `[ ]` A removed writer names no current key | V1 | a link counts when its signer held a writing role at any point, so a removed writer and the relay together can seal a key of their own under a link that passes; the relay can also withhold the newer links that would show the key superseded. Done when a row binding the signer of the link that names the current key to a writer present at that point of the log reads `verified` |
| `[ ]` A key the relay substituted is told apart | V1 | a key that no member vouched for is refused at open as `KeyNotReceived`, the same as a key that never came. The user cannot tell a hostile relay from a slow one. Done when the bus carries a refusal of its own for it, the screen shows it as a security warning, and a row states it and reads `verified` |
| `[ ]` A document labelled Murmure by the relay is checked | V1 | the open skips the key checks of a Murmure document by its type, and the type comes from the relay's listing: a relay can relabel any document to skip them. Done when the type the open trusts comes from a sealed source, or the checks run for every type, and a row states it and reads `verified` |
| `[ ]` A peer sees a member writing behind the log | V1 | an owner who opens a delta under a stale key carrying an older head learns nothing. Done when a row binding that detection reads `verified` |
| `[ ]` The page refuses a frame and gives its workers a policy | V1 | a meta policy cannot refuse a page that frames this one, and a worker takes its policy from the answer that serves its script, so the Sync worker and the call worker run under none (`SRV-260`). Done when a row binding `frame-ancestors 'none'` and a worker policy to the answers of the deployment's page server reads `verified` |
| `[ ]` The page policy holds under Firefox | V1 | the browser test installs its violation listener through the DevTools Protocol, so Firefox never runs it. Done when `SRV-260` names a Firefox mechanism |
| `[ ]` A web embed loads only the sites a deployment names | V1 | the page policy admits any `https:` page in a frame, so a Toile web embed loads any site, sandboxed without its origin. Done when the deployment names the sites an embed may load, the post-build hook writes them into `frame-src`, and a row reads `verified` |
| `[ ]` A sealed file takes the strength floor | V1 | the passphrase that seals an `.ambre` file meets no floor (`ambre-spec-docfile.md` section 6), so whoever holds the file guesses offline at the cost of `ZK-035`'s attacker. The prompt refuses only an empty or mismatched passphrase. Done when a row binding the seal to a passphrase `ChosenPassphrase::check` admits, with its refusal shown at the prompt, reads `verified` |
| `[ ]` The setup screen shows why a passphrase was refused | V1 | the Sync worker answers a weak setup with `AuthFailure::WeakPassphrase` and its words, and no screen sends a setup yet. Done when a row binding the setup screen to the words of the refusal, read by a screen reader, reads `verified` |
| `[ ]` A rotation of the master key proves the old wrap | V1 | `POST /keys/master/rotate` replaces the wrapped seed on a session alone, so a stolen session locks its owner out of the seed (audit P3). Done when a row refusing a rotation that the account's signing key does not sign reads `verified` |
| `[ ]` An address search matches a whole address | V1 | the user search answers the address of any account for a two-character fragment, and the account setup route needs no session, so anyone enumerates every address (audit P4). Done when a row binding a search by address to a whole address reads `verified` |
| `[ ]` A client refuses a signed join older than its own | V1 | a signed join carries an instant no client checks, so a relay replays an old join to a later joiner, who wraps its sender key to it (audit P5). Done when a row refusing a join older than the client's own reads `verified` |
| `[ ]` An opened snapshot is the one asked for | V1 | a snapshot opens without a check that its sealed identifier is the requested one, so a relay swaps two snapshots of one document (audit P6). Done when a row refusing a snapshot whose sealed identifier differs reads `verified` |
| `[ ]` A first open with no pin of a copy another member wrapped is refused | V1 | an open with neither a pin nor a kept anchor still accepts the anchor the relay chose (trust on first use, which the owner declined). Done when such an open is refused or warned per the owner's choice, and a row states it and reads `verified` |

### Foundations: platform and architecture

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` The collation a locale selects for find | V1 | `ambre-spec-find.md` and `ambre-spec-locale.md` record that a byte comparison and a locale-aware one disagree, and leave open which collation a document's locale selects and for which operation. A find in a document tagged in one locale then matches as a find in any other. Done when a row of `ambre-spec-find.md` states the collation a find uses under a locale and reads `verified`, and `ambre-spec-locale.md` no longer names collation as open |
| `[ ]` The launcher search folds accents | V1 | `ambre-spec-launcher.md` and `ambre-spec-find.md`: a launcher query folds case alone, so "ecole" does not find "École". Done when a row of `ambre-spec-launcher.md` states the fold a query uses and reads `verified` |
| `[ ]` The launcher's relative times and bands refresh while open | V1 | `ambre-spec-launcher.md`: a card's relative time and its band are computed when the list or the query changes, so a list left open keeps saying "just now" and keeps a day-old write in the past day. Done when a row states when both refresh and reads `verified` |
| `[ ]` The document locale is chosen and read everywhere, to design | V1 | no menu writes a document's locale and nothing sets the reader's convention, Prose and Toile take no convention, no document says where the interface language comes from, and an array constant keeps its separators. Done when the rules of `ambre-spec-locale.md` section 6 are written as contract rows, and each reads `verified` |
| `[ ]` One device for every canvas | V1 | each canvas of a view holds a GPU state of its own, since a WebGL2 device presents only to the canvas that created it. A Murmure thread thus loads a second device, a second glyph atlas and a second font context. Under WebGPU one device can serve every canvas, and the glyph atlas can be shared. The Story also measures what a second canvas costs on a phone |
| `[ ]` `CANVAS P2` -- frame quality | V1 | frame profiling, allocation-free painting, dynamic theme, translation and right-to-left |
| `[ ]` Drawn content reaches a screen reader | V1 | a page and a board draw their content on the canvas, so a screen reader reads the chrome and none of the text being edited. The Prose page fills an accessible surface with the block under the caret and announces each move into a new context, and a state the canvas draws also reaches the page. Done when `CHROME-029` and the Prose accessible surface rows read `verified` |
| `[ ]` Antialiased vector geometry | V1 | every screen pipeline runs at one sample and the solid shader carries no coverage term, so a diagonal or a curve is a hard staircase. Only glyphs escape, through a CPU-rasterized coverage atlas. A high-density display hides it by paying twice the pixels; a 1x monitor does not |
| `[ ]` A board of photographs stays fluid | V1 | every upload lands in the frame that first draws it, so opening a board on thirty pictures decodes thirty thumbnails at once; and zoomed far enough out the texture budget evicts what the next frame asks for again, re-uploading pyramids whose top levels that zoom never samples. It wants an upload budget per frame and a chain truncated to the scale drawn at -- which needs the decoded pixels kept to re-upload from (`CANVAS-007`) |
| `[ ]` A web page loads only when asked | V1 | an embed fetches its page the moment a pan brings it on screen, so the site, and anyone on the way to it, learns that this user has this board open -- a flow that leaves the client, which the zero-knowledge boundary rules on the relay alone and has no row for. The user decides instead: ask, always, or never, remembered per site and never shared |
| `[ ]` The icon pipeline | V1 | vector icons on the canvas. The last open item of `CANVAS P0` |
| `[ ]` The adaptive chrome | V1 | the navigation, the side panels, the toolbar overflow and the wide data views adapt at the three declared widths. Today two labels and one bottom sheet move, and the regions the design names exist at no width |
| `[ ]` User settings | V1 | a sealed store of one user's settings that follows them across devices: Murmure mute and layout first, then macro approvals and secret values |
| `[ ]` Avatars | V1 | a picture per user, drawn in every application, held in the clear by the relay and served only to the users who share a document with its owner. Done when `PROFILE-001` and `PROFILE-003` to `PROFILE-009` read `verified` |
| `[ ]` The first spawn of the converter thread is timed | V1 | the first job now spawns the converter thread while its module downloads, and no run measures how long a worker takes to start on a phone, so the first decode may wait on the spawn rather than the fetch. Done when a row of `ambre-spec-budgets.md` holds the time from the first job to the thread running, measured on a phone, with its date, its device and its command |
| `[ ]` A converter thread held lost is ended | V1 | a converter thread past its answer deadline is dropped, never ended: its worker and its stack stay in the shared heap for the session, up to three of them (`RESPAWN_LIMIT`). Done when a thread held lost is terminated and its memory returned, or a row of `ambre-spec-budgets.md` counts what it keeps and reads `verified` |
| `[ ]` The converter modules shrink back | V1 | three module ceilings rose with HTML interop, Images and the JPEG story: the markup module, the images module from 700 000 to 830 000 raw bytes and the tables module from 130 000 to 140 000 (`ambre-spec-budgets.md section 5`). Each stays under its brotli budget. Done when a cut lowers each of the three ceilings, measured by `just size` |
| `[ ]` The relay links no other client-only module of the core | V1 | the split moved the document file, the export file and the locale, but `ambre-core` still carries modules only the client reads, for example fonts, colours, units and pixel spaces, layout, text offsets and calendar. The relay compiles them all the same. Done when each `ambre-core` module that the relay, the wire and the SFU never name sits in `ambre-client-core`, and a test lists the modules the relay side reads |
| `[ ]` The UI thread holds one copy of each font face | V1 | the UI thread registers each fetched face into its context and keeps a second copy to ship to the Logic thread: `LazyFontLoader::drain_into` clones the bytes, and boot ships the base set beside its own context. The Logic thread now holds one copy (`CANVAS-097`). Done when a row binding one copy per face on the UI thread reads `verified` |
| `[ ]` The bus refuses an application's message by what it is | V1 | the driver holds `BUS-032` by a name: an enum of `ambre-bus` whose name ends in `Command` or `Event` and is no envelope. The shared interop shapes were named `InteropRequest` and `InteropAnswer` partly to pass it (`ambre-spec-interop.md`), so a renamed application message would pass too. Each enum and struct of `ambre-bus` a payload carries is either generic over the application's parameter or one line of a register with its reason, and the driver refuses any other, whatever its name. Done when `BUS-032` reads `verified` under that mechanism and a test plants an application enum under a neutral name |
| `[ ]` One find and replace | V1 | one match session with a live cursor, in every app, instead of Trame's whole-grid re-scan |
| `[ ]` One zoom model | V1 | one zoom engine behind the shared widget: Trame gains keyboard and pinch zoom and zoom-to-selection, Prose gains zoom. Pinch belongs here, in every app, and a finger's pan and flick to the "One kinetic scroll" row |
| `[ ]` One text-editing substrate | V1 | one editing core under both layout drivers: the input pipe, composition, the caret and selection model |
| `[ ]` One resize command | V1 | one row and column resize primitive for both table models. No shared table object |
| `[ ]` One media cache | V1 | every decoded image goes through one bounded cache, Toile and Trame included |
| `[ ]` One repaint contract | V1 | Prose and Toile honour the shared partial-repaint modes instead of a parallel mechanism |
| `[ ]` One undo bound | V1 | a bounded undo depth in every app. What merges into one step stays per app |
| `[ ]` One name for the moving selection end | V1 | `focus` everywhere; the fixed end stays `anchor`. A rename |
| `[ ]` One floating-menu shell | V1 | one placement, flip and keyboard model under every popover and floating bar |
| `[ ]` One performance read-out | V1 | the frame read-out available in every app, not in Trame alone |
| `[ ]` The launcher's perf read-out and TOFU pin read in modules | V1 | the frame loop of `lib.rs` still mirrors the perf snapshot into the status bar and reads the TOFU pin inline. Done when each concern is one module and `lib.rs` keeps the boot and the frame loop alone |
| `[ ]` A format undo across a peer's deletion | V1 | the replay of a format step (`UNDO-015`) writes nothing when a peer deleted characters inside the formatted range, as `ambre-spec-undo.md` section 3 states, so the user sees an undo that does not work. Done when a rule of `ambre-spec-undo.md` for that case reads `verified` |
| `[ ]` A same-tab paste on a browser refusing the web types | V1 | the ownership token rides in the HTML when the `web ` types are refused, and a Trame or Toile copy carries no HTML. Such a paste falls to rung 3. Done when `UI-THREAD-056` reads `verified` |

### Build, CI and verification

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` The crate table gate runs as its own CI step | V1 | `scripts/crate_table.py` runs under `just crate-boundaries`, because `just primitives` refuses a lint line that compiles nothing. So a red step names the boundary gate when the table is wrong. Done when a `just crate-table` recipe runs the gate alone, as its own step of the CI `lint` job |
| `[ ]` The newtype census reads an associated type an impl downstream gives | V1 | the census of `ARCH-062` reads a field `F::Repr` through each impl of its trait that the compile of the struct sees. An impl in a crate downstream of the struct is out of that compile, so a struct whose only primitive instance comes from a downstream impl stays out. No struct of the workspace is in that case today. Done when a test of the gate refuses a struct whose `u32` instance comes from an impl in a downstream crate and has no census line |
| `[ ]` A view module directory no gate reads fails the gate | V1 | the view gate reads the `ui`, `view`, `render` and `chrome` modules of an application off the path, so a view moved under another directory name leaves Trame, Toile or Prose unguarded in silence. Murmure is held by `UI-THREAD-062`. Done when a test fails for an application crate whose UI-thread code sits outside those names |
| `[ ]` The view gate reads an engine state held under no name | V1 | the view gate reads paths, method calls and signatures. A local bound by inference to a value that holds the document passes when nothing on its line names it. Done when the gate reads the type of each local in UI-thread code, and a planted breach through an inferred local is refused |
| `[ ]` The panel modal role and the pipe attributes read by rustc | V1 | `scripts/crate_boundaries.py` still reads `aria-modal`, `aria-hidden` and `inert` by regular expression (`CHROME-028`, `CHROME-025`, `CHROME-067`), yet `view!` writes them as toolkit attribute methods rustc resolves, so a helper macro or a static subtree hides them. Done when the driver reads each attribute where its call or the inert HTML takes it, and the script keeps only the role and tab-order strings |
| `[ ]` A guard bound to the element it guards | V1 | the driver accepts a drag control, a menu or a frame when its file names a `Debouncer`, a menu key handler or a `sandbox` anywhere, as the script did. So a second control in the same file rides the guard of the first. Done when each guard is read on the value or the element it guards, and a planted breach of two controls in one file is refused |
| `[ ]` A drag type or a frame tag held in a constant read by the driver | V1 | the driver reads the literal a call takes, so `const KIND: &str = "range"; .r#type(KIND)` passes, as it passed the script. Nothing in the tree uses it. Done when the driver evaluates a `&str` constant argument, and a planted breach through a constant is refused |
| `[ ]` Reproducible counts through Loro and parley | V1 | under the fixed hash key, `remote_batch` still moves by about 0.1 % and `paragraph_shape` by about 0.3 % on 2026-11-10: Loro's block cache keys `ahash` at random, and parley and fontique seed hashbrown from the clock (`ambre-spec-testing.md section 2`). Done when each counts the same total in three runs and its ceiling follows the rule of that section |
| `[ ]` The shared crates cite no application rule | V1 | `ambre-app`, `ambre-logic`, `ambre-draw`, `ambre-canvas`, `ambre-bus` and `ambre-chrome` cite Prose and Toile rules outside a file that names the application: a shared layer that knows one application, or a rule that belongs in the foundations. Done when `spec_code_citations.allow` holds no `owner` line |
| `[ ]` A mechanism rung for a CI job outside lint | V1 | `ARCH-110` names `ci-gate` for a probe the `devcontainer` workflow runs, while the template reserves the token for the `lint` job. Done when the ladder of `ambre-spec-testing.md section 2` gains a rung for a CI job that runs a stack, or the token widens to cover it, and `ARCH-110` cites it |
| `[ ]` The four-peer proptest runs long on a schedule | V1 | `every_reference_reads_its_live_element_or_ref` at 512 cases missed two cache bugs that 100 000 cases found in minutes, and each failing seed makes the coverage lane flaky until fixed. Done when a scheduled job runs it with a high `PROPTEST_CASES` and reports a new seed without blocking a branch |
| `[ ]` One copy of each sort in Main | V1 | on 2026-11-15 `twiggy` read about 314 KB of stable sorts and 223 KB of unstable sorts in Main, one copy of about 3.5 KB per element type and per closure, and about 164 KB of stable sorts over Ambre types alone. The cut of `Main back under its ceiling` shared seven callers through `ambre_core::order`; Trame sheet order, Prose regions, Murmure channel order, the `Id<Message>` and `Id<Channel>` sorts, the probe percentiles and about 120 more call sites still link their own copy. Done when every sort of an application crate goes through `ambre_core::order` or a lint names why not, and Main shrinks by the figure measured |
| `[ ]` A collect into a map in Main is a loop | V1 | a `collect` into a `BTreeMap` sorts the pairs with a stable sort of its own before it builds the map, about 4.5 KB of wasm per key and value types. Two were cut on 2026-11-15; `ambre-opc` and the `loro` maps still carry one. Done when a gate refuses a `collect` into a `BTreeMap` or a `BTreeSet` in a crate Main links |
| `[ ]` The belt reports every uncaught page error | V1 | only the two IME tests record uncaught errors, so any other belt test that a trap breaks fails as a timeout. Done when the driver records `error` events on every page and every wait fails with the stack |

### Trame

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Paste as plain text in Trame | V1 | the plain-paste chord and a context-menu entry land the plain-text mirror as literal cells, the typed payload and the tab's own copy ignored |
| `[ ]` Functions that answer a reference | V1 | a function given a range answers a reference into it, and a lookup names its result column instead of counting it. Done when `TRAME-057` and `TRAME-058` read `verified` |
| `[ ]` User-defined functions | V1 | a user names a formula of their own and calls it. Done when `TRAME-059` reads `verified` |
| `[ ]` Dynamic arrays and spill | V1 | a formula answering many values fills the cells beside it, refuses a blocked cell and clears what it no longer covers. Done when `TRAME-060`, `TRAME-206` and `TRAME-207` read `verified` |
| `[ ]` Data tools | V1 | personal filter views, conditional formatting, data validation, and a total that skips filtered rows |
| `[ ]` Grid surface | V1 | frozen rows and columns, function autocompletion, live peer cursors, and a mark on every row and column header the selection spans. Done when `TRAME-UI-019`, `TRAME-UI-029`, `TRAME-UI-057`, `TRAME-UI-170`, `TRAME-UI-187` and `PRESENCE-016` read `verified` |
| `[ ]` Hide and show a sheet | V1 | an Excel import keeps a hidden sheet hidden, and no gesture shows it again or hides another: a tab menu entry for each, over the `hidden_sheets` container the import writes. Found by the Excel import |
| `[ ]` Cell comments | V1 | a note anchored to a cell, surviving a sort and a reorder |
| `[ ]` Media in a sheet | V1 | Trame declares images and resolves none. Wire it to the shared media flow |
| `[ ]` A machine-readable error | V1 | an error carries a code a host can branch on, not only a sentence |
| `[ ]` Bounded float objects | V1 | an absurd image or chart size cannot make layout cost unbounded |
| `[ ]` A fill extrapolates a date series | V1 | a drag over two dates repeats them: the fill extrapolates numbers alone, and a date pattern the corpus promises is detected nowhere |
| `[ ]` Trame answers once per pass | V1 | since `ARCH P9` the Logic thread answers each document once per pass, but the grid engine still recomputes its filters, its totals and the visible cells after every command. A burst of typing or a fast fill pays that once per command, all but the last stale. Scheduled only if the Keystroke latency probes Story (`ambre-roadmap.md` E6) shows the grid falling behind |
| `[ ]` Links in Trame | V1 | clickable per-cell links, on the link and open-URL model shared with Prose and Toile |
| `[ ]` A sweep slice that fails halfway still commits what it wrote | V1 | `Axes::sweep` returns `Swept::default()` when a list delete fails after earlier deletes of the same slice succeeded, so `sweep_slice` reads no write and skips the commit: the deletes wait in the open transaction for the next local commit, an undo step, and the cached axis view misses them -- the class of the register-only slice fixed by `A hidden line leaves the cached axis after a sweep`. Unreachable while the cached view matches the list. Done when a slice commits whenever the document holds pending operations, and a test forces a failing delete |

### Toile

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` A group cycle a peer writes is broken | V1 | a peer can store a group among its own members, or two groups in each other's. Every walk now ends on such a cycle, but a group on it keeps a stale box on the incremental path that a rebuild gives none, the one exception `TOILE-186` names. Every reader of the group tree treats a group on a cycle the same way, and the exception goes (`TOILE-276`, `TOILE-UI-398`) |
| `[ ]` Paste as plain text in Toile | V1 | the plain-paste chord and a context-menu entry land the plain text as one text element at the pointer, where a paste of text today lands nothing |
| `[ ]` Snap to the board grid | V1 | a drag, a resize and a nudge quantize to the grid step the board asks for, which is drawn today and magnetic nowhere |
| `[ ]` Rich text on the board | V1 | a note body carries per-run formatting, on the shared text substrate Prose unblocks. Today it carries one style per element |
| `[ ]` Auto-layout reaches a frame | V1 | the solver has no caller outside its own tests and the command has no menu entry, no inspector row and no chord: a layout rule is stored, converges, and moves nothing on screen |
| `[ ]` Boolean operations reach a surface | V1 | union, intersection and difference are built and tested behind a command no menu entry, no button and no chord invokes |
| `[ ]` Every connector shape a user can make | V1 | the format stores path kinds and anchors the tool writes for none of them, so a document can hold an arrow no gesture produces |
| `[ ]` Toile answers once per pass | V1 | since `ARCH P9` the Logic thread answers each document once per pass, but the board engine still sends its scene changes after every command. A drag or a burst of typing on a note pays that once per command, all but the last stale. Scheduled only if the Keystroke latency probes Story (`ambre-roadmap.md` E6) shows the board falling behind |
| `[ ]` Toile link navigation | V1 | a jump along a link leaves no way back, and a link to a deleted frame reads as a live one. Done when `TOILE-295` and `TOILE-296` read `verified` |
| `[ ]` Pressure-sensitive strokes | V1 | each freehand sample records a pressure that nothing reads, so a stroke keeps one width under a pressure-sensitive pen. Done when `TOILE-297` reads `verified` |
| `[ ]` Autoplay and loop in a presentation | V1 | the deck configuration stores autoplay and loop, and no playback reads either. Done when `TOILE-FEAT-113` and `TOILE-FEAT-114` read `verified` |
| `[ ]` A group write walks no board | V1 | a bind, an unbind, a move or a delete on a group re-reads the whole element map, so its cost grows with the board and not with the group. Done when `TOILE-294` reads `verified` |
| `[ ]` A board shows a picture at full resolution | V1 | the screen draws each picture from its inline thumbnail, capped at a 256 px edge. The deck export fetches and opens the stored file of each picture; the paint could take the same path, showing the thumbnail while the full blob loads. Done when `TOILE-IO-035` reads `verified` |

### Prose

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` The same page on two architectures | V1 | two peers pin the same faces and the same shaper, and `PROSE-109` compares two font contexts on one host. Nothing compares two machines, where hinting, curve evaluation and a maths library may not accumulate alike. An `insta` snapshot of a layout measured on the host, replayed under `wasm-bindgen-test` in a browser and compared byte for byte. The wasm tier already runs in headless Chrome, and runs no Prose layout test yet |
| `[ ]` Suggestions beyond typing | V1 | suggestion mode records typing and deleting only, and refuses a split, a join, a block command or a format change while it is on (`PROSE-FEAT-301`). A burst groups by adjacency alone, since the merge window it names rests on an undo interval no document defines (`PROSE-FEAT-085`) |
| `[ ]` A note edit reflows only its page | V1 | every reflow reads the text of every note to decide whether the notes still hold, and an edit to one note's text lays the whole document out again, since its height moved. Past a few hundred notes each keystroke pays for both. Keeping each note's height with its text, and resuming placement at the first page that note sits on, would bound it to the page |
| `[ ]` A long paragraph typed into within the layout budget | V1 | the view shapes a window around a key, and a key whose breaks never fall back shapes the rest of its paragraph. Shaping is most of the cost and Parley shapes no text twice, so the budget holds 500 characters at worst, not the 2 000 of a long paragraph (`PROSE-UI-215`). Breaking lines over glyphs shaped once per word would shape the edited word alone |
| `[ ]` The format bar's other controls | V1 | the bar carries the five presence marks alone. The link, heading, quote, colour and size controls, superscript, subscript and clearing the formatting reach no control yet |
| `[ ]` The slash menu's other items | V1 | the menu reaches nine features through twelve items. A picture, a footnote, a cross reference, a comment, an equation and the heading levels past three have no item |
| `[ ]` Rich paste: tables and pictures | V1 | an HTML table lands as a table and a pasted picture lands in the text, where the reader now keeps one paragraph per cell and drops the picture |
| `[ ]` Hanging indents | V1 | a negative first-line indent starts the first line left of the rest, as a bibliography or a definition list sets it. The layout carries the indent as a box ahead of the text, which has no negative width, so the value is laid out as none and the ruler's handle stops at the left indent |
| `[ ]` Picture handles and menu | V1 | the resize, alignment, alternative-text and caption commands reach the Prose engine, and nothing on the page sends them. Corner handles that resize with the aspect locked, edge handles that resize one side, both on release, and a picture menu with the four alignments, the alternative text and the caption. Done when each gesture has a row that reads `verified` |
| `[ ]` Text wrapped around a picture | V1 | a picture aligned left or right is stored and laid out as a block, because the shaper has no exclusion zones. Lines that break around the box of a picture, on screen and in the PDF, so an imported Word file with a wrapped picture keeps its layout |
| `[ ]` The text tree of Loro holds less per character | V1 | the rest of the Prose memory ratio, 14.8 times its Markdown on 2026-11-15, sits in the B-tree of `generic-btree` under Loro, which the workspace does not own. Done when a change that shrinks its nodes is proposed upstream and either lands in a release the workspace takes, or, if upstream stalls, a `[patch]` fork carries it with its reason in `ambre-spec-budgets.md` |
| `[ ]` An announced session draws no caret it did not place | V1 | a Prose session announces itself once its document is ready, and a session that never placed its caret is drawn at the start of the first block on every peer's screen. Done when `PROSE-130` reads `verified` |
| `[ ]` A Prose view that lost an event catches up | V1 | a Trame or Toile view that lost an event is resent its document, and a Prose view is resent nothing: it reconciles its text change by change, so a lost change stays out of its count until a reload (`BUS-043`). Done when `BUS-043` reads `verified` |

### Murmure

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` A composed name that follows its members | V1 | the members a direct message shows are read with the listing: one who leaves, or changes their display name, shows as before until the next listing. A long list is drawn whole, where it should name a few and count the rest |
| `[ ]` A stale channel name | V1 | the name sealed beside `meta` is a hint: a seal that failed to reach the relay, or two renames racing, leave the older one until the channel opens. A relay can also serve an older seal, never forge or read one. Re-seal on open when the hint differs from what meta reads |
| `[ ]` The members of a channel | V1 | the member list shows the members of a space, so a private channel or a direct message offers none. Their members are a subset the relay already lists per document, and the list would read them there |
| `[ ]` The unread line in a thread | V1 | the line marks only the first unread message of a channel, so a thread opened on new replies shows none. The thread already keeps a read cursor of its own, and the line would read it there |
| `[ ]` The hover bar of a message | V1 | a pointer over a message shows no actions, so each one takes a right-click. A bar of the most used commands, painted on the hovered row, would read the same command table |
| `[ ]` Pin, bookmark and copy link | V1 | the menu of a message offers reply, mark unread and copy text. Pinning a message, bookmarking it and copying a link to it need a document field and a link scheme no spec defines yet |
| `[ ]` A copied quote that pastes back with its mentions | V1 | a copy from the list writes the `text/plain` mirror alone. The private type of `ambre-spec-clipboard.md` would carry the mentions and the message of each part, which a paste in the composer turns back into a quote. Nothing reads that type in Murmure yet |
| `[ ]` Retrying a closing segment in place | V1 | a retry of the closing segment the channel opened without closes and reopens the whole channel, so its watchers and its typing go and come back. Loading it into the open channel needs the engine to adopt a closing segment after the open |
| `[ ]` The caret after an undo | V1 | an undo or a redo in the composer puts the caret at the end of the draft, since the history keeps text alone. Each step would keep its caret, as Prose does |
| `[ ]` A ceiling on the channels watched unopened | V1 | every channel never opened costs one subscription per live segment, with no bound. A member of many channels pays a socket subscription each. A budget row and an order that keeps the most recent channels would bound it |
| `[ ]` Moderation | V1 | a moderator hides the message of another member |
| `[ ]` Custom emoji | V1 | emoji a space admin adds for the members of that space |
| `[ ]` Presence settings | V1 | turning off the presence and typing indicators, kept in the user settings |
| `[ ]` Presence states | V1 | away after an idle delay and busy set by the member, each with its own mark in the member list, and a status that follows the member to every device. Done when `MURMURE-447`, `MURMURE-456` to `MURMURE-458` and `MURMURE-459` read `verified` |
| `[ ]` Push notifications | V1 | a notification with every tab closed, a level per channel, and mentions routed by the relay |
| `[ ]` Wake a hidden tab from the worker | V1 | a browser may slow the timers of a tab hidden for minutes, so its relay runs late and a notification lags. A message from a worker wakes the page as soon as an event waits, in place of the one-second timer of `CANVAS-075` |
| `[ ]` Mute and hide | V1 | a channel muted or hidden, kept in the user settings |
| `[ ]` The personal sidebar | V1 | personal groups and order of channels, and bookmarks, kept in the user settings |
| `[ ]` Followed threads | V1 | one view of every followed thread with unread replies |
| `[ ]` Search on a new device | V1 | a signed index per final segment, queried before its segment is fetched |
| `[ ]` Space retention | V1 | one retention duration per space, applied by the relay |
| `[ ]` Direct message conversion | V1 | a direct message becomes a private channel of a space, with its history or from a fresh start. A fresh start is a history boundary, which the message list shows as the start of the channel |
| `[ ]` Forum channels | V1 | posts with a title and tags, each opening its own thread |
| `[ ]` Chat import | V1 | an archive of Slack or Discord imported into a space. The import runs on the client and cannot sign as the archive's authors, so each imported message is attested by the member who imports it and names its original author as a label, not a signed identity. V1: a team switches with its old chat kept read-only, so a missing import is a cost, not a reason to stay |
| `[ ]` Murmure export | V1 | a channel written to one `.ambre` file, through a container version that holds several documents. V1: it is a backup and a way out, and no team keeps its old suite for want of it |
| `[ ]` Two channels side by side | V1 | a split view that pins and shows two channels at once, each with its own list and composer |
| `[ ]` Pinned messages | V1 | messages pinned to a channel for every member, in a `pinned` list of the channel `meta` document |
| `[ ]` A link to a message | V1 | a link that opens a channel at one message, for a member of the channel |
| `[ ]` Archived channels | V1 | a channel kept readable and closed to writes |
| `[ ]` Channel guests | V1 | a user outside a space invited into one of its channels as a guest, who sees that channel alone. The relay refuses such an invite today (`SRV-225`). Done when `MURMURE-600` reads `verified` |
| `[ ]` Murmure deltas leave Loro JSON, to design | V1 | a delta travels as a Loro JSON update (`MURMURE-223`), so Main links about 200 KB raw of `serde_json` and of the `json_schema` code of loro: about 45 KB brotli, by the main-size-cut estimate of 2026-11-11. Leaving JSON touches `MURMURE-223`, `MURMURE-226`, `MURMURE-227` and `MURMURE-228`, and bumps the schema (`MURMURE-020`). It reopens two rejected alternatives of `ambre-spec-murmure-messages.md` section 5, a fork of the segment and a decoder of our own for the binary format, which the story argues again. Done when the messages spec rules the delta format in rows that read `verified`, and `just size` lowers the ceiling of Main by the bytes saved |
| `[ ]` The Murmure runtime tests live in its crate | V1 | the runtime tests drive it through the Logic dispatch, so `ambre-murmure` exposes its state to them under its testing feature. Done when they run against the runtime and its outcome in `ambre-murmure`, and the testing accessors are gone |

### Import and export

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Pictures in a Markdown export | V1 | a Markdown export drops each picture and names it, because the file could name no address. Rule on a way out: a folder of files beside the text, a `data:` address, or the drop kept |
| `[ ]` A PDF page is drawn in bands | V1 | one pass draws one page whole, so a page denser than the CI case of `A dense PDF page holds the slice in CI` can pass the slice budget. The drawing of a page is cut into bands drawn over passes. Done when a page twice as dense as that case draws in bands and each pass holds the row "Prose PDF export, one slice" |
| `[ ]` Sheet print headers and gridlines | V1 | a printed page always draws the grid lines and never the row numbers or column letters. Done when the export reads both flags from the stored page setup, and a row binding each reads `verified` |
| `[ ]` Sheet PDF export budget | V1 | no budget row and no instruction count cover one page slice of the Trame export. Done when a row of `ambre-spec-budgets.md` bounds one slice and `scripts/instructions.py` counts `print_slice` under a ceiling |
| `[ ]` Sheet PDF browser test | V1 | `PDF-003` for Trame is held by a host view test alone. Done when the browser tier drives the palette entry of a sheet export, as Prose does, and `PDF-003` cites it |
| `[ ]` An imported deck keeps its groups | V1 | a PowerPoint group lands ungrouped and is named as approximated, where an SVG group is rebuilt. Done when a group of a slide lands as a board group and a row of the PowerPoint import spec reads `verified` |
| `[ ]` One XML tree for the office codecs | V1 | ambre-docx, ambre-odt, ambre-xlsx and ambre-pptx each own an XML tree over quick-xml; the ODP reader would add a fifth. Done when the codecs share one tree in the package layer and each crate drops its own |
| `[ ]` A deck import within its budget | V1 | no row of `ambre-spec-budgets.md` names the import of a deck and no bench reads it. Done when a budget row names it and the `import` bench reads a deck of 50 slides |
| `[ ]` Picture crop and text alignment on deck import | V1 | a cropped picture fills its box whole, and a centred text lands at the top left, as the board text has no alignment. Done when the crop is applied and alignment rows read `verified` |
| `[ ]` A running export shows its progress | V1 | Toile answers a progress report after each page of an export and each slice of an import, and no view draws it. Done when the shell shows the bar under a browser test |
| `[ ]` The Prose import fuzz reaches past one kilobyte of form | V1 | the `prose_import` target caps its input at 1 024 bytes, because postcard recurses once per nesting level and a deeper decode overflows the stack. So the fuzz never drives a large form across many slices. Done when the target decodes inputs of 64 KiB without a stack overflow, through a depth check or a bounded decoder, and a test holds the bound |
| `[ ]` Office interop held by three oracles | V1 | the fixture round trip holds the files someone wrote by hand (Office interop held by its input fixtures). A round trip from Ambre through DOCX, xlsx and PPTX and back is checked on documents a property test generates, for every construct the format can hold. A scheduled CI job opens each export in LibreOffice headless and compares what it extracts with what Ambre wrote. Each oracle compares data, never displayed text. How Ambre shows an instant or a number has rows of its own, tested under a fixed time zone and locale, so a display fault and an import fault never hide behind each other. Done when both run on DOCX, xlsx and PPTX and their rows read `verified` |
| `[ ]` Word run effects the Prose model can hold | V1 | small capitals and all capitals are dropped on a Word import, hidden text keeps its text but not its hiding, and a double strikethrough lands as a single one; each is named in the report (`PROSE-IO-086`), and none survives a round trip. The Prose model gains a form for each: the view draws it, the Word reader maps it and the writer writes it back. All capitals stays a look over the text as typed, never a change of the characters. Done when `PROSE-IO-086` names none of the four and a round trip from `pandoc/inline_formatting.docx` keeps them, its rows reading `verified` |
| `[ ]` A merged label in the text layer of one page | V1 | a merged label sits in the text layer of each page its block reaches, so a search finds it on a page that shows none of it. Done when a glyph run outside a page's clip is not written to that page, under a row that reads `verified` |
| `[ ]` A suggestion keeps its date | V1 | a tracked change from Word or OpenDocument lands without its date, and the import names it dropped (`PROSE-IO-118`). The suggestion entry holds a creation instant, and the Word and OpenDocument writers write it. Done when a dated tracked change survives an import and an export at its UTC instant |
| `[ ]` An import into a new workbook takes the default sheet's place | V1 | an import into a new workbook keeps the empty default sheet and renames the file's `Sheet1` to `Sheet1 (2)`; the export leaves the default sheet out (`TRAME-IO-082`), but the name the user reads has still changed. The import takes the default sheet's place when it holds nothing, or the report names the rename. Done when a workbook whose sheet is `Sheet1` lands under its own name |
| `[ ]` An empty default sheet a formula names still leaves | V1 | the export leaves out an untouched default sheet (`TRAME-IO-082`), even when a formula of another sheet names it; the file then names a missing sheet and Excel reads `#REF!`. Done when such a sheet leaves, and a test holds it |
| `[ ]` The sheet fixture round trip reads the document apart from the export map | V1 | `TRAME-IO-080` compares two documents as `excel_workbook` reads them, the export's own map, and a column width by its span alone. A loss that map shares passes. Done when the comparison reads the stored cells, styles and sizes, widths by value, and the row stays `verified` |

### Calls

| Story | Milestone | Touches |
| :--- | :--- | :--- |
| `[ ]` Call crypto review | V1 | an external review of the chunk format, the sender keys and the DTLS stack of the SFU, started once the format is frozen. No call reaches users before it closes with no high finding The owner moved it to V1 on 2026-11-15: the MVP ships calls on the internal pre-audit alone |
| `[ ]` A crossing offer in a browser | V1 | an offer of a member that crosses an offer of the SFU gets no answer (`CALL-114`), and the page then rolls its own back, answers the SFU and offers again. The SFU side is tested, and no belt drives the crossing in a browser, since the SFU offers its slots once, at the start of a call. Done when a belt test on Chrome and on Firefox makes the SFU offer while a first share offers, and the share and the camera both reach the other member |
| `[ ]` A receiver drops an older epoch once a newer one opens | V1 | a receiver keeps a sender's older epochs open until eight openers pass, so a chunk under a retired key still opens after the switch (audit P7). CALL-116 stops a removed participant at an honest SFU; this is defence against a malicious SFU. Done when a row refusing a chunk of an older epoch after a newer one opened reads `verified` |
| `[ ]` The browser tier holds the sender binding of the SFU | V1 | CALL-116 is held by the forwarding core on synthetic packets alone; no browser test sends a forged chunk through a real call (audit N11). Done when a belt row where a member's chunk under another index reaches nobody reads `verified` |

---

## 5. POST-V1 -- NOT SCHEDULED

### Foundations

| Story | Touches |
| :--- | :--- |
| `[ ]` Split-secret organization recovery | the escrow has a release path and no deposit path, and the quorum is clamped to one so the register advertises no guarantee the relay cannot keep. Threshold reconstruction, and a deposit, are what make the clamp liftable |
| `[ ]` Murmure bots and webhooks | bots holding their own keys, slash commands, interactive messages, and inbound webhooks through an encrypting bridge |
| `[ ]` Link previews | a preview of a web page fetched once by the sender and sealed in the message, and the name of a linked Ambre document for its members |
| `[ ]` Cross-app rich paste | a paste from one app into another keeps its structure instead of falling back to plain text |
| `[ ]` Machine members | a program joins a document as a member, marked as a machine, with its declared host, and its removal rotates the key |
| `[ ]` Write-only inboxes | a program posts into a document through an asymmetric inbox it cannot read, and a member client merges each post once |
| `[ ]` Addressed machines | a program reads only the envelopes a user addresses to it, under a key derived from the document key, and answers through the inbox |
| `[ ]` Document macros | a user saves a signed macro in a document, approves each version, and runs it as one undoable change |
| `[ ]` Macro triggers | a macro runs after a kind of local edit, in the editing client only |
| `[ ]` Macro network access | a macro calls a domain its user approved, held by the browser's content policy |
| `[ ]` Macro marketplace | a publisher signs a macro package, an editor installs it into a document, and an update waits for a new installation |
| `[ ]` Organization trust anchor | an organization signing key held by its owners, admin certificates it signs, and a fingerprint members verify |
| `[ ]` Organization macro catalog | a catalog document of macros an organization approves, and the policy its members' clients apply. Needs the trust anchor |
| `[ ]` Sealed profiles | the display name and the avatar sealed under a profile key wrapped for each contact, so the relay learns neither |
| `[ ]` `ARCH P4` -- offline durability | edit while offline and lose nothing on reconnect |
| `[ ]` Clipped paths on the canvas | a shape crossing a clip edge is drawn whole today. Clip it, and give a stroke a screen-constant width |
| `[ ]` Gradient brushes on the canvas | the screen surface paints a solid brush alone, so a path given a gradient paints nothing. Done when `CANVAS-092` reads `verified` |
| `[ ]` The Markdown writer moves to the markup module | Prose's Markdown writer still links into Main: it writes text from Prose's intermediate form, which no module can read without linking the application. It links no third-party crate, so it waits until the size of Main calls for it. Done when the writer runs in the markup module and `just size` lowers the ceiling of Main by the bytes saved |
| `[ ]` The clock of the converter deadline is named for a worker | the deadline of the converter thread reads an `Instant<PageLoad>`, but on a worker the origin is the worker's start, not the page load. Done when the clock marker names the origin it measures from on every thread, and the glossary carries it |
| `[ ]` The memory budgets add up | the UI-thread table of `ambre-spec-budgets.md section 6` totals forty-one megabytes over rows that sum to sixty-five: the picture upload budget was added beside the others and charged to no total. Prose then states fifty megabytes of video memory for three pages against a shared wgpu row of twenty, and the table records the arithmetic rather than settling it. A stated total its own rows exceed holds no regression |
| `[ ]` The page accepts trusted types alone | `require-trusted-types-for 'script'` would refuse a DOM injection the policy lets through, but Leptos writes templates through `innerHTML`. Done when a row binding the directive reads `verified` |

### Build, CI and verification

| Story | Touches |
| :--- | :--- |
| `[ ]` Hot-path benchmarks and the dependency audit | measured crypto costs, and a machine check that no plaintext crate reaches the boundary |
| `[ ]` The name resolver reads the manifests | five `Cargo.toml` files justify a dependency by naming the spec that needs it, and `spec_refs.py` opens neither a manifest nor any file outside `crates/**/*.rs` and `specs/**/*.md`, so those citations are held by nothing |
| `[ ]` The review status means something | every spec reads `Status: Draft`. The two other values `ambre-spec-template.md` declares are defined by no row and set by no one, so the corpus cannot tell a reviewed document from an unread one |
| `[ ]` Each surface's name read back from the page | each view names the shared text pipe after its content, and a test holds the words. Nothing holds the call: a view that names the wrong content, or an embedded one that renames the pipe, fails no test (`CHROME-024`). Done when a test mounts each application's view, top-level and embedded, and reads the name the pipe carries |
| `[ ]` Visual tests of canvas content | no canvas content is verifiable in CI: headless browsers read back a blank frame |
| `[ ]` The suite checked adversarially | `cargo-mutants` over the crates whose tests are generated, so a test that asserts nothing fails |
| `[ ]` Snapshot tests of the wire and the surfaces | `insta` over the recording surface's operation sequences, the wire encodings and the PDF primitives |
| `[ ]` A coverage gate on added lines | `cargo-llvm-cov` in diff mode, which turns a baseline nobody reads into a check |
| `[ ]` A worker pause the belt confirms | the belt pauses a worker through a DevTools message that gets no answer, so it never knows the pause took. A busy test sees its effect, and the image test sees none. Done when each pause waits for `Debugger.paused` over WebDriver BiDi |

### Prose

| Story | Touches |
| :--- | :--- |
| `[ ]` The later writing set | regular-expression find, bookmarks, version history, linked styles, pageless mode, line numbers, tables of figures, an index, a bibliography, section breaks and columns, protected zones, an equation editor |
| `[ ]` The spell checker | misspelled words underlined, with suggestions |

### Trame

| Story | Touches |
| :--- | :--- |
| `[ ]` Range protection | a range only its owner may edit, and the refusal the others see |
| `[ ]` Data tools, second pass | colour scales, data bars, top-N formats, and validation by range, date or pattern |
| `[ ]` Post-V1 surface | sparklines, charts, reference autocompletion, smooth scroll |
| `[ ]` The Google Sheets bridge | a workbook kept in step with Google Sheets. The bridge takes the workbook outside the zero-knowledge boundary, so it waits past V1 |
| `[ ]` Version history | named snapshots a user can return to, in place of an undo stack lost on close |
| `[ ]` Engine optimization | the indexes and parallelism a million-cell sheet needs, measured against benchmarks |
| `[ ]` Recompute on mobile hardware | measure the recompute on a target device. Past 50 ms at P95 it must yield instead of blocking |
| `[ ]` The shared style dictionary | the style indirection the schema prescribes and no code writes. Gated on a benchmark: it needs a reclamation policy first |
| `[ ]` Named formulas | `LAMBDA` and `LET`, and a formula saved under a name that any cell of the workbook calls as a function |
| `[ ]` Cached sheet indexes | a scroll on one sheet no longer rebuilds every other sheet's index |
| `[ ]` Stable engine sheet indexes | switching a tab no longer resets the engine |
| `[ ]` A recompute bounded by what changed | the cycle check and the topological walk cover the cells that changed, not the whole graph |
| `[ ]` Workbook PDF export | only the active sheet prints. Done when several sheets, or a sheet that is not shown, print into one file under a row that reads `verified` |

### Toile

| Story | Touches |
| :--- | :--- |
| `[ ]` The facilitation layer | template boards, cards with tags, a shared timer, a voting session and the surface that casts a vote, a minimap |
| `[ ]` Engine and render optimization | a large board stays fluid: bulk index updates, cheaper layer diffs, memoized routing, atlas eviction |

---

## 6. HOW TO RESUME WITH NO PRIOR CONTEXT

Read section 1, then section 3, the milestone in progress, and work its open rows. Open a spec
only when you start the row that names it. `just spec-context-story "<NAME>"` prints what the rows
of one story need, and says when that story is cheaper read whole. No other file schedules anything.
