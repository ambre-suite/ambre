# AMBRE -- FEATURE: CHAT IMPORT AND EXPORT
**Type:** Vertical (feature)

> A team moving from Slack or Discord brings its history along. The archive is read on the client,
> and the relay never sees a line of it in the clear. A member takes a channel out the same way, in
> one `.ambre` file.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-365` | `INV` | An import MUST run on the client: the archive MUST NOT reach the relay unsealed. | this spec section 3; `ZK-001` | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-366` | `INV` | An imported message MUST carry the importing member as its author, and the original author as its origin. | this spec section 3; `MURMURE-078` | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-367` | `OBS` | An imported message MUST show its original author and time, marked as imported. | this spec section 3 | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Import an archive

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-368` | `POST` | Importing an archive MUST create one private channel of the chosen space per channel of the archive. | this spec section 3; `MURMURE-048` | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-369` | `POST` | An import MUST write the history as final segments, in the order of the archive. | this spec section 3; `MURMURE-063` | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-370` | `POST` | An import MUST attach the files the archive holds, through the shared media flow. | this spec section 3; `MURMURE-132` | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-371` | `POST` | An import MUST end with a report that lists what it could not carry. | this spec section 3; `INTEROP-003`; `INTEROP-012` | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-372` | `FAIL` | Given an archive it cannot read, the import MUST stop before it creates anything. | this spec section 3 | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-373` | `FRAME` | An import MUST NOT change an existing channel. | this spec section 3 | not built | unbuilt -- lands with the `Chat import` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Export a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-438` | `POST` | An export MUST write every document of a channel the member reads into one `.ambre` file. | this spec section 3; `ambre-spec-docfile.md` | not built | unbuilt -- lands with the `Murmure export` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-439` | `FRAME` | An export MUST NOT change a document of the channel. | this spec section 3 | not built | unbuilt -- lands with the `Murmure export` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** an import writes final segments of the segment size of `ambre-spec-budgets.md
  section 6`, so a long history lands as whole segments, never as a tail of deltas.
* **Boundary.** the archive is read and sealed on the client (`MURMURE-365`). The relay receives
  sealed segments and sealed files, as for any channel.
* **Verification.** 11 `unimplemented`: no import exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | The archive holds | The space holds | Every member sees | Test |
|---|---|---|---|---|
| imports a Slack export into the space "Team" | three channels, two years of messages | three private channels | the old messages, each marked "imported from Slack", with its original author (`MURMURE-367`) | none |
| imports a Discord export with an unsupported embed | the embed | the messages, without the embed | a report that names the embed it skipped (`MURMURE-371`) | none |
| imports a truncated file | nothing readable | nothing new | an error, and no half-made channel (`MURMURE-372`) | none |

---

## 3. MODEL

**CRDT containers.** the `origin` of a message, in `ambre-spec-murmure-schema.md section 3`: the
original author's name and time.

**The importer signs.** The authors of the archive are not Ambre users and hold no key. So every
imported message is authored by the importing member, which the authorship check requires
(`MURMURE-366`, `MURMURE-078`). The origin names the original author, for display alone.

**The history lands final** (`MURMURE-369`). The grace period of `MURMURE-062` guards the edits of
a live message, which an imported one never had. The import is the only author of those snapshots,
so it confirms them itself (`AUTHOR-021`).

**Formats.** The Slack export and the JSON export of Discord, with the shared report
(`1-Foundations/ambre-spec-interop.md section 3`).

**The way out.** The container of `ambre-spec-docfile.md` holds one document, and a channel is
several. The `Murmure export` Story (`ambre-roadmap.md` E5) adds a container version that holds
a list. A segment the member cannot open stays out of the file.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| import, in the space menu, for a space admin | pick an archive, then the channels to bring |
| the end of an import | the report of what was carried and what was not |
| export, in the channel menu | one `.ambre` file of the channel, plaintext or sealed under a passphrase |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The importer signs, with an origin | messages signed under the original authors | those authors hold no key in Ambre, and a forged signature is what the authorship check exists to refuse |
| Final segments | replaying the history as live messages | two years of messages would flood every member's badges and pushes, and pile up deltas to compact |
| The client reads the archive | an import service on the relay | the relay would read every message of the archive |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No import exists.
* **Only two formats.** Teams, Mattermost and Rocket.Chat exports are not read.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a private channel is created | `features/ambre-spec-murmure-channels.md` |
| How a file is uploaded | `ambre-spec-media.md` |
| What a conversion report holds | `1-Foundations/ambre-spec-interop.md` |
