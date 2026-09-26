# AMBRE -- APP: MURMURE (BEYOND THE MVP)

**Type:** Sketch (the part of an application that has no spec yet)
**Implementation:** design-only

> A sketch, and not a specification: what Murmure becomes after its MVP. It declares no contract
> row, so it binds nothing and no rule is cited from it. The MVP is specified in `specs/5-Murmure/`,
> and this sketch no longer restates it.
>
> What users like and dislike in Teams, Slack and Discord is in
> `ambre-murmure-user-research.md`, with a do and do-not table for Murmure. This sketch applies
> that table; the rows it rejects are in section 6.

## 1. VISION

Murmure is the discussion center of the suite. Every message is an encrypted CRDT mutation, so the
relay carries it and never reads it. The messaging view stays minimal, and the other applications
stay one command away.

**What the specs already rule.**

| Topic | Spec |
| :--- | :--- |
| The documents of a channel, their containers and their kinds | `5-Murmure/ambre-spec-murmure-schema.md` |
| The engine, and a channel as a set of documents | `5-Murmure/features/ambre-spec-murmure-engine.md` |
| Spaces, the catalogue and the self-join | `5-Murmure/features/ambre-spec-murmure-spaces.md` |
| Private channels, direct messages, invites and leaves | `5-Murmure/features/ambre-spec-murmure-channels.md` |
| Segments, the finalization and its lease | `5-Murmure/features/ambre-spec-murmure-segments.md` |
| Messages, edits, deletes and amendments | `5-Murmure/features/ambre-spec-murmure-messages.md` |
| Reactions, mentions, threads and unread | the feature spec of each |
| Typing, presence, attachments, emoji | the feature spec of each |
| Notifications in an open tab, and search in loaded segments | the feature spec of each |
| Layout, keyboard, message list and composer | the feature spec of each |
| Signed membership, residency, authorship | `1-Foundations/` |

---

## 2. BEYOND THE MVP, STORY BY STORY

Each heading names its Story in `ambre-roadmap.md` E5, unless it says otherwise.

### Push notifications

A notification reaches a user with every tab closed. Only the existence of a message is pushed;
the client decrypts the content on receipt.

* **Levels.** Per channel: all, mentions, none. A large channel defaults to mentions.
* **Every push shows a notification.** Web Push requires `userVisibleOnly: true`, and Safari
  cancels a subscription after three silent pushes.
* **The relay filters.** It stores the level per member and channel, as Matrix MSC4028 does for
  muted rooms. It sends no push for a muted channel.
* **Mentions are routed.** For the mentions level, the sender client lists the public keys of the
  mentioned members in the routing header. The relay pushes to those members only.
* **The level has two copies.** The user settings are the source of truth for the interface. The
  client writes a copy to the relay, which decides which push to send.
* **Cost.** The relay learns the levels, and who mentions whom. The zero-knowledge boundary table
  records both before this ships.

### Personal state: mute, hide, the personal sidebar, followed threads, presence settings

Murmure keeps no store of its own. It writes one `"murmure"` section in the sealed per-user store
of the `User settings` Story (`ambre-roadmap.md` E6). That Story decides the key, the layout of the
document, and how a new device finds the store.

- `"layout"` (Map) -- `{channel_id: {group, position, hidden, collapsed}}`
- `"notify"` (Map) -- `{channel_id: "all" | "mentions" | "none"}`
- `"bookmarks"` (Map) -- `{message_id: {channel_id, segment_id, added}}`
- `"followed"` (Map) -- `{root_message_id: {channel_id, since}}`
- `"presence"` (Map) -- whether the typing and presence indicators are on

Each key is a channel or a message, so a last-writer-wins map fits every entry. The store merges
the edits of several devices. The Rust shapes use the newtypes of the schema: `ChannelId`,
`MessageId`, `Timestamp`.

**Leave and hide are two acts.** Leave adds a signed entry to the membership log, and the relay
removes the row. Hide sets `hidden`: the user keeps the key and can still read. A user who hides a
channel and believes they left still receives its traffic, so the interface says which one applies.

**A bookmark is personal, and a pin is shared.** A pin lives in the `meta` document of the
channel. A bookmark lives in the user settings.

**Followed threads.** One "Threads" view lists every followed thread with unread replies. A thread
never archives on a fixed delay. The user can mark a whole thread as read.

### Moderation

A moderator hides the message of another member. A hide is within the power of the relay already:
the relay can withhold any frame. So the relay may check that the role still holds, with no new
power.

1. The relay refuses a moderation from a member whose role it has revoked. The relay orders every
   frame, so the answer is unique.
2. Each client checks that the signer held the moderator role in the signed membership log, at the
   entry the operation cites. Every client reads the same log, so every client agrees.

