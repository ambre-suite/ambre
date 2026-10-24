# AMBRE -- FEATURE: VERIFYING A RECIPIENT'S KEY

**Type:** Horizontal (framework)
**Implementation:** implemented -- the fingerprint, the two phases, the pin and both prompts ship.

> Before a document is shared with someone, their key is shown as a short readable fingerprint and
> the user confirms it. The key is remembered, and the next time it changes the share stops.

Sharing means wrapping the document's key to a public key the server handed us. A server that
substitutes that key reads everything shared afterwards, and nothing in the cryptography notices.
This is the step that makes a substitution visible to a person.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with the
other five documents of the DOM side.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-048` | `INV` | A fingerprint MUST be stable for a key. | this spec section 3; `ambre-spec-server.md` | `ambre_crypto::tests::fingerprint_is_stable_grouped_base32` -- the value is frozen for a known key, so any change to the tag, the hash variant, the truncation or the encoding fails | none | verified |
| `CHROME-071` | `INV` | A fingerprint MUST differ for any other key. | this spec section 3; `ambre-spec-server.md` | `ambre_crypto::tests::fingerprint_distinguishes_keys` -- a one-bit change moves the string | none | verified |
| `CHROME-072` | `OBS` | A fingerprint MUST be readable aloud. | this spec section 3 | `ambre_crypto::tests::fingerprint_is_stable_grouped_base32` -- its second assertion, the alphabet and the grouping into eight blocks of four being what a person reads over the phone | none | verified |
| `CHROME-049` | `INV` | The fingerprint MUST be computed where the keys are: the chrome MUST NOT hash anything. | this spec section 3; `ambre-spec-zero-knowledge.md` (`ZK-001`) | crate-graph -- neither the launcher nor the shared toolkit declares the crypto crate, so the string arrives already formatted; ci-gate -- `scripts/crate_boundaries.py` refuses the crypto crate in any manifest but the worker's, and the Logic crate's fixture-only one | none | verified |
| `CHROME-053` | `INV` | The stored key MUST survive a browser that already holds documents: adding it MUST NOT disturb them. | this spec section 3 | `ambre_logic::persistence::wasm_tests::an_upgrade_adds_trusted_keys_and_keeps_snapshots`; `::trusted_key_roundtrips_and_absent_is_none` -- wasm tier, a real browser database holding a snapshot is opened at the next version | none | verified |
| `CHROME-054` | `OBS` | The prompt MUST state, in plain words, that without out-of-band comparison the protection reduces to trusting the server. | this spec section 3; `ambre-spec-server.md` | `ambre_app::tofu::tests::the_prompt_states_its_limit_and_offers_refusal_first` -- the prompt mounted for a new and a changed key, the sentence read off the page | none | verified |
| `CHROME-077` | `OBS` | The prompt MUST offer the refusing action first. | this spec section 3, the prompt is a modal | `ambre_app::tofu::tests::the_prompt_states_its_limit_and_offers_refusal_first` -- the refusing action is the first of the two, for a new and a changed key | none | verified |

### 1.2 Invite someone

The first phase resolves the recipient, fetches their exchange key and computes the fingerprint. It
shares nothing.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-073` | `POST` | The first phase MUST answer with the recipient's fingerprint. | this spec section 3, the invitation in two phases | `ambre_sync::sharing::tests::fetch_invitee_key_emits_key_and_matching_fingerprint` -- the first phase driven over a scripted relay answers with the fetched key and its fingerprint, and posts nothing | none | verified |
| `CHROME-046` | `FRAME` | A document key MUST NOT be wrapped to a recipient before the trust decision is taken. | this spec section 3; `ambre-spec-zero-knowledge.md` | type-level -- the first phase's parameters carry no document key and no exchange key, so there is nothing in scope to wrap with, the wrap living in the second phase alone | none | verified |
| `CHROME-050` | `FAIL` | A key never seen before MUST interrupt: the invitation MUST NOT proceed until the user confirms. | this spec section 3 | `ambre_app::tofu::tests::a_new_or_changed_key_asks_and_only_the_pinned_key_invites` -- a key with no pin becomes a prompt, not an invitation; ci-gate -- `scripts/crate_boundaries.py` refuses an invitation built on the UI thread outside the file that holds the decision | none | verified |
| `CHROME-051` | `FAIL` | A key that differs from the stored one MUST block the invitation. | this spec section 3 | `ambre_app::tofu::tests::a_new_or_changed_key_asks_and_only_the_pinned_key_invites` -- a substituted key is classified as changed and becomes a prompt | none | verified |

