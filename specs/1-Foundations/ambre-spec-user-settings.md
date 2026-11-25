# AMBRE -- FEATURE: USER SETTINGS

**Type:** Horizontal (framework)

> The choices one user makes for themselves, kept in one sealed document that follows them from
> device to device. A theme or a panel docked to the left on the laptop holds on the phone. The
> relay reads none of them.

This document owns the settings document and the key that seals it. It also owns where the relay
keeps it, how two devices merge their changes, and what a page shows before the unlock. What each
setting does is the rule of the document that owns its section.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `SETTINGS`, and it belongs to this document alone.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-001` | `INV` | The settings of a user MUST live in one document whose only member is that user. | this spec section 3, the store; `ambre-spec-membership.md` | `ambre_server::routes::tests::handler_tests::the_settings_document_takes_no_member_and_is_never_listed` -- the invite answers 409; `ambre_logic::settings_flow::tests::a_delta_of_the_user_moves_the_view_and_another_users_does_not` -- another user's delta changes nothing | none | verified |
| `SETTINGS-002` | `INV` | The relay MUST hold at most one settings document per user. | this spec section 3, the store | `ambre_server::routes::tests::handler_tests::an_account_holds_one_settings_document` -- a second claim stores nothing | none | verified |
| `SETTINGS-003` | `INV` | Each setting MUST belong to exactly one section, and each section to one owning document. | this spec section 3, the catalogue | `ambre_client_core::settings::tests::each_setting_belongs_to_one_section_under_one_key` | none | verified |
| `SETTINGS-004` | `INV` | A setting MUST be one last-writer-wins entry, keyed by what it configures. | this spec section 3, the merge | `ambre_logic::settings::tests::two_devices_converge_after_writing_apart` | none | verified |
| `SETTINGS-005` | `INV` | The settings document MUST stay pinned while its user is signed in. | this spec section 3, the store; `RESID-003` | structural -- the Logic thread holds the settings session beside the document table that residency evicts, and no eviction reads it | none | verified |
| `SETTINGS-006` | `INV` | The settings document MUST stay within its byte budget. | this spec section 3, the store; `ambre-spec-budgets.md section 6` | `ambre_logic::settings::tests::the_whole_catalogue_stays_within_its_byte_budget` -- every setting of the catalogue at its longest word | none | verified |
| `SETTINGS-009` | `CONC` | Two devices signing in for the first time at once MUST converge on one settings document. | this spec section 3, the store; `SETTINGS-002` | `ambre_server::routes::tests::handler_tests::two_first_claims_at_once_converge_on_one_document` | none | verified |
| `SETTINGS-016` | `CONC` | Two devices that change one setting, one of them offline, MUST converge on one value once both reach the relay. | this spec section 3, the merge; `ambre-spec-delta-order.md` | `ambre_logic::settings::tests::two_devices_converge_after_writing_apart` | none | verified |
| `SETTINGS-017` | `INV` | The key of the settings document MUST be wrapped for the exchange key of its user, as the key of any document. | this spec section 3, the key; `ambre-spec-membership.md section 3` | `ambre_sync::settings::tests::an_account_with_none_claims_one_and_opens_the_relay_answer` -- the self-wrap of the creation rides the claim | none | verified |
| `SETTINGS-045` | `INV` | Every device of the user MUST open the settings key from its first self-wrap, after a recovery too. | this spec section 3, the key; `SETTINGS-017`; `ambre-spec-zero-knowledge.md`, `ZK-009`, `ZK-043`; `ambre-spec-session.md`, `SRV-030`; `ambre-spec-sync.md`, `SYNC-249` | `ambre_sync::settings::tests::the_settings_self_wrap_opens_under_the_key_a_recovery_derives` -- the words of the account derive the registered exchange key and open the copy, and another seed opens nothing; `ambre_server::routes::tests::handler_tests::store_master_keys_is_first_time_only` -- the relay writes the exchange key of an account once, database tier | none | verified |
| `SETTINGS-018` | `INV` | The settings document MUST declare its kind inside its sealed content. | this spec section 3, the store | `ambre_logic::settings::tests::a_created_document_names_its_kind_in_its_first_delta`; `ambre_logic::settings_flow::tests::a_burst_of_changes_leaves_as_one_delta_after_the_delay` -- the first delta names the kind | none | verified |
| `SETTINGS-019` | `INV` | Every setting MUST declare a default value. | this spec section 3, the catalogue | type-level -- `SettingValue` requires `Default`, and `Setting::read` falls back on it | none | verified |
| `SETTINGS-020` | `INV` | A setting whose meaning or type changes MUST take a new key, and never reuse the old one. | this spec section 3, the catalogue | `ambre_client_core::settings::tests::every_released_key_keeps_its_section_and_its_name` | none | verified |
| `SETTINGS-021` | `INV` | A setting whose stored value a client cannot decode MUST read as its default on that client. | this spec section 3, the merge | `ambre_client_core::settings::tests::an_absent_or_unknown_word_reads_as_the_default`; `ambre_logic::settings::tests::an_unknown_setting_survives_an_older_client` | none | verified |
| `SETTINGS-022` | `INV` | The width of the context panel MUST stay on the device, outside the settings document. | this spec section 3, the catalogue | `ambre_client_core::settings::tests::every_released_key_keeps_its_section_and_its_name` -- the frozen catalogue holds no width | none | verified |
| `SETTINGS-023` | `INV` | The theme MUST be a setting of the account. | this spec section 3, the catalogue; `ambre-spec-design-tokens.md section 2` | `ambre_client_core::settings::tests::every_released_key_keeps_its_section_and_its_name` | none | verified |
| `SETTINGS-024` | `INV` | The interface language MUST be a setting of the account. | this spec section 3, the catalogue; `ambre-spec-locale.md section 3` | `ambre_client_core::settings::tests::every_released_key_keeps_its_section_and_its_name` | none | verified |
| `SETTINGS-044` | `INV` | Every command and property title MUST carry its words in each interface language that ships. | this spec section 3, the catalogue | type-level -- `ambre_commands::TitleWords` holds one field per language, so a table that omits one does not compile; `ambre_commands::tests::a_table_names_each_missing_part_of_a_description` -- a blank language is a defect | none | verified |
| `SETTINGS-025` | `INV` | The side of the context panel MUST be a setting of the account. | this spec section 3, the catalogue | `ambre_client_core::settings::tests::every_released_key_keeps_its_section_and_its_name` | none | verified |
| `SETTINGS-026` | `INV` | The fold of the context panel MUST be a setting of the account, one value per application. | this spec section 3, the catalogue | `ambre_client_core::settings::tests::every_released_key_keeps_its_section_and_its_name` | none | verified |
| `SETTINGS-027` | `INV` | The number convention of a reader and their own locale MUST be settings of the account. | this spec section 3, the catalogue; `ambre-spec-locale.md section 3` | `ambre_client_core::settings::tests::every_released_key_keeps_its_section_and_its_name` | none | verified |
| `SETTINGS-028` | `INV` | The boot hint MUST carry the theme and the interface language, and no other setting. | this spec section 3, the boot | type-level -- `BootHint` holds the theme and the interface language alone | none | verified |

### 1.2 Open the settings on a device

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-007` | `POST` | A client that unlocks its account MUST ask the relay for its settings document and open it like any document. | this spec section 3, the store | `ambre_sync::settings::tests::a_known_settings_document_is_opened_without_a_claim`; `ambre_logic::settings_flow::tests::the_named_settings_document_opens_and_the_view_reads_the_defaults` | none | verified |
| `SETTINGS-008` | `POST` | A user with no settings document MUST get one at the first unlock. | this spec section 3, the store | `ambre_sync::settings::tests::an_account_with_none_claims_one_and_opens_the_relay_answer`; `ambre_server::routes::tests::handler_tests::an_account_holds_one_settings_document` | none | verified |
| `SETTINGS-010` | `POST` | Until the settings document is loaded, every setting the boot hint does not carry MUST read as its default. | this spec section 3, the boot | `ambre_client_core::settings::tests::an_absent_or_unknown_word_reads_as_the_default`; `ambre_logic::settings_flow::tests::the_named_settings_document_opens_and_the_view_reads_the_defaults` | none | verified |
| `SETTINGS-029` | `POST` | Once the settings document is loaded, the client MUST write the boot hint from it. | this spec section 3, the boot | `ambre_app::switchboard::tests::the_settings_of_the_account_reach_the_page_as_one_effect` -- each settings change calls for the effect that writes the hint; `ambre_app::platform::tests::the_boot_hint_reads_back_from_the_storage_and_a_sign_out_deletes_it` -- wasm tier, the written hint reads back | none | verified |
| `SETTINGS-011` | `FRAME` | Opening the settings MUST NOT hold back the open of another document. | this spec section 3, the store | `ambre_logic::settings_flow::tests::the_settings_open_holds_back_no_other_open` | none | verified |
| `SETTINGS-030` | `FAIL` | Given a settings document whose sealed kind is not the settings kind, the client MUST refuse it, write nothing into it and keep the defaults. | this spec section 3, the store; `SETTINGS-018` | `ambre_logic::settings::tests::a_document_with_no_settings_marker_takes_no_write` | none | verified |
| `SETTINGS-031` | `FAIL` | Given a device that already opened the settings document of its user, a relay answer naming another one MUST be refused, and the device MUST keep the one it opened. | this spec section 3, the store; `SETTINGS-002` | `ambre_logic::settings_flow::tests::the_kept_settings_document_opens_and_another_relay_answer_is_refused` -- the relay answer waits for the store, and the kept document opens whichever answer comes first; `ambre_logic::persistence::tests::a_kept_settings_document_outlives_the_page_and_is_never_replaced` -- wasm tier, the kept name reads back on a later page and a second one never replaces it | none | verified |
| `SETTINGS-040` | `FAIL` | Given a second settings document named in one session, the client MUST refuse it and keep the one it opened. | this spec section 3, the store; `SETTINGS-031` | `ambre_logic::settings_flow::tests::a_second_settings_document_named_in_a_session_is_refused` | none | verified |
| `SETTINGS-050` | `FAIL` | Given a second settings document named in one session, the client MUST show its user an error toast that names the refusal and what to do, in the interface language. | this spec section 3, the store; `SETTINGS-040`; `ambre-spec-chrome.md` | `ambre_logic::settings_flow::tests::a_refused_settings_document_warns_the_user` -- the refusal reaches the view as a warning, and the kept document named again raises none; `ambre_app::switchboard::tests::a_refused_settings_document_raises_an_error_toast_in_the_page_language` -- one announced error toast, in English and in French | none | verified |

