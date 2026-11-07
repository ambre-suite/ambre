# AMBRE -- FEATURE: THE DOCUMENT LOCALE

**Type:** Horizontal (framework)
**Implementation:** design-only -- no command writes a locale today, and every consumer that needs
one names the gap. Prose's date field already reads the register. The rest lands with the
`One document locale` Story (`ambre-roadmap.md` E8).

> A document carries one language tag, so two collaborators on differently configured machines read
> the same number, the same currency and the same date. What the reader sees in the menus stays the
> reader's own: the document's locale is the content's, never the interface's.

A number format, a date field, a PDF writer and a command title all ask the same question -- which
language is this. Four documents answer it four ways today, and one of them, Prose's date field,
already reads the register. This document owns the tag, where it lives, the order a consumer
resolves it in, and the convention each reader sees and types it under.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `LOCALE`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-001` | `INV` | A document MUST carry one locale, and one only. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-002` | `INV` | The locale MUST be a well-formed BCP 47 language tag. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-003` | `INV` | The language of the menus, the chips and the dialogs MUST be the reader's own, never the document's locale. | this spec section 3; `ambre-spec-chrome.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). It is the half the register is easiest to get wrong: one tag in the document would otherwise translate the interface of every collaborator who opens it | unimplemented |
| `LOCALE-004` | `CONC` | Two peers setting the locale concurrently MUST converge on one tag, the later write standing. | this spec section 3; `ambre-spec-delta-order.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-005` | `IO` | The locale MUST survive a save and an open. | this spec section 3; `ambre-spec-docfile.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). It travels between peers as an ordinary delta, which is `LOCALE-004`'s convergence and not a rule of its own: the register is a key of the document map, so the file and the wire carry it like every other key | unimplemented |
| `LOCALE-006` | `IO` | The locale MUST leave the client sealed, and the relay MUST NOT learn it. | this spec section 3; `ambre-spec-zero-knowledge.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). It is a content key inside the document map, so the sealed envelope carries it like any other; the row exists because a language tag is the kind of small field a later routing or indexing feature is tempted to lift out | unimplemented |
| `LOCALE-014` | `INV` | A number, a date and a formula MUST be stored in no locale. | this spec section 3; `2-Trame/features/ambre-spec-trame-number-formats.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). It is what lets one stored value show in any locale, which `LOCALE-009` relies on | unimplemented |
| `LOCALE-015` | `INV` | Each reader MUST carry one convention of three: everything in the document's locale, shown in the document's and typed in their own, or everything in their own. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-016` | `INV` | A reader who picked no convention MUST see the document's locale and type in their own. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-018` | `INV` | Formula text MUST be stored in one canonical form: a decimal point, and a comma between arguments. | this spec section 3; `2-Trame/features/ambre-spec-trame-formula-engine.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-021` | `OBS` | A view that shows the reader locale where it differs from the document's MUST say so. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |

### 1.2 Set the document locale

A user picks the document's language, or a new document takes its creator's. One register moves,
and no stored value with it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-007` | `PRE` | Setting the locale MUST require the write access an edit requires. | this spec section 4; `ambre-spec-membership.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-008` | `POST` | After a locale is set, every view that formats a number, a currency or a date MUST show the new locale's form, on every peer. | this spec section 3; `2-Trame/features/ambre-spec-trame-number-formats.md`, `TRAME-110` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-009` | `FRAME` | Setting the locale MUST NOT rewrite a stored value: what a cell, a field or a block holds MUST be unchanged. | this spec section 3; `2-Trame/features/ambre-spec-trame-cell-editing.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). It is what makes the register re-localizable: the raw text is the document and the form is derived, so a second locale re-renders where a stored string would have to be migrated | unimplemented |
| `LOCALE-010` | `FAIL` | A locale that is not a well-formed tag MUST be refused, and the document MUST keep the locale it had. | this spec section 4 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-019` | `POST` | A new document MUST be seeded with its creator's default locale. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). The seed writes the register once, at creation, and it is no resolution, so `LOCALE-012` holds | unimplemented |

### 1.3 Resolve a locale for a view or an export

A consumer asks for the language to format in. It may be given one, and the document may have none.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-011` | `POST` | A consumer MUST resolve the locale in one order: the one the request names, then the document's, then none. | this spec section 3; `4-Prose/features/ambre-spec-prose-pdf-export.md`, `PROSE-IO-035` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-012` | `FRAME` | Resolution MUST NOT read the machine's locale, its clock zone or its region. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). It is the rule the whole register exists for: a machine default is what makes two collaborators read one document two ways, and it is invisible to whoever is not on the second machine | unimplemented |
| `LOCALE-013` | `FAIL` | When no locale resolves, the consumer MUST carry none and MUST NOT fall back on a fixed language. | this spec section 3; `4-Prose/features/ambre-spec-prose-pdf-export.md`, `PROSE-IO-035` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-020` | `POST` | Every export, print, PDF and CSV MUST render in the document's locale, whoever runs it. | this spec section 3; `4-Prose/features/ambre-spec-prose-pdf-export.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). A locale the request names still wins (`LOCALE-011`); the reader's convention never does | unimplemented |

