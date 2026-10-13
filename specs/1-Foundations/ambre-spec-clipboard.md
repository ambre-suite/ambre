# AMBRE -- FEATURE: THE CLIPBOARD

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile
**Implementation:** partial -- the host path ships for two applications. No tier drives a
clipboard event, so the host half is held by reading it.

> Copy, cut and paste, the same way in every editor. What crosses to another application is
> readable text; what comes back into Ambre keeps everything the user copied.

Three editors copy three different things: a cell range, a set of shapes, a run of formatted
text. The path they travel is one. This document owns that path; each application owns what
it puts in the payload.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `UI-THREAD`, shared with
`ambre-spec-ui.md`, `ambre-spec-commands.md` and `ambre-spec-design-tokens.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UI-THREAD-006` | `INV` | The clipboard MUST carry a `text/plain` mirror plus one app-agnostic private type. | this spec section 2 | `ambre_app::host::tests::the_asynchronous_item_carries_everything_the_copy_published` -- the written item, read back, holds the mirror and the private type | none | verified |
| `UI-THREAD-043` | `INV` | An application MUST NOT define a clipboard type of its own. | this spec section 2, the four names | `ambre_bus::clipboard_tests::only_the_launcher_names_a_clipboard_type` -- the code of every crate is scanned, and a private type spelled outside the launcher fails it | none | verified |
| `UI-THREAD-053` | `INV` | Every view MUST choose between the copy it kept and what the browser read through one shared rule, and a plain paste MUST take the read. | this spec section 2, the ladder | `ambre_bus::clipboard_tests::a_paste_takes_the_kept_copy_only_on_the_token`; structural -- `ClipboardBlob::source` is the one site, and Trame, Toile and Prose each call it | none | verified |
| `UI-THREAD-024` | `INV` | Every paste affordance MUST reach the view through one path, whatever the gesture that raised it. | this spec section 3 | `ambre_app::host::pipe::tests::a_paste_event_and_the_paste_button_end_in_one_call` -- a raised `paste` event and a button press each end in the host's one paste call | none | verified |
| `UI-THREAD-025` | `INV` | The host MUST NOT name an application on a clipboard path: it MUST address the active view. | this spec section 2 | `ambre_app::host::pipe::tests::a_paste_event_and_the_paste_button_end_in_one_call` -- both paths reach a host that names no application. Structural: `pipe::wire_pipe` and `pipe::button_paste` take any `PipeHost`, so no application type is in reach | none | verified |

### 1.2 Copy or cut the selection

The payload does not exist when the key fires, so the copy is two writes: one on the event, one a
frame later. What a cut deletes, and when, is each application's own policy.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UI-THREAD-010` | `POST` | A copy MUST be built by the Logic thread from the whole document, never culled to what the viewport holds. | this spec section 2 | `ambre_trame::logic::tests::group7::copy_range_covers_the_full_selection_not_a_viewport` -- Trame's half. `ambre_toile::ui::view::view_tests::a_copy_names_the_whole_selection_and_sends_nothing_else` -- Toile's half, an id past the viewport included | none | verified |
| `UI-THREAD-008` | `POST` | A deferred copy MUST prevent the browser default, so an empty write never clobbers what the clipboard already holds. | this spec section 2 | `ambre_app::host::pipe::tests::a_deferred_copy_writes_the_token_alone_and_prevents_the_default` -- a raised `copy` event ends prevented, with no `text/plain` written | none | verified |
| `UI-THREAD-009` | `POST` | The clipboard write MUST happen inside the user activation window the copy opened. | this spec section 2 | none | unheld -- the window is a browser property, so nothing on host can read it. Throwaway Chromium 141 and Firefox 156 runs saw a write from the next frame land. A headless browser grants or refuses no activation, so no tier drives one; lands with the `The clipboard write inside the activation window` Story (`ambre-roadmap.md` E9) | untested |
| `UI-THREAD-023` | `POST` | Where the browser accepts the `web ` types, the asynchronous write MUST publish everything the copy event published, so it never removes what the copy established. | this spec section 2 | `ambre_app::host::tests::the_asynchronous_item_carries_everything_the_copy_published` -- the item, read back, holds the mirror, the prefixed private type and the ownership token | none | verified |
| `UI-THREAD-044` | `FRAME` | A copy MUST NOT change the document. | this spec section 2, the opening of the model; derived | Trame `ambre_trame::logic::tests::group7::copy_range_covers_the_full_selection_not_a_viewport` -- it asserts the outcome carries no delta; Prose `ambre_prose::input_tests::a_copy_leaves_the_document_unchanged` -- the text reads back unchanged; Toile `ambre_toile::ui::view::view_tests::a_copy_names_the_whole_selection_and_sends_nothing_else` -- the copy request goes alone, and the Logic thread answers it for display only | none | verified |
| `UI-THREAD-045` | `FAIL` | A cut whose copy fails MUST delete nothing. | this spec section 2, cut is each application's own policy | `ambre_toile::ui::view::view_tests::a_cut_whose_copy_went_nowhere_deletes_nothing`; `ambre_trame::ui::view::view_tests::a_cut_whose_copy_went_nowhere_arms_nothing`; `ambre_prose::input_tests::a_refused_copy_deletes_nothing` -- each drives a copy the channel refuses, and no delete follows | none | verified |

