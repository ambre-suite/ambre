# AMBRE -- FEATURE: MURMURE MODERATION
**Type:** Vertical (feature)
**Implementation:** design-only

> A moderator removes the message of another member. Every client checks the standing of the
> moderator against the signed membership log. The relay refuses a member whose standing it revoked.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-305` | `INV` | A moderator MUST be the owner of the channel, or an admin of its space. | this spec section 3; `ambre-spec-membership.md` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-309` | `OBS` | A message a moderator removed MUST show as removed by a moderator, with no text. | this spec section 3; `MURMURE-185` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Remove the message of another member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-306` | `POST` | Removing a message MUST delete it with `by: moderator`, in the message or in an amendment of its locked segment. | this spec section 3; `MURMURE-088` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-312` | `POST` | A removal MUST cite the membership entry that gives its signer the standing. | this spec section 3; `SYNC-102` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-310` | `POST` | Removing a message MUST erase every file it carries: its attachments and the image of its preview. | this spec section 3; `SRV-158` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-311` | `FRAME` | Removing a message MUST NOT change its author, its thread or its reactions. | this spec section 3 | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Check a removal

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-307` | `POST` | Every client MUST accept a removal only when the entry it cites gave its signer the standing. | this spec section 3; `SYNC-103` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-308` | `FAIL` | Given a removal from a member whose standing the relay revoked, the relay MUST refuse it. | this spec section 3; `SRV-157` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-299` | `FAIL` | Given a removal that cites an entry later than the removal, or none, every client MUST refuse the delta. | this spec section 3; `MURMURE-079` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-300` | `FRAME` | Checking a removal MUST NOT load a document beyond the membership log. | this spec section 3 | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a removal is one delta, checked against the log the client holds.
* **Boundary.** a removal is sealed like any delta (`ZK-001`). The relay learns who holds the
  standing, which is the membership metadata `ambre-spec-zero-knowledge.md` accepts. It never
  learns which message was removed.
* **Verification.** 10 `unimplemented`: no moderation exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice, a space admin, does | Mallory does | The channel holds | Every member sees | Test |
|---|---|---|---|---|
| removes an abusive message of Mallory | -- | the message, deleted `by: moderator` | "removed by a moderator" (`MURMURE-309`) | none |
| -- | removes a message of Alice, with no standing | the delta refused | Alice's message, unchanged (`MURMURE-307`) | none |
| removes a message just as the owner revokes her admin role | -- | whichever the relay ordered first | one answer on every client (`MURMURE-308`) | none |

---

## 3. MODEL

**CRDT containers.** the message map and the amendments of `ambre-spec-murmure-schema.md section 3`:
a removal sets `deleted`, empties `content`, and records `by: moderator` (`MURMURE-306`).

**No new role.** A moderator is the owner of the channel, or an admin of its space (`MURMURE-305`).
Both standings already live in the signed membership log, which only grows. So a client can check a
removal against the role its signer held at the time.

**A removal, checked in two places.**

1. The relay refuses a removal from a member whose standing it has revoked (`MURMURE-308`,
   `SRV-157`). The relay orders every delta, so a revoke and a removal at once have one answer.
2. Each client reads the entry the removal cites (`MURMURE-312`). It accepts the removal only when
   that entry gave the signer the standing (`MURMURE-307`). Every client reads the same log, so
   every client agrees (`MURMURE-079`).

A hide is within the power of the relay already, since the relay can withhold any wire message. So letting
it check the standing adds no power it did not hold.

**The authorship check makes room for it.** The check of `features/ambre-spec-murmure-messages.md`
refuses a change to the message of another author. A removal is the one change it accepts from
someone else, and only with a standing the log proves.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| remove, on the message of another member, for a moderator | the message shows as removed by a moderator |
| the same action, for a member without the standing | not offered |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| No new role | a moderator role of its own | a fourth role to grant, clamp and show, for a power the owner and the space admins already hold |
| The relay refuses a revoked standing | the clients alone check | a revoke concurrent with a removal has no single answer without the order the relay gives |
| Roles in the signed log | roles in a last-writer-wins map | a compaction would drop every past role, and a client could no longer check a removal against the role its signer held |
| A moderator removes | a moderator edits | an edit under another name would put words in a member's mouth |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No moderation exists.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Who holds which role, and how a role changes | `ambre-spec-membership.md` |
| How a delete empties a message and writes an amendment | `features/ambre-spec-murmure-messages.md` |
| How a file is erased | `ambre-spec-object-store.md` |
