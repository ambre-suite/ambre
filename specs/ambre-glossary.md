# AMBRE -- THE GLOSSARY

**Type:** Register (every term of art, its gloss, and the synonyms it forbids)

> One row per term. The gloss names the idea; the reference holds the definition. `Forbidden` lists
> the words that mean the same thing and must not be written.

* **The gloss is not the definition.** It names an idea in a few words so a reader knows which term
  to reach for. The document in `Reference` defines it, and a reader who needs the rule goes there.
  A gloss that grows a condition, a figure or an RFC 2119 keyword
  has become a second specification, free to drift from the first. `scripts/prose.py` refuses one.
* **`Forbidden` is what a script holds.** `prose.py` reads this column and fails on any word it
  names. A frozen compound is written in parentheses and stays legal: `deny (not default-deny)`
  refuses the verb and allows the CORS policy.
* **A term earns a row by crossing documents.** A word used only in the document that defines it
  needs no row: the reader is already there.
* **The list grows by drift.** A synonym found in review becomes a `Forbidden` word here, and the
  gate holds it from that commit on. Nothing is listed ahead of a measurement.
* **This file is a register, not a specification.** It states no rule of its own, so it declares no
  contract table: `ambre-spec-template.md section 4` holds what a file of this class owes.

---

## 1. THE THREE THREADS, AND WHAT CROSSES THEM

| Term | Definition | Forbidden | Reference |
| :--- | :--- | :--- | :--- |
| thread | one of the three execution contexts of a client | | `1-Foundations/ambre-spec-architecture.md` |
| instance | one running copy of the application | | `1-Foundations/ambre-spec-architecture.md` |
| worker | a browser worker hosting a thread | | `1-Foundations/ambre-spec-architecture.md` |
| channel | a typed queue between two threads | | `1-Foundations/ambre-spec-architecture.md` |
| plane | a class of traffic on the bus | | `1-Foundations/ambre-spec-message-bus.md` |
| content plane | the plane carrying document edits | | `1-Foundations/ambre-spec-message-bus.md` |
| service plane | the plane carrying correlated requests and responses | | `1-Foundations/ambre-spec-message-bus.md` |
| envelope | the typed wrapper of one hop | | `1-Foundations/ambre-spec-message-bus.md` |
| role gate | the check of this client's role on a document | | `1-Foundations/ambre-spec-message-bus.md` |
| full channel | a channel with no room left | | `1-Foundations/ambre-spec-message-bus.md` |
| drop | throw a message or a command away | discard | `1-Foundations/ambre-spec-message-bus.md` |
| command | a user intent travelling to the Logic thread | | `1-Foundations/ambre-spec-commands.md` |
| chord | a key combination bound to a command | | `1-Foundations/ambre-spec-commands.md` |
| chrome | the interface around a document | | `1-Foundations/ambre-spec-chrome.md` |
| launcher | the surface that opens and lists documents | | `1-Foundations/ambre-spec-launcher.md` |
| view | the UI-thread object painting one document | | `1-Foundations/ambre-spec-ui.md` |

## 2. SYNC, AND THE DOCUMENT THAT CONVERGES

| Term | Definition | Forbidden | Reference |
| :--- | :--- | :--- | :--- |
| delta | one encrypted change to a document | | `1-Foundations/ambre-spec-delta-order.md` |
| delta order | the rule placing a delta against its peers | | `1-Foundations/ambre-spec-delta-order.md` |
| clock | the per-author counter carried by a delta | | `1-Foundations/ambre-spec-delta-order.md` |
| quarantine | the hold for a delta that cannot apply yet | | `1-Foundations/ambre-spec-delta-order.md` |
| snapshot | a full document state, encrypted and signed | | `1-Foundations/ambre-spec-delta-order.md` |
| sync | the thread owning the network and the crypto | | `1-Foundations/ambre-spec-sync.md` |
| relay | the server, which carries traffic it cannot read | | `1-Foundations/ambre-spec-zero-knowledge.md` |
| boundary | the line between what the relay learns and what it never learns | | `1-Foundations/ambre-spec-zero-knowledge.md` |
| peer | one participant of a document | | `1-Foundations/ambre-spec-presence.md` |
| presence | who is on a document, and where | | `1-Foundations/ambre-spec-presence.md` |
| ephemeral | a message kept by nothing after delivery | | `1-Foundations/ambre-spec-broadcast.md` |
| broadcast | one message sent to every subscriber | | `1-Foundations/ambre-spec-broadcast.md` |
| sequencer | the relay half that numbers deltas | | `1-Foundations/ambre-spec-sequencer.md` |
| seq | the number the sequencer assigns | | `1-Foundations/ambre-spec-sequencer.md` |
| membership | who may open a document, and as what | | `1-Foundations/ambre-spec-membership.md` |
| invite | the two-phase gesture adding a member | | `1-Foundations/ambre-spec-membership.md` |
| session | one authenticated connection of one client | | `1-Foundations/ambre-spec-session.md` |
| organization | the account a document and its members belong to | | `1-Foundations/ambre-spec-organizations.md` |
| blob | an opaque encrypted payload held outside a document | | `1-Foundations/ambre-spec-object-store.md` |
| media | a picture or a file placed in a document | | `1-Foundations/ambre-spec-media.md` |
| evict | choose a cache victim and remove it | | `1-Foundations/ambre-spec-drawing-surface.md` |
| refuse | turn an input away | deny (not default-deny) | `1-Foundations/ambre-spec-schema-version.md` |

## 3. THE CRYPTOGRAPHY, AND THE FILE

