# AMBRE -- FEATURE: USER SETTINGS

**Type:** Horizontal (framework)

> The choices one user makes for themselves, kept in one sealed document that follows them from
> device to device. A theme or a panel docked to the left on the laptop holds on the phone. The
> relay reads none of them.

This document owns the settings document and the key that seals it. It also owns where the relay
keeps it, how two devices merge their changes, and what a page shows before the unlock. What each
setting does is the rule of the document that owns its section.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `SETTINGS`, and it belongs to this document alone.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-001` | `INV` | The settings of a user MUST live in one document whose only member is that user. | this spec section 3, the store; `ambre-spec-membership.md` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-002` | `INV` | The relay MUST hold at most one settings document per user. | this spec section 3, the store | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-003` | `INV` | Each setting MUST belong to exactly one section, and each section to one owning document. | this spec section 3, the catalogue | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-004` | `INV` | A setting MUST be one last-writer-wins entry, keyed by what it configures. | this spec section 3, the merge | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-005` | `INV` | The settings document MUST stay pinned while its user is signed in. | this spec section 3, the store; `RESID-003` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-006` | `INV` | The settings document MUST stay within its byte budget. | this spec section 3, the store; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-009` | `CONC` | Two devices signing in for the first time at once MUST converge on one settings document. | this spec section 3, the store; `SETTINGS-002` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-016` | `CONC` | Two devices that change one setting, one of them offline, MUST converge on one value once both reach the relay. | this spec section 3, the merge; `ambre-spec-delta-order.md` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-017` | `INV` | The key of the settings document MUST be wrapped for the exchange key of its user, as the key of any document. | this spec section 3, the key; `ambre-spec-membership.md section 3` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-018` | `INV` | The settings document MUST declare its kind inside its sealed content. | this spec section 3, the store | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-019` | `INV` | Every setting MUST declare a default value. | this spec section 3, the catalogue | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-020` | `INV` | A setting whose meaning or type changes MUST take a new key, and never reuse the old one. | this spec section 3, the catalogue | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-021` | `INV` | A setting whose stored value a client cannot decode MUST read as its default on that client. | this spec section 3, the merge | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-022` | `INV` | The width of the context panel MUST stay on the device, outside the settings document. | this spec section 3, the catalogue; `ambre-spec-shell.md section 2` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-023` | `INV` | The theme MUST be a setting of the account. | this spec section 3, the catalogue; `ambre-spec-design-tokens.md section 2` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-024` | `INV` | The interface language MUST be a setting of the account. | this spec section 3, the catalogue; `ambre-spec-locale.md section 3` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-025` | `INV` | The side of the context panel MUST be a setting of the account. | this spec section 3, the catalogue; `CHROME-194` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-026` | `INV` | The fold of the context panel MUST be a setting of the account, one value per application. | this spec section 3, the catalogue; `CHROME-196` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-027` | `INV` | The number convention of a reader and their own locale MUST be settings of the account. | this spec section 3, the catalogue; `ambre-spec-locale.md section 3` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-028` | `INV` | The boot hint MUST carry the theme and the interface language, and no other setting. | this spec section 3, the boot | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |

### 1.2 Open the settings on a device

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-007` | `POST` | A client that unlocks its account MUST ask the relay for its settings document and open it like any document. | this spec section 3, the store | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-008` | `POST` | A user with no settings document MUST get one at the first unlock. | this spec section 3, the store | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-010` | `POST` | Until the settings document is loaded, every setting the boot hint does not carry MUST read as its default. | this spec section 3, the boot | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-029` | `POST` | Once the settings document is loaded, the client MUST write the boot hint from it. | this spec section 3, the boot | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-011` | `FRAME` | Opening the settings MUST NOT hold back the open of another document. | this spec section 3, the store | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-030` | `FAIL` | Given a settings document whose sealed kind is not the settings kind, the client MUST refuse it, write nothing into it and keep the defaults. | this spec section 3, the store; `SETTINGS-018` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-031` | `FAIL` | Given a device that already opened the settings document of its user, a relay answer naming another one MUST be refused, and the device MUST keep the one it opened. | this spec section 3, the store; `SETTINGS-002` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |

### 1.3 Boot a page before the unlock

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-032` | `POST` | A page with a boot hint MUST boot in the theme the hint names. | this spec section 3, the boot; `UI-THREAD-065` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-033` | `POST` | A page with a boot hint MUST boot in the interface language the hint names. | this spec section 3, the boot | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-034` | `FRAME` | Reading the boot hint MUST NOT wait on the network, on a key or on the local store. | this spec section 3, the boot | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |

