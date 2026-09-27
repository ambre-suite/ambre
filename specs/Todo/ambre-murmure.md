# AMBRE -- APP: MURMURE (BEYOND THE MVP)

**Type:** Sketch (the part of an application that has no spec yet)
**Implementation:** design-only

> A sketch, and not a specification: what Murmure becomes after its MVP. It declares no contract
> row, so it binds nothing and no rule is cited from it. The MVP is specified in `specs/5-Murmure/`,
> and this sketch no longer restates it.
>
> What users like and dislike in Teams, Slack and Discord is in
> `research/ambre-murmure-user-research.md`, with a do and do-not table for Murmure. This sketch applies
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

### Pinned messages

A member pins a message for every member of the channel. The `meta` document gains a `pinned` List
of `{segment: DocumentId, message: MessageId}`, in pin order. A pin names its segment, so the
client loads that segment alone to show it.

### A link to a message

A link names the channel, the segment and the message. It opens only for a member of the channel:
the link carries no key.

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

### Product

From `research/ambre-murmure-user-research.md`. Several rows hold for the MVP too; the specs apply them.

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
