# AMBRE -- FEATURE: SIGNING IN, AND THE TOKEN PAIR

**Type:** Horizontal (framework)
**Implementation:** partial -- the development sign-in, the account setup and the token pair all
ship. The provider sign-in is registered so that it cannot run.

> How a person proves who they are, and how the relay keeps believing it for the next fifteen
> minutes. Nothing here touches a document, and nothing here ever sees a passphrase.

An account is an address, a display name, and two public keys derived from a seed the relay only
ever holds sealed. This document owns the ways in, the pair of tokens a way in produces, and what
ends a session.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-server.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-028` | `INV` | An address MUST resolve to exactly one account. | audit `N5` | `ambre_server::db::users::tests::create_user_duplicate_email_is_conflict` -- the first of the two collisions; `::verified_email_collision_is_conflict` -- the second; `::get_user_by_email_is_case_insensitive` -- the comparison that decides what counts as the same address | none | verified |
| `SRV-029` | `INV` | A self-asserted address MUST NOT lock out the provider-verified holder of it. | audit `N3` | `ambre_server::db::users::tests::oidc_login_reclaims_email_from_unverified_squatter`; `::verified_email_cannot_be_squatted_by_setup`; `::oidc_relogin_keeps_its_own_email` | none | verified |
| `SRV-024` | `INV` | Any instance MUST accept a token another instance minted. | `ambre-spec-server.md`, `SRV-021` | none | unheld -- the signing key comes from one seed the deployment shares, and the process refuses to start without it outside development mode; nothing boots two instances to exchange a token; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-025` | `INV` | An access token MUST expire within fifteen minutes. | this spec section 3; `ZK-006` | `jwt::tests::expired_token_is_rejected` -- an expiry is enforced, never what it may be | partial -- the lifetime is one operational figure with a fifteen-minute default and no ceiling, so a deployment may set an hour with no code change, the same absence `ZK-008` states for a presigned link; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | partial |
| `SRV-027` | `INV` | A session MUST NOT be issued under a placeholder signing key. | this spec section 3; `SRV-007` | `ambre_server::routes::auth::tests::a_stored_key_that_does_not_parse_mints_no_session` -- a stored key that does not parse is refused, never replaced by a zero key | fails -- a new provider account is inserted with an all-zero key, and its first session is minted under it. The refresh then keeps that key after the real one is registered; lands with the `No session is minted under a fallback key` Story (`ambre-roadmap.md` E7) | failing |
| `SRV-031` | `INV` | An access token MUST NOT reach a log, a proxy record or a browser history. | `ZK-006`; audit `M15` | none | unheld -- the rule holds only outside this process: the relay's own request span records the path and never the query, and the token still travels in the socket address, so the reverse proxy must strip it and the client logs it (`ambre-spec-sync.md`, `SYNC-061`); lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 A request is authenticated

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-003` | `POST` | A route that needs a caller MUST verify the access token, its signature and its expiry. | this spec section 3 | `ambre_server::jwt::tests::expired_token_is_rejected` -- the refusal on the expiry; `::wrong_key_is_rejected` -- the refusal on the signature; `::mint_then_verify_roundtrips` -- the accepted case carries the identity through; `routes::tests::handler_tests::unauth_endpoints_return_401` -- one route's refusal end to end, one route and not the surface | none | verified |
| `SRV-090` | `FRAME` | The token check MUST NOT read a store: it is a local signature check and nothing else. | this spec section 3; `ambre-spec-budgets.md section 3` | none | unheld -- the extractor verifies the token and hands the claims on, and the ordering is held by the extractor running before the handler; nothing refuses a handler that queries first and authenticates afterwards. One route already takes an optional caller and reads first (`ambre-spec-server.md`, `SRV-018`); lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 A person signs in, and sets up their account

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-091` | `FRAME` | A sign-in MUST NOT carry a passphrase, nor anything the relay could derive a key from. | `ZK-003`; this spec section 3 | cargo-deny -- `argon2` is granted to `ambre-crypto` alone, and `ambre-server` declares neither, so the relay has no way to turn a secret into a key: what it stores is the sealed master material and the sealed salt, and it opens neither | none | verified |
| `SRV-023` | `FAIL` | The development sign-in MUST be refused unless the deployment enables it. | this spec section 3 | `ambre_server::routes::tests::handler_tests::dev_login_rejected_when_dev_mode_off` -- the refusal; `::dev_login_creates_user_and_returns_token` -- the enabled case still works, so neither end can rot into the other | none | verified |
| `SRV-030` | `FAIL` | The sealed master material MUST be written once; a later write MUST be refused. | this spec section 3 | `ambre_server::routes::tests::handler_tests::store_master_keys_is_first_time_only` -- the first upload succeeds and the second answers a conflict rather than overwriting, and the re-seal route is the passphrase path and touches neither public key | none | verified |
| `SRV-022` | `POST` | A sign-in with an identity provider MUST reach that provider. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_provider_route_reaches_its_provider` -- the shipped router hands the provider in the path to the handler, which names an unconfigured one in its refusal | partial -- no test runs the flow against a configured provider, so the exchange with it is held by nothing; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | partial |
| `SRV-074` | `FAIL` | A configured provider the relay cannot reach at startup MUST stop the start, never disable sign-in in silence. | audit `M7`; this spec section 3 | none | unheld -- the start logs the failure and exits when the credentials are configured, and warns and disables provider sign-in when they are absent; no test boots the relay against a provider that does not answer; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 A session ends

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-009` | `FAIL` | A refresh token MUST be consumable exactly once, including by two callers at the same instant. | this spec section 3; audit `M6` | `ambre_server::redis_ops::redis_integration_tests::refresh_consume_rejects_a_replayed_token` -- the second use fails; `::only_one_racing_caller_claims_a_refresh_token` -- exactly one of two concurrent claims wins, database tier | none | verified |
| `SRV-026` | `POST` | A sign-out MUST revoke the caller's own refresh token. | this spec section 3 | none | unheld -- `ambre_server::routes::tests::handler_tests::logout_clears_session` asserts the route answers 200 and never that the token stops working: a sign-out that revoked nothing would pass it; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-092` | `FRAME` | A sign-out MUST NOT revoke a token it did not issue to this caller. | this spec section 3 | none | unheld -- the route resolves the token to an identity and refuses one belonging to another, and no test presents a foreign token. A sign-out that over-reached would end a stranger's session from a request they never made; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** the provider callback of `ambre-spec-budgets.md section 3`, under 200 ms, which
  reads **none exists**. The token verification carries no budget: it is a local signature check on
  every request, and no row measures it.
