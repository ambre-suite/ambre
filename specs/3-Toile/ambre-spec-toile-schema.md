# AMBRE -- SPEC: TOILE CRDT SCHEMA
**Type:** Schema (persistence format, one document per application)
**Implementation:** partial

> The persisted shape of a Toile board: every Loro container, its key, its value, and the
> granularity at which two peers race. It owns identity, keys, merge granularity and the convergence
> properties that follow from that shape. It owns no gesture, no permission and no event.

---

## 0. SCOPE, AND THE MENTION RULE

A schema decision is often justified by an action. An element identity survives a move, a reorder
and a regroup, and its `{peer}_{counter}` shape answers concurrent minting. So an action appears
here as a **cause**, never as a **rule**.

| Position | Actions | Why |
|---|---|---|
| Container table (section 3), hierarchy (section 4) | forbidden | pure format |
| Container note (section 5), `Rationale` of a decision (section 6) | allowed, as a cause | the reason is the action |
| An invariant statement (section 1) | forbidden when it binds an action | "a locked element MUST NOT be moved by a command" belongs to the elements feature |

**Budgets**, each with the destination its overflow takes:

| Cap | Overflow goes to |
|---|---|
| 600 words of prose for the whole document, outside tables and lists | a table, or nothing |
| 40 words for a container note (section 5) | section 6, whose `Rationale` column is not capped |

---

## 1. INVARIANTS