### 1.4 Change a setting

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-012` | `POST` | A change MUST reach every other signed-in device of the user as a delta of the settings document. | this spec section 3, the merge | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-035` | `POST` | A change made while the socket is down MUST reach the relay once the socket is back. | this spec section 3, the merge; `RESID-019` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-036` | `POST` | Changes made within one coalescing delay MUST leave the device as one delta. | this spec section 3, the budget; `ambre-spec-budgets.md section 8` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-037` | `POST` | A change of the theme or of the interface language MUST rewrite the boot hint of the device. | this spec section 3, the boot | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-013` | `FRAME` | Changing a setting MUST NOT change a setting of another section. | this spec section 3, the catalogue; `SETTINGS-003` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-038` | `FRAME` | Changing a setting MUST NOT remove or rewrite a setting the client does not know. | this spec section 3, the merge; `SETTINGS-021` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |

### 1.5 Sign out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-014` | `POST` | Signing out MUST delete the local copy of the settings document. | this spec section 3, the store; `ambre-spec-residency.md` | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-039` | `POST` | Signing out MUST delete the boot hint. | this spec section 3, the boot | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `SETTINGS-015` | `FRAME` | Signing out MUST NOT delete the settings document on the relay. | this spec section 3, the store | not built | unbuilt -- lands with the `The settings of a user follow the account` Story (`ambre-roadmap.md` MVP) | unimplemented |

* **Budget.** the settings document row of `ambre-spec-budgets.md section 6`, and the coalescing
  delay of `ambre-spec-budgets.md section 8`. Both wait on a measure.
* **Boundary.** the settings document is sealed like any document (`ZK-001`), and no setting leaves
  in another field (`ZK-053`). The relay learns which document holds the settings, and the size and
  time of each delta (`ZK-054`).
* **Verification.** 39 `unimplemented`: no settings document exists. Every row lands with one
  Story.

---

## 2. WHAT EACH USER SEES

| Alice does | on | The settings document holds | What Alice sees | Test |
|---|---|---|---|---|
| docks the panel to the left in Prose | her laptop | the side `left` in the `shell` section | the panel on the left on her desktop too, in the three applications (`SETTINGS-012`, `SETTINGS-025`) | none |
| drags the panel wider | her laptop | nothing | the panel wider on the laptop alone (`SETTINGS-022`) | none |
| switches to the dark theme, then reloads | her laptop | the theme `dark` | the unlock screen dark, with no light frame first (`SETTINGS-032`) | none |
| folds the panel in Trame, offline on the train | her laptop | the fold of Trame, once the laptop is back online | the panel folded in Trame on her phone, open in Prose (`SETTINGS-016`, `SETTINGS-026`) | none |
| sets the theme to light on the phone, while the offline laptop sets it to dark | both | one theme, the same on both devices | one theme on both, whichever write wins (`SETTINGS-016`) | none |
| signs in on a new phone | the phone | nothing yet, on the phone | the system theme, then her settings once they load (`SETTINGS-010`) | none |
| signs in on two new devices at the same moment | both | one document | the same settings on both (`SETTINGS-009`) | none |
| signs out | her phone | her settings, on the relay | nothing left on the phone, and everything back at the next sign-in (`SETTINGS-014`, `SETTINGS-039`) | none |
| -- | -- | the relay points her new phone at a document of her own that is no settings document | the defaults, and that document unchanged (`SETTINGS-030`) | none |

---

## 3. MODEL

**The store: an ordinary document, with one member.** The settings take the document pipeline
whole: the sealed envelope, the sequencer, the snapshots, residency and the offline queue. The relay
records which document holds the settings of a user, once (`SETTINGS-002`). Two first unlocks at
once receive the same document (`SETTINGS-009`).

| What one settings document costs | Figure | Why it stays small |
|---|---|---|
| relay rows | one document, one member row, one log entry, one pointer on the account | the member row is a self-wrap, and the log holds the creation alone |
| keys | one document key | no member can be removed, so no rotation runs (`ambre-spec-key-rotation.md`) |
| relay work, per session | one subscription and one open | the open walks a log of one entry, and its first entry carries the user's own signature |
| a device signed out or lost | nothing | every device of a user holds the same Master Seed, so no device has a key to revoke |

**The creation, at the first unlock.** The client takes three steps:

1. It asks the relay for the settings document of its user (`SETTINGS-007`).
2. With none, it mints a document, its first log entry and its self-wrap (`SETTINGS-008`).
3. It sends all three and the pointer in one request. The relay stores them in one transaction when
   the account holds no pointer. Otherwise it stores nothing and answers the winner, which the
   client opens (`SETTINGS-009`).

A first unlock needs the relay, so the creation never runs offline.

