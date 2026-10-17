# AMBRE -- FEATURE: THE LAUNCHER

**Type:** Horizontal (framework)
**Implementation:** partial -- the development sign-in, listing, creating, renaming and deleting
ship, and so does the provider sign-in up to its tokens. An account has no face to unlock or set up
its keys. The list has no order, no grouping and no search.

> The home screen at the root of the site: sign in, then every document you can open, whatever kind
> it is. Picking one opens it; picking a kind creates one.

It hosts no document content, only chrome, which is why it belongs to no application. One view
lists a spreadsheet, a board and a page, and gains the next kind without a change.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with the
other five documents of the DOM side.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-035` | `INV` | The home screen MUST list every kind of document through one view: it MUST NOT belong to an application. | this spec section 2 | `ambre_app::landing::tests::creatable_types_round_trip_through_doc_path` -- every creatable kind walks one path builder and one parser, so a kind the view named separately would not round-trip | none | verified |
| `CHROME-038` | `INV` | The kind picker MUST offer exactly the kinds whose engine can be instantiated, naming no kind of its own. | this spec section 2 | `ambre_app::landing::tests::picker_lists_creatable_types_and_reports_pick` -- the picker mounted in a real document carries one entry per creatable kind in order under its generic label, and the pick is reported | none | verified |
| `CHROME-044` | `INV` | The home screen MUST perform no cryptography: a name MUST reach it already decrypted. | `ambre-spec-zero-knowledge.md` (`ZK-001`); `UI-THREAD-004` | cargo-deny -- `deny.toml` lets only `ambre-sync` and `ambre-logic` declare `ambre-crypto`, and only the crypto wrappers declare a cipher, so the page's crate cannot name one | none | verified |
| `CHROME-080` | `INV` | A composed name MUST NOT be stored. | this spec section 2; `ambre-spec-zero-knowledge.md` | type-level -- every composition returns a `ComposedName`, with no `Debug`, no `Display`, no `AsRef<str>` and no `Into<String>`, in a crate that takes no `serde`. Its four `compile_fail` doctests show a conversion refused, and its text reaches the tab title and a view alone; `ambre_chrome::composed::tests::only_a_missing_or_blank_name_is_the_placeholder`; ci-gate -- `scripts/crate_boundaries.py` refuses the placeholder spelled outside `composed.rs`, so no composition can be rebuilt as a `String` | none | verified |
| `CHROME-036` | `OBS` | A card MUST show the document's decrypted name. | this spec section 2; `ambre-spec-zero-knowledge.md` | `ambre_app::landing::tests::a_card_shows_the_name_or_the_placeholder` -- a named card mounted and read back | none | verified |
| `CHROME-079` | `OBS` | An unnamed document MUST show a placeholder -- never an identifier, a hash, or the kind's own word. | this spec section 2; `ambre-spec-zero-knowledge.md` | `ambre_app::landing::tests::a_card_shows_the_name_or_the_placeholder` -- an unnamed card reads the placeholder, which is no creatable kind's word | none | verified |
| `CHROME-043` | `OBS` | A context that carries no kind icon MUST compose the name with the generic kind label, at the point of use. | this spec section 2 | `ambre_chrome::composed::tests::a_tab_title_composes_the_name_with_the_kind_label` -- the one function the tab title is set from | none | verified |
| `CHROME-045` | `OBS` | The sign-in panel MUST show neither route until the server has answered which one applies. | this spec section 2 | `ambre_app::landing::tests::the_sign_in_waits_for_the_answer` -- the panel mounted with no answer shows neither route, then the answer shows its own | none | verified |
| `CHROME-099` | `OBS` | The recovery words MUST be shown once, at the setup, and be readable nowhere afterwards. | this spec section 2; `ambre-spec-zero-knowledge.md` section 3 | not built | unbuilt -- lands with the `A provider account is unlocked or set up` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.2 Resolve a path

The address bar hands over a string. It becomes the home screen or one document, and nothing else.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-040` | `POST` | A path MUST resolve to the home screen or to exactly one document. | this spec section 2 | `ambre_app::route::tests::root_is_landing`; `::each_doc_type_slug_parses`; `::path_doc_roundtrip`; `::landing_route_has_no_doc` | none | verified |
| `CHROME-082` | `FRAME` | Resolving a path MUST NOT open or close a document. | derived | type-level -- the parser is a pure function from a string to a route and holds no handle to a document, a thread or a surface, so a resolution with a side effect has nothing in scope to act on | none | verified |
| `CHROME-081` | `FAIL` | An unknown or malformed path MUST resolve to the home screen. | this spec section 2 | `ambre_app::route::tests::unknown_slug_falls_back_to_landing`; `::malformed_id_falls_back_to_landing`; `::trailing_and_extra_segments_tolerated` | none | verified |

