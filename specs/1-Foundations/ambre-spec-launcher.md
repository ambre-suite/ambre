# AMBRE -- FEATURE: THE LAUNCHER

**Type:** Horizontal (framework)

> The home screen at the root of the site: sign in, then every document you can open, whatever kind
> it is. Picking one opens it; picking a kind creates one.

It hosts no document content, only chrome, which is why it belongs to no application. One view
lists a spreadsheet, a board and a page, and gains the next kind without a change.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with the
other five documents of the DOM side.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-035` | `INV` | The home screen MUST list every kind of document through one view: it MUST NOT belong to an application. | this spec section 2 | `ambre_app::landing::tests::creatable_types_round_trip_through_doc_path` -- every creatable kind walks one path builder and one parser, so a kind the view named separately would not round-trip | none | verified |
| `CHROME-038` | `INV` | The kind picker MUST offer exactly the kinds whose engine can be instantiated, naming no kind of its own. | this spec section 2 | `ambre_app::landing::tests::picker_lists_creatable_types_and_reports_pick` -- the picker mounted in a real document carries one entry per creatable kind in order under its generic label, and the pick is reported | none | verified |
| `CHROME-044` | `INV` | The home screen MUST perform no cryptography: a name MUST reach it already decrypted. | `ambre-spec-zero-knowledge.md` (`ZK-001`); `UI-THREAD-004` | cargo-deny -- `deny.toml` lets only `ambre-sync` and `ambre-logic` declare `ambre-crypto`, and only the crypto wrappers declare a cipher, so the page's crate cannot name one | none | verified |
| `CHROME-080` | `INV` | A composed name MUST NOT be stored. | this spec section 2; `ambre-spec-zero-knowledge.md` | type-level -- every composition returns a `ComposedName`, with no `Debug`, no `Display`, no `AsRef<str>` and no `Into<String>`, in a crate that takes no `serde`. Its four `compile_fail` doctests show a conversion refused, and its text reaches the tab title and a view alone; `ambre_chrome::composed::tests::only_a_missing_or_blank_name_is_the_placeholder`; ci-gate -- `scripts/crate_boundaries.py` refuses the placeholder spelled outside `composed.rs`, so no composition can be rebuilt as a `String` | none | verified |
| `CHROME-036` | `OBS` | A card MUST show the document's decrypted name. | this spec section 2; `ambre-spec-zero-knowledge.md` | `ambre_app::landing::tests::a_card_shows_the_name_or_the_placeholder` -- a named card mounted and read back | none | verified |
| `CHROME-079` | `OBS` | An unnamed document MUST show a placeholder -- never an identifier, a hash, or the kind's own word. | this spec section 2; `ambre-spec-zero-knowledge.md` | `ambre_app::landing::tests::a_card_shows_the_name_or_the_placeholder` -- an unnamed card reads the placeholder, which is no creatable kind's word | none | verified |
| `CHROME-043` | `OBS` | A context that carries no kind icon MUST compose the name with the generic kind label, at the point of use. | this spec section 2 | `ambre_chrome::composed::tests::a_tab_title_composes_the_name_with_the_kind_label` -- the one function the tab title is set from | none | verified |
| `CHROME-045` | `OBS` | The sign-in panel MUST show neither route until the server has answered which one applies. | this spec section 2 | `ambre_app::landing::tests::the_sign_in_waits_for_the_answer` -- the panel mounted with no answer shows neither route, then the answer shows its own | none | verified |
| `CHROME-099` | `OBS` | The recovery words MUST be shown once, at the setup, and be readable nowhere afterwards. | this spec section 2; `ambre-spec-zero-knowledge.md` section 3 | `ambre_belt::account::tests::the_setup_shows_the_words_once_and_the_recovery_reads_them` -- browser tier, the setup shows 24 words, and after the confirmation neither the page, a reload nor the web storage holds them; `ambre_app::account::tests::the_words_show_once_and_leave_with_the_confirmation` -- wasm tier, the list reads the words in order, and the confirmation empties the face; `ambre_app::switchboard::tests::the_words_and_a_refusal_reach_the_keys_face` -- the words reach the keys face alone; type-level -- the words cross the UI thread as a `Confidential<RecoveryWords>`, and `present_words`, named in `account.rs` alone (`ARCH-049`), is the one read of them | none | verified |
| `CHROME-130` | `OBS` | The recovery form MUST show the refusal the Sync worker answers, in the words of that refusal. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-043`; `CHROME-128` | `ambre_belt::account::tests::the_setup_shows_the_words_once_and_the_recovery_reads_them` -- browser tier, a mistyped word, a wrong count and a bad checksum each show their own words; `ambre_app::account::tests::the_recovery_sends_the_words_and_shows_the_refusal` -- wasm tier, the refusal reads as its words, and one with no form shown changes nothing; `ambre_app::switchboard::tests::the_words_and_a_refusal_reach_the_keys_face` -- a refusal of the worker reaches the form | none | verified |
| `CHROME-131` | `OBS` | The setup MUST show the words of a refused passphrase, its reason included, in a line a screen reader announces. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-036`; `ambre-spec-accessibility.md` | `ambre_app::keys_gate::view::tests::a_weak_setup_passphrase_is_announced_with_its_reason` -- wasm tier, the alert line reads the words of the refusal, and each field names it as its description; `ambre_app::switchboard::tests::a_refused_passphrase_reads_on_the_gate_that_asked` -- the refusal reaches the gate, and a toast when no gate shows | none | verified |
| `CHROME-136` | `OBS` | A passphrase the Sync worker refuses, wrong or weak, MUST raise an error toast in the words of the refusal, beside the line of the face that asked. A screen reader MUST NOT announce that toast: the alert line of the face is the one announcement. | this spec section 2; `CHROME-097`; `CHROME-131`; `ambre-spec-accessibility.md`, `CHROME-006` | `ambre_app::switchboard::tests::a_refused_passphrase_raises_a_toast_no_screen_reader_announces` -- a wrong and a weak passphrase on the gate or a form each raise one visual error toast in their words, and a refusal of the recovery words reads on the form alone; `ambre_chrome::chrome_ui::a_visual_toast_is_hidden_from_a_screen_reader` -- it reads the hidden state of a visual and of an announced toast back off a real `document`; `ambre_bus::event::tests::only_a_wrong_or_weak_passphrase_refuses_the_passphrase` -- the match on the failure has no wildcard, so a new failure takes a decision | none | verified |
| `CHROME-137` | `OBS` | The recovery words MUST show in the shared modal over the launcher, and only their confirmation MAY close it: Escape and the backdrop MUST NOT. | this spec section 2; `CHROME-099`; `CHROME-132`; `ambre-spec-accessibility.md`, `CHROME-028` | `ambre_app::account::tests::the_words_open_in_a_modal_that_only_their_confirmation_closes` -- wasm tier, the words show in an open modal that leaves the launcher in place, a native close shows them again, and the confirmation removes the modal; `ambre_chrome::chrome_ui::an_explicit_modal_closes_on_its_signal_alone` -- wasm tier, a modal only its caller closes refuses the cancel of Escape, ignores the backdrop and opens again after a native close | none | verified |
| `CHROME-134` | `OBS` | A refusal of the recovery form opened from the unlock face MUST read on that form, never on the hidden gate. | this spec section 2; `CHROME-130`; `CHROME-133` | `ambre_app::switchboard::tests::the_words_and_a_refusal_reach_the_keys_face` -- a refusal with the gate and a form shown reaches the form; `ambre_belt::account::tests::the_unlock_face_leads_to_a_recovery_that_opens_the_account` -- browser tier, a bad checksum reads on the form opened from the unlock face | none | verified |
| `CHROME-144` | `OBS` | Each face of the keys gate MUST name the account the provider signed in, by the name and the address it gave, and the setup MUST NOT ask for either. | this spec section 2; `CHROME-096`; `CHROME-098` | `ambre_app::switchboard::tests::a_provider_sign_in_names_its_account_on_the_gate` -- a provider sign-in hands the gate its name and address before the face; `ambre_app::keys_gate::view::tests::each_face_names_the_provider_account_and_asks_for_no_identity` -- wasm tier, both faces name the account, hold no field but passphrases, and the setup sends the passphrase alone | none | verified |
| `CHROME-143` | `OBS` | The passphrase form of the account settings MUST show each refusal the Sync worker answers, in the words of that refusal. | this spec section 2; `ambre-spec-sync.md`, `SYNC-252`, `SYNC-295`; `CHROME-130` | `ambre_app::account::tests::the_passphrase_change_sends_both_passphrases_and_shows_each_refusal` -- wasm tier, a wrong, a weak and a stale refusal each read on the form; `ambre_app::switchboard::tests::a_passphrase_change_closes_its_form_or_reads_its_refusal` -- each refusal reaches the form | none | verified |
| `CHROME-135` | `INV` | The development panel MUST link to the recovery form only while a session holds: signed out, a recovery fails as not signed in. In a session on a development relay, the panel MUST show that link, and no sign-in field. | this spec section 2; `CHROME-128`; `CHROME-133` | `ambre_app::landing::tests::the_recovery_link_shows_only_in_a_session` -- wasm tier, the panel signed out shows the setup link and no recovery link, and in a session the recovery link and no sign-in field; `ambre_belt::account::tests::the_setup_shows_the_words_once_and_the_recovery_reads_them` -- browser tier, the panel after a reload shows no recovery link, and the recovery runs from the unlock face; `ambre_belt::account::tests::a_setup_ends_in_the_session_once_the_words_are_confirmed` -- browser tier, the session the setup ends in shows the recovery link | none | verified |
| `CHROME-123` | `OBS` | The search field MUST carry an accessible name inside a search landmark, and the count a query lists MUST be announced to a screen reader. | this spec section 2; `ambre-spec-accessibility.md` | `ambre_app::landing::tests::the_search_field_is_named_announces_its_count_and_clears_on_escape` -- wasm tier, the name, the landmark and the status line read back: it says nothing before a search, then two documents, one, and that nothing matches | none | verified |
| `CHROME-124` | `OBS` | A query that answers nothing MUST say so, never the invitation an empty list shows. | this spec section 2 | `ambre_app::landing::tests::a_search_filters_the_bands_and_sends_nothing` -- wasm tier, the one message shown is the search's own | none | verified |

### 1.2 Resolve a path

The address bar hands over a string. It becomes the home screen or one document, and nothing else.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-040` | `POST` | A path MUST resolve to the home screen or to exactly one document. | this spec section 2 | `ambre_app::route::tests::root_is_landing`; `::each_doc_type_slug_parses`; `::path_doc_roundtrip`; `::landing_route_has_no_doc` | none | verified |
| `CHROME-082` | `FRAME` | Resolving a path MUST NOT open or close a document. | derived | type-level -- the parser is a pure function from a string to a route and holds no handle to a document, a thread or a surface, so a resolution with a side effect has nothing in scope to act on | none | verified |
| `CHROME-081` | `FAIL` | An unknown or malformed path MUST resolve to the home screen. | this spec section 2 | `ambre_app::route::tests::unknown_slug_falls_back_to_landing`; `::malformed_id_falls_back_to_landing`; `::trailing_and_extra_segments_tolerated` | none | verified |