### 1.3 Boot a page before the unlock

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-032` | `POST` | A page with a boot hint MUST boot in the theme the hint names. | this spec section 3, the boot | `ambre_app::platform::tests::a_page_boots_in_the_theme_of_the_stored_boot_hint` -- wasm tier; `ambre_app::settings_view::tests::a_page_boots_in_the_theme_of_its_boot_hint` | none | verified |
| `SETTINGS-033` | `POST` | A page with a boot hint MUST boot in the interface language the hint names. | this spec section 3, the boot | `ambre_app::platform::tests::a_page_boots_in_the_language_of_the_stored_boot_hint` -- wasm tier; `ambre_app::settings_view::tests::a_page_boots_in_the_language_of_its_boot_hint`; `ambre_chrome::language::tests::a_french_tag_of_any_region_shows_french`; `ambre_chrome::language::tests::a_tag_of_none_takes_the_browser_language` | none | verified |
| `SETTINGS-034` | `FRAME` | Reading the boot hint MUST NOT wait on the network, on a key or on the local store. | this spec section 3, the boot | structural -- the page reads the hint through the synchronous local storage, before the first frame and before any wait | none | verified |
| `SETTINGS-042` | `POST` | A pick in the language picker of the avatar menu MUST change the interface language of the page and of the account. | this spec section 3, the catalogue; `ambre-spec-locale.md section 3` | `ambre_app::shell::app_root::tests::the_header_ends_on_share_and_the_avatar_menu` -- wasm tier, the French option writes `fr` | none | verified |
| `SETTINGS-043` | `POST` | A new interface language MUST redraw every label that reads it, with no reload. | this spec section 3, the boot | `ambre_chrome::language::tests::a_label_read_in_a_reactive_scope_follows_a_new_language` | none | verified |
| `SETTINGS-047` | `POST` | A word of the Prose chrome MUST show in the interface language of the page. | this spec section 3, the interface language | `ambre_prose::view::slash::language_tests::the_slash_menu_reads_and_finds_its_items_in_the_interface_language` | none | verified |
| `SETTINGS-048` | `POST` | A change of the interface language MUST show every word in the new language, the titles at once and the rest from the next load of the page. | this spec section 3, the interface language; `SETTINGS-035` | `ambre_logic::settings_flow::tests::a_language_change_reloads_the_page_after_its_copy` -- the reload follows its copy; `ambre_app::local_store::tests::the_page_reloads_once_the_copy_before_the_reload_landed` -- wasm tier, no reload while a write is in flight; `ambre_app::shell::app_root::tests::the_header_ends_on_share_and_the_avatar_menu` -- wasm tier, the pick writes the boot hint first | none | verified |
| `SETTINGS-049` | `POST` | After a reload, the page MUST keep the interface language of its local copy over an earlier one the relay replays. | this spec section 3, the interface language; `SETTINGS-035` | `ambre_logic::settings_flow::tests::the_reloaded_page_keeps_the_language_of_its_copy_over_the_replay` | none | verified |

### 1.4 Change a setting

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-012` | `POST` | A change MUST reach every other signed-in device of the user as a delta of the settings document. | this spec section 3, the merge | `ambre_logic::settings::tests::a_change_reaches_the_other_device_as_one_delta`; `ambre_logic::settings_flow::tests::a_burst_of_changes_leaves_as_one_delta_after_the_delay` -- a second device takes the delta | none | verified |
| `SETTINGS-041` | `POST` | Once the settings document answers, the side and the fold of the context panel MUST follow the account on every device. | this spec section 3, the catalogue; `SETTINGS-025`; `SETTINGS-026` | `ambre_app::shell::panel::tests::the_side_and_the_fold_follow_the_account` -- wasm tier, the account moves the panel, a change of the person reaches it, and no value of the account is written back; `ambre_belt::settings::tests::the_settings_follow_the_account_across_two_devices` -- browser tier, two devices of one account | none | verified |
| `SETTINGS-046` | `POST` | A change of the interface language MUST reach the boot hint of every other signed-in device of the user. | this spec section 3, the boot; `SETTINGS-024`; `SETTINGS-037` | `ambre_belt::settings::tests::the_settings_follow_the_account_across_two_devices` -- browser tier, two devices of one account, one way and back, the language picked in the avatar menu | none | verified |
| `SETTINGS-035` | `POST` | A change made while the socket is down MUST reach the relay once the socket is back. | this spec section 3, the merge; `RESID-019` | `ambre_logic::settings_flow::tests::a_change_made_while_the_socket_is_down_leaves_once_it_is_back` -- in the page, the change past the last echo leaves again; `ambre_logic::settings_flow::tests::a_change_left_offline_by_an_earlier_page_leaves_at_the_next_open` -- after a reload, the local copy sends it under the nonce of its session once the replay is in | none | verified |
| `SETTINGS-036` | `POST` | Changes made within one coalescing delay MUST leave the device as one delta. | this spec section 3, the budget; `ambre-spec-budgets.md section 8` | `ambre_logic::settings::tests::a_burst_of_writes_leaves_as_one_delta`; `ambre_logic::settings_flow::tests::a_burst_of_changes_leaves_as_one_delta_after_the_delay` | none | verified |
| `SETTINGS-037` | `POST` | A change of the theme or of the interface language MUST rewrite the boot hint of the device. | this spec section 3, the boot | `ambre_app::switchboard::tests::the_settings_of_the_account_reach_the_page_as_one_effect`; `ambre_app::platform::tests::the_boot_hint_reads_back_from_the_storage_and_a_sign_out_deletes_it` -- wasm tier, a later hint replaces the first | none | verified |
| `SETTINGS-013` | `FRAME` | Changing a setting MUST NOT change a setting of another section. | this spec section 3, the catalogue; `SETTINGS-003` | `ambre_logic::settings::tests::a_write_changes_no_other_section` | none | verified |
| `SETTINGS-038` | `FRAME` | Changing a setting MUST NOT remove or rewrite a setting the client does not know. | this spec section 3, the merge; `SETTINGS-021` | `ambre_logic::settings::tests::an_unknown_setting_survives_an_older_client` | none | verified |

