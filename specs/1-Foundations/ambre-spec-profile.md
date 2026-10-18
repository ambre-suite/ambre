# AMBRE -- FEATURE: THE USER PROFILE
**Type:** Horizontal (framework)
**Implementation:** partial -- the initials are drawn; no picture is held.

> The avatar that stands for a user in every application: a picture the user chooses, or their
> initials on their identity colour. The relay holds the picture in the clear, and serves it only
> to the users who share a document with its owner.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `PROFILE`, and it belongs
to this document alone.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROFILE-001` | `INV` | A user MUST have one avatar, the same in every application and every space. | this spec section 3 | not built | unbuilt -- lands with the `Avatars` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROFILE-002` | `OBS` | A user with no picture MUST show as their initials on their identity colour. | this spec section 3; `PRESENCE-006` | `ambre_chrome::avatar::tests::initials_come_from_the_first_two_words_or_letters` -- two words, one word and a blank name; `ambre_draw::peer::tests::the_css_colour_is_the_canvas_colour_in_hex` -- the avatar takes the cursor colour; `ambre_app::shell::murmure_regions::the_member_list_shows_each_member_by_name_initials_and_presence` -- browser tier, the computed background of a member's avatar is their identity colour | none | verified |

### 1.2 Set a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROFILE-003` | `POST` | Setting a picture MUST make it the avatar of the user wherever an avatar is drawn. | this spec section 3 | not built | unbuilt -- lands with the `Avatars` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROFILE-004` | `FRAME` | Setting a picture MUST NOT change the display name or any document. | this spec section 3 | not built | unbuilt -- lands with the `Avatars` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROFILE-005` | `FAIL` | Given a picture past the bound of `ambre-spec-budgets.md section 6`, the relay MUST refuse it and keep the previous avatar. | this spec section 3 | not built | unbuilt -- lands with the `Avatars` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.3 Remove the picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROFILE-006` | `POST` | Removing the picture MUST show the initials again. | this spec section 3 | not built | unbuilt -- lands with the `Avatars` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROFILE-007` | `POST` | Removing the picture MUST delete it from object storage. | this spec section 3; `ambre-spec-object-store.md` | not built | unbuilt -- lands with the `Avatars` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROFILE-008` | `FRAME` | Removing the picture MUST NOT change the display name. | this spec section 3 | not built | unbuilt -- lands with the `Avatars` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.4 Read an avatar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROFILE-009` | `FAIL` | Given a caller who shares no document with the owner, the relay MUST refuse the picture with the answer it gives for an unknown user. | this spec section 3; `SRV-005` | not built | unbuilt -- lands with the `Avatars` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** the avatar row of `ambre-spec-budgets.md section 6`, for each picture a view draws.
* **Boundary.** the relay holds the picture in the clear, an accepted leak `ZK-018` names. Only the
  users who share a document with the owner read it (`PROFILE-009`).
* **Verification.** 1 `verified`, 8 `unimplemented`. The initials are built, the picture is not.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| sets no picture | shares a space with her | no picture | "AL" on Alice's identity colour (`PROFILE-002`) | none |
| sets a picture | shares a document with her | the picture | her picture in Murmure, Prose, Trame and Toile (`PROFILE-001`) | none |
| sets a picture | shares nothing with her | the picture | nothing: the relay refuses it (`PROFILE-009`) | none |

---

## 3. MODEL

**CRDT containers.** none. The avatar is a relay record of the account, beside the display name
(`ambre-spec-session.md section 3`).

**The two forms of an avatar.**

| Form | When | Drawn as |
|---|---|---|
| initials | the user set no picture | the first letter of the first two words of the display name, or the first two letters of a single word, on the colour of `PRESENCE-006` |
| picture | the user set one | the picture, cropped to a square and masked to a circle |

**The picture.** The client crops and scales it to the square of `ambre-spec-budgets.md section 6`,
then uploads it through a presigned link (`ambre-spec-object-store.md`). The relay keeps one object
per user and replaces it on the next upload. A client caches each picture by the address the relay
serves, which changes with each upload.

**Who reads it.** The relay serves a picture to a caller who shares at least one document with its
owner. A space is a document (`MURMURE-031`), so every member of a space reads every avatar of it.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the account menu, "Set a picture" | a file picker, then a square crop |
| the account menu, "Remove the picture" | the initials come back |
| any view that draws a user | the avatar, at the size of the view |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The picture sits in the clear on the relay | a picture sealed under a profile key wrapped for each contact | each new contact and each removal needs a wrap or a rotation, a key flow as wide as the membership graph, for a picture the user chose to show |
| The picture sits in the clear on the relay | the picture of the identity provider, by its address | each view would tell the provider who looks at whom |
| One avatar for the whole product | one avatar per application or per space | a user would set the same picture four times, and a collaborator would not recognize them from one application to the next |
| Initials by default | a generic silhouette | a silhouette tells two users apart only by name, where initials on a colour do it at a glance |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No picture is built.** The account holds none, so every avatar shows initials.
* **A sealed profile is not ruled.** The display name and the picture in the clear are the accepted
  leak of `ZK-018`. Sealing both is the `Sealed profiles` Story (`ambre-roadmap.md` post-V1).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| the display name and the address of an account | `ambre-spec-session.md` |
| the identity colour of a user | `ambre-spec-presence.md` |
| the presigned links and the object store | `ambre-spec-object-store.md` |
| what the relay may learn of a user | `ambre-spec-zero-knowledge.md` |