### 1.3 Open a document, or go back home

The route changed. Either the booted engine can draw what was asked for, or the page starts again.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-039` | `POST` | An open whose render kind equals the booted one MUST swap in place. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-010` | `ambre_app::route::tests::cross_kind_open_needs_reload` -- the two of its four combinations where the kinds agree | none | verified |
| `CHROME-083` | `POST` | An open whose render kind differs from the booted one MUST reload the page. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-010` | `ambre_app::route::tests::cross_kind_open_needs_reload` -- the two combinations where they differ, one engine being booted per page, so this is the rule that keeps a board from being drawn by a grid | none | verified |
| `CHROME-042` | `FRAME` | Going back home MUST NOT unmount the content surface: it MUST be covered and frozen. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-010` | `ambre_canvas::frame::tests::a_closed_modal_resumes_only_what_it_suspended` -- a route away stops the loop, and a modal closing over it does not restart it; `ambre_canvas::frame::tests::a_paused_loop_relays_and_does_no_frame_work` -- a stopped loop does no frame work | none | verified |

### 1.4 Create a document

The user picks a kind. Nothing is named.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-037` | `POST` | A new document MUST be created unnamed. | this spec section 2; `ambre-spec-zero-knowledge.md` | ci-gate -- `scripts/crate_boundaries.py` refuses `CreateDocument`, the command that carries a name, in any UI-thread file, and refuses the open-time create unless it sends `name_enc: None` | none | verified |
| `CHROME-084` | `FRAME` | Creating a document MUST NOT store a placeholder as its name. | this spec section 2; `ambre-spec-zero-knowledge.md` | ci-gate -- the same gate as `CHROME-037`: the open-time create sends no name field at all, so no placeholder can ride it | none | verified |

### 1.5 Delete a document

There is no recovery, so the gesture is gated on the user reading the name back.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-041` | `FAIL` | A delete MUST be confirmed against the document's own name before anything is dispatched. | this spec section 2; README pillar "E2EE" | `ambre_app::landing::tests::a_delete_waits_for_the_confirmation_on_the_name` -- the card's delete dispatches nothing, the modal names the document, and only its confirm dispatches | none | verified |

### 1.6 Sign in, then unlock or set up

The provider says who the person is; a passphrase opens their keys, or a setup creates them.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-095` | `POST` | The provider control MUST hand the browser to the authorization address the relay answers. | this spec section 2; `ambre-spec-session.md`, `SRV-172` | `ambre_app::landing::tests::the_provider_control_asks_for_the_address` -- the control sends one request for the address and leaves the page where it is; `ambre_app::signin::tests::the_provider_address_is_followed_and_its_state_kept` -- the answered address is followed, and one that is not `https` is not; `ambre_sync::auth::provider_tests::a_provider_name_outside_its_alphabet_is_refused` | none | verified |
| `CHROME-096` | `POST` | A return from the provider MUST establish the session on the live threads, its tokens reaching the Sync worker alone. | this spec section 2; `ambre-spec-sync.md`, `SYNC-060` | `ambre_app::signin::tests::a_provider_return_is_read_from_its_address` -- the page reads the code and the state and hands them on; `ambre_sync::auth::provider_tests::a_provider_sign_in_keeps_its_tokens_on_the_worker` -- the worker stores both tokens, and what Logic hears names the person with no byte of a token | none | verified |
| `CHROME-101` | `FAIL` | A return from the provider MUST be refused unless it carries the state this tab was handed. | this spec section 2; `ambre-spec-session.md`, `SRV-022` | `ambre_app::signin::tests::a_return_this_tab_did_not_start_is_refused` | none | verified |
| `CHROME-097` | `POST` | An account that holds its keys MUST be asked for its passphrase before the list is shown. | this spec section 2; `ambre-spec-session.md` section 3 | not built | unbuilt -- lands with the `A provider account is unlocked or set up` Story (`ambre-roadmap.md` E6) | unimplemented |
| `CHROME-098` | `POST` | An account that owes its key setup MUST be shown the setup, never an error, whether the sign-in or a later refusal reveals it. | this spec section 2; `ambre-spec-sync.md`, `SYNC-134` | not built | unbuilt -- lands with the `A provider account is unlocked or set up` Story (`ambre-roadmap.md` E6) | unimplemented |
| `CHROME-100` | `FRAME` | A passphrase typed on this screen MUST NOT outlive its submission on the UI thread. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-010` | not built | unbuilt -- lands with the `A provider account is unlocked or set up` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** none of its own: the list is a handful of rows of DOM and `ambre-spec-budgets.md`
  budgets no chrome operation. The frame the home screen does not draw is a saving, charged to
  `ambre-spec-frame.md`.
