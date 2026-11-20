# AMBRE -- FEATURE: SIGNING IN, AND THE TOKEN PAIR

**Type:** Horizontal (framework)

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
| `SRV-024` | `INV` | Any instance MUST accept a token another instance minted. | `ambre-spec-server.md`, `SRV-021` | `ambre_server::routes::tests::handler_tests::a_session_crosses_instances_and_a_sign_out_ends_it_on_both` -- two instances on one seed, database tier: a token one mints opens a route on the other, and a pair the other renews opens a route on the first | none | verified |
| `SRV-025` | `INV` | An access token MUST expire within fifteen minutes. | this spec section 3; `ZK-006` | `jwt::tests::expired_token_is_rejected` -- an expiry is enforced; `ambre_server::routes::tests::handler_tests::an_access_token_never_outlives_fifteen_minutes` -- a configured day still mints a fifteen-minute token | none | verified |
| `SRV-235` | `INV` | The seed the access tokens are signed under MUST be wiped from memory when it is dropped. | this spec section 5; `ZK-006` | `jwt::tests::a_jwt_seed_is_wiped_and_never_printed` -- `JwtSeed` holds the `ed25519_dalek::SigningKey` in a `Box`, and the key wipes itself on drop (`ZK-019`). The hex text sits in a `SeedText`, is decoded inside `Secret::try_filled` and then wiped, so neither the text nor the seed is left behind, and a value that is not 32 bytes of hex is refused with a fixed message that quotes no character of it | none | verified |
| `SRV-236` | `INV` | The token signing seed MUST NOT reach a log. | this spec section 5; `ZK-006` | `jwt::tests::a_jwt_seed_is_wiped_and_never_printed` -- the `Debug` form of `JwtSeed` is `JwtSeed([REDACTED])`, with no byte and no length | none | verified |
| `SRV-027` | `INV` | A session MUST NOT be issued under a placeholder signing key. | this spec section 3; `SRV-007` | `ambre_server::routes::auth::tests::a_provider_account_without_keys_gets_no_signing_key`; `ambre_server::routes::auth::tests::a_stored_key_that_does_not_parse_mints_no_session`; `ambre_server::routes::tests::handler_tests::a_session_before_key_setup_carries_no_key`; `ambre_server::routes::tests::handler_tests::the_socket_refuses_a_session_without_keys` -- a provider account without keys gets a session with no key, which only the setup routes take and a refresh before the key store keeps keyless. The socket closes on it. A stored key that does not parse is refused | none | verified |
| `SRV-163` | `INV` | An account that has not registered its keys MUST NOT be offered as a recipient. | this spec section 3; `SRV-027` | `ambre_server::routes::tests::handler_tests::an_account_without_keys_is_offered_to_no_sharer`; `ambre_server::routes::tests::handler_tests::an_account_holds_both_keys_or_neither` -- the search, the key lookup and the invite pass over it until its keys are stored. A membership written before is left out of the list. The store refuses one key without the other | none | verified |
| `SRV-031` | `INV` | An access token MUST NOT reach a log, a proxy record or a browser history. | `ZK-006`; audit `M15` | `ambre_server::routes::tests::handler_tests::a_token_in_the_socket_address_opens_no_session`; `::a_token_in_the_first_frame_opens_the_session` -- the socket takes its token in its first frame and never reads its address; `ambre_sync::websocket::tests::the_socket_address_carries_no_token`; `ambre_sync::websocket::tests::an_opened_socket_authenticates_before_it_subscribes` -- the client sends it before any subscription | none | verified |
| `SRV-234` | `INV` | A display name the relay stores MUST be trimmed, non-blank and at most the cap of `ambre-spec-budgets.md section 6`. A provider's name past the cap MUST be cut to it, and a development sign-in under a name that does not fit MUST be refused. | this spec section 3; `SRV-023` | `ambre_server::routes::tests::handler_tests::a_dev_sign_in_stores_a_display_name_that_fits` -- a padded name is stored trimmed, and one past the cap is refused with no row; `ambre_server::routes::auth::tests::a_provider_name_is_trimmed_and_cut_to_the_cap` -- the provider's claims, a name past the cap among them | none | verified |
| `SRV-267` | `INV` | The ephemeral store MUST hold no refresh token and no sign-in state in the clear, only its SHA-256. | audit `N10`; this spec section 3 | `ambre_server::secrets::tests::a_store_key_holds_the_hash_of_the_token_and_never_the_token` -- the store key of each is its prefix and the hash, and the two keys are the only door to the store (`ARCH-049`) | none | verified |
| `SRV-291` | `INV` | The ephemeral store MUST NOT hold a refresh record or a sign-in state without its lifetime, at any instant. | this spec section 3; `SRV-267` | `ambre_server::redis_ops::redis_integration_tests::a_record_is_written_with_its_lifetime` -- database tier: one script writes each record and its lifetime, and both read back with a lifetime | none | verified |

