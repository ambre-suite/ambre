# AMBRE -- FEATURE: INTEROP VOCABULARY

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile, Murmure

> A user brings a file in, or takes a document out, and reads the conversion report: what the file
> kept and what it lost. The words and the counts are the same in every application.

Each application owns its formats and the constructs its documents hold. This document owns the rest:
the command and event shapes, the report, the progress and the failure.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `INTEROP`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `INTEROP-001` | `INV` | Every application MUST carry its imports, its exports and their answers on the one shared command shape and the one shared event shape. | this spec section 3 | type-level -- the command and the event of Prose, Trame, Toile and Murmure each hold the shared shape as their one interop variant | none | verified |
| `INTEROP-002` | `INV` | A conversion that produced nothing MUST answer with an error, never with a report. | this spec section 3 | structural -- the shared event carries a report on the two finished answers and an error on the failure alone | none | verified |
| `INTEROP-003` | `INV` | A report MUST count one construct once per outcome, never once per occurrence. | this spec section 3 | `ambre_bus::interop::tests::the_same_construct_twice_is_one_entry_with_its_count` | none | verified |
| `INTEROP-004` | `INV` | A conversion error MUST print in a log the length of its reason, never the reason. | this spec section 3; `ZK-006` | `ambre_bus::interop::tests::an_error_prints_the_length_of_its_reason_alone` | none | verified |
| `INTEROP-005` | `INV` | In Prose, Trame and Toile, an import MUST count as a write, so the role gate withholds it from a reader. | this spec section 3; `1-Foundations/ambre-spec-architecture.md section 4` | `ambre_bus::interop::tests::an_import_writes_and_an_export_reads`; Trame `ambre_logic::app::tests::an_import_writes_and_an_export_reads_in_every_application` -- the CSV import; Toile `ambre_logic::app::tests::an_import_writes_and_an_export_reads_in_every_application` -- the SVG import; Prose `ambre_logic::app::tests::an_import_writes_and_an_export_reads_in_every_application` -- the Markdown import | none | verified |
| `INTEROP-006` | `INV` | In Prose, Trame and Toile, an export MUST NOT count as a write, so a reader keeps it. | this spec section 3; `1-Foundations/ambre-spec-architecture.md section 4` | `ambre_bus::interop::tests::an_import_writes_and_an_export_reads`; Trame `ambre_logic::app::tests::an_import_writes_and_an_export_reads_in_every_application` -- the CSV and the native exports; Toile `ambre_logic::app::tests::an_import_writes_and_an_export_reads_in_every_application` -- the SVG and the PDF exports; Prose `ambre_logic::app::tests::an_import_writes_and_an_export_reads_in_every_application` -- the Markdown export | none | verified |
| `INTEROP-007` | `OBS` | In Prose, Trame and Toile, a finished conversion MUST reach the user as one sentence naming each construct lost, with its count. | this spec section 4 | `ambre_bus::interop::tests::a_report_reads_as_one_sentence`; Trame `ambre_trame::tests::chrome_ui::an_interop_report_reaches_the_user`; Toile `ambre_toile::tests::chrome_ui::an_import_report_reaches_the_user`; Prose `ambre_prose::tests::chrome_panel::an_interop_report_reaches_the_user` | none | verified |
| `INTEROP-012` | `OBS` | In Murmure, a finished conversion MUST reach the user as one sentence naming each construct lost, with its count. | this spec section 4 | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |
| `INTEROP-010` | `INV` | A progress report MUST NOT count past its total. | this spec section 3 | `ambre_bus::interop::tests::a_progress_never_passes_its_total` | none | verified |