* **Boundary.** the relay stores the sealed master material, the sealed salt and both public keys,
  and sees none of the three secrets behind them (`ZK-003`, `SRV-091`). What it mints is its own
  token, and `SRV-012` refuses it every other cryptographic act. The one leak stated here is the
  token in the socket address (`SRV-031`).
* **Verification.** 7 `verified`, 2 `partial`, 1 `failing`, 6 `untested`. What holds is what a
  request test can drive: the token gate, the address namespace, the one-shot key write and the
  refresh race. What is loose is every rule about a deployment rather than a request. The one
  failure is the placeholder key of a new provider account.
---

## 2. WHAT EACH USER SEES

Two people over one address namespace, which is the only state a session shares.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| signs up with an address she asserts | signs up with the same address | one account, Alice's | Bob is told the address is taken (`SRV-028`) | `db::users::tests::create_user_duplicate_email_is_conflict` |
| signs up asserting Bob's address | signs in with his provider, on that address | one account, Bob's | Alice loses the address; Bob signs in (`SRV-029`) | `db::users::tests::oidc_login_reclaims_email_from_unverified_squatter` |
| signs in with her provider | signs up asserting Alice's address | one account, Alice's | Bob is refused (`SRV-029`) | `db::users::tests::verified_email_cannot_be_squatted_by_setup` |
| renews her session twice with one refresh token | watches | one live refresh token | the second renewal is refused, and Alice signs in again (`SRV-009`) | `redis_ops::redis_integration_tests::refresh_consume_rejects_a_replayed_token` |
| signs out | holds Alice's refresh token | no refresh token for Alice | Bob's renewal is refused too (`SRV-026`) | `routes::tests::handler_tests::logout_clears_session` |

