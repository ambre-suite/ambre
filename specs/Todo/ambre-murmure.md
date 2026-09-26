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
* **Spaces (Workspaces):** Logical team groupings. A space admin manages the catalogue of its public channels.
* **Channels (Public/Private):** Thematic streams, grouped in the categories of the space catalogue.
* **Threads:** One model everywhere: a thread is a reply chain on any message, in a channel or a direct message. Each thread is its own document (see *Threads are documents*).
  * The root message shows the reply count and the last replier. A new post never hides an older thread.
  * "Also send to channel" copies a reply into the main stream, so a channel reader misses no decision.
  * One "Threads" view lists every followed thread with unread replies. A thread never archives on a fixed delay.
  * The user can mark a whole thread as read.
* **Forum channels:** Each post has a title and tags and opens its own thread. The forum works on mobile, tags included.
* **Direct Messages (DM):** A direct message is a private channel with no space, between two or more members. A member may give it a name. It uses the same key model as any channel, and it offers no forward secrecy (see *Direct messages are channels*). A direct message between the same members is reused, never created twice.
* **Conversion:** A direct message becomes a private channel of a space. It keeps its identifier and its segments. The converting member keeps the history for later members or starts them fresh.

### B. Chat Experience
* **Message Editor:** Simplified version of the **Prose** engine (Markdown, emojis, `@` mentions, code blocks). A mention is a mark on the text that carries the user it names (see *Mentions*).
* **Reactions:** Emoji additions on messages with CRDT aggregation (no conflict on reaction counts). A reaction is a light reply, and it never counts in the unread badge. The minimal form offers Unicode emoji. Custom emoji per space come later.
* **Pins:** Shared with the channel, through the `pinned` array of the `meta` document.
* **Bookmarks:** Personal, and a different feature from a pin. The client stores a bookmark in the user settings (see *Channel list and personal state*).
* **Status & Presence:** Real-time indicators (Online, Busy, Typing...) via WebSocket relay.
  * Each indicator tells the relay when a member is active, and that timing is metadata.
  * Both indicators are on by default, and visible to the members of the channel only. The setting that turns them off lives in the user settings, which come after the minimal form.
* **Read state:**
  * The read cursor syncs across devices, and the client computes the unread count (see *Unread tracking*).
  * The user can mark any message as unread.
  * One key marks everything as read.
* **Keyboard first:** quick switcher, jump to the first unread, edit the last message. No frequent action requires the mouse.

### D. Interface principles
* The messaging view stays minimal. The other applications stay one command away.
* No permanent navigation bar that the user cannot hide.
* Start time and memory hold the budgets of `ambre-spec-budgets.md`. The client never loads a full channel history at start.

### C. Sharing & Integration
* **File Transfer:** Drag and drop of files, images and animated GIFs, through the shared media flow (see *Attachments, emoji and animated images*). A message stores the reference only.
* **Unfurling:** Secure preview of links and internal documents (Trame, Toile, Prose) directly in the stream.
* **Calls & Video:** Native integration of the **Vue** engine to launch a conference from a channel in one click.

---

## 3. TECHNICAL ARCHITECTURE

### Time-segment architecture