### 1.2 A request is authenticated

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-003` | `POST` | A route that needs a caller MUST verify the access token, its signature and its expiry. | this spec section 3 | `ambre_server::jwt::tests::expired_token_is_rejected` -- the refusal on the expiry; `::wrong_key_is_rejected` -- the refusal on the signature; `::mint_then_verify_roundtrips` -- the accepted case carries the identity through; `routes::tests::handler_tests::unauth_endpoints_return_401` -- one route's refusal end to end, one route and not the surface | none | verified |
| `SRV-090` | `FRAME` | The token check MUST NOT read a store: it is a local signature check and nothing else. | this spec section 3; `ambre-spec-budgets.md section 3` | `ambre_server::auth::tests::the_token_check_reads_no_store` -- both stores are closed before the extractor runs, and the session still resolves. The check is a function that cannot wait on a store | none | verified |
| `SRV-303` | `FAIL` | A socket MUST end when the access token that opened it expires. | this spec section 3; `SRV-090` | `ambre_server::routes::tests::handler_tests::a_socket_ends_when_its_token_expires` -- database tier: a socket opened by a token of two seconds stays open before its expiry and is dropped after it | none | verified |

### 1.3 A person signs in, and sets up their account

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-091` | `FRAME` | A sign-in MUST NOT carry a passphrase, nor anything the relay could derive a key from. | `ZK-003`; this spec section 3 | cargo-deny -- `argon2` is granted to `ambre-crypto` alone, and `ambre-server` declares neither, so the relay has no way to turn a secret into a key: what it stores is the sealed master material and the sealed salt, and it opens neither | none | verified |
| `SRV-023` | `FAIL` | The development sign-in MUST be refused unless the deployment enables it. | this spec section 3 | `ambre_server::routes::tests::handler_tests::dev_login_rejected_when_dev_mode_off` -- the refusal; `::dev_login_creates_user_and_returns_token` -- the enabled case still works, so neither end can rot into the other | none | verified |
| `SRV-319` | `FRAME` | The relay MUST NOT create an account that no provider vouched for, except by the development sign-in. | this spec section 3; `SRV-029`; `SRV-023` | `ambre_server::routes::tests::handler_tests::the_relay_serves_no_anonymous_account_setup` -- database tier: the served router answers no setup body with success, and writes no account; type-level -- `ambre_bus::RestCommand` holds no command that creates an account, so no client path sends one | none | verified |
| `SRV-030` | `FAIL` | The sealed master material MUST be written once; a later write MUST be refused. | this spec section 3 | `ambre_server::routes::tests::handler_tests::store_master_keys_is_first_time_only` -- the first upload succeeds and the second answers a conflict rather than overwriting, and the re-seal route is the passphrase path and touches neither public key | none | verified |
| `SRV-293` | `FAIL` | Given a first key store under a signing key another account registered, the relay MUST refuse it and write nothing. | this spec section 3; `SRV-030`; `SYNC-215` | `ambre_server::routes::tests::handler_tests::a_signing_key_belongs_to_one_account` -- database tier: a first key store under a registered key answers 409, even with the proof of that key, and writes no master material | none | verified |
| `SRV-294` | `FAIL` | Given a first key store that the signing key it registers does not sign over the account and the two keys, the relay MUST refuse it and write nothing. | this spec section 3; `SRV-293` | `ambre_server::routes::tests::handler_tests::a_key_store_the_registered_key_does_not_sign_is_refused_and_writes_nothing` -- database tier: a proof another key signed answers `INVALID_SIGNATURE`, a proof over another account and a body with no proof are refused, and no key is written, while the proof of the key lands; `ambre_protocol::key_registration::tests::the_transcript_names_the_account_and_both_keys` -- the signature covers the domain, the account and both keys; `ambre_sync::auth::tests::a_key_setup_stores_a_seed_that_a_later_unlock_opens` -- the worker sends the proof of the seed's signing key | none | verified |
| `SRV-278` | `FAIL` | Given a re-seal of the master material that the account's signing key does not sign, the relay MUST refuse it and write nothing. | this spec section 3; `SRV-030` | `ambre_server::routes::tests::handler_tests::a_re_seal_the_account_key_does_not_sign_is_refused_and_writes_nothing` -- a proof signed by another key, and one signed over another sealed seed than the body carries, answer `INVALID_SIGNATURE`, a body with no proof is refused, and the stored seal stays each time, while the proof of the account's key lands, database tier; `ambre_protocol::master_rotation::tests::the_transcript_names_the_replaced_salt_and_the_new_sealed_seed` -- the signature covers the domain, the replaced salt and the new sealed seed | none | verified |
| `SRV-279` | `FAIL` | Given a re-seal whose proof names a sealed seed the relay no longer holds, the relay MUST refuse it and write nothing. | this spec section 3; `SRV-278` | `ambre_server::routes::tests::handler_tests::a_re_seal_over_a_replaced_seal_is_refused_and_writes_nothing` -- two re-seals over one seal race and one lands, then the other and a replay of the winner answer `CONFLICT`, and the winner's seal stays, database tier | none | verified |
| `SRV-232` | `FAIL` | Given a development sign-in or a key registration with a public key that is not 32 bytes, the relay MUST refuse it and write no row. | this spec section 3; `SRV-231` | `ambre_server::routes::tests::handler_tests::a_public_key_of_the_wrong_length_is_refused_and_written_nowhere` -- a short signing key and a short exchange key each refuse the sign-in and the registration, and no account or key material is written, database tier | none | verified |
| `SRV-172` | `POST` | A sign-in with an identity provider MUST be sent to that provider's authorization endpoint, under the configured client. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_provider_route_reaches_its_provider` -- the shipped router hands the provider in the path to the handler, which names an unconfigured one in its refusal; `ambre_server::oidc::tests::a_sign_in_is_sent_to_the_configured_provider` -- a provider stub is discovered, and the sign-in goes to its authorization endpoint under the configured client | none | verified |
| `SRV-022` | `POST` | A provider's return MUST exchange its code for an identity token the relay verifies, and sign the user in. | this spec section 3 | `ambre_server::oidc::tests::a_provider_code_is_exchanged_for_a_verified_identity` -- a stub provider signs an identity token with a key it publishes, and the code is exchanged under the kept verifier for its claims; `::an_identity_the_relay_cannot_verify_is_refused` -- a foreign key and a wrong one-time value fail the verification, and a wrong verifier fails the exchange; `ambre_server::routes::tests::handler_tests::a_provider_return_signs_the_person_in` -- through the shipped router, the return opens a session that `/auth/me` answers as that person, and it is not taken twice | none | verified |
| `SRV-074` | `FAIL` | A configured provider the relay cannot reach at startup MUST stop the start, never disable sign-in in silence. | audit `M7`; this spec section 3 | `ambre_server::oidc::tests::a_configured_provider_that_does_not_answer_stops_the_start` -- the start decision against an issuer nothing listens on is an error, which `main` returns. With no client id nothing is discovered and sign-in is off | none | verified |
| `SRV-296` | `FRAME` | A request of the relay to the identity provider MUST NOT follow a redirect, and MUST end at its deadline. | this spec section 3; `SRV-022`; `ambre-spec-budgets.md` section 8 | `ambre_server::oidc::tests::the_provider_client_follows_no_redirect_and_keeps_a_deadline` -- a redirect comes back as itself, and a provider that stalls fails the request at the deadline | none | verified |
| `SRV-295` | `POST` | A reclaim of an address MUST tell each member who invited the account that held it. | this spec section 3; `SRV-029` | `ambre_server::routes::tests::handler_tests::a_reclaim_tells_each_inviter_of_the_account_that_held_the_address` -- database tier: the sign-in that reclaims names the one inviter per document, the inviter's topic hears `AddressReclaimed` for that document and member, and a second sign-in owes no notice | none | verified |

