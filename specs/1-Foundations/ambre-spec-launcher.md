# AMBRE -- FEATURE: THE LAUNCHER

**Type:** Horizontal (framework)
**Implementation:** partial -- signing in, listing, creating, renaming and deleting all ship; the
list has no order, no grouping and no search.

> The home screen at the root of the site: sign in, then every document you can open, whatever kind
> it is. Picking one opens it; picking a kind creates one.

It hosts no document content, only chrome, which is why it belongs to no application: one view
lists a spreadsheet, a board and a page, and gains the next kind without a change.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with the
other five documents of the DOM side.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `CHROME-035` | The home screen MUST list every kind of document through one view: it MUST NOT belong to an application. | this spec section 2 | `ambre_app::landing::tests::creatable_types_round_trip_through_doc_path` walks every creatable kind through one path builder and one parser, so a kind the view named separately would not round-trip | verified |
| `CHROME-036` | A card MUST show the document's decrypted name, and an unnamed one MUST show a placeholder -- never an identifier, a hash, or the kind's own word. | this spec section 2; `ambre-spec-zero-knowledge.md` | none. The card renders the name or the placeholder and has no branch that reaches for an identifier. Nothing refuses the fallback a later author would add, which is how an identifier reaches a screen | unverified |
| `CHROME-037` | A new document MUST be created unnamed: a placeholder MUST NOT be stored as a name. | this spec section 2; `ambre-spec-zero-knowledge.md` | none. Creation sends no name at all, so there is nothing to store. `SYNC-067` holds the ciphertext half; nothing holds this half | unverified |
| `CHROME-038` | The kind picker MUST offer exactly the kinds whose engine can be instantiated, and MUST NOT name a kind of its own. | this spec section 2 | `ambre_app::landing::tests::picker_lists_creatable_types_and_reports_pick` mounts the picker in a real document, asserts one entry per creatable kind in order under its generic label, and asserts the pick is reported | verified |
| `CHROME-039` | Opening a document whose render kind differs from the booted one MUST reload the page; a same-kind open MUST swap in place. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-010` | `ambre_app::route::tests::cross_kind_open_needs_reload` covers all four combinations of the one predicate the cards and the picker share | verified |
| `CHROME-040` | A path MUST resolve to the home screen or to exactly one document; an unknown or malformed path MUST resolve to the home screen. | this spec section 2 | `ambre_app::route::tests::root_is_landing`, `::each_doc_type_slug_parses`, `::path_doc_roundtrip`, `::unknown_slug_falls_back_to_landing`, `::malformed_id_falls_back_to_landing`, `::trailing_and_extra_segments_tolerated`, `::landing_route_has_no_doc` | verified |
| `CHROME-041` | A delete MUST be confirmed against the document's own name before anything is dispatched. | this spec section 2; README pillar "E2EE" | none. The card stores the target and opens a modal that renders the name; the dispatch sits behind the confirming action. Nothing refuses a second delete path added without the modal, and there is no recovery from one | unverified |
| `CHROME-042` | The content surface MUST NOT be unmounted for the home screen: it MUST be covered and frozen. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-010` | none. The overlay covers the canvas cell and the route poll pauses the loop, so the GPU surface, the Logic thread and the Sync worker survive. Nothing observes that the loop is actually paused | unverified |
| `CHROME-043` | A context that carries no kind icon MUST compose the name with the generic kind label, at the point of use, and MUST NOT store the composition. | this spec section 2 | none. The tab title is recomputed each frame from the name and the label and written only when it changes, so nothing composed is ever persisted. Nothing refuses a composition sent to the server as a name | unverified |
| `CHROME-044` | The home screen MUST perform no cryptography: a name MUST reach it already decrypted. | `ambre-spec-zero-knowledge.md` (`ZK-001`); `UI-THREAD-004` | none. The launcher declares no crypto crate, and the names arrive on the display payload the Sync thread filled. The crate graph is the same fact `CHROME-001` reads, and no lint holds it here | unverified |
| `CHROME-045` | The sign-in panel MUST show neither route until the server has answered which one applies. | this spec section 2 | none. The panel renders a neutral placeholder while the answer is absent and branches once it arrives. Nothing refuses a default that would flash the wrong sign-in for one frame | unverified |