* **Boundary.** the routing metadata the list shows is plaintext by design, and the name beside it
  is not: it arrives decrypted from the Sync thread (`CHROME-044`). The launcher opens one
  unauthenticated same-origin read of the sign-in configuration, keeps the sign-in's own state and
  owns the local store. Each is an exception `UI-THREAD-004` names.
* **Verification.** 20 `verified`, 4 `unimplemented`. What holds is what a pure function or one
  mounted control carries, and what a scan of the source refuses. Creation is held by the gate that
  keeps the named command off the page. A composed name is held by its type, which converts to no
  `String` (`CHROME-080`).
---

## 2. MODEL

**CRDT containers.** none. The launcher reads a list the Sync thread built and sends commands; it
opens no document.

**The faces, and the session that picks one.** The screen is a full-bleed overlay over the content
surface, and which face it shows is decided by the state of the session.

| Session | Face |
|---|---|
| unknown, and the server has not answered | a neutral placeholder (`CHROME-045`) |
| unknown, and the server reports a development mode | a name field that signs in without reloading the page |
| unknown, otherwise | a single button that hands the browser to the identity provider (`CHROME-095`) |
| signed in with the provider, keys stored | a passphrase field that opens them (`CHROME-097`) |
| signed in with the provider, keys owed | the setup: a passphrase, then the recovery words once (`CHROME-098`, `CHROME-099`) |
| known | the header, the kind picker and the document list |

**Why a provider sign-in is not yet a session.** The provider vouches for who the person is, and
the relay answers a token pair for that. Neither can open a key. A development sign-in derives its
keys from the name, so it skips both middle faces.

The return from the provider is exchanged on the Sync worker, never on this thread. The answer to
that exchange carries both tokens, and they live in that worker alone (`CHROME-096`). The page only
reads the code from its address, then replaces the address so the code leaves the history.

**Whose return it is.** The provider comes back to `/signin/<provider>`, which the deployment
registers as its redirect address. The relay checks that it issued the state, not that this browser
asked for it. So the tab keeps the state it was handed in its own session storage, and drops a
return that carries another (`CHROME-101`). Without it, a link forged by a third party signs the
person into the third party's account. The address the relay answers is followed only if it is
`https`: a `javascript:` one would run in this page.

The development route matters for a reason that is not convenience. It signs in on the live
threads, so the Sync worker keeps the keys it already holds instead of losing them to a page
reload.

**A card.** One row per document, and four things on it.

| Field | Source | Note |
|---|---|---|
| the kind tile | the document's kind | one monogram per kind, coloured. It stands in for an icon the icon pipeline does not build yet |
| the name | the decrypted title | absent means never named, never opened, or no key yet -- all three render the placeholder (`CHROME-036`) |
| the modification time | the server's own timestamp | relative wording, and routing metadata rather than content |
| the shared badge | more than one member | a member count is routing metadata too |

Each card carries an inline rename and a delete. The delete is irreversible: there is no
server-side recovery of a ciphertext whose keys are gone. It passes through a modal that names the
document first (`CHROME-041`).

**Opening one, and the one predicate that decides how.** The view a page boots with is fixed for the
life of that page. It can be handed another document, but not another kind of document.

