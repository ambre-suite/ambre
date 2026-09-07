# AMBRE -- FEATURE: SPELL CHECKING

**Type:** Vertical (feature)
**Implementation:** design-only

> Misspelled words underlined as the document is written, with suggestions, checked on the reader's
> own machine. No word of the document is sent anywhere to be checked.

Almost nothing here is shared state: the errors are derived locally and written back nowhere
(`PROSE-FEAT-106`), and a word added to the dictionary belongs to the reader, not to the document
(`PROSE-FEAT-108`). One gesture does reach a peer -- accepting a suggestion replaces a word
(`PROSE-FEAT-109`) -- and it is an ordinary text edit, whose convergence
`features/ambre-spec-prose-text-editing.md` owns. There is no two-peer table below.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-019` | `INV` | The spell check MUST run on the reader's own machine. | this spec section 2; `PROSE-025` | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-102` | `INV` | The checker MUST NOT send document text out of the client, to a spelling service or to anything else. | this spec section 2; `PROSE-025` | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-041` | `INV` | The check MUST run at the lowest priority the Logic thread offers, behind every command, every layout and every synchronization. | this spec section 2 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-103` | `INV` | A check in progress MUST NOT delay a keystroke, a layout or a synchronization. | this spec section 2; derived | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-042` | `INV` | A dictionary MUST be fetched as its own asset, on the first check of its language. | this spec section 2 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-104` | `INV` | The main binary MUST NOT carry a dictionary. | this spec section 2; this spec section 4 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-105` | `INV` | A reported error MUST carry at most five suggestions. | this spec section 2; this spec section 3 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-106` | `INV` | The document MUST NOT hold the errors: they are derived from its text, on each reader's machine. | this spec section 2; derived | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Add a word to the dictionary

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-043` | `POST` | After the command, the word MUST be in the reader's own encrypted settings. | this spec section 2 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-107` | `POST` | After the command, that word MUST NOT be reported as an error again for this reader, in this document or in another. | this spec section 3 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-108` | `FRAME` | Adding a word MUST NOT touch the document, so no peer receives anything. | this spec section 2; this spec section 4 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.3 Choose a suggestion

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-109` | `POST` | After the command, the misspelled word MUST read as the suggestion chosen, as an ordinary edit that peers receive. | this spec section 2; this spec section 3 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-110` | `FRAME` | Accepting a suggestion MUST NOT touch text outside the error's range, nor the reader's own dictionary. | derived | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.4 Choose the language

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-FEAT-111` | `POST` | After the language is set, the document MUST be checked again against that language's dictionary. | this spec section 2; this spec section 3 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-112` | `FRAME` | Setting the language MUST NOT touch the text of the document. | derived | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** the spell check, a row of `ambre-spec-budgets.md section 1`. It is the one
  operation of this application that is allowed to be slow, which is what the priority rule buys.
* **Boundary.** the added word leaves the document and is written to the reader's settings store,
  under the reader's own key. Nothing new crosses the relay: `PROSE-FEAT-102` bans sending the
  text, `PROSE-FEAT-108` keeps the personal dictionary out of the shared document, and `ZK-001`
  seals what the settings store writes. The one thing that does reach a peer is the replacement of
  `PROSE-FEAT-109`, which travels as any other text edit does.
* **Verification.** 15 `unimplemented`, nothing else: `crates/ambre-prose` is a seven-line
  `lib.rs`, `ambre-bus` carries no Prose command, and `zspell` is in no `Cargo.toml`. Nothing holds
  `PROSE-FEAT-019` or `PROSE-FEAT-102`: `crates/ambre-prose/Cargo.toml` names no network crate,
  but the crate is empty, `PROSE-025` is itself `unimplemented`, and `deny.toml` bans
  `ambre-crypto` and `loro` by wrapper list while naming no network crate at all, so nothing
  refuses this one the path -- see section 5. Three rows stated
  two rules each, and in each pair the second half is the one a mechanism could reach:
  `PROSE-FEAT-104` is what a size check on the built bundle weighs, `PROSE-FEAT-102` is what a
  `deny.toml` entry would hold, and `PROSE-FEAT-103` is the difference between running last and
  yielding -- a pass that starts behind everything and then walks a hundred pages without giving
  the thread back delays the next keystroke while satisfying `PROSE-FEAT-041` whole.
  `PROSE-FEAT-105` is the cap section 5 records as living in the type and in no rule, and
  `PROSE-FEAT-106` is the ban the opening paragraph rests on. The three commands carried one row
  between them, the settings write: nothing said what accepting a suggestion leaves behind or may
  not touch, nothing said what setting a language rechecks, and nothing said that the added word
  stops being underlined, which is the whole of what the gesture is for. `PROSE-FEAT-105`,
  `PROSE-FEAT-107`, `PROSE-FEAT-109`, `PROSE-FEAT-110` and `PROSE-FEAT-112` are what a host test
  holds the day the checker exists; `PROSE-FEAT-103` needs a running thread and lands in the wasm
  tier of `ambre-spec-testing.md section 6`.

