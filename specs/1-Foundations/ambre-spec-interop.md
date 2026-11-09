# AMBRE -- FEATURE: INTEROP VOCABULARY

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile, Murmure
**Implementation:** partial -- Murmure carries the shapes and reads no format yet

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
| `INTEROP-009` | `POST` | In Prose, an import of a large file MUST report its progress. | this spec section 3; `4-Prose/features/ambre-spec-prose-import-export.md` (`PROSE-IO-021`) | not built | unbuilt -- lands with the `Large-file import` Story (`ambre-roadmap.md` E6) | unimplemented |
| `INTEROP-013` | `POST` | In Trame, an import of a large file MUST report its progress. | this spec section 3; `2-Trame/features/ambre-spec-trame-import-export.md` (`TRAME-IO-013`) | not built | unbuilt -- lands with the `xlsx interop through the OOXML module` Story (`ambre-roadmap.md` E6) | unimplemented |
| `INTEROP-011` | `FAIL` | In Prose, Trame and Toile, given a file that produces nothing, the import MUST leave the document as it was and tell the user why. | this spec section 3 | `ambre_bus::interop::tests::every_answer_but_a_progress_reads_as_a_message`; Trame `ambre_trame::logic::tests::group7::an_import_starting_past_the_grid_edge_writes_nothing`; Toile `ambre_toile::logic::state::import::tests::a_broken_file_fails_without_touching_the_board`; Prose `ambre_prose::logic::interop::tests::a_file_that_does_not_read_changes_nothing` | none | verified |
| `INTEROP-014` | `FRAME` | A progress report MUST NOT raise a message the user reads. | this spec section 4 | `ambre_bus::interop::tests::every_answer_but_a_progress_reads_as_a_message` -- on the progress, which reads no message | none | verified |

### 1.3 Take a document out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `INTEROP-015` | `FAIL` | In Trame and Toile, given a document the format cannot write, the export MUST produce no file and tell the user why. | this spec section 3 | `ambre_bus::interop::tests::every_answer_but_a_progress_reads_as_a_message`; Trame `ambre_trame::logic::tests::group7::export_with_nothing_to_write_says_so`; Toile `ambre_toile::tests::chrome_ui::a_refused_export_says_why` | none | verified |

* **Budget.** none of its own: a report holds one entry per construct and outcome, and a progress
  report is one event per slice. Each conversion holds the row of its format in
  `ambre-spec-budgets.md section 1`.
* **Boundary.** none: every conversion runs in the Logic thread, and only the sealed delta of an
  import leaves the device (`ZK-001`). A report names constructs, never content.
* **Verification.** 12 `verified`, 3 `unimplemented`. Murmure reads no format yet, and only Toile
  slices an import, so Prose and Trame report no progress.

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
| Trame | `TrameImport`: CSV | `TrameExport`: CSV, native, sealed native | `TrameConstruct`: row, column, unreadable record, record or field past the grid |
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

**Code.** `crates/ambre-bus/src/interop.rs` holds the shapes, the report and the sentence. Each
application's message module declares its parameters, and its notifier toasts the sentence.

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
* **A rehearsal refusal in Prose is a write refusal, not a conversion failure.** An import the engine
  refuses midway answers the engine's own refusal, and no rule says which answer it owes.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Which formats an application reads and writes, and what each keeps | `2-Trame/features/ambre-spec-trame-import-export.md`; `3-Toile/features/ambre-spec-toile-import-export.md`; `4-Prose/features/ambre-spec-prose-import-export.md`; `5-Murmure/features/ambre-spec-murmure-import.md` |
| The native file and its seal | `ambre-spec-docfile.md` |
| The PDF writer | `ambre-spec-pdf-export.md` |
| How a converter is fetched before its first use | the `The converters fetched on demand` Story (`ambre-roadmap.md` E6) |
| The toast host | `ambre-spec-chrome.md` |