### 1.3 Open a document, or go back home

The route changed. Either the booted engine can draw what was asked for, or the page starts again.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-039` | `POST` | An open whose render kind equals the booted one MUST swap in place. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-010` | `ambre_app::route::tests::cross_kind_open_needs_reload` -- the two of its four combinations where the kinds agree | none | verified |
| `CHROME-083` | `POST` | An open whose render kind differs from the booted one MUST reload the page. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-010` | `ambre_app::route::tests::cross_kind_open_needs_reload` -- the two combinations where they differ, one engine being booted per page, so this is the rule that keeps a board from being drawn by a grid | none | verified |
| `CHROME-042` | `FRAME` | Going back home MUST NOT unmount the content surface: it MUST be covered and frozen. | this spec section 2; `ambre-spec-architecture.md`, `ARCH-010` | `ambre_canvas::frame::tests::a_closed_modal_resumes_only_what_it_suspended` -- a route away stops the loop, and a modal closing over it does not restart it; `ambre_canvas::frame::tests::a_paused_loop_relays_and_does_no_frame_work` -- a stopped loop does no frame work | none | verified |
| `CHROME-116` | `POST` | Opening a page MUST let its canvas render, a Murmure page included, though it binds no document. | this spec section 2; `CHROME-042` | `ambre_app::route::tests::a_murmure_page_renders_and_home_freezes` -- wasm tier, a Murmure page and a document page render, and home alone freezes; `ambre_belt::murmure::tests::a_message_one_member_sends_reaches_the_other` -- browser tier, the composer of a Murmure page is drawn and takes the keys | none | verified |
| `CHROME-104` | `POST` | An open that failed MUST end its loading screen and tell the user the document could not open. | this spec section 2; `ambre-spec-message-bus.md`, `BUS-030` | `ambre_app::switchboard::tests::a_failed_open_ends_the_loading_screen` -- the active document's failed open lowers the screen and raises a notice, and another document's leaves it | none | verified |
| `CHROME-127` | `FAIL` | Given an open that failed on a key no member signed, the notice MUST name a security fault, as an error, never as the warning of a failed open. | this spec section 2; `CHROME-104`; `ambre-spec-membership.md`, `SYNC-212` | `ambre_app::switchboard::tests::a_key_no_member_signed_is_shown_as_a_security_fault` -- the failed open raises an error that names a security fault, and a key not received keeps the plain warning | none | verified |

### 1.4 Create a document

The user picks a kind. Nothing is named.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-037` | `POST` | A new document MUST be created unnamed. | this spec section 2; `ambre-spec-zero-knowledge.md` | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses `CreateDocument`, the command that carries a name, named in any UI-thread file. It reads the body `trigger_lazy_create` builds by its type, refuses it unless its `name_enc` is the `None` of `Option`, and the judge refuses a workspace where no unit records it (`ambre_primitives::driver::chrome::an_invitation_or_a_named_create_under_a_re_export_is_refused_on_the_ui_thread`, `ambre_primitives::driver::chrome::an_open_time_create_that_sends_a_name_is_refused_and_an_unnamed_one_is_a_fact`, `ambre_primitives::boundaries::chrome::tests::an_open_time_create_no_unit_records_fails_closed`) | none | verified |
| `CHROME-084` | `FRAME` | Creating a document MUST NOT store a placeholder as its name. | this spec section 2; `ambre-spec-zero-knowledge.md` | ci-gate -- the same driver check as `CHROME-037`: the open-time create sends `None` as its name, so no placeholder can ride it | none | verified |

