# AMBRE -- FEATURE: CALLS FROM A CHANNEL
**Type:** Vertical (feature)
**Implementation:** design-only

> A member starts an audio and video call from a channel in one click. The members of the channel
> join it, and the call is encrypted end to end.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-407` | `INV` | A call started from a channel MUST admit the members of that channel alone. | this spec section 3; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-019`) | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Start a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-408` | `POST` | Starting a call MUST post a call message in the channel that the members can join from. | this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-409` | `POST` | The call message MUST show who is in the call and whether it ended, read from the call, never written into the message. | this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-410` | `FRAME` | Starting a call MUST NOT change the membership of the channel. | this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Join a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-411` | `POST` | Joining from a call message MUST bring the member into that call. | this spec section 3; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-016`) | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-412` | `FAIL` | Given a call that has ended, the call message MUST offer no join. | this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-413` | `FRAME` | Joining a call MUST NOT move a read cursor. | this spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none here: the participant cap, the latency and the load of a call are the call rows
  of `ambre-spec-budgets.md section 1`.
* **Boundary.** the call message is sealed like any message (`ZK-001`). The audio and video are
  sealed under sender keys of the call (`CALL-001`), and `ZK-021` and `ZK-022` bound what the SFU
  and the relay learn.
* **Verification.** 7 `unimplemented`: no call exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| starts a call | clicks join | a call message | both in the call, and the message says who is in it (`MURMURE-409`) | none |
| starts a call | is not a member | nothing Bob can read | nothing (`MURMURE-407`) | none |
| leaves the call | stays | the call message | the message drops Alice from the list, and Bob goes on alone | none |

---

## 3. MODEL

**CRDT containers.** the `call` of a message, in `ambre-spec-murmure-schema.md section 3`: the
identifier of the call.

**Murmure starts the call; the call media carries it.** A call message names the call
(`MURMURE-408`). Its live state comes from the call, since no member may write the message of
another (`MURMURE-409`). `1-Foundations/ambre-spec-call-media.md` owns the audio, the video and their
keys.

**The channel admits; the call keys the media.** Each participant wraps its sender key only to a
member that the membership log of the channel lists (`MURMURE-407`). A sender key never derives from
the channel key, so a member who never joins opens nothing of the call (`CALL-002`).

**A call ends when its last participant leaves.** Its message stays in the channel after it
ends. No channel holds a call that waits empty.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the call button of a channel | a call starts, and a call message appears |
| join, on a call message | the member joins the call |
| leave, in the call | the member leaves, and the message updates its list |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A call message in the channel | a call outside any channel | the members would share a link by hand, and anyone holding it could join |
| A call message in the channel | a voice room that stays open in the channel | a room shows presence at every instant, and it holds a connection to the SFU for each member who waits in it |
| Calls are a feature of Murmure | a separate application for calls | a call has no document of its own, and it starts only from a channel. Its admission is the membership of that channel |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No call exists (`1-Foundations/ambre-spec-call-media.md`).
* **No rule caps the calls of one channel.** Whether a second member can start a call while one is
  live is not ruled.
* **The label on the screen is not chosen.** The term is `call` in the corpus; the word the
  interface shows belongs to `ambre-spec-locale.md`.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How the audio and video of a call are encrypted, signalled and carried | `1-Foundations/ambre-spec-call-media.md` |
| What the relay and the SFU learn of a call | `1-Foundations/ambre-spec-zero-knowledge.md` |