An active channel accumulates ~200 messages/day with 20 participants. A single `Loro::Doc` per channel would become unmanageable (50 MB+ in 2 years with Loro's tombstone overhead). The solution: **one channel = a chain of segments**, each segment being an independent `Loro::Doc`.

### A channel is a set of documents

Every `Loro::Doc` of a channel is an ordinary Ambre document, with its own `document_id`. It
reuses the whole document pipeline: the sequencer, the `seq` and the clock, the snapshots, the
replay and the access check.

| Document | How many | Holds |
| :--- | :--- | :--- |
| `meta` | one per channel | name, topic, pins, the list of segments |
| segment | one per time slice | the messages of that slice |
| amendments | one per amended sealed segment | the late edits and deletes of that segment |
| thread | one per message with replies | the replies to that message |

**The relay groups them.** It keeps `channel_documents(channel_id, document_id, kind)`. Access to
each document follows from the membership of its channel. Every document of a channel is sealed
under the key of the channel for the current key period.

**Subscribing to a channel** means subscribing to its live documents: `meta`, the active segment
and the closing segment. A thread document is subscribed while it is open.

**No channel plane.** The channel frames of the protocol (`ChannelUpdate`, `ChannelEphemeral`)
disappear. A delta is a document delta, and the typing indicator rides the ephemeral frames of
the `meta` document.

**The relay learns the grouping.** It learns which documents form a channel and the kind of each
one. It already infers most of that from the traffic. The mapping is outside the `ZK-007` set today.

**The client only loads the active segment + metadata** (~2 MB max on first display). Older segments are lazy-loaded when scrolling up.

### CRDT Structure -- Doc meta (per channel)

`Loro::Doc (meta)`:

- `"channel"` (Map) -- `{name, type, created, topic}` -- `type` is `dm`, `private` or `public`; `name` and `topic` for a private channel or a direct message, where `name` is optional for a direct message; the `space` document names a public channel
- `"pinned"` (Array) -- `[{msg_id, segment_id}]`
- `"segments"` (Array), for example:
  - `{id: "seg_001", document_id, from_ts: 1710000000, to_ts: 1720000000, msg_count: 5000, state: "sealed", amended: true}`
  - `{id: "seg_002", document_id, from_ts: 1720000000, to_ts: 1730000000, msg_count: 5000, state: "closing", amended: false}`
  - `{id: "seg_003", document_id, from_ts: 1730000000, to_ts: null, msg_count: 1847, state: "active", amended: false}`

The `meta` document holds no member list. The relay is the authority on membership, and the signed
membership log lets each client check what the relay serves (see *Signed membership log*).

### CRDT Structure -- Segment (one Loro::Doc per segment)

`Loro::Doc (segment)`:

- `"messages"` (Array) -- chronological order; each entry a Map:
  - `id`
  - `author` -- the user who signed the delta that created it
  - `timestamp`
  - `content` -- Loro::Text (editable, rich formatting, mentions as marks)
  - `attachments` -- List `[{media_ref, name, media_type, size}]`
  - `thread` -- the `document_id` of its thread, once the message has a reply
  - `also_in_thread` -- the root `msg_id`, when a reply is also sent to the channel
  - `reactions` -- Map `{emoji: Map {user: timestamp}}`: each member writes under their own key only
  - `edited` -- bool
  - `deleted` -- tombstone (soft delete)

`content` as `Loro::Text` enables message editing and rich formatting (bold, links, inline code, mentions).

### Mentions

A mention is a Loro rich-text mark named `mention` over the displayed name, for example `@Alice`.
The mark value is the user identifier. The client finds the mentions of a message by reading its
marks, never by parsing the text. A later rename of Alice leaves the mark valid, and the display
reads the current name.

### Threads are documents

The replies to a message live in their own document, one per root message. The root message points
to it through its `thread` field.

`Loro::Doc (thread)`:

- `"root"` (Map) -- `{channel_id, root_msg_id}`
- `"replies"` (Array) -- the same entry shape as a segment message, without `thread`

**The root shows the head of its thread.** The reply count and the last replier are read from the
thread document. The client loads the head of a thread only for a root visible on screen.

**A thread that is not open is followed through the channel.** The relay publishes a small signal on
the topic of the channel when a thread document receives a frame. The relay already knows that the
document is a thread of this channel, so the signal teaches it nothing. The client then reloads
the head of that thread if its root is on screen.

**A very long thread becomes a chain of segments**, with the same closing and sealing as a channel.

**"Also send to channel"** writes the reply in the thread and a copy in the active segment. The copy
carries `also_in_thread`, so the channel reader sees the decision and can open the thread.

* The "followed" state is personal. The client stores it with the bookmarks, in the user settings.

### Object Storage

Every document of a channel uses the document layout of `ambre-spec-object-store.md`, under its
own `document_id`. No object name carries a channel branch. Attachments use the media layout of
the same document.

### Segment sealing

A segment moves through three states: `active`, `closing`, `sealed`. Closing and sealing are two
acts, so that a recent message is never in a sealed segment.

**Closing.** When the active segment exceeds ~5000 messages or ~2 MB of snapshot:

1. The client opens a new active segment in `meta`. New messages go there.
2. The old segment becomes `closing`. It still accepts edits and deletes as ordinary deltas.

At most two segments accept deltas: the active one and the closing one.

**Sealing.** When the grace period of a closing segment ends:

1. Any online member client asks the relay for a seal lease, `SealLease(document_id)` on the segment.
2. The relay grants the lease to one client, for a bounded time. The relay freezes the segment:
   it refuses every later delta for it, and answers with the last `seq`.
3. The client encodes a shallow Loro snapshot of the deltas up to that `seq`, with no history,
   encrypts it and PUTs it to Object Storage.
4. The client builds and PUTs the search index of the segment (section 5).
5. The client sends `SealCommit(document_id, snapshot_id, horizon)`. The relay checks the lease and
   records the seal time, which retention reads.
6. The client updates Doc meta (`state: "sealed"`).

A lease that expires releases the segment to the next client. The relay reuses the horizon of a
document snapshot (`ambre-spec-sequencer.md`).

**The relay knows the state of each segment.** It already knows each segment as a document of the
channel. The state is outside the `ZK-007` set today, like the retention duration.

**An offline edit can meet a frozen segment.** The relay refuses the delta. The Sync worker then
rewrites the edit as an amendment (see *Amendments to a sealed segment*). Without that rewrite, the
edit is lost.

**The deltas are purged after a second client confirms the seal.** The relay is blind, so a
malicious member can seal a snapshot that drops messages. A document keeps its deltas behind ten
snapshots, but a sealed segment purges them, so the loss would be final.

1. A second member client rebuilds the state from the deltas and compares it with the snapshot.
2. On a match, the second client confirms, and the relay purges the deltas.
3. With no confirmation, the relay purges after a verification delay. That delay needs a row in
   `ambre-spec-budgets.md`.

The grace period needs a row in `ambre-spec-budgets.md`, for example 24 h, to measure before it is
fixed. A message younger than the grace period is never in a sealed segment. So an edit of a recent
message is an ordinary delta, and never creates an amendments document.

**The sealed snapshot is shallow.** A full Loro snapshot keeps the deleted text in its history. A
shallow snapshot drops that history. So a message deleted before sealing leaves no text in the
sealed ciphertext.

Sealed segments are **immutable** -- no more deltas, just a final snapshot. This simplifies storage and sync.

### Amendments to a sealed segment

An edit or a delete of a message older than the grace period is rare. It goes to the amendments
document of its segment, never to the sealed snapshot.

`Loro::Doc (amendments)`, one per sealed segment, created on the first amendment:

- `"amendments"` (Map) -- `{msg_id: {content: Loro::Text} | {deleted: true, by: "author" | "moderator"}}`

The segment entry in `meta` carries `amended: true` once the document exists. So a client fetches
the amendments document only when it exists. A reader applies the amendments over the sealed
snapshot.

**Re-sealing makes a delete an erasure.** An amendment hides a deleted message, and the original text
stays in the sealed ciphertext. A client re-seals the segment, rarely and in batches:

1. It writes a new shallow snapshot with the amendments folded in.
2. It rebuilds and PUTs the search index.
3. It removes from the amendments document the entries that the new snapshot folded in.

The delay between a delete and its erasure needs a row in `ambre-spec-budgets.md`.

### Who may change a message

`ambre-spec-op-authorship.md` binds each operation to the user who signed its delta. Murmure reads
that binding to enforce its own rules.

**The rules.**

- A member creates a message only under their own name: its `author` is the signer.
- A member edits or deletes only their own message. The message container records the PeerID that
  created it, and that PeerID resolves to its author.
- A member adds or removes only their own reaction: the reaction key is the signer.

**The check runs before import.** The client imports the delta into a fork of the segment and reads
the operations it produces. A delta that breaks one rule is refused whole, by every client alike.
The fork costs up to one segment, about 2 MB. That cost needs a row in `ambre-spec-budgets.md`,
measured by a prototype.

**Moderation is out of the minimal form.** Only the author changes a message. A moderator who
hides the message of another member is the `Moderation` Story (`ambre-roadmap.md` E5).

**How moderation will hold.** A hide is within the power of the relay already: the relay can
withhold any frame. So the relay may check that the role still holds, with no new power.

1. The relay refuses a moderation from a member whose role it has revoked. The relay orders every
   frame, so the answer is unique.
2. Each client checks that the signer held the moderator role in the signed membership log, at the
   entry the operation cites. Every client reads the same log, so every client agrees.

Two choices of the minimal form keep that path open. The roles live in the signed membership log,
which only grows. An amendment records whether the author or a moderator deleted.

### Messaging compaction

More aggressive than for documents: compaction every **1-2 min** (vs 5 min for docs) because an active channel generates far more updates.

### Unread tracking

**The relay keeps a read cursor per user and per document**, in plaintext (`channel_read_cursors`,
`ambre-spec-server.md section 3`). It already knows the last `seq` of each document.

**The relay answers a yes or no, never a count.** "Activity since your cursor" is a comparison of
two numbers it holds, so it learns nothing new. The sidebar shows that dot at once.

**The client computes the count.** A reaction and an edit never count, and only the client can
tell a message from a reaction. It computes the count and the mentions once it holds the deltas.
The relay never sees the kind of a frame.

### Typing indicators

Native use case for SecSync **Ephemeral messages** -- never persisted, anti-replay counter. They ride
the ephemeral frames of the `meta` document, and they are on by default.

### Server-side authorization (Blind Relay enforcement)

Every document of a channel is a document for the relay, so the document guarantees hold with no
second path: the subscription check, the signature check before sequencing, and the clock and
`seq` of the Lua path. The one addition is that membership of the channel grants access to each
of its documents.

The channel handlers of the relay today accept a subscription with no membership check and
rebroadcast with no signature check (`SRV-047`, `SRV-048`). They disappear with the channel plane.

### Signed membership log

**The relay is the authority on access, and it cannot lie about membership.** Every act on
membership is an entry signed by its author, and the relay stores and serves the entries.

- An invite, a revoke and a role change each add one signed entry.
- An entry names the channel or the document, the member, the act, and the previous entry.
- Each client checks the chain back to the creator. A row with no valid chain is shown as unverified.

**A revoke is signed too.** Otherwise the relay could show a revoked member again by replaying the
old invite, which stays valid.

**What the relay can still do: withhold recent entries.** It can show a list that is late, never a
list it made up. A revoke rotates the key, so a revoked member reads nothing new either way.

**The log replaces a role list in `meta`.** It carries the role history that moderation checks in
V1, so a second copy would only drift. The rule serves documents as well as channels, so it lives
in `ambre-spec-membership.md`.

### Key management

One channel = one key from a crypto standpoint, shared by all its documents. Sharing = encrypting the key with each member's X25519 pubkey. When a member leaves: rotation via the flow of `ambre-spec-key-rotation.md`.

**A rotation closes the active segment.** The new key opens a new active segment. So a segment is
sealed under one key, and never holds deltas under two.

### Direct messages are channels

A direct message and a channel share one model: one `channel_id`, one `meta` document, one chain of
segments, one Document Key.

| | Direct message | Private channel | Public channel |
| :--- | :--- | :--- | :--- |
| `type` in `meta` | `dm` | `private` | `public` |
| space | none | one | one |
| name | optional, in `meta`; composed from the members at display when absent | in `meta` | in the `space` document |
| key held by | the members | the members | the members and the `space` document |

**The name of a direct message is optional.** Any member may set it or change it in `meta`. With no
name, the client composes one from the member names at display, and never stores it. A stored
composition would go stale when a member renames, as `CHROME-080` rules for a document.

**No forward secrecy.** Three choices of this sketch require every key to stay available:

- a new device or a new member reads the full history (see *New member onboarding*);
- compaction and sealing encrypt the full state under the current key;
- the search index on the relay is encrypted under the channel key.

Matrix met the same conflict: its key backup keeps every Megolm key. What protects a direct message
is the key rotation when a member leaves, and the retention of a space.

**Conversion.** A member converts a direct message into a private channel of a space, in one act:

1. The client sets `type: "private"` and writes a name in `meta`.
2. The relay records the space of the channel.
3. The `channel_id` and the segments stay.
4. The converting member chooses the history of later members: keep it, or start fresh.

A direct message converts to a private channel only. A private channel becomes public through a
second act by a space admin, which writes its key into the catalogue.

**Keep is the default.** The key stays, and every later member reads the whole history.

**Start fresh uses a history boundary.** The conversion rotates the key and records a history
boundary in `meta`. A later member receives only the keys after the boundary, so the older
segments stay unreadable to that member. The members of the direct message keep the older keys
and still read the whole history.

**The boundary protects against later members only.** A member of the direct message can still
copy the text or pass on an older key.

**Each member of the direct message is warned.** The warning names the space and the history
choice before the conversion applies. The same warning appears when a third member joins a
direct message of two.

**The history boundary is general.** Any invite into a private channel can use it, as "invite
without history". The conversion is its first use.

A member of the direct message who is not a member of the space stays a member of the channel.

**A direct message is reused.** Opening a direct message with the same members opens the existing
one. The relay finds it from the membership it already holds, so it learns nothing new.

### Attachments, emoji and animated images

**Attachments** go through the shared media flow (`ambre-spec-media.md`). The message stores the
reference, the name, the media type and the size. The media address is a hash of the plaintext
today, which that document states as a defect (`SYNC-050`). Murmure multiplies the files, so the
fix lands before or with the attachments.

**Emoji.** The minimal form draws Unicode emoji and offers a picker. A color emoji font is large, so
it loads on first use and needs a row in `ambre-spec-budgets.md`. Custom emoji per space come later.

**Animated images.** An animated GIF is an attachment. The client decodes it off the render thread
and animates it on the canvas. It stays still under reduced motion, and off screen. There is no GIF
search: each search would send what the user types to a third party.

### Channel list and personal state

Four stores hold what the channel sidebar shows. Each fact has one owner.

| Fact | Owner | Visible to the relay |
| :--- | :--- | :--- |
| The channels a user can read | the relay, through `channel_members` | yes, routing metadata |
| The catalogue of a space: public channels, names, topics, categories, order | the `space` document | no, sealed |
| The Document Key of a public channel | the `space` document | no, sealed |
| The name and topic of a private channel or a direct message | the channel `meta` document | no, sealed |
| The public flag and the space of a channel | the relay | yes |
| The personal layout, mute, bookmarks and followed threads | the user settings | no, sealed |
| The notification level | the user settings, with a copy on the relay | yes, the copy |
| The read cursor | the relay, through `channel_read_cursors` | yes |

**The channel list is membership.** The client asks the relay for its channels. The answer carries
each `channel_id` and its wrapped Document Key. The launcher lists documents the same way today
(`CHROME-044`). No document holds a second copy of that list, because such a copy drifts after a
revoke.

**The space document is the catalogue of a space.** It is one sealed Loro document per space, and
every member of the space is a member of it.

**A space is a document.** Joining a space means joining its `space` document, through the same
invite and the same signed membership log as any document. The relay holds nothing of its own for
a space, beyond the space each channel belongs to.

- Space admins are `editor` or `owner`. The relay refuses a write from a `viewer` (`SRV-008`).
- Other members are `viewer`. They read the catalogue and cannot change it.

`Loro::Doc (space)`:

- `"categories"` (Map) -- `{category_id: {name, position}}`
- `"channels"` (Map) -- `{channel_id: {name, topic, category_id, position, key}}`

The catalogue lists public channels only. Every member of the space reads it, so a private channel
in it would show its name and its existence to the whole space. A private channel and a direct
message appear through membership alone. Their place in the sidebar is in the user settings.

**The space document owns the name of a public channel.** A member who has not joined holds no key
for the channel `meta` document, so the name cannot live there. The name has one owner, so only a
space admin renames a public channel. The `"channel"` Map of the `meta` document names a private
channel or a direct message only.

**The client joins the catalogue to its memberships.**

| Catalogue entry | Membership | The sidebar shows |
| :--- | :--- | :--- |
| yes | yes | a joined channel |
| yes | no | a channel to join |
| no | yes | a private channel or a direct message |

**Joining a public channel needs no other member.** The `key` field holds the Document Key of the
channel, sealed by the space document. The client wraps that key for its own X25519 key and asks
the relay for a membership row. The relay grants the row to a space member only for a channel it
knows as public in that space. So the relay holds the public flag and the space of each channel.
That flag is outside the `ZK-007` set today, like the notification level below.

**A self-join is checked against the catalogue, never against the relay flag.** The joining member
signs their own entry in the membership log. Every client accepts that entry only when both hold:

- the signer is a verified member of the space document;
- the catalogue of the space lists the channel.

Only space admins write the catalogue, and the relay cannot read it. A relay that flips a private
channel to public can create a row, and every client shows that row as unverified.

**The user settings hold the personal state.** Murmure keeps no store of its own. It writes one
`"murmure"` section in the sealed per-user store of the
`User settings` Story (`ambre-roadmap.md` E6).
Macro approvals and the personal dictionary of Prose already wait on that store. That Story
decides the key, the document layout and how a new device finds the store.

- `"layout"` (Map) -- `{channel_id: {group, position, hidden, collapsed}}`
- `"notify"` (Map) -- `{channel_id: "all" | "mentions" | "none"}`
- `"bookmarks"` (Map) -- `{msg_id: {channel_id, segment_id, added}}`
- `"followed"` (Map) -- `{root_msg_id: {channel_id, since}}`

Each key is a channel or a message, so a last-writer-wins map fits every entry. The store merges
the edits of several devices.

**Leave and hide are two acts.**

- Leave adds a signed entry to the membership log, and the relay removes the row. A private channel then rotates its Document Key. A
  public channel does not: every space member can read its key in the catalogue.
- Hide sets `hidden` in the user settings. The user keeps the key and can still read.

A user who hides a channel and believes they left still receives its traffic.

**Leaving a space rotates every key it shared.** The space document rotates its Document Key. An
admin also rotates the key of each public channel and writes the new keys to the catalogue. The
cost grows with the number of public channels. That cost is the price of joining without a
member online.

**Each public channel keeps its own key.** A space member reads every key in the catalogue, so a
shared key would expose nothing more to that member. The gain is for a member of one channel who
is not a member of the space, such as a bot or a guest. That member holds the key of one channel
and reads no other channel.

**A guest is never a member of the space document.** The catalogue holds the key of every public
channel, so a guest who reads it would hold them all. A guest enters a channel only through an
invite, signed by the member who invites. The membership log then shows who is a guest, who let
them in, and in which channels. A guest cannot self-join: the first condition above fails.

**The rotation needs a budget row before Murmure ships.** The row goes in `ambre-spec-budgets.md`
and bounds the rotation on a space leave. A measure against that row decides whether the shared key
comes back (section 8).

**The notification level has two copies.** The user settings are the source of truth for the
interface. The client writes a copy of each level to the relay, because the relay decides which
push to send (section 4). The copy and `channel_read_cursors` are outside the `ZK-007` set today.
The zero-knowledge boundary table must record both before either ships.

### New member onboarding

Choice: access to **the entire history** (Document Key is shared). Consistent with a collaborative office suite (unlike Signal which limits to member join time).

* Each rotation creates a new Document Key. A new member needs every key since the latest history boundary, or since the channel start when there is none (see *Direct messages are channels*).
* The inviting client sends one signed bundle of all these keys, as Matrix MSC4268 does.
* The Matrix MSC3061 implementation forwarded keys one by one, and it had a vulnerability.

### Decryption failures

A client can hold a message whose key is missing, for example on a new device.

* The message shows the cause and the remedy, for example "key bundle not received, ask a member".
* A bare "unable to decrypt" is forbidden. Element users rank that state as a main complaint.

### History and retention

**The default is the full history.** Murmure keeps every segment as long as the space exists. A
sealed segment weighs about 2 MB.

**Retention is a space setting.** A space admin sets one duration N for the whole space. No channel
overrides it.

**The relay applies retention.** A client-side policy would never erase while no admin is online, so
the erasure promise of the General Data Protection Regulation (GDPR) would be false.

- The relay holds N in plaintext. N is outside the `ZK-007` set today, like the public flag.
- The relay deletes a sealed segment, its index and its amendments once the seal time is older
  than N. The relay knows the seal time without a key.
- A client removes the missing segment from the `segments` array of `meta` when it finds the gap.

**The guarantee is a range.** A message stays at least N, and at most N plus the time span of its
segment. The interface states that range, never N alone.

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
* **Store:** The client encrypts the index with the Document Key and PUTs it beside the snapshot of the segment document.
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

### Amended segments
The index of a sealed segment stays immutable.
1. The client queries the sealed index.
2. The client drops the matches whose `msg_id` the amendments document deletes.
3. The client queries a small index of the amendments document, built on the fly.

A re-seal replaces the sealed index (section 3, *Amendments to a sealed segment*).

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
| **Deltas on a sealed segment** | Every old segment becomes a surface of concurrent writes again, and its search index becomes mutable. Segmentation exists to prevent both. |
| **A new sealed snapshot on each edit** | A typo fix would need the seal lock and a full index rebuild. |
| **Amendments in the active segment** | Loading an old segment would need every later segment, to find its amendments. |
| **Sealing on size alone** | The last message before the cut lands in a sealed segment, and its first typo fix creates an amendments document. |
| **Retention applied by a client** | No erasure happens while no admin is online. |
| **A retention setting per channel** | Two settings for one space, and a member cannot tell which one holds. |
| **Forward secrecy for direct messages** | A full history on a new device, the compaction and the search index all need every key. Reconsider it only for a separate product: direct messages with no history on the relay, as Signal does. |
| **A separate model for direct messages** | Converting a direct message into a channel would copy its history into a new document, with new identifiers and a new key. One model makes the conversion a change of `type`. |
| **A direct message converted straight into a public channel** | The whole space would read a history written for a few members. The public step is a second act, by a space admin. |
| **A history duration chosen at conversion** | Keys change at a rotation, not on a date. A cut inside a segment would split and re-seal it under two keys. A cut at a segment edge would show a false duration. The retention of the space already bounds the history. |
| **A client-side lock on sealing** | A client does not reach Redis. The relay arbitrates through a lease. |
| **Purging the deltas at the seal** | A false snapshot from a malicious member would lose messages for good. |
| **Roles in a last-writer-wins map** | Compaction would drop every past role. A client could no longer check a moderation against the role its author held. |
| **Moderation in the minimal form** | A role lives in `meta` and a message in a segment, two documents. A revoke concurrent with a delete has no single answer without the relay check and the role log. |
| **A dedicated channel plane, with its own `seq` per segment in the relay Lua** | A second pipeline to build and secure beside the document one. A channel as a set of documents reuses the sequencer, the snapshots, the replay and the access check. |
| **One `threads` document per channel** | It grows with every thread of the channel for years. One document per thread grows with that thread only. |
| **A member list in the `meta` document** | It duplicates the relay and drifts after a revoke. The signed membership log checks the relay instead. |
| **Membership the relay serves unsigned** | The relay could show any member list. An unsigned start would also leave every early membership unverifiable for good. |
| **A frame kind in plaintext, so the relay counts unread messages** | One more leak in the `ZK-007` set, for a count the client computes from the deltas it fetches anyway. |
| **A new direct message each time** | Two conversations between the same members split one history. |
| **GIF search through a third-party service** | Each search sends what the user types to that service. |
| **A self-join checked against the public flag of the relay** | The relay could flip a private channel to public and show a verified member who holds no key. The catalogue is sealed and written by admins only. |
| **A guest who browses the catalogue** | The catalogue holds the key of every public channel, so the guest would read them all. A member invites a guest channel by channel. |
| **A sealed index document of the user's channels** | The relay already holds the membership. A second list drifts after a revoke and shows a channel whose key is gone. |
| **Bookmarks and followed threads in local storage** | A new device starts empty, and a new device is where the user needs them most. |
| **Private channels in the space catalogue** | Every space member reads the catalogue, so each private channel would show its name and existence to the whole space. |
| **Public channel names in the channel `meta` document** | A member who has not joined holds no key for `meta`, so the channel to join would have no name. |
| **One key for every public channel of a space** | A bot or a guest invited to one channel would read every public channel of the space. Reconsider it if a measure shows that the rotation on a space leave is too slow for the admin client, or too frequent for large spaces. |
| **A member online to wrap the key of a public channel** | Joining would wait on another member. The catalogue carries the key, and a space leave pays for it with a rotation. |
| **A Murmure document of its own for personal state** | The `User settings` Story (`ambre-roadmap.md` E6) already plans one sealed store per user. A second store gives the same user two sources of truth. |
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