### 1.5 Sign out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SETTINGS-014` | `POST` | Signing out MUST delete the local copy of the settings document. | this spec section 3, the store; `ambre-spec-residency.md` | `ambre_logic::settings_flow::tests::signing_out_erases_the_local_copy_of_the_settings` -- the sign-out hands the store the erase of the copy | none | verified |
| `SETTINGS-039` | `POST` | Signing out MUST delete the boot hint. | this spec section 3, the boot | `ambre_app::platform::tests::the_boot_hint_reads_back_from_the_storage_and_a_sign_out_deletes_it` -- wasm tier, the delete leaves no hint | none | verified |
| `SETTINGS-015` | `FRAME` | Signing out MUST NOT delete the settings document on the relay. | this spec section 3, the store | `ambre_server::routes::tests::handler_tests::a_sign_out_keeps_the_settings_document_on_the_relay` | none | verified |

* **Budget.** the settings document row of `ambre-spec-budgets.md section 6`, and the coalescing
  delay of `ambre-spec-budgets.md section 8`. Both wait on a measure.
* **Boundary.** the settings document is sealed like any document (`ZK-001`), and no setting leaves
  in another field (`ZK-054`). The relay learns which document holds the settings, and the size and
  time of each delta (`ZK-055`).
* **Verification.** 50 `verified`.

