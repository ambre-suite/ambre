# AMBRE -- SPEC: MURMURE CRDT SCHEMA
**Type:** Schema (persistence format, one document per application)
**Implementation:** partial -- the `meta` and segment documents are built

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
| `MURMURE-001` | `INV` | Every Murmure document MUST carry a `header` naming its kind, and its channel or its space. | this spec section 2 | `ambre_murmure::channel_tests::every_document_carries_a_header_and_the_foundation_version` -- the `meta` document and the segment of a new channel, and the version in each one's foundation `meta` map | none | verified |
| `MURMURE-002` | `INV` | A message's identity MUST be a `MessageId` minted by the peer that creates it; a position in a list MUST NOT be used as a key. | this spec section 2 | `ambre_murmure::channel_tests::a_sent_message_lands_in_the_active_segment_with_a_fresh_id_and_its_author` -- two sends mint two identifiers; type-level -- `MessageView` carries a `MessageId` and no position, so the view has no index to key on | none | verified |
| `MURMURE-003` | `INV` | A message's `author` MUST hold a user identifier, never a display name and never a key. | this spec section 3; `ambre-spec-op-authorship.md` | `ambre_murmure::channel_tests::a_sent_message_lands_in_the_active_segment_with_a_fresh_id_and_its_author`; type-level -- `MessageView` holds its author as a `UserId` | none | verified |
| `MURMURE-004` | `INV` | All message text MUST live in a `LoroText`; a `LoroMap` MUST NOT hold message text. | this spec section 3 | `ambre_murmure::channel_tests::the_body_lives_in_a_text_and_no_map_holds_it`; `ambre_murmure::schema_shape::a_body_stored_as_a_string_moves_the_shape` -- a body stored as a map string turns the frozen shape red | none | verified |
| `MURMURE-005` | `CONC` | Two peers typing in the same message MUST both keep their text. | this spec section 3 | `ambre_murmure::channel_tests::two_peers_typing_in_one_message_keep_both_texts` -- two peers type into one body at once, exchange deltas, and both read the same text holding both insertions | none | verified |
| `MURMURE-006` | `INV` | A reaction MUST be one entry of one flat map, keyed by the pair of its emoji and its reacting user. | this spec section 3 | `ambre_murmure::reaction_tests::two_members_reacting_at_once_both_stay`; `ambre_murmure::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version` -- one flat map per message, one key per member and emoji | none | verified |
| `MURMURE-007` | `CONC` | Two members reacting with the same emoji at once MUST both appear in the merged reaction. | this spec section 3; this spec section 6 | `ambre_murmure::reaction_tests::two_members_reacting_at_once_both_stay` -- both peers read both members under one emoji | none | verified |
| `MURMURE-008` | `INV` | A mention MUST be a `mention` mark whose value is a user identifier or the channel itself. | this spec section 3 | `ambre_murmure::mention_tests::a_mention_is_a_mark_that_carries_its_member`; `ambre_murmure::mention_tests::an_at_channel_names_the_channel_and_no_member` -- the mark holds a user or the channel identifier | none | verified |
| `MURMURE-009` | `INV` | A thread document MUST name its channel and its root message in its `header`. | this spec section 2 | not built | unbuilt -- lands with the `Threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-010` | `INV` | The `segments` list MUST hold at most one `active` and at most one `closing` segment. | this spec section 3 | `ambre_murmure::closing_tests::two_writers_of_one_close_converge_on_one_successor` -- two merged `meta` documents read one active and one closing segment; `ambre_server::db::channels::tests::a_channel_holds_one_active_segment` -- database tier, the relay store refuses a second active or closing segment | none | verified |
| `MURMURE-011` | `CONC` | Two peers closing the active segment at once MUST converge on one successor segment. | this spec section 5; this spec section 6 | `ambre_murmure::closing_tests::two_writers_of_one_close_converge_on_one_successor` -- the creator holds the successor and the other fetches it; `ambre_server::db::channels::tests::two_racing_closes_register_one_successor`; `ambre_server::routes::channels::tests::a_writer_closes_a_segment_and_a_late_close_learns_the_winner` -- database tier, the relay answers both closes one successor | none | verified |
| `MURMURE-012` | `INV` | A final segment document MUST hold no operation later than its finalization. | this spec section 3 | `ambre_murmure::finalizing_tests::an_accepted_commit_marks_the_segment_final_for_every_member` -- a reaction to a final segment is refused; `::a_final_snapshot_at_odds_with_the_deltas_is_refused` -- a final snapshot holding an operation no delta carried is a mismatch | none | verified |
| `MURMURE-013` | `INV` | An amendments document MUST hold entries only for messages of its own segment. | this spec section 2 | `ambre_murmure::amending_tests::an_amendment_outside_its_segment_is_refused` -- an entry for a message the segment lacks, and a document whose header names another segment, are each refused | none | verified |
| `MURMURE-014` | `INV` | The catalogue MUST list public channels only. | this spec section 3 | `ambre_murmure::space_tests::the_catalogue_refuses_a_private_channel_and_a_direct_message` -- the refusal writes nothing; `ambre_server::routes::channels::tests::an_editor_of_the_space_registers_a_public_channel` -- database tier, the relay holds the channel public in its space | none | verified |
| `MURMURE-015` | `INV` | The `meta` document MUST NOT hold a member list. | this spec section 5; `ambre-spec-membership.md` | `ambre_murmure::channel_tests::meta_holds_no_member_list`; `ambre_murmure::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version` -- a member list is a root the frozen shape does not carry | none | verified |
| `MURMURE-016` | `INV` | An unnamed direct message MUST hold no `name` key: a name composed from its members MUST NOT be stored. | this spec section 3; `CHROME-080` | `ambre_logic::direct::tests::a_created_direct_message_is_held_and_sent_whole` -- a new direct message holds no `name` key; `ambre_murmure::channel_tests::clearing_a_name_removes_its_key` -- nor does a cleared name | none | verified |
| `MURMURE-017` | `INV` | A message MUST hold an attachment by its media reference; no attachment byte MUST enter a document. | this spec section 3; `ambre-spec-media.md` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-018` | `INV` | An unset property of a Murmure document MUST be the absence of its key, never a "cleared" marker. | this spec section 3 | `ambre_murmure::channel_tests::an_unset_property_is_an_absent_key` -- an unnamed channel writes no `name` key | none | verified |
| `MURMURE-019` | `INV` | A Murmure document whose `schema_version` exceeds the compiled `SCHEMA_VERSION` MUST be refused whole. | this spec section 7; `ambre-spec-schema-version.md` | `ambre_murmure::channel_tests::a_document_a_newer_client_wrote_is_refused_whole`; `ambre_murmure::channel_tests::a_document_with_no_version_finds_no_migration_and_is_refused` -- the refusal comes from `ambre_core::schema::verdict`, called first at every open | none | verified |
| `MURMURE-157` | `INV` | The catalogue MUST hold each channel key sealed under the key of the space, so that no key material reaches the Logic thread. | this spec section 5; `SYNC-039`; `ambre-spec-zero-knowledge.md section 2` | `ambre_murmure::space_tests::a_listed_channel_carries_its_name_topic_category_position_and_key` -- stored and answered unopened; `ambre_logic::spaces::tests::a_self_join_hands_the_sealed_key_to_sync_and_writes_no_catalogue`; type-level -- `SealedChannelKey` exposes its bytes and no opening | none | verified |
| `MURMURE-020` | `INV` | A change to a container's key shape or value layout MUST raise `SCHEMA_VERSION` in the same commit. | this spec section 7; `ambre-spec-fixtures.md section 2` | `ambre_murmure::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version` -- the build's shape is compared against the section frozen for the compiled `SCHEMA_VERSION`, so a moved key or value type is red until the number rises; ci-gate -- `scripts/schema_freeze.py`, run by `just schema-freeze` in the CI `lint` job, which holds a published section append-only | none | verified |
| `MURMURE-437` | `INV` | The shape each Murmure `SCHEMA_VERSION` writes MUST be frozen in the shapes file of `ambre-murmure`, under the freeze gate. | this spec section 7; `SCHEMA-015`; `SCHEMA-018` | `ambre_murmure::schema_shape::the_shape_a_build_writes_is_the_shape_frozen_for_its_version`; `ambre_murmure::schema_shape::every_declared_key_appears_in_the_shape`; ci-gate -- `scripts/schema_freeze.py`, run by `just schema-freeze` in the CI `lint` job, which refuses a crate stamping a `SCHEMA_VERSION` with no frozen shape | none | verified |

* **Budget.** none: a format has no operation to budget. What a segment weighs, and what memory the
  documents take, are rows of `ambre-spec-budgets.md`, cited by the features that own them.
* **Boundary.** none: every document here reaches the relay as ciphertext alone (`ZK-001`). What
  the relay learns of the grouping of these documents is `ZK-015`.
* **Verification.** 20 `verified`, 2 `unimplemented`. The engine tests and the shape belt of
  `crates/ambre-murmure` hold every built container and the version gate.

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
`ai/ambre-rust-style.md`. `DocumentId`, `UserId`, `ChannelId` and `MessageId` exist in `ambre-core`.
`SpaceId` and `CategoryId` join them through the same `define_id!` macro, with the spaces Story.

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
pub enum NoticeKind { MemberAdded }     // a notice has an empty body; absent on a written message
pub enum MentionTarget { User(UserId), Channel }
pub enum NotifyLevel { All, Mentions, None }
```