1. the target path is built from the kind and the identifier;
2. one predicate compares the target's render kind with the booted one;
3. same kind -- the router pushes the path and the surface swaps the document in place, keeping the
   worker and its session;
4. different kind -- the browser navigates, and the page boots the right view.

The cards and the kind picker share that predicate, so creating and opening cannot disagree
(`CHROME-039`).

**Creating one.** The picker is derived from the list of kinds whose engine can actually be
instantiated. It never names a kind, and gains the next one for free (`CHROME-038`). Picking a
kind mints an identifier on the client and opens the document at once. There is no name prompt: the
prompt is friction on every single creation, and the honest default is an unnamed document.

**Opening one from a file.** The same two steps in the other order. The file is read first, because
its header names the application, and only then is a path built and navigated to. So this screen
holds the gesture and owns none of its rules. The header read, the migration, the refusals and the
hand-off through the local snapshot store all belong to `ambre-spec-docfile.md section 4`.

One thing is shared rather than restated: the predicate that says which kinds have an engine. The picker
offers those kinds and the file path refuses the others. A file cannot open a document the screen
would not have let the user create (`CHROME-038`).

**Naming, and why the default is nothing.**

| Rule | Reason |
|---|---|
| unnamed until the user renames | a stored placeholder is indistinguishable from a user who typed it, and "is this named yet" stops being answerable |
| no counter, such as a second document numbered two | counter state collides across offline devices, because each client holds its own decrypted list and no server has authority over a name |
| no kind word inside the name | the tile already carries the kind, and repeating it is what this design removes |
| a context with no tile composes at the point of use | the browser tab reads a name and a generic kind label together, composed on the spot and stored nowhere (`CHROME-043`) |

The generic label is the kind described plainly: a spreadsheet, a whiteboard, a document. It is
deliberately not the product name for that kind, which is what the tile carries.

