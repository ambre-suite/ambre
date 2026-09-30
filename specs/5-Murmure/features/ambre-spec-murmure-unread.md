# AMBRE -- FEATURE: MURMURE UNREAD STATE
**Type:** Vertical (feature)
**Implementation:** partial -- the cursors, the activity answer and mark read are built; no
  count and no view

> The sidebar shows where something new is, and how much. The count follows the user across their
> devices, and a reaction or an edit never makes a channel unread.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-113` | `INV` | The unread count of a channel MUST count new messages alone, never reactions and never edits. | this spec section 3 | not built | unbuilt -- lands with the `Unread counts and mark unread` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-114` | `INV` | The read cursor of a user MUST be the same on every device of that user. | this spec section 3; `ambre-spec-server.md` | `ambre_server::routes::read_cursors::tests::a_cursor_moved_on_one_device_is_read_on_every_other` -- two devices of one user read the cursor either moved; structural -- the cursor table is keyed by user and document, and no column names a device | none | verified |

### 1.2 Read a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-115` | `POST` | Showing the latest message of a channel MUST move the read cursor to it. | this spec section 3 | `ambre_logic::channels::tests::marking_a_channel_read_moves_its_live_segments_and_clears_its_mark`; `ambre_sync::updates::tests::a_read_reaches_the_last_update_delivered_and_no_further`; `ambre_murmure::channel_tests::the_final_segments_are_listed_loaded_or_not` -- the command moves the live segments to the last update delivered and the finals to their last number | partial -- no view sends the command when the latest message reaches the screen; lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | partial |
| `MURMURE-116` | `FRAME` | Moving a read cursor MUST NOT move the cursor of another member. | this spec section 3 | `ambre_server::routes::read_cursors::tests::marking_everything_read_moves_the_callers_cursors_alone`; `ambre_server::routes::read_cursors::tests::a_listed_move_touches_the_listed_documents_alone` -- another member's cursors stay where they were | none | verified |

### 1.3 Mark a message unread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-117` | `POST` | Marking a message unread MUST move the cursor to just before it. | this spec section 3 | not built | unbuilt -- lands with the `Unread counts and mark unread` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-154` | `FRAME` | Marking a message unread MUST NOT move the cursor of another channel. | this spec section 3 | not built | unbuilt -- lands with the `Unread counts and mark unread` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Mark everything read

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-118` | `POST` | One command MUST move the cursor of every channel to its last number. | this spec section 3 | `ambre_server::routes::read_cursors::tests::marking_everything_read_moves_the_callers_cursors_alone` -- every segment of every channel reads inactive after the move | none | verified |
| `MURMURE-155` | `FRAME` | Marking everything read MUST NOT move the cursor of another member. | this spec section 3 | `ambre_server::routes::read_cursors::tests::marking_everything_read_moves_the_callers_cursors_alone` -- the other member's activity answer is unchanged | none | verified |

### 1.5 Show activity at start

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-119` | `POST` | The sidebar MUST show the activity answer of the relay before any count is known. | this spec section 3; `SRV-109` | `ambre_logic::channels::tests::an_activity_answer_marks_each_channel_once_and_opens_nothing` -- Logic hands the view one mark per channel | partial -- no sidebar draws the mark; lands with the `The Murmure layout and keyboard` Story (`ambre-roadmap.md` E5) | partial |
| `MURMURE-120` | `POST` | A count MUST replace the activity mark once the summaries or the document give it. | this spec section 3; `RESID-014` | not built | unbuilt -- lands with the `Unread counts and mark unread` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-156` | `FRAME` | Showing activity MUST NOT load a document. | this spec section 3; `RESID-016` | `ambre_logic::channels::tests::an_activity_answer_marks_each_channel_once_and_opens_nothing` -- the answer reaches the view and no open reaches the Sync worker | none | verified |

* **Budget.** the counters cost a light subscription per channel not held
  (`ambre-spec-budgets.md section 6`).
* **Boundary.** the relay holds the read cursors and answers yes or no, never a count
  (`SRV-110`, `ZK-015`). The kind of each update stays sealed.
* **Verification.** 5 `verified`, 2 `partial`, 4 `unimplemented`. the relay rows are held by the
  read cursor route tests against a real database; the client rows by the Logic and Sync unit tests.
  Four rows wait on a sequence number per message, which the count and mark unread both need.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| posts a message | reads another channel | one new message | an unread count of 1 on the channel (`MURMURE-113`) | none |
| reacts to a message | reads another channel | one reaction | no badge moves (`MURMURE-113`) | none |
| reads a channel on her phone | -- | the cursor moved on the relay | the channel read on her laptop too (`MURMURE-114`) | `ambre_server::routes::read_cursors::tests::a_cursor_moved_on_one_device_is_read_on_every_other` |

---

## 3. MODEL

**CRDT containers.** none. The read cursor is a relay record, one per member and document
(`ambre-spec-server.md section 3`). It lives as long as the membership: a removal deletes it
(`SRV-137`).

**Three sources, one badge.**

| Source | Gives | When |
|---|---|---|
| the relay activity answer | a mark: something is new | at start, before anything loads |
| the sealed summaries | a count of messages and mentions | as updates arrive |
| the loaded document | the exact count | when the channel opens |

**Code.**

| Part | Where |
|---|---|
| the cursor table | `crates/ambre-server/migrations/20260930000000_read_cursors.sql` |
| the cursor queries | `crates/ambre-server/src/db/read_cursors.rs` |
| the cursor, activity and mark read routes | `crates/ambre-server/src/routes/read_cursors.rs` |
| the requests and the last number delivered | `crates/ambre-sync/src/channels.rs` |
| the mark read command and the mark per channel | `crates/ambre-logic/src/channels.rs` |

**Mark read.** A live segment still grows. Its cursor moves to the last number the Sync worker
delivered, so a message sequenced and not yet shown stays unread. A final segment no longer changes,
so the relay moves its cursor to its own last number, and the client need not load it.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a channel reaches its latest message on screen | its cursor moves, and its badge clears |
| mark unread on a message | the channel shows unread from that message |
| mark all read | every badge clears |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The client counts | a frame kind the relay reads | one more field in front of the relay, for a count the client computes itself |
| Messages alone count | every frame counts | a reaction would make a channel unread, which users of every chat product refuse |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No count.** The client cannot map a message to its sequence number, above all inside a
  snapshot. Neither the count nor mark unread has the number it needs, which is the
  `Unread counts and mark unread` Story (`ambre-roadmap.md` E5).
* **No view.** No sidebar draws the mark and no message list sends mark read.