### 1.2 Bring a file in

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `INTEROP-008` | `POST` | In Toile, an import applied in slices MUST report its progress after every slice but the last. | this spec section 3; `3-Toile/features/ambre-spec-toile-import-export.md` (`TOILE-IO-028`) | Toile `ambre_toile::logic::state::import::tests::a_large_import_lands_one_slice_at_a_time` -- each slice short of the end reports further on against one total, and the last one reports the report instead | none | verified |
| `INTEROP-009` | `POST` | In Prose, an import of a large file MUST report its progress. | this spec section 3; `4-Prose/features/ambre-spec-prose-import-export.md` (`PROSE-IO-021`) | Prose `ambre_prose::logic::sliced::tests::a_large_import_lands_one_slice_at_a_time` -- each slice short of the end reports further on against one total, and the last one reports the report instead | none | verified |
| `INTEROP-013` | `POST` | In Trame, an import of a large file MUST report its progress. | this spec section 3; `2-Trame/features/ambre-spec-trame-import-export.md` (`TRAME-IO-013`) | Trame `ambre_trame::logic::xlsx_tests::a_large_import_lands_one_slice_at_a_time` -- each slice short of the end reports further on against one total, and the last one reports the report instead | none | verified |
| `INTEROP-011` | `FAIL` | In Prose, Trame and Toile, given a file that produces nothing, the import MUST leave the document as it was and tell the user why. | this spec section 3 | `ambre_bus::interop::tests::every_answer_but_a_progress_reads_as_a_message`; Trame `ambre_trame::logic::tests::group7::an_import_starting_past_the_grid_edge_writes_nothing`; Toile `ambre_toile::logic::state::import::tests::a_broken_file_fails_without_touching_the_board`; Prose `ambre_prose::logic::interop::tests::a_file_that_does_not_read_changes_nothing` | none | verified |
| `INTEROP-014` | `FRAME` | A progress report MUST NOT raise a message the user reads. | this spec section 4 | `ambre_bus::interop::tests::every_answer_but_a_progress_reads_as_a_message` -- on the progress, which reads no message | none | verified |

### 1.3 Take a document out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `INTEROP-015` | `FAIL` | In Trame and Toile, given a document the format cannot write, the export MUST produce no file and tell the user why. | this spec section 3 | `ambre_bus::interop::tests::every_answer_but_a_progress_reads_as_a_message`; Trame `ambre_trame::logic::tests::group7::export_with_nothing_to_write_says_so`; Toile `ambre_toile::tests::chrome_ui::a_refused_export_says_why` | none | verified |

### 1.4 Hold a file a converter test reads

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `INTEROP-016` | `INV` | Each input fixture MUST have exactly one provenance line: in `fixtures/THIRD_PARTY.md` when copied from another project, in `fixtures/AUTHORED.md` otherwise. | this spec section 3 | ci-gate -- `scripts/fixture_provenance.py`, run by `just fixture-provenance` in the CI `lint` job, refuses an input fixture in neither register, and one with two lines. A generator script no line names counts as an input fixture. The tests of `scripts/test_fixture_provenance.py` plant each one | none | verified |
| `INTEROP-017` | `INV` | Each provenance line MUST name an input fixture that exists. | this spec section 3 | ci-gate -- `scripts/fixture_provenance.py`, run by `just fixture-provenance` in the CI `lint` job, refuses a line whose path names no file, and a line naming an `.expected` file or a generator script. The tests of `scripts/test_fixture_provenance.py` plant each one | none | verified |
| `INTEROP-018` | `INV` | Each file under a `fixtures` directory of a crate MUST weigh no more than the cap of its budget row. | this spec section 3; `ambre-spec-budgets.md section 7` | ci-gate -- `scripts/fixture_provenance.py`, run by `just fixture-provenance` in the CI `lint` job, reads the cap from the `Interop input fixture` row and refuses each file of a fixture directory over it, the `.expected` files included. A budget register with no such row fails too. The tests of `scripts/test_fixture_provenance.py` plant each one | none | verified |
| `INTEROP-019` | `INV` | A line of `fixtures/THIRD_PARTY.md` MUST name an `https` repository, a full commit hash, the upstream path and the copyright holder. | this spec section 3 | ci-gate -- `scripts/fixture_provenance.py`, run by `just fixture-provenance` in the CI `lint` job, refuses a repository that is no `https` URL, a commit of fewer than 40 hexadecimal digits, an empty upstream path and an empty copyright holder. The tests of `scripts/test_fixture_provenance.py` plant each one | none | verified |
| `INTEROP-020` | `INV` | A copied input fixture MUST be under a licence of the allowed set, and the licence text of its repository MUST sit under `fixtures/licences/`. | this spec section 3 | ci-gate -- `scripts/fixture_provenance.py`, run by `just fixture-provenance` in the CI `lint` job, refuses a licence outside the allowed set, a repository with no licence text, and a licence text no line copies from. The tests of `scripts/test_fixture_provenance.py` plant each one | none | verified |
| `INTEROP-021` | `INV` | An input fixture MUST NOT be named after a bug report, by its own name or by its upstream name. | this spec section 3 | ci-gate -- `scripts/fixture_provenance.py`, run by `just fixture-provenance` in the CI `lint` job, refuses a tracker name then a number, or a bare number of five digits or more, in the name of the fixture or of its upstream file. The tests of `scripts/test_fixture_provenance.py` plant each one | none | verified |
| `INTEROP-022` | `INV` | Each input fixture MUST have an oracle: an `.expected` file beside it, or a test source or generator script of its crate that names it. | this spec section 3 | ci-gate -- `scripts/fixture_provenance.py`, run by `just fixture-provenance` in the CI `lint` job, refuses an input fixture with no `.expected` beside it that no `.rs` file of its crate and no generator script of its crate names. A test of another crate counts for nothing. The tests of `scripts/test_fixture_provenance.py` plant each one | none | verified |
| `INTEROP-023` | `INV` | Each `.expected` file MUST sit beside the input fixture whose result it holds. | this spec section 3 | ci-gate -- `scripts/fixture_provenance.py`, run by `just fixture-provenance` in the CI `lint` job, refuses an `.expected` file with no input fixture of the same name beside it, in either spelling. The tests of `scripts/test_fixture_provenance.py` plant each one | none | verified |
| `INTEROP-024` | `INV` | A line of `fixtures/AUTHORED.md` MUST give `hand-written` or a generator script as its origin, and that script MUST exist and name the file. | this spec section 3 | ci-gate -- `scripts/fixture_provenance.py`, run by `just fixture-provenance` in the CI `lint` job, refuses an origin that is neither `hand-written` nor `generated by` a script, an empty `Holds` cell, a missing script, and a script that does not name the file. The tests of `scripts/test_fixture_provenance.py` plant each one | none | verified |
| `INTEROP-025` | `INV` | An input fixture MUST NOT be a real document: a file a person wrote for their own use, attached to a bug report or found on the web. | this spec section 3 | human-review -- the reviewer opens each input fixture a change adds, and reads its `Holds` cell or its upstream path | unheld -- `domain-meaning` -- the gate reads names, sizes and registers, never what a file says | human-review |
| `INTEROP-026` | `INV` | The licence a line of `fixtures/THIRD_PARTY.md` names MUST be the licence of its upstream repository at the commit the line names. | this spec section 3 | human-review -- the reviewer reads the licence file of the upstream repository at the commit each new line names | unheld -- `domain-meaning` -- the gate checks that the licence named is one of the allowed set, never that it is the true one | human-review |

