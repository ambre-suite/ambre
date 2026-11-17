# AMBRE -- FEATURE: SPELL CHECKING

**Type:** Vertical (feature)

> Misspelled words underlined as the document is written, with suggestions, checked on the reader's
> own machine. No word of the document is sent anywhere to be checked.

Almost nothing here is shared state. The errors are derived locally and written back nowhere
(`PROSE-FEAT-106`). A word a reader ignores belongs to that reader, not to the document
(`PROSE-FEAT-108`). One gesture does reach a peer: accepting a suggestion replaces a word
(`PROSE-FEAT-109`). It is an ordinary text edit, whose convergence
`features/ambre-spec-prose-text-editing.md` owns. There is no two-peer table below.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-019` | `INV` | The spell check MUST run on the reader's own machine. | this spec section 2; `PROSE-025` | `ambre_logic::dispatch::tests::a_prose_spell_check_runs_on_the_converter_thread_and_sync_sees_no_word` -- the check runs in the spelling module on the converter thread of the client; type-level -- a check is a `Conversion`, which only the converter thread and the Logic thread carry | none | verified |
| `PROSE-FEAT-102` | `INV` | The checker MUST NOT send document text out of the client, to a spelling service or to anything else. | this spec section 2; `PROSE-025` | `ambre_logic::dispatch::tests::a_prose_spell_check_runs_on_the_converter_thread_and_sync_sees_no_word` -- every command Sync receives during a check is read for the misspelled word, and the one it gets is the fetch of the module by its family | none | verified |
| `PROSE-FEAT-041` | `INV` | The check MUST run on the converter thread, and the Logic thread MUST NOT wait for its answer. | this spec section 2; `ambre-spec-architecture.md section 2`, the converter thread | `ambre_prose::logic::spelling::tests::the_check_leaves_for_the_converter_thread_and_nothing_waits` -- the pass returns with the check in flight and nothing underlined; `ambre_logic::dispatch::tests::a_prose_spell_check_runs_on_the_converter_thread_and_sync_sees_no_word` -- the underline lands once the converter thread answered | none | verified |
| `PROSE-FEAT-103` | `INV` | An edit made while a check is in flight MUST land at once, and its block MUST be checked again after that check answers. | this spec section 2; derived | `ambre_prose::logic::spelling::tests::the_check_leaves_for_the_converter_thread_and_nothing_waits` -- the edit lands, the stale answer underlines nothing, and the next check underlines the word where it moved | none | verified |
| `PROSE-FEAT-042` | `INV` | A dictionary MUST be fetched as its own module, on the first check of its language. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-080` | `ambre_logic::dispatch::tests::a_prose_spell_check_runs_on_the_converter_thread_and_sync_sees_no_word` -- the first check of a French document asks Sync for the `SpellFr` module, and for no other | none | verified |
| `PROSE-FEAT-104` | `INV` | The main binary MUST NOT carry a dictionary. | this spec section 2; this spec section 4; `ARCH-078` | ci-gate -- `scripts/converter_boundary.py`, run by `just converter-boundary` in the CI `lint` job, refuses `spellbook` in Main and in Sync. The dictionaries are embedded by the features `spell_en` and `spell_fr` of `ambre-convert`, which pull `spellbook`, so neither instance can carry one | none | verified |
| `PROSE-FEAT-105` | `INV` | A reported error MUST carry at most five suggestions. | this spec section 2; this spec section 3 | `ambre_convert::engine::spell::tests::a_suggestion_answers_at_most_five_corrections`; `ambre_prose::logic::spelling::tests::a_suggestion_comes_back_at_most_five` | none | verified |
| `PROSE-FEAT-106` | `INV` | The document MUST NOT hold the errors: they are derived from its text, on each reader's machine. | this spec section 2; derived | `ambre_prose::logic::spelling::tests::a_check_writes_nothing_to_the_document` -- a check and its answer write no delta and no presence sample, and the snapshot reads as before | none | verified |