### 1.3 Paste

Four rungs of fidelity, in one order. The first that can answer does.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UI-THREAD-007` | `POST` | A paste MUST decide same-tab full fidelity on the ownership token the copy wrote, never by comparing the `text/plain` mirror. | this spec section 2 | `ambre_bus::clipboard_tests::a_paste_takes_the_kept_copy_only_on_the_token` -- the same text without the token serves the read | none | verified |
| `UI-THREAD-054` | `POST` | A paste raised after the copy's asynchronous write landed MUST read the clipboard back, so the token and the private bytes that write moved still decide it. | this spec section 2 | `ambre_app::host::pipe::tests::a_paste_after_a_landed_write_reads_the_clipboard_back` -- the browser's read answers with the item a landed write leaves, and the paste decides on its token | none | verified |
| `UI-THREAD-055` | `POST` | Where the browser refuses the `web ` types, the asynchronous write MUST still publish the mirror and the HTML. | this spec section 2 | `ambre_app::host::tests::a_browser_refusing_the_web_types_still_gets_the_mirror_and_the_html` -- a constructor refusing the `web ` types gets a second item with the mirror and the marked HTML | none | verified |
| `UI-THREAD-056` | `POST` | Where the browser refuses the `web ` types, a same-tab paste MUST still reach rung 2. | this spec section 2 | `ambre_bus::clipboard_tests::the_token_reads_back_from_the_marked_html` -- the token's round trip | partial -- the token rides in an HTML comment, so a copy without HTML, every Trame and Toile copy, loses rung 2 on such a browser; lands with the `A same-tab paste on a browser refusing the web types` Story (`ambre-roadmap.md` E9) | unimplemented |
| `UI-THREAD-022` | `POST` | A paste MUST take the fidelity ladder in one order, first match wins. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_paste_takes_the_highest_rung_it_can_serve`; `ambre_trame::ui::view::view_tests::a_paste_takes_the_highest_rung_it_can_serve` -- each offers two rungs at once and reads which one answered | none | verified |
| `UI-THREAD-046` | `POST` | A paste MUST NOT skip a rung it can serve. | this spec section 2, the ladder | `ambre_toile::ui::view::view_tests::a_paste_takes_the_highest_rung_it_can_serve`; `ambre_trame::ui::view::view_tests::a_paste_takes_the_highest_rung_it_can_serve` -- rungs 2 to 4. `ambre_toile::ui::view::view_tests::a_paste_into_an_open_editor_inserts_the_text`; `ambre_trame::ui::view::view_tests::a_paste_into_an_open_editor_inserts_the_text` -- rung 1 | none | verified |
| `UI-THREAD-047` | `FRAME` | A paste MUST NOT change what the clipboard holds. | derived | `ambre_app::host::pipe::tests::a_paste_writes_nothing_to_the_clipboard` -- the host half. `ambre_toile::ui::view::view_tests::a_paste_writes_nothing_back`; `ambre_trame::ui::view::view_tests::a_paste_writes_nothing_back` -- a paste leaves no blob for the host to write | none | verified |