The MVP keeps that path open. The roles live in the signed membership log, which only grows. An
amendment records whether the author or a moderator deleted (`DeletedBy`).

### Search on a new device

A new device holds only the active segment, and the relay cannot search.

**An index per final segment.**

* The client that finalizes a segment builds its inverted index.
* It seals the index under the channel key, and uploads it beside the snapshot of the segment.
* A final segment is immutable, so its index is too. The index needs no incremental update.
* The relay sees one more sealed blob, with a size close to the snapshot it already sees.

**A query on a new device.**

1. The client downloads the segment indexes. An index is much smaller than its snapshot.
2. The client queries the indexes.
3. The client fetches and decrypts only the segments with a match.

The result list shows its coverage, for example "searched 2 of 14 segments", and offers to fetch
the rest. A partial result is never silent.

**Integrity.** A malicious member can publish a false index.

* The finalizing client signs the index.
* The client checks each match against the decrypted segment. That check removes every false
  positive.
* A false negative stays possible. A client can rebuild an index in the background and compare.

**Amended segments.** The index of a final segment stays immutable. The client drops the matches
the amendments delete, and queries a small index of the amendments document built on the fly. A
re-finalization replaces the index.

**Storage and stack.** The index lives in the Origin Private File System (OPFS), inside a worker,
and not in memory. Synchronous OPFS access exists only in a worker. The stack is pure Rust:
`unicode-segmentation` with diacritic folding for French, `fst` for the terms, `roaring` for the
posting lists. tantivy is richer, but its WASM support is fragile, and it needs a prototype first.

**Filters.** Author, date, channel, file type, and "replies to me", including replies that did not
mention the user.

### Space retention

**The default is the full history.** Murmure keeps every segment as long as the space exists.

**Retention is a space setting.** A space admin sets one duration N for the whole space. No channel
overrides it.

**The relay applies retention.** A client-side policy would never erase while no admin is online,
so the erasure promise of the General Data Protection Regulation (GDPR) would be false.

- The relay holds N in plaintext. N is outside the `ZK-007` set today, like the public flag.
- The relay deletes a final segment, its index and its amendments once its finalization is older
  than N. The relay knows the finalization time without a key.
- A client removes the missing segment from the `segments` list of `meta` when it finds the gap.

**The guarantee is a range.** A message stays at least N, and at most N plus the time span of its
segment. The interface states that range, never N alone.

### Direct message conversion

A member converts a direct message into a private channel of a space, in one act:

1. The client sets the type to private and writes a name in `meta`.
2. The relay records the space of the channel.
3. The channel identifier and the segments stay.
4. The converting member chooses the history of later members: keep it, or start fresh.

A direct message converts to a private channel only. A private channel becomes public through a
second act by a space admin, who writes its key into the catalogue.

**Keep is the default.** The key stays, and every later member reads the whole history.

**Start fresh records a history boundary.** The conversion rotates the key and records the boundary
in `meta`. A later member receives only the keys after it (`MURMURE-058`). The members of the
direct message keep the older keys.

**The boundary protects against later members only.** A member of the direct message can still
copy the text or pass on an older key.

**Each member is warned.** The warning names the space and the history choice before the
conversion applies.

**The history boundary is general.** Any invite into a private channel can use it, as "invite
without history". The conversion is its first use.

A member of the direct message who is not a member of the space stays a member of the channel.

### Forum channels

Each post has a title and tags, and opens its own thread document. The forum works on a phone,
tags included.

### Custom emoji

A space admin adds emoji for the members of that space. The images go through the shared media
flow.

### Chat import

An archive of Slack or Discord, in JSON, imported into a space.

### Later than V1

* **Unfurling.** A preview of a link or of an internal document (Trame, Toile, Prose) in the
  stream.
* **Calls.** A conference started from a channel in one click, through Rendez-Vous
  (`Todo/ambre-rendez-vous.md`).
* **Bots and webhooks.** Section 4. `ambre-roadmap.md` section 8 leaves them unscheduled.

---

## 3. NOT YET IN A SPEC

Nothing of the earlier sketch is left without a row. The compaction of an active segment is
`MURMURE-222`, and a segment that cannot be opened is `MURMURE-218` to `MURMURE-221`. The keys an
invitee needs come through the key chain of `ambre-spec-key-rotation.md` (`SYNC-115` to
`SYNC-122`). The authorship check reads each delta as a JSON update (`MURMURE-223`).

---

## 4. EXTENSIBILITY & BOTS (E2EE AUTOMATION)

> Autonomous agents operating inside the encrypted perimeter.