**The kind is sealed.** The first delta writes the settings kind into the document (`SETTINGS-018`).
A relay could point a client at another document of that user, whose log the user also signed. The
sealed kind refuses it, and no setting is written into a Prose document (`SETTINGS-030`). A device
keeps the identifier it opened, and refuses an answer naming another (`SETTINGS-031`).

**The key.** The document key is random, and wrapped for the user's own exchange key, as any
document key is (`SETTINGS-017`). The exchange key derives from the Master Seed, so the account
derives the wrap and no new derivation domain exists (`ZK-009`).

| Event | What happens to the key |
|---|---|
| a passphrase change | nothing: the same seed is sealed again, and the exchange key does not move (`SYNC-251`) |
| a recovery | nothing: the words spell the same seed (`ZK-043`) |
| a new device | nothing: the device unlocks the same seed, then unwraps the same copy |
| a new exchange key of the user | the copy must be wrapped again, as for every document of the user |

**The merge.** Each section is a map, and each entry is one last-writer-wins value
(`SETTINGS-004`). Two devices changing two settings never race. Two devices changing one setting
converge on the write the CRDT orders last (`SETTINGS-016`). The order is a logical clock, so the
winner is the same on both devices. It is not always the later write by the wall clock.

**An unknown setting survives an older client.** A client reads the keys it knows and leaves the
others alone (`SETTINGS-038`). A value it cannot decode, such as a theme a later release added,
reads as the default on that client, and stays stored (`SETTINGS-021`).

**The boot.** The local copy of the settings document is sealed under a key of the session
(`RESID-025`). Before the unlock, no page can read it. So each device keeps a boot hint: the theme
and the interface language, in the clear, in the browser's local storage (`SETTINGS-028`).

1. The page reads the hint before the first frame, with no wait (`SETTINGS-034`).
2. With a hint, the page boots in its theme and its language (`SETTINGS-032`, `SETTINGS-033`).
3. With none, the page boots in the theme the system asks for (`UI-THREAD-065`).
4. Once the settings load, the client writes the hint from them (`SETTINGS-029`). Each later change
   of either setting writes it again (`SETTINGS-037`).

A kept session unlocks a reload with no passphrase (`ambre-spec-session.md`). That unlock still
ends after the first frame, so the hint serves it too.

The hint mirrors two settings of the account, and holds no choice of its own. It names no document
and no user. A sign-out deletes it with the local copy (`SETTINGS-039`). The panel shows only once
a document opens, which is after the unlock, so no panel setting enters the hint.

**The catalogue.**

| Setting | Section | Values | Default | Owner |
|---|---|---|---|---|
| theme | `shell` | `system`, `light`, `dark` | `system` | `ambre-spec-design-tokens.md` |
| side of the context panel | `shell` | `left`, `right` | `right` | `ambre-spec-shell.md` |
| fold of the context panel, one per application | `shell` | `open`, `folded` | `open` | `ambre-spec-shell.md` |
| interface language | `locale` | a BCP 47 tag | the browser's language at the first run | `ambre-spec-locale.md` |
| the reader's own locale | `locale` | a BCP 47 tag, or none | the browser's language at the first run | `ambre-spec-locale.md` |
| number convention | `locale` | `document`, `type-mine`, `all-mine` | `type-mine` | `ambre-spec-locale.md` |
| the Murmure layout and levels, the macro approvals, the personal dictionary | `murmure`, `macros`, `prose` | the rule of each owner | the rule of each owner | the owner of each section |

A theme at `system` follows the scheme of each device. So two devices of one account can show two
themes, each the one its system asks for.

**The width stays on the device.** A width in CSS pixels takes another share of each screen. So
300 px is a quarter of a laptop and a tenth of a large monitor. The width lives in the local store of the
device, as `ambre-spec-shell.md` rules (`SETTINGS-022`).

**A new setting.** An owner adds one declaration: a section, a key, a type and a default
(`SETTINGS-019`). A key is frozen. A new meaning or a new type takes a new key (`SETTINGS-020`).

```rust
pub enum SettingsSection { Shell, Locale, Murmure, Macros, Prose }
pub enum DocumentKind { Settings }      // joins the kinds each application declares

/// One entry of the settings document. The key is frozen once released (`SETTINGS-020`).
pub trait Setting {
    const SECTION: SettingsSection;
    const KEY: StaticKey<Settings>;     // "panel.side": unique inside its section
    type Value: SettingValue;           // decodes to None on a value it does not know
    fn default_value() -> Self::Value;  // SETTINGS-019
}

pub enum Theme { System, Light, Dark }
pub enum PanelSide { Left, Right }
pub enum PanelFold { Open, Folded }

/// The two settings a page reads before the unlock (`SETTINGS-028`).
pub struct BootHint { pub theme: Theme, pub interface_language: Option<LanguageTag> }
```