### 1.3 Confirm the recipient's key

The user has looked at the fingerprint and accepted it. The key is pinned, then the second phase
wraps.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-052` | `POST` | The confirmed key MUST be stored before the invitation is dispatched. | this spec section 3 | `ambre_app::tofu::tests::a_confirmation_pins_before_it_dispatches` -- the pin and the dispatch recorded in the order they run | none | verified |
| `CHROME-074` | `POST` | The confirmed key MUST be stored under the recipient's own identity. | this spec section 3; this spec section 6 | `ambre_sync::sharing::tests::fetch_invitee_key_refuses_a_malformed_id` -- a malformed identifier fails the first phase, so no pin is filed under an invented one | none | verified |
| `CHROME-078` | `FRAME` | Confirming a key MUST NOT change any other recipient's pin. | derived | `ambre_logic::persistence::wasm_tests::a_repin_leaves_every_other_pin_as_it_was` -- wasm tier, two pins, one confirmed again, the other read back unchanged | none | verified |
| `CHROME-047` | `FAIL` | The wrap MUST be refused unless the key still equals the one the user confirmed. | this spec section 3; `ambre-spec-server.md` | `ambre_sync::sharing::tests::invite_pin_mismatch_refuses_and_never_wraps` -- the second phase served a substituted key answers a conflict, and posts no share | none | verified |

### 1.4 Refuse the recipient's key

Escape, the backdrop and the refusing action are three routes to one outcome.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-055` | `POST` | Refusing MUST store no key. | this spec section 3 | `ambre_app::tofu::tests::every_refusal_route_leaves_nothing_to_pin_or_send` -- Escape, the backdrop and the refusing action each leave nothing confirmed, and only a confirmation reaches the pin | none | verified |
| `CHROME-075` | `POST` | Refusing MUST send no invitation. | this spec section 3 | `ambre_app::tofu::tests::every_refusal_route_leaves_nothing_to_pin_or_send` -- each route leaves nothing confirmed, and only a confirmation reaches the dispatch | none | verified |
| `CHROME-076` | `FRAME` | Refusing MUST NOT leave a pending decision behind. | this spec section 3 | `ambre_app::tofu::tests::every_refusal_route_leaves_nothing_to_pin_or_send` -- each route clears the pending prompt | none | verified |

* **Budget.** none: one hash of 32 bytes, once per invitation, on the thread that already holds the
  keys. `ambre-spec-budgets.md` budgets no operation here.
* **Boundary.** this is the boundary. The confirmed key is stored on the client and never sent, and
  the fingerprint is public by construction. The invitation carries the expected key so the wrapping
  thread can refuse. The keys never leave the Sync thread (`ambre-spec-zero-knowledge.md`).
* **Verification.** 18 `verified`. The two phases run over a scripted relay. The classification and
  the confirmation are pure, and the prompt is mounted in the browser tier. A scan holds who
  hashes a key and who builds an invitation.

---

## 2. WHAT EACH USER SEES

The two peers here are the person sharing and the person invited; the shared state is the invitation
itself.

| The inviter does | The recipient does | What the server holds | What each sees | Test |
|---|---|---|---|---|
| invites someone for the first time | nothing yet | the recipient's public keys | the inviter sees a fingerprint and must confirm; the recipient sees nothing until it is sent | `ambre_crypto::tests::fingerprint_is_stable_grouped_base32` |
| confirms | nothing | the wrapped key, after the confirmation | the inviter sees the invitation sent; the recipient gains the document | `ambre_sync::sharing::tests::invite_request_wraps_for_the_invitee_only` |
| refuses | nothing | nothing new | the inviter is back where they started, with nothing stored | `ambre_app::tofu::tests::every_refusal_route_leaves_nothing_to_pin_or_send` |
| invites the same person again | nothing | the same keys | no interruption at all | `ambre_logic::persistence::wasm_tests::trusted_key_roundtrips_and_absent_is_none` |
| invites them after they reinstalled | reinstalled, so they hold new keys | new public keys | the inviter is blocked and must re-verify out of band | `ambre_app::tofu::tests::a_new_or_changed_key_asks_and_only_the_pinned_key_invites` |
| invites them while the server substitutes a key | nothing | a key that is not the recipient's | the inviter is blocked, with exactly the wording of the previous row | `ambre_sync::sharing::tests::invite_pin_mismatch_refuses_and_never_wraps` |

