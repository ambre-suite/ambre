# AMBRE -- FEATURE: THE ORGANIZATION, AND THE ESCROW

**Type:** Horizontal (framework)
**Implementation:** partial -- the organization, its roles and the release path all ship; nothing
ever deposits an escrowed key, and the quorum is clamped to one.

> A company that loses an employee must not lose that employee's documents. An organization holds a
> key of its own, and a member may leave a copy of their own seed sealed to it.

This is the one place where somebody other than a person can reach that person's documents, so it is
the one place where the zero-knowledge promise is deliberately narrowed. The narrowing is stated
here, in rows, rather than left as a property of a table.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-server.md` and the four other documents of this layer.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `SRV-060` | An organization MUST NOT be left without an owner. | this spec section 3 | `ambre_server::routes::orgs::tests::integration::owner_cannot_demote_last_owner`, `::owner_cannot_remove_last_owner`, `::owner_can_leave_when_another_owner_remains`. The count is taken inside the same transaction as the change, so two concurrent departures cannot both pass it | verified |
| `SRV-061` | A caller MUST NOT grant an organization role above its own. | audit `N2`; this spec section 3 | `ambre_server::routes::orgs::tests::admin_cannot_grant_owner`, `::org_role_rank_ordering`; `routes::orgs::tests::integration::admin_cannot_escalate_via_add_member` | verified |
| `SRV-071` | A caller MUST NOT change or remove an organization member ranked above it. | audit `N2`; this spec section 3 | `ambre_server::routes::orgs::tests::integration::admin_cannot_remove_owner`, `::update_member_role_rejects_invalid_role` | verified |
| `SRV-062` | A declared quorum MUST NOT exceed the quorum the relay enforces. | audit `N1`; this spec section 3 | `ambre_server::routes::orgs::tests::shamir_threshold_above_1_rejected_until_reconstruction_lands`, `::shamir_threshold_1_accepted`, `::shamir_out_of_range_rejected` | verified |
| `SRV-063` | Only an owner MUST be able to release a member's escrowed key. | this spec section 3; `SRV-062` | `ambre_server::routes::orgs::tests::integration::recovery_requires_auth` covers the caller with no session. Nothing drives an administrator or an ordinary member against the route, which is the refusal that matters while the quorum is one | unverified |
| `SRV-064` | Every release of an escrowed key MUST be recorded. | this spec section 3 | none. The release writes one row naming the organization, the subject, the caller and the instant, before it answers; no test reads that row back, and nothing refuses a release whose record failed to write | unverified |
| `SRV-065` | A release MUST require the quorum the organization declared. | audit `N1`; this spec section 3 | not built -- lands with the `Split-secret organization recovery` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `SRV-066` | The relay MUST NOT be able to combine a set of shares into a secret. | `ZK-004`; `SRV-012` | crate graph: `deny.toml` refuses the secret-sharing crate in this process, and the continuous-integration `deny` job fails the manifest that declares it. It was declared and unused until audit `C4` removed it | verified |
| `SRV-067` | A member's escrowed key MUST reach the relay already sealed to the organization. | `ZK-001`; this spec section 3 | none, and **no route deposits one**: the register carries the column and the release reads it, and nothing anywhere writes it, so the release can only ever answer that there is nothing to release -- section 5 | unverified |

* **Budget.** none. Nothing here is on a hot path: an organization is read when a person opens their
  settings, and a release is a rare deliberate act. No row of
  `ambre-spec-budgets.md` names one.
* **Boundary.** this is the one narrowing of the boundary
  `ambre-spec-zero-knowledge.md` draws, and it narrows what a **person** can reach, never
  what the **relay** can. The escrowed key is sealed to the organization's own public key, so the
  relay stores an opaque blob and hands it to an owner who decrypts it elsewhere (`SRV-067`).
  `SRV-066` keeps the relay unable to combine anything, and the release is recorded (`SRV-064`).
* **Verification.** 5 `verified`, 3 `unverified`, 1 `unimplemented`. What holds is the membership:
  the last owner, the rank clamps, and the refusal of a quorum the relay cannot keep. What is loose
  is the release itself -- who may ask for one, and whether the record survives -- and the deposit,
  which no route performs at all.

---

## 2. WHAT EACH USER SEES

Two people in one organization. The membership is the shared state.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| creates an organization | is added as an administrator | two members, one owner | Bob administers, and cannot promote himself (`SRV-061`) | `routes::orgs::tests::integration::admin_cannot_escalate_via_add_member` |
| is the only owner and demotes herself | administers | two members, unchanged | Alice is refused: the organization would have no owner (`SRV-060`) | `routes::orgs::tests::integration::owner_cannot_demote_last_owner` |
| administers | is the owner and Alice removes him | two members, unchanged | Alice is refused: she may not touch a member above her (`SRV-071`) | `routes::orgs::tests::integration::admin_cannot_remove_owner` |
| creates an organization declaring a quorum of three | watches | nothing | Alice is refused: the relay enforces a quorum of one (`SRV-062`) | `routes::orgs::tests::shamir_threshold_above_1_rejected_until_reconstruction_lands` |
| is the owner and asks for Bob's escrowed key | has left the company | one recovery record | Alice gets nothing: no deposit was ever made (`SRV-067`) | none -- section 5 |

---

## 3. MODEL

**CRDT containers.** none. An organization is relay state, and none of it converges.

**Three roles**, ranked exactly as a document's are, and clamped on both the grant and the target.

| Role | May read the member list | May add or remove | May change a role | May release an escrowed key |
|---|---|---|---|---|
| owner | yes | yes | yes | yes (`SRV-063`) |
| admin | yes | up to admin, and never above (`SRV-061`, `SRV-071`) | no | no |
| member | yes | no | no | no |

**What the register holds.**

| Field | Form on the relay |
|---|---|
| the organization's name | plaintext |
| the organization's public key | plaintext, supplied by the caller who creates it |
| the declared quorum, and the number of shares it is split into | plaintext (`SRV-062`) |
| one member row per participant, with a role | plaintext |
| a member's share of the organization's private key | sealed, opaque, and read by nothing |
| a member's own seed, sealed to the organization | sealed, opaque, and written by nothing (`SRV-067`) |
| one row per release | the organization, the subject, the caller, the instant (`SRV-064`) |

**The escrow, as designed.** A member seals their own seed to the organization's public key and
deposits the result. The organization's private key is split into shares, one sealed to each holder,
so releasing a seed needs a quorum of holders to combine their shares outside the relay. The relay
stores both sealed things and can combine neither (`SRV-066`).

**The escrow, as built.** The deposit does not exist, and the quorum is one.

| Step of the design | Today |
|---|---|
| a member deposits their sealed seed | no route writes it (`SRV-067`) |
| the private key is split into shares | the column exists; no route computes a share |
| a quorum combines its shares | a single owner asks, and the relay releases (`SRV-063`, `SRV-065`) |
| the release is recorded | it is (`SRV-064`) |
| the recovered seed is re-sealed for the member | outside the relay, by whoever asked |

**Why the declared quorum is clamped to one.** A stored quorum of three, with no reconstruction to
enforce it, is a guarantee the register advertises and nothing keeps: an auditor reading the row
would conclude three people are needed where one suffices. Refusing the declaration is what keeps
the data honest until the reconstruction lands (`SRV-062`, `SRV-065`).

**Why the release is owner-only rather than administrator-only.** With a quorum of one the release
is single-authority, so the narrowest role is the only defensible gate. When a quorum is really
enforced the gate stops being a role and becomes the count of shares presented.

**Code.**

* `crates/ambre-server/src/routes/orgs.rs` -- the organization, the roles, the clamps and the
  release;
* `crates/ambre-server/src/db/orgs.rs` -- the queries, and the last-owner count inside its
  transaction.

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The escrow seals a member's own seed to the organization, rather than storing a second copy of anything | the relay must not become the party that can read a document, so the recovery key has to be somebody else's key and not a server key. Sealing to the organization keeps the relay exactly as blind as it is elsewhere (`SRV-067`, `ZK-004`) | the relay holding a recovery key, which would end the zero-knowledge pillar in one field |
| The private key is split, rather than held by one person | recovery is the capability to read an employee's documents, so a single holder is a single point of compromise and a single point of loss. A quorum makes the act visible to several people by construction | one recovery holder per organization |
| A quorum the relay cannot enforce may not be declared | the register is what an auditor reads, and a number in it that nothing keeps is worse than an absent feature: it invites the trust it does not deserve. Refusing the declaration keeps the two in step (`SRV-062`) | storing the intended quorum and enforcing one, with a note somewhere |
| Every release is recorded before it answers | the escrow is a designed exception to the pillar, so the only thing that makes it acceptable is that using it leaves a trace nobody involved can suppress (`SRV-064`) | releasing without a record, which makes the exception invisible |
| The roles are ranked exactly as a document's are | two rank models is two places to get a clamp wrong, and the escalation the clamp exists to stop is the same one on both sides. The vocabulary differs because the objects differ; the comparison does not (`SRV-061`, `SRV-071`, and `ambre-spec-server.md`, `SRV-016`) | a capability check per route, which is where the escalation paths came from on the document side |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing deposits an escrowed key.** The register declares the column, the release reads it, and
  no route on the relay and no path in the client ever writes it. The whole recovery feature is
  therefore reachable and useless: an owner asking for a member's key is told there is none
  (`SRV-067`).
* **No share is ever computed.** The member row carries a sealed share and the only thing that ever
  writes one is a caller passing it in, which no client does. So the split the design rests on
  exists as two columns and a validation rule.
* **A single owner is the whole quorum.** Recovery is single-authority today, which the clamp makes
  honest rather than safe: an owner alone can obtain any member's escrowed key, once a deposit
  exists (`SRV-063`, `SRV-065`).
* **The organization's public key is accepted unchecked.** The caller supplies it, and the relay
  decodes it without asking whether it is a key of the right kind or the right length, so a
  malformed value is stored and only fails much later, in a client.
* **An organization is not bound to its members' documents.** The register carries a member's
  organization on the account row and no route sets it, so nothing links a document to an
  organization and no rule states what an organization may see of one.
* **Deleting an organization removes its members and its records.** The recovery record cascades
  with the organization, so the trace `SRV-064` exists for is removable by the party it exists to
  hold accountable.
* **The route group has no client.** No screen in the client creates an organization, lists one or
  asks for a release, so the whole surface is reachable by request alone.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Where a seed comes from, and what derives the keys under it | `ambre-spec-zero-knowledge.md` |
| How an account stores its own sealed material | `ambre-spec-session.md`, `SRV-030` |
| Which key opens a document, and who the members of one are | `ambre-spec-membership.md` |
| The roles of a document, which are not these | `ambre-spec-server.md` |
| What the relay may learn at all | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| Why the relay cannot combine anything | `ZK-004`, `SRV-012` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
