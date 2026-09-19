# AMBRE -- FEATURE: CELL EDITING (INLINE EDITOR AND FORMULA BAR)

**Type:** Vertical (feature)
**Implementation:** partial -- the session and the commit are built; the gate before them is not.

> A user types into a cell. What reaches the document is the raw text; what the grid shows is the
> computed value. Reopening the cell shows the formula again, never its result.

One edit session, two surfaces: the editor over the cell and the bar above the grid. They are one
feature because they share one buffer: typing in either is typing in the same edit. They are also
one because a commit is the tail of a session, not a second thing.

The same document holds the gate the commit passes through: a protection says who may write, a
validation says what may be written. Neither is built.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-011` | `INV` | The viewport MUST carry each cell's raw text alongside its displayed value, so reopening the editor shows the formula. | this spec section 3 | `ambre_trame::ui::grid::tests::group2::viewport_data_preserves_formula_raw_for_editing`; `::optimistic_commit_does_not_echo_formula_as_result` | none | verified |
| `TRAME-UI-050` | `INV` | The editor MUST hold no buffer of its own: every buffer operation resolves to the shared text core. | this spec section 3 | type-level -- `CellEditor` owns a `TextEditor` and nothing else, and reaches it through `Deref` and `DerefMut`, so no second buffer can exist; `ambre_trame::ui::grid::tests::editor_parity::parity_caret_motion_and_insert`; `::parity_shift_selection`; `::parity_select_replace_and_delete`; `::parity_backspace_and_delete_forward`; `::parity_selection_collapse_on_arrows` -- the behaviour equal to the core's | none | verified |
| `TRAME-UI-055` | `INV` | The formula bar and the in-cell editor MUST show one buffer, never two. | this spec section 4 | none | unheld -- the mirroring runs on browser-only signals (`ui/view/frame.rs`), which no headless test can reach; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-155` | `INV` | An edit session MUST open on a user gesture alone. | this spec section 4 | none | fails -- while the formula input holds focus, the frame opens a session whenever its text differs from the active cell's stored value, so a collaborator writing into that cell opens an edit the user never started, seeded with the stale text; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | failing |
| `TRAME-FEAT-024` | `INV` | Protection MUST identify its owner by `UserId`, never by `pub_key`. | this spec section 3 | not built | unbuilt -- lands with the `Range protection` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-UI-010` | `OBS` | An optimistic commit MUST NOT display the formula source as its own result. | this spec section 3 | `ambre_trame::ui::grid::tests::group2::optimistic_commit_does_not_echo_formula_as_result` | none | verified |
| `TRAME-UI-091` | `OBS` | The formula bar MUST show the reference of the cell the edit will reach. | this spec section 4 | none | unheld -- the reference is a browser-only signal set beside the active cell, and no headless test reaches it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 Open an edit session and type

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-156` | `POST` | Opening a session on a cell that holds a formula MUST seed the buffer with the formula, never with its result. | this spec section 4 | none | unheld -- `cell_seed` reads the cache's `raw` field, which `TRAME-UI-011` keeps, and the seeding itself lives on `TrameView`, where no test drives it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-157` | `POST` | `Alt+Enter` MUST be the only way to put a newline inside a cell. | this spec section 4 | none | unheld -- `interact.rs` routes `Enter` with `alt` to the buffer's `newline` and every other `Enter` to a commit, and nothing drives either; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-158` | `FRAME` | An open session MUST NOT write to the document before it is committed. | this spec section 3 | none | unheld -- `begin_edit` sends no command and the only writes leave from `commit_edit` and `commit_edit_filled`, and no test states the absence; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-159` | `FRAME` | While a session is open, a key that edits the buffer MUST NOT move the grid selection. | this spec section 4 | none | unheld -- the edit-mode arm of `interact.rs` returns before the grid keymap for every buffer key, `Ctrl+A` included, and nothing asserts the grid stayed put; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-056` | `FRAME` | A composition in progress MUST NOT reach the document before it ends. | this spec section 4 | none | unheld -- `CompositionUpdate` writes only the editor preedit and `CompositionEnd` commits it into the buffer, and no test drives either; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Commit the edit

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-051` | `PRE` | A commit started from the grid MUST read the buffer only after the paint flush applied the queued edits. | this spec section 3 | none | unheld -- a grid commit parks a `PendingCommit` and the tail of `ui/view/frame.rs` drains it after `draw_edit` flushes, which holds by the order of two statements in one function. Nothing in the types forbids a second drain site, and no test reads the buffer at key time to show the difference; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-160` | `POST` | A commit MUST send the buffer's raw text to the cell the session opened on. | this spec section 3 | none | unheld -- `commit_edit` sends `SetCellText` for the editor's row and column, and the session lives on `TrameView`, on the far side of the headless seam; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-054` | `POST` | A commit of empty text MUST clear the cell. | this spec section 3 | `ambre_trame::logic::tests::group1::edited_text_overrides_generated_label` -- it writes a value, then commits an empty `SetCellText`, and reads the cell back blank | none | verified |
| `TRAME-UI-052` | `POST` | A commit MUST close the undo group, so one edit is one undo item. | this spec section 3 | `ambre_trame::logic::tests::group1::undo_then_redo_round_trips_a_cell_edit`; `group2::filtered_column_edit_is_one_undo_item` | none | verified |
| `TRAME-UI-161` | `POST` | A commit by `Enter` MUST move the selection down. | this spec section 4 | none | unheld -- the chord parks `PendingCommit::Nav(1, 0)`, drained beside the commit, and no test drives it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-171` | `POST` | A commit by `Tab` MUST move the selection right. | this spec section 4 | none | unheld -- the chord parks `PendingCommit::Nav(0, 1)`, drained beside the commit, and no test drives it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-162` | `FRAME` | A commit MUST NOT write to any cell other than the one the session opened on. | this spec section 3 | none | unheld -- `commit_edit` names one cell in the command and one in the local cache, and nothing watches the rest of the sheet; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-163` | `FAIL` | A commit the channel cannot carry MUST NOT leave the cell reading as written. | this spec section 3 | none | fails -- on a full channel `commit_edit` logs "retry next frame", updates the local cache anyway and drops the command. No retry exists, so the cell shows a value the document never took; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | failing |
| `TRAME-UI-164` | `FAIL` | A write the document cannot store MUST say so. | this spec section 3; section 5 | none | fails -- `write_cell` returns without writing when the cell lies beyond the materialization cap, with no event and no toast, against the section 5 decision that a refusal always answers; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | failing |

### 1.4 Cancel the edit

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-165` | `POST` | Escape MUST close the session and leave the cell reading as it was stored. | this spec section 4 | none | unheld -- `cancel_edit` drops the editor and resets the formula text from `cell_seed`, and both live on `TrameView`; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-015` | `FRAME` | Escape MUST emit no `TrameCommand`. | this spec section 4 | none | unheld -- the handler exists (`ui/view/lifecycle.rs::cancel_edit`) and the absence of an outbound command is asserted nowhere, which is exactly what a test has to state; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 Fill the selection with one value

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-053` | `POST` | One value typed into a multi-cell selection MUST be one undo item, not one per cell. | this spec section 4 | `ambre_trame::logic::tests::group2::batch_write_is_one_undo_item` | none | verified |
| `TRAME-UI-166` | `POST` | A filled formula MUST reach every selected cell as typed, with no reference shifted. | this spec section 4 | none | unheld -- `commit_edit_filled` clones one string per cell, so the text cannot shift, and no test fills a formula and reads a reference back; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-167` | `POST` | A fill MUST keep the selection and the active cell where they were. | this spec section 4 | none | unheld -- `commit_edit_filled` moves neither, unlike the `Nav` commit beside it, and nothing asserts it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-168` | `FRAME` | A fill MUST NOT reach a cell outside the selection. | this spec section 4 | none | unheld -- the batch is built from `selection.selected_cells()`, and nothing watches a cell beside the selection; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.6 Edit from the formula bar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-169` | `POST` | A commit from the formula bar MUST write the bar's text, not the buffer the grid last held. | this spec section 4 | none | unheld -- the commit arm pushes `formula_text` into the editor before committing, and the path is browser-only; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-092` | `FRAME` | Typing in the formula bar MUST NOT move the focus out of it. | this spec section 4 | none | unheld -- `begin_edit_external` opens the session without touching focus, so the keystrokes keep landing in the bar, and nothing asserts it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.7 Write into a protected or validated range

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-FEAT-023` | `PRE` | The write MUST be judged against the user the session carries, never an identity read out of the payload. | this spec section 3 | not built | unbuilt -- lands with the `Range protection` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-FEAT-021` | `POST` | A `Warn` validation MUST allow the mutation and surface a warning alongside it. | this spec section 3 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-047` | `FRAME` | A blocked write MUST emit no CRDT delta. | this spec section 3 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-020` | `FAIL` | A `Reject` validation MUST block the local mutation and name the rule it broke. | this spec section 3; section 5 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-048` | `FAIL` | A write by a user a protection does not allow MUST be blocked, and MUST answer with the protection's description. | this spec section 3 | not built | unbuilt -- lands with the `Range protection` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.8 Take in a write from a collaborator

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-FEAT-022` | `POST` | A remote delta violating a local validation or protection MUST be applied. | this spec section 3; section 5 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-050` | `POST` | A remote delta applied against a local validation or protection MUST surface a warning. | this spec section 5 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-049` | `FRAME` | Taking in a remote delta MUST NOT refuse it, whatever local rule it breaks. | this spec section 3; section 5 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.9 Complete a name being typed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-057` | `POST` | A function name being typed in a formula MUST offer its completions. | this spec section 5 | not built | unbuilt -- lands with the `Grid surface` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-UI-058` | `POST` | A cell reference being typed in a formula MUST offer its completions. | this spec section 5 | not built | unbuilt -- lands with the `Post-V1 surface` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-UI-170` | `FRAME` | Offering a completion MUST NOT ask the Logic thread. | this spec section 5 | not built | unbuilt -- lands with the `Grid surface` Story (`ambre-roadmap.md` E6) | unimplemented |


* **Budget.** four rows of `ambre-spec-budgets.md section 1` -- the keystroke to a draft, the
  keystroke to CRDT confirmation, `SetCellText` to the viewport, the autocompletion. Every one of
  them reads **none exists**.
* **Boundary.** none: the draft lives on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. The committed text reaches the relay sealed (`ZK-001`).
* **Verification.** 6 `verified`, 3 `failing`, 19 `untested`, 12 `unimplemented`. What holds is the
  commit path a cell text takes into the document. What is loose is the gesture around it: eleven
  rows say what an edit MUST NOT touch and nothing watches one of them. The session lives on
  `TrameView`, which owns a canvas, a textarea and Leptos signals, so the headless tier stops at
  `TrameUiState` below it. Nine unbuilt rows are the gate of section 3, whose events ship and whose
  handlers do nothing.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| types `5` into `B2` | types `7` into `B2` | one value, elected by LWW | the same number on both screens; the later writer wins | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` |
