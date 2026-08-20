# AMBRE -- APP: MURMURE (COLLABORATIVE MESSAGING)
**Status:** Draft
**Implementation:** design-only

> Specifications for real-time communication, channels, and E2EE persistence.

## 1. VISION & FLOW
Murmure is the command and discussion center. Unlike classic SaaS tools, every message is an encrypted CRDT mutation, ensuring that even the "Blind Relay" server cannot read the content.
- **Engine:** Logic thread (Flow management) + wgpu + Parley/Swash (High-performance rendering).
- **Performance:** Local indexing enabling instant search across millions of messages.

---

## 2. CORE FEATURES

### A. Spaces & Channels (Discord/Slack Structure)
* **Spaces (Workspaces):** Logical team groupings with PKI permission management.
* **Channels (Public/Private):** Thematic streams organized in folders.
* **Threads:** Branched discussions to avoid polluting the main stream.
* **Direct Messages (DM):** 1:1 channels or restricted groups with forward secrecy.

### B. Chat Experience
* **Message Editor:** Simplified version of the **Prose** engine (Markdown, emojis, `@` mentions, code blocks).
* **Reactions:** Emoji additions on messages with CRDT aggregation (no conflict on reaction counts).
* **Pins & Favorites:** Marking important messages in local storage.
* **Status & Presence:** Real-time indicators (Online, Busy, Typing...) via WebSocket relay.

### C. Sharing & Integration
* **File Transfer:** Drag & drop of assets (images, docs). Files are encrypted and stored on MinIO; Murmure only stores the reference.
* **Unfurling:** Secure preview of links and internal documents (Trame, Toile, Prose) directly in the stream.
* **Calls & Video:** Native integration of the **Vue** engine to launch a conference from a channel in one click.

---

## 3. TECHNICAL ARCHITECTURE

### Time-segment architecture

An active channel accumulates ~200 messages/day with 20 participants. A single `Loro::Doc` per channel would become unmanageable (50 MB+ in 2 years with Loro's tombstone overhead). The solution: **one channel = a chain of segments**, each segment being an independent `Loro::Doc`.

```
channel_{id}/
├── meta                          ← Lightweight Loro::Doc (name, topic, members, pinned)
├── segments/
│   ├── seg_001                   ← messages 1-5000 (sealed, read-only)
│   ├── seg_002                   ← messages 5001-10000 (sealed, read-only)
│   └── seg_003                   ← messages 10001-... (active segment, writable)
```

**The client only loads the active segment + metadata** (~2 MB max on first display). Older segments are lazy-loaded when scrolling up.

### CRDT Structure -- Doc meta (per channel)

```
Loro::Doc (meta)
├── "channel" (Map)        → {name, type, created, topic}
├── "members" (Map)        → {"pubkey_alice": {role, joined}}
├── "pinned" (Array)       → [{msg_id, segment_id}]
└── "segments" (Array)     → [
│     {id: "seg_001", from_ts: 1710000000, to_ts: 1720000000, msg_count: 5000, sealed: true},
│     {id: "seg_002", from_ts: 1720000000, to_ts: 1730000000, msg_count: 5000, sealed: true},
│     {id: "seg_003", from_ts: 1730000000, to_ts: null, msg_count: 1847, sealed: false}
│   ]
```

### CRDT Structure -- Segment (one Loro::Doc per segment)

```
Loro::Doc (segment)
├── "messages" (Array)           → chronological order
│   └── (Map) {
│         id,
│         author_pubkey,
│         timestamp,
│         content,               → Loro::Text (editable, rich formatting)
│         reply_to,              → parent message id (threads)
│         reactions,             → Map {"👍": ["pubkey_a", "pubkey_b"]}
│         edited,                → bool
│         deleted                → tombstone (soft delete)
│       }
└── "media" (Map)                → same system as documents
```

`content` as `Loro::Text` enables message editing and rich formatting (bold, links, inline code, mentions).

### Object Storage

See **section 11** for the full hierarchy. Summary of messaging keys:

```
channels/{channel_id}/meta/snapshots/{snapshot_id}
channels/{channel_id}/meta/deltas/{seq:010}
channels/{channel_id}/segments/{seg_id}/snapshot      ← sealed = final snapshot only
channels/{channel_id}/segments/{seg_id}/deltas/{seq:010} ← active segment only
channels/{channel_id}/media/{content_hash}
```

### Segment sealing

When the active segment exceeds ~5000 messages (or ~2 MB snapshot):

1. Client acquires Redis lock (`channel:{id}:seal`, SET NX, TTL 30s)
2. Encodes the final segment snapshot, encrypts it, PUT to Object Storage
3. Updates Doc meta (marks `sealed: true`, creates new segment)
4. Old segment deltas are purged (snapshot is sufficient)

Sealed segments are **immutable** -- no more deltas, just a final snapshot. This simplifies storage and sync.

### Messaging compaction

More aggressive than for documents: compaction every **1-2 min** (vs 5 min for docs) because an active channel generates far more updates.

### Unread tracking

The only real server-side addition. The server tracks a "last read seq" cursor per user per channel -- unencrypted metadata (Postgres, see section 9 `channel_read_cursors`).

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

### New member onboarding

Choice: access to **the entire history** (Document Key is shared). Consistent with a collaborative office suite (unlike Signal which limits to member join time).

### Optional retention

With segments, a retention policy = deleting segments older than N months (`DELETE` Object Storage + Doc meta update). Useful for GDPR / right to erasure compliance.

### UI

1D virtualization (virtualized list) + Parley rendering for rich content. Much simpler than the spreadsheet's Grid2D.


---

## 4. NOTIFICATIONS & REAL TIME
* **Push Notifications:** Privacy-respecting notification proxy system (only message existence is notified, content is decrypted by client on receipt).
* **Cold Start Sync:** On startup, Murmure downloads missed deltas since the last disconnection to rebuild the channel state.

---

## 5. LOCAL SEARCH
Since the server is "blind", indexing is done client-side:
* **SQLite/FTS5 (WASM):** Full-text indexing of decrypted content.
* **Filters:** Search by author, date, channel, or file type.

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
* **Slash Commands:** Registered command system (e.g. `/jira create`, `/crypto price`) with client-side autocompletion.
* **WASM Applets (Sandboxing):** Ability to run data cleanup or formatting scripts directly in the Logic thread without leaving the security scope.

### C. Integration Modes
* **Native (Rust/TS SDK):** Full client running on a third-party server, connecting via WebSocket to the Blind Relay.
* **Inbound Webhooks (Bridge):** "Encryptor" micro-service that receives standard JSON (GitHub, Stripe) and transforms it into a signed Loro mutation before sending to the relay.
* **Ephemeral Bots:** Scripts launched on demand by a user to perform a one-off task on the local document (e.g. "Sort all messages by sentiment").

### D. Bot UX
* **Trust Indicator:** Visual badge certifying the bot uses an organization-verified key.
* **Audit Logs:** Every bot action is a signed mutation in the CRDT, making any modification traceable and reversible (Time-travel).


## 8. REJECTED TECHNICAL CHOICES
* **Matrix/XMPP Protocols:** Rejected in favor of native integration into the SecSync/Loro pipeline for full consistency with the rest of the Ambre suite.