### 1.4 A session ends

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-009` | `FAIL` | A refresh token MUST be consumable exactly once, including by two callers at the same instant. | this spec section 3; audit `M6` | `ambre_server::redis_ops::redis_integration_tests::refresh_consume_rejects_a_replayed_token` -- the second use fails; `::only_one_racing_caller_claims_a_refresh_token` -- exactly one of two concurrent claims wins, database tier | none | verified |
| `SRV-026` | `POST` | A sign-out MUST revoke the caller's own refresh token. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_session_crosses_instances_and_a_sign_out_ends_it_on_both` -- after a sign-out on one instance, the refresh token is refused on both | none | verified |
| `SRV-092` | `FRAME` | A sign-out MUST NOT revoke a token it did not issue to this caller. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_sign_out_revokes_no_token_of_another_caller` -- a foreign refresh token survives, and the caller's own is revoked | none | verified |
| `SRV-320` | `POST` | A renewal of a session minted before the keys MUST carry the signing key the account registered since. | this spec section 3; `SRV-027`; `ambre-spec-sync.md`, `SYNC-296` | `ambre_server::routes::tests::handler_tests::a_renewal_after_the_key_store_carries_the_registered_key` -- database tier: of two keyless pairs, one stores the keys, and the refresh of the other mints a token under the stored key | none | verified |

* **Budget.** the provider callback of `ambre-spec-budgets.md section 3`, under 200 ms, which
  reads **none exists**. The token verification carries no budget: it is a local signature check on
  every request, and no row measures it.
* **Boundary.** the relay stores the sealed master material, the sealed salt and both public keys,
  and sees none of the three secrets behind them (`ZK-003`, `SRV-091`). What it mints is its own
  token, and `SRV-012` refuses it every other cryptographic act. The socket takes its token in its
  first frame, so no address carries one (`SRV-031`).
* **Verification.** 33 `verified`. What holds is what a request test can drive:
  the token gate, the address namespace, the one-shot key write and the refresh race. A provider
  exchange runs against a stub that signs its own identity tokens. A real provider never runs under
  a test.
---

## 2. WHAT EACH USER SEES

Two people over one address namespace, which is the only state a session shares.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| holds an account under an address she asserted, before the setup was retired | holds another account under the same address | one account, Alice's | the second is refused as a taken address (`SRV-028`) | `db::users::tests::create_user_duplicate_email_is_conflict` |
| holds an account under Bob's address, which she asserted | signs in with his provider, on that address | one account, Bob's | Alice loses the address; Bob signs in. Each member who invited Alice's account is told (`SRV-029`, `SRV-295`) | `db::users::tests::oidc_login_reclaims_email_from_unverified_squatter`; `routes::tests::handler_tests::a_reclaim_tells_each_inviter_of_the_account_that_held_the_address` |
| signs in with her provider | asserts Alice's address on an account | one account, Alice's | Bob is refused (`SRV-029`) | `db::users::tests::verified_email_cannot_be_squatted_by_setup` |
| looks for a way to create an account with no provider | watches | no account | no route creates it (`SRV-319`) | `routes::tests::handler_tests::the_relay_serves_no_anonymous_account_setup` |
| renews her session twice with one refresh token | watches | one live refresh token | the second renewal is refused, and Alice signs in again (`SRV-009`) | `redis_ops::redis_integration_tests::refresh_consume_rejects_a_replayed_token` |
| signs out | holds Alice's refresh token | no refresh token for Alice | Bob's renewal is refused too (`SRV-026`) | `routes::tests::handler_tests::logout_clears_session` |

---

## 3. MODEL

**CRDT containers.** none. A session is relay state, and none of it converges.

**Two ways in, one outcome.** Each produces the same pair of tokens.

| Way in | Establishes the identity by | Available |
|---|---|---|
| an identity provider | the provider's own assertion, over a handshake with a proof of possession and a one-time value | in every deployment that configures one (`SRV-022`) |
| a development sign-in | a name, from which the client derives its keys with no secret at all | only where the deployment enables it (`SRV-023`) |

**No account without a provider.** An account setup once took a self-asserted address and created
an account no provider vouched for. It is retired: no route and no client command creates such an
account (`SRV-319`). Accounts it created remain, so the reclaim rule still holds for them
(`SRV-029`). A development sign-in with no key stands in for a provider account that owes its keys.
The development tiers reach the key setup through it.

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
delta. Reading the key from the token turns that into a local comparison. A lookup per delta would
instead put a query on the hottest path the relay has (`ambre-spec-broadcast.md`).

**A session before the keys.** A provider account exists before its keys, because the client
derives them after the provider answers. Until the one-shot key write lands, the session carries
no signing key. The profile, the master material, the key write and the sign-out accept it. Every
other route and the socket refuse it as a setup still owed. The key write answers with a new pair
under the key it stored (`SRV-027`).

A key write can land while another pair of the account stays keyless: a second tab, or a page
reloaded during the setup. The renewal of a keyless pair reads the register, so the next pair
carries the registered key (`SRV-320`). The read happens only for a keyless pair, never on the
renewal of a set-up account.

**An account before its keys.** It stores none, rather than a stand-in. The search, the key lookup
and the invite pass over it, so no document key is ever wrapped to it (`SRV-163`).

**A renewal is a rotation.** The claim of a refresh token is atomic, where read-then-delete would
let two callers holding one token both mint a session. A stolen token replayed after its owner used
it is therefore refused rather than honoured (`SRV-009`).

**The store keys hold a hash.** The ephemeral store names a refresh token and a sign-in state by
the SHA-256 of the value. A copy of the store then signs nobody in (`SRV-267`). The relay takes
`sha2` for it: a hash with no key, already in the tree through `ambre-crypto`. It grants none of
the capabilities `ZK-004` bans.

**What the relay stores of an account.**

| Field | Form on the relay |
|---|---|
| the address, and whether a provider verified it | plaintext (`SRV-028`, `SRV-029`) |
| the display name | plaintext, trimmed and bounded in length (`SRV-234`) |
| the master material, and the salt that derives its key | sealed by the client, opaque here (`ZK-003`) |
| the signing key and the exchange key | plaintext public halves |
| the provider and its subject, where a provider established the account | plaintext |

The master material is written once and refused afterwards. Overwriting the public keys would
leave every document key already sealed to the previous exchange key unopenable (`SRV-030`).
A passphrase change re-seals the material alone and moves neither public key.

**One signing key, one account.** A member list shows every member's signing key. An outsider who
registered a member's key under an account of their own would, once invited, take that member's
edits from attribution. So the register refuses a second account under a registered key at the
first key store (`SRV-293`).

**The first key store proves the key.** The unique index stops a second registration of a key, not
the first. An outsider could still register a member's key before the member does. So the client
signs the account and the two keys with the signing key it registers. The relay verifies the
signature under that key before it writes, and refuses a proof another key signed
(`SRV-294`).

The account in the proof binds it to one account: a proof read on the wire registers the key
nowhere else. The domain separates this signature from every other signature of the account.

**A reclaim tells the inviters.** A reclaim renames the address of the self-asserted account, and
leaves its member rows and wrapped keys. Each member who invited that account wrapped a document
key for an address no provider proved. The relay reads those inviters in the transaction of the
reclaim. It then sends each one `AddressReclaimed` on its own topic, naming the document and the
member (`SRV-295`).

A notice is best-effort. An inviter that misses it still reads the member list, which shows the
member unproven and without the address.

**A re-seal proves the seal it replaces.** A passphrase change and a recovery each replace the
sealed master material. A session alone does not authorize the write: a stolen session would
replace the seal and lock the owner out of the seed. The client signs a proof with the account's
signing key, which only the seed derives. The proof covers three values after its own domain:

| Part | Why |
|---|---|
| the salt of the seal the client read | each seal draws a fresh salt, so the salt names one seal |
| the new salt | the relay stores the pair, and a swap of either part fails the proof |
| the new sealed seed | last, after two fixed widths, so no two inputs share a transcript |

The relay verifies the proof under the signing key the account registered, and refuses a proof
another key signed (`SRV-278`). It then writes only while it still holds the named seal, in one
statement. A replay, or a second re-seal that read the same seal, names a seal already replaced
and is refused (`SRV-279`). The domain separates this signature from every other signature of the
account. The relay verifies an Ed25519 signature here, the one act `SRV-012` allows it.

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
| an access token nears the end of its announced lifetime | the client renews the pair ahead of the expiry, once (`ambre-spec-sync.md`, `SYNC-183`, `SYNC-184`) |
| a request is refused for an expired token | the client renews once, as the fallback, and replays it (`ambre-spec-sync.md`, `SYNC-062`) |
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
| The token carries the caller's signing key | reading the key from the register on every delta | the write path verifies a signature per delta, so a lookup there is a query per keystroke burst. The claim is minted from the register at sign-in, so it is as fresh as the session (`ambre-spec-broadcast.md`) |
| A provider-verified address wins over a self-asserted one | first claimant wins, which is what the namespace rule alone produced | the address is what a share resolves against, so letting anyone assert any address turns a signup form into a way to squat a person's identity and receive documents meant for them. The verified claimant reclaims it, and the unverified account keeps everything except the address (`SRV-029`) |
| The master material is written once | letting the write repeat | it reads like an idempotent upload and is not: a second write replaces both public keys, and every document key already sealed to the previous exchange key stops opening. The failure is silent and total -- the account loses every shared document it had |
| A re-seal carries a proof signed by the account's key | the session alone, which is what the route took until audit `P3` | a session is a bearer token. Whoever steals one replaces the sealed seed under a passphrase they chose, and the owner's own passphrase no longer opens it. The signing key never leaves the client, so a stolen session cannot sign (`SRV-278`) |
| The proof names the seal it replaces, and the relay writes over that seal alone | the relay builds the proof's transcript from the seal it holds | a proof over a seal already replaced then fails as a bad signature. The client cannot tell a concurrent re-seal from a fault, and retries nothing it understands. A named seal answers `CONFLICT` instead, apart from the `INVALID_SIGNATURE` of a forged proof (`SRV-279`) |
| An account comes from a provider, or from the development sign-in | an account setup that takes a self-asserted address, which the relay served until audit `W67` | the address is what a share resolves against. A setup lets an outsider claim a person's address first and receive the keys of every document shared to it. The reclaim only renames the address later, after the keys are wrapped (`SRV-319`) |
| The first key store carries a proof the registered key signs | the session alone, with the unique index of `SRV-293` | the index refuses a second registration, not the first. An outsider who registers a member's key first takes that member's edits from attribution once invited. The signing key never leaves the client, so only its holder signs (`SRV-294`) |
| The proof signs the account | a challenge the relay mints and keeps | the account already names one registration, and a challenge adds a round trip and a record to expire. A proof over another account fails, so a proof read on the wire registers nothing (`SRV-294`) |
| A renewal of a keyless pair reads the register | a keyless pair stays keyless until a sign-in | a page reloaded during the setup holds a keyless pair while its first store lands. The socket refuses that pair, and its renewal would never carry the key. Only a keyless pair reads the register, so the renewal of a set-up account stays free of a query (`SRV-320`) |
| The development sign-in derives its keys from the name | a fixture account with a stored passphrase | it is a secret to store and keep in step for an account whose only requirement is to be the same on every run. Deriving from the name gives that with nothing stored, and the switch that enables it is off by default |

---

## 6. NOT BUILT, AND NOT YET RULED

* **One provider is configured, and the corpus names four.** The deployment reads the credentials of
  a single provider.
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
