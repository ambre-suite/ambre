# AMBRE -- APP: MURMURE (COLLABORATIVE MESSAGING)

**Type:** Sketch (an application that has not started)
**Implementation:** design-only

> A sketch, and not a specification: what Murmure would be, written before a crate exists. It
> declares no contract row, so it binds nothing and no rule is cited from it. Real-time
> communication, channels, and E2EE persistence.
>
> What users like and dislike in Teams, Slack and Discord is in
> `ambre-murmure-user-research.md`, with a do and do-not table for Murmure. This sketch applies
> that table; the rows it rejects are in section 8.

## 1. VISION & FLOW
Murmure is the command and discussion center. Unlike classic SaaS tools, every message is an encrypted CRDT mutation, ensuring that even the "Blind Relay" server cannot read the content.
- **Engine:** Logic thread (Flow management) + wgpu + Parley/Swash (High-performance rendering).
- **Performance:** Local indexing enabling instant search across millions of messages.

---

## 2. CORE FEATURES

### A. Spaces & Channels (Discord/Slack Structure)
* **Spaces (Workspaces):** Logical team groupings with PKI permission management.
* **Channels (Public/Private):** Thematic streams organized in folders.
* **Threads:** One model everywhere: a thread is a reply chain on any message, in a channel or a direct message.
  * The root message shows the reply count and the last replier. A new post never hides an older thread.
  * "Also send to channel" copies a reply into the main stream, so a channel reader misses no decision.
  * One "Threads" view lists every followed thread with unread replies. A thread never archives on a fixed delay.
  * The user can mark a whole thread as read.
* **Forum channels:** Each post has a title and tags and opens its own thread. The forum works on mobile, tags included.
* **Direct Messages (DM):** 1:1 channels or restricted groups with forward secrecy.

### B. Chat Experience
* **Message Editor:** Simplified version of the **Prose** engine (Markdown, emojis, `@` mentions, code blocks).
* **Reactions:** Emoji additions on messages with CRDT aggregation (no conflict on reaction counts). A reaction is a light reply: custom emoji per space, and a reaction never counts in the unread badge.
* **Pins:** Shared with the channel, through the `pinned` array of the `meta` document.
* **Bookmarks:** Personal, and a different feature from a pin. The client stores a bookmark in the user settings (see *Channel list and personal state*).
* **Status & Presence:** Real-time indicators (Online, Busy, Typing...) via WebSocket relay.
  * Each indicator tells the relay when a member is active, and that timing is metadata.
  * Each indicator has a user setting. The default is an open decision.
* **Read state:**
  * The read cursor syncs across devices (see *Unread tracking*).
  * The user can mark any message as unread.
  * One key marks everything as read.
* **Keyboard first:** quick switcher, jump to the first unread, edit the last message. No frequent action requires the mouse.

### D. Interface principles
* The messaging view stays minimal. The other applications stay one command away.
* No permanent navigation bar that the user cannot hide.
* Start time and memory hold the budgets of `ambre-spec-budgets.md`. The client never loads a full channel history at start.

### C. Sharing & Integration
* **File Transfer:** Drag & drop of assets (images, docs). Files are encrypted and stored on MinIO; Murmure only stores the reference.
* **Unfurling:** Secure preview of links and internal documents (Trame, Toile, Prose) directly in the stream.
* **Calls & Video:** Native integration of the **Vue** engine to launch a conference from a channel in one click.

---

## 3. TECHNICAL ARCHITECTURE

### Time-segment architecture