* **Budget.** none of its own. The list is a handful of rows of DOM and
  `ambre-spec-budgets.md` budgets no chrome operation. The frame the home screen does
  not draw is a saving, charged to `ambre-spec-frame.md`.
* **Boundary.** the routing metadata the list shows -- a modification time and a member count -- is
  plaintext by design (`ambre-spec-zero-knowledge.md`), and the name beside it is not: it
  arrives decrypted from the Sync thread (`CHROME-044`). The launcher itself opens one request, an
  unauthenticated same-origin read of the server's sign-in configuration, which carries no document
  data and no identity. `UI-THREAD-004` names one exception to the UI thread's ban and this is a
  second one, recorded in section 5.
* **Verification.** 4 `verified`, 7 `unverified`, 0 `unimplemented`. The verified four are the ones a
  pure function carries: the path mapping, the reload predicate and the kind list, all extracted out
  of the DOM and all covered. The unverified seven are properties of a rendered page against a live
  session, and the tier that mounts a page does not have one.

---

## 2. MODEL

**CRDT containers.** none. The launcher reads a list the Sync thread built and sends commands; it
opens no document.

**Two faces, one signal.** The screen is a full-bleed overlay over the content surface, and which
face it shows is decided by whether a session name is known.

| Session | Face |
|---|---|
| unknown, and the server has not answered | a neutral placeholder (`CHROME-045`) |
| unknown, and the server reports a development mode | a name field that signs in without reloading the page |
| unknown, otherwise | a single button that hands the browser to the identity provider |
| known | the header, the kind picker and the document list |

The development route matters for a reason that is not convenience: it signs in on the live threads,
so the Sync worker keeps the keys it already holds instead of losing them to a page reload.

**A card.** One row per document, and four things on it.

| Field | Source | Note |
|---|---|---|
| the kind tile | the document's kind | one monogram per kind, coloured. It stands in for an icon the icon pipeline does not build yet |
| the name | the decrypted title | absent means never named, never opened, or no key yet -- all three render the placeholder (`CHROME-036`) |
| the modification time | the server's own timestamp | relative wording, and routing metadata rather than content |
| the shared badge | more than one member | a member count is routing metadata too |

Each card carries an inline rename and a delete. The delete is irreversible -- there is no
server-side recovery of a ciphertext whose keys are gone -- so it passes through a modal that names
the document first (`CHROME-041`).

**Opening one, and the one predicate that decides how.** The view a page boots with is fixed for the
life of that page: it can be handed another document, but not another kind of document.

1. the target path is built from the kind and the identifier;
2. one predicate compares the target's render kind with the booted one;
3. same kind -- the router pushes the path and the surface swaps the document in place, keeping the
   worker and its session;
4. different kind -- the browser navigates, and the page boots the right view.

The cards and the kind picker share that predicate, so creating and opening cannot disagree
(`CHROME-039`).

**Creating one.** The picker is derived from the list of kinds whose engine can actually be
instantiated, so it never names a kind and gains the next one for free (`CHROME-038`). Picking a
kind mints an identifier on the client and opens the document at once. There is no name prompt: the
prompt is friction on every single creation, and the honest default is an unnamed document.

**Naming, and why the default is nothing.**

| Rule | Reason |
|---|---|
| unnamed until the user renames | a stored placeholder is indistinguishable from a user who typed it, and "is this named yet" stops being answerable |
| no counter, such as a second document numbered two | counter state collides across offline devices, because each client holds its own decrypted list and no server has authority over a name |
| no kind word inside the name | the tile already carries the kind, and repeating it is what this design removes |
| a context with no tile composes at the point of use | the browser tab reads a name and a generic kind label together, composed on the spot and stored nowhere (`CHROME-043`) |

The generic label is the kind described plainly -- a spreadsheet, a whiteboard, a document -- and it
is deliberately not the product name for that kind, which is what the tile carries.

