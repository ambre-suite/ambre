# AMBRE -- FEATURE: THE DOCUMENT LOCALE

**Type:** Horizontal (framework)
**Implementation:** implemented -- each application sets and reads the register. Trame formats,
parses and exports under it, and a new document takes its creator's locale. Section 6 holds what
is left.

> A document carries one language tag, so two collaborators on differently configured machines read
> the same number, the same currency and the same date. What the reader sees in the menus stays the
> reader's own: the document's locale is the content's, never the interface's.

A number format, a date field, a PDF writer and a command title all ask the same question -- which
language is this. This document owns the tag, where it lives, the order a consumer resolves it in,
and the convention each reader sees and types it under.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `LOCALE`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-001` | `INV` | A document MUST carry one locale, and one only. | this spec section 3 | `ambre_trame::logic::tests::group15::the_locale_is_one_register_that_survives_a_save_and_an_open`; `ambre_toile::logic::state::groups::tests::the_board_locale_converges_and_survives_a_reopen` -- one `meta.locale` key in each schema, frozen by the shape belts | none | verified |
| `LOCALE-002` | `INV` | The locale MUST be a well-formed BCP 47 language tag. | this spec section 3 | `ambre_core::locale::tests::a_locale_is_a_well_formed_tag_or_refused`; `ambre_core::lang::tests::a_malformed_text_is_no_tag` -- `DocumentLocale` holds a `Validated<LangTag>`, whose one door is the RFC 5646 shape check | none | verified |
| `LOCALE-003` | `INV` | The language of the menus, the chips and the dialogs MUST be the reader's own, never the document's locale. | this spec section 3; `ambre-spec-chrome.md` | type-level -- every command title is `fn(&ambre_commands::Locale) -> String`, and no function builds an `ambre_commands::Locale` from a `DocumentLocale` | none | verified |
| `LOCALE-004` | `CONC` | Two peers setting the locale concurrently MUST converge on one tag, the later write standing. | this spec section 3; `ambre-spec-delta-order.md` | `ambre_trame::logic::tests::group15::two_peers_setting_the_locale_at_once_converge`; `ambre_toile::logic::state::groups::tests::the_board_locale_converges_and_survives_a_reopen` -- the register is a last-writer-wins key of `meta` | none | verified |
| `LOCALE-005` | `IO` | The locale MUST survive a save and an open. | this spec section 3; `ambre-spec-docfile.md` | `ambre_trame::logic::tests::group15::the_locale_is_one_register_that_survives_a_save_and_an_open`; `ambre_toile::logic::state::groups::tests::the_board_locale_converges_and_survives_a_reopen` -- a snapshot export, then an import through the schema gate | none | verified |
| `LOCALE-006` | `IO` | The locale MUST leave the client sealed, and the relay MUST NOT learn it. | this spec section 3; `ambre-spec-zero-knowledge.md` | `ambre_logic::dispatch::tests::a_locale_set_leaves_as_one_delta_and_nothing_else` -- a set reaches Sync as one `SendUpdate`, the delta the worker seals, and no bus envelope to Sync names a `DocumentLocale` | none | verified |
| `LOCALE-014` | `INV` | A number, a date and a formula MUST be stored in no locale. | this spec section 3; `2-Trame/features/ambre-spec-trame-number-formats.md` | `ambre_trame::logic::tests::group15::a_number_typed_in_the_readers_locale_is_stored_in_none`; `ambre_trame::logic::tests::group15::a_date_typed_in_the_readers_order_is_stored_as_its_serial`; `ambre_trame::logic::tests::group15::a_formula_is_shown_in_the_readers_form_and_stored_canonical`; `ambre_core::locale::tests::a_typed_number_reads_back_canonical` | none | verified |
| `LOCALE-015` | `INV` | Each reader MUST carry one convention of three: everything in the document's locale, shown in the document's and typed in their own, or everything in their own. | this spec section 3 | `ambre_core::locale::tests::each_convention_picks_the_locale_of_each_half`; `ambre_trame::logic::tests::group15::under_the_document_convention_the_reader_types_the_documents_form` -- `NumberConvention` is a closed enum of three | none | verified |
| `LOCALE-016` | `INV` | A reader who picked no convention MUST see the document's locale and type in their own. | this spec section 3 | `ambre_core::locale::tests::each_convention_picks_the_locale_of_each_half`; `ambre_trame::logic::tests::group15::a_number_typed_in_the_readers_locale_is_stored_in_none` -- `TypeMine` is the `Default` | none | verified |
| `LOCALE-018` | `INV` | Formula text MUST be stored in one canonical form: a decimal point, and a comma between arguments. | this spec section 3; `2-Trame/features/ambre-spec-trame-formula-engine.md` | `ambre_trame::logic::tests::group15::a_formula_is_shown_in_the_readers_form_and_stored_canonical`; `ambre_trame::logic::locale::tests::a_formula_takes_the_readers_separators_both_ways` | none | verified |
| `LOCALE-021` | `OBS` | A view that shows the reader locale where it differs from the document's MUST say so. | this spec section 3 | `ambre_trame::ui::locale::tests::a_view_in_another_locale_says_so`; `ambre_core::locale::tests::a_reader_shown_another_locale_is_told`; `ambre_trame::tests::chrome_ui::the_status_bar_says_when_it_shows_another_locale`; `ambre_trame::logic::tests::group15::the_readers_convention_writes_nothing_and_is_announced` | none | verified |

### 1.2 Set the document locale

A user picks the document's language, or a new document takes its creator's. One register moves,
and no stored value with it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-007` | `PRE` | Setting the locale MUST require the write access an edit requires. | this spec section 4; `ambre-spec-membership.md` | `ambre_logic::dispatch::tests::a_viewers_locale_set_is_dropped` -- the command is mutating in each application, so the role gate drops a viewer's | none | verified |
| `LOCALE-008` | `POST` | After a locale is set, every view that formats a number, a currency or a date MUST show the new locale's form, on every peer. | this spec section 3; `2-Trame/features/ambre-spec-trame-number-formats.md`, `TRAME-110` | `ambre_trame::logic::tests::group15::a_locale_set_on_one_peer_reformats_every_peer_and_moves_no_value`; `ambre_prose::logic::bands::tests::a_set_locale_redraws_the_date_and_moves_no_block` | none | verified |
| `LOCALE-009` | `FRAME` | Setting the locale MUST NOT rewrite a stored value: what a cell, a field or a block holds MUST be unchanged. | this spec section 3; `2-Trame/features/ambre-spec-trame-cell-editing.md` | `ambre_trame::logic::tests::group15::a_locale_set_on_one_peer_reformats_every_peer_and_moves_no_value`; `ambre_prose::logic::bands::tests::a_set_locale_redraws_the_date_and_moves_no_block` -- the cells and the bands read back as they were | none | verified |
| `LOCALE-010` | `FAIL` | A locale that is not a well-formed tag MUST be refused, and the document MUST keep the locale it had. | this spec section 4 | `ambre_core::locale::tests::a_locale_is_a_well_formed_tag_or_refused` -- the set command takes a `DocumentLocale`, so a malformed text is refused at the parse and reaches no register, and a malformed register a peer wrote reads as none | none | verified |
| `LOCALE-019` | `POST` | A new document MUST be seeded with its creator's default locale. | this spec section 3 | `ambre_logic::dispatch::tests::a_created_document_is_seeded_with_its_creators_locale` -- the seed is the first edit of a document this reader created, and an opened one is not written | none | verified |