**The last two rows are indistinguishable, and that is the point.** The interface cannot tell a
reinstall from an interception. It says so, and asks the user to compare the fingerprint over a
route the server does not control, such as a call.

---

## 3. MODEL

**CRDT containers.** none. A confirmed key is local to this browser and travels with no document.

**Trust on first use, and what it actually buys.** There is no key directory to trust, so the first
key the server hands over is taken on faith and remembered. What that buys is not confidentiality
on the first share. It is that every share afterwards is compared against it, so a substitution has
to happen on the first contact or be seen.

| State | Condition | What the user gets |
|---|---|---|
| new | nothing stored for this recipient | the fingerprint, and the invitation waits for a decision |
| trusted | the stored key equals the fetched one | nothing: the invitation proceeds |
| changed | the stored key differs | a blocking alert, refusing by default |

**The fingerprint.** A hash of the public key, truncated and grouped so a person can read it over
the phone.

| Step | Value |
|---|---|
| input | a domain tag, then the 32 bytes of the exchange key |
| hash | BLAKE2b, the 512-bit variant |
| truncation | the first 20 bytes, which is 160 bits |
| encoding | base32, the standard alphabet, no padding |
| presentation | the 32 characters in 8 groups of 4, separated by spaces |

The domain tag separates this hash from every key derivation over the same input, so a fingerprint
can never collide with a derived key's digest. 160 bits is far beyond collision reach and still fits
one line. The value is computed on the Sync thread and carried up as a finished string
(`CHROME-049`).

**The invitation, in two phases.** The split is the whole design: a key is never wrapped to before a
person has looked at it.

1. the user names an invitee; the first phase resolves them, fetches their exchange key, computes
   the fingerprint, and answers. **Nothing is wrapped and nothing is shared** (`CHROME-046`);
2. the Main thread compares the fetched key with the stored one -- a comparison of bytes, not
   cryptography -- and classifies the state above;
3. trusted goes straight to the second phase; new and changed raise the prompt;
4. on confirmation, the key is stored, then the second phase is dispatched carrying it
   (`CHROME-052`, `CHROME-074`);
5. the second phase re-fetches the key and wraps only if it still matches, refusing with a conflict
   otherwise (`CHROME-047`).

**Step 5 is not redundant.** Without it a compromised server serves an honest key to the first phase
and a substituted one to the second. The confirmation the user gave would then be theatre. The
expected key travels with the invitation precisely so the thread that wraps can refuse.

**The prompt is a modal, and that is a rule rather than a style.** A toast dismisses itself after
seconds and is announced politely, so a user who looks away misses a possible interception. The
prompt stops the page and names the recipient. It shows the fingerprint with a copy action, states
the limit of the protection (`CHROME-054`), and offers refusal first. A legitimate cause for a
changed key exists: the recipient reinstalled and lost their keys. Re-confirming is therefore
possible, and it is deliberately not the easy path.

**What is pinned, and what is not.** The exchange key is the one that receives wrapped document
keys, so substituting it breaks confidentiality; it is pinned. The signing key authenticates
presence samples, where spoofing forges a cursor rather than a key; it is not.