An active channel accumulates ~200 messages/day with 20 participants. A single `Loro::Doc` per channel would become unmanageable (50 MB+ in 2 years with Loro's tombstone overhead). The solution: **one channel = a chain of segments**, each segment being an independent `Loro::Doc`.

Channel storage layout (`channel_{id}/`):

- `meta` -- lightweight Loro::Doc (name, topic, members, pinned)
- `segments/`:
  - `seg_001` -- messages 1-5000 (sealed, read-only)
  - `seg_002` -- messages 5001-10000 (sealed, read-only)
  - `seg_003` -- messages 10001-... (active segment, writable)

**The client only loads the active segment + metadata** (~2 MB max on first display). Older segments are lazy-loaded when scrolling up.

### CRDT Structure -- Doc meta (per channel)

`Loro::Doc (meta)`:

- `"channel"` (Map) -- `{name, type, created, topic}`
- `"members"` (Map) -- `{"pubkey_alice": {role, joined}}`
- `"pinned"` (Array) -- `[{msg_id, segment_id}]`
- `"segments"` (Array), for example:
  - `{id: "seg_001", from_ts: 1710000000, to_ts: 1720000000, msg_count: 5000, sealed: true}`
  - `{id: "seg_002", from_ts: 1720000000, to_ts: 1730000000, msg_count: 5000, sealed: true}`
  - `{id: "seg_003", from_ts: 1730000000, to_ts: null, msg_count: 1847, sealed: false}`

### CRDT Structure -- Segment (one Loro::Doc per segment)

`Loro::Doc (segment)`:

- `"messages"` (Array) -- chronological order; each entry a Map:
  - `id`
  - `author_pubkey`
  - `timestamp`
  - `content` -- Loro::Text (editable, rich formatting)
  - `reply_to` -- parent message id (threads)
  - `reactions` -- Map `{"thumbs-up": ["pubkey_a", "pubkey_b"]}`
  - `edited` -- bool
  - `deleted` -- tombstone (soft delete)
- `"media"` (Map) -- same system as documents

`content` as `Loro::Text` enables message editing and rich formatting (bold, links, inline code, mentions).

### Thread tracking

A reply can land in a later segment than its root. So the reply count and the last replier cannot live in the root segment alone.

* Proposal: a per-channel `threads` document maps each root `msg_id` to `{reply_count, last_reply_ts, last_author}`.
* The "followed" state is personal. The client stores it with the bookmarks, in the user settings.
* Open question: the size of the `threads` document in a channel with years of history. It may need the same segment chain.

### Object Storage

See `ambre-spec-object-store.md` for the layout that exists. The channel branches below exist in no
code and in no object name today. Summary of messaging keys:

Object storage paths:

- `channels/{channel_id}/meta/snapshots/{snapshot_id}`
- `channels/{channel_id}/meta/deltas/{seq:010}`
- `channels/{channel_id}/segments/{seg_id}/snapshot` -- sealed = final snapshot only
- `channels/{channel_id}/segments/{seg_id}/deltas/{seq:010}` -- active segment only
- `channels/{channel_id}/media/{content_hash}`

### Segment sealing

When the active segment exceeds ~5000 messages (or ~2 MB snapshot):

1. Client acquires Redis lock (`channel:{id}:seal`, SET NX, TTL 30 s)
2. Encodes the final segment snapshot, encrypts it, PUT to Object Storage
3. Updates Doc meta (marks `sealed: true`, creates new segment)
4. Old segment deltas are purged (snapshot is sufficient)

Sealed segments are **immutable** -- no more deltas, just a final snapshot. This simplifies storage and sync.

### Messaging compaction

More aggressive than for documents: compaction every **1-2 min** (vs 5 min for docs) because an active channel generates far more updates.

### Unread tracking

The only real server-side addition. The server tracks a "last read seq" cursor per user per channel -- unencrypted metadata (Postgres, `channel_read_cursors`, `ambre-spec-server.md section 3`).

### Typing indicators

Native use case for SecSync **Ephemeral messages** -- never persisted, anti-replay counter.

### Server-side authorization (Blind Relay enforcement)

> **MANDATORY when Murmure lands.** The current WS handler (`ambre-server`) enforces
> real access control for documents but NOT for channels: `handle_subscribe_channel`
> accepts any `channel_id` with no membership check, and `handle_channel_update` /
> `handle_channel_ephemeral` rebroadcast without verifying the Ed25519 signature.
> Since Murmure is not started, no channel path is wired to real data yet; before it
> is, the relay MUST mirror the document guarantees so E2EE is not the ONLY line of
> defense.

A channel is one document crypto-side (see *Key management*), so it reuses the same
membership model. The relay MUST enforce, server-side:

1. **Subscription ACL** -- `SubscribeChannel` requires a `channel_members` row for the
   caller (same shape as `document_members`), re-checked on a short TTL exactly like
   `MEMBERSHIP_RECHECK_TTL` for documents. Without it any authenticated user who learns
   a `channel_id` can receive every broadcast (Zero-Knowledge leak: sender identity,
   timing, volume -- even though the ciphertext stays sealed) and inject traffic.
2. **Signature verification** -- every `ChannelUpdate` MUST pass `verify_update_signature`
   before it is sequenced/persisted/broadcast, identical to `handle_update`. Ephemerals
   (typing/presence) carry the anti-replay counter and are signature-checked too.
3. **Clock + seq** -- persisted channel updates go through the same
   `verify_clock_and_assign_seq` Lua path (per active segment) as document deltas.

Rationale: the Blind Relay's promise is "the server only knows routing metadata for
*authorized* members." An unauthenticated-to-the-channel subscriber breaks that promise
at the metadata layer, and unsigned rebroadcast lets a non-member pollute members'
sessions (integrity / DoS). E2EE protects content, not membership metadata or integrity.

### Key management

One channel = one document from a crypto standpoint, with its Document Key. Sharing = encrypting the Document Key with each member's X25519 pubkey. When a member leaves: rotation via the flow described in section 6.

### Channel list and personal state

Four stores hold what the channel sidebar shows. Each fact has one owner.

| Fact | Owner | Visible to the relay |
| :--- | :--- | :--- |
| The channels a user can read | the relay, through `channel_members` | yes, routing metadata |
| The channel name and topic | the channel `meta` document | no, sealed |
| The categories and the channel order of a space | the `space` document | no, sealed |
| The personal layout, mute, bookmarks and followed threads | the user settings | no, sealed |
| The notification level | the user settings, with a copy on the relay | yes, the copy |
| The read cursor | the relay, through `channel_read_cursors` | yes |

**The channel list is membership.** The client asks the relay for its channels. The answer carries
each `channel_id` and its wrapped Document Key. The client then opens each `meta` document and
decrypts the name. The launcher lists documents the same way today (`CHROME-044`). No document holds
a second copy of that list, because such a copy drifts after a revoke.

**The space document holds the shared structure.** It is a sealed Loro document per space. It maps
each `channel_id` to its category and its position. It also lists the public channels that a
member has not joined yet. Every member of the space sees the same content.

**The user settings hold the personal state.** Murmure keeps no store of its own. It writes one
`"murmure"` section in the sealed per-user store of the `User settings` Story (`ambre-roadmap.md`
post-V1). Macro approvals and the personal dictionary of Prose already wait on that store. That
Story decides the key, the document layout and how a new device finds the store.

- `"layout"` (Map) -- `{channel_id: {group, position, hidden, collapsed}}`
- `"notify"` (Map) -- `{channel_id: "all" | "mentions" | "none"}`
- `"bookmarks"` (Map) -- `{msg_id: {channel_id, segment_id, added}}`
- `"followed"` (Map) -- `{root_msg_id: {channel_id, since}}`

Each key is a channel or a message, so a last-writer-wins map fits every entry. The store merges
the edits of several devices.

**Leave and hide are two acts.**

- Leave removes the `channel_members` row. A private channel then rotates its Document Key.
- Hide sets `hidden` in the user settings. The user keeps the key and can still read.

A user who hides a channel and believes they left still receives its traffic.

**The notification level has two copies.** The user settings are the source of truth for the
interface. The client writes a copy of each level to the relay, because the relay decides which
push to send (section 4). The copy and `channel_read_cursors` are outside the `ZK-007` set today.
The zero-knowledge boundary table must record both before either ships.

### New member onboarding

Choice: access to **the entire history** (Document Key is shared). Consistent with a collaborative office suite (unlike Signal which limits to member join time).

* Each rotation creates a new Document Key. A new member needs every key since the channel start.
* The inviting client sends one signed bundle of all these keys, as Matrix MSC4268 does.
* The Matrix MSC3061 implementation forwarded keys one by one, and it had a vulnerability.

### Decryption failures

A client can hold a message whose key is missing, for example on a new device.

* The message shows the cause and the remedy, for example "key bundle not received, ask a member".
* A bare "unable to decrypt" is forbidden. Element users rank that state as a main complaint.

### Optional retention

With segments, a retention policy = deleting segments older than N months (`DELETE` Object Storage + Doc meta update). Useful for GDPR / right to erasure compliance.

Retention is an explicit admin policy. Without one, Murmure keeps the full history as long as the organization keeps it.

### UI

1D virtualization (virtualized list) + Parley rendering for rich content. Much simpler than the spreadsheet's Grid2D.


---

## 4. NOTIFICATIONS & REAL TIME
* **Push Notifications:** Privacy-respecting notification proxy system (only message existence is notified, content is decrypted by client on receipt).
* **Notification levels:** Per channel: all, mentions, none. A large channel defaults to mentions.
  * Every push MUST show a notification. Web Push requires `userVisibleOnly: true`, and Safari cancels a subscription after three silent pushes.
  * Proposal: the relay stores the level per member and channel, as Matrix MSC4028 does for muted rooms. The relay sends no push for a muted channel.
  * Proposal: for the mentions level, the sender client lists the public keys of the mentioned members in the routing header. The relay pushes to those members only.
  * Cost: the relay learns the levels, and who mentions whom. The zero-knowledge boundary table must record both before this ships.
* **Cold Start Sync:** On startup, Murmure downloads missed deltas since the last disconnection to rebuild the channel state.

---

## 5. LOCAL SEARCH
The relay is blind, so it cannot search. The client builds and queries every index. The hard case is a new device: it holds only the active segment.

### Index per sealed segment
* **Build:** The client that seals a segment also builds the inverted index of that segment.
* **Store:** The client encrypts the index with the Document Key and PUTs it at `channels/{channel_id}/segments/{seg_id}/index`.
* **Immutable:** A sealed segment is immutable, so its index is immutable too. The index needs no incremental update.
* **Active segment:** Each client indexes the active segment locally, message by message.
* **Relay view:** One more encrypted blob, with a size close to the snapshot size that the relay already sees.

### Query on a new device
1. The client downloads the segment indexes. An index is much smaller than its snapshot.
2. The client queries the indexes.
3. The client fetches and decrypts only the segments with a match.

The result list shows the coverage, for example "searched 2 of 14 segments", and offers to fetch the rest. A partial result is never silent.

### Integrity
A malicious member can publish a false index.
* The sealing client signs the index.
* The client checks each match against the decrypted segment content. This check removes every false positive.
* A false negative stays possible. A client can rebuild an index in the background and compare it with the signed one.

### Local storage and stack
* The index lives in OPFS (Origin Private File System), inside a worker, and not in RAM. Synchronous OPFS access exists only in a worker. The client memory budget forbids an index of millions of messages in RAM.
* Pure Rust: `unicode-segmentation` for tokens, with diacritic folding for French. `fst` for the term dictionary, `roaring` for the posting lists.
* tantivy is richer, but its WASM support is fragile. It needs a prototype before any choice.

### Filters
Author, date, channel, file type, and "replies to me", including replies that did not mention the user.

### Open point
The index format assumes that a sealed segment is immutable. The sketch does not say how a user edits or deletes an old message in a sealed segment. Decide that before the index format is frozen.

---

## 6. INTEROPERABILITY
* **Import:** Import of Slack archives (JSON) or Discord.
* **Webhooks:** Encrypted inbound webhook support for external server notifications (CI/CD, Alerts).

---

## 7. EXTENSIBILITY & BOTS (E2EE AUTOMATION)
> Autonomous agent system operating within the encrypted enclave.

### A. Identity & Security (Machine-to-Machine)
* **PKI Native:** Each bot holds its own Ed25519 key pair. To "read" a channel, the bot must be invited by an administrator who shares the Loro document session key.
* **Zero-Knowledge Architecture:** The relay server never sees commands sent to bots, nor their responses. All bot-client traffic is end-to-end encrypted.

### B. Bot Features
* **Interactive Messages:** Sending messages with buttons, dropdown menus, and forms (UI schemas rendered via wgpu + Parley/Swash).
* **Cross-App Triggers:** - *Trame -> Murmure:* A bot monitors a budget cell and posts an alert if the threshold is exceeded.
    - *Murmure -> Toile:* A `/brainstorm` command automatically creates a new Board with a mind-map template.
* **Slash Commands:** Registered command system (for example `/jira create`, `/crypto price`) with client-side autocompletion.
* **WASM Applets (Sandboxing):** Ability to run data cleanup or formatting scripts directly in the Logic thread without leaving the security scope.

### C. Integration Modes
* **Native (Rust/TS SDK):** Full client running on a third-party server, connecting via WebSocket to the Blind Relay.
* **Inbound Webhooks (Bridge):** "Encryptor" micro-service that receives standard JSON (GitHub, Stripe) and transforms it into a signed Loro mutation before sending to the relay.
* **Ephemeral Bots:** Scripts launched on demand by a user to perform a one-off task on the local document (for example "Sort all messages by sentiment").

### D. Bot UX
* **Trust Indicator:** Visual badge certifying the bot uses an organization-verified key.
* **Audit Logs:** Every bot action is a signed mutation in the CRDT, making any modification traceable and reversible (Time-travel).


## 8. REJECTED ALTERNATIVES

### Technical

| Alternative | Rejected because |
| :--- | :--- |
| **Matrix/XMPP protocols** | Native integration into the SecSync/Loro pipeline keeps full consistency with the rest of the Ambre suite. |
| **Searchable encryption on the relay** (keyed-hash token per word, SSE) | The relay sees which queries repeat and which documents match. Published leakage-abuse attacks (Cash et al., 2015) recover content from that leak. It breaks the zero-knowledge pillar. |
| **Full download and re-index on each device** | The Element model: Seshat (Rust, tantivy) indexes locally on desktop only. Element Web still has no search in encrypted rooms. A new device waits for the full history. |
| **SQLite/FTS5 compiled to WASM** | A C library, against the Rust-first pillar. |
| **A sealed index document of the user's channels** | The relay already holds the membership. A second list drifts after a revoke and shows a channel whose key is gone. |
| **Bookmarks and followed threads in local storage** | A new device starts empty, and a new device is where the user needs them most. |
| **A Murmure document of its own for personal state** | The `User settings` Story already plans one sealed store per user. A second store gives the same user two sources of truth. |
| **Client-side filtering of each push after decryption** | Web Push requires a visible notification for each push. Safari cancels the subscription after three silent pushes. |

### Product
From `ambre-murmure-user-research.md`.

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