### 1.3 Resolve a locale for a view or an export

A consumer asks for the language to format in. It may be given one, and the document may have none.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-011` | `POST` | A consumer MUST resolve the locale in one order: the one the request names, then the document's, then none. | this spec section 3; `4-Prose/features/ambre-spec-prose-pdf-export.md`, `PROSE-IO-035` | `ambre_core::locale::tests::resolution_takes_the_request_then_the_document_then_none` -- every consumer calls `ambre_core::resolve` | none | verified |
| `LOCALE-012` | `FRAME` | Resolution MUST NOT read the machine's locale, its clock zone or its region. | this spec section 3 | type-level -- `ambre_core::resolve` takes the request and the register and nothing else, and `ambre-core` links neither `web-sys` nor `js-sys`. The one read of the browser's language is the shell's first run, which stores a setting and resolves nothing | none | verified |
| `LOCALE-013` | `FAIL` | When no locale resolves, the consumer MUST carry none and MUST NOT fall back on a fixed language. | this spec section 3; `4-Prose/features/ambre-spec-prose-pdf-export.md`, `PROSE-IO-035` | `ambre_core::locale::tests::resolution_takes_the_request_then_the_document_then_none`; `ambre_core::locale::tests::each_locale_writes_its_own_separators` -- none resolves to the neutral forms, and so does a tag the CLDR table lacks | none | verified |
| `LOCALE-020` | `POST` | Every export, print, PDF and CSV MUST render in the document's locale, whoever runs it. | this spec section 3; `4-Prose/features/ambre-spec-prose-pdf-export.md` | `ambre_trame::logic::tests::group15::an_export_writes_the_documents_locale_whoever_runs_it` -- the CSV export, the one export of the suite today that formats a number or a date. Prose's PDF writer is `PROSE-IO-035`'s, and has no locale yet | none | verified |

### 1.4 Read and type under a convention

A reader sees and types numbers and dates. The convention picks the locale of each half, and the
document stores neither.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-022` | `PRE` | A typed numeric date whose day and month could swap MUST show an unambiguous long-form preview, in the reader's language, before it is committed. | this spec section 3 | `ambre_trame::ui::locale::tests::a_swappable_date_shows_its_long_form_before_the_commit`; `ambre_core::locale::tests::a_typed_date_whose_parts_could_swap_is_flagged`; `ambre_trame::logic::tests::group15::a_date_typed_in_the_readers_order_is_stored_as_its_serial` | none | verified |
| `LOCALE-023` | `POST` | The edit bar MUST show a formula in the reader's convention, and store what the reader types in the canonical form. | this spec section 3; `LOCALE-018` | `ambre_trame::logic::tests::group15::a_formula_is_shown_in_the_readers_form_and_stored_canonical`; `ambre_trame::logic::locale::tests::a_formula_takes_the_readers_separators_both_ways` | none | verified |
| `LOCALE-017` | `FRAME` | A reader's convention MUST NOT change what the document stores. | this spec section 3 | `ambre_trame::logic::tests::group15::the_readers_convention_writes_nothing_and_is_announced`; `ambre_logic::dispatch::tests::the_readers_locale_reaches_each_document_and_writes_nothing` -- the version vector does not move, and nothing reaches Sync | none | verified |