---

## 2. WHAT EACH USER SEES

| Alice does | on | The settings document holds | What Alice sees | Test |
|---|---|---|---|---|
| docks the panel to the left in Prose | her laptop | the side `left` in the `shell` section | the panel on the left on her desktop too, in Prose, Trame and Toile (`SETTINGS-012`, `SETTINGS-025`) | none |
| drags the panel wider | her laptop | nothing | the panel wider on the laptop alone (`SETTINGS-022`) | none |
| switches to the dark theme, then reloads | her laptop | the theme `dark` | the unlock screen dark, with no light frame first (`SETTINGS-032`) | none |
| folds the panel in Trame, offline on the train | her laptop | the fold of Trame, once the laptop is back online | the panel folded in Trame on her phone, open in Prose (`SETTINGS-016`, `SETTINGS-026`) | none |
| sets the theme to light on the phone, while the offline laptop sets it to dark | both | one theme, the same on both devices | one theme on both, whichever write wins (`SETTINGS-016`) | none |
| signs in on a new phone | the phone | nothing yet, on the phone | the default theme, then her settings once they load (`SETTINGS-010`) | none |
| signs in on two new devices at the same moment | both | one document | the same settings on both (`SETTINGS-009`) | none |
| signs out | her phone | her settings, on the relay | nothing left on the phone, and everything back at the next sign-in (`SETTINGS-014`, `SETTINGS-039`) | none |
| forgets her passphrase, and recovers with her 24 words on a new laptop | the laptop | her settings, under the same copy of the key | her settings, with no re-wrap and no prompt (`SETTINGS-045`) | none |
| -- | -- | the relay points her new phone at a document of her own that is no settings document | the defaults, and that document unchanged (`SETTINGS-030`) | none |
| -- | her laptop | the relay names another settings document after the first | her own settings, and an error toast that tells her to warn her administrator if it repeats (`SETTINGS-050`) | none |