**Code:** `crates/ambre-crypto/src/hash.rs` computes the fingerprint.
`crates/ambre-sync/src/sharing.rs` holds both phases and the comparison before the wrap, and
`crates/ambre-sync/src/members.rs` dispatches them. `crates/ambre-bus/src/rest.rs` carries the two
commands and the answer. `crates/ambre-app/src/tofu.rs` classifies the state, orders the pin before
the dispatch, builds the second phase and renders the prompt. `crates/ambre-app/src/lib.rs` reads
and writes the pin.
`crates/ambre-logic/src/persistence.rs` holds the store and its upgrade.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| an invitee is named in the share dialog | their key is fetched, and nothing is shared yet |
| the recipient is unknown to this browser | a modal shows their name and their fingerprint, and the invitation waits |
| the fingerprint's copy action | the string is on the clipboard, for a message or a call |
| the confirming action | the key is remembered and the invitation goes out |
| Escape, the backdrop, or the refusing action | nothing is remembered and nothing is sent |
| the same recipient is invited again | no interruption |
| the key differs from the remembered one | a modal states the divergence as an alert, refusing by default |
| the key changes between the two phases | the invitation fails with a conflict, and no key is wrapped |
| any modal here is open | the content surface is frozen behind it |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Trust on first use, with an out-of-band fingerprint | trusting the server's keys silently | it is what the absence of this step means. There is no key directory and no external authority, so the first key must be taken on faith by somebody; remembering it turns every later share into a comparison, which is the cheapest thing that makes substitution visible at all. |
| The invitation is two phases | showing the fingerprint after the share, as a notification | the key is wrapped by the time the fingerprint is displayed, so the confirmation asks about something that has already happened. Splitting the invitation is what makes the decision precede the act. |
| The expected key travels with the second phase | trusting that the key fetched twice is the same key | the server answers both phases, so it can serve an honest key to the one that is displayed and a substituted one to the one that wraps. Carrying the confirmed value makes the wrapping thread able to refuse, which is the only place the check can be trusted. |
| The key is stored before the invitation is dispatched | storing on the answer | it is later and can be lost: a dispatch that succeeds while the store write failed leaves a share sent under a key the next comparison does not know, which reads as a change and blocks the next legitimate share. The safe order is the pessimistic one. |
| A key change is a modal, never a toast | a toast | it auto-dismisses in seconds and is announced politely, so the one event the whole feature exists for is the one a user can blink and miss. A modal costs an interruption and buys the user's attention exactly when it matters. |
| A key change is a modal, never a toast | a badge in the share dialog | it is passive and appears only where the user was already heading, so a key change is reported to whoever happens to look. Nothing stops a share from being sent past it. |
| Refusal is the default action | confirming as the default, with refusal beside it | a changed key has a boring cause most of the time, which is precisely why the confirming action must not be the reflex. The user has to choose the unsafe branch deliberately. |
| The limit is stated in the prompt | a tooltip, a help page, or a footnote | trust on first use protects nothing at all unless the fingerprint is compared out of band, and a user who does not know that believes they are protected. A protection that overstates itself is worse than none, and all three of these are read after the decision or never. |
| Only the exchange key is pinned | pinning both keys | it doubles the prompts for a threat that forges a cursor. The exchange key is the one that receives wrapped document keys, so substituting it is what breaks confidentiality; the signing key authenticates presence samples, where a substitution reads nothing. |
| The fingerprint is computed where the keys are | computing it in the chrome from a key handed up | the thread that holds keys is the only crypto consumer, and it is what keeps every other thread free of a crypto dependency. The chrome receives a string it cannot have computed. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The fingerprint's construction has one home, and it is section 2 of this document.** Two other
  documents once carried a second one: a 256-bit BLAKE2b digest truncated to twenty bytes. The code
  hashes with the 512-bit variant, which produces a different string for the same key. The frozen
  value in the test is the 512-bit one. Nothing but review keeps a third description from appearing,
  because no script compares a stated construction with the one a test freezes.
* **Nothing exercises the flow end to end.** Each step is held on its own: the phases over a
  scripted relay, the decision as pure code, the prompt in the browser. No tier joins them through
  a real relay, a second user and the Main thread's relay loop.
* **Nothing rules on a recipient with several devices.** The exchange key is per user, so a second
  device is a key change today; whether it should be is undecided.
* **The store is per browser and never syncs.** A user who signs in from another machine has an
  empty set of confirmed keys, so every recipient is new again. There is no rule saying whether that
  is acceptable or a gap.
* **Nothing lets a user review or forget a confirmed key.** There is no list, no removal and no way
  to re-verify outside an invitation, so a mistaken confirmation is permanent for that browser.
* **A key transparency log is the answer this step approximates.** An append-only auditable log
  removes the first-contact hole entirely, and it is a design nobody has started.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a document key is wrapped | `ambre-spec-membership.md` |
| What a revocation rotates | `ambre-spec-key-rotation.md` |
| The endpoints that serve a user's public keys | `ambre-spec-server.md` |
| Which thread may hold a key at all | `ambre-spec-zero-knowledge.md` |
| The share dialog around this step: the member list, the roles, the revoke | `ambre-spec-chrome.md` |
| The modal itself, and the frame loop it pauses | `ambre-spec-chrome.md`, `CHROME-011` |
| Why an alert is a modal rather than a live region | `ambre-spec-accessibility.md` |
| The browser database this store shares with the snapshots | `ambre-spec-architecture.md section 2` |
| Who a peer is, and what a presence sample proves | `ambre-spec-presence.md` |