* **Budget.** none of its own: a locale is one register read when a view formats, and the formatting
  itself is the consumer's frame cost in `ambre-spec-budgets.md`. Setting it is one delta, under the
  edit budget every write already holds.
* **Boundary.** the locale is content. It is a key of the document map, sealed with everything else,
  and `ambre-spec-zero-knowledge.md` is what keeps it there (`LOCALE-006`). Nothing here reaches the
  relay in clear.
* **Verification.** 23 `verified`. Host tests in `ambre-core`, `ambre-trame`, `ambre-toile`,
  `ambre-prose` and `ambre-logic`, two type-level rules, and one wasm test of the status-bar notice.
  No browser test reads the mount of the date preview.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets the document to French, on a French machine | reads the same sheet, on an English machine | one tag, `fr` | both read `1 234,50 EUR`, and Bob's menus stay English (`LOCALE-003`, `LOCALE-008`) | `group15::a_locale_set_on_one_peer_reformats_every_peer_and_moves_no_value` |
| sets the document to French | sets it to German, at the same moment | one tag, the later write standing | both read the same language, whichever it is; neither keeps a private one (`LOCALE-004`) | `group15::two_peers_setting_the_locale_at_once_converge` |
| exports to PDF without naming a language, from a document with no tag | -- | no tag | the file carries no language, rather than Alice's machine's (`LOCALE-011`, `LOCALE-013`) | not built: no PDF writer reads a locale yet (`PROSE-IO-035`) |
| types `3,5` in a cell, her own locale French, the document English, under the default convention | reads the cell | the number 3.5, in no locale | both read `3.5`, the document's form; Alice typed in her own (`LOCALE-014`, `LOCALE-016`) | `group15::a_number_typed_in_the_readers_locale_is_stored_in_none` |
| exports a CSV, her own convention set to everything in French | -- | one tag, `en` | the file reads the English forms, whoever runs it (`LOCALE-020`) | `group15::an_export_writes_the_documents_locale_whoever_runs_it` |