---

## 3. MODEL

**The store: an ordinary document, with one member.** The settings take the document pipeline
whole: the sealed envelope, the sequencer, the snapshots, residency and the offline queue. The relay
records which document holds the settings of a user, once (`SETTINGS-002`). Two first unlocks at
once receive the same document (`SETTINGS-009`).

| What one settings document costs | Figure | Why it stays small |
|---|---|---|
| relay rows | one document, one member row, one log entry, one pointer on the account | the member row is a self-wrap, and the log holds the creation alone |
| keys | one document key | no member can be removed, so no rotation runs (`ambre-spec-key-rotation.md`) |
| relay work, per session | one subscription and one open | the open walks a log of one entry, and its first entry carries the user's own signature |
| a device signed out or lost | nothing | every device of a user holds the same Master Seed, so no device has a key to revoke |

**The creation, at the first unlock.** The client takes three steps:

1. It asks the relay for the settings document of its user (`SETTINGS-007`).
2. With none, it mints a document, its first log entry and its self-wrap (`SETTINGS-008`).
3. It sends all three and the pointer in one request. The relay stores them in one transaction when
   the account holds no pointer. Otherwise it stores nothing and answers the winner, which the
   client opens (`SETTINGS-009`).

A first unlock needs the relay, so the creation never runs offline.

**The kind is sealed.** The first delta writes the settings kind into the document (`SETTINGS-018`).
A relay could point a client at another document of that user, whose log the user also signed. The
sealed kind refuses it, and no setting is written into a Prose document (`SETTINGS-030`). A device
keeps the identifier it opened, and refuses an answer naming another (`SETTINGS-031`).

