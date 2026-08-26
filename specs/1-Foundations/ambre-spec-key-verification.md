# AMBRE -- FEATURE: VERIFYING A RECIPIENT'S KEY

**Type:** Horizontal (framework)
**Implementation:** partial -- the fingerprint, the two phases, the pin and both prompts ship; the
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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `CHROME-046` | A document key MUST NOT be wrapped to a recipient before the trust decision is taken. | this spec section 2; `ambre-spec-zero-knowledge.md` | type-level: the first phase's parameters carry no document key and no exchange key, so there is nothing in scope to wrap with. The wrap lives in the second phase alone | verified |
| `CHROME-047` | The wrap MUST be refused unless the key still equals the one the user confirmed. | this spec section 2; `ambre-spec-server.md` | none. The second phase re-fetches the key and compares it with the confirmed value before building the request, and answers a conflict otherwise. `ambre_sync::sharing::tests::invite_request_wraps_for_the_invitee_only` covers the wrap and not the comparison | unverified |
| `CHROME-048` | A fingerprint MUST be stable for a key and MUST differ for any other: it MUST be readable aloud. | this spec section 2; `ambre-spec-server.md` | `ambre_crypto::tests::fingerprint_is_stable_grouped_base32` freezes the value for a known key and asserts the grouping and the alphabet; `::fingerprint_distinguishes_keys` asserts a one-bit change moves it | verified |
| `CHROME-049` | The fingerprint MUST be computed where the keys are: the chrome MUST NOT hash anything. | this spec section 2; `ambre-spec-zero-knowledge.md` (`ZK-001`) | crate graph: neither the launcher nor the shared toolkit declares the crypto crate, so the string arrives already formatted. No lint holds the next crate that would | unverified |
| `CHROME-050` | A key never seen before MUST interrupt: the invitation MUST NOT proceed until the user confirms. | this spec section 2 | none. The absence of a stored key raises the prompt and the dispatch sits behind the confirming action. Nothing asserts that no other path reaches the second phase | unverified |
| `CHROME-051` | A key that differs from the stored one MUST block, and the refusing action MUST be the default. | this spec section 2 | none. The prompt carries the divergence, states it as an alert, and offers refusal first. Nothing refuses a reordering of the two actions | unverified |
| `CHROME-052` | The confirmed key MUST be stored before the invitation is dispatched. | this spec section 2 | none. The store write and the dispatch sit in that order in one block. A reversal leaves a share sent under a key the next comparison does not know | unverified |
| `CHROME-053` | The stored key MUST survive a browser that already holds documents: adding it MUST NOT disturb them. | this spec section 2 | `ambre_logic::persistence::wasm_tests::upgrade_v1_to_v2_adds_trusted_keys_and_keeps_snapshots`, `::trusted_key_roundtrips_and_absent_is_none` -- wasm tier, a real browser database | verified |
| `CHROME-054` | The prompt MUST state, in plain words, that without out-of-band comparison the protection reduces to trusting the server. | this spec section 2; `ambre-spec-server.md` | none. The sentence is in the prompt beside the fingerprint and a copy action, and it is not a footnote. Nothing refuses its removal, which is the whole honesty of the step | unverified |
| `CHROME-055` | Refusing MUST leave nothing behind: no key stored, and no invitation sent. | this spec section 2 | none. Escape, the backdrop and the refusing action all clear the pending decision, and the dispatch reads that same value. Nothing asserts the three routes agree | unverified |

* **Budget.** none: one hash of 32 bytes, once per invitation, on the thread that already holds the
  keys. `ambre-spec-budgets.md` budgets no operation here.
* **Boundary.** this is the boundary. The confirmed key is stored on the client and never sent; the
  fingerprint is a hash of a public key, so it is public by construction; and the invitation carries
  the expected key so the wrapping thread can refuse. The keys never leave the Sync thread
  (`ambre-spec-zero-knowledge.md`), and the store on the Main thread holds public keys only.
* **Verification.** 3 `verified`, 7 `unverified`, 0 `unimplemented`. The three verified rules are the
  three a host or a browser tier can reach without a session: a type-level absence, a frozen
  fingerprint, and a database upgrade. The seven unverified rules are the flow itself, which needs a
  server, a second user and a substituted key to observe -- and the mock relay of
  `ambre-spec-sync.md` does not model an invitation (`ambre-spec-membership.md`).

---

## 2. MODEL

**CRDT containers.** none. A confirmed key is local to this browser and travels with no document.

**Trust on first use, and what it actually buys.** There is no key directory to trust, so the first
key the server hands over is taken on faith and remembered. What that buys is not confidentiality on
the first share; it is that every share afterwards is compared against it, so a substitution has to
happen on the first contact or be seen.

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
   (`CHROME-052`);