---

## 3. MODEL

**CRDT containers.** none. A session is relay state, and none of it converges.

**Three ways in, one outcome.** Each produces the same pair of tokens.

| Way in | Establishes the identity by | Available |
|---|---|---|
| an identity provider | the provider's own assertion, over a handshake with a proof of possession and a one-time value | in every deployment that configures one (`SRV-022`) |
| an account setup | the caller's own assertion of an address, plus the public keys it derived (`SRV-029`) | always |
| a development sign-in | a name, from which the client derives its keys with no secret at all | only where the deployment enables it (`SRV-023`) |

**The handshake with a provider.** The relay mints the state value and the one-time value. It keeps
them in the ephemeral store for ten minutes, and consumes them on the way back. A callback whose
state does not resolve is refused before any exchange. A provider that returns an address it has
not verified is refused after it.

**The pair.** Two tokens, two lifetimes, and only one of them revocable.

| Token | Form | Lifetime | Revoked by |
|---|---|---|---|
| the access token | signed by the relay, carrying the identity and its signing key, verified locally (`SRV-003`) | fifteen minutes (`SRV-025`) | nothing: it dies at its expiry |
| the refresh token | an opaque value, resolved in the ephemeral store | thirty days | a sign-out, or its own single use (`SRV-009`, `SRV-026`) |

**Why the access token carries the signing key.** The live write path verifies a signature per
frame. Reading the key from the token turns that into a local comparison. A lookup per frame would
instead put a query on the hottest path the relay has (`ambre-spec-broadcast.md`).

**A renewal is a rotation.** The claim of a refresh token is atomic, where read-then-delete would
let two callers holding one token both mint a session. A stolen token replayed after its owner used
it is therefore refused rather than honoured (`SRV-009`).

**What the relay stores of an account.**

| Field | Form on the relay |
|---|---|
| the address, and whether a provider verified it | plaintext (`SRV-028`, `SRV-029`) |
| the display name | plaintext, bounded in length |
| the master material, and the salt that derives its key | sealed by the client, opaque here (`ZK-003`) |
| the signing key and the exchange key | plaintext public halves |
| the provider and its subject, where a provider established the account | plaintext |

The master material is written once and refused afterwards. Overwriting the public keys would
leave every document key already sealed to the previous exchange key unopenable (`SRV-030`).
A passphrase change re-seals the material alone and moves neither public key.

**The socket's token.** A browser socket carries no header of ours. The upgrade therefore takes its
token in the address, and the relay verifies it exactly as it verifies a header. A refusal closes
with a
code of its own, so a client renews rather than backs off (`ambre-spec-broadcast.md`, `SRV-046`).

**Code.**

* `crates/ambre-server/src/routes/auth.rs` -- the three ways in, and the renewal;
* `crates/ambre-server/src/oidc.rs` -- the provider handshake and the claim checks;
* `crates/ambre-server/src/auth.rs` -- the extractors, and the one place a pair is minted;
* `crates/ambre-server/src/jwt.rs` -- the signing key, the claims and the verification;
* `crates/ambre-server/src/routes/keys.rs` -- the one-shot key write and the re-seal;
* `crates/ambre-server/src/db/users.rs` -- the address namespace and the reclaim rule.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the sign-in screen opens | it asks the relay which ways in this deployment offers (`ambre-spec-launcher.md`) |
| a person chooses their provider | the relay answers with an address to visit, and remembers the handshake for ten minutes |
| the provider returns | the relay exchanges the code, upserts the account and answers the pair |
| a person signs up with a passphrase | the client derives the keys, the relay stores the sealed material and answers the pair |
| a request is refused for an expired token | the client renews once and replays it (`ambre-spec-sync.md`, `SYNC-062`) |
| the socket is refused on authentication | the client renews and reopens, with no action from the person (`SYNC-063`) |
| a person signs out | the refresh token is revoked, and the access token dies at its own expiry |