**The budget.** The catalogue above weighs under 1 KB, against the row of
`ambre-spec-budgets.md section 6`. A burst of changes leaves as one delta per coalescing delay of
`ambre-spec-budgets.md section 8` (`SETTINGS-036`). A drag of the width writes nothing to the
relay, since the width stays on the device.

**The boundary.** The relay learns which document holds the settings of a user, from the pointer
it keeps. It learns the size and the instant of each delta, as for any document (`ZK-007`). A size
can tell which setting changed, since a client of known code writes it. That is the timing and size
metadata `ambre-spec-zero-knowledge.md` accepts (`ZK-054`).

**Code.** not built.

---

## 4. SURFACE

The settings document has no screen of its own. Each owner shows its settings where the user
changes them. The theme sits in the avatar menu, and the side and the fold on the panel.

| Trigger | Behaviour |
|---|---|
| a page boots | it takes the theme and the language of the boot hint, or the system theme without one |
| the account unlocks | the settings open from the local copy, then from the relay, and replace the defaults |
| a setting changed on one device | the other signed-in devices show it at their next delta |
| a setting changed offline | the change waits in the local store and leaves once the socket is back |
| a sign-out | the local copy and the boot hint go, and the relay copy stays |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| An ordinary document with one member | a sealed blob per account, replaced by compare-and-swap | it needs a route, a table and a seal of its own, and a merge of two offline writes the client codes again by hand. No other device learns of a change until it asks. The document pipeline already holds the seal, the order, the offline queue and the push, for one member row and one log entry |
| An ordinary document with one member | the settings in the clear on the account record | the relay would read every setting, which `ZK-053` refuses |
| An ordinary document with one member | a key derived from the Master Seed for the settings alone | a third derivation domain beside the two `ZK-009` freezes, for a document the existing wrap already opens |
| A pointer the relay keeps | a document identifier derived from the Master Seed | a third derivation domain again, and an identifier derived from a secret where every other is a UUID v7. It hides nothing: a document with one member, opened first at every unlock, names itself |
| One document per user | one document per application | each application would find, create and pin its own, and a new device would open several to show one screen |
| Settings in a document | the browser's local storage | a new device starts empty, and a new device is where the user needs the settings most |
| One section per owner | one flat map for every owner | two owners could write one key, and nothing would say which owns it |
| One last-writer-wins entry per setting | one value holding every setting | two devices changing two settings would lose one of the two changes |
| The order of the CRDT decides | a wall-clock instant in each value | the clocks of two devices differ, and a device whose clock runs ahead would win every race for as long as it runs ahead |
| A boot hint in the clear | the system theme until the unlock | each unlock screen would flash the wrong theme, in the wrong language, before the settings load |
| A boot hint in the clear | a sealed boot hint | no key exists before the unlock (`RESID-025`), so a sealed hint cannot be read when it is needed |
| A boot hint of two settings | every setting in the hint | the panel shows after the unlock, when the sealed copy opens. Each setting in the clear tells more to whoever holds the device |
| The width stays on the device | the width in the settings document | a width in pixels is a different share of each screen, so a width chosen on a laptop is wrong on a large monitor. A drag would also write to the relay at each release |
| One delta per coalescing delay | one delta per change | a user who folds and opens the panel five times writes five deltas, and the relay learns the rhythm of each click |
| Size and time of a delta are accepted | each settings delta padded to one size | the sealed envelope pads nothing today, so a padding is a change of the envelope for a few bits about appearance. `ZK-007` already admits the size of every delta |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No settings document exists. The theme is read from the system at each
  boot, and the panel stays on the device. The reader's own locale sits in the browser's local
  storage.
* **No control returns the theme to `system`.** The theme switch has two values. A third entry of
  the avatar menu, or a long press, is the owner's to rule (`ambre-spec-design-tokens.md`).
* **Nothing translates the interface yet.** The setting stores a language, and no command title
  reads it (`ambre-spec-locale.md section 6`).
* **No rule says what a new exchange key does to the settings.** A user's new exchange key needs a
  path for every document: `A member's new exchange key enters the log` Story (`ambre-roadmap.md` MVP).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What the theme changes on screen | `ambre-spec-design-tokens.md` |
| Where the panel docks, how it folds, and the bounds of its width | `ambre-spec-shell.md` |
| What the interface language, the reader's locale and the convention change | `ambre-spec-locale.md` |
| What the `murmure` section holds, and what each entry does | the Murmure feature specs |
| How a macro approval and a secret value are used | `ambre-spec-macros.md` |
| How the key of a document is wrapped for its member | `ambre-spec-membership.md` |
| How a document is held in memory and on disk | `ambre-spec-residency.md` |
| What the relay may learn of any document | `ambre-spec-zero-knowledge.md` |