5. the second phase re-fetches the key and wraps only if it still matches, refusing with a conflict
   otherwise (`CHROME-047`).

**Step 5 is not redundant.** Without it a compromised server serves an honest key to the first phase
and a substituted one to the second, and the confirmation the user gave is theatre. The expected key
travels with the invitation precisely so the thread that wraps can refuse.

**The prompt is a modal, and that is a rule rather than a style.** A toast dismisses itself after
seconds and is announced politely, so a user who looks away misses a possible interception. The
prompt stops the page, names the recipient, shows the fingerprint with a copy action, states the
limit of the protection (`CHROME-054`), and offers refusal first. A legitimate cause for a changed
key exists -- the recipient reinstalled and lost their keys -- so re-confirming is possible, and it
is deliberately not the easy path.

**What is pinned, and what is not.** The exchange key is the one that receives wrapped document
keys, so substituting it breaks confidentiality; it is pinned. The signing key authenticates
ephemeral presence frames, where spoofing forges a cursor rather than a key; it is not.

**Code:** `crates/ambre-crypto/src/hash.rs` computes the fingerprint;
`crates/ambre-sync/src/sharing.rs` holds both phases and the comparison before the wrap;
`crates/ambre-sync/src/members.rs` dispatches them; `crates/ambre-bus/src/rest.rs` carries the two
commands and the answer; `crates/ambre-app/src/lib.rs` classifies the state and writes the pin;
`crates/ambre-app/src/shell/app_root.rs` renders the prompt; and
`crates/ambre-logic/src/persistence.rs` holds the store and its upgrade.

---

## 3. WHAT EACH USER SEES

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
reinstall from an interception, so it says so and asks the user to compare the fingerprint over a
channel the server does not control.

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Trust on first use, with an out-of-band fingerprint | there is no key directory and no external authority, so the first key must be taken on faith by somebody. Remembering it turns every later share into a comparison, which is the cheapest thing that makes substitution visible at all | trusting the server's keys silently, which is what the absence of this step means |
| The invitation is two phases | a single phase means the key is wrapped by the time the fingerprint is displayed, so the confirmation asks about something that has already happened. Splitting it is what makes the decision precede the act | showing the fingerprint after the share, as a notification |
| The expected key travels with the second phase | the server answers both phases, so it can serve an honest key to the one that is displayed and a substituted one to the one that wraps. Carrying the confirmed value makes the wrapping thread able to refuse, which is the only place the check can be trusted | trusting that the key fetched twice is the same key |
| The key is stored before the invitation is dispatched | a dispatch that succeeds while the store write failed leaves a share sent under a key the next comparison does not know, which reads as a change and blocks the next legitimate share. The safe order is the pessimistic one | storing on the answer, which is later and can be lost |
| A key change is a modal, never a toast | a toast auto-dismisses in seconds and is announced politely, so the one event the whole feature exists for is the one a user can blink and miss. A modal costs an interruption and buys the user's attention exactly when it matters | a toast, or a badge in the share dialog |
| Refusal is the default action | a changed key has a boring cause most of the time, which is precisely why the confirming action must not be the reflex. The user has to choose the unsafe branch deliberately | confirming as the default, with refusal beside it |
| The limit is stated in the prompt, not in documentation | trust on first use protects nothing at all unless the fingerprint is compared out of band, and a user who does not know that believes they are protected. A protection that overstates itself is worse than none | a tooltip, a help page, or a footnote |
| Only the exchange key is pinned | it is the key that receives wrapped document keys, so substituting it is what breaks confidentiality. The signing key authenticates presence frames, where a substitution forges a cursor and reads nothing | pinning both keys, which doubles the prompts for a threat that forges a cursor |
| The fingerprint is computed where the keys are | the thread that holds keys is the only crypto consumer, and it is what keeps every other thread free of a crypto dependency. The chrome receives a string it cannot have computed | computing it in the chrome from a key handed up |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A malformed identity from the server becomes a fresh random one.** The first phase parses the
  recipient's identifier and falls back to a newly generated value when it cannot. The pin is filed
  under that value, so it matches nothing on the next invitation: the recipient looks new every
  time, and a substituted key looks like a first contact. This is a defect and no row states it,
  because the rule it breaks is about identity rather than about trust.
* **The fingerprint's construction has one home, and it is section 2 of this document.** Two other
  documents once carried a second one -- a 256-bit BLAKE2b digest truncated to twenty bytes -- and
  the code hashes with the 512-bit variant, which produces a different string for the same key. The
  frozen value in the test is the 512-bit one. Nothing but review keeps a third description from
  appearing: no script compares a stated construction with the one a test freezes.
* **Nothing exercises the flow end to end.** Three rows are covered by a hash, a type and a database.
  The comparison before the wrap, the classification of the three states and the three refusal
  routes have no test in any tier, and the mock relay does not model an invitation.
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
