# AMBRE -- SPEC: MURMURE CRDT SCHEMA
**Type:** Schema (persistence format, one document per application)
**Implementation:** design-only

> The persisted shape of Murmure: five kinds of Loro document, every container, its key, its value,
> and the granularity at which two peers race. It owns identity, keys, merge granularity and the
> convergence properties that follow from that shape. It owns no gesture, no permission and no event.

---

## 0. SCOPE, AND THE MENTION RULE

A schema decision is often justified by an action. A message survives an edit, and a reaction
survives a concurrent one. So an action appears here as a **cause**, never as a **rule**.

| Position | Actions | Why |
|---|---|---|
| Container table (section 3), hierarchy (section 4) | forbidden | pure format |
| Container note (section 5), `Why not` of a refused alternative (section 6) | allowed, as a cause | the reason is the action |
| An invariant statement (section 1) | forbidden when it binds an action | "a member MUST edit only their own message" belongs to the messages feature |

**Budgets**, each with the destination its overflow takes:

| Cap | Overflow goes to |
|---|---|
| 600 words of prose for the whole document, outside tables and lists | a table, or nothing |
| 40 words for a container note (section 5) | section 6, whose `Why not` column is not capped |

---

## 1. INVARIANTS

Prefix `MURMURE`, shared with the feature documents: one prefix, one numbering, no renumbering.
Grammar and status values are normative in `ambre-spec-template.md`.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-001` | `INV` | Every Murmure document MUST carry a `header` naming its kind, its channel or its space, and its `schema_version`. | this spec section 2 | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-002` | `INV` | A message's identity MUST be a `MessageId` minted by the peer that creates it; a position in a list MUST NOT be used as a key. | this spec section 2 | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-003` | `INV` | A message's `author` MUST hold a user identifier, never a display name and never a key. | this spec section 3; `ambre-spec-op-authorship.md` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-004` | `INV` | All message text MUST live in a `LoroText`; a `LoroMap` MUST NOT hold message text. | this spec section 3 | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-005` | `CONC` | Two peers typing in the same message MUST both keep their text. | this spec section 3 | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-006` | `INV` | A reaction MUST be one entry of one flat map, keyed by the pair of its emoji and its reacting user. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-007` | `CONC` | Two members reacting with the same emoji at once MUST both appear in the merged reaction. | this spec section 3; this spec section 6 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-008` | `INV` | A mention MUST be a `mention` mark whose value is a user identifier or the channel itself. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-009` | `INV` | A thread document MUST name its channel and its root message in its `header`. | this spec section 2 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-010` | `INV` | The `segments` list MUST hold at most one `active` and at most one `closing` segment. | this spec section 3 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-011` | `CONC` | Two peers closing the active segment at once MUST converge on one successor segment. | this spec section 5; this spec section 6 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-012` | `INV` | A final segment document MUST hold no operation later than its finalization. | this spec section 3 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-013` | `INV` | An amendments document MUST hold entries only for messages of its own segment. | this spec section 2 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-014` | `INV` | The catalogue MUST list public channels only. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-015` | `INV` | The `meta` document MUST NOT hold a member list. | this spec section 5; `ambre-spec-membership.md` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-016` | `INV` | An unnamed direct message MUST hold no `name` key: a name composed from its members MUST NOT be stored. | this spec section 3; `CHROME-080` | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-017` | `INV` | A message MUST hold an attachment by its media reference; no attachment byte MUST enter a document. | this spec section 3; `ambre-spec-media.md` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-018` | `INV` | An unset property of a Murmure document MUST be the absence of its key, never a "cleared" marker. | this spec section 3 | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-019` | `INV` | A Murmure document whose `schema_version` exceeds the compiled `SCHEMA_VERSION` MUST be refused whole. | this spec section 7; `ambre-spec-schema-version.md` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5). The refusal is foundation code, held by `SCHEMA-008`; Murmure owes the call from its own entry points | unimplemented |
| `MURMURE-157` | `INV` | The catalogue MUST hold each channel key sealed under the key of the space, so that no key material reaches the Logic thread. | this spec section 5; `SYNC-039`; `ambre-spec-zero-knowledge.md section 2` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-020` | `INV` | A change to a container's key shape or value layout MUST raise `SCHEMA_VERSION` in the same commit. | this spec section 7; `ambre-spec-fixtures.md section 2` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5). The rule binds a code change to a constant, the same argument as `SCHEMA-009` | unimplemented |
| `MURMURE-437` | `INV` | The shape each Murmure `SCHEMA_VERSION` writes MUST be frozen in the shapes file of `ambre-murmure`, under the freeze gate. | this spec section 7; `SCHEMA-015`; `SCHEMA-018` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none: a format has no operation to budget. What a segment weighs, and what memory the
  documents take, are rows of `ambre-spec-budgets.md`, cited by the features that own them.
* **Boundary.** none: every document here reaches the relay as ciphertext alone (`ZK-001`). What
  the relay learns of the grouping of these documents is `ZK-015`.
* **Verification.** 22 `unimplemented`, nothing else: `crates/ambre-murmure` is a `lib.rs`. Four
  rows state a merge result, and a convergence test reaches them once a channel exists.

---

## 2. IDENTITY

* A **channel** is identified by its `channel_id`, minted at creation. It names a set of documents,
  never one document.
* Every **document** of a channel or a space has its own `document_id`, a UUID v7 minted by the
  client. Its `header` names its kind and the channel or the space it belongs to (`MURMURE-001`).
* A **message** is identified by a `MessageId` minted by the peer that creates it (`MURMURE-002`). Its
  position in a list moves with every concurrent append.
* A **segment** is identified by its `document_id`. A thread reply, a pin and an amendment reach a
  message through the pair of its segment and its `MessageId`.
* A **thread** document names its channel and its root message (`MURMURE-009`).
* An **amendments** document names its channel and its segment, and holds only that segment's
  messages (`MURMURE-013`).

**Every value has its type.** The containers below name the Rust newtype each value maps to, per
`ai/ambre-rust-style.md`. `DocumentId`, `UserId` and `ChannelId` exist in `ambre-core`.
`MessageId`, `SpaceId` and `CategoryId` join them through the same `define_id!` macro.

```rust
pub struct Timestamp(u64);              // Unix milliseconds, display only: a clock is untrusted
pub struct Emoji(String);               // exactly one extended grapheme cluster
pub enum ReactionEmoji { Unicode(Emoji), Custom(CustomEmojiId) }
pub struct ReactionKey { pub emoji: ReactionEmoji, pub user: UserId }  // one map key, encoded as a string
pub struct CustomEmojiId(Uuid);         // minted by the admin's client
pub struct TagId(Uuid);                 // minted by the forum owner's client
pub struct ImportOrigin { pub name: String, pub ts: Timestamp }
pub struct MeetingId(Uuid);             // a Rendez-Vous meeting
pub struct MediaType(String);           // a lower-case type and subtype
pub struct ByteSize(u64);
pub struct SealedChannelKey(Vec<u8>);   // a channel key, sealed under the space key
pub enum DocumentKind { Space, Meta, Segment, Amendments, Thread }
pub enum ChannelType { Direct, Private, Public }
pub enum ChannelLayout { Stream, Forum }  // absent means a stream
pub enum SegmentState { Active, Closing, Final }
pub enum DeletedBy { Author, Moderator }
pub enum MentionTarget { User(UserId), Channel }
pub enum NotifyLevel { All, Mentions, None }
```

---

## 3. CONTAINERS

`State` is `design` for every row: no code writes a Murmure container yet. `LWW` names the unit two
concurrent writers race for. Every document also carries the `peers` registry of
`ambre-spec-op-authorship.md`, which is not repeated below.

| Document | Container | Loro type | Key | Value | LWW unit | Owning feature | State |
|---|---|---|---|---|---|---|---|
| every | `header` | Map | `kind`, `channel_id` or `space_id`, `schema_version`, `created_at` | `DocumentKind`, `ChannelId` or `SpaceId`, `u32`, `Timestamp` | the key | -- (this spec section 7) | design |
| `space` | `space` | Map | `name` | String | the key | spaces | design |
| `space` | `categories` | Map | a `CategoryId` | `{name: String, position: u32}` | the category entry | spaces | design |
| `space` | `channels` | Map | a `ChannelId` | `{name: String, topic: String, category_id: CategoryId, position: u32, key: SealedChannelKey}` | the field | spaces | design |
| `space` | `emoji` | Map | a `CustomEmojiId` | `{name: String, media_ref, media_type: MediaType}` | the entry | emoji | design |
| `meta` | `channel` | Map | `type`, `layout`, `name`, `topic` | `ChannelType`, `ChannelLayout`, String, String | the key | channels | design |
| `meta` | `segments` | List | position | `{document_id: DocumentId, from_ts: Timestamp, to_ts: Timestamp, msg_count: u32, state: SegmentState, amended: bool}` | the field | segments | design |
| `meta` | `pinned` | Map | a `MessageId` | `{segment: DocumentId, pinned_at: Timestamp}` | the entry | pins | design |
| `meta` | `tags` | Map | a `TagId`, for a forum | `{name: String}` | the entry | forums | design |
| `meta` | `commands` | Map | a command name | `{bot: PublicKey, description: String}` | the entry | bots | design |
| segment, thread | `messages` | List | position | one message map | list order, no LWW | messages | design |
| segment, thread | `messages/{n}` | Map | `id`, `author`, `ts`, `thread`, `also_in_thread`, `edited`, `deleted`, `title`, `origin`, `machine`, `call` | `MessageId`, `UserId`, `Timestamp`, `DocumentId`, `MessageId`, `bool`, `bool`, String, `ImportOrigin`, `PublicKey`, `MeetingId` | the key | messages | design |
| segment, thread | `messages/{n}/content` | Text | -- | the message body, with the `mention` mark | none: char-CRDT | messages, mentions | design |
| segment, thread | `messages/{n}/attachments` | List | position | `{media_ref, name: String, media_type: MediaType, size: ByteSize}`, the reference typed by `ambre-spec-media.md` | list order, no LWW | attachments | design |
| segment, thread | `messages/{n}/reactions` | Map | a `ReactionKey`: a `ReactionEmoji` and a `UserId` | a `Timestamp` | the entry | reactions | design |
| segment, thread | `messages/{n}/tags` | Map | a `TagId` | `true` | the entry | forums | design |
| segment, thread | `messages/{n}/actions` | List | position | `{id: String, label: String}` | list order, no LWW | bots | design |
| segment, thread | `messages/{n}/preview` | Map | `title`, `description`, `image` | String, String, a media reference | the key | link-previews | design |
| amendments | `amendments` | Map | a `MessageId` | `{content}` or `{deleted: bool, by: DeletedBy}` | the entry | messages | design |
| amendments | `amendments/{id}/content` | Text | -- | the edited body | none: char-CRDT | messages | design |
| settings | `murmure/layout` | Map | a `ChannelId` | `{group: SidebarGroup, position: u32, hidden: bool, collapsed: bool}` | the entry | sidebar | design |
| settings | `murmure/groups` | Map | a `PersonalGroupId` | `{name: String, position: u32}` | the entry | sidebar | design |
| settings | `murmure/notify` | Map | a `ChannelId` | `NotifyLevel` | the entry | notifications | design |
| settings | `murmure/bookmarks` | Map | a `MessageId` | `{channel: ChannelId, segment: DocumentId, added: Timestamp}` | the entry | sidebar | design |
| settings | `murmure/followed` | Map | a `DocumentId`, the thread | `{channel: ChannelId, since: Timestamp}` | the entry | threads | design |
| settings | `murmure/presence` | Map | `indicators` | bool | the key | typing-presence | design |

**The `state` of a segment** is `active`, `closing` or `final`. **The `by` of a delete** is `author`
or `moderator`. **The `mention` mark** carries a user id, or the channel for an `@channel`. It expands
to nothing: text typed beside it is not part of the mention. **The `emoji` mark** carries a
`CustomEmojiId` and draws its image.

**Absence is a value** (`MURMURE-018`). No `name` on a direct message means the client composes one
at display (`MURMURE-016`). No `thread` means the message has no reply. No `amended` segment has no
amendments document.

---

## 4. HIERARCHY

* a `space` document
  * `header`, `space`, `emoji`
  * `categories`, per category: its name and position
  * `channels`, per public channel: its name, topic, category, position and key
* a `meta` document, one per channel
  * `header`, `channel`, `segments`, `pinned`, `tags`
* a segment document, one per time slice of a channel
  * `header`, and `messages`, per message: its fields, `content`, `attachments`, `reactions` and
    `tags`
* an amendments document, one per amended final segment
  * `header`, and `amendments`, per message: `content`, or the delete
* a thread document, one per message with a reply
  * `header`, and `messages`, the same message shape
* the `murmure` section of the user settings (`ambre-spec-user-settings.md`), one per user
  * `layout`, `groups`, `notify`, `bookmarks`, `followed` and `presence`

---

## 5. CONTAINER NOTES

* **`header`.** One map every kind carries. A reader checks the channel and the version in it
  before it reads anything else.
* **`channels` in `space`.** The `key` field holds the key of a public channel, sealed under the
  space key by the Sync worker. The Logic thread holds only that sealed value (`MURMURE-157`).
* **`segments` in `meta`.** The relay arbitrates a close, so two peers closing at once receive one
  successor (`MURMURE-011`). The list records it, and the relay holds the state that decides.
* **`messages`.** A List, not a MovableList: a message never moves. Concurrent appends take the
  order Loro gives them, the same on every peer.
* **`reactions`.** One flat map, one entry per emoji and user. Two first reactors would each create
  a map per emoji under one key, and Loro keeps one. A flat key never collides (`MURMURE-007`).
* **`meta`, with no member list.** Membership is the relay's table, checked through the signed log
  of `ambre-spec-membership.md` (`MURMURE-015`).

---

## 6. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A message is keyed by a `MessageId` | its position in the list | a concurrent append moves every position after it, so a pin or a reply would point at another message |
| A reaction is one entry per emoji and user | a list of the users who reacted | two concurrent removals of one list slot remove the wrong member, and two concurrent adds duplicate one |
| A reaction is one entry per emoji and user | a counter per emoji | a counter cannot say who reacted, and a member could not remove their own reaction alone |
| The relay arbitrates a close | a successor identifier derived from the closing segment | two peers would create one document twice, and the relay would have to accept a second creation of an existing identifier |
| The relay arbitrates a close | letting each peer open its own successor | the channel would fork into two active segments, which `MURMURE-010` refuses |
| The catalogue holds a sealed channel key | the plain channel key in the space document | the space document lives in the Logic thread, and no key material may reach it (`SYNC-039`) |
| One `header` per document | inferring the kind from the containers present | a reader would guess before it can check, and a document of another channel would read as a valid one |
| Messages in a List | a MovableList | nothing moves a message, and a MovableList costs more to merge for an operation nobody performs |
| One flat map keyed by emoji and user | a map per emoji, holding a map per user | two members first reacting with one emoji create two child containers under one key, and one of the two reactions is lost |

---

## 7. FORMAT DUTIES

| Duty | Held by |
|---|---|
| Version | `header.schema_version` on every document (`MURMURE-001`) |
| Read | a version past the compiled one is refused whole (`MURMURE-019`) |
| Migration | the walk of `ambre-spec-schema-version.md`, per document kind |
| Bump | `MURMURE-020` |
| Freeze | the key set and the value types of each version, frozen as in the other applications and read by `scripts/schema_freeze.py` (`MURMURE-437`) |
| Unknown field | a map key no build knows is ignored on read, so a key a newer client wrote is dropped rather than refused |
| Delta format | a segment or thread delta is a Loro JSON update of schema version 1 (`MURMURE-227`). A change of that version is a bump (`MURMURE-020`) |
| Size | a segment closes at the segment size row of `ambre-spec-budgets.md section 6` |
| Deltas | every document takes the document pipeline: the sequencer, the sealed envelope and the replay. A remote delta carries no version and applies blind, as in the other applications. A segment, thread or amendments delta is a Loro JSON update, whose own `schema_version` a client checks (`MURMURE-223`) |

---

## 8. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Who may write, edit or delete a message | the messages feature, and `ambre-spec-op-authorship.md` |
| When a segment closes, and how it is finalized | the segments feature, and `ambre-spec-sequencer.md` |
| Who is a member of a channel or a space | `ambre-spec-membership.md`, `ambre-spec-server.md` |
| What the relay learns of these documents | `ambre-spec-zero-knowledge.md`, `ZK-015` |
| Which documents a client holds in memory | `ambre-spec-residency.md` |
| How the settings document is found, opened and shared out | `ambre-spec-user-settings.md` |