| types `=A1+1` into `B2` | deletes row 1 | the formula, its reference broken | both read `#REF!` in `B2` (`TRAME-007`) | the same property, whose op set carries `SetText` and `DeleteRow` |
| types into `B2` | sorts the range | the text on its own row, wherever the sort moved it | both see the value follow its row (`TRAME-004`) | the same property, whose op set carries `SetText` and `Sort` |
| is editing `B2`, uncommitted | writes `9` into `B2` | `9`, until A commits | A keeps typing over a value that already changed underneath, and nothing says so | none, and no rule covers it -- section 6 |

---

## 3. MODEL

**Containers.** `cells_{sheet}`, declared in `ambre-spec-trame-schema.md section 3`. The key is the
row and column uid pair, the value is the raw text, and the cell is the LWW unit. Computed values
never enter it (`TRAME-010`).

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::SetCellText { row, col, text }` | UI to Logic | the raw text. An empty `text` clears the cell; there is no `ClearCell` |
| `TrameCommand::SetCellsBatch { cells }` | UI to Logic | one text per cell, written as a single undo item |
| `TrameCommand::CommitEdit` | UI to Logic | nothing. It closes the undo group the edit opened |

**View state.** Fields of `TrameView`:

```rust
editor: Option<CellEditor>,             // the open session, or navigation mode
pending_commit: Option<PendingCommit>,  // a commit deferred to the paint flush (`TRAME-UI-051`)
editor_text_drag: bool,                 // a pointer drag selecting text inside the cell
```

```rust
pub struct CellEditor {
    pub row: u32,
    pub col: u32,
    pub(crate) core: TextEditor,  // the shared buffer, caret and selection (`TRAME-UI-050`)
}
```

**The commit, in steps:**

1. The key handler records the intent in `pending_commit` and marks the frame dirty.
2. The paint flush applies every queued insertion, so the buffer is final.
3. The drain reads the buffer, sends `SetCellText`, then `CommitEdit`.
4. The local cache takes the raw text at once, so the cell reads back before the answer arrives.
5. The selection moves in the committed direction: down for Enter, right for Tab, nowhere for a
   click outside.

**Code.**

* `ui/view/lifecycle.rs` -- `begin_edit`, `begin_edit_external`, `commit_edit`, `commit_edit_filled`,
  `cancel_edit`;
* `ui/view/interact.rs` -- the whole edit-mode input routing;
* `ui/grid/mod.rs` -- `CellEditor` and its delegation to the shared core;
* `ui/view/frame.rs` -- the formula-bar mirror and the deferred commit drain.

**The gate**, not built. Two containers declared in `ambre-spec-trame-schema.md section 3`, each
keyed by the two corners of the range it covers:

| Container | Holds |
|---|---|
| `protections_{sheet}` | the owner, the users it also allows, and a description |
| `validations_{sheet}` | the predicate, what a failure does, and the message to show |

```rust
pub enum ValidationKind {
    List(Vec<String>),                                   // a fixed value set
    ListRange(String),                                   // the values of a range, as an A1 reference
    NumberRange { min: f64, max: f64, inclusive: bool },
    NumberInteger,
    DateRange { min: Option<String>, max: Option<String> },
    TextLength { min: Option<u32>, max: Option<u32> },
    Regex(String),
    Formula(String),                                     // must return TRUE
}

