# AMBRE -- FEATURE: SPACES AND THE CHANNEL CATALOGUE
**Type:** Vertical (feature)
**Implementation:** design-only

> A team gathers in a space. The space lists its public channels, and any member joins one with no
> one else online. A guest reaches only the channels a member invites them into.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-031` | `INV` | A space MUST be a document, and its members MUST be the members of that document. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-032` | `INV` | A member with the viewer role on the space MUST NOT change the catalogue. | this spec section 3; `SRV-008` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-033` | `INV` | The catalogue MUST hold the key of each public channel it lists, sealed under the key of the space. | this spec section 3; `ambre-spec-murmure-schema.md section 3` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-034` | `INV` | A guest of a channel MUST NOT be a member of the space document. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-236` | `INV` | A removal from a channel MUST count as verified when the log of its space holds a signed removal of the same member. | this spec section 3; `SYNC-106` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-254` | `INV` | A key link of a public channel MUST count as verified when an admin of its space signed it. | this spec section 3; `SYNC-121`, `SRV-146` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Create a space

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-035` | `POST` | Creating a space MUST make its creator the owner of the space document. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-036` | `FRAME` | Creating a space MUST NOT add a member other than its creator. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 List a public channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-037` | `POST` | A listed channel MUST carry its name, its topic, its category, its position and its key. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-038` | `FRAME` | Listing a channel MUST NOT change the membership of any channel. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Join a public channel alone

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-039` | `POST` | A self-join MUST have the Sync worker unseal the key read from the catalogue and wrap it for the joining member alone. | this spec section 3; `SYNC-010` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-040` | `POST` | A self-join MUST add an entry the joining member signs. | this spec section 3; `SYNC-106` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-041` | `FAIL` | Given a self-join entry for a channel the catalogue does not list, every client MUST show the member as unverified. | this spec section 3; `SYNC-108` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-042` | `FRAME` | A self-join MUST NOT write the catalogue. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Leave a space

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-043` | `POST` | A member leaving a space MUST cause the rotation of the space key and of the key of each public channel. | this spec section 3; `ambre-spec-key-rotation.md` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-044` | `POST` | The catalogue MUST hold the rotated key of each public channel once the rotation completes. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-045` | `FRAME` | Leaving a space MUST NOT change a channel of the catalogue beyond its key. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-234` | `POST` | Leaving a space, or being removed from it, MUST remove the member from every channel of the space, private channels included. | this spec section 3; `SRV-136` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-235` | `POST` | Each private channel the member leaves that way MUST owe a rotation. | this spec section 3; `SYNC-127` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-237` | `FRAME` | Leaving a space MUST NOT remove the member from a direct message or from a channel of another space. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-258` | `FAIL` | Given the last owner leaving a space that keeps other members, the client MUST first hand the owner role of the space document to one of them. | this spec section 3; `SRV-015` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 Delete a public channel or a space

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-247` | `POST` | Deleting a public channel MUST remove its entry from the catalogue. | this spec section 3; `MURMURE-243` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-248` | `POST` | Deleting a space MUST delete every channel of the space, then the space document. | this spec section 3; `SRV-143` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-249` | `FAIL` | Given a space delete from a member who does not own the space document, the relay MUST refuse it. | this spec section 3; `SRV-015` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-250` | `FRAME` | Deleting a space MUST NOT delete a direct message. | this spec section 3; `MURMURE-046` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a leave costs one rotation per public channel, bounded by the space leave row of
  `ambre-spec-budgets.md section 1`.
* **Boundary.** the catalogue and every key in it are sealed with the space document (`ZK-001`).
  The relay holds the public flag and the space of each channel (`ZK-015`).
* **Verification.** 25 `unimplemented`: no space exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The space holds | Each user sees | Test |
|---|---|---|---|---|
| lists a public channel | is a member of the space | the channel in the catalogue | Bob sees a channel to join | none |
| is offline | joins the public channel alone | a new member row, signed by Bob | Bob reads the channel at once (`MURMURE-039`) | none |
| -- | the relay flips a private channel to public and adds Mallory | a row the catalogue does not back | Mallory listed as unverified (`MURMURE-041`) | none |
| invites Carol into one channel as a guest | -- | Carol in that channel, not in the space | Carol sees that channel alone (`MURMURE-034`) | none |
| leaves the space | stays | rotated keys in the catalogue | Bob reads on, and Alice reads nothing new (`MURMURE-043`) | none |

---

## 3. MODEL

**CRDT containers.** the `space` document of `ambre-spec-murmure-schema.md section 3`: `space`,
`categories` and `channels`.

**Who writes the catalogue.** A space admin is an owner or an editor of the space document. A
member with the viewer role reads it, and the relay refuses a write from them (`SRV-008`).

**A self-join, in steps.**

1. The Logic thread reads the sealed key of the channel in the catalogue, a `SealedChannelKey`, and
   hands it to the Sync worker. No key material reaches the Logic thread (`MURMURE-157`).
2. The Sync worker unseals it with the space key, wraps it for its own exchange key (`SYNC-010`),
   and signs a join entry (`SYNC-106`).
3. The relay creates the row only for a member of the space and a channel it holds as public
   (`SRV-105`, `SRV-106`).
4. Every client checks the entry against the catalogue, never against the relay flag.
5. The joining client walks the key chain back from the key of the catalogue (`SYNC-119`). It
   reads the whole history of the channel, before a rotation too.

**A leave takes every channel of the space with it** (`MURMURE-234`). The relay removes the member
from each channel of the space, and each private one owes a rotation (`SRV-136`). The removal from
the space, signed in its log, is what verifies each channel removal (`MURMURE-236`). A guest is
never a member of the space, so a guest's channels are not touched.

**The keys, on a leave.** An admin need not be a member of a public channel to re-key it
(`SRV-146`). The link it signs is verified by its role in the space, not in the channel
(`MURMURE-254`). The first admin client online after the leave rotates the space key, then the key of
each public channel. Its Sync worker seals each new key under the new space key, and the catalogue
takes the sealed value.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| create a space | a new space, with its creator as owner |
| the catalogue of a space | the public channels, grouped by category, joined ones marked |
| join on a public channel | the channel opens, and joins the sidebar |
| invite a guest into a channel | the guest reaches that channel alone |
| leave a space | the space leaves the sidebar, and its keys rotate |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A self-join is checked against the catalogue | trusting the public flag of the relay | the relay could flip a private channel to public and show a verified member |
| A guest stays out of the space document | letting a guest read the catalogue | the catalogue holds every public key, so the guest would read every public channel |
| One key per public channel | one key for every public channel of a space | a guest or a bot of one channel would read them all |
| The catalogue lists public channels only | private channels in the catalogue | every member of the space reads the catalogue, so it would show each private channel |
| The catalogue names a public channel | the name in the channel `meta` document | a member who has not joined holds no key for `meta`, so the channel to join would have no name |
| The catalogue carries the sealed key | a member online to wrap the key | a join would wait on another member, and a space leave pays for the catalogue with a rotation |
| A space leave takes its private channels | keeping them, and offering to leave | a member an admin removed from the space would keep reading its private channels |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No space, no catalogue and no self-join exist.
