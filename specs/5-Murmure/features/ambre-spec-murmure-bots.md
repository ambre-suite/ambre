# AMBRE -- FEATURE: MURMURE BOTS AND WEBHOOKS
**Type:** Vertical (feature)
**Implementation:** design-only

> A program takes part in a conversation: it answers a slash command, posts an alert, or offers
> buttons. It is a sealed client like any member, so the relay never reads what it reads or writes.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-387` | `OBS` | A message a machine wrote MUST show the machine badge and its declared host beside its name. | this spec section 3; `MACH-003`, `MACH-004` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-388` | `INV` | A bot MUST reach a channel through one class of `ambre-spec-machine-access.md`, invited like a guest. | this spec section 3; `MACH-001` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-389` | `INV` | The slash commands of a bot MUST be listed in the `commands` map of the channel `meta` document, written by the member who added it. | this spec section 3; `ambre-spec-murmure-schema.md section 3` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Run a slash command

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-390` | `POST` | A slash command MUST be completed on the client, from the `commands` map, and sent as an envelope addressed to its bot. | this spec section 3; `MACH-027` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-391` | `POST` | The answer of a bot MUST reach the channel as a message it signs, or as a post a member merges. | this spec section 3; `MACH-024` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-392` | `FRAME` | Running a slash command MUST NOT show its text to the other members. | this spec section 3; `MACH-008` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.3 Answer an interactive message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-393` | `POST` | A click on an action of a bot message MUST send an envelope addressed to that bot, naming the action. | this spec section 3; `MACH-027` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-394` | `FAIL` | Given an action whose bot has left the channel, the client MUST disable it. | this spec section 3 | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-395` | `FRAME` | Clicking an action MUST NOT change the message that carries it. | this spec section 3 | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.4 Receive a webhook

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-396` | `POST` | A webhook MUST reach a channel as an inbox post, sealed by an encrypting bridge the owner registered. | this spec section 3; `MACH-017` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-397` | `POST` | A merged post MUST become a message authored by the merging member, with the bridge as its machine origin. | this spec section 3; `MURMURE-078` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-398` | `FRAME` | A bridge MUST NOT hold the key of the channel. | this spec section 3; `MACH-008` | not built | unbuilt -- lands with the `Murmure bots and webhooks` Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** none of its own: a bot is a client, and a post is merged as `ambre-spec-machine-access.md`
  bounds it.
* **Boundary.** every bot message is sealed (`ZK-001`). A webhook bridge seals what it receives and
  never holds the channel key (`MURMURE-398`), so the relay stores only sealed posts.
* **Verification.** 12 `unimplemented`: no bot exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | The bot does | The channel holds | Every member sees | Test |
|---|---|---|---|---|
| types `/deploy staging` | reads the addressed envelope, deploys | the answer of the bot | the answer, with the machine badge (`MURMURE-387`) | none |
| -- | a CI bridge receives a failed build | a merged post | the alert, marked as coming from the bridge (`MURMURE-397`) | none |
| clicks "Approve" on a bot message | receives the action | the bot's follow-up | the follow-up; the message itself unchanged (`MURMURE-395`) | none |

---

## 3. MODEL

**CRDT containers.** the `commands` map of `meta` (`MURMURE-389`), the `actions` of a message, and
the `machine` origin of a merged post, in `ambre-spec-murmure-schema.md section 3`.

**Three classes, three uses.**

| Use | Class of `ambre-spec-machine-access.md` | What the bot reads |
|---|---|---|
| a bot that reads the channel and answers | machine member | the whole channel |
| a slash command, a button | addressed machine | the envelopes addressed to it alone |
| a webhook | inbox writer, through an encrypting bridge | nothing |

**Commands are listed by a member.** An addressed bot holds no key of the channel, so it cannot write
`meta`. The member who adds it writes its commands there (`MURMURE-389`).

**A slash command stays between the member and the bot.** The client completes it from the
`commands` map, then sends it as an addressed envelope (`MURMURE-390`). The other members see the
answer, not the command (`MURMURE-392`).

**A webhook is merged, not written.** A bridge receives the JSON of GitHub or Stripe, seals it to
the inbox of the channel, and signs the post (`MURMURE-396`). A member client merges it into a
message it signs itself, with the bridge named as the origin (`MURMURE-397`). The authorship check
holds: the signer is the author.

**Scripts a user runs are macros.** A cleanup or a formatting script that runs in the Logic thread
is a macro of `ambre-spec-macros.md`, not a bot.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| `/` in the composer | the commands of the bots of the channel, completed on the client |
| an action of a bot message | the action goes to the bot |
| add a bot, in the channel menu | the machine dialog of `ambre-spec-machine-access.md` |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Bots as sealed clients | a bot API on the relay | the relay would read what the bot reads, which breaks the zero-knowledge pillar |
| A slash command is addressed | a slash command posted in the channel | every member would read commands meant for one bot |
| A member merges a webhook | a bridge that holds the channel key | a third-party server would read the whole channel to post one alert |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No bot exists.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What a machine is, and how it is added and removed | `ambre-spec-machine-access.md` |
| Scripts that run inside a document | `ambre-spec-macros.md` |