**A refusal warns the user.** A relay that names another settings document tries to swap the
settings of the user, which is a sign of attack. The client keeps its own, and an error toast says
so (`SETTINGS-050`). The toast tells the user to warn the administrator if the refusal repeats. The
toast asks for no decision: the client refused the document already.

**Offline.** The device keeps the identifier beside its trust pins, in the clear: the relay knows it
already. At sign-in, the relay's answer waits for that row, and the kept document opens first. The
document keeps a sealed local copy, as any document does, and no eviction takes it. A sign-out
erases the copy and keeps the identifier (`SETTINGS-014`). A change made offline reaches the relay in
one of two ways (`SETTINGS-035`):

1. The socket comes back in the same page: what the relay never echoed leaves again.
2. The page closed first: the next open sends it under the nonce of the session that wrote it, once
   the relay's replay is in (`AUTHOR-028`).

**The key.** The document key is random, and wrapped for the user's own exchange key, as any
document key is (`SETTINGS-017`). The exchange key derives from the Master Seed, so the account
derives the wrap and no new derivation domain exists (`ZK-009`).

| Event | What happens to the key |
|---|---|
| a passphrase change | nothing: the same seed is sealed again, and the exchange key does not move (`SYNC-251`) |
| a recovery | nothing: the words spell the same seed (`ZK-043`) |
| a new device | nothing: the device unlocks the same seed, then unwraps the same copy |
| a new exchange key of the user | none exists: the relay writes the exchange key once (`SRV-030`), and a recovery refuses another seed (`SYNC-249`) |

**One exchange key for the life of the account.** Every event above unlocks the same seed, and the
seed derives the same exchange key (`ZK-009`). So each device opens the copy of the creation, and
no re-wrap exists (`SETTINGS-045`). A new seed needs a new account, since the relay keeps the keys
of an account once. That account claims a settings document of its own (`SETTINGS-008`).

**The merge.** Each section is a map, and each entry is one last-writer-wins value
(`SETTINGS-004`). Two devices changing two settings never race. Two devices changing one setting
converge on the write the CRDT orders last (`SETTINGS-016`). The order is a logical clock, so the
winner is the same on both devices. It is not always the later write by the wall clock.

**An unknown setting survives an older client.** A client reads the keys it knows and leaves the
others alone (`SETTINGS-038`). A value it cannot decode, such as a theme a later release added,
reads as the default on that client, and stays stored (`SETTINGS-021`).

**The boot.** The settings document opens only once the account unlocks, and a local copy would be
sealed under a key of the session too (`RESID-025`). So each device keeps a boot hint: the theme
and the interface language, in the clear, in the browser's local storage (`SETTINGS-028`).

1. The page reads the hint before the first frame, with no wait (`SETTINGS-034`).
2. With a hint, the page boots in its theme and its language (`SETTINGS-032`, `SETTINGS-033`).
3. With none, the page boots in the default theme of `ambre-spec-design-tokens.md`.
4. Once the settings load, the client writes the hint from them (`SETTINGS-029`). Each later change
   of either setting writes it again (`SETTINGS-037`).

A kept session unlocks a reload with no passphrase (`ambre-spec-session.md`). That unlock still
ends after the first frame, so the hint serves it too.

The hint mirrors two settings of the account, and holds no choice of its own. It names no document
and no user. A sign-out deletes it (`SETTINGS-039`). The panel shows only once
a document opens, which is after the unlock, so no panel setting enters the hint.

**The catalogue.**

| Setting | Section | Values | Default | Owner |
|---|---|---|---|---|
| theme | `shell` | `system`, `light`, `dark` | `system` | `ambre-spec-design-tokens.md` |
| side of the context panel | `shell` | `left`, `right` | `right` | the shell, with its context panel |
| fold of the context panel, one per application | `shell` | `open`, `folded` | `open` | the shell, with its context panel |
| interface language | `locale` | a BCP 47 tag | the browser's language at the first run | `ambre-spec-locale.md` |
| the reader's own locale | `locale` | a BCP 47 tag, or none | the browser's language at the first run | `ambre-spec-locale.md` |
| number convention | `locale` | `document`, `type-mine`, `all-mine` | `type-mine` | `ambre-spec-locale.md` |
| the Murmure layout and levels, the macro approvals, the personal dictionary | `murmure`, `macros`, `prose` | the rule of each owner | the rule of each owner | the owner of each section |
| no entry yet | `trame`, `toile` | -- | -- | the Trame engine, the Toile engine |