---

## 3. MODEL

**Containers.** one: the root `meta` map, key `locale`, a BCP 47 tag as a string. All three
applications already carry `schema_version` in that map (`ambre-spec-schema-version.md`). The
register costs no new container, and converges as a last-writer-wins key like its neighbours
(`LOCALE-004`).

**The value.** A tag, not a list, and not a set of separators. `fr`, `fr-CA`, `de-CH` name a
language and a region, and the number, currency and date forms follow from the tag. Ambre stores
which language, never what the language renders (`LOCALE-009`).

```rust
/// The document's locale: one well-formed tag, or none when the document names no language.
/// `None` is a value, not a missing one: a consumer that resolves to it formats without a locale.
pub struct DocumentLocale(Option<Validated<LangTag>>);

impl DocumentLocale {
    /// The locale a text spells. Refuses a text that is no well-formed tag (`LOCALE-010`).
    pub fn parse(text: &str) -> Result<Self, LocaleError>;
}
```

**The seed.** A new document takes its creator's default locale, written once, at creation
(`LOCALE-019`). That write is no resolution: every later read takes the register, and no consumer
reads a machine to format, so `LOCALE-012` still holds.

**Storage is neutral.** A number, a date and a formula are stored in no locale (`LOCALE-014`). A
formula's text keeps one canonical form, a decimal point and a comma between arguments
(`LOCALE-018`). The edit bar translates it both ways for the reader's convention (`LOCALE-023`).
An export, a print, a PDF and a CSV render in the document's locale, whoever runs them
(`LOCALE-020`).

**The reader convention.** Each reader carries one of a closed set of three (`LOCALE-015`). The
reader locale is the one a reader holds on their own device:

| Convention | Shown in | Typed in |
| :--- | :--- | :--- |
| `Document` | the document's locale | the document's locale |
| `TypeMine` | the document's locale | the reader's own locale |
| `AllMine` | the reader's own locale | the reader's own locale |

`TypeMine` is the default (`LOCALE-016`). The convention never changes what is stored
(`LOCALE-017`). Under `AllMine`, a view in a locale other than the document's says so
(`LOCALE-021`). A typed numeric date whose day and month could swap shows a long-form preview
first (`LOCALE-022`).

```rust
/// Which locale a reader sees and types numbers and dates in. A closed set, held per reader.
pub enum NumberConvention { Document, TypeMine, AllMine }
```

**Where each half lives, and where the forms come from.**

| Element | Where | Why |
| :--- | :--- | :--- |
| The reader's own locale | the device's local storage, key `ambre.reader_locale`, empty for none; never a document | two small values read once at boot need no asynchronous store |
| Its first value | the browser's language, read once at the first run and stored; a malformed one is stored as none | a stored setting, so no later read of the machine happens (`LOCALE-012`) |
| The convention | `TypeMine` until a picker exists | `LOCALE-016` |
| Its later home | the settings document of the `User settings` Story (`ambre-roadmap.md` E6) | -- |
| The forms | `ambre_core::locale::table`, written by `scripts/locale_table.py` from CLDR 48.0.0, held by `just locale-table-check` | no mark is typed by hand |
| The locales | `en`, `en-GB`, `fr`, `fr-CA`, `fr-CH`, `de`, `de-AT`, `de-CH`, `es`, `it`, `it-CH`, `pt`, `nl` | -- |
| Any other tag | its language's record, else the neutral forms: a decimal point, no group, the ISO date | `LOCALE-013`; the neutral forms are what Trame drew before the register |
| A numeric date | CLDR's short order and mark, the year written whole | a year cut to two digits loses its century |

**Two languages, and they never meet.** The document's locale is a property of the content: the
numbers, the currencies, the dates and the collation a reader sees inside the document. The
interface language is a property of the reader, held by the client and never written anywhere
(`LOCALE-003`). A command title is the second one -- `title: fn(&Locale) -> String` in
`ambre-spec-commands.md` takes the reader's, and this document supplies neither.

**Resolving, in order.** Three steps, and the third is a value:

1. the request names a locale -- an export parameter, a print dialog -- and it wins (`LOCALE-011`);
2. otherwise the document's tag, when it has one;
3. otherwise none, carried as none (`LOCALE-013`). The machine is never read (`LOCALE-012`).