* **Budget.** none: a copy serializes the selection once and a paste inserts it once, and
  `ambre-spec-budgets.md` budgets no clipboard operation. A very large payload costs section 5.
* **Boundary.** the payload leaves the client, in plaintext, by the user's own gesture. That is
  outside the sealed path of `ambre-spec-zero-knowledge.md` and it is the only outbound
  surface the UI thread has besides the font fetch (`UI-THREAD-004`). Nothing here reaches the
  relay, holds a key or opens a socket.
* **Verification.** 16 `verified`, 1 `untested`, 1 `unimplemented`. The wasm tier raises real
  clipboard events and reads the written item back; the host tier drives each view's ladder. The
  loose row is the activation window, which a headless browser does not model. The unbuilt one is
  rung 2 without the `web ` types.
---

## 2. MODEL

**CRDT containers.** none. A copy reads the document and a paste writes it through the ordinary
commands of the application; nothing about the clipboard is persisted or shared.

**The transport.** One shape crosses the seam between the host and the active view, and the same
shape is serialized onto the operating system clipboard.

```rust
/// `text` is the plain-text mirror every other application can read; `ambre` is
/// the active application's own bytes, opaque to the host. `html` is the formatted half
/// other editors read and write; only Prose fills it today.
pub struct ClipboardBlob { pub text: String, pub html: Option<String>, pub ambre: Option<Vec<u8>> }
```

**The four names on the clipboard.** The host owns all four, and an application names none
(`UI-THREAD-006`).

| Name | Written by | Carries |
|---|---|---|
| `text/plain` | both writes | the mirror, for a paste into another application |
| `text/html` | both writes, when the application built it | the formatted half, for a paste into a word processor or a mail client |
| `application/x-ambre` | the synchronous copy event | the application's bytes, base64 |
| `web application/x-ambre` | the asynchronous write | the same bytes, under the prefix that API demands |
| `application/x-ambre-token` | the synchronous copy event | one random value per page load, proving a same-tab copy |
| `web application/x-ambre-token` | the asynchronous write | the same token, under the prefix that API demands |

**A copy, in steps.** The payload does not exist when the key fires, which is what forces the two
writes.

1. the browser raises `copy` or `cut` on the transparent input, and the host asks the active view
   for an outcome;
2. `Text` means an inline text selection the view already holds: the host writes it now and
   prevents the default;
3. `Deferred` means the view sent the authoritative command to the Logic thread. The host writes
   the ownership token, prevents the default and writes nothing else (`UI-THREAD-008`);
4. `Nothing` means no selection: the browser default proceeds;
5. the Logic thread builds the payload from the whole document (`UI-THREAD-010`) and answers;
6. the view drains the answer, keeps it in memory and hands the blob to the host;
7. the host writes the mirror, the prefixed private type and the ownership token
   asynchronously, one frame later, still inside the activation window
   (`UI-THREAD-009`). Where the write lands it replaces the whole clipboard. The token and the
   private bytes then exist only under the `web ` types, which a `paste` event cannot see. The
   next paste reads them back (`UI-THREAD-054`). A browser that refuses the `web ` types gets
   the mirror and the HTML alone (`UI-THREAD-055`), the token in an HTML comment (`UI-THREAD-056`).

**A paste, in steps.** The ladder is one order and the first rung that can answer does
(`UI-THREAD-022`).

| Rung | Serves | Condition |
|---|---|---|
| 1 | the plain text, to the caret | an inline edit session is open |
| 2 | this tab's in-memory payload, at full fidelity | the ownership token is still on the clipboard |
| 3 | the decoded private payload | the clipboard carries the application's bytes |
| 4 | the plain-text mirror, parsed by the application | anything else |

**Why a token and not a text match.** Rung 2 exists for the case where the asynchronous write was
refused. Two tabs holding the same text are exactly that case. A text comparison
reports them as one copy, so the question is answered by a value only this page could have written
(`UI-THREAD-007`).

**Cut is each application's own policy.** Toile deletes right after the copy command; Trame keeps
its marching ants and clears the source when the paste lands. What is shared is the order: the copy
is asked for first, and a failed copy deletes nothing (`UI-THREAD-045`).

**Code.**

* `crates/ambre-app/src/host/pipe.rs` -- the three event listeners, the button's paste, the token
  and `own_copy`;