### 1.5 Delete a document

There is no recovery, so the gesture is gated on the user reading the name back.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-041` | `FAIL` | A delete MUST be confirmed against the document's own name before anything is dispatched. | this spec section 2; README pillar "E2EE" | `ambre_app::landing::tests::a_delete_waits_for_the_confirmation_on_the_name` -- the card's delete dispatches nothing, the modal names the document, and only its confirm dispatches | none | verified |

### 1.6 Sign in, then unlock or set up

The provider says who the person is; a passphrase opens their keys, or a setup creates them.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-095` | `POST` | The provider control MUST hand the browser to the authorization address the relay answers. | this spec section 2; `ambre-spec-session.md`, `SRV-172` | `ambre_app::landing::tests::the_provider_control_asks_for_the_address` -- the control sends one request for the address and leaves the page where it is; `ambre_app::signin::tests::the_provider_address_is_followed_and_its_state_kept` -- the answered address is followed, and one that is not `https` is not; `ambre_core::provider::tests::a_provider_parses_from_its_word_alone` -- a word that could reach another relay route names no provider, and the bus carries a provider only as one of the closed set | none | verified |
| `CHROME-139` | `FAIL` | Given an authorization address off the authorization host of every known provider, the sign-in MUST NOT follow it. | this spec section 3; `CHROME-095` | `ambre_app::signin::tests::the_provider_address_is_followed_and_its_state_kept` -- browser tier: an address on another host, on a host that only starts with the provider's, behind a user part or on another port is refused and keeps no state; `ambre_core::provider::tests::each_provider_pins_the_host_of_its_authorization_endpoint` -- each provider names its one host | none | verified |
| `CHROME-096` | `POST` | A return from the provider MUST establish the session on the live threads, its tokens reaching the Sync worker alone. | this spec section 2; `ambre-spec-sync.md`, `SYNC-060` | `ambre_app::signin::tests::a_provider_return_is_read_from_its_address` -- the page reads the code and the state and hands them on; `ambre_sync::auth::provider_tests::a_provider_sign_in_keeps_its_tokens_on_the_worker` -- the worker stores both tokens, and what Logic hears names the person with no byte of a token | none | verified |
| `CHROME-101` | `FAIL` | A return from the provider MUST be refused unless it carries the state this tab was handed. | this spec section 2; `ambre-spec-session.md`, `SRV-022` | `ambre_app::signin::tests::a_return_this_tab_did_not_start_is_refused` | none | verified |
| `CHROME-097` | `POST` | An account that holds its keys MUST be asked for its passphrase before the list is shown. | this spec section 2; `ambre-spec-session.md` section 3 | `ambre_app::switchboard::tests::a_provider_account_is_asked_to_unlock_or_to_set_up` -- a sign-in whose keys are stored asks the unlock, and asks no list; `ambre_app::keys_gate::view::tests::an_unlock_sends_the_passphrase_once_and_empties_the_field` -- wasm tier, one field sends the unlock once; `::a_wrong_passphrase_is_said_and_the_face_stays` -- a refused unlock reads its words and keeps the face | none | verified |
| `CHROME-098` | `POST` | An account that owes its key setup MUST be shown the setup, never an error, whether the sign-in or a later refusal reveals it. | this spec section 2; `ambre-spec-sync.md`, `SYNC-134`, `SYNC-255` | `ambre_app::switchboard::tests::a_provider_account_is_asked_to_unlock_or_to_set_up` -- a sign-in that owes its keys and a setup owed, from an unlock that finds none or a refused request, each ask the setup and raise no notice; `ambre_app::keys_gate::view::tests::a_setup_sends_only_a_confirmed_passphrase` -- wasm tier, two fields that agree send the setup once; `ambre_sync::auth::tests::a_key_setup_stores_a_seed_that_a_later_unlock_opens` -- the worker stores the keys on the signed-in account; `ambre_app::account::tests::the_setup_sends_the_passphrase_and_empties_its_fields` -- wasm tier, the setup form of the development panel sends one setup | none | verified |
| `CHROME-100` | `FRAME` | A passphrase typed on this screen MUST NOT outlive its submission on the UI thread. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-010` | type-level -- the gate's state `KeysGate` holds its face and its refusal alone, and the text is read once from the field into a `Confidential`, which wipes it on drop; `ambre_app::keys_gate::view::tests::an_unlock_sends_the_passphrase_once_and_empties_the_field`; `::a_setup_sends_only_a_confirmed_passphrase` -- wasm tier, every field reads empty after a submission, a refused one included | none | verified |
| `CHROME-129` | `FRAME` | A passphrase typed in the setup, the recovery or the passphrase form MUST NOT outlive its submission on the UI thread. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-010`; `CHROME-100` | `ambre_app::account::tests::the_setup_sends_the_passphrase_and_empties_its_fields` -- wasm tier, both fields read empty after a submission, a refused one included; `ambre_app::account::tests::the_recovery_sends_the_words_and_shows_the_refusal` -- wasm tier, the same on the recovery form; `ambre_app::account::tests::the_passphrase_change_sends_both_passphrases_and_shows_each_refusal` -- wasm tier, the three fields of the passphrase form read empty after a submission; type-level -- the text moves from the field into a `Confidential<Passphrase>`, the type the bus carries (`ZK-010`) | none | verified |
| `CHROME-128` | `POST` | The recovery form MUST hand the words it reads to the Sync worker. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-043` | `ambre_belt::account::tests::the_setup_shows_the_words_once_and_the_recovery_reads_them` -- browser tier, the worker refuses a mistyped word, a wrong count and a bad checksum, and passes the words the setup showed; `ambre_app::account::tests::the_recovery_sends_the_words_and_shows_the_refusal` -- wasm tier, the form sends one recovery | none | verified |
| `CHROME-132` | `POST` | An account setup MUST end in the session of the account it created: once the recovery words are confirmed, the list MUST show, never the sign-in panel. | this spec section 2; `ambre-spec-sync.md`, `SYNC-261`; `CHROME-099` | `ambre_belt::account::tests::a_setup_ends_in_the_session_once_the_words_are_confirmed` -- browser tier, the words show in a modal over the list of the new user, and their confirmation leaves that list with no sign-in panel | none | verified |
| `CHROME-133` | `POST` | The unlock face MUST link to the recovery form, and a recovery there MUST open the list on the recovered keys, never the unlock face again. | this spec section 2; `ambre-spec-zero-knowledge.md`, `ZK-043`; `CHROME-097`; `CHROME-128`; `ambre-spec-sync.md`, `SYNC-293` | `ambre_app::keys_gate::view::tests::the_unlock_face_links_to_the_recovery_form` -- wasm tier, the unlock face links to the form and the setup face does not; `ambre_app::switchboard::tests::the_words_and_a_refusal_reach_the_keys_face` -- the answer of a recovery closes the form; `ambre_belt::account::tests::the_unlock_face_leads_to_a_recovery_that_opens_the_account` -- browser tier, a provider account recovers from the unlock face, and the list then shows with no gate | none | verified |
| `CHROME-142` | `POST` | Each face of the keys gate MUST offer a sign-out, which ends the session on the live threads. | this spec section 2; `CHROME-107` | `ambre_app::keys_gate::view::tests::each_face_offers_a_sign_out` -- wasm tier, the unlock and the setup faces each send one sign-out, however many clicks | none | verified |

### 1.7 Sign out

The session ends where it was opened: on this screen. So does a change of the account passphrase.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-107` | `POST` | A sign-out control MUST end the session on the live threads and show the sign-in panel. | this spec section 3; `ambre-spec-session.md`, `SRV-026`; `ambre-spec-sync.md`, `SYNC-066`, `SYNC-177` | `ambre_app::landing::tests::the_sign_out_control_ends_the_session_once` -- the control sends one sign-out however often it is clicked, and the page stays put until the worker confirms; `ambre_app::signin::tests::a_sign_out_forgets_the_user_and_boots_home_afresh` -- the confirmation boots `/`, where no user is left to sign in | none | verified |
| `CHROME-108` | `FRAME` | A sign-out MUST leave nothing of the session it ended on screen: no list, no open document, no draft. | this spec section 3; `MURMURE-216` | `ambre_app::signin::tests::a_sign_out_forgets_the_user_and_boots_home_afresh` -- the page is replaced by a fresh boot, which holds no state of the one it replaced, and that boot signs no one in; `ambre_murmure::ui::drafts::tests::a_sign_out_deletes_every_draft` | none | verified |
| `CHROME-145` | `POST` | The passphrase form of the account settings MUST send the current and the new passphrase to the Sync worker. | this spec section 2; `ambre-spec-sync.md`, `SYNC-251`; `CHROME-129` | `ambre_app::account::tests::the_passphrase_change_sends_both_passphrases_and_shows_each_refusal` -- wasm tier, one change carries both passphrases and empties the three fields, and refused fields send nothing; `ambre_app::switchboard::tests::a_passphrase_change_closes_its_form_or_reads_its_refusal` -- the answer closes the form with a notice | none | verified |