| Term | Definition | Forbidden | Reference |
| :--- | :--- | :--- | :--- |
| seal | encrypt, and bind the ciphertext to its context | | `1-Foundations/ambre-spec-sealed-envelope.md` |
| sealed envelope | the shape every encrypted message takes | | `1-Foundations/ambre-spec-sealed-envelope.md` |
| commitment tag | the tag binding a ciphertext to one key | | `1-Foundations/ambre-spec-sealed-envelope.md` |
| key hierarchy | the derivation tree of every key a client holds | | `1-Foundations/ambre-spec-zero-knowledge.md` |
| document key | the key that seals one document | | `1-Foundations/ambre-spec-zero-knowledge.md` |
| user key | the key derived from the account passphrase | | `1-Foundations/ambre-spec-zero-knowledge.md` |
| key rotation | replacing a document key, and what follows | | `1-Foundations/ambre-spec-key-rotation.md` |
| fingerprint | the readable digest of a public key | | `1-Foundations/ambre-spec-key-verification.md` |
| machine member | a program holding the document key, bound by its role | | `1-Foundations/ambre-spec-machine-access.md` |
| inbox | the write-only drop box of one document | | `1-Foundations/ambre-spec-machine-access.md` |
| inbox writer | a program that posts to an inbox and reads nothing | | `1-Foundations/ambre-spec-machine-access.md` |
| addressed key | the key that opens only the frames sent to one machine | | `1-Foundations/ambre-spec-machine-access.md` |
| macro | code stored in a document, run by a member client after approval | | `1-Foundations/ambre-spec-macros.md` |
| integrator | the member who put one version of a macro in one document | | `1-Foundations/ambre-spec-macros.md` |
| publisher | whoever wrote and signed a macro, member or not | | `1-Foundations/ambre-spec-macros.md` |
| container | the file wrapping a document on disk | | `1-Foundations/ambre-spec-docfile.md` |
| doc_type | the header byte naming the owning application | | `1-Foundations/ambre-spec-docfile.md` |
| magic | the four bytes opening every container | | `1-Foundations/ambre-spec-docfile.md` |
| sealed container | a container a passphrase opens | | `1-Foundations/ambre-spec-docfile.md` |
| commit | land a transaction | | `1-Foundations/ambre-spec-docfile.md` |
| schema version | the version of the CRDT shape inside a payload | | `1-Foundations/ambre-spec-schema-version.md` |
| migration | one forward step between two versions | | `1-Foundations/ambre-spec-schema-version.md` |

## 4. THE SCREEN

| Term | Definition | Forbidden | Reference |
| :--- | :--- | :--- | :--- |
| frame | one pass of the paint loop | | `1-Foundations/ambre-spec-frame.md` |
| repaint | painting a frame again | | `1-Foundations/ambre-spec-frame.md` |
| dirty flag | the mark saying a repaint is owed | | `1-Foundations/ambre-spec-frame.md` |
| drawing surface | the trait every painting target implements | | `1-Foundations/ambre-spec-drawing-surface.md` |
| back end | one implementation of the drawing surface | | `1-Foundations/ambre-spec-drawing-surface.md` |
| render asset | a font or a picture the painter needs | | `1-Foundations/ambre-spec-render-assets.md` |
| design token | a named value of the visual system | | `1-Foundations/ambre-spec-design-tokens.md` |
| palette | the set of colours one theme resolves to | | `1-Foundations/ambre-spec-design-tokens.md` |
| theme | the light or dark choice a palette answers | | `1-Foundations/ambre-spec-design-tokens.md` |
| zoom | the scale between document space and screen space | | `1-Foundations/ambre-spec-zoom.md` |
| preset ladder | the ordered zoom stops a gesture steps through | | `1-Foundations/ambre-spec-zoom.md` |
| undo | taking back the last change of this client | | `1-Foundations/ambre-spec-undo.md` |
| document locale | the language tag the content is formatted in | | `1-Foundations/ambre-spec-locale.md` |
| interface language | the language the reader's own menus are in | | `1-Foundations/ambre-spec-locale.md` |

## 5. THE METHOD

| Term | Definition | Forbidden | Reference |
| :--- | :--- | :--- | :--- |
| belt | the floor of tests an increment cannot land without | | `1-Foundations/ambre-spec-testing.md` |
| seam | the joint where a test reaches inside | | `1-Foundations/ambre-spec-testing.md` |
| tier | one of the three places a test runs | | `1-Foundations/ambre-spec-testing.md` |
| mechanism ladder | the ordered strengths a verification can claim | | `1-Foundations/ambre-spec-testing.md` |
| regime | the verification a component shape owes | | `1-Foundations/ambre-spec-testing.md` |
| fixture | a stored artifact a test reads instead of building | | `1-Foundations/ambre-spec-fixtures.md` |
| freeze | pin an artifact at a version | | `1-Foundations/ambre-spec-fixtures.md` |
| frozen artifact | a fixture pinned at an older version | | `1-Foundations/ambre-spec-fixtures.md` |
| budget | a performance or resource target with a row | | `1-Foundations/ambre-spec-budgets.md` |
| contract row | one falsifiable rule of a specification | | `ambre-spec-template.md` |
| invariant | a contract row holding at every instant | | `ambre-spec-template.md` |
| rejected alternative | an option weighed and set aside, in writing | | `ambre-spec-template.md` |
| reject | settle a pending suggestion against its author | | `4-Prose/features/ambre-spec-prose-track-changes.md` |
| story | one scheduled unit of work | | `ambre-roadmap.md` |
| epic | a group of stories | | `ambre-roadmap.md` |