* **Budget.** none of its own: a report holds one entry per construct and outcome, and a progress
  report is one event per slice. Each conversion holds the row of its format in
  `ambre-spec-budgets.md section 1`. An input fixture holds the `Interop input fixture` row of its
  section 7 (`INTEROP-018`).
* **Boundary.** none: every conversion runs in the Logic thread, and only the sealed delta of an
  import leaves the device (`ZK-001`). A report names constructs, never content.
* **Verification.** 23 `verified`, 2 `human-review`, 1 `unimplemented`. Murmure reads no format yet.
  Toile, Trame and Prose slice a large import and report its progress. Whether an input fixture is
  a real document, and whether its licence is the true one, stay a reader's call.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| imports a file that loses two footnotes | watches | the file's blocks, as one undo item | Alice reads the import and the two footnotes lost; Bob sees the new content and no message (`INTEROP-007`) | none -- the report is Alice's alone, and no test drives two peers |
| exports, as a reader | edits | Bob's edit alone: the export writes nothing (`INTEROP-006`) | Alice gets her file; Bob sees nothing of it | Trame `ambre_logic::app::tests::an_import_writes_and_an_export_reads_in_every_application` |

---

## 3. MODEL

**Containers.** None of its own. An import writes the containers each application declares. An
export reads the document and writes nothing.

**Commands and events** (`ambre_bus::interop`). One shape each, generic over what differs:

| Shape | Direction | Carries |
|---|---|---|
| `InteropRequest::Import(I)` | UI to Logic | `I`, the application's import: the format with its options, and the bytes |
| `InteropRequest::Export(E)` | UI to Logic | `E`, the application's export: the format with its options |
| `InteropAnswer::ImportCompleted { report }` | Logic to UI | what the import carried and lost |
| `InteropAnswer::ExportReady { file, report }` | Logic to UI | the file name, the media type and the bytes; what the format could not express |
| `InteropAnswer::Progress(progress)` | Logic to UI | the steps done of a total, in the application's own unit |
| `InteropAnswer::Failed(failure)` | Logic to UI | which way the file went, and why nothing was produced |