### A. Identity & Security (Machine-to-Machine)
* **PKI Native:** Each bot holds its own Ed25519 key pair. To read a channel, the bot must be
  invited by a member, who wraps the channel key for it.
* **Zero-Knowledge Architecture:** The relay never sees commands sent to bots, nor their
  responses. All bot-client traffic is end-to-end encrypted.

### B. Bot Features
* **Interactive Messages:** Messages with buttons, menus and forms, drawn by the Murmure canvas.
* **Cross-App Triggers:**
  - *Trame -> Murmure:* A bot monitors a budget cell and posts an alert past a threshold.
  - *Murmure -> Toile:* A `/brainstorm` command creates a new board with a mind-map template.
* **Slash Commands:** Registered commands (for example `/jira create`) with client-side
  completion.
* **WASM Applets (Sandboxing):** Data cleanup or formatting scripts run in the Logic thread without
  leaving the security scope.

### C. Integration Modes
* **Native (Rust/TS SDK):** A full client on a third-party server, connecting to the relay.
* **Inbound Webhooks (Bridge):** An encrypting bridge receives standard JSON (GitHub, Stripe) and
  turns it into a signed Loro mutation before sending it to the relay.
* **Ephemeral Bots:** Scripts a user launches on demand for a one-off task on the local document.

### D. Bot UX
* **Trust Indicator:** A badge certifying that the bot uses an organization-verified key.
* **Audit Logs:** Every bot action is a signed mutation in the CRDT, so any change is traceable.

---

## 5. RELATED SKETCHES

| Question | Where |
| :--- | :--- |
| What users like and dislike in Teams, Slack and Discord | `Todo/ambre-murmure-user-research.md` |
| Video calls | `Todo/ambre-rendez-vous.md` |

---

## 6. REJECTED ALTERNATIVES

The alternatives a spec of `specs/5-Murmure/` rejects are in that spec. This table keeps the rest.

### Technical

| Alternative | Rejected because |
| :--- | :--- |
| **Matrix/XMPP protocols** | Native integration into the Ambre sync pipeline keeps one model for every application of the suite. |
| **Searchable encryption on the relay** (keyed-hash token per word, SSE) | The relay sees which queries repeat and which documents match. Published leakage-abuse attacks (Cash et al., 2015) recover content from that leak. |
| **Full download and re-index on each device** | The Element model: Seshat (Rust, tantivy) indexes on desktop only, and Element Web has no search in encrypted rooms. A new device waits for the full history. |
| **SQLite/FTS5 compiled to WASM** | A C library, against the Rust-first pillar. |
| **Retention applied by a client** | No erasure happens while no admin is online. |
| **A retention setting per channel** | Two settings for one space, and a member cannot tell which one holds. |
| **A direct message converted straight into a public channel** | The whole space would read a history written for a few members. The public step is a second act, by a space admin. |
| **A history duration chosen at conversion** | Keys change at a rotation, not on a date. A cut inside a segment would split it under two keys, and a cut at a segment edge would show a false duration. Retention already bounds the history. |
| **Roles in a last-writer-wins map** | Compaction would drop every past role. A client could no longer check a moderation against the role its author held. |
| **Moderation in the MVP** | A role lives in the membership log and a message in a segment. A revoke concurrent with a delete has no single answer without the relay check. |
| **Bookmarks and followed threads in local storage** | A new device starts empty, and a new device is where the user needs them most. |
| **A Murmure document of its own for personal state** | The `User settings` Story (`ambre-roadmap.md` E6) already plans one sealed store per user. A second store gives the same user two sources of truth. |
| **Client-side filtering of each push after decryption** | Web Push requires a visible notification for each push. Safari cancels the subscription after three silent pushes. |

### Product

From `ambre-murmure-user-research.md`. Several rows hold for the MVP too; the specs apply them.

| Alternative | Rejected because |
| :--- | :--- |
| **Several message models** (flat chat, threaded posts) | Teams users do not know which one to use. Teams added a third model, a threads layout, in August 2025. |
| **A new post hides older threads** | Teams users lose threads they did not close. |
| **Auto-archive of a thread on a fixed delay** | Discord users find the delay too short. |
| **Unread cleared only when the user opens the conversation** | Users of all three products want to control their read state. |
| **A permanent navigation bar that the user cannot hide** | The 2023 Slack redesign caused a backlash from users of basic messaging. |
| **History deletion to sell a plan** | Slack deleted free history older than one year from 2024-08-26. Users read it as a hostage. |
| **Any server-side processing of content** (model training, analysis) | The 2024 Slack backlash over training on messages. It also breaks the zero-knowledge pillar. |
| **Identity documents to use the product** | The 2026 Discord backlash over age verification by ID or face scan. |