Prefix `TOILE`, shared with the feature documents: one prefix, one numbering, no renumbering.
`TOILE-003` and `TOILE-018` sit here because each states a property of the format, not of the
engine. `TOILE-227` and `TOILE-228` are the halves those two hid. `TOILE-229`, `TOILE-230` and
`TOILE-231` state format rules sections 2, 3 and 7 already lean on and no row carried. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-003` | `INV` | Element identity MUST be a minted `ElementId` carried as a value; a positional or slot key MUST NOT be used. | this spec section 2 | structural -- `ElementId::mint` renders the document `PeerID` and a per-session counter (`crates/ambre-bus/src/app/toile.rs`), and every reference (a connector endpoint, a `children` entry, a `layers` entry, a reaction key) carries that string as a value, so no position is a key; `ambre_toile::logic::convergence_tests::copy_does_not_mutate_and_paste_remints_offset` -- a pasted element draws a fresh id; `ambre_toile::logic::convergence_tests::paste_reproduces_selection_on_a_separate_board` -- the minting carried across boards (neither walks a move, which is `TOILE-227`) | none | verified |
| `TOILE-227` | `INV` | A reference to an element MUST still resolve after a move, a reorder and a regroup. | this spec section 2; this spec section 6 | `ambre_toile::logic::convergence_tests::move_translates_bounds` -- it resolves the id after a translate; `ambre_toile::logic::convergence_tests::reorder_to_back_moves_node` -- it reads the `layers` order back after a `mov`, which is the call section 6 rejects the Loro slot id for; `ambre_toile::logic::convergence_tests::group_unions_bounds_and_sits_below_members` -- it resolves the members after they are regrouped | none | verified |
| `TOILE-229` | `INV` | A composite key MUST re-read as the pair it joins: a `layers` entry as tag and id, a reaction key as target and user. | this spec section 2 | structural -- an id is hex and `_`, a `UserId` is a hyphenated UUID, so no half can carry the `:` or the `/` the key splits on; `ambre_toile::logic::convergence_tests::reorder_to_back_moves_node` -- it walks the `NodeId` codec both ways; `ambre_toile::logic::convergence_tests::two_peers_converge` -- it compares the reaction tally of two peers, which is read by splitting the composite key | none | verified |
| `TOILE-231` | `INV` | Every id in the document MUST render at the fixed width `mint` produces: 25 characters for an id, 27 for a `layers` entry. | this spec section 2; this spec section 7 | none | claim -- the width is a habit of the minting path, not a property of the type, because `ElementId` is a public tuple over `String` and `NodeId::decode` rebuilds one from whatever follows `E:`, so any string entering as an id is an id. Nothing reads either number (`crates/ambre-bus/src/app/toile.rs` carries no test at all) and the size line of section 7 counts on both; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-018` | `INV` | An element's whole style MUST be a single LWW register. | this spec section 3; architecture section 2.D | structural -- one `LoroValue::Binary` under the `style` key per element map (`logic/state/elements.rs`), decoded whole by `ElementStyle::from_bytes`, so no attribute has a register to be elected on its own | none | verified |
| `TOILE-228` | `CONC` | Two concurrent restyles of one element MUST converge on the whole style one of the two wrote, never a per-attribute blend. | this spec section 3; architecture section 2.D | proptest -- `ambre_toile::logic::convergence_tests::two_peers_converge` draws 48 interleavings of restyles of the same element across two peers and compares the full rendered viewport, so a blend of two writers' attributes diverges (Trame's dedicated no-blend tests have no counterpart here) | none | verified |
| `TOILE-230` | `INV` | An unset property MUST be the absence of its key; a container MUST NOT carry a "cleared" marker. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_auto_layout_add_change_clear` -- it clears a frame's auto-layout and reads back `None` with the key gone, and the clear of an already-absent key broadcasting nothing, both of which a marker would break | none | verified |
| `TOILE-027` | `INV` | A Toile document MUST carry `meta.schema_version`. | architecture section 2.D; this spec section 7 | `ambre_toile::logic::schema::tests::a_new_board_carries_the_current_version` -- the stamp `schema::stamp` writes inside the scaffolding commit of `from_doc`, alongside the board defaults, so a board carries it before any user edit (the protocol it obeys is `ambre-spec-schema-version.md`) | none | verified |
| `TOILE-029` | `INV` | A container under a fixed key MUST be created once, with its parent; it MUST NOT be created by the first peer that writes into it. | this spec section 5 | `ambre_toile::logic::convergence_tests::concurrent_notes_edits_converge` -- a frame's `notes`, seeded at frame birth so two concurrent first edits both survive; `ambre_toile::logic::convergence_tests::concurrent_first_decks_converge_on_one_list` -- the deck, where two peers writing a first slide order keep both slides; `ambre_toile::logic::schema::tests::a_board_whose_deck_sits_in_the_presentation_map_keeps_it` -- the step that carries an older board into the root containers | none | verified |
| `TOILE-030` | `CONC` | A sticky or text body MUST converge character by character; it MUST NOT be a register two authors race for whole. | this spec section 3; `features/ambre-spec-toile-text-editing.md section 3` | none | fails -- for a sticky and a text element, the body is a flat String under the element's `text` key, so one of two authors typing in the same note loses their sentence. The two `LoroText` bodies of `ambre-toile` are a frame's `notes` and a comment message. It lands with the `A note body converges character by character` Story (`ambre-roadmap.md` E7) | failing |
* **Budget.** none: a format has no operation to budget. What loading a board costs is the remote
  change row of `ambre-spec-budgets.md section 1`, and leg 4 of `ambre-spec-budgets.md section 4`.
* **Boundary.** none: the containers this table declares reach the relay as ciphertext alone
  (`ZK-001`), and the media entry holds a content hash, never the pixels
  (`features/ambre-spec-toile-media.md section 3`).
* **Verification.** 8 `verified`, 1 `failing`, 1 `untested`. What holds is the table's shape: the
  container per subject, and the composite key re-read as its pair. The key set a build writes is
  held too, append-only, by a CI gate. What is loose is the move-invariance two paste tests never
  walk. The one failure names the code that breaks it (`TOILE-030`).

---

## 2. IDENTITY

One identity type serves elements, connectors, frames and comment threads.

* `ElementId` is `"{peer:016x}_{counter:08x}"` -- 25 characters. `peer` is the document `PeerID`;
  `counter` is a per-session monotonic counter.
* `ConnectorId` uses the same rendering and is a distinct type, so the compiler refuses one where
  the other is meant. `FrameId` and `TargetId` are aliases of `ElementId`.
* A `layers` entry joins a tag and an id with `:`: `"E:{element_id}"` or `"C:{connector_id}"`.
* A `reactions` key joins a target and a user with `/`: `"{target_id}/{user_id}"`. An id is hex and
  `_`, a `UserId` is a hyphenated UUID, so neither half can contain the separator.
* `crates/ambre-bus/src/app/toile.rs` holds the two id types and the `NodeId` codec.

Both the id shape and the two composite keys are decisions, with their rejected alternatives in
section 6. The width is `TOILE-231`, the two splits are `TOILE-229`.

---

## 3. CONTAINERS

`State` is `built` when the container exists in `crates/ambre-toile/src/`, `design` when this schema
declares it and no code writes it yet. `LWW` names the unit at which two concurrent writers race:
the last writer takes that whole unit. A nested container carries its path; the leaf is the name the
code uses.

| Container | Loro type | Key | Value | LWW unit | Owning feature document | State |
|---|---|---|---|---|---|---|
| `meta` | Map | `schema_version` | `u32` | the key | -- (foundation; this spec section 7) | built |
| `board` | Map | `name`, `bg_color`, `grid_size`, `grid_snap` | String, `u32`, `f64`, `bool` | the key | viewport | built |
| `elements` | Map | `element_id` | one element map | the element entry | elements | built |
| `elements/{id}` | Map | a field name, per the table below | per field | the field | elements | built |
| `elements/{id}/children` | MovableList | position | `element_id` | list order, no LWW | frames, groups | built |
| `elements/{id}/notes` | Text | -- | speaker notes | none: char-CRDT | presentation | built |
| `connectors` | Map | `connector_id` | one connector map | the connector entry | connectors | built |
| `connectors/{id}` | Map | a field name, per the table below | per field | the field | connectors | built |
| `connectors/{id}/label` | Text | -- | the edge caption | none: char-CRDT | connectors | built |
| `layers` | MovableList | position | `"E:{element_id}"` or `"C:{connector_id}"` | list order, no LWW | z-order | built |
| `media` | Map | `blake2b` content hash | one media map | the media entry | media | built |
| `media/{hash}` | Map | `w`, `h`, `mime`, `thumb` | `i64`, `i64`, String, Binary | the key | media | built |
| `presentation_order` | MovableList | position | `frame_id` | list order, no LWW | presentation | built |
| `presentation_config` | Map | `transition_ms`, `autoplay`, `loop` | `f64`, `bool`, `bool` | the key | presentation | built |
| `reactions` | Map | `"{target_id}/{user_id}"` | `Binary(postcard(Reaction))` | that user's reaction on that target | reactions | built |
| `comments` | Map | `thread_id` | one thread map | the thread entry | comments | built |
| `comments/{id}` | Map | `anchor`, `resolved`, `messages` | `Binary(postcard(CommentAnchor))`, `bool`, the list below | the key | comments | built |
| `comments/{id}/messages` | MovableList | position | one message map | list order, no LWW | comments | built |
| `comments/{id}/messages/{n}` | Map | `author`, `ts`, `body` | String, `i64`, the text below | the key | comments | built |
| `comments/{id}/messages/{n}/body` | Text | -- | the message body | none: char-CRDT | comments | built |
| `tags` | Map | `tag_id` | `{label, color}` | the tag entry | elements | design |
| `timer` | Map | `ends_at`, `running` | `i64`, `bool` | the key | reactions | design |
| `vote_session` | Map | `open`, `budget` | `bool`, `u8` | the key | reactions | design |

**The element map, field by field.** `Kinds` names the element types that carry the field. A frame is
an ordinary element and carries the whole geometry block. A **group carries no coordinates at all**,
because its box is the derived union of its members (`TOILE-009`,
`features/ambre-spec-toile-groups.md`). A reader of a group's map finds the type, the rotation, the
lock and the style, and reads the box from the derived cache.

| Field | Type | Holds | Kinds | State |
|---|---|---|---|---|
| `type` | String | `sticky`, `shape`, `text`, `image`, `frame`, `group`, `drawing`, `embed`, `doc_embed` | every | built |
| `x`, `y`, `w`, `h` | `f64` | the world AABB, top-left, pre-rotation | every but `group` | built |
| `rot` | `f64` | radians, about the AABB centre | every | built |
| `locked` | `bool` | the advisory lock | every | built |
| `style` | Binary | the layout version byte then `postcard(ElementStyle)`, one register per element (`TOILE-018`, `TOILE-UI-071`) | every | built |
| `link` | Binary | `postcard(LinkTarget)`; absent means no link | every | built |
| `name` | String | the frame title, which Find reads | `frame` | design |
| `text` | String | the body (`TOILE-030`) | `sticky`, `text` | built |
| `shape` | String | `rect`, `ellipse`, `triangle`, `line`, `path` | `shape` | built |
| `path` | String | the SVG path data of a vector node | `shape` | built |
| `points` | Binary | `postcard(Vec<StrokePoint>)`, the raw samples | `drawing` | built |
| `hash` | String | a key into `media` | `image` | built |
| `url` | String | the sandboxed web address | `embed` | built |
| `doc_id`, `doc_type` | String | the nested document reference | `doc_embed` | built |
| `autolayout` | Binary | `postcard(AutoLayout)`; absent means free positioning | `frame` | built |
| `title`, `tags`, `assignee`, `due` | -- | the deferred card fields | `card` | design |

**The connector map, field by field.**

| Field | Type | Holds | State |
|---|---|---|---|
| `from`, `to` | String | the endpoint `element_id`s | built |
| `from_anch`, `to_anch` | String | `t`, `b`, `l`, `r`, `c`, or `f:{u}:{v}` for a free anchor | built |
| `path_type` | String | `s` straight, `e` elbow, `c` curved | built |
| `arrow_start`, `arrow_end` | `bool` | the two end caps | built |
| `label` | Text | the edge caption, seeded empty at birth | built |
| `style` | Binary | the layout version byte then `postcard(ElementStyle)`, seeded at birth | built |

**Absence is a value** (`TOILE-230`). No `link` key means no hyperlink; no `autolayout` key means
free positioning. An empty `presentation_order` list means no deck; no `reactions` key under a target
prefix means no reaction. No container carries a "cleared" marker.

---

## 4. HIERARCHY

* `LoroDoc` root
  * `meta`
  * `board`
  * `elements`
    * per element, keyed by `element_id`: the geometry and payload fields, and for a frame or a
      group `children`, `notes` and `autolayout`
  * `connectors`
    * per connector, keyed by `connector_id`: the endpoints, the anchors, the path kind, the end
      caps, and `label`
  * `layers`
  * `media`
    * per blob, keyed by content hash: `w`, `h`, `mime`, `thumb`
  * `presentation_order`
  * `presentation_config`
  * `reactions`
  * `comments`
    * per thread, keyed by `thread_id`: `anchor`, `resolved`, `messages`
      * per message: `author`, `ts`, `body`
  * `tags`, `timer`, `vote_session`

---

## 5. CONTAINER NOTES

Only where a schema decision needs a reason. 40 words each.

* **`board`.** `name` is declared and unwritten: `from_doc` seeds the three display keys and no
  command sets a board name. No rule says what a peer reads when the key is absent.
* **`elements`.** One flat map holds every kind, frames and groups included. A parallel `frames`
  map would need its own move, resize, delete and reorder commands, and would split paint order
  over two structures.
* **`elements/{id}/notes`.** Seeded empty at frame birth, not on the first edit. A container
  broadcast in the creation delta lets two concurrent first edits target the same text. A lazy seed
  mints rival containers and drops one side.
* **`elements/{id}` payload.** The fields sit flat in the element map. The `payload` sub-map of the
  earlier design is not built, and one map fewer merges per element.
* **`connectors`.** Endpoints and a path kind, never geometry. Routing is recomputed on read, so
  moving an element writes one register and no connector op (`TOILE-006`).
* **`layers`.** One list for elements and connectors, so a connector can sit behind an element. A
  tagged string keeps the list flat and order-preserving.
* **`media`.** The entry holds the dimensions, the type and an inline thumbnail, never the pixels.
  The full blob is content-addressed and encrypted (`features/ambre-spec-toile-media.md section 3`).
* **`presentation_order`, `presentation_config`.** Two root containers rather than one map holding
  them. A root container's identity is its name. Every peer addresses the same one, and none is
  created by the first peer to build a deck (`TOILE-029`).
* **`reactions`.** A flat composite key, not a sub-map per target. A per-target sub-map is created
  on the first reaction to that target. Two peers reacting at once mint rival containers, and one
  set of reactions is dropped.
* **`comments`.** Each thread is its own entry under a deterministic root, so two peers commenting
  on one spot mint distinct threads. Message order is the list order; `ts` is display only, because
  a clock is untrusted.
* **`tags`, `timer`, `vote_session`.** Declared and unwritten. The tag palette rides with the
  deferred `card`; the timer and the vote session are the facilitation layer, out of the MVP.

---

## 6. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A minted `ElementId` carried as the map key, and as the value of every reference to it | a positional or slot-derived key | every reorder invalidates it, and a reorder is the whole gesture the board exists for. A minted id is invariant under a move, a reorder, a regroup and a delete, so a connector endpoint, a frame child list, a `layers` entry and a reaction key all keep resolving |
| A minted `ElementId` carried as the map key, and as the value of every reference to it | the Loro slot id (`ListItemId`) | `LoroMovableList::mov` reassigns the moved element's op id, so every reference keyed by that slot is orphaned on the first reorder. It is the same call Trame reversed for its rows (`ambre-spec-trame-schema.md section 6`) |
| `{peer:016x}_{counter:08x}`, fixed width | a document-global monotonic counter | it costs one LWW collision per concurrent create, on a board where concurrent create is the normal case. Two peers minting at the same moment draw a distinct `peer`, so identity needs no coordination and both creations survive |
| One binary register for a whole element style | one register per attribute | on a blind relay there is no cross-register atomicity, so a fill update racing a stroke update leaves a half-applied style. LWW on one blob converges every peer to one writer's whole style, which is the cross-app pattern of `architecture section 2.D` that Trame's `cellstyle_{sheet}` also follows |
| `locked` and `link` outside `ElementStyle` | folding both into `ElementStyle` | it puts three unrelated concerns on one register, and a restyle then silently reverts a lock. A lock is a lifecycle flag and a link is navigation; neither is a visual attribute |
| A frame is an ordinary element | a parallel `frames` map | it duplicates the move, resize, delete and reorder surface, and splits paint order over two structures. As an element there is one lifecycle for every node and one `layers` list, and frame-specific content is three payload fields |
| A group is a frame without chrome | an independent `groups` container with its own membership list | it is a third lifecycle, and two membership rules to keep in step. Reusing the frame's machinery costs one element type and no new container, and the group's bounds are the derived union of its members |
| `layers` holds a tagged string, not a sub-map | a `LoroMap` per entry | it is a container per z-order slot, and a reorder that touches two structures. A tagged string keeps the list flat and order-preserving, and a `mov` is one operation on one element |
| The deck in two root containers | one `presentation` map holding an `order` list and a `config` map | a sub-container under a fixed key is created by the first peer that writes into it, so two peers building a deck at the same moment mint rival lists and the engine keeps one. A root container is addressed by its name, which every peer computes alike |
| A flat `"{target_id}/{user_id}"` key for a reaction | a nested sub-map per target | it is created lazily by the first reactor, so two peers reacting on a fresh target mint rival containers and Loro drops one. The `reactions` root always exists, so concurrent inserts under distinct composite keys all converge, and splitting on the first `/` recovers the target |
| Keyed by `UserId`, never `PeerID` | keying by `PeerID` | it double-counts the tally and leaves a vote no later session can remove. `PeerID` is per session, so one user on two devices votes twice, while a reaction must survive a reload, a key rotation and a second device |
| Raw stroke samples only, never the smoothed curve | storing the smoothed polyline beside the samples | smoothing is a pure function of the samples, so replicating it costs bandwidth and can disagree with a peer's own render |
| A nested Ambre document is a reference, never inlined | inlining the child's content into this board | it is one key for two documents, and a viewer of the board reads a document they were never granted. As a reference the child keeps its own `DocumentId`, `DocumentKey` and `LoroDoc`, the relay sees no embedding table, and the child's access rules stay the child's |
| No `styles` dictionary | a shared style dictionary with an `s` indirection | it is a lookup paid before its benchmark. A board holds far fewer distinct styles than a spreadsheet holds cells, and the deduplication is unmeasured |

---

## 7. FORMAT DUTIES

| Duty | Rule | Where |
|---|---|---|
| Version | root `meta`, key `schema_version` (`u32`); absent means 0 | architecture section 2.D; `TOILE-027` |
| Read | equal proceeds, older migrates, newer is refused whole (`SchemaError::DocumentTooNew`), read in a scratch document so a refusal writes nothing | architecture section 2.D; `ambre-spec-schema-version.md section 2` |
| Migration | the registry lives in `ambre-toile` (`logic/schema.rs`), one row per bump; the foundation owns the protocol, never the semantics. `1 -> 2` puts the layout version in front of every `style` register, `2 -> 3` carries the deck out of the `presentation` map into two root containers | architecture section 2.D; `SCHEMA-010` |
| Bump | a key, a value type or the meaning of a value changes: `SCHEMA_VERSION` rises and one named fixture is frozen in the same commit | `SCHEMA-018`; `ambre-spec-schema-version.md` `SCHEMA-017`; `ambre-spec-fixtures.md section 2` |
| Unknown field | `postcard` decodes a blob whole and tolerates none, so an `ElementStyle` written by a newer client falls back to `Default` -- no longer silently: the read raises one warning per import (`SCHEMA-003`) | architecture section 2.D |
| Size, ids | an id is 25 bytes and a `layers` entry 27; a 50 000-element board is 1.3 MB of ids alone | this spec section 2; `TOILE-231` |
| Size, blobs | `media` carries a thumbnail inline and the full blob out of band; the board never holds pixels | `features/ambre-spec-toile-media.md section 3` |
| Deltas | a remote delta carries no version and applies blind; mixed-version live editing is bounded, not solved | architecture section 2.D |

---

## 8. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| That nothing derived enters the document: a routed connector, a smoothed stroke, a group's bounds | the connectors, freehand and groups features; `TOILE-006`, `TOILE-008`, `TOILE-009`. Toile has three derived quantities and one row each, where Trame has one and holds it here (`TRAME-010`) |
| What a locked element refuses, and how far the lock binds | the elements feature; `TOILE-012` |
| When a connector goes dangling, and what the user is offered | the connectors feature; `TOILE-007` |
| Where a comment pin anchors, and what a deleted anchor renders | the comments feature; `TOILE-024`, `TOILE-025` |
| That a child paints after its frame | the frames feature; `TOILE-011` |
| That one command is one undo item | the engine; `TOILE-014` |
| How a hash becomes pixels | `features/ambre-spec-toile-media.md section 3` |
| Who may write, and what the server enforces | architecture section 2.D; `ambre-spec-server.md` |
| That a document newer than the binary is refused whole | `ambre-spec-schema-version.md`; `SCHEMA-008` |