---

## 3. CONTAINERS

`State` is `built` where `crates/ambre-murmure` writes the container, and `design` elsewhere. `LWW`
names the unit two concurrent writers race for. Every document also carries the `peers` registry of
`ambre-spec-op-authorship.md`, which is not repeated below.

| Document | Container | Loro type | Key | Value | LWW unit | Owning feature | State |
|---|---|---|---|---|---|---|---|
| every | `header` | Map | `kind`, `channel_id` or `space_id`, `created_at` | `DocumentKind`, `ChannelId` or `SpaceId`, `Timestamp` | the key | -- (this spec section 7) | built |
| every | `meta` | Map | `schema_version` | `u32` | the key | -- (`ambre-spec-schema-version.md`) | built |
| `space` | `space` | Map | `name` | String | the key | spaces | design |
| `space` | `categories` | Map | a `CategoryId` | `{name: String, position: u32}` | the category entry | spaces | design |
| `space` | `channels` | Map | a `ChannelId` | `{name: String, topic: String, category_id: CategoryId, position: u32, key: SealedChannelKey}` | the field | spaces | design |
| `space` | `emoji` | Map | a `CustomEmojiId` | `{name: String, media_ref, media_type: MediaType}` | the entry | emoji | design |
| `meta` | `channel` | Map | `type`, `layout`, `name`, `topic` | `ChannelType`, `ChannelLayout`, String, String | the key | channels | built |
| `meta` | `segments` | List | position | `{document_id: DocumentId, from_ts: Timestamp, to_ts: Timestamp, msg_count: u32, state: SegmentState, amendments: DocumentId, folded: DocumentId}` | the field | segments | built |
| `meta` | `pinned` | Map | a `MessageId` | `{segment: SegmentId, pinned_at: Timestamp}` | the entry | pins | design |
| `meta` | `tags` | Map | a `TagId`, for a forum | `{name: String}` | the entry | forums | design |
| `meta` | `commands` | Map | a command name | `{bot: PublicKey, description: String}` | the entry | bots | design |
| segment | `header` | Map | `folded`, `folded_seq` | `DocumentId`, `u64`, on a fold snapshot | the key | segments | built |
| segment, thread | `messages` | List | position | one message map | list order, no LWW | messages | built |
| segment, thread | `messages/{n}` | Map | `id`, `author`, `ts`, `thread`, `also_in_thread`, `edited`, `deleted`, `title`, `origin`, `machine`, `call`, `notice`, `member` | `MessageId`, `UserId`, `Timestamp`, `DocumentId`, `MessageId`, `bool`, `bool`, String, `ImportOrigin`, `PublicKey`, `MeetingId`, `NoticeKind`, `UserId` | the key | messages | built |
| segment, thread | `messages/{n}/content` | Text | -- | the message body, with the `mention` mark | none: char-CRDT | messages, mentions | built |
| segment, thread | `messages/{n}/attachments` | List | position | `{media_ref, name: String, media_type: MediaType, size: ByteSize}`, the reference typed by `ambre-spec-media.md` | list order, no LWW | attachments | design |
| segment, thread | `messages/{n}/reactions` | Map | a `ReactionKey`: a `ReactionEmoji` and a `UserId` | a `Timestamp` | the entry | reactions | built |
| segment, thread | `messages/{n}/tags` | Map | a `TagId` | `true` | the entry | forums | design |
| segment, thread | `messages/{n}/actions` | List | position | `{id: String, label: String}` | list order, no LWW | bots | design |
| segment, thread | `messages/{n}/preview` | Map | `title`, `description`, `image` | String, String, a media reference | the key | link-previews | design |
| amendments | `header` | Map | `segment_id` | `SegmentId` | the key | messages | built |
| amendments | `amendments` | Map | a `MessageId` | `{content}` or `{deleted: bool, by: DeletedBy}` | the entry | messages | built |
| amendments | `amendments/{id}/content` | Text | -- | the edited body, with the `mention` mark | none: char-CRDT | messages | built |
| settings | `murmure/layout` | Map | a `ChannelId` | `{group: SidebarGroup, position: u32, hidden: bool, collapsed: bool}` | the entry | sidebar | design |
| settings | `murmure/groups` | Map | a `PersonalGroupId` | `{name: String, position: u32}` | the entry | sidebar | design |
| settings | `murmure/notify` | Map | a `ChannelId` | `NotifyLevel` | the entry | notifications | design |
| settings | `murmure/bookmarks` | Map | a `MessageId` | `{channel: ChannelId, segment: SegmentId, added: Timestamp}` | the entry | sidebar | design |
| settings | `murmure/followed` | Map | a `DocumentId`, the thread | `{channel: ChannelId, since: Timestamp}` | the entry | threads | design |
| settings | `murmure/presence` | Map | `indicators` | bool | the key | typing-presence | design |