pub enum ValidationOnInvalid {
    Reject,   // block the write (`TRAME-FEAT-020`)
    Warn,     // write, and warn (`TRAME-FEAT-021`)
    Info,     // mark the cell, and nothing else
}
```

**Steps, when a local write reaches the Logic thread:**

1. find the protection covering the cell; a cell no protection covers passes;
2. refuse the write when this user is neither the owner nor allowed, and reply with the description
   (`TRAME-FEAT-023`);
3. find the validation covering the cell, and test the value against it;
4. refuse the write on a failing `Reject` rule, and emit no delta (`TRAME-FEAT-020`);
5. write, and add a warning to the reply, on a failing `Warn` rule (`TRAME-FEAT-021`);
6. continue into the ordinary path: mutate, recompute, broadcast.

**Steps, when a remote delta arrives:** the delta is applied first and tested afterwards. The
test uses the author the Sync thread resolved and handed down, never an identity read out of the
payload. A
violation raises a warning and changes nothing else (`TRAME-FEAT-022`, `TRAME-FEAT-023`). Identity
is the `UserId` the session carries, never a public key, which rotates (`TRAME-FEAT-024`).

**Code.** `chrome/commands.rs` turns each refusal event into a toast, and is the only part built.
The command variants exist in `ambre-bus`; `logic/commands.rs` answers them with an empty outcome.

---

## 4. SURFACE

**Opening a session.**

| Gesture | Seeds the buffer with |
|---|---|
| a printable key | that character alone |
| `F2` | the cell's raw text, caret at the end |
| a double-click on the cell | the same as `F2` |
| typing in the formula bar | the bar's text; the in-cell editor opens without stealing focus |

**Keys, while a session is open.** An open editor claims the keyboard. Every key below acts on the
buffer, and the grid map of `features/ambre-spec-trame-selection.md section 4` applies only when no
session is open. The two undo chords are read before that split and reach the document either way
(`features/ambre-spec-trame-undo.md section 4`).

| Key | Effect |
|---|---|
| `Enter` | commits and moves down |
| `Tab` | commits and moves right |
| `Alt+Enter` | inserts a newline inside the cell, and is the only way to add one |
| `Ctrl+Enter` | writes the value into every selected cell and keeps the selection (`TRAME-UI-053`) |
| `Escape` | cancels: nothing leaves the UI thread (`TRAME-UI-015`) |
| `Up`, `Down` | move the caret between the cell's lines, and never move to another cell |
| `Left`, `Right`, `Home`, `End` | move the caret; `Shift` extends the text selection on either axis |
| `Backspace`, `Delete`, a printable character | edit the buffer |
| `Ctrl+A` | selects the whole buffer, never the grid |

The four committing chords take effect just after the paint flush, because the buffer is only final
there (`TRAME-UI-051`).

**Pointer.** A press inside the edited cell starts a text selection; a press anywhere else commits
and selects the cell under it. A right-click commits too, so the menu opens on the cell it points
at. A double-click inside selects the word.

**The formula bar**, above the grid, is DOM chrome: a name box and a text input.

| Part | Behaviour |
|---|---|
| the name box | shows the active cell's reference, and only shows it (`TRAME-UI-091`) |
| the input | always editable. Focus or a keystroke opens a session on the active cell (`TRAME-UI-092`) |
| `Enter` in the bar | commits and moves down; `Ctrl+Enter` fills the selection instead |
| `Tab` in the bar | commits and moves right |
| `Escape` in the bar | cancels, and the bar reverts to the cell's stored value |
| while a session is open | the bar and the in-cell editor show one buffer (`TRAME-UI-055`) |

**Rendering.**

| What | Drawn as |
|---|---|
| the edited cell | its own background and a focus border, painted under the shared core |
| the buffer | the shared core's glyphs, selection band and blinking caret, including the live preedit |
| a committed cell whose value has not arrived | unconfirmed, per `features/ambre-spec-trame-grid-viewport.md section 1` |
| the formula bar | the cell reference, and the buffer while editing |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The buffer is the shared text core, not a field of its own | Trame's own `content: String` plus a byte cursor | it is what the earlier design carried, and it is a second copy of an engine that would drift. A spreadsheet cell needs IME, bidi, word selection and a caret that survives shaping -- the same engine Toile's text elements need; the wrapper only binds it to a cell |
| The inline editor is a transparent DOM textarea over the cell | a canvas-only editor | it would have to reimplement composition. Over the DOM, IME, the OS text menu and the native clipboard events come for free, and the canvas paints only the cell behind it |
| A grid commit is deferred to the paint flush | applying each edit synchronously | it needs a font context on the input path. The shared core applies queued edits when a font context is live, which is paint time; reading the buffer at key time would miss the last keystroke |
| The raw text is the document, the value is derived | storing the computed value beside the text | two peers on different locales, and any future function set, must read one source. A computed value in the CRDT would be a second one |
| A refusal answers, and never stays silent | dropping the write with no reply | a write that vanishes teaches the user that the application is unreliable; a message naming the rule teaches the rule |
| A remote delta is applied even when it violates a local rule | refusing the remote delta | it breaks convergence, leaving two peers on two documents, which costs more than a value out of range. So a rule is an input aid, never an integrity barrier, and the configuration surface must say so |
| Protection is enforced by the client, not by the server | a server-side check | it would need the plaintext. The relay is blind and holds ciphertext, so it cannot read a range, let alone judge a write against one. A modified client can therefore bypass a protection, and the invitation surface must say so |
| A protection names its owner by user, not by device key | storing the public key, which is what the earlier design carried | a device key rotates when a member is revoked, and every protection naming one would then refuse its own owner |
| A filled selection is one batch command | `SetCellText` in a loop | a stream of one command per cell would be one undo item per cell, and an undo of "type this everywhere" must be one step |
| Both completions are answered on the UI thread | asking the Logic thread for the candidate list on each keystroke | a completion appears between two keystrokes, and a round trip cannot promise that. The function catalogue is fixed at compile time and the neighbouring values are already in the current window |

---

## 6. NOT BUILT, AND NOT YET RULED

* **An edit over a value that changed underneath.** The open buffer is local, so a remote write to
  the same cell lands in the cache while the user types. The commit overwrites it. Nothing warns
  the user and no rule says what should happen: the two-peer table's last row is the case.
* **Autocompletion** (`TRAME-UI-057`, `TRAME-UI-058`). Neither exists in the code. There is no
  completion module, no static function list, no dropdown, and no function of either name anywhere
  in the workspace. What exists is a design, and the path it names is empty.
* **The name box only reports.** It is a `span`, not an input, so a user cannot type `B240` to jump
  there, and cannot select a named range by name. Named ranges exist in the engine
  (`features/ambre-spec-trame-named-ranges.md`) with no way to reach one from here.
* **The pointer maps to one line.** `editor_local_point` returns a local `y` of zero, so a click in
  a multi-line cell resolves against the first line. `Alt+Enter` and the up and down caret moves are
  built, which is what makes the gap visible.
* **A commit the channel cannot carry is lost, and the cell shows it anyway.** On a full UI-to-Logic
  channel `commit_edit` logs "retry next frame" and updates the local cache regardless
  (`TRAME-UI-163`). No retry exists anywhere: the command is dropped, and the cell reads as written
  on one screen and blank on every other.
* **A write beyond the materialization cap vanishes** (`TRAME-UI-164`). `write_cell` returns without
  writing and without an event, against the section 5 decision that a refusal always answers.
* **A collaborator's write can open an edit session** (`TRAME-UI-155`). While the formula input holds
  focus, each frame compares its text to the active cell's stored value and opens a session on any
  difference. A remote write to that cell makes them differ, so an edit opens seeded with the stale
  text. The next `Enter` commits it over the collaborator's value. It is the same case as the
  first bullet above, reached without the user typing at all.
* **The gate is decided and unbuilt** (`TRAME-FEAT-020..024`). The commands, the events and the
  toasts exist. The handlers return an empty outcome, so a validation never blocks a value and a
  protection never blocks a user. No container is written and no surface offers to create either.
* **The gate covers one path, and the document has four.** The design above tests a cell write.
  Paste, autofill, replace-all and a structural delete reach the same cells and pass no gate, and no
  rule states whether they must.
* **The session has no headless test.** Six rows above say `none` for the same reason. `TrameView`
  is reachable only with a canvas and Leptos signals, while `TrameUiState` under it is driven by
  tests today. The seam exists, and the session is on the wrong side of it.