### 1.2 A check answers

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-333` | `POST` | After a check answers, each word of a block that the dictionary does not know MUST be reported with its range, and no other word. | this spec section 2 | `ambre_prose::logic::spelling::tests::a_misspelled_word_is_reported_with_its_range` -- an English and a French paragraph; `ambre_convert::engine::spell::tests::an_english_check_names_the_unknown_words`; `::a_french_check_names_the_unknown_words` -- an elision and a typographic apostrophe included | none | verified |
| `PROSE-FEAT-335` | `FRAME` | A check and its answer MUST NOT touch the document, so no peer receives anything. | this spec section 2; `PROSE-FEAT-106` | `ambre_prose::logic::spelling::tests::a_check_writes_nothing_to_the_document` -- no delta, no presence sample, and the snapshot reads as before | none | verified |

### 1.3 Ignore a word

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-334` | `POST` | After the command, the word MUST NOT be reported as an error in this document for the rest of the session. | this spec section 3 | `ambre_prose::logic::spelling::tests::an_ignored_word_is_underlined_nowhere_and_nothing_is_written` -- each block holding the word is cleared, and a later check of a new occurrence reports none | none | verified |
| `PROSE-FEAT-043` | `POST` | After the command, the word MUST be in the reader's own encrypted settings. | this spec section 2 | not built | unbuilt -- lands with the `The personal dictionary` Story (`ambre-roadmap.md` V1) | unimplemented |
| `PROSE-FEAT-107` | `POST` | After the command, that word MUST NOT be reported as an error again for this reader, in this document or in another. | this spec section 3 | not built | unbuilt -- lands with the `The personal dictionary` Story (`ambre-roadmap.md` V1) | unimplemented |
| `PROSE-FEAT-108` | `FRAME` | Ignoring a word MUST NOT touch the document, so no peer receives anything. | this spec section 2; this spec section 4 | `ambre_prose::logic::spelling::tests::an_ignored_word_is_underlined_nowhere_and_nothing_is_written` -- no delta, no presence sample, and the snapshot reads as before | none | verified |

### 1.4 Choose a suggestion

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-109` | `POST` | After the command, the misspelled word MUST read as the suggestion chosen, as an ordinary edit that peers receive. | this spec section 2; this spec section 3 | `ambre_prose::logic::spelling::tests::a_correction_is_an_edit_of_the_word_alone` -- the outcome carries a delta, and the next check underlines nothing | none | verified |
| `PROSE-FEAT-110` | `FRAME` | Accepting a suggestion MUST NOT touch text outside the error's range, nor the reader's own dictionary. | derived | `ambre_prose::logic::spelling::tests::a_correction_is_an_edit_of_the_word_alone` -- a range that names no error now is refused and the text stays, and the correction replaces the word alone | none | verified |

### 1.5 The language

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-111` | `POST` | The check MUST use the dictionary of the document's locale, else of the reader's own locale, else none. After either locale changes, the document MUST be checked again. | this spec section 2; `ambre-spec-locale.md` | `ambre_prose::logic::spelling::tests::the_language_follows_the_document_then_the_reader` -- no language checks nothing, the reader's French checks in French, and a document set to English is checked again in English | none | verified |
| `PROSE-FEAT-112` | `FRAME` | Setting the language MUST NOT touch the text of the document. | derived; `LOCALE-009` | `ambre_prose::logic::bands::tests::a_set_locale_redraws_the_date_and_moves_no_block` -- the language is the document locale, whose set moves no block | none | verified |

