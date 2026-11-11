# AMBRE -- FEATURE: FORUM CHANNELS
**Type:** Vertical (feature)

> A forum is a channel of posts. Each post has a title and tags, and its discussion happens in its own
> thread. A member finds a topic by its tag, on a desktop as on a phone.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-355` | `INV` | A forum MUST be a public or private channel whose layout is forum, and whose messages are posts. | this spec section 3; `ambre-spec-murmure-schema.md section 3` | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-356` | `INV` | A post MUST carry a title. | this spec section 3 | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-357` | `INV` | The tags of a forum MUST be defined in its `meta` document, by its owner alone. | this spec section 3 | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-358` | `OBS` | A forum, its tags and its filter MUST work at the phone width. | this spec section 3; `ambre-spec-adaptive-layout.md section 2` | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Write a post

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-359` | `POST` | Posting in a forum MUST create the post and its thread document. | this spec section 3; `MURMURE-105` | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-360` | `POST` | A post MUST carry the tags its author picked among the tags of the forum. | this spec section 3 | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-361` | `FAIL` | Given a post with no title, the composer MUST refuse to send it. | this spec section 3; `MURMURE-356` | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-362` | `FRAME` | Posting MUST NOT change the tags of the forum. | this spec section 3 | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Browse a forum

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-363` | `POST` | A forum MUST list its posts by last activity, and filter them by tag. | this spec section 3 | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-416` | `FAIL` | Given a post tag the forum no longer defines, the forum MUST leave it out of the post and of the filter. | this spec section 3; `MURMURE-357` | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-364` | `FRAME` | Filtering a forum MUST NOT load a thread document. | this spec section 3; `MURMURE-112` | not built | unbuilt -- lands with the `Forum channels` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a post is a message, and its thread is a thread document.
* **Boundary.** the title, the tags and the posts are sealed like any message (`ZK-001`). The relay
  learns that a channel is a forum no more than it learns any channel type: the type is in `meta`.
* **Verification.** 11 `unimplemented`: no forum exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The forum holds | Each user sees | Test |
|---|---|---|---|---|
| posts "Release 2.4" with the tag "release" | -- | one post, and its thread | the post at the top of the forum (`MURMURE-359`) | none |
| -- | replies in that post | the reply, in the thread of the post | the post rises to the top (`MURMURE-363`) | none |
| filters by "release" on her phone | -- | -- | the posts with that tag alone (`MURMURE-358`) | none |

---

## 3. MODEL

**CRDT containers.** of `ambre-spec-murmure-schema.md section 3`: the `tags` map of `meta`, and
the `title` and `tags` of a message. `ChannelLayout::Forum` makes a channel a forum,
whatever its type: a forum is public or private like any channel.

```rust
pub struct TagId(Uuid);   // minted by the owner's client
```

**A post opens its thread at once** (`MURMURE-359`). Every reply goes to the thread, so the forum
stays one line per post, under the one thread model of `features/ambre-spec-murmure-threads.md`.

**Tags belong to the forum.** Its owner defines them in `meta` (`MURMURE-357`). An author picks among
them (`MURMURE-360`). A post keeps its tags in a map keyed by tag, so two edits of one tag never
race for a slot.

**Last activity orders the list.** The activity signals of the threads move a post up
(`MURMURE-363`), with no thread loaded.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| new post, in a forum | a title, the tags of the forum, and the body |
| a tag, in the filter bar | the posts with that tag |
| a post | its thread opens |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A post opens its thread at once | a thread on the first reply, as in a channel | a forum is read post by post, and a reply in the stream would break that |
| Tags defined by the owner | free tags on each post | a forum of a hundred spellings of one topic cannot be filtered |
| One thread model | a reply model of its own for forums | users of Teams do not know which model to use when there are two |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No forum exists.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a thread holds its replies | `features/ambre-spec-murmure-threads.md` |
| How a channel is created, and its type | `features/ambre-spec-murmure-channels.md` |