Each application has a section, even one that holds no setting yet. A section with no entry is an
empty map, so a later setting of an application adds a declaration and never a section.

A theme at `system` follows the scheme of each device. So two devices of one account can show two
themes, each the one its system asks for.

**The interface language.** The avatar menu holds a picker beside the theme, with one choice per
language that ships: English and French (`SETTINGS-042`). A French tag of any region shows French,
and every other tag shows English. A stored tag of none takes the browser's language. The page
holds one language, and every label that reads it redraws on a change (`SETTINGS-043`). Each title
carries its words in every language, on its description (`SETTINGS-044`).

Every other word reads the language once, where the view draws it (`SETTINGS-047`). A change of
language reloads the page once the local copy holds it (`SETTINGS-048`). The change leaves for
the relay after the coalescing delay, as any change does. The copy opens before the relay answers,
so the reloaded page keeps the new language over an earlier one the relay replays (`SETTINGS-049`).

**The width stays on the device.** A width in CSS pixels takes another share of each screen. So
300 px is a quarter of a laptop and a tenth of a large monitor. The width lives in the local store of the
device (`SETTINGS-022`).

**A new setting.** An owner adds one declaration: a section, a key, a type and a default
(`SETTINGS-019`). A key is frozen. A new meaning or a new type takes a new key (`SETTINGS-020`).

```rust
pub enum SettingsSection { Shell, Locale, Prose, Trame, Toile, Murmure, Macros }
pub enum DocType { Prose, Trame, Toile, Murmure, Settings }  // Settings: no page shows it

/// A value of a setting: a word in the document, and a default (`SETTINGS-019`).
pub trait SettingValue: Default + Clone + PartialEq {
    fn to_word(&self) -> SettingWord;
    fn from_word(word: &SettingWord) -> Option<Self>;  // None: a word this release does not know
}

/// One entry: its section and its frozen key (`SETTINGS-020`).
pub struct Setting<V> { section: SettingsSection, key: StaticKey<SettingKeys>, value: PhantomData<fn() -> V> }

pub enum ThemeChoice { System, Light, Dark }
pub enum PanelSide { Left, Right }
pub enum PanelFold { Open, Folded }

/// What the shell reads: every setting, read whole (`SETTINGS-010`).
pub struct UserSettings { pub theme: ThemeChoice, pub panel_side: PanelSide, /* one field per setting */ }
/// What the shell writes, through `LogicCommand::ChangeSetting` (`SETTINGS-012`).
pub enum SettingChange { Theme(ThemeChoice), PanelSide(PanelSide), PanelFold(PanelApplication, PanelFold), /* ... */ }

/// The two settings a page reads before the unlock (`SETTINGS-028`).
pub struct BootHint { pub theme: ThemeChoice, pub interface_language: DocumentLocale }
```

The view receives `UiEvent::SettingsChanged` after every change, here or on another device. The
shell reads the side and the fold from it once the panel lands, and writes them through
`LogicCommand::ChangeSetting` (`SETTINGS-041`). Until the settings answer, the panel boots from the
words the device kept, so it does not move at boot.

**The budget.** The catalogue above weighs under 1 KB, against the row of
`ambre-spec-budgets.md section 6`. A burst of changes leaves as one delta per coalescing delay of
`ambre-spec-budgets.md section 8` (`SETTINGS-036`). A drag of the width writes nothing to the
relay, since the width stays on the device.

**The boundary.** The relay learns which document holds the settings of a user, from the pointer
it keeps. It learns the size and the instant of each delta, as for any document (`ZK-007`). A size
can tell which setting changed, since a client of known code writes it. That is the timing and size
metadata `ambre-spec-zero-knowledge.md` accepts (`ZK-055`).

**Code.**

* `crates/ambre-client-core/src/settings.rs` -- the catalogue, the values, the boot hint;
* `crates/ambre-logic/src/settings.rs` -- the document, its checks and the coalescing delay;
* `crates/ambre-logic/src/settings_flow.rs` -- the open, the imports, the writes, the copy and the view;
* `crates/ambre-logic/src/persistence.rs` and `crates/ambre-app/src/persist.rs` -- the kept identifier;
* `crates/ambre-sync/src/settings.rs` -- the read or the claim at each unlock;
* `crates/ambre-server/src/routes/settings.rs` and `src/db/settings.rs` -- the pointer and the claim;
* `crates/ambre-app/src/settings_view.rs` and `src/platform.rs` -- the boot and the hint;
* `crates/ambre-chrome/src/language.rs` -- the interface language of the page.