---

## 2. MODEL

**Containers.** None in the document. The personal dictionary is in the reader's settings store
(`PROSE-FEAT-043`), which the document knows nothing about.

**The checker and what it reports:**

```rust
pub struct SpellChecker {
    dict: zspell::Dictionary,
    lang: String,
}

pub struct SpellError {
    pub range:       (u32, u32),
    pub word:        String,
    /// At most five.
    pub suggestions: Vec<String>,
}
```

**When it runs.** At the lowest of the four Logic-thread priorities, behind every command, every
layout and every synchronization (`PROSE-FEAT-041`). A document being typed into never waits for
it, and a slow pass costs a late underline rather than a late character.

**How the dictionary arrives.** As its own asset, fetched when a language is first checked, never
compiled into the binary (`PROSE-FEAT-042`): the binary is capped at two megabytes compressed
(`README.md`, initial load), which one dictionary would spend on readers who never see it.

**The three commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `SetSpellCheckLang { lang }` | loads that dictionary and rechecks |
| `AcceptSpellSuggestion { block_id, range, replacement }` | replaces the word, as an ordinary edit |
| `AddToDictionary { word }` | writes the reader's own dictionary, not the document |

Errors reach the view as `SpellErrors`, one block at a time, and are held per block in the view
state (`features/ambre-spec-prose-engine.md section 3`). The wavy underline is drawn by the page
renderer (`features/ambre-spec-prose-page-rendering.md`).

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| a change to a block's text | rechecks that block, at the lowest priority |
| choosing a language | loads the dictionary and rechecks the document |
| a click on an underlined word | offers at most five suggestions |
| choosing a suggestion | replaces the word, as an ordinary edit that peers receive |
| adding a word to the dictionary | stops underlining it for this reader alone |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The check runs on the reader's machine | a spelling service | it is handed the document one paragraph at a time, which is precisely the content the relay is built never to see. No version of a remote checker is compatible with the pillar, so the choice is local or nothing |
| It runs at the lowest priority | checking on the same priority as an edit | it makes typing wait for a dictionary lookup. Spelling is the only thing here nobody waits for and it walks text on every change, so at any higher priority it competes with the keystroke path and the cost lands where it is most visible |
| The dictionary is a separate asset | compiling the dictionary in | it spends the load budget on readers who never use it. A Hunspell dictionary is a large share of the initial download, and most readers of a document never open the spell checker in it |
| The personal dictionary is the reader's, not the document's | storing added words in the document | it shares a personal dictionary with every peer, and through the relay. A word one reader knows is not a fact about the document, and the reader's own store means no new data crosses the boundary |
| Spelling, and no grammar | a grammar model in the browser | it is fifty times the whole application: more than a hundred megabytes against a binary budget of two compressed. A grammar checker is therefore either remote, which the pillar refuses, or a download nobody would accept |
| A pure Rust dictionary engine | a Hunspell binding | it fails the Rust-first pillar, and it would have to be built for the browser target besides |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing rules on where the language comes from.** A command sets it, and no rule says what it
  is before one does -- the document's language, the browser's, or none.
* **The personal dictionary does not follow the reader.** It is local to the browser for now, and
  syncing it through the settings channel is named as later work with no rule behind it.
* **Nothing rules on what is checked.** Whether a code block, a link target, a word inside a
  suggestion or the text of a header is checked is undecided, and each answer changes what the
  reader sees underlined.
* **Nothing rules on the cost of a first check.** Loading a dictionary and checking a long
  document is the one unbounded pass in the application, and only its priority is stated.
* **Nothing bans the network from this crate.** `deny.toml` holds capability bans as a crate name
  and the list of crates allowed to depend on it, and uses it for `ambre-crypto` and for `loro`.
  The client's network path is `gloo-net`, which `crates/ambre-sync/Cargo.toml` takes and no ban
  names. `PROSE-FEAT-102` is the half of `PROSE-025` such an entry would hold before
  `ambre-prose` has a line of code, and it is not written.
* **The Logic thread has no priorities.** `PROSE-FEAT-041` names the lowest of them, and no
  document declares any: `ambre-spec-message-bus.md` gives commands a class -- lossy or lossless --
  and no rank, and `ambre-bus` carries nothing of the sort. The rule is written against a
  mechanism that would have to be built first, and it is the only rule in the corpus that asks for
  one.