**Code.** `ambre_core::locale` holds the register type, the convention, `resolve` and the forms,
and `ambre_core::locale::table` is the generated CLDR table. Each application writes the register
through a `SetDocumentLocale` command. Trame's `logic::locale` formats, parses and translates
formulas, and `ui::locale` writes the notice and the date preview. The Logic thread holds the
reader's half and seeds a created document.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the document-language entry in a menu | a picker opens on the current tag, through a command (`ambre-spec-commands.md`) |
| picking a language | one delta, and every view reformats on every peer (`LOCALE-008`) |
| picking a language without write access | refused, like any edit (`LOCALE-007`) |
| an export that names a language | the named one wins over the document's (`LOCALE-011`) |
| a document with no tag, and a request with none | formatted without a locale, never with the machine's (`LOCALE-013`) |

The reader's own interface language is set outside the document, and no document changes it.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The locale is the document's, carried in it | each client formatting on its machine's locale | it is the defect the register exists to close: two collaborators read one sheet as `1,234.50` and `1 234,50`, one of them quotes a figure, and nothing on either screen says they disagree |
| One tag, resolved in one order | a locale per view, per sheet or per block | it multiplies the question without answering it, and a document with two locales has no answer for an export, which is one file |
| The content's language and the interface's are two | one language for the document and the menus | it translates the interface of every collaborator who opens a French document, which nobody asked for and no gesture undoes (`LOCALE-003`) |
| None is a value, and no fixed fallback | defaulting to one language when nothing resolves | it is wrong for every reader who does not share it, and the file says nothing about being wrong -- the same rule `PROSE-IO-035` already holds for the PDF writer |
| The forms generated from CLDR, at a pinned version | a table typed by hand | a group mark such as French's narrow no-break space or Swiss German's apostrophe is the value a hand types wrong, and no reader sees that it is wrong |
| A compact table of thirteen locales in `ambre-core` | `icu4x` | several hundred kilobytes of WebAssembly against the 4 MB first download (`ambre-spec-budgets.md`), for a set of locales the table already covers |
| Storing the tag, never the rendered form | storing the formatted text beside the raw value | a formatted string in the CRDT is a second source of truth: it goes stale on the first locale change, and every formula reading the cell reads the wrong one (`LOCALE-009`) |
| A key of the existing `meta` map | a root container of its own | one string does not earn a container, and `meta` already holds the per-document registers every application stamps at construction |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No picker writes a locale.** The register is set by the creator's seed and by the
  `SetDocumentLocale` command of each application, which no menu entry sends yet. Which menu carries
  the entry, and whether the tag shows in the status bar, is `ambre-spec-chrome.md`'s to decide.
* **No picker sets the reader's half.** The reader's own locale is the browser's language at the
  first run, and the convention is `TypeMine`. Nothing changes either yet.
* **Two consumers wait on it.** The PDF writer (`PROSE-IO-035`), and the collation a find uses
  (`ambre-spec-find.md`). Each names the gap, and none invents an answer.
* **Prose and Toile take no convention.** Prose's date field draws the document's locale whatever
  the reader's convention, and Toile formats no number.
* **Collation is named, not ruled.** `ambre-spec-find.md` records that a byte comparison and a
  locale-aware one disagree; which collation a tag selects, and for which operation, is open.
* **Nothing supplies the reader's interface language.** `ambre-spec-commands.md` carries
  `title: fn(&Locale) -> String` and no document says who holds that argument. This one rules it out
  of the document (`LOCALE-003`); where the client reads it from is unwritten.
* **An array constant keeps its separators.** The edit bar translates the decimal mark and the
  argument separator of a formula, and leaves the separators of a `{...}` array as stored.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Which forms a number, a currency or a date takes in a sheet | `2-Trame/features/ambre-spec-trame-number-formats.md` |
| What a date field stores, and its three forms | `4-Prose/features/ambre-spec-prose-headers-footers.md` |
| What language a PDF file declares | `4-Prose/features/ambre-spec-prose-pdf-export.md` |
| How a search compares two strings | `ambre-spec-find.md` |
| Where the `meta` map lives, and which keys it carries | each application's schema document |
| How a command title is translated | `ambre-spec-commands.md` |