---

## 4. SURFACE

The settings document has no screen of its own. Each owner shows its settings where the user
changes them. The theme sits in the avatar menu, and the side and the fold on the panel.

| Trigger | Behaviour |
|---|---|
| a page boots | it takes the theme and the language of the boot hint, or the default theme without one |
| the account unlocks | the settings open from the relay, and replace the defaults |
| a setting changed on one device | the other signed-in devices show it at their next delta |
| a setting changed offline | the change waits in the local store and leaves once the socket is back |
| a sign-out | the boot hint goes, and the relay copy stays |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| An ordinary document with one member | a sealed blob per account, replaced by compare-and-swap | it needs a route, a table and a seal of its own, and a merge of two offline writes the client codes again by hand. No other device learns of a change until it asks. The document pipeline already holds the seal, the order, the offline queue and the push, for one member row and one log entry |
| An ordinary document with one member | the settings in the clear on the account record | the relay would read every setting, which `ZK-054` refuses |
| An ordinary document with one member | a key derived from the Master Seed for the settings alone | a third derivation domain beside the two `ZK-009` freezes, for a document the existing wrap already opens |
| One self-wrap for the life of the account | a new wrap of the settings key at each new device or recovery | the exchange key does not move, so the new wrap equals the old one. A route that replaces the copy would let a hostile relay offer a key of its own |
| A pointer the relay keeps | a document identifier derived from the Master Seed | a third derivation domain again, and an identifier derived from a secret where every other is a UUID v7. It hides nothing: a document with one member, opened first at every unlock, names itself |
| One document per user | one document per application | each application would find, create and pin its own, and a new device would open several to show one screen |
| Settings in a document | the browser's local storage | a new device starts empty, and a new device is where the user needs the settings most |
| One section per owner | one flat map for every owner | two owners could write one key, and nothing would say which owns it |
| One last-writer-wins entry per setting | one value holding every setting | two devices changing two settings would lose one of the two changes |
| The order of the CRDT decides | a wall-clock instant in each value | the clocks of two devices differ, and a device whose clock runs ahead would win every race for as long as it runs ahead |
| An error toast for a refused settings document | a modal, as for a key change | a key change asks the user for a decision (`ambre-spec-key-verification.md`). Here the client refused the document already, so a modal would stop the page for nothing to decide |
| A boot hint in the clear | the default theme until the unlock | each unlock screen would flash the wrong theme, in the wrong language, before the settings load |
| A boot hint in the clear | a sealed boot hint | no key exists before the unlock (`RESID-025`), so a sealed hint cannot be read when it is needed |
| A boot hint of two settings | every setting in the hint | the panel shows after the unlock, when the sealed copy opens. Each setting in the clear tells more to whoever holds the device |
| The width stays on the device | the width in the settings document | a width in pixels is a different share of each screen, so a width chosen on a laptop is wrong on a large monitor. A drag would also write to the relay at each release |
| One delta per coalescing delay | one delta per change | a user who folds and opens the panel five times writes five deltas, and the relay learns the rhythm of each click |
| Size and time of a delta are accepted | each settings delta padded to one size | the sealed envelope pads nothing today, so a padding is a change of the envelope for a few bits about appearance. `ZK-007` already admits the size of every delta |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The reader's locale still reads the device.** The locale and the convention are settings of the
  account, and the boot still reads the reader's locale from the browser's local storage.
* **No control returns the theme to `system`.** The theme switch has two values. A third entry of
  the avatar menu, or a long press, is the owner's to rule (`ambre-spec-design-tokens.md`).
* **The context panel is not specified here.** Its side, its width and its fold are named in the
  catalogue. The rows that read them are written with the panel itself.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What the theme changes on screen | `ambre-spec-design-tokens.md` |
| Where the panel docks, how it folds, and the bounds of its width | the shell, with its context panel |
| What the interface language, the reader's locale and the convention change | `ambre-spec-locale.md` |
| What the `murmure` section holds, and what each entry does | the Murmure feature specs |
| How a macro approval and a secret value are used | `ambre-spec-macros.md` |
| How the key of a document is wrapped for its member | `ambre-spec-membership.md` |
| How a document is held in memory and on disk | `ambre-spec-residency.md` |
| What the relay may learn of any document | `ambre-spec-zero-knowledge.md` |
