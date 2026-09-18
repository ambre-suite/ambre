# AMBRE -- FEATURE: VERIFYING A RECIPIENT'S KEY

**Type:** Horizontal (framework)
**Implementation:** partial -- the fingerprint, the two phases, the pin and both prompts ship. The
identity the pin is filed under is not always the recipient's.

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

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-048` | `INV` | A fingerprint MUST be stable for a key. | this spec section 3; `ambre-spec-server.md` | `ambre_crypto::tests::fingerprint_is_stable_grouped_base32` -- the value is frozen for a known key, so any change to the tag, the hash variant, the truncation or the encoding fails | none | verified |
| `CHROME-071` | `INV` | A fingerprint MUST differ for any other key. | this spec section 3; `ambre-spec-server.md` | `ambre_crypto::tests::fingerprint_distinguishes_keys` -- a one-bit change moves the string | none | verified |
| `CHROME-072` | `OBS` | A fingerprint MUST be readable aloud. | this spec section 3 | `ambre_crypto::tests::fingerprint_is_stable_grouped_base32` -- its second assertion, the alphabet and the grouping into eight blocks of four being what a person reads over the phone | none | verified |
| `CHROME-049` | `INV` | The fingerprint MUST be computed where the keys are: the chrome MUST NOT hash anything. | this spec section 3; `ambre-spec-zero-knowledge.md` (`ZK-001`) | crate-graph -- neither the launcher nor the shared toolkit declares the crypto crate, so the string arrives already formatted | partial -- no lint holds the next crate that would declare it; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-053` | `INV` | The stored key MUST survive a browser that already holds documents: adding it MUST NOT disturb them. | this spec section 3 | `ambre_logic::persistence::wasm_tests::upgrade_v1_to_v2_adds_trusted_keys_and_keeps_snapshots`; `::trusted_key_roundtrips_and_absent_is_none` -- wasm tier, a real browser database | none | verified |
| `CHROME-054` | `OBS` | The prompt MUST state, in plain words, that without out-of-band comparison the protection reduces to trusting the server. | this spec section 3; `ambre-spec-server.md` | none | unheld -- the sentence is in the prompt beside the fingerprint and a copy action, and it is not a footnote. Nothing refuses its removal, which is the whole honesty of the step; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-077` | `OBS` | The prompt MUST offer the refusing action first. | this spec section 3, the prompt is a modal | none | unheld -- the two actions sit in that order in one view, and re-confirming a changed key is deliberately not the easy path. Nothing refuses a reordering, which is the half `CHROME-051` was carrying; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 Invite someone

The first phase resolves the recipient, fetches their exchange key and computes the fingerprint. It
shares nothing.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-073` | `POST` | The first phase MUST answer with the recipient's fingerprint. | this spec section 3, the invitation in two phases | none | unheld -- the phase resolves, fetches and formats, and no test in any tier drives it: the mock relay does not model an invitation; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-046` | `FRAME` | A document key MUST NOT be wrapped to a recipient before the trust decision is taken. | this spec section 3; `ambre-spec-zero-knowledge.md` | type-level -- the first phase's parameters carry no document key and no exchange key, so there is nothing in scope to wrap with, the wrap living in the second phase alone | none | verified |
| `CHROME-050` | `FAIL` | A key never seen before MUST interrupt: the invitation MUST NOT proceed until the user confirms. | this spec section 3 | none | unheld -- the absence of a stored key raises the prompt and the dispatch sits behind the confirming action. Nothing asserts that no other path reaches the second phase; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-051` | `FAIL` | A key that differs from the stored one MUST block the invitation. | this spec section 3 | none | unheld -- the comparison classifies the state and the prompt carries the divergence as an alert. Nothing drives a substituted key through the classification; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Confirm the recipient's key

The user has looked at the fingerprint and accepted it. The key is pinned, then the second phase
wraps.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-052` | `POST` | The confirmed key MUST be stored before the invitation is dispatched. | this spec section 3 | none | unheld -- the store write and the dispatch sit in that order in one block, and a reversal leaves a share sent under a key the next comparison does not know; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-074` | `POST` | The confirmed key MUST be stored under the recipient's own identity. | this spec section 3; this spec section 6 | none | fails -- the first phase falls back to a freshly generated identifier when the server's is malformed, and the pin is filed under that. The recipient then looks new on every invitation, so a substituted key reads as a first contact and the whole comparison is dead; lands with the `A verified key is pinned under its owner` Story (`ambre-roadmap.md` E7) | failing |
| `CHROME-078` | `FRAME` | Confirming a key MUST NOT change any other recipient's pin. | derived | none | unheld -- the write addresses one identity, so the rule holds by the shape of the call. Nothing reads a second pin back across a confirmation, and `CHROME-074` is the reason to care: a wrong identity is already a write landing where it should not; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-047` | `FAIL` | The wrap MUST be refused unless the key still equals the one the user confirmed. | this spec section 3; `ambre-spec-server.md` | none | unheld -- the second phase re-fetches the key and compares it with the confirmed value before building the request, and answers a conflict otherwise. `ambre_sync::sharing::tests::invite_request_wraps_for_the_invitee_only` covers the wrap and not the comparison; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 Refuse the recipient's key