### 1.8 Order, group and search the list

The list holds every document the Sync thread listed, names decrypted. The user reads it most recent
first, in bands of age, and narrows it by a query that never leaves the page.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-118` | `POST` | The list MUST order documents by the relay's last write, the most recent first, then by name, a named document before an unnamed one, then by identifier. | this spec section 2 | `ambre_app::doc_list::tests::the_most_recent_write_comes_first_and_a_tie_is_broken_by_name` -- a tie on the write reads the folded name, then the identifier, and the reversed input reads the same; `ambre_app::landing::tests::a_search_filters_the_bands_and_sends_nothing` -- wasm tier, the mounted list reads its cards in that order | none | verified |
| `CHROME-119` | `POST` | The list MUST group documents by the age of their last write: the past day, week and month, older, then those no relay listed. | this spec section 2 | `ambre_app::doc_list::tests::each_document_lands_in_the_band_of_its_age` -- each band at its bounds, in display order; `::a_write_ahead_of_the_reader_clock_is_in_the_first_band`; `::each_band_has_a_heading_of_its_own`; `ambre_app::landing::tests::a_search_filters_the_bands_and_sends_nothing` -- wasm tier, the mounted list shows each band under its heading | none | verified |
| `CHROME-126` | `POST` | The list MUST NOT show a group that holds no document. | this spec section 2; `CHROME-119` | `ambre_app::doc_list::tests::an_empty_band_is_not_shown` -- two documents far apart show two bands, and an empty list none; `ambre_app::landing::tests::a_search_filters_the_bands_and_sends_nothing` -- wasm tier, the mounted list shows the headings of its two bands alone, and a query that keeps nothing shows no heading | none | verified |
| `CHROME-120` | `POST` | A query MUST list exactly the documents whose name contains it in any case, in the order and the groups of the whole list; a blank query MUST list every document. | this spec section 2; `CHROME-008` | `ambre_app::doc_list::tests::a_query_keeps_the_names_that_contain_it_in_list_order` -- the case, the surrounding blanks and the bands; `ambre_chrome::palette::tests::a_query_matches_any_case_and_any_position` -- the one matching rule the list shares with the command palette; `ambre_app::landing::tests::a_search_filters_the_bands_and_sends_nothing` -- wasm tier, a query typed in the field | none | verified |
| `CHROME-121` | `POST` | An unnamed document MUST answer the blank query alone: the placeholder it shows is no name. | this spec section 2; `CHROME-079` | `ambre_app::doc_list::tests::an_unnamed_document_answers_only_the_blank_query` -- an absent and an empty name, against the placeholder's own word | none | verified |
| `CHROME-122` | `FRAME` | Ordering, grouping and searching the list MUST NOT send anything off the UI thread, and MUST NOT open or decrypt a document. | this spec section 2; `ambre-spec-zero-knowledge.md` (`ZK-001`); `CHROME-044` | type-level -- `doc_list::arrange` is a pure function of the listed rows, the query and the clock, and holds no outbox, no handle and no key, so a search has nothing in scope to send or to open; `ambre_app::landing::tests::a_search_filters_the_bands_and_sends_nothing` -- wasm tier, the list's outbox hears nothing while queries are typed | none | verified |
| `CHROME-125` | `POST` | Escape in the search field MUST empty the query and list every document again. | this spec section 2 | `ambre_app::landing::tests::the_search_field_is_named_announces_its_count_and_clears_on_escape` -- wasm tier, the key empties the field, the three cards return and the status goes quiet | none | verified |

* **Budget.** none of its own: the list is a handful of rows of DOM and `ambre-spec-budgets.md`
  budgets no chrome operation. The frame the home screen does not draw is a saving, charged to
  `ambre-spec-frame.md`.
* **Boundary.** the routing metadata the list shows is plaintext by design, and the name beside it
  is not: it arrives decrypted from the Sync thread (`CHROME-044`). The launcher opens one
  unauthenticated same-origin read of the sign-in configuration, keeps the sign-in's own state and
  owns the local store. Each is an exception `UI-THREAD-004` names.
* **Verification.** 53 `verified`. What holds is what a pure function or one mounted control
  carries, and what a scan of the source refuses. Creation is held by the gate that keeps the named
  command off the page. A composed name is held by its type, which converts to no `String`
  (`CHROME-080`).
---

## 2. MODEL

**CRDT containers.** none. The launcher reads a list the Sync thread built and sends commands; it
opens no document.

**The faces, and the session that picks one.** The screen is a full-bleed overlay over the content
surface, and which face it shows is decided by the state of the session.

| Session | Face |
|---|---|
| unknown, and the server has not answered | a neutral placeholder (`CHROME-045`) |
| unknown, and the server reports a development mode | a name field that signs in without reloading the page, and two links to the setup and to the recovery |
| unknown, otherwise | a single button that hands the browser to the identity provider (`CHROME-095`) |
| signed in with the provider, keys stored | the account the provider named (`CHROME-144`), a passphrase field that opens its keys (`CHROME-097`), a link to the recovery form (`CHROME-133`) and a sign-out (`CHROME-142`) |
| signed in with the provider, keys owed | the account the provider named (`CHROME-144`), the setup: a passphrase, then the recovery words once (`CHROME-098`, `CHROME-099`), and a sign-out (`CHROME-142`) |
| known | the header, the kind picker and the document list |

**Why a provider sign-in is not yet a session.** The provider vouches for who the person is, and
the relay answers a token pair for that. Neither can open a key. A development sign-in derives its
keys from the name, so it skips both middle faces.

The return from the provider is exchanged on the Sync worker, never on this thread. The answer to
that exchange carries both tokens, and they live in that worker alone (`CHROME-096`). The page only
reads the code from its address, then replaces the address so the code leaves the history.

**Whose return it is.** The provider comes back to `/signin/<provider>`, which the deployment
registers as its redirect address. The relay checks that it issued the state, not that this browser
asked for it. So the tab keeps the state it was handed in its own session storage, and drops a
return that carries another (`CHROME-101`). Without it, a link forged by a third party signs the
person into the third party's account. The address the relay answers is followed only if it is
`https`: a `javascript:` one would run in this page.

The address must also lie on the authorization host of a provider the client knows. Otherwise a
relay shows a page of its choice where the person expects the provider's password prompt
(`CHROME-139`).

**The keys gate.** The two middle faces share one gate, which stands before the sign-in panel and
the list while it shows.

The passphrase lives in the field alone. The gate reads it once, at
submission, into a wrapper that wipes it, and empties the field in the same step (`CHROME-100`).
The setup takes the passphrase twice, by the rule of the seal prompt. A typo there leaves the
recovery words as the only way in.

A refusal, the worker's or the fields' own, fills one alert line
that each field names as its description (`CHROME-131`). The worker's words for a weak passphrase
name the reason, and a screen reader announces them where a sighted user reads them.

Each face names the account the provider signed in, by the name and the address the provider gave
(`CHROME-144`). The relay already holds both, so the setup asks for neither. A person who signed in
with the wrong account sees it, and each face offers a sign-out (`CHROME-142`). The header, which
holds the other sign-out, shows only once a session holds.

**The way back in.** The unlock face links to the recovery form, which covers the gate
(`CHROME-133`). A refusal of the form reads on the form, because the gate under it is hidden
(`CHROME-134`). A recovery re-seals the seed under the new passphrase, then opens the session on
that seed (`ambre-spec-sync.md`, `SYNC-293`). The form closes and the list shows, with no second
unlock (`CHROME-133`).

**Changing the passphrase.** The header of a session links to the passphrase form of the account
settings, which covers the launcher. The form takes the current passphrase, then the new one twice.
It sends both to the Sync worker, which re-seals the seed (`CHROME-145`, `SYNC-251`). The answer
closes the form.

Each refusal reads on the form in its own words (`CHROME-143`). A refusal names a wrong current
passphrase, a weak new one, or a seal another device changed meanwhile. A stale seal asks for a new
try (`ambre-spec-sync.md`, `SYNC-295`).

**A setup ends in a session.** The Sync worker answers a setup with the recovery words, then
announces the session of the user the relay assigned (`SYNC-261`). The words cover the list until
the person confirms them, and the list of the new account then shows (`CHROME-132`).

The development route matters for a reason that is not convenience. It signs in on the live
threads, so the Sync worker keeps the keys it already holds instead of losing them to a page
reload.

**Signing out.** The control sits beside the user's name in the header, and sends one sign-out to
the live threads. The Logic thread drops every document. The Sync worker clears its keys, then asks
the relay to revoke the refresh token (`SYNC-177`). Its confirmation deletes the drafts
(`MURMURE-216`). Once that write has landed, the page forgets who it signed in and boots `/` afresh
(`CHROME-108`).

**Why it reloads.** A sign-out in place would have to empty each list, panel and sidebar an
application fills, and the next application would forget one. The fresh boot empties all of them,
at the cost of one boot per sign-out. It also ends the Sync worker, so no byte of a key outlives
the session in that worker's memory.

**A card.** One row per document, and four things on it.

| Field | Source | Note |
|---|---|---|
| the kind tile | the document's kind | one monogram per kind, coloured. It stands in for an icon the icon pipeline does not build yet |
| the name | the decrypted title | absent means never named, never opened, or no key yet -- all three render the placeholder (`CHROME-036`) |
| the modification time | the server's own timestamp | relative wording, and routing metadata rather than content |
| the shared badge | more than one member | a member count is routing metadata too |

Each card carries an inline rename and a delete. The delete is irreversible: there is no
server-side recovery of a ciphertext whose keys are gone. It passes through a modal that names the
document first (`CHROME-041`).

**The order, the bands and the search.** The list arranges the rows the Sync thread listed, and
nothing else. Each row already carries its decrypted name and the relay's time of its last write.
So the arrangement opens no document, decrypts nothing more and sends no request (`CHROME-122`).

| Step | Rule | Reason |
|---|---|---|
| order | the last write, the most recent first; then the name folded to lower case, a named document first; then the identifier (`CHROME-118`) | the document a user returns to is the one they last touched. The two tie-breaks make two listings of one set read the same, whatever order the relay or the local store answered in |
| bands | the past day, the past week, the past month, older, then the documents no relay listed; an empty band is not shown (`CHROME-119`, `CHROME-126`) | the bands measure elapsed time on the reader's clock, as the relative time of a card does. They need no time zone and never contradict the card |
| search | the trimmed query, contained in the name in any case: the rule of the command palette (`CHROME-120`) | one matching rule for the suite. The query is free text on the UI thread. The relay never sees it, and could match nothing, since it holds no name |
| unnamed | an unnamed document answers the blank query alone (`CHROME-121`) | its placeholder is rendered, never stored as a name. A query for the placeholder's word would match a name that does not exist |

A query keeps the order and the bands of the whole list, so a result sits where the user finds it
without a search. A query that answers nothing says so, in words that differ from the invitation of
an empty list (`CHROME-124`).

The field is a search landmark with its own accessible name. One status line under it is a polite live
region. It holds the invitation of an empty list, the count a query lists, or the news that the
query answers nothing (`CHROME-123`). It is empty, and hidden, while no query is typed. Escape
empties the query (`CHROME-125`).

**Why one line, and not a section per band.** On 2026-11-14, `just size` gave Main 3.6 KB of room
under its ceiling. A memo, a section per band and a message apart from the status cost 25 KB raw.
One flat list, the heading on the first row of its band, and one status line fit the room.

**Opening one, and the one predicate that decides how.** The view a page boots with is fixed for the
life of that page. It can be handed another document, but not another kind of document.

1. the target path is built from the kind and the identifier;
2. one predicate compares the target's render kind with the booted one;
3. same kind -- the router pushes the path and the surface swaps the document in place, keeping the
   worker and its session;
4. different kind -- the browser navigates, and the page boots the right view.

The cards and the kind picker share that predicate, so creating and opening cannot disagree
(`CHROME-039`).

**An open that fails.** The loading screen comes down, and a notice says the document could not
open (`CHROME-104`). One cause changes the notice: a key no member signed. The relay chose that key,
so the notice is an error that names a security fault, and never the warning of a wait
(`CHROME-127`).

**Creating one.** The picker is derived from the list of kinds whose engine can actually be
instantiated. It never names a kind, and gains the next one for free (`CHROME-038`). Picking a
kind mints an identifier on the client and opens the document at once. There is no name prompt: the
prompt is friction on every single creation, and the honest default is an unnamed document.

**Opening one from a file.** The same two steps in the other order. The file is read first, because
its header names the application, and only then is a path built and navigated to. So this screen
holds the gesture and owns none of its rules. The header read, the migration, the refusals and the
hand-off through the local snapshot store all belong to `ambre-spec-docfile.md section 4`.

One thing is shared rather than restated: the predicate that says which kinds have an engine. The picker
offers those kinds and the file path refuses the others. A file cannot open a document the screen
would not have let the user create (`CHROME-038`).

**Naming, and why the default is nothing.**

| Rule | Reason |
|---|---|
| unnamed until the user renames | a stored placeholder is indistinguishable from a user who typed it, and "is this named yet" stops being answerable |
| no counter, such as a second document numbered two | counter state collides across offline devices, because each client holds its own decrypted list and no server has authority over a name |
| no kind word inside the name | the tile already carries the kind, and repeating it is what this design removes |
| a context with no tile composes at the point of use | the browser tab reads a name and a generic kind label together, composed on the spot and stored nowhere (`CHROME-043`) |

The generic label is the kind described plainly: a spreadsheet, a whiteboard, a document. It is
deliberately not the product name for that kind, which is what the tile carries.

**Code:** `crates/ambre-app/src/landing.rs` holds the overlay, the cards, the search field, the picker
and the sign-in panel. `crates/ambre-app/src/doc_list.rs` holds the order, the bands and the
search, as one pure function.

`crates/ambre-app/src/keys_gate.rs` holds the unlock and the setup faces.
`crates/ambre-app/src/route.rs` holds the path mapping and the reload predicate.
`crates/ambre-app/src/lib.rs` holds the switchboard closure that fills the list.
`crates/ambre-app/src/route_binding.rs` pauses the loop behind the overlay, and
`crates/ambre-app/src/tab_title.rs` keeps the tab title in step. `crates/ambre-core/src/doc_type.rs`
holds the kinds, their monograms, their generic labels and the creatable list. The style is
`crates/ambre-app/styles.css`.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| the site is opened at its root | the overlay covers a frozen content surface, and the sign-in panel or the list appears |
| the server has not answered yet | a neutral placeholder, and no sign-in control that might be the wrong one |
| a name is entered in the development panel | the session is established on the live threads, with no page reload |
| the identity provider button | the browser leaves for the provider and comes back signed in (`CHROME-095`, `CHROME-096`) |
| a return this tab did not start | it is dropped, its address cleared, and the sign-in panel shows again (`CHROME-101`) |
| a provider account holds its keys | a passphrase field opens them, then the list appears (`CHROME-097`) |
| the recovery link of the unlock face | the recovery form; once it succeeds, the list of the account shows (`CHROME-133`, `CHROME-134`) |
| a provider account owes its keys, or the relay refuses it for that later | the setup: a passphrase, then the recovery words, shown once (`CHROME-098`, `CHROME-099`) |
| the recovery words are confirmed | the words leave the page and the view, and no reload shows them again (`CHROME-099`). The list of the new account shows (`CHROME-132`) |
| the recovery form is submitted | the words reach the Sync worker, and a refusal shows its own words: a mistyped word, a wrong count or a bad checksum (`CHROME-128`, `CHROME-130`) |
| a setup passphrase the worker refuses as weak | the line under the fields says why, and a screen reader announces it (`CHROME-131`) |
| the sign-out control | the session ends, and the sign-in panel shows over nothing of it (`CHROME-107`, `CHROME-108`) |
| the sign-out of a keys gate face | the session ends, and the sign-in panel shows (`CHROME-142`) |
| the passphrase control of the header | the passphrase form; once the change lands, the form closes with a notice, and a refusal reads on the form (`CHROME-145`) |
| the list is empty | a placeholder invites creating one, rather than a bare empty list |
| the list is shown | the documents, most recently written first, under the heading of their band (`CHROME-118`, `CHROME-119`) |
| a query is typed in the search field | the list keeps the documents whose name contains it, in their order and bands, and the count is announced (`CHROME-120`, `CHROME-123`) |
| a query answers nothing | a message says that no name contains it (`CHROME-124`) |
| Escape in the search field | the query empties, and every document is listed again (`CHROME-125`) |
| a card is clicked | the document opens, in place or after a reload depending on its kind |
| the kind picker is opened | one entry per creatable kind, each under its generic label |
| a kind is picked | a document of that kind opens immediately, unnamed |
| the open-a-file control is used | a picker accepts an `.ambre` file, and the document it carries opens -- in the application the file's own header names, not in the one the picker was clicked from. The rule and its refusals belong to `ambre-spec-docfile.md section 4`; what this screen owns is holding the gesture, because at the moment of picking no document is open and no engine is booted |
| a card is renamed inline | the new name is sent, encrypted by the Sync thread, and the card updates |
| a card's delete is chosen | a modal names the document and asks; confirming removes the card at once |
| a document is open and renamed from its header | the same path as the card's rename |
| the tab is looked at | the document's name and its generic kind label, or the product name on the home screen |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The home screen is app-agnostic and lives in the launcher | a home screen per application | it lists documents of every kind, so one per application means every application maintains a list of the others' documents and a new kind has to be added to each. |
| The home screen is app-agnostic and lives in the launcher | one owned by whichever application shipped first | it would make one application's crate the owner of the others' entry point. Kept in the launcher, a new kind appears in the list by being creatable. |
| A new document is unnamed, with no prompt | a name prompt on create | naming is the one thing the user cannot do usefully before there is content, and asking is friction on every single creation. An unnamed document is a decidable state that a rename resolves whenever the user is ready. |
| A new document is unnamed, with no prompt | a generated default name | it is indistinguishable from a name the user chose, so nothing downstream can tell an unnamed document from a named one, and the user is left deleting a title rather than writing one. |
| A placeholder is rendered, never stored | writing the placeholder into the stored name | a stored placeholder is a name as far as every other layer can tell: the ciphertext holds it, the list shows it, and nothing can distinguish it from a deliberate title. Rendering it keeps the absence of a name a fact rather than a guess. |
| No counters in a default name | numbering new documents per kind | there is no server authority over names, and each device decrypts its own list, so two offline devices both create the same numbered name. Deletions then leave holes the counter never reclaims. |
| A cross-kind open reloads the page | swapping the view at run time | it means every view is always linked and always mountable. The booted view is a closed choice made at page load, and swapping a document inside it cannot change which view is mounted: a reload is honest and costs a session the worker rebuilds, while a push that leaves the wrong view mounted is a blank document. |
| The content surface is covered, never unmounted | unmounting the canvas while the home screen is shown | it destroys the GPU surface, the offscreen pair and the atlases, and rebuilding them is the slowest thing the client does. Covering it and pausing the loop costs a paused frame. |
| The card shows an icon, a name and a date | encrypted thumbnails on the cards | a preview of an end-to-end encrypted document is a whole pipeline: an offscreen render on save, an invalidation policy across peers, an extra encrypted object per document, and a decrypt of every image when the list loads. The gain over a tile and a name is cosmetic. |
| The list shows no content statistics | counts of cells, words or pages on the cards | a modification time, a role and a member count are routing metadata the server already holds in plaintext and hands over for free. A word count or a cell count is content, so it must be computed on the client and stored encrypted -- a second coherent object to maintain, for a number. |
| The worker exchanges the provider's code | the page posting the code and handing the tokens on | the answer to that exchange carries both tokens. Posted from the page, they would sit on the UI thread, which `SYNC-060` forbids. The page reads the code from its address and passes it on |
| The tab keeps the state it was handed | trusting the relay's check of the state | the relay knows it issued the state, not to whom. A third party starts a sign-in of their own and sends the link back to the person, who is then signed in as the third party. |
| The kind tile is a monogram | waiting for the icon pipeline before shipping the list | the icon pipeline does not exist, and a monogram is a tile of text that needs nothing. It is a stand-in and it is legible about being one. |
| The list orders by the last write | an order the user picks: by name, by kind, by creation | one order and no control is the smaller screen. A second order is a control, a stored choice and a second test of every band, for a list a search already narrows. |
| The list groups by the age of the last write | groups by kind | the tile already carries the kind. A group per kind splits the recency order: the last document written sits under every document of another kind. |
| The list groups by the age of the last write | groups by calendar day: today, yesterday | a calendar day needs the reader's time zone at every render, and a band that disagrees with the card's relative time reads as a bug. Elapsed time needs neither. |
| The list groups by the age of the last write | folders or tags | a folder name is content, so it is sealed and stored like a document name: a second encrypted object kept coherent across peers, for a list a query already narrows. |
| The search runs on the client | a search on the relay, over an index | the relay holds no name, by `ZK-001`. A blind index of names leaks the equality and the frequency of words, which is content. The client already holds every decrypted name of the list. |
| The search matches a substring, in any case | a fuzzy or subsequence match | the command palette rejected it first: three typed letters must list the rows that contain them, and one assertion falsifies that. |
| The search reads the name alone | the search also reads the content of each document | it opens and decrypts every document at each listing. The screen today decrypts one short name per row. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The search folds case alone.** An accent stays a different letter, so a query without one does
  not find a name with one. `ambre-spec-find.md` leaves the fold of the suite open, and the list
  takes the palette's.
* **The bands never refresh.** As the relative time, a band is computed when the list or the query
  changes. A list left open for a day keeps a write of the day before in the past day.
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