* `crates/ambre-app/src/host/mod.rs` -- the type names, the base64 codec, the asynchronous write;
* `crates/ambre-app/src/host/frame.rs` -- the one-frame-later mirror, driven from the render loop;
* `crates/ambre-app/src/platform.rs` -- the toolbar button's second path (`UI-THREAD-024`).

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| the copy and cut chords | ride the native events; they are not decoded as chords (`ambre-spec-text-input.md`, `CHROME-021`) |
| the paste chord | rides the native event, and the view walks the ladder |
| the legacy chords | mapped to the same three intents (`ambre-spec-commands.md`, `UI-THREAD-011`) |
| a Copy, Cut or Paste entry in a menu or a toolbar | the same three intents, through a command |
| a paste while an inline edit is open | the text goes to the caret, whatever else the clipboard holds |

The user sees nothing of the two writes. What they see is whether pasting back into Ambre keeps
what they copied, and whether pasting into another application yields readable text.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The payload is built by the Logic thread | building the payload from the view's cache | it is culled by construction: the UI thread holds one viewport, so a payload built there truncates at the edge of what was scrolled, and the loss is invisible until the paste. The document lives on the Logic thread, and asking costs one round trip |
| The write happens one frame later | preparing a payload ahead of every copy | it pays for every copy that is never pasted, and the payload does not exist when the key fires anyway |
| The write happens one frame later | handing the clipboard item a promise it resolves later | it pays the same cost on every copy, for a window the asynchronous API already grants: the activation the key opened lasts longer than the one frame the payload needs |
| Same-tab fidelity is decided by a written token | comparing the plain-text mirror | it calls two tabs holding the same text one copy. A written token answers the exact question -- is this clipboard still ours -- with a value no other page can produce, where every cheaper answer is a guess about content |
| One private type, owned by the host | a private type per application, declared in the application crate | it multiplies the names on the clipboard by the number of applications, and every reader then has to know them all. One name plus opaque bytes leaves the shape to whoever wrote it |
| Three per-application clipboard documents stand beside this one | one clipboard document for the suite, with the three payloads as sections of it | the path is one and the payload is not: a cell range, an identity-free drawing and a run of formatted text are three designs with three sets of tests. Collapsing them into rows here would report three implementation states as one, which is what the ledger exists to prevent. Only a row that names no mechanism of its own is withdrawn, and one was -- `TRAME-UI-066`, a verbatim copy of `UI-THREAD-009` |
| The mirror is what crosses to other applications, and it may be empty | a shared interchange format every application must encode into on every copy | a drawing has no honest text form, so Toile would be encoding something wrong rather than writing nothing. A structured interchange above the three payloads is the `Cross-app rich paste` Story (`ambre-roadmap.md` POST-V1), not this |

---

## 5. NOT BUILT, AND NOT YET RULED

* **No tier drives a clipboard event.** The wasm tier mounts components in a document and never
  raises `copy`, `cut` or `paste`. The host half of every row here is held by reading it.
* **Nothing caps a payload.** A copy of a whole document builds it twice in memory, once as text
  and once as bytes. No rule says what happens when the clipboard refuses it.
* **A paste from another Ambre application falls to text.** The bytes are the writing application's
  own shape, so a spreadsheet range pasted into a page arrives as tab-separated text.
* **Nothing rules on a drop.** Text or a file dropped onto the page is named as a gesture in three
  application documents and no command carries it.
* **An image on the clipboard is ignored.** Only the private type and the mirror are read, so a
  picture copied from a browser drops.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What a spreadsheet range carries, and how a pasted formula shifts | `2-Trame/features/ambre-spec-trame-clipboard.md` |
| What a drawing carries, and how a pasted identity is minted | `3-Toile/features/ambre-spec-toile-clipboard.md` |
| What formatted text carries, and what a rich paste inserts | `4-Prose/features/ambre-spec-prose-clipboard.md` |
| Which chords mean copy, cut and paste | `ambre-spec-commands.md` |
| Why the chords are not decoded again from the keyboard | `ambre-spec-text-input.md` |
| Why the UI thread may hold no document data at all | `ambre-spec-ui.md`, `UI-THREAD-020` |
| What a paste is allowed to write into the document | each application's editing feature |