```rust
// ambre_bus::interop
pub enum InteropRequest<I, E> { Import(I), Export(E) }

pub enum InteropAnswer<C> {
    ImportCompleted { report: ConversionReport<C> },
    ExportReady     { file: ExportedFile, report: ConversionReport<C> },
    Progress(ConversionProgress),
    Failed(InteropFailure),
}

/// Each application names its own constructs, as one closed enum.
pub trait Construct: Copy + Eq + fmt::Debug {
    fn label(self) -> &'static str;
    fn plural(self) -> &'static str;
}

pub enum Outcome { Carried, Moved, Approximated, Dropped }
pub struct ReportEntry<C> { pub construct: C, pub outcome: Outcome, pub count: Count<Occurrences> }
pub struct ConversionReport<C> { direction: Direction, format: FileFormat, entries: Vec<ReportEntry<C>> }

pub enum InteropError { ParseFailed(String), TooLarge(Count<Octets>), Stopped(String) }
pub struct ConversionProgress { direction: Direction, done: Count<Steps>, total: Count<Steps> }
```

**Per application.** Each declares its three parameters beside its engine:

| Application | Import `I` | Export `E` | Construct `C` |
|---|---|---|---|
| Prose | `ProseImport`: Markdown | `ExportFormat`: native, Markdown | `ProseConstruct`: a feature a format lacks, an element a reader skips, a block it moves |
| Trame | `TrameImport`: CSV, Excel | `TrameExport`: CSV, Excel, native, sealed native | `TrameConstruct`: row, column, unreadable record, record or field past the grid, cell past the grid, and each construct of the Excel codec |
| Toile | `ToileImport`: SVG, picture, Figma, Miro | `ToileExport`: SVG, PDF, native, sealed native | `ToileConstruct`: element, sticky note, group, connector, picture, text, paint |
| Murmure | `MurmureImport`: none | `MurmureExport`: none | `MurmureConstruct`: none |

**Steps, on an import:**

1. the reader reads the whole file into the application's form. A file that does not read answers a
   failure and writes nothing (`INTEROP-002`, `INTEROP-011`);
2. the reader records each construct it moves, approximates or drops, once per outcome
   (`INTEROP-003`);
3. the application writes the form as one undo item. A sliced import reports its progress after
   every slice but the last (`INTEROP-008`);
4. the application records what it carried, and answers the report.

**Steps, on an export:** the writer walks the document, records what the format cannot express, and
answers the file with its report. A document the writer cannot write answers a failure
(`INTEROP-015`).

**The sentence.** An import opens on what it carried: `Imported 3 elements`. Each outcome that lost
something follows, worst last, naming three constructs and counting the rest:
`dropped: footnote (2), HTML, and 2 more`. A failure reads `Import failed:` or `Export failed:` and
its reason.

**Input fixtures.** A converter test reads files in a foreign format that sit beside the code. An
input fixture is any file under a directory named `fixtures` inside `crates/`, except an `.expected`
file and a generator script. Each one comes from one of three sources:

| Source | Register | What the line names |
|---|---|---|
| the test suite of an open-source project, at one commit | `fixtures/THIRD_PARTY.md` | the repository, the commit, the upstream path, the licence, the copyright holder |
| a file written by hand in this repository | `fixtures/AUTHORED.md` | `hand-written`, and what the file holds |
| a committed script's output, from a hand-written file or from the codec's own output | `fixtures/AUTHORED.md` | the script, and what the file holds |

Three sources are forbidden: a real document, a file named after a bug report, and a file under a
licence outside the allowed set. A bug report attachment is the usual real document. A user sent
it to an upstream tracker, and the user never agreed to its copy here. A tracker name then a number,
such as `tdf123456` or `bug-1234`, marks such a file, as does a bare number of five digits or more.

The allowed set holds the licences an AGPL-3.0 repository may carry. The permissive ones are `MIT`,
`ISC`, `0BSD`, `Zlib`, `Unlicense`, `BSD-2-Clause`, `BSD-3-Clause`, `Apache-2.0` and `CC0-1.0`. The
others are `MPL-2.0`, `CC-BY-4.0`, and the `-or-later` forms of the GNU licences: `GPL-2.0`,
`GPL-3.0`, `LGPL-2.1`, `LGPL-3.0` and `AGPL-3.0`. `GPL-2.0-only` is out: it does not combine with
the AGPL. Each licence text sits under `fixtures/licences/`, named after the repository:
`calamine.txt` for `tafia/calamine`.

Each input fixture has an oracle. It is an `.expected` file beside it, as `a.expected` or
`a.xlsx.expected`, or a test that names the file and asserts on what it reads. A hand-written source
of a generated file has the script that reads it as its oracle. Each file of a fixture directory
holds the `Interop input fixture` row of `ambre-spec-budgets.md section 7`.

**Code.** `crates/ambre-bus/src/interop.rs` holds the shapes, the report and the sentence. Each
application's message module declares its parameters, and its notifier toasts the sentence.
`scripts/fixture_provenance.py` holds the input fixtures against the two registers.

---

## 4. SURFACE

