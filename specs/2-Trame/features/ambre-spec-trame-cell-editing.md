# AMBRE -- FEATURE: CELL EDITING (INLINE EDITOR AND FORMULA BAR)

**Type:** Vertical (feature)
**Implementation:** partial -- the session and the commit are built; the gate before them is not.

> A user types into a cell. What reaches the document is the raw text; what the grid shows is the
> computed value. Reopening the cell shows the formula again, never its result.

One edit session, two surfaces: the editor over the cell and the bar above the grid. They are one
feature because they share one buffer -- typing in either is typing in the same edit -- and because
a commit is the tail of a session, not a second thing.

The same document holds the gate the commit passes through: a protection says who may write, a
validation says what may be written. Neither is built.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-UI-010` | An optimistic commit MUST NOT display the formula source as its own result. | this spec section 3 | `ambre_trame::ui::grid::tests::group2::optimistic_commit_does_not_echo_formula_as_result` | verified |
| `TRAME-UI-011` | The viewport MUST carry each cell's raw text alongside its displayed value, so reopening the editor shows the formula. | this spec section 3 | `ambre_trame::ui::grid::tests::group2::viewport_data_preserves_formula_raw_for_editing`, `::optimistic_commit_does_not_echo_formula_as_result` | verified |
| `TRAME-UI-015` | Escape MUST cancel an edit in progress without emitting any `TrameCommand`. | this spec section 4 | none. The handler exists (`ui/view/lifecycle.rs::cancel_edit`), the absence of an outbound command is asserted nowhere -- and an absence is exactly what a test has to state | unverified |
| `TRAME-UI-050` | Every buffer operation of the editor MUST resolve to the shared text core. | this spec section 3 | `ambre_trame::ui::grid::tests::editor_parity::parity_caret_motion_and_insert`, `::parity_shift_selection`, `::parity_select_replace_and_delete`, `::parity_backspace_and_delete_forward`, `::parity_selection_collapse_on_arrows` | verified |
| `TRAME-UI-051` | A commit started from the grid MUST read the buffer only after the paint flush applied the queued edits. | this spec section 3 | type-level: a grid commit sets `pending_commit: Option<PendingCommit>` (`ui/view/mod.rs`) and the only drain site is the tail of `ui/view/frame.rs`, after `draw_edit` flushes | verified |
| `TRAME-UI-052` | A commit MUST close the undo group, so one edit is one undo item. | this spec section 3 | `ambre_trame::logic::tests::group1::undo_then_redo_round_trips_a_cell_edit`, `group2::filtered_column_edit_is_one_undo_item` | verified |
| `TRAME-UI-053` | One value typed into a multi-cell selection MUST be one undo item, not one per cell. | this spec section 4 | `ambre_trame::logic::tests::group2::batch_write_is_one_undo_item` | verified |
| `TRAME-UI-054` | A commit of empty text MUST clear the cell. | this spec section 3 | none. `commit_edit` sends the empty text and the engine treats it as a clear; nothing drives the empty commit | unverified |
| `TRAME-UI-055` | The formula bar and the in-cell editor MUST show one buffer, never two. | this spec section 4 | none. The mirroring runs on browser-only signals (`ui/view/frame.rs`), which no headless test can reach | unverified |
| `TRAME-UI-056` | A composition in progress MUST NOT reach the document before it ends. | this spec section 4 | none. `CompositionUpdate` writes only the editor preedit and `CompositionEnd` commits it into the buffer; no test drives either | unverified |
| `TRAME-UI-091` | The formula bar MUST show the reference of the cell the edit will reach. | this spec section 4 | none. The reference is a browser-only signal set beside the active cell, and no headless test reaches it | unverified |
| `TRAME-UI-092` | Typing in the formula bar MUST NOT move the focus out of it. | this spec section 4 | none. `begin_edit_external` opens the session without touching focus, precisely so the keystrokes keep landing in the bar; nothing asserts it | unverified |
| `TRAME-UI-057` | A function name being typed in a formula MUST offer its completions. | this spec section 5 | not built -- lands with the "Grid surface" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-UI-058` | A cell reference being typed in a formula MUST offer its completions. | this spec section 5 | not built -- lands with the "Post-V1 surface" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-FEAT-020` | A `Reject` validation MUST block the local mutation and emit no CRDT delta. | this spec section 3 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-021` | A `Warn` validation MUST allow the mutation and surface a warning alongside it. | this spec section 3 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-022` | A remote delta violating a local validation MUST be applied and MUST surface a warning; it MUST NOT be rejected. | this spec section 3; section 5 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-023` | Range protection MUST block a local edit by a non-owner, and MUST warn rather than reject on a remote violation. | this spec section 3 | not built -- lands with the "Range protection" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-FEAT-024` | Protection MUST identify its owner by `UserId`, never by `pub_key`. | this spec section 3 | not built -- lands with the "Range protection" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** four rows of `ambre-spec-budgets.md section 1`: the keystroke to a draft in
  < 2 ms, the keystroke to CRDT confirmation in < 15 ms, `SetCellText` to the viewport in < 5 ms,
  and the autocompletion in < 2 ms. Every one of them reads **none exists**.
* **Boundary.** none: the draft lives on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path, and the committed text reaches the relay sealed (`ZK-001`).
* **Verification.** 6 `verified`, 6 `unverified`, 7 `unimplemented`. The six gaps share one cause:
  the edit session lives on `TrameView`, which owns a canvas, a textarea and Leptos signals, so the
  headless tier does not reach it. The buffer below it is covered by parity and the engine above it
  by the undo tests; what is loose is the session between them. Five of the seven unbuilt rows are
  the gate of section 3, whose events and toasts ship and whose handlers do nothing.

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

**Containers.** `cells_{sheet}`, declared in `ambre-spec-trame-schema.md section 3`: the key is the
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

**Steps, when a remote delta arrives:** the delta is applied first and tested afterwards, against
the author the Sync thread resolved and handed down, never an identity read out of the payload. A
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

**Keys, while a session is open.** An open editor claims the keyboard: every key below acts on the
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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The buffer is the shared text core, not a field of its own | a spreadsheet cell needs IME, bidi, word selection and a caret that survives shaping. That is the same engine Toile's text elements need, and a second copy would drift; the wrapper only binds it to a cell | Trame's own `content: String` plus a byte cursor, which is what the earlier design carried |
| The inline editor is a transparent DOM textarea over the cell | IME, the OS text menu and the native clipboard events come for free, and the canvas paints only the cell behind it | a canvas-only editor, which would have to reimplement composition |
| A grid commit is deferred to the paint flush | the shared core applies queued edits when a font context is live, which is paint time; reading the buffer at key time would miss the last keystroke | applying each edit synchronously, which needs a font context on the input path |
| The raw text is the document, the value is derived | two peers on different locales, and any future function set, must read one source. A computed value in the CRDT would be a second one | storing the computed value beside the text |
| A refusal answers, and never stays silent | a write that vanishes teaches the user that the application is unreliable; a message naming the rule teaches the rule | dropping the write with no reply |
| A remote delta is applied even when it violates a local rule | rejecting it would leave two peers on two documents, which costs more than a value out of range. So a rule is an input aid, never an integrity barrier, and the configuration surface must say so | refusing the remote delta, which breaks convergence |
| Protection is enforced by the client, not by the server | the relay is blind and holds ciphertext, so it cannot read a range, let alone judge a write against one. A modified client can therefore bypass a protection, and the invitation surface must say so | a server-side check, which would need the plaintext |
| A protection names its owner by user, not by device key | a device key rotates when a member is revoked, and every protection naming one would then refuse its own owner | storing the public key, which is what the earlier design carried |
| A filled selection is one batch command | a stream of one command per cell would be one undo item per cell, and an undo of "type this everywhere" must be one step | `SetCellText` in a loop |
| Both completions are answered on the UI thread | a completion appears between two keystrokes, and a round trip to the Logic thread cannot promise that. The function catalogue is fixed at compile time and the neighbouring values are already in the current window | asking the Logic thread for the candidate list on each keystroke |

---

## 6. NOT BUILT, AND NOT YET RULED

* **An edit over a value that changed underneath.** The open buffer is local, so a remote write to
  the same cell lands in the cache while the user types, and the commit overwrites it. Nothing warns
  the user and no rule says what should happen: the two-peer table's last row is the case.
* **Autocompletion** (`TRAME-UI-057`, `TRAME-UI-058`). Neither exists in the code: there is no
  completion module, no static function list, no dropdown, and no function of either name anywhere
  in the workspace. What exists is a design, and the path it names is empty.
* **The name box only reports.** It is a `span`, not an input, so a user cannot type `B240` to jump
  there, and cannot select a named range by name. Named ranges exist in the engine
  (`features/ambre-spec-trame-named-ranges.md`) with no way to reach one from here.
* **The pointer maps to one line.** `editor_local_point` returns a local `y` of zero, so a click in
  a multi-line cell resolves against the first line. `Alt+Enter` and the up and down caret moves are
  built, which is what makes the gap visible.
* **The gate is decided and unbuilt** (`TRAME-FEAT-020..024`). The commands, the events and the
  toasts exist; the handlers return an empty outcome, so a validation never blocks a value and a
  protection never blocks a user. No container is written and no surface offers to create either.
* **The gate covers one path, and the document has four.** The design above tests a cell write.
  Paste, autofill, replace-all and a structural delete reach the same cells and pass no gate, and no
  rule states whether they must.
* **The session has no headless test.** Six rows above say `none` for the same reason. `TrameView`
  is reachable only with a canvas and Leptos signals, while `TrameUiState` under it is driven by
  tests today -- so the seam exists, and the session is on the wrong side of it.