### 1.4 Read and type under a convention

A reader sees and types numbers and dates. The convention picks the locale of each half, and the
document stores neither.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-022` | `PRE` | A typed numeric date whose day and month could swap MUST show an unambiguous long-form preview, in the reader's language, before it is committed. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-023` | `POST` | The edit bar MUST show a formula in the reader's convention, and store what the reader types in the canonical form. | this spec section 3; `LOCALE-018` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-017` | `FRAME` | A reader's convention MUST NOT change what the document stores. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |

* **Budget.** none of its own: a locale is one register read when a view formats, and the formatting
  itself is the consumer's frame cost in `ambre-spec-budgets.md`. Setting it is one delta, under the
  edit budget every write already holds.
* **Boundary.** the locale is content. It is a key of the document map, sealed with everything else,
  and `ambre-spec-zero-knowledge.md` is what keeps it there (`LOCALE-006`). Nothing here reaches the
  relay in clear.
* **Verification.** 23 `unimplemented`. No command writes the register, and the four consumers
  that need it each name the same gap. Section 6 lists them.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets the document to French, on a French machine | reads the same sheet, on an English machine | one tag, `fr` | both read `1 234,50 EUR`, and Bob's menus stay English (`LOCALE-003`, `LOCALE-008`) | not built |
| sets the document to French | sets it to German, at the same moment | one tag, the later write standing | both read the same language, whichever it is; neither keeps a private one (`LOCALE-004`) | not built |
| exports to PDF without naming a language, from a document with no tag | -- | no tag | the file carries no language, rather than Alice's machine's (`LOCALE-011`, `LOCALE-013`) | not built |
| types `3,5` in a cell, her own locale French, the document English, under the default convention | reads the cell | the number 3.5, in no locale | both read `3.5`, the document's form; Alice typed in her own (`LOCALE-014`, `LOCALE-016`) | not built |
| prints the document, her own convention set to everything in French | -- | one tag, `en` | the print reads the English forms, whoever runs it (`LOCALE-020`) | not built |

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

**Where the reader's half lives.** For MVP, the reader's own locale and convention live on the
device, in IndexedDB, and never in a document. The `User settings` Story (`ambre-roadmap.md` E6)
folds them into the settings document later.

**Two languages, and they never meet.** The document's locale is a property of the content: the
numbers, the currencies, the dates and the collation a reader sees inside the document. The
interface language is a property of the reader, held by the client and never written anywhere
(`LOCALE-003`). A command title is the second one -- `title: fn(&Locale) -> String` in
`ambre-spec-commands.md` takes the reader's, and this document supplies neither.

**Resolving, in order.** Three steps, and the third is a value:

1. the request names a locale -- an export parameter, a print dialog -- and it wins (`LOCALE-011`);
2. otherwise the document's tag, when it has one;
3. otherwise none, carried as none (`LOCALE-013`). The machine is never read (`LOCALE-012`).

**Code.** not built -- it lands with the `One document locale` Story (`ambre-roadmap.md` E8).
Prose's schema already declares the register, and Prose's date field reads it. No command writes
it yet, and section 6 holds what is left.

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
| Storing the tag, never the rendered form | storing the formatted text beside the raw value | a formatted string in the CRDT is a second source of truth: it goes stale on the first locale change, and every formula reading the cell reads the wrong one (`LOCALE-009`) |
| A key of the existing `meta` map | a root container of its own | one string does not earn a container, and `meta` already holds the per-document registers every application stamps at construction |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No command writes a locale.** The picker, the seed, the conventions and the resolution order are
  a design bound to the `One document locale` Story (`ambre-roadmap.md` E8), not code.
* **One key, decided.** The Prose, Trame and Toile schemas share one key, `meta.locale`, and Prose's
  former `lang` is dropped. Prose holds the key today, and its date field reads it. Each schema
  document declares the key; Trame and Toile add it with the Story.
* **Four consumers wait on it.** Trame's currencies and dates (`TRAME-110`), Prose's date field
  (`PROSE-FEAT-162`), the PDF writer (`PROSE-IO-035`), and the collation a find uses
  (`ambre-spec-find.md`). Each names the gap, and none invents an answer. The date field stores a
  format precisely so the locale stays separate from it.
* **Collation is named, not ruled.** `ambre-spec-find.md` records that a byte comparison and a
  locale-aware one disagree; which collation a tag selects, and for which operation, is open.
* **Nothing supplies the reader's interface language.** `ambre-spec-commands.md` carries
  `title: fn(&Locale) -> String` and no document says who holds that argument. This one rules it out
  of the document (`LOCALE-003`); where the client reads it from is unwritten.
* **The picker is unplaced.** Which menu carries the entry, and whether the tag shows in the status
  bar, is `ambre-spec-chrome.md`'s to decide when the register exists.

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
