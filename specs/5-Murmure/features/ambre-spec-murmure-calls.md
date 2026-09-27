# AMBRE -- FEATURE: CALLS FROM A CHANNEL
**Type:** Vertical (feature)
**Implementation:** design-only

> A member starts a video call from a channel in one click. The members of the channel join it, and
> the call itself is Rendez-Vous, encrypted end to end.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-407` | `INV` | A call started from a channel MUST admit the members of that channel alone. | this spec section 3; `Todo/ambre-rendez-vous.md` | not built | unbuilt -- lands with the `Rendez-Vous` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Start a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-408` | `POST` | Starting a call MUST post a call message in the channel that the members can join from. | this spec section 3 | not built | unbuilt -- lands with the `Rendez-Vous` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-409` | `POST` | The call message MUST show who is in the call and whether it ended, read from the meeting, never written into the message. | this spec section 3 | not built | unbuilt -- lands with the `Rendez-Vous` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-410` | `FRAME` | Starting a call MUST NOT change the membership of the channel. | this spec section 3 | not built | unbuilt -- lands with the `Rendez-Vous` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.3 Join a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-411` | `POST` | Joining from a call message MUST open that meeting in Rendez-Vous. | this spec section 3 | not built | unbuilt -- lands with the `Rendez-Vous` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-412` | `FAIL` | Given a call that has ended, the call message MUST offer no join. | this spec section 3 | not built | unbuilt -- lands with the `Rendez-Vous` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-413` | `FRAME` | Joining a call MUST NOT move a read cursor. | this spec section 3 | not built | unbuilt -- lands with the `Rendez-Vous` Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** none here: the media of a call is the budget of Rendez-Vous.
* **Boundary.** the call message is sealed like any message (`ZK-001`). What the call itself
  reveals is the boundary of Rendez-Vous (`Todo/ambre-rendez-vous.md`).
* **Verification.** 7 `unimplemented`: no call exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| starts a call | clicks join | a call message | both in the call, and the message says who is in it (`MURMURE-409`) | none |
| starts a call | is not a member | nothing Bob can read | nothing (`MURMURE-407`) | none |

---

## 3. MODEL

**CRDT containers.** the `call` of a message, in `ambre-spec-murmure-schema.md section 3`: the
identifier of the Rendez-Vous meeting.

**Murmure starts the call, Rendez-Vous runs it.** A call message names the meeting (`MURMURE-408`).
Its live state comes from the meeting, since no member may write the message of another
(`MURMURE-409`). The meeting key derives from the channel key, so only members join
(`MURMURE-407`).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the call button of a channel | a call starts, and a call message appears |
| join, on a call message | the member joins the call |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A call message in the channel | a call outside any channel | the members would share a link by hand, and anyone holding it could join |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No call exists, and Rendez-Vous is a sketch (`Todo/ambre-rendez-vous.md`).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a call is encrypted, signalled and carried | `Todo/ambre-rendez-vous.md` |
