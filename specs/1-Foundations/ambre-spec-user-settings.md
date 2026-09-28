# AMBRE -- FEATURE: USER SETTINGS

**Type:** Horizontal (framework)
**Implementation:** design-only

> The choices one user makes for themselves, kept in one sealed document that follows them from
> device to device. A muted channel, a sidebar order or a macro approval set on the laptop holds on
> the phone.

This document owns the settings document, how a device finds it, and how its sections are shared out.
What each section holds is the rule of the application that owns it.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `SETTINGS`, and it belongs to this document alone.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-001` | `INV` | The settings of a user MUST live in one document whose only member is that user. | this spec section 3; `ambre-spec-membership.md` | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-002` | `INV` | The relay MUST hold at most one settings document per user. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-003` | `INV` | Each application MUST own one section of the settings document, and write no other. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-004` | `INV` | A setting MUST be one last-writer-wins entry, keyed by what it configures. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-005` | `INV` | The settings document MUST stay pinned while its user is signed in. | this spec section 3; `RESID-003` | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-006` | `INV` | The settings document MUST stay within its byte budget. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-009` | `CONC` | Two devices signing in for the first time at once MUST converge on one settings document. | this spec section 3; `SETTINGS-002` | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.2 Open the settings on a device

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-007` | `POST` | A signed-in client MUST ask the relay for its settings document and open it like any document. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-008` | `POST` | A user with no settings document MUST get one at the first sign-in. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-010` | `POST` | Until the settings document is loaded, every setting MUST read as its default. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-011` | `FRAME` | Opening the settings MUST NOT hold back the open of another document. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.3 Change a setting

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-012` | `POST` | A change MUST reach every other signed-in device of the user as a delta of the settings document. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-013` | `FRAME` | Changing a setting MUST NOT change the section of another application. | this spec section 3; `SETTINGS-003` | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.4 Sign out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-014` | `POST` | Signing out MUST delete the local copy of the settings document. | this spec section 3; `ambre-spec-residency.md` | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |
| `SETTINGS-015` | `FRAME` | Signing out MUST NOT delete the settings document on the relay. | this spec section 3 | not built | unbuilt -- lands with the `User settings` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** one row of `ambre-spec-budgets.md section 6`, the settings document, which the
  residency budget of the documents in memory also counts. It waits on a measure.
* **Boundary.** the settings document is sealed like any document (`ZK-001`). The relay learns
  which document holds the settings of a user, and nothing of what it holds. That is the membership
  metadata `ambre-spec-zero-knowledge.md` accepts, since the user is its only member.
* **Verification.** 15 `unimplemented`: no settings document exists. Every row lands with one
  Story.

---

## 2. WHAT EACH USER SEES

| Alice does | on | The settings document holds | What Alice sees | Test |
|---|---|---|---|---|
| mutes a channel | her laptop | the channel in the `murmure` section | the channel muted on her phone too (`SETTINGS-012`) | none |
| signs in on a new phone | the phone | nothing yet, on the phone | the defaults, then her settings once they load (`SETTINGS-010`) | none |
| signs in on two new devices at the same moment | both | one document | the same settings on both (`SETTINGS-009`) | none |
| signs out | her phone | her settings, on the relay | nothing left on the phone, and everything back at the next sign-in (`SETTINGS-014`) | none |

---

## 3. MODEL

**An ordinary document, with one member.** The settings take the document pipeline whole: the
sealed envelope, the sequencer, the snapshots, residency. Its key is wrapped for the user's own
exchange key, as any document key is (`SYNC-010`). So a new device opens it with nothing more than
the Master Seed, and no new key derivation exists.

**One per user.** The relay records which document holds the settings of a user, and creates the
record once (`SETTINGS-002`). Two first sign-ins at once receive the same document, as two first
replies receive one thread document (`SETTINGS-009`).

**One section per application.**

```rust
pub struct SettingsSection(String);  // the owning application: "murmure", "macros", "prose"
pub enum DocumentKind { Settings }   // joins the kinds each application declares
```

| Section | Holds | Owner |
|---|---|---|
| `murmure` | the layout, the notification levels, the bookmarks, the followed threads, the presence choice | `5-Murmure/` |
| `macros` | the approvals and the secret values of each macro | `ambre-spec-macros.md` |
| `prose` | the personal dictionary | `4-Prose/features/ambre-spec-prose-spell-check.md` |

Each section is a Map, and each entry is keyed by what it configures: a channel, a message, a macro,
a word (`SETTINGS-004`). A last-writer-wins value per key is what two devices editing one setting
need. Two devices editing two settings never race.

**Defaults first.** A setting absent from the document reads as its default (`SETTINGS-010`). A
device that has not loaded the settings yet shows the defaults, and every document opens without
waiting on them (`SETTINGS-011`).

**Code.** not built.

---

## 4. SURFACE

The settings document has no screen of its own. Each application shows its settings where the user
changes them: a channel menu, a macro prompt, a spelling suggestion.

| Trigger | Behaviour |
|---|---|
| a setting changed on one device | the other signed-in devices show it at their next delta |
| a sign-out | the local copy goes, and the relay copy stays |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| An ordinary document with one member | a key derived from the Master Seed for the settings alone | a third derivation domain beside the two `ZK-009` freezes, for a document the existing wrap already opens |
| One document per user | one document per application | each application would find, create and pin its own, and a new device would open several to show one screen |
| Settings in a document | the browser's local storage | a new device starts empty, and a new device is where the user needs the settings most |
| One section per application | one flat map for every application | two applications could write one key, and nothing would say which owns it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No settings document exists, and each application keeps its choices on the
  device or nowhere.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What the `murmure` section holds, and what each entry does | the Murmure feature specs |
| How a macro approval and a secret value are used | `ambre-spec-macros.md` |
| How the key of a document is wrapped for its member | `ambre-spec-membership.md` |
| How a document is held in memory and on disk | `ambre-spec-residency.md` |