| Trigger | What the user reads |
|---|---|
| a finished import or export | the sentence, as an information toast when nothing was lost and a warning otherwise (`INTEROP-007`) |
| a conversion that produced nothing | a warning naming the direction and the reason (`INTEROP-011`, `INTEROP-015`) |
| a slice of a running import | nothing: the view draws a bar, and a toast per slice would bury the user (`INTEROP-014`) |

The gesture that starts a conversion is each application's: a menu row, the palette, a drop. The
notifier of each application maps the shared event to a toast, and the toast host is
`ambre-spec-chrome.md`'s.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One generic shape, three parameters per application | one closed list of every construct of the suite | a sheet cannot lose a sticky note. One list would make every application match the constructs of the others, and a new reader in one application would touch the three others |
| One generic shape, three parameters per application | one event per format and per application | it is the vocabulary this document replaces: three spellings of one behaviour, and a fourth with each new format |
| The shared shapes live in `ambre-bus` as `InteropRequest` and `InteropAnswer` | a `Command` and an `Event` enum of `ambre-bus` | `BUS-032` keeps an application's own message out of the bus. These shapes are no application's: each application's command and event embeds them, as a find embeds the shared query |
| The shared shapes live in `ambre-bus` as `InteropRequest` and `InteropAnswer` | a copy of the shapes in each application | the copies are the four vocabularies this document replaces, and the notifier of each application could no longer share the sentence |
| A count per construct and outcome | one line per occurrence | a long file buries the user in repeats, and a list nobody reads at the corner of a screen is a loss in silence by another means |
| A count per construct and outcome | free text per occurrence | a sentence the reader writes cannot be counted, merged or tested, and it may quote the file into a log |
| A progress in the application's own unit | a percentage | the reader knows its units, the view needs only the ratio, and a percentage rounds a small step to nothing |
| An input fixture is any file of a `fixtures` directory of a crate | a list of the extensions a converter reads | a file of a new format would pass unseen until someone extended the list, and the gate exists to see the file nobody declared |
| Two registers, one per origin, under `fixtures/` | a provenance file beside each input fixture | the copied file without its sidecar is the defect itself, and the licence texts would scatter across the crates |
| The gate reads names, sizes and the two registers | the gate reads the metadata of each file, such as the author of a DOCX | half the formats carry none, and a real document cleared of it passes. The reviewer reads the content anyway (`INTEROP-025`) |
| An oracle is an `.expected` file beside the input, or a test of its crate that names it | an `.expected` beside every input, as the story asked | the codecs already assert on what they read in Rust, and a dump for each would duplicate those tests. The test pins that the file is read and what the test asserts, not the whole result: a weaker oracle, kept as a known gap of the story |
| A cap per file, in the budget register | a cap on the total of every fixture | the total grows with each format a story brings, and a cap one story crosses is a cap the next story raises without reading it |
| The report and the error are two answers | one answer carrying both | "the file opened with three losses" and "the file did not open" are different facts, and a caller acts on them differently |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Murmure converts nothing yet.** Its import and export carry no format, so neither command can be
  built. The `Chat import` Story (`ambre-roadmap.md` E5) brings the first reader.
  The `Murmure export` Story (`ambre-roadmap.md` E5) brings the first writer.
* **No view draws the progress.** Toile reports each slice, and no surface draws a bar from it.
* **A Prose export failure has no test.** The native export of a document whose snapshot fails
  answers the shared failure, and no test drives that path.
* **An approximation has few readers yet.** The outcome exists for the Word and spreadsheet writers.
  Toile's SVG reader records one, a group reduced to its one member. Its PowerPoint map records a
  sticky note, a freehand stroke and a group, among others.
* **A refused import in Prose answers a write refusal, not a conversion failure.** An import list the
  nesting rule refuses answers the engine's own refusal, and no rule says which answer it owes.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Which formats an application reads and writes, and what each keeps | `2-Trame/features/ambre-spec-trame-import-export.md`; `3-Toile/features/ambre-spec-toile-import-export.md`; `4-Prose/features/ambre-spec-prose-import-export.md`; `5-Murmure/features/ambre-spec-murmure-import.md` |
| The native file and its seal | `ambre-spec-docfile.md` |
| The PDF writer | `ambre-spec-pdf-export.md` |
| How a converter is fetched before its first use | the `The converters fetched on demand` Story (`ambre-roadmap.md` E6) |
| The toast host | `ambre-spec-chrome.md` |
| The fixtures of Ambre's own documents, under `fixtures/current/` and `fixtures/frozen/` | `ambre-spec-fixtures.md` |