Escape, the backdrop and the refusing action are three routes to one outcome.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-055` | `POST` | Refusing MUST store no key. | this spec section 3 | none | unheld -- the three routes clear the pending decision and the store write sits behind the confirming action alone. Nothing asserts the three agree; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-075` | `POST` | Refusing MUST send no invitation. | this spec section 3 | none | unheld -- the dispatch reads the same pending value the three routes clear, so the two halves of the old row rest on one mechanism and neither is driven. This is the half that reaches the relay; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-076` | `FRAME` | Refusing MUST NOT leave a pending decision behind. | this spec section 3 | none | unheld -- a decision left behind is the next invitation dispatching against a recipient the user never looked at, and the three routes are the three places it could survive; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** none: one hash of 32 bytes, once per invitation, on the thread that already holds the
  keys. `ambre-spec-budgets.md` budgets no operation here.
* **Boundary.** this is the boundary. The confirmed key is stored on the client and never sent, and
  the fingerprint is public by construction. The invitation carries the expected key so the wrapping
  thread can refuse. The keys never leave the Sync thread (`ambre-spec-zero-knowledge.md`).
* **Verification.** 5 `verified`, 1 `failing`, 12 `untested`, 0 `unimplemented`. What holds is what
  a tier reaches without a session: a type-level absence, three assertions over the fingerprint, and
  a database upgrade. The rest is the flow itself, which needs a server, a second user and a
  substituted key to observe. The mock relay models no invitation.

---

## 2. WHAT EACH USER SEES

The two peers here are the person sharing and the person invited; the shared state is the invitation
itself.

| The inviter does | The recipient does | What the server holds | What each sees | Test |
|---|---|---|---|---|
| invites someone for the first time | nothing yet | the recipient's public keys | the inviter sees a fingerprint and must confirm; the recipient sees nothing until it is sent | `ambre_crypto::tests::fingerprint_is_stable_grouped_base32` |
| confirms | nothing | the wrapped key, after the confirmation | the inviter sees the invitation sent; the recipient gains the document | `ambre_sync::sharing::tests::invite_request_wraps_for_the_invitee_only` |
| refuses | nothing | nothing new | the inviter is back where they started, with nothing stored | none |
| invites the same person again | nothing | the same keys | no interruption at all | `ambre_logic::persistence::wasm_tests::trusted_key_roundtrips_and_absent_is_none` |
| invites them after they reinstalled | reinstalled, so they hold new keys | new public keys | the inviter is blocked and must re-verify out of band | none |
| invites them while the server substitutes a key | nothing | a key that is not the recipient's | the inviter is blocked, with exactly the wording of the previous row | none |

**The last two rows are indistinguishable, and that is the point.** The interface cannot tell a
reinstall from an interception. It says so, and asks the user to compare the fingerprint over a
channel the server does not control.

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
ephemeral presence frames, where spoofing forges a cursor rather than a key; it is not.

**Code:** `crates/ambre-crypto/src/hash.rs` computes the fingerprint.
`crates/ambre-sync/src/sharing.rs` holds both phases and the comparison before the wrap, and
`crates/ambre-sync/src/members.rs` dispatches them. `crates/ambre-bus/src/rest.rs` carries the two
commands and the answer. `crates/ambre-app/src/lib.rs` classifies the state and writes the pin, and
`crates/ambre-app/src/shell/app_root.rs` renders the prompt.
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
| Only the exchange key is pinned | pinning both keys | it doubles the prompts for a threat that forges a cursor. The exchange key is the one that receives wrapped document keys, so substituting it is what breaks confidentiality; the signing key authenticates presence frames, where a substitution reads nothing. |
| The fingerprint is computed where the keys are | computing it in the chrome from a key handed up | the thread that holds keys is the only crypto consumer, and it is what keeps every other thread free of a crypto dependency. The chrome receives a string it cannot have computed. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A malformed identity from the server becomes a fresh random one.** The first phase parses the
  recipient's identifier and falls back to a newly generated value when it cannot. The pin is filed
  under that value, so it matches nothing on the next invitation. The recipient looks new every
  time, and a substituted key looks like a first contact. `CHROME-074` states it, and states it as
  failing: the rule reads as one about identity and the damage it does is to the trust decision.
* **The fingerprint's construction has one home, and it is section 2 of this document.** Two other
  documents once carried a second one: a 256-bit BLAKE2b digest truncated to twenty bytes. The code
  hashes with the 512-bit variant, which produces a different string for the same key. The frozen
  value in the test is the 512-bit one. Nothing but review keeps a third description from appearing,
  because no script compares a stated construction with the one a test freezes.
* **Nothing exercises the flow end to end.** Three rows are covered by a hash, a type and a database.
  The comparison before the wrap, the classification of the three states and the three refusal
  routes have no test in any tier. The mock relay does not model an invitation.
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
| Who a peer is, and what a presence frame proves | `ambre-spec-presence.md` |
