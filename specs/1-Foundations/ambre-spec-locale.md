# AMBRE -- FEATURE: THE DOCUMENT LOCALE

**Type:** Horizontal (framework)
**Implementation:** design-only -- no document carries a locale today, and every consumer that needs
one names the gap. The register lands with the `One document locale` Story (`ambre-roadmap.md` E8).

> A document carries one language tag, so two collaborators on differently configured machines read
> the same number, the same currency and the same date. What the reader sees in the menus stays the
> reader's own: the document's locale is the content's, never the interface's.

A number format, a date field, a PDF writer and a command title all ask the same question -- which
language is this. Four documents answer it four ways today, and one of them (`meta.lang` in Prose)
already names the register under another word. This document owns the tag, where it lives, and the
order a consumer resolves it in.

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

### 1.2 Set the document locale

A user picks the document's language. One register moves, and no stored value with it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-007` | `PRE` | Setting the locale MUST require the write access an edit requires. | this spec section 4; `ambre-spec-membership.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-008` | `POST` | After a locale is set, every view that formats a number, a currency or a date MUST show the new locale's form, on every peer. | this spec section 3; `2-Trame/features/ambre-spec-trame-number-formats.md`, `TRAME-110` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-009` | `FRAME` | Setting the locale MUST NOT rewrite a stored value: what a cell, a field or a block holds MUST be unchanged. | this spec section 3; `2-Trame/features/ambre-spec-trame-cell-editing.md` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). It is what makes the register re-localizable: the raw text is the document and the form is derived, so a second locale re-renders where a stored string would have to be migrated | unimplemented |
| `LOCALE-010` | `FAIL` | A locale that is not a well-formed tag MUST be refused, and the document MUST keep the locale it had. | this spec section 4 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |

### 1.3 Resolve a locale for a view or an export

A consumer asks for the language to format in. It may be given one, and the document may have none.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `LOCALE-011` | `POST` | A consumer MUST resolve the locale in one order: the one the request names, then the document's, then none. | this spec section 3; `4-Prose/features/ambre-spec-prose-pdf-export.md`, `PROSE-IO-035` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |
| `LOCALE-012` | `FRAME` | Resolution MUST NOT read the machine's locale, its clock zone or its region. | this spec section 3 | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8). It is the rule the whole register exists for: a machine default is what makes two collaborators read one document two ways, and it is invisible to whoever is not on the second machine | unimplemented |
| `LOCALE-013` | `FAIL` | When no locale resolves, the consumer MUST carry none and MUST NOT fall back on a fixed language. | this spec section 3; `4-Prose/features/ambre-spec-prose-pdf-export.md`, `PROSE-IO-035` | not built | unbuilt -- lands with the `One document locale` Story (`ambre-roadmap.md` E8) | unimplemented |

* **Budget.** none of its own: a locale is one register read when a view formats, and the formatting
  itself is the consumer's frame cost in `ambre-spec-budgets.md`. Setting it is one delta, under the
  edit budget every write already holds.
* **Boundary.** the locale is content. It is a key of the document map, sealed with everything else,
  and `ambre-spec-zero-knowledge.md` is what keeps it there (`LOCALE-006`). Nothing here reaches the
  relay in clear.
* **Verification.** 0 `verified`, 13 `unimplemented`. No document carries the register, and the four
  consumers that need it each name the same gap. Section 6 lists them.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets the document to French, on a French machine | reads the same sheet, on an English machine | one tag, `fr` | both read `1 234,50 EUR`, and Bob's menus stay English (`LOCALE-003`, `LOCALE-008`) | not built |
| sets the document to French | sets it to German, at the same moment | one tag, the later write standing | both read the same language, whichever it is; neither keeps a private one (`LOCALE-004`) | not built |
| exports to PDF without naming a language, from a document with no tag | -- | no tag | the file carries no language, rather than Alice's machine's (`LOCALE-011`, `LOCALE-013`) | not built |

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
/// The document's locale: one BCP 47 tag, or none when the document names no language.
/// `None` is a value, not a missing one: a consumer that resolves to it formats without a locale.
pub struct DocumentLocale(pub Option<String>);
```

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
Prose's schema already declares `meta.lang`, which is this register under another name. That key is
unwritten, and section 6 holds the reconciliation.

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

* **No document carries a locale.** The register, the picker and the resolution order are a design
  bound to the `One document locale` Story (`ambre-roadmap.md` E8), not code.
* **Prose declares the same register under another name.** `meta.lang`, a BCP 47 tag, is in
  `4-Prose/ambre-spec-prose-schema.md` and unwritten. When this register lands, the two are one key:
  either Prose's `lang` becomes the shared name, or it is dropped for `locale`. The schema owns that
  rename, and nothing reads either key today.
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