**Code:** `crates/ambre-app/src/landing.rs` holds the overlay, the cards, the picker and the sign-in
panel. `crates/ambre-app/src/route.rs` holds the path mapping and the reload predicate.
`crates/ambre-app/src/lib.rs` holds the relay that fills the list, pauses the loop behind the
overlay and keeps the tab title in step. `crates/ambre-core/src/doc_type.rs` holds the kinds, their
monograms, their generic labels and the creatable list. The style is `crates/ambre-app/styles.css`.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| the site is opened at its root | the overlay covers a frozen content surface, and the sign-in panel or the list appears |
| the server has not answered yet | a neutral placeholder, and no sign-in control that might be the wrong one |
| a name is entered in the development panel | the session is established on the live threads, with no page reload |
| the identity provider button | the browser leaves for the provider and comes back signed in (`CHROME-095`, `CHROME-096`) |
| a return this tab did not start | it is dropped, its address cleared, and the sign-in panel shows again (`CHROME-101`) |
| a provider account holds its keys | a passphrase field opens them, then the list appears (`CHROME-097`) |
| a provider account owes its keys, or the relay refuses it for that later | the setup: a passphrase, then the recovery words, shown once (`CHROME-098`, `CHROME-099`) |
| the list is empty | a placeholder invites creating one, rather than a bare empty list |
| a card is clicked | the document opens, in place or after a reload depending on its kind |
| the kind picker is opened | one entry per creatable kind, each under its generic label |
| a kind is picked | a document of that kind opens immediately, unnamed |
| the open-a-file control is used | a picker accepts an `.ambre` file, and the document it carries opens -- in the application the file's own header names, not in the one the picker was clicked from. The rule and its refusals belong to `ambre-spec-docfile.md section 4`; what this screen owns is holding the gesture, because at the moment of picking no document is open and no engine is booted |
| a card is renamed inline | the new name is sent, encrypted by the Sync thread, and the card updates |
| a card's delete is chosen | a modal names the document and asks; confirming removes the card at once |
| a document is open and renamed from its header | the same path as the card's rename |
| the tab is looked at | the document's name and its generic kind label, or the product name on the home screen |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The home screen is app-agnostic and lives in the launcher | a home screen per application | it lists documents of every kind, so one per application means every application maintains a list of the others' documents and a new kind has to be added to each. |
| The home screen is app-agnostic and lives in the launcher | one owned by whichever application shipped first | it would make one application's crate the owner of the others' entry point. Kept in the launcher, a new kind appears in the list by being creatable. |
| A new document is unnamed, with no prompt | a name prompt on create | naming is the one thing the user cannot do usefully before there is content, and asking is friction on every single creation. An unnamed document is a decidable state that a rename resolves whenever the user is ready. |
| A new document is unnamed, with no prompt | a generated default name | it is indistinguishable from a name the user chose, so nothing downstream can tell an unnamed document from a named one, and the user is left deleting a title rather than writing one. |
| A placeholder is rendered, never stored | writing the placeholder into the stored name | a stored placeholder is a name as far as every other layer can tell: the ciphertext holds it, the list shows it, and nothing can distinguish it from a deliberate title. Rendering it keeps the absence of a name a fact rather than a guess. |
| No counters in a default name | numbering new documents per kind | there is no server authority over names, and each device decrypts its own list, so two offline devices both create the same numbered name. Deletions then leave holes the counter never reclaims. |
| A cross-kind open reloads the page | swapping the view at run time | it means every view is always linked and always mountable. The booted view is a closed choice made at page load, and swapping a document inside it cannot change which view is mounted: a reload is honest and costs a session the worker rebuilds, while a push that leaves the wrong view mounted is a blank document. |
| The content surface is covered, never unmounted | unmounting the canvas while the home screen is shown | it destroys the GPU surface, the offscreen pair and the atlases, and rebuilding them is the slowest thing the client does. Covering it and pausing the loop costs a paused frame. |
| The card shows an icon, a name and a date | encrypted thumbnails on the cards | a preview of an end-to-end encrypted document is a whole pipeline: an offscreen render on save, an invalidation policy across peers, an extra encrypted object per document, and a decrypt of every image when the list loads. The gain over a tile and a name is cosmetic. |
| The list shows no content statistics | counts of cells, words or pages on the cards | a modification time, a role and a member count are routing metadata the server already holds in plaintext and hands over for free. A word count or a cell count is content, so it must be computed on the client and stored encrypted -- a second coherent object to maintain, for a number. |
| The worker exchanges the provider's code | the page posting the code and handing the tokens on | the answer to that exchange carries both tokens. Posted from the page, they would sit on the UI thread, which `SYNC-060` forbids. The page reads the code from its address and passes it on |
| The tab keeps the state it was handed | trusting the relay's check of the state | the relay knows it issued the state, not to whom. A third party starts a sign-in of their own and sends the link back to the person, who is then signed in as the third party. |
| The kind tile is a monogram | waiting for the icon pipeline before shipping the list | the icon pipeline does not exist, and a monogram is a tile of text that needs nothing. It is a stand-in and it is legible about being one. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The list has no order, no grouping and no search.** Documents arrive in whatever order the
  server returned them, and a user with a hundred of them has a hundred rows.
* **Nothing rules on a failed sign-in.** A refused development name, an identity provider that
  returns an error and an expired session have no stated behaviour on this screen.
* **The delete is optimistic and unrolled.** The card is removed as soon as the confirmation is
  taken, and a delete the server refuses leaves the list wrong until the next listing.
* **A relative time never refreshes.** It is computed when the row renders, so a list left open
  keeps saying five minutes ago.
* **The kinds that cannot be created are still routable.** A path naming a kind with no engine
  parses to a document route, and what the page then does is unstated.
* **No suggested title exists.** Deriving one from the first content -- a heading, a first cell -- is
  per kind and belongs to each application rather than here.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a name is encrypted, and what the server sees of it | `ambre-spec-zero-knowledge.md`, `ambre-spec-sync.md` |
| The listing, rename, create and delete requests themselves | `ambre-spec-server.md` |
| What signing in establishes, and how a session is refreshed | `ambre-spec-session.md` |
| Inviting someone, and the trust step before the key is wrapped | `ambre-spec-key-verification.md` |
| The shell, the modal and the toasts the home screen is built from | `ambre-spec-chrome.md` |
| Why the modal traps focus, and what a screen reader is told | `ambre-spec-accessibility.md` |
| What a narrow window changes | `ambre-spec-adaptive-layout.md` |
| What a paused frame loop holds, and what a resume costs | `ambre-spec-frame.md` |
| What happens after the document is open | each application's feature documents |