What a person sees of all this is a sign-in screen and a session that survives a page reload. The
third thing is an authentication error asking them to sign in again. `ambre-spec-launcher.md` says
where each of those is drawn.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A signed token plus a revocable refresh token | a cookie session, which is what the relay carried until the two-instance split | the client's network belongs to a worker, and the browser attaches a cookie to neither its socket nor its worker requests reliably. A cookie is also an ambient credential, so every mutating route would need protection against a forged cross-site request -- a token the client attaches deliberately needs none |
| Revocation lives on the refresh token, not on the access token | a single stateless token with no refresh | nothing can revoke it before its expiry. A token verified locally is what removes a lookup from the write path, and a token that can be revoked cannot be verified locally: keeping the short one stateless and the long one revocable buys both, and bounds the loss to one lifetime |
| The signing key comes from one seed the deployment shares | a key per process | it works until the second instance: every instance must accept every other instance's token, and a per-process key makes a client's session depend on which instance answers. Absent the seed, the process refuses to start outside development mode rather than minting tokens no sibling can read (`SRV-024`) |
| The token carries the caller's signing key | reading the key from the register on every frame | the write path verifies a signature per frame, so a lookup there is a query per keystroke burst. The claim is minted from the register at sign-in, so it is as fresh as the session (`ambre-spec-broadcast.md`) |
| A provider-verified address wins over a self-asserted one | first claimant wins, which is what the namespace rule alone produced | the address is what a share resolves against, so letting anyone assert any address turns a signup form into a way to squat a person's identity and receive documents meant for them. The verified claimant reclaims it, and the unverified account keeps everything except the address (`SRV-029`) |
| The master material is written once | letting the write repeat | it reads like an idempotent upload and is not: a second write replaces both public keys, and every document key already sealed to the previous exchange key stops opening. The failure is silent and total -- the account loses every shared document it had |
| The development sign-in derives its keys from the name | a fixture account with a stored passphrase | it is a secret to store and keep in step for an account whose only requirement is to be the same on every run. Deriving from the name gives that with nothing stored, and the switch that enables it is off by default |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No test runs a provider exchange.** The provider routes reach their handler, and nothing stands
  in for a provider, so the exchange itself has never run under a test (`SRV-022`).
* **One provider is configured, and the corpus names four.** The deployment reads the credentials of
  a single provider.
* **A new provider account signs in under an all-zero key.** The account is inserted with a zero
  signing key until its setup registers the real one, and its first session is minted under that
  placeholder. The refresh keeps the key it was minted with, so the session outlives the setup
  under it. The column is unique, so a second new account collides with the first (`SRV-027`).
* **The token lifetime has a default and no ceiling.** Fifteen minutes is a figure of the
  deployment, and nothing refuses a day (`SRV-025`), which is the shape `ZK-008` states for the
  presigned link.
* **A sign-out revokes one token, and a person may hold several.** Each renewal mints a refresh
  token, and a sign-out revokes the one the client presents. A session opened on another device
  survives it, and nothing lists what a person holds.
* **The relay never expires an account.** Nothing removes an account, its sealed material or its
  public keys, and no route deletes one.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Where the passphrase is turned into a key, and by what | `ambre-spec-zero-knowledge.md`, `ZK-003` |
| Where the client keeps its two tokens, and what wipes them | `ambre-spec-sync.md`, `SYNC-060`, `SYNC-066` |
| What a sign-in screen shows, and what happens after it | `ambre-spec-launcher.md` |
| What a fingerprint of a public key is, and who compares one | `ambre-spec-key-verification.md` |
| Which key opens a document once a caller is known | `ambre-spec-membership.md` |
| The pipeline a verified caller then goes through | `ambre-spec-server.md` |
| The socket the token opens, and its close codes | `ambre-spec-broadcast.md` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