### 1.6 The page

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-247` | `OBS` | A misspelled word MUST be drawn with a wavy line in the error colour along its bottom, across its width alone, and with no tint over its text. | this spec section 3 | `ambre_prose::view::paint::tests::a_misspelled_word_is_drawn_with_a_wave_and_no_tint` -- the recording surface | none | verified |
| `PROSE-UI-248` | `OBS` | A press on a misspelled word MUST open, under that word, a popover that lists its suggestions and an ignore. A pick MUST send its command and close the popover, and a press elsewhere MUST close it. | this spec section 3; `ambre-spec-ui.md section 2` | `ambre_prose::view::annotate::tests::a_press_on_a_misspelled_word_opens_its_popover` -- the view; `ambre_prose::tests::chrome_ui::the_spelling_popover_lists_the_corrections_and_hands_back_a_pick` -- wasm tier: the popover under the word, its waiting state, its picks and the intent each one writes | none | verified |

* **Budget.** the spell check of a paragraph, the first load of a dictionary and the brotli of each
  spelling module, rows of `ambre-spec-budgets.md`. The check runs on the converter thread, so no
  figure of it lands on the Logic thread.
* **Boundary.** nothing new crosses the relay. The module arrives as any converter module does,
  named by its family alone (`ARCH-083`). `PROSE-FEAT-102` bans sending the text, and
  `PROSE-FEAT-108` keeps the ignored words out of the shared document.
* **Verification.** 18 `verified`, 2 `unimplemented` bound to the The personal dictionary Story
  (`ambre-roadmap.md`).
  Host tests in `ambre-convert`, `ambre-prose` and `ambre-logic`, one CI gate, and one wasm test of
  the popover.

---

## 2. MODEL

**Containers.** None in the document. The spelling session is state of the Logic thread, one per
document. It holds the misspelled words of each block, the ignored words and the check in flight.

**The engine.** `spellbook`, a pure Rust reader of Hunspell dictionaries, in `ambre-convert`. Each
dictionary is a converter module of its own: `spell_en` embeds the American English dictionary of
SCOWL, and `spell_fr` the French dictionary of Grammalecte (`crates/ambre-convert/dictionaries`).
A module parses its dictionary on its first request and keeps it for the next ones.

**The language.** The document's locale, else the reader's own locale, else none
(`PROSE-FEAT-111`). Its primary subtag picks the dictionary: `en` and `fr` today. Any other
language checks nothing.

**A check.** The Logic thread cuts the text of each block into words, as the word count does
(UAX #29), and keeps the range of each one. It hands the converter thread the distinct words of the
stale blocks, at most 4 096 in one check. The module answers the positions of those it does not
know.

One check is in flight per document. A block edited meanwhile waits for the next one. An answer
for a block whose text moved is dropped (`PROSE-FEAT-103`).

**A suggestion.** A suggestion costs tens of milliseconds a word, against a few microseconds for a
lookup. So a check carries none: the press on an underlined word asks for the suggestions of that
word alone, in a job of its own.

**The commands** (`ambre_prose::message`):

| Shape | Behaviour |
|---|---|
| `SuggestSpelling { block_id, range }` | asks the module for the corrections of that word, and writes nothing |
| `CorrectSpelling { block_id, range, replacement }` | replaces the word, as an ordinary edit |
| `IgnoreSpelling { block_id, range }` | stops underlining that word for the session, and writes nothing |

The view is told `Misspelled`, one block at a time, and `SpellingSuggestions` for the word pressed.
The page painter draws the wave (`features/ambre-spec-prose-page-rendering.md`), and the popover is
DOM chrome beside the suggestion popover.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| a change to a block's text | checks that block again, on the converter thread |
| the document's or the reader's locale changes | drops every underline and checks the document again |
| a press on an underlined word | opens a popover under it: up to five suggestions, then Ignore |
| choosing a suggestion | replaces the word, as an ordinary edit that peers receive |
| Ignore | stops underlining the word in this document, for this reader and this session |
| a press anywhere else | closes the popover |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The check runs on the reader's machine | a spelling service | it is handed the document one paragraph at a time, which is precisely the content the relay is built never to see. No version of a remote checker is compatible with the pillar, so the choice is local or nothing |
| It runs on the converter thread | checking on the Logic thread at a low priority | a lookup of a French word costs microseconds, but parsing its dictionary costs tens of milliseconds, and a suggestion as much a word. The converter thread already runs what the Logic thread must never wait on, and the Logic thread holds no priority to rank a check below a keystroke |
| One module per dictionary | one module holding the engine and every dictionary | a reader who writes in French would download the English dictionary too. The engine is small beside a dictionary, so each module carries its own copy |
| Suggestions on a press | suggestions for every error in each check | a suggestion costs up to 250 ms a word on the host. A pasted page of errors would hold the converter thread for seconds, and a picture decode behind it |
| `spellbook` | `zspell` | the registry states no licence for it, and `deny.toml` admits a licence only by name |
| A pure Rust dictionary engine | a Hunspell binding | it fails the Rust-first pillar, and it would have to be built for the browser target besides |
| The ignored words are the reader's, not the document's | storing them in the document | it shares a personal dictionary with every peer, and through the relay. A word one reader knows is not a fact about the document |
| Spelling, and no grammar | a grammar model in the browser | it is fifty times the whole application: more than a hundred megabytes. A grammar checker is therefore either remote, which the pillar refuses, or a download nobody would accept |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The ignored words do not follow the reader.** They hold for one document and one session. A
  dictionary of the reader's own, in the sealed settings, is the The personal dictionary Story
  (`ambre-roadmap.md`).
* **Nothing rules on what is checked.** Every body block is checked: a code block, a link's text and
  a band included. Whether a code block or a word inside a suggestion is skipped is undecided.
* **Two dictionaries.** A language other than English and French checks nothing, and the reader is
  not told so.