**Code:** `crates/ambre-app/src/landing.rs` holds the overlay, the cards, the picker and the sign-in
panel; `crates/ambre-app/src/route.rs` holds the path mapping and the reload predicate;
`crates/ambre-app/src/lib.rs` holds the relay that fills the list, pauses the loop behind the
overlay and keeps the tab title in step; `crates/ambre-core/src/doc_type.rs` holds the kinds, their
monograms, their generic labels and the creatable list. The style is `crates/ambre-app/styles.css`.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| the site is opened at its root | the overlay covers a frozen content surface, and the sign-in panel or the list appears |
| the server has not answered yet | a neutral placeholder, and no sign-in control that might be the wrong one |
| a name is entered in the development panel | the session is established on the live threads, with no page reload |
| the identity provider button | the browser leaves for the provider and comes back signed in |
| the list is empty | a placeholder invites creating one, rather than a bare empty list |
| a card is clicked | the document opens, in place or after a reload depending on its kind |
| the kind picker is opened | one entry per creatable kind, each under its generic label |
| a kind is picked | a document of that kind opens immediately, unnamed |
| a card is renamed inline | the new name is sent, encrypted by the Sync thread, and the card updates |
| a card's delete is chosen | a modal names the document and asks; confirming removes the card at once |
| a document is open and renamed from its header | the same path as the card's rename |
| the tab is looked at | the document's name and its generic kind label, or the product name on the home screen |

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The home screen is app-agnostic and lives in the launcher | it lists documents of every kind and holds none of their content, so putting it in an application crate would make one application's crate the owner of the others' entry point. Kept where it is, a new kind appears in it by being creatable | a home screen per application, or one owned by whichever application shipped first |
| A new document is unnamed, with no prompt | naming is the one thing the user cannot do usefully before there is content, and asking is friction on every single creation. An unnamed document is a decidable state that a rename resolves whenever the user is ready | a name prompt on create, or a generated default name |
| A placeholder is rendered, never stored | a stored placeholder is a name as far as every other layer can tell: the ciphertext holds it, the list shows it, and nothing can distinguish it from a deliberate title. Rendering it keeps the absence of a name a fact rather than a guess | writing the placeholder into the stored name |
| No counters in a default name | there is no server authority over names, and each device decrypts its own list, so two offline devices both create the same numbered name. Deletions then leave holes the counter never reclaims | numbering new documents per kind |
| A cross-kind open reloads the page | the booted view is a closed choice made at page load, and swapping a document inside it cannot change which view is mounted. A reload is honest and costs a session the worker rebuilds; a push that leaves the wrong view mounted is a blank document | swapping the view at run time, which means every view is always linked and always mountable |
| The content surface is covered, never unmounted | unmounting it destroys the GPU surface, the offscreen pair and the atlases, and rebuilding them is the slowest thing the client does. Covering it and pausing the loop costs a paused frame | unmounting the canvas while the home screen is shown |
| The card shows an icon, a name and a date, and no preview | a preview of an end-to-end encrypted document is a whole pipeline: an offscreen render on save, an invalidation policy across peers, an extra encrypted object per document, and a decrypt of every image when the list loads. The gain over a tile and a name is cosmetic | encrypted thumbnails on the cards |
| The list shows no content statistics | a modification time, a role and a member count are routing metadata the server already holds in plaintext and hands over for free. A word count or a cell count is content, so it must be computed on the client and stored encrypted -- a second coherent object to maintain, for a number | counts of cells, words or pages on the cards |
| The kind tile is a monogram, not an icon | the icon pipeline does not exist, and a monogram is a tile of text that needs nothing. It is a stand-in and it is legible about being one | waiting for the icon pipeline before shipping the list |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The launcher opens a request `UI-THREAD-004` does not name.** It reads the server's sign-in
  configuration over an unauthenticated same-origin request, before any session exists. The rule as
  written names one exception, same-origin fonts, and its verification cell claims a single request
  in the whole client. There are two, and this is the second.
* **The launcher also owns the local store.** The snapshots and the pinned keys live in a browser
  database opened on this thread, because its callbacks need a live event loop
  (`ambre-spec-architecture.md section 2`). That is a persistence path on the UI thread, which is
  the other half of the same rule, and it is deliberate.
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
