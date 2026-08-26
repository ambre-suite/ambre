# AMBRE -- FEATURE: SPELL CHECKING

**Type:** Vertical (feature)
**Implementation:** design-only

> Misspelled words underlined as the document is written, with suggestions, checked on the reader's
> own machine. No word of the document is sent anywhere to be checked.

Nothing here is shared state: the errors are derived locally and written back nowhere, and a word
added to the dictionary belongs to the reader, not to the document. There is no two-peer table
below.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-019` | The spell checker MUST run locally and MUST NOT send document text anywhere. | this spec section 2; `PROSE-025` | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-041` | The check MUST run at the lowest priority of the Logic thread: it MUST NOT delay a keystroke, a layout or a synchronization. | this spec section 2 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-042` | The dictionary MUST be loaded as a separate asset, on demand; it MUST NOT be part of the main binary. | this spec section 2 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `PROSE-FEAT-043` | A word added to the dictionary MUST go to the reader's own encrypted settings; it MUST NOT enter the document. | this spec section 2 | not built -- lands with the "The spell checker" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** the spell check, a row of `ambre-spec-budgets.md section 1`. It is the one
  operation of this application that is allowed to be slow, which is what the priority rule buys.
* **Boundary.** the added word leaves the document and is written to the reader's settings store,
  under the reader's own key. Nothing new crosses the relay: `PROSE-FEAT-019` bans sending the
  text, `PROSE-FEAT-043` keeps the personal dictionary out of the shared document, and `ZK-001`
  seals what the settings store writes.
* **Verification.** 4 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-019` is held by the crate's own dependencies before a test exists: `PROSE-025`
  refuses `ambre-prose` a network path at all, so the text has nowhere to go.

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

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The check runs on the reader's machine | a spelling service is handed the document one paragraph at a time, which is precisely the content the relay is built never to see. There is no version of a remote checker that is compatible with the pillar, so the choice is local or nothing | a spelling service, which sends the document to a third party |
| It runs at the lowest priority | spelling is the only thing here nobody waits for, and it walks text on every change. Given any higher priority it competes with the keystroke path, and the cost lands where it is most visible | checking on the same priority as an edit, which makes typing wait for a dictionary lookup |
| The dictionary is a separate asset | a Hunspell dictionary is a large share of the initial download, and most readers of a document never open the spell checker in it. Fetching it on demand keeps the first load under its budget | compiling the dictionary in, which spends the load budget on readers who never use it |
| The personal dictionary is the reader's, not the document's | a word one reader knows is not a fact about the document, and writing it there would push a personal vocabulary to every collaborator and through the relay. Keeping it in the reader's own store also means no new data crosses the boundary | storing added words in the document, which shares a personal dictionary with every peer |
| Spelling, and no grammar | the grammar engines that could run on the reader's machine carry a model of more than a hundred megabytes, against a binary budget of two compressed. A grammar checker is therefore either remote, which the pillar refuses, or a download nobody would accept | a grammar model in the browser, which is fifty times the whole application |
| A pure Rust dictionary engine | the alternative is a C library through bindings, which the Rust-first pillar rejects and which would have to be built for the browser target besides | a Hunspell binding, which fails the Rust-first pillar |

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
* **The suggestion cap is stated in the shape and in no rule.** The type documents at most five,
  and no invariant holds it.