**The `state` of a segment** is `active`, `closing` or `final`. **The `by` of a delete** is `author`
or `moderator`. **The `mention` mark** carries a user id, or the channel for an `@channel`. It expands
to nothing: text typed beside it is not part of the mention. **The `emoji` mark** carries a
`CustomEmojiId` and draws its image.

**Absence is a value** (`MURMURE-018`). No `name` on a direct message means the client composes one
at display (`MURMURE-016`). No `thread` means the message has no reply. A segment entry with no
`amendments` has no amendments document. A segment entry whose `folded` names its `amendments` holds
them in its final snapshot, so no client opens them. A segment header with no `folded` folds
nothing in.

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
* an amendments document, one per amended locked segment
  * `header`, and `amendments`, per message: `content`, or the delete
* a thread document, one per message with a reply
  * `header`, and `messages`, the same message shape
* the `murmure` section of the user settings (`ambre-spec-user-settings.md`), one per user
  * `layout`, `groups`, `notify`, `bookmarks`, `followed` and `presence`

---

## 5. CONTAINER NOTES

* **`header`.** One map every kind carries. A reader checks the kind and the channel in it before
  it reads any other container.
* **`meta` map.** The foundation's version register, carried by every kind (`SCHEMA-004`). It shares
  its name with the `meta` document and nothing else. The version is checked before the `header`.
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
| The version in the foundation's `meta` map | the version in the `header` | the version would live in one place for Murmure and in another for Trame and Prose, the drift the shared names exist to stop |
| One `header` per document | inferring the kind from the containers present | a reader would guess before it can check, and a document of another channel would read as a valid one |
| Messages in a List | a MovableList | nothing moves a message, and a MovableList costs more to merge for an operation nobody performs |
| One flat map keyed by emoji and user | a map per emoji, holding a map per user | two members first reacting with one emoji create two child containers under one key, and one of the two reactions is lost |

---

## 7. FORMAT DUTIES

| Duty | Held by |
|---|---|
| Version | `meta.schema_version` on every document, the foundation's register (`SCHEMA-004`) |
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
