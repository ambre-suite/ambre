# AMBRE -- FEATURE: THE ORGANIZATION, AND THE ESCROW

**Type:** Horizontal (framework)
**Implementation:** partial -- the organization, its roles and the release path all ship. Nothing
ever deposits an escrowed key, and the quorum is clamped to one.

> A company that loses an employee must not lose that employee's documents. An organization holds a
> key of its own, and a member may leave a copy of their own seed sealed to it.

This is the one place where somebody other than a person can reach that person's documents. It is
therefore the one place where the zero-knowledge promise is deliberately narrowed. The narrowing is stated
here, in rows, rather than left as a property of a table.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-server.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-060` | `INV` | An organization MUST NOT be left without an owner. | this spec section 3 | `ambre_server::routes::orgs::tests::integration::owner_cannot_demote_last_owner`; `::owner_cannot_remove_last_owner`; `::owner_can_leave_when_another_owner_remains` -- the count is taken inside the same transaction as the change, so two concurrent departures cannot both pass it | none | verified |
| `SRV-066` | `INV` | The relay MUST NOT be able to combine a set of shares into a secret. | `ambre-spec-zero-knowledge.md`, `ZK-004`; `ambre-spec-server.md`, `SRV-012` | cargo-deny; crate-graph -- `deny.toml` refuses the secret-sharing crate in this process, and the continuous-integration `deny` job fails the manifest that declares it | none | verified |
| `SRV-067` | `INV` | A member's escrowed key MUST reach the relay already sealed to the organization. | `ZK-001`; this spec section 3 | not built | unbuilt -- lands with the `Split-secret organization recovery` Story (`ambre-roadmap.md` post-V1). No route deposits one. The register carries the column and the release reads it, and nothing anywhere writes it, so the release can only ever answer that there is nothing to release, section 5 | unimplemented |

### 1.2 Create an organization

The caller supplies a name, a public key and the quorum it wants its escrow to require.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-062` | `FAIL` | A declared quorum MUST NOT exceed the quorum the relay enforces. | audit `N1`; this spec section 3 | `ambre_server::routes::orgs::tests::shamir_threshold_above_1_rejected_until_reconstruction_lands`; `::shamir_threshold_1_accepted`; `::shamir_out_of_range_rejected` -- the three read the refusal, the one value accepted and the range | none | verified |

### 1.3 Change who is in the organization

An owner or an administrator adds a member, changes a role, or removes someone. Every clamp is on
the caller's own rank.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-061` | `FAIL` | A caller MUST NOT grant an organization role above its own. | audit `N2`; this spec section 3 | `ambre_server::routes::orgs::tests::admin_cannot_grant_owner`; `::org_role_rank_ordering`; `::integration::admin_cannot_escalate_via_add_member` | none | verified |
| `SRV-071` | `FAIL` | A caller MUST NOT remove an organization member ranked above it. | audit `N2`; this spec section 3 | `ambre_server::routes::orgs::tests::integration::admin_cannot_remove_owner` -- an administrator asking for an owner's removal is refused | none | verified |
| `SRV-076` | `FAIL` | A caller MUST NOT change the role of an organization member ranked above it. | audit `N2`; this spec section 3 | `ambre_server::routes::orgs::tests::integration::an_administrator_cannot_change_an_owners_role` -- through the role route and the add upsert alike, and the owner keeps its rank | none | verified |

### 1.4 Release an escrowed key

An owner asks for a departed member's sealed seed. Nothing is reconstructed on the relay: it hands
over an opaque blob.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-064` | `POST` | Every release of an escrowed key MUST be recorded. | this spec section 3 | none | unheld -- the release writes one row naming the organization, the subject, the caller and the instant, before it answers; no test reads that row back, and nothing refuses a release whose record failed to write; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-065` | `POST` | A release MUST require the quorum the organization declared. | audit `N1`; this spec section 3 | not built | unbuilt -- lands with the `Split-secret organization recovery` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `SRV-077` | `FRAME` | A release MUST NOT change the subject's key material. | this spec section 3, the escrow as designed; derived | none | unheld -- the route reads the sealed column and writes only the record, so a release is a read of the escrow and never a re-issue of it. Nothing asserts the column is unchanged afterwards, and a release that rewrote it would take the member's own recovery away; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-063` | `FAIL` | Only an owner MUST be able to release a member's escrowed key. | this spec section 3; `SRV-062` | `ambre_server::routes::orgs::tests::integration::recovery_requires_auth` -- the caller with no session; `::only_an_owner_releases_an_escrowed_key` -- an administrator and a member are refused, and an owner passes the gate | none | verified |

* **Budget.** none. Nothing here is on a hot path: an organization is read when a person opens their
  settings, and a release is a rare deliberate act. No row of `ambre-spec-budgets.md` names one.
* **Boundary.** this is the one narrowing of the boundary `ambre-spec-zero-knowledge.md` draws. It
  narrows what a **person** can reach, never what the **relay** can. The escrowed key is sealed
  to the organization's own public key, stored opaque and handed to an owner who decrypts it
  elsewhere (`SRV-067`). `SRV-066` keeps the relay unable to combine anything.
* **Verification.** 7 `verified`, 2 `untested`, 2 `unimplemented`. What holds is the membership: the
  last owner, the grant and role clamps, the removal clamp, and who may ask for a release. What is
  loose is whether the release record survives, and the deposit, which no route performs.
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
| admin | yes | up to admin, and never above (`SRV-061`, `SRV-071`, `SRV-076`) | no | no |
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
deposits the result. The organization's private key is split into shares, one sealed to each holder.
Releasing a seed needs a quorum of holders to combine their shares outside the relay. The relay
stores both sealed things and can combine neither (`SRV-066`).

**The escrow, as built.** The deposit does not exist, and the quorum is one.

| Step of the design | Today |
|---|---|
| a member deposits their sealed seed | no route writes it (`SRV-067`) |
| the private key is split into shares | the column exists; no route computes a share |
| a quorum combines its shares | a single owner asks, and the relay releases (`SRV-063`, `SRV-065`) |
| the release is recorded | it is (`SRV-064`) |
| the recovered seed is re-sealed for the member | outside the relay, by whoever asked (`SRV-077`) |

**Why the declared quorum is clamped to one.** A stored quorum of three, with no reconstruction to
enforce it, is a guarantee the register advertises and nothing keeps. An auditor reading the row
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

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The escrow seals a member's own seed to the organization | the relay holding a recovery key | it would end the zero-knowledge pillar in one field: the relay would become the party that can read a document. The recovery key has to be somebody else's key and not a server key, and sealing to the organization keeps the relay exactly as blind as it is elsewhere (`SRV-067`, `ZK-004`) |
| The private key is split, rather than held by one person | one recovery holder per organization | recovery is the capability to read an employee's documents, so a single holder is a single point of compromise and a single point of loss. A quorum makes the act visible to several people by construction |
| A quorum the relay cannot enforce may not be declared | storing the intended quorum and enforcing one, with a note somewhere | the register is what an auditor reads, and a number in it that nothing keeps is worse than an absent feature: it invites the trust it does not deserve. Refusing the declaration keeps the two in step (`SRV-062`) |
| Every release is recorded before it answers | releasing without a record | it makes the exception invisible. The escrow is a designed exception to the pillar, so the only thing that makes it acceptable is that using it leaves a trace nobody involved can suppress (`SRV-064`) |
| The roles are ranked exactly as a document's are | a capability check per route | it is where the escalation paths came from on the document side. Two rank models is two places to get a clamp wrong, and the escalation the clamp exists to stop is the same one on both sides: the vocabulary differs because the objects differ, the comparison does not (`SRV-061`, `SRV-071`, and `ambre-spec-server.md`, `SRV-016`) |

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
  honest rather than safe. An owner alone can obtain any member's escrowed key, once a deposit
  exists (`SRV-063`, `SRV-065`).
* **The organization's public key is accepted unchecked.** The caller supplies it, and the relay
  decodes it without asking whether it is a key of the right kind or the right length. A malformed
  value is stored and only fails much later, in a client.
* **An organization is not bound to its members' documents.** The register carries a member's
  organization on the account row and no route sets it. Nothing links a document to an organization,
  and no rule states what an organization may see of one.
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
