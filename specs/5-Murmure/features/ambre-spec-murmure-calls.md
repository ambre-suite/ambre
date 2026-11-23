# AMBRE -- FEATURE: CALLS FROM A CHANNEL
**Type:** Vertical (feature)

> A member starts an audio and video call from a channel in one click. The members of the channel
> join it, and the call is encrypted end to end.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-407` | `INV` | A call started from a channel MUST admit the members of that channel alone. | this spec section 3; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-019`) | `ambre_server::ws::handler::call::tests::a_member_joins_under_pseudonyms_the_relay_draws_per_join` -- database tier: a stranger to the channel takes no seat; `ambre_sync::call::tests::a_joiner_outside_the_channel_gets_no_wrap` -- no key reaches a joiner the log does not hold | none | verified |
| `MURMURE-594` | `CONC` | Two members who start a call in one channel at once MUST sit in one call, named by one call message. | this spec section 3, racing starts; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-160`) | `ambre_server::ws::handler::call::tests::two_starts_at_once_make_one_call_that_holds_both_starters` -- database tier: one room, a roster of both starters, and a new call once the last left; `ambre_sync::call_link::tests::a_start_seated_in_the_live_call_of_its_channel_joins_it` -- the start the relay seats in another call posts no call message | none | verified |
| `MURMURE-592` | `OBS` | Given a call message with no end written, whose call the relay holds no state of, the reader MUST show the call ended. | this spec section 3, the end of a call; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-070`) | `ambre_murmure::logic::call_tests::a_call_message_shows_who_is_in_it_then_ended_once_the_relay_holds_no_state`; `ambre_server::ws::handler::call::tests::a_call_whose_last_seat_stops_its_heartbeat_reads_as_gone` -- database tier; `ambre_belt::murmure::tests::a_call_whose_last_participant_crashed_reads_ended` -- browser tier, under Chrome and under Firefox | none | verified |
| `MURMURE-602` | `OBS` | Given a microphone that the browser refuses or does not have, the header MUST show that the call did not start, and why. | this spec section 3, the call controls | `ambre_murmure::ui::call_controls::tests::a_refusal_says_why_and_leaves_the_call_button`; `ambre_app::shell::murmure_regions::the_call_controls_name_themselves_and_take_the_focus` -- wasm tier: the refusal is an alert beside the call button; `ambre_belt::murmure::tests::a_refused_microphone_starts_no_call_and_says_why` -- browser tier, under Chrome and under Firefox: a refusal and a missing device each read their reason | none | verified |
| `MURMURE-605` | `INV` | Each call control MUST carry an accessible name and take the keyboard focus, and mute MUST carry its pressed state. | this spec section 3, the call controls; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | `ambre_app::shell::murmure_regions::the_call_controls_name_themselves_and_take_the_focus` -- wasm tier: each control is a button with its name that takes the focus, and mute reads its pressed state; `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` -- browser tier, under Chrome and under Firefox: the call button names itself and takes the focus | none | verified |
| `MURMURE-607` | `OBS` | The device panel MUST list each microphone the browser names, after the default one. | this spec section 3, the devices | `ambre_murmure::ui::call_devices::tests::the_panel_lists_each_named_device_after_the_default_one`; `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` -- browser tier, under Chrome and under Firefox: the default microphone comes first | none | verified |
| `MURMURE-608` | `OBS` | Before the browser grants the microphone, the device panel MUST show the default microphone alone, and say that the names show once the microphone is allowed. | this spec section 3, the devices | `ambre_murmure::ui::call_devices::tests::before_the_grant_the_panel_shows_the_default_microphone_alone_and_says_why`; `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` -- browser tier, under Chrome and under Firefox: a list with no name shows the default microphone alone, and the hint | none | verified |
| `MURMURE-609` | `OBS` | The device panel MUST offer a choice of speaker only where the browser routes the audio of a call to a chosen output. | this spec section 3, the devices; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-082`) | `ambre_murmure::ui::call_devices::tests::the_panel_lists_each_named_device_after_the_default_one` -- no element that routes, or no output by its identifier, offers no speaker; `ambre_belt::murmure::tests::a_chosen_speaker_plays_every_slot_and_gives_way_when_it_goes` -- browser tier: Chrome offers the speakers it lists, and Firefox, which lists none, offers no choice | none | verified |
| `MURMURE-610` | `OBS` | While a call uses the default device in place of a chosen one that went away, the header MUST say so. | this spec section 3, the devices; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-081`, `CALL-083`) | `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` -- browser tier, under Chrome and under Firefox: a status reads that the call sends the default microphone; `ambre_belt::murmure::tests::a_chosen_speaker_plays_every_slot_and_gives_way_when_it_goes` -- browser tier, under Chrome: a status reads that the call plays on the default speaker | none | verified |
| `MURMURE-611` | `INV` | Each control of the device panel MUST carry an accessible name and take the keyboard focus. | this spec section 3, the devices; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | `ambre_app::shell::murmure_regions::the_device_controls_name_themselves_and_take_the_focus` -- wasm tier: the devices control is a button with its name and its open state, and the choice of microphone is a select its label names, and each takes the focus | none | verified |
| `MURMURE-614` | `OBS` | During a call, the device panel MUST offer a volume for each other participant of the call, named by its member. | this spec section 3, the volumes; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-086`) | `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: during the call, Ada's panel offers a volume named for Bob and one for Carol; `ambre_murmure::ui::call_volumes::tests::a_read_only_volume_offers_no_volume_for_a_participant` -- a volume read back unchanged offers the volumes | none | verified |
| `MURMURE-615` | `OBS` | Where the browser cannot set the volume of an audio element, the device panel MUST offer no volume for a participant. | this spec section 3, the volumes; `1-Foundations/ambre-spec-call-media.md section 3` | `ambre_murmure::ui::call_volumes::tests::a_read_only_volume_offers_no_volume_for_a_participant`; `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: once an element keeps its own volume, the panel offers the gain and no volume | none | verified |
| `MURMURE-616` | `OBS` | During a call, the device panel MUST offer the gain of the microphone, a return to the automatic gain, and a meter of the level the microphone sends. | this spec section 3, the volumes; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-088`) | `ambre_belt::murmure::tests::a_gain_set_by_hand_scales_the_sound_at_one_rate` -- browser tier, under Chrome and under Firefox: the panel offers the gain and its automatic return, and its meter reads the microphone; `ambre_murmure::ui::call_volumes::tests::the_meter_shows_the_root_mean_square_of_a_window` | none | verified |
| `MURMURE-617` | `INV` | Each volume, the gain, its return to the automatic gain and the meter MUST carry an accessible name, and each slider MUST take the keyboard focus and move by the arrow keys. | this spec section 3, the volumes; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | `ambre_belt::murmure::tests::a_gain_set_by_hand_scales_the_sound_at_one_rate` -- browser tier, under Chrome and under Firefox: the gain is a slider named, which takes the focus and which the left arrow moves, and the meter is a meter named; `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: each volume is a slider named for its member, which takes the focus and which the left arrow moves; `ambre_murmure::ui::call_volumes::tests::a_slider_spells_a_volume_and_a_gain_in_percent` | none | verified |
| `MURMURE-624` | `INV` | The camera button MUST carry an accessible name, take the keyboard focus and carry its pressed state. | this spec section 3, the camera; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | `ambre_app::shell::murmure_regions::the_camera_button_names_itself_and_carries_its_pressed_state` -- wasm tier: the camera control is a button with its name, which takes the focus and reads its pressed state; `ambre_murmure::ui::call_controls::tests::a_press_opens_the_camera_and_a_second_stops_it` -- the name says what a press does | none | verified |
| `MURMURE-659` | `INV` | The fill button MUST carry an accessible name, take the keyboard focus and carry its pressed state. | this spec section 3, the video region; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | `ambre_app::shell::murmure_regions::the_fill_button_names_itself_and_widens_the_call` -- wasm tier: a native button, which the keyboard reaches and presses | none | verified |
| `MURMURE-669` | `INV` | The page buttons and each pin button MUST carry an accessible name and take the keyboard focus, and a pin button MUST carry its pressed state. | this spec section 3, the video region; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | `ambre_app::shell::murmure_regions::the_page_buttons_name_themselves_and_turn_the_page`; `ambre_app::shell::murmure_regions::the_participants_button_names_itself_and_lists_each_participant` | none | verified |
| `MURMURE-633` | `INV` | The share button MUST carry an accessible name, take the keyboard focus and carry its pressed state. | this spec section 3, the screen; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | `ambre_app::shell::murmure_regions::the_share_button_names_itself_and_carries_its_pressed_state` -- wasm tier: the share control is a button with its name, which takes the focus and reads its pressed state; `ambre_murmure::ui::call_controls::tests::a_press_shares_the_screen_and_a_second_stops_it` -- the name says what a press does | none | verified |
| `MURMURE-693` | `INV` | Each control of the call bubble and of the call pill MUST carry an accessible name and take the keyboard focus, and mute MUST carry its pressed state. | this spec section 3, away from the channel of the call; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-689` | `OBS` | A sidebar entry whose channel holds a live call the page knows MUST show a call mark and the count of its participants, and no name. | this spec section 3, who is in the call; `ZK-024` | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-690` | `OBS` | The sidebar entry of the channel the view shows MUST show the initials of each participant of its live call. | this spec section 3, who is in the call; `ZK-024` | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-691` | `OBS` | A sidebar entry whose channel the page never opened MUST show the live call of that channel. | this spec section 3, who is in the call; `ZK-024` | not built | unbuilt -- lands with the `Live calls of unopened channels in the sidebar` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-696` | `OBS` | While the page is in a call and shows no channel region of it, over the phone width it MUST show the call bubble: the channel, the time since the start, the faces, mute, the camera, the share, the way back and hang up. | this spec section 3, away from the channel of the call | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-697` | `OBS` | While the page is in a call and shows no channel region of it, under the phone width it MUST show the call pill under the header: the channel, the time since the start, the faces, mute and hang up. | this spec section 3, away from the channel of the call | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-698` | `INV` | The call bubble MUST publish the part of the canvas it covers. | this spec section 3, away from the channel of the call; `1-Foundations/ambre-spec-shell.md section 2` (`CHROME-171`) | not built | unbuilt -- lands with the `The floating cover` Story (`ambre-roadmap.md` MVP) | unimplemented |
| `MURMURE-699` | `OBS` | A press on the faces of the call bubble MUST show the video of the member who speaks, or the received screen, drawn by the render runtime of the page. | this spec section 3, away from the channel of the call; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-015`) | not built | unbuilt -- lands with the `Video in the call bubble` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-629` | `OBS` | While the page receives a screen, the video region MUST draw it in a tile taller than each tile of a camera. | this spec section 3, the video region; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-015`) | `ambre_murmure::ui::call_videos::tests::a_screen_takes_a_tile_taller_than_each_camera_above_their_row`; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: the moved pixels of Bob's canvas span more than the tallest tile of a camera | none | verified |
| `MURMURE-630` | `OBS` | Given a share that the screen of another member took over, the header MUST say that another member shares. | this spec section 3, the screen; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-107`) | `ambre_murmure::ui::call_controls::tests::another_screen_takes_over_a_share_and_the_header_says_so`; `ambre_app::shell::murmure_regions::the_share_button_names_itself_and_carries_its_pressed_state` -- wasm tier: the take-over is a status; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Ada's header reads that another member shares | none | verified |
| `MURMURE-645` | `OBS` | Given a ring of the caller that ended with no answer, the header MUST say whether the member declined or did not answer, until the next call gesture. | this spec section 3, the ring | `ambre_murmure::ui::call_ring::tests::the_header_says_why_a_ring_ended_with_no_answer`; `ambre_app::shell::murmure_regions::the_header_says_why_a_call_rang_with_no_answer` -- wasm tier: the notice is a status beside the call button | none | verified |
| `MURMURE-646` | `INV` | The title of the ring alert MUST read as an alert, and accept and decline MUST each carry an accessible name and take the keyboard focus. | this spec section 3, the ring; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | `ambre_app::shell::murmure_regions::the_ring_alert_names_the_caller_and_offers_accept_and_decline` -- wasm tier: the title is an alert, and each control is a button with its name that takes the focus | none | verified |

### 1.2 Start a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-408` | `POST` | Starting a call MUST post one call message in the channel once the relay seats its starter. | this spec section 3 | `ambre_murmure::logic::call_tests::a_start_posts_one_call_message_and_leaves_the_members`; `ambre_sync::call_link::tests::a_start_is_heard_once_seated_and_the_last_leave_asks_for_the_end` -- no call message before the seat; `ambre_logic::channels::tests::a_start_and_a_last_leave_heard_write_the_call_message_and_its_end`; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox | none | verified |
| `MURMURE-409` | `POST` | The call message MUST show the members the relay seats in its call, read from the relay and never written into the message. | this spec section 3, who is in the call; `CALL-070` | `ambre_server::ws::handler::call::tests::a_member_out_of_the_call_reads_the_signed_join_of_each_seat` -- database tier; `ambre_murmure::logic::call_tests::a_call_message_shows_who_is_in_it_then_ended_once_the_relay_holds_no_state`; `ambre_logic::channels::tests::who_is_in_a_call_reaches_the_view_and_writes_nothing`; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox: Carol, out of the call, reads both participants | none | verified |
| `MURMURE-410` | `FRAME` | Starting a call MUST NOT change the membership of the channel. | this spec section 3 | `ambre_murmure::logic::call_tests::a_start_posts_one_call_message_and_leaves_the_members` -- the `meta` document reads the same after the post | none | verified |
| `MURMURE-599` | `POST` | The call button in the header of a channel or a direct message MUST start a call in that channel. | this spec section 3, the call controls; `MURMURE-408` | `ambre_murmure::ui::call_controls::tests::a_channel_offers_the_call_button_and_a_call_offers_mute_and_hang_up_in_its_place`; `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` -- browser tier, under Chrome and under Firefox: Ada calls Bob by the button in the header of their direct message, and each decodes the other | none | verified |

### 1.3 Join a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-411` | `POST` | Joining from a call message MUST bring the member into that call. | this spec section 3; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-016`) | `ambre_murmure::ui::commands::tests::a_live_call_message_offers_a_join_of_its_call`; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox: Bob joins from the menu of the message, and the two decode each other | none | verified |
| `MURMURE-412` | `FAIL` | Given a call that has ended, the call message MUST offer no join. | this spec section 3 | `ambre_murmure::ui::commands::tests::a_call_that_ended_offers_no_join`; `ambre_server::ws::handler::call::tests::a_join_of_a_call_the_relay_holds_no_state_of_is_refused_as_ended` -- database tier; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox: the menu of an ended call holds no join | none | verified |
| `MURMURE-413` | `FRAME` | Joining a call MUST NOT move a read cursor. | this spec section 3 | structural -- `MessageAction::JoinCall` carries no command of the Logic thread, and the shell hands it to the call client of the page; `ambre_murmure::ui::commands::tests::a_live_call_message_offers_a_join_of_its_call` | none | verified |

### 1.4 Show who is in the call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-595` | `POST` | A reader MUST ask the relay who is in a live call only while its view shows the channel, at most once per interval of `ambre-spec-budgets.md section 1`. | this spec section 3, who is in the call; `ZK-024` | `ambre_logic::channels::tests::a_tick_asks_who_is_in_each_live_call_once_per_interval` | none | verified |
| `MURMURE-596` | `FAIL` | Given a signed join whose member the membership log of the channel does not hold under its key, the reader MUST leave it out of who is in the call. | this spec section 3, who is in the call; `CALL-019` | `ambre_sync::call_roster::tests::a_join_the_log_does_not_hold_names_no_one_in_the_call`; `ambre_sync::call_link::tests::who_is_in_a_call_reaches_the_logic_thread_once_the_log_checks_it` | none | verified |
| `MURMURE-597` | `FRAME` | Asking who is in a call MUST NOT write into the channel. | this spec section 3, who is in the call | `ambre_logic::channels::tests::who_is_in_a_call_reaches_the_view_and_writes_nothing` -- no document moves, and nothing leaves for the relay | none | verified |

### 1.5 End a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-591` | `POST` | The leave the relay counts as the last of a call MUST write the end into its call message. | this spec section 3, the end of a call; `CALL-069` | `ambre_murmure::logic::call_tests::the_last_leave_writes_an_end_every_member_reads`; `ambre_sync::call_link::tests::a_start_is_heard_once_seated_and_the_last_leave_asks_for_the_end`; `ambre_sync::call_link::tests::a_leave_with_no_answer_ends_the_call_and_writes_no_end`; `ambre_logic::channels::tests::a_start_and_a_last_leave_heard_write_the_call_message_and_its_end`; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox | none | verified |
| `MURMURE-593` | `FAIL` | Given a delta that writes an end under another member's key, every client MUST refuse the delta. | this spec section 3, the end of a call; `MURMURE-094` | `ambre_murmure::logic::call_tests::an_end_under_another_members_key_is_refused` | none | verified |
| `MURMURE-598` | `FRAME` | Writing the end MUST NOT change the call message beyond the entry of its writer. | this spec section 3, the end of a call; `MURMURE-095` | `ambre_murmure::logic::call_tests::the_last_leave_writes_an_end_every_member_reads` -- the message reads the same but its call | none | verified |

### 1.6 The call controls

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-601` | `FAIL` | Given a microphone that the browser refuses or does not have, a call gesture MUST post no call message and take no seat. | this spec section 3, the call controls; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-076`) | `ambre_belt::murmure::tests::a_refused_microphone_starts_no_call_and_says_why` -- browser tier, under Chrome and under Firefox: the page spawns no call worker, makes no peer connection and opens no call socket | none | verified |
| `MURMURE-603` | `POST` | While the page is in a call, the header MUST offer mute and hang up in place of the call button. | this spec section 3, the call controls | `ambre_murmure::ui::call_controls::tests::a_channel_offers_the_call_button_and_a_call_offers_mute_and_hang_up_in_its_place`; `ambre_app::shell::murmure_regions::the_call_controls_name_themselves_and_take_the_focus` -- wasm tier; `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` -- browser tier, under Chrome and under Firefox: a page in a call shows no call button | none | verified |
| `MURMURE-604` | `POST` | Hang up MUST leave the call, so that each other member reads the member who hung up out of it. | this spec section 3, the call controls; `MURMURE-409`; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-069`) | `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` -- browser tier, under Chrome and under Firefox: Ada hangs up, her call worker erases its keys, and Bob reads himself alone in the call | none | verified |
| `MURMURE-606` | `FRAME` | A mute MUST NOT write into the channel. | this spec section 3, the call controls; `ZK-025` | structural -- the mute control runs `toggle_mute` of the page, which disables the track and posts nothing to a worker; `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` -- browser tier, under Chrome and under Firefox: the channel reads the same call message through the mute | none | verified |


### 1.7 Choose a device

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-612` | `POST` | A device chosen in the panel MUST be the one each later call of this browser opens. | this spec section 3, the devices | `ambre_murmure::ui::call_devices::tests::a_choice_reads_back_from_the_local_storage`; `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` -- browser tier, under Chrome and under Firefox: the local storage holds the identifier, and the next call of Ada opens that microphone | none | verified |
| `MURMURE-613` | `FRAME` | A choice of device MUST NOT write into a channel or into the settings of the user. | this spec section 3, the devices; `ZK-026` | structural -- a choice writes the local storage of the browser alone, and no command of the bus can carry an `Opaque<DeviceIds>`, which has no serde form (`ZK-026`) | none | verified |

### 1.8 Set a volume

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-618` | `POST` | A volume set for a member, and a gain set by hand, MUST be the ones each later call of this browser applies. | this spec section 3, the volumes | `ambre_murmure::ui::call_volumes::tests::the_volumes_and_the_gain_read_back_from_the_local_storage`; `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: the volume of Bob stays in the local storage of Ada; `ambre_belt::murmure::tests::a_gain_set_by_hand_scales_the_sound_at_one_rate` -- browser tier, under Chrome and under Firefox: the return to the automatic gain removes its entry | none | verified |
| `MURMURE-619` | `FRAME` | A volume or a gain MUST NOT write into a channel or into the settings of the user. | this spec section 3, the volumes; `ZK-027` | structural -- the volumes and the gain write the local storage of the browser alone, and no command of the bus carries a volume or a gain; `ambre_belt::murmure::tests::a_participant_turned_down_stays_silent_from_slot_to_slot` -- browser tier, under Chrome and under Firefox: setting a volume posts no message to a worker or a port | none | verified |

### 1.9 Show the videos of a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-620` | `OBS` | While the page receives a video of its call, the channel region MUST draw each such video in its video region, above the composer. | this spec section 3, the video region; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-015`) | `ambre_murmure::ui::view::tests::a_received_video_draws_in_a_band_above_the_composer`; `ambre_murmure::ui::call_videos::tests::the_band_places_each_slot_in_its_tile`; `ambre_belt::murmure::tests::a_member_draws_the_video_of_another_on_its_canvas` -- browser tier, under Chrome and under Firefox: the moved pixels of Bob's canvas lie in one band | none | verified |
| `MURMURE-621` | `OBS` | While the page is in no call, the channel region MUST show no video region. | this spec section 3, the video region | `ambre_murmure::ui::view::tests::no_video_shows_no_video_region` -- the region comes with the call and goes with its end; `ambre_murmure::ui::call_videos::tests::no_call_takes_no_band_and_a_short_region_keeps_its_list` | none | verified |
| `MURMURE-651` | `OBS` | During a call, the video region MUST show a tile for each participant of the page it shows: its picture while a video of it arrives, else its initials, and its name below. | this spec section 3, the video region; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-084`) | `ambre_murmure::ui::call_videos::tests::each_participant_takes_a_tile_of_a_grid_of_three_by_three_at_most`; `ambre_murmure::ui::call_videos::tests::a_tile_without_video_shows_the_initials_and_each_tile_its_name`; `ambre_murmure::ui::call_volumes::tests::a_video_slot_follows_the_member_of_its_sender`; `ambre_call::tests::a_video_slot_tells_the_page_of_each_new_sender_once`; `ambre_murmure::ui::view::tests::no_video_shows_no_video_region` -- a call with every camera off shows its region | none | verified |
| `MURMURE-652` | `OBS` | In the fill grid and in the strip of the scene, the member's own tile MUST take the last place, at the size of each other tile. | this spec section 3, the video region | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-684` | `OBS` | In the band, the member's own tile MUST sit apart from the other tiles, at the end of the band, smaller than each of them. | this spec section 3, the video region | `ambre_murmure::ui::call_videos::tests::the_own_tile_is_small_and_marks_a_mute` | none | verified |
| `MURMURE-685` | `OBS` | The member's own tile MUST mark a mute of the member. | this spec section 3, the video region; `ZK-025` | `ambre_murmure::ui::call_videos::tests::the_own_tile_is_small_and_marks_a_mute` | none | verified |
| `MURMURE-686` | `OBS` | In the fill grid, the video region MUST lay out the tiles of the page and the member's own tile in the count of columns that gives the largest tiles. | this spec section 3, the video region | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-687` | `OBS` | While the call is widened and receives a screen or holds a pin, the video region MUST lay out the scene: the screen, else the pinned tile, across the region, with the other tiles in a strip beside it. | this spec section 3, the video region; `MURMURE-629` | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-688` | `OBS` | While a participant on another page speaks, the arrow toward that page MUST be marked. | this spec section 3, the video region; `MURMURE-653` | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-656` | `OBS` | While the camera of the member goes into the call, its own tile MUST draw its picture, mirrored. | this spec section 3, the video region; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-015`, `CALL-100`) | `ambre_murmure::ui::call_videos::tests::the_own_tile_draws_the_camera_mirrored_while_it_is_on`; `ambre_canvas::video::tests::a_mirrored_picture_swaps_its_left_and_right`; `ambre_app::call_stage::tests::the_own_camera_reaches_the_render_runtime_while_it_is_on` -- wasm tier: the track of the camera reaches the render runtime and leaves it once the camera stops | none | verified |
| `MURMURE-653` | `OBS` | The tile of a participant whose received audio carries speech MUST be outlined, until the hold of `ambre-spec-budgets.md section 1` after its last speech. | this spec section 3, the video region; `ZK-025` | `ambre_murmure::ui::call_videos::tests::a_member_who_speaks_is_outlined_for_the_hold` -- the level of speech, the hold and the outline of that tile alone; `ambre_app::call_stage::tests::the_level_of_a_slot_is_that_of_its_latest_source` -- wasm tier: the page reads the level of a receiver from the source it heard last | none | verified |

### 1.10 Turn the camera on or off

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-622` | `POST` | During a call, the camera button MUST send the camera into the call, and a second press MUST stop it. | this spec section 3, the camera; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-100`) | `ambre_murmure::ui::call_controls::tests::a_press_opens_the_camera_and_a_second_stops_it`; `ambre_belt::murmure::tests::a_member_draws_the_video_of_another_on_its_canvas` -- browser tier, under Chrome and under Firefox: Ada's press sends her camera, which Bob's canvas draws, and a second press leaves her sender with no track | none | verified |
| `MURMURE-623` | `FAIL` | Given a camera that the browser refuses or does not have, the call MUST go on with its audio, and the header MUST show that the camera did not open, and why. | this spec section 3, the camera | `ambre_murmure::ui::call_controls::tests::a_refused_camera_says_why_and_keeps_the_call`; `ambre_app::shell::murmure_regions::the_camera_button_names_itself_and_carries_its_pressed_state` -- wasm tier: the refusal is an alert, and mute and hang up stay; `ambre_belt::murmure::tests::a_member_draws_the_video_of_another_on_its_canvas` -- browser tier, under Chrome and under Firefox: Bob's refused camera reads its reason, and Bob still decodes Ada | none | verified |
| `MURMURE-625` | `FRAME` | Turning the camera on or off MUST NOT write into the channel. | this spec section 3, the camera; `ZK-001` | structural -- the camera button runs `toggle_camera` of the page, which posts no command to a worker; `ambre_belt::murmure::tests::a_member_draws_the_video_of_another_on_its_canvas` -- browser tier, under Chrome and under Firefox: Ada's press posts nothing to a worker or a port but the roster ask | none | verified |

### 1.11 Share a screen

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-632` | `POST` | During a call, the share button MUST send a screen or a window the member picks into the call, and a second press MUST stop it. | this spec section 3, the screen; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-103`) | `ambre_murmure::ui::call_controls::tests::a_press_shares_the_screen_and_a_second_stops_it`; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Ada's press sends her screen, which Bob decodes | none | verified |
| `MURMURE-634` | `FAIL` | Given a capture of the screen that the browser refuses or the member cancels, the header MUST show that no screen is shared, and why. | this spec section 3, the screen; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-109`) | `ambre_murmure::ui::call_controls::tests::a_refused_screen_says_why_and_a_press_tries_again`; `ambre_app::shell::murmure_regions::the_share_button_names_itself_and_carries_its_pressed_state` -- wasm tier: the refusal is an alert, and mute and hang up stay; `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Bob's refused capture reads its reason | none | verified |
| `MURMURE-631` | `FRAME` | A share of the screen MUST NOT write into the channel. | this spec section 3, the screen; `ZK-032` | structural -- the share button runs `toggle_share` of the page, whose one command marks the start for the socket of the SFU (`ZK-032`); `ambre_belt::murmure::tests::a_member_shares_its_screen_and_a_later_share_takes_it_over` -- browser tier, under Chrome and under Firefox: Ada's share posts that start and nothing else but the roster ask | none | verified |

### 1.12 Ring a direct message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-641` | `POST` | Given a call message that another member posts in a direct message, each open page of Murmure of the member MUST ring, with an accept and a decline, whatever channel it shows. | this spec section 3, the ring; `1-Foundations/ambre-spec-residency.md section 3` | `ambre_sync::light::tests::a_call_cue_of_a_light_document_reaches_logic_with_no_document_loaded` -- a channel the page does not hold cues its call with no document loaded; `ambre_logic::channels::tests::a_cue_of_a_closed_channel_names_its_channel_to_the_view`; `ambre_logic::channels::tests::a_call_message_of_another_member_cues_a_ring_and_the_last_leave_cues_its_end` -- an open channel cues the call message of another member; `ambre_murmure::ui::call_ring::tests::a_ring_cue_of_a_direct_message_rings_a_free_page_until_its_timeout`; `ambre_app::shell::murmure_regions::the_ring_alert_names_the_caller_and_offers_accept_and_decline` -- wasm tier: the cue of the direct message shows the alert, with its accept and its decline | none | verified |
| `MURMURE-642` | `POST` | Accept MUST join the call as Join call does, the microphone first, and decline MUST write the end of the member into the call message. | this spec section 3, the ring; `MURMURE-411`; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-076`) | `ambre_murmure::ui::call_ring::tests::accept_joins_the_call_and_decline_writes_the_decline`; structural -- the join of a ring runs `crate::call::join`, the path of Join call; `ambre_logic::channels::tests::a_decline_opens_the_channel_and_writes_its_end_once_open` -- a channel the page does not hold opens, and the end leaves once its segment may be written; `ambre_app::shell::murmure_regions::the_ring_alert_names_the_caller_and_offers_accept_and_decline` -- wasm tier: a press of decline sends the decline of the call to the Logic thread | none | verified |
| `MURMURE-643` | `POST` | A ring with no answer MUST stop on the page of the member once the ring timeout of `ambre-spec-budgets.md section 1` has passed since the call message, and MUST write nothing. | this spec section 3, the ring | `ambre_murmure::ui::call_ring::tests::a_ring_cue_of_a_direct_message_rings_a_free_page_until_its_timeout` -- the tick at the timeout stops the ring, and asks the page to do nothing | none | verified |
| `MURMURE-644` | `POST` | On the page of the caller, a decline or the ring timeout with no answer MUST end the ring and hang up the call, and another member admitted into the call MUST end the ring with no hang-up. | this spec section 3, the ring; `MURMURE-604` | `ambre_murmure::ui::call_ring::tests::a_decline_or_the_timeout_ends_the_ring_of_the_caller`; `ambre_logic::channels::tests::a_decline_opens_the_channel_and_writes_its_end_once_open` -- the end of a decline leaves under a decline cue | none | verified |
| `MURMURE-647` | `FRAME` | A ring MUST NOT show the relay anything beyond the call message, and, on a decline, the open of its channel and the end of the member who declines. | this spec section 3, the ring; `1-Foundations/ambre-spec-zero-knowledge.md section 2` (`ZK-001`); `1-Foundations/ambre-spec-residency.md section 3` | type-level -- the cue rides the activity summary, which the Sync worker seals inside its update, and no command of the Sync worker names a ring; `ambre_logic::channels::tests::a_decline_opens_the_channel_and_writes_its_end_once_open` -- the decline asks for the channel, then leaves as one update of its segment | none | verified |
| `MURMURE-648` | `FAIL` | Given a ring cue older than the ring timeout, or a page in a call, the page MUST NOT ring. | this spec section 3, the ring | `ambre_murmure::ui::call_ring::tests::a_ring_cue_of_a_direct_message_rings_a_free_page_until_its_timeout` -- a cue replayed past the timeout and a page on the line ring nothing | none | verified |
| `MURMURE-649` | `FRAME` | A call of a channel that is no direct message MUST ring no page. | this spec section 3, the ring | `ambre_murmure::ui::call_ring::tests::a_ring_cue_of_a_direct_message_rings_a_free_page_until_its_timeout` -- the cue of a channel rings nothing; `ambre_app::shell::murmure_regions::the_ring_alert_names_the_caller_and_offers_accept_and_decline` -- wasm tier: the cue of a public channel shows no alert | none | verified |
| `MURMURE-650` | `POST` | A decline or an end written into the call message MUST stop its ring on each page that hears it. | this spec section 3, the ring; `MURMURE-591` | `ambre_murmure::ui::call_ring::tests::a_decline_or_an_end_of_the_call_stops_its_ring`; `ambre_logic::channels::tests::a_call_message_of_another_member_cues_a_ring_and_the_last_leave_cues_its_end` -- the last leave writes its end under an end cue | none | verified |

### 1.13 Page through the participants and pin one

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-654` | `POST` | Past the page size that section 3 gives the layout and the width, the video region MUST show the other participants in pages of that size, in the order the call admitted them, and a press on an arrow MUST show the next or the previous page. | this spec section 3, the video region; `CHROME-029` | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-667` | `POST` | During a call, the page MUST ask the SFU for the video of exactly the participants whose tile the grid shows, the pinned one included, at each change of the page, the pin or the participants. | this spec section 3, the video region; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-126`) | `ambre_murmure::ui::call_videos::tests::the_choice_names_exactly_the_tiles_the_grid_shows`; `ambre_murmure::ui::call_volumes::tests::a_member_is_chosen_by_the_sender_it_was_admitted_with_last`; `ambre_belt::murmure::tests::a_page_switch_and_a_pin_choose_the_videos_the_grid_shows` -- browser tier: a turn of the page posts the senders of the next page, and each post names the senders of the tiles shown | none | verified |
| `MURMURE-668` | `POST` | A press on the tile of another participant, or on its pin button in the list of the participants, MUST pin it, and a second press MUST unpin it. The pinned tile MUST stay in view on every page, larger than each tile of the grid, and the page MUST ask its video large. | this spec section 3, the video region; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-128`) | `ambre_murmure::ui::call_videos::tests::a_pinned_participant_stays_large_on_every_page`; `ambre_murmure::ui::view::tests::a_press_on_an_arrow_turns_the_page_and_a_press_on_a_tile_pins_it`; `ambre_app::shell::murmure_regions::the_participants_button_names_itself_and_lists_each_participant` -- wasm tier: the pin button of the list pins and unpins; `ambre_belt::murmure::tests::a_page_switch_and_a_pin_choose_the_videos_the_grid_shows` -- browser tier: Ada pins Bob, and decodes his video at the top size while Carol alone speaks | none | verified |
| `MURMURE-704` | `POST` | Under the phone width, the choice of videos MUST name the phone count of `ambre-spec-budgets.md section 1`, and over it the desktop count. | this spec section 3, the video region; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-041`) | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-655` | `FRAME` | Opening the list of the participants MUST NOT write into the channel. | this spec section 3, the video region; `ZK-025` | structural -- the list reads the participants the page holds, and the participants button posts nothing to a worker | none | verified |
| `MURMURE-670` | `FRAME` | A turn of the page or a pin MUST NOT write into the channel, and the choice of videos it changes MUST reach the socket of the SFU alone. | this spec section 3, the video region; `1-Foundations/ambre-spec-zero-knowledge.md section 2` (`ZK-044`) | `ambre_belt::murmure::tests::a_page_switch_and_a_pin_choose_the_videos_the_grid_shows` -- browser tier: Ada's turns and pins post a choice of videos and nothing else but the roster ask; `ambre_sync::call_link::tests::a_choice_of_videos_reaches_the_sfu_alone_and_never_the_relay` | none | verified |

### 1.14 Fill the channel region with a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-657` | `POST` | During a call, the fill button MUST widen the video region to the whole height above the composer, and a second press MUST bring back the band. | this spec section 3, the video region | `ambre_murmure::ui::call_videos::tests::a_filled_call_takes_the_whole_height_above_the_composer`; `ambre_murmure::ui::view::tests::a_filled_call_leaves_the_list_no_height_and_keeps_the_composer`; `ambre_app::shell::murmure_regions::the_fill_button_names_itself_and_widens_the_call` -- wasm tier: a press widens the call and a second shrinks it | none | verified |
| `MURMURE-658` | `FRAME` | Widening the call or bringing back the band MUST NOT write into the channel, and MUST leave the composer in place. | this spec section 3, the video region; `ZK-025` | `ambre_murmure::ui::view::tests::a_filled_call_leaves_the_list_no_height_and_keeps_the_composer` -- the composer still takes a press; structural -- the fill button runs `toggle_fill` of the page, which sets a signal of the page and posts nothing to a worker | none | verified |

### 1.15 Widen the call on a screen

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-692` | `POST` | The first picture of a received screen MUST widen the call into the scene. | this spec section 3, the video region; `MURMURE-687` | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-694` | `POST` | Given a call the member brought back to the band while it received a screen, the call MUST stay a band until another share starts. | this spec section 3, the video region | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-695` | `FRAME` | Widening the call on a screen MUST NOT write into the channel, nor move the keyboard focus. | this spec section 3, the video region; `ZK-025` | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |

### 1.16 Leave the channel of a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-700` | `POST` | The way back of the bubble, or a press on the pill, MUST show the channel of the call in Murmure. | this spec section 3, away from the channel of the call | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-701` | `POST` | Showing another channel or another application MUST keep the page in its call. | this spec section 3, away from the channel of the call | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-703` | `POST` | A drag of the call bubble MUST move it, and its release MUST rest it in the nearest corner of the content. | this spec section 3, away from the channel of the call | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-702` | `FRAME` | Leaving the channel of a call, or coming back to it, MUST NOT write into a channel, nor move a read cursor, nor change the choice of videos. | this spec section 3, away from the channel of the call; `ZK-025` | not built | unbuilt -- lands with the `The Murmure call and sidebar rework` Story (`ambre-roadmap.md` V1) | unimplemented |

### 1.17 Present a document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-705` | `POST` | Present MUST open the document of the presenter for each participant who is a member of it, following the view of the presenter. | this spec section 3, present a document | not built | unbuilt -- lands with the `Present a document in a call` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-706` | `FAIL` | Given a participant who is no member of the document, Present MUST show that participant nothing of it, and MUST offer the presenter to invite them as a viewer or to share the screen. | this spec section 3, present a document; `1-Foundations/ambre-spec-key-rotation.md section 3` (`SYNC-116`, `SYNC-119`) | not built | unbuilt -- lands with the `Present a document in a call` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-707` | `POST` | An edit or a scroll of a follower MUST stop it following the presenter. | this spec section 3, present a document | not built | unbuilt -- lands with the `Present a document in a call` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-708` | `POST` | A press on the follow bar MUST bring the follower back to the view of the presenter. | this spec section 3, present a document | not built | unbuilt -- lands with the `Present a document in a call` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-709` | `FRAME` | Presenting MUST NOT derive a key of the call from a key of the document, nor carry any part of the document over the media of the call. | this spec section 3, present a document; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-002`, `CALL-010`) | not built | unbuilt -- lands with the `Present a document in a call` Story (`ambre-roadmap.md` V1) | unimplemented |
| `MURMURE-710` | `FRAME` | Presenting MUST NOT change the membership of the document. | this spec section 3, present a document; `ZK-001` | not built | unbuilt -- lands with the `Present a document in a call` Story (`ambre-roadmap.md` V1) | unimplemented |

* **Budget.** the interval between two asks of who is in a call is the call roster row of
  `ambre-spec-budgets.md section 1`. The time a call rings is the call ring row. The call rows of
  that section hold the participant cap, the videos per receiver and the time to live. The paint of
  the videos is the call video paint row.
* **Boundary.** the call message and its end are sealed like any message (`ZK-001`). A ring
  rides their sealed summaries, so the relay learns no ring and no decline (`MURMURE-647`). What the relay
  learns from an ask of who is in a call is `ZK-024`. `ZK-021` and `ZK-022` bound the rest.
* **Verification.** 70 `verified`, 27 `unimplemented`. The relay half holds in the database tier.
  The engine and the two workers hold on the host, and the whole path in the browser tier, under
  Chrome and Firefox.

---

## 2. WHAT EACH USER SEES

| Ada does | Bob does | The channel holds | Each member sees | Test |
|---|---|---|---|---|
| calls Bob from the header of their direct message | joins from the message | a call message | each hears the other (`MURMURE-599`) | `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` |
| mutes | listens | the same message | Bob hears silence from Ada (`CALL-074`) | `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` |
| hangs up | stays | the same message | Bob alone in the call (`MURMURE-604`) | `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` |
| mutes, then picks another microphone in the device panel | listens | the same message | Bob hears silence from Ada until her unmute, then the new microphone (`CALL-078`, `CALL-080`) | `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` |
| unplugs the microphone she chose | listens | the same message | Bob hears the default microphone of Ada, and her header says so (`CALL-081`, `MURMURE-610`) | `ambre_belt::murmure::tests::a_member_changes_its_microphone_during_a_call_with_no_gap` |
| turns her camera on during the call | is in the call | the same message | Bob's channel draws Ada's video above his composer (`MURMURE-620`, `MURMURE-622`) | `ambre_belt::murmure::tests::a_member_draws_the_video_of_another_on_its_canvas` |
| turns her camera on, and her browser refuses it | is in the call | the same message | Ada reads that the camera is blocked, and each still hears the other (`MURMURE-623`) | `ambre_belt::murmure::tests::a_member_draws_the_video_of_another_on_its_canvas` |
| calls, and her browser refuses the microphone | nothing | nothing new | Ada reads that the microphone is blocked (`MURMURE-601`, `MURMURE-602`) | `ambre_belt::murmure::tests::a_refused_microphone_starts_no_call_and_says_why` |
| starts a call | nothing | a call message | Ada in the call, and a join (`MURMURE-408`, `MURMURE-409`) | `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` |
| is in the call | joins from the message | the same message | Ada and Bob in the call (`MURMURE-411`) | `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` |
| leaves | leaves last | the message and its end | the call ended, and no join (`MURMURE-591`, `MURMURE-412`) | `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` |
| is alone in the call, and her browser dies | nothing | the message, no end | the call ended, one time to live later at most (`MURMURE-592`) | `ambre_belt::murmure::tests::a_call_whose_last_participant_crashed_reads_ended` |
| calls Bob from the header of their direct message | reads another channel | a call message | Bob's page rings, with an accept and a decline (`MURMURE-641`) | `ambre_app::shell::murmure_regions::the_ring_alert_names_the_caller_and_offers_accept_and_decline` |
| waits | declines | the message and Bob's end | Ada's call ends, and her header says the call was declined (`MURMURE-644`, `MURMURE-645`) | `ambre_murmure::ui::call_ring::tests::a_decline_or_the_timeout_ends_the_ring_of_the_caller` |
| waits | does not answer | the message and Ada's end | the ring stops on both pages at the timeout, and Ada's header says no answer (`MURMURE-643`, `MURMURE-644`) | `ambre_murmure::ui::call_ring::tests::a_decline_or_the_timeout_ends_the_ring_of_the_caller` |
| starts a call | starts a call at the same instant | one call message | one call that holds both (`MURMURE-594`) | `ambre_server::ws::handler::call::tests::two_starts_at_once_make_one_call_that_holds_both_starters` |

---

## 3. MODEL

**CRDT containers.** the `call` and the `call_end` of a message, in
`ambre-spec-murmure-schema.md section 3`. `call` names the call. `call_end` holds one entry per
member who wrote the end, keyed by that member, as `reactions` is.

**A start, in steps.**

1. The client mints the call identifier, its random bits from the generator of the OS.
2. It joins the call as a start. The relay creates the state of the call and seats the starter.
3. Once seated, the client posts the call message in the channel (`MURMURE-408`).

So a call message never names a call the relay did not hold. A join from the message is no start:
the relay refuses it once the state of the call is gone (`CALL-068`). The membership of the channel
admits the call, and the start writes nothing to it (`MURMURE-407`, `MURMURE-410`).

**Racing starts.** The relay holds one key per channel that names its live call. A start takes
that key in one script, and creates its call. A start that finds the key on another call joins
that call in its place (`CALL-160`). The admission names that call, and the start posts no call
message (`MURMURE-594`).

A join of that call refused as ended frees the key, so a call lost with no last leave never
holds the channel (`CALL-161`). The heartbeat of each seat refreshes the key, and the last leave
deletes it. The relay learns nothing new: the state of each call already names its channel.

**Who is in the call.** The relay holds the signed join of each seat (`ZK-022`). A reader asks the
relay for them, and verifies each against the membership log of the channel (`MURMURE-596`). The
message shows the members that pass. Nothing of the list is written into the message
(`MURMURE-409`).

The view ticks while it shows the channel. Each tick asks for each live call of the
held segments, once per interval at most (`MURMURE-595`).

**A live call in the sidebar.** The sidebar marks each channel whose held segments name a live
call, with the count of its participants and no name (`MURMURE-689`). The entry of the channel the
view shows adds their initials (`MURMURE-690`). The count comes from the same ask, so the sidebar
asks for each live call of each held channel at the same interval. A channel never opened holds
no call message on the page, and shows no mark yet (`MURMURE-691`).

`ZK-024` holds for each such ask: the relay learns the session, the call, the channel and the
instant, and stores none of it. An ask for a channel the view does not show tells it no more.
The relay already holds the subscription of that session to the channel, and the seats of the
call. The instant of the ask says that the page is open, which its socket already says. The count
and the initials read the same answer, so a name in the sidebar would cost the relay nothing
either. The sidebar keeps names out for its own width, never for the relay.

**What the relay learns by the ask** is `ZK-024`: the session, the call and the instant. The
watching sample of the same tick already tells the relay that this session shows the channel
(`MURMURE-121`). The relay knows the calls of the channel from their
state. The ask reads that state and writes nothing.

**The end of a call.** Two sources, in order:

1. **The end written.** The relay answers a leave with whether its seat was the last (`CALL-069`).
   That client writes its end into the call message, sealed (`MURMURE-591`). Each member writes
   under its own key alone (`MURMURE-593`). A message of a final segment takes no end.
2. **The state gone.** A last participant who crashes, or loses its network, writes no end. The
   relay closes a socket that answered no ping for one time to live. It drops the state of
   the call with its last seat (`CALL-073`, `SRV-251`). The next ask answers that the relay
   holds none, and the reader shows the call ended (`MURMURE-592`).

The relay keeps no record that a call ended: the absence of its state is all it holds. So the
relay never learns which calls of a channel ended, and holds no history and no duration
(`ZK-022`). An end written by a member stays when the relay forgets the call.

**The call controls.** The header of a shown channel holds them, a direct message included. Each
control is a button with an icon of its own and an accessible name (`MURMURE-605`).

| The page | The header offers |
|---|---|
| in no call, a channel shown | the call button (`MURMURE-599`) |
| asking for the microphone | no control |
| in a call | mute and hang up, whatever channel it shows (`MURMURE-603`) |

A start in a channel whose call runs sits in that call: the relay holds one call per channel
(`CALL-160`). So the call button joins a call that runs, and the page reads no list to decide it.

A call gesture is the call button, or Join call in the menu of a call message. The page asks the
browser for the microphone first. Only once it holds the microphone does the page start or join
the call (`CALL-076`). So a refusal posts no call message and takes no seat (`MURMURE-601`). The
header then shows why the call did not start, until the next call gesture (`MURMURE-602`):

| The browser answers | The header shows |
|---|---|
| a refusal of the user or of a policy | the microphone is blocked |
| no capture device | no microphone is found |
| any other failure | the microphone did not open |

Mute stops the sound of the member and keeps the call. What mute does on the wire is
`CALL-074` and `CALL-075`. Hang up is a leave, so the relay takes the seat out of the roster. Each
reader of the call message then reads the member out of the call (`MURMURE-604`). A last hang-up
writes the end, as any last leave does (`MURMURE-591`).

**The ring.** A call of a direct message rings the other member. The call message alone tells
that member's pages of the call. Its sealed summary carries a cue: the call, a ring, and the
instant its writer wrote it (`1-Foundations/ambre-spec-residency.md section 3`). A page that does
not hold the channel reads the cue from the summary alone, as it counts a badge (`MURMURE-641`).

1. The caller starts the call from the header, and posts the call message once seated
   (`MURMURE-408`). Its header says that it waits for an answer.
2. Each open page of Murmure of the other member hears the cue. A free page rings: an alert
   names the caller, and a tone sounds. The alert offers accept and decline.
3. Accept joins the call as Join call does, the microphone first (`MURMURE-642`).
4. Decline writes the end of the member into the call message, under a decline cue. A page
   that does not hold the channel opens it to write the end (`MURMURE-642`).

The ring ends at the first of these:

| The event | The page of the member | The page of the caller |
|---|---|---|
| the member accepts | stops ringing, and joins | stops waiting once the call admits the member (`MURMURE-644`) |
| the member declines | stops ringing | hangs up, and says the call was declined (`MURMURE-644`, `MURMURE-645`) |
| the ring timeout passes | stops ringing, and writes nothing (`MURMURE-643`) | hangs up, and says no answer (`MURMURE-644`, `MURMURE-645`) |
| the caller hangs up | stops ringing at the end cue (`MURMURE-650`) | ends the call |

A hang-up of the caller is the last leave, so it writes the end under an end cue. That end
also stops the ring of another tab of the member that declined. A page in a call rings for no
other call, and a cue older than the timeout rings nothing: a reconnect replays old cues
(`MURMURE-648`). A call of any other channel rings no page: its members join from the call
message (`MURMURE-649`).

The timeout reads the instant of the cue on the clock of the reader. So a drift between the two
clocks moves the end of the ring on the page of the member. The ring timeout is the call ring row of
`ambre-spec-budgets.md section 1`.

The title of the alert reads as an alert, so a screen reader reads it when it shows. Accept and
decline are buttons with an accessible name, each in the order of the keyboard (`MURMURE-646`).
The alert takes no focus: a member who types keeps the key in the field. The tone is
synthesized by the page, two pitches together, one second on and two off. A browser that holds
audio until a gesture keeps the tone silent, and the alert still shows.

**What the relay learns by a ring** is what a call message and an end tell it (`ZK-001`). That
is an update of the channel, at its instant. The cue travels sealed inside it. A page that declines
and does not hold the channel asks for its documents, as a read does (`MURMURE-647`).

**The devices.** The devices control sits beside the call controls of the header, in a call and
out of one. It opens the device panel: a choice of microphone, and a choice of speaker where the
browser can route the audio of a call (`MURMURE-609`). Out of a call, the panel is where the member
sets the devices of the next call. Murmure has no settings screen, so the panel is its setting.

| The browser lists | The panel shows |
|---|---|
| no name, before the microphone is granted | the default microphone alone, and that names show once it is allowed (`MURMURE-608`) |
| each device by name | the default one, then each device (`MURMURE-607`) |
| no output by its identifier | no choice of speaker |

The choice stays in the storage of the browser on this device (`MURMURE-612`). It never reaches a
channel or the settings document of the user (`MURMURE-613`). A device identifier names nothing on
another device, and a list of devices nearly names a machine (`ZK-026`).

A chosen device can go away. The call goes on with the default one, which
`1-Foundations/ambre-spec-call-media.md` states. The header then shows which device the call uses,
until the member chooses again or the call ends (`MURMURE-610`):

| A device goes away | The header shows |
|---|---|
| the chosen microphone | the call sends the default microphone |
| the chosen speaker | the call plays on the default output |

The choice stays stored: the device that comes back serves the next call.

**The volumes.** During a call, the device panel also offers the volumes of the call:

| The panel shows | When |
|---|---|
| a volume for each other participant, named by its member (`MURMURE-614`) | the browser sets the volume of an audio element |
| no volume for a participant (`MURMURE-615`) | the volume of an element is read-only, as on iOS |
| the gain of the microphone, a return to the automatic gain, and a meter of the level sent (`MURMURE-616`) | always, during a call |

Each volume and the gain is a slider with an accessible name, which the arrow keys move. The
return to the automatic gain is a checkbox, and the meter names the level it shows
(`MURMURE-617`). A slider moved by hand turns the automatic gain off. What a volume and a gain do
to the sound is `1-Foundations/ambre-spec-call-media.md section 3`. The panel shows no volume for
the whole call: the system holds it.

The page keeps each volume by member, and the gain, in the storage of the browser on this device
(`MURMURE-618`). A volume suits one listener on one device. It never reaches a channel
or the settings document of the user (`MURMURE-619`, `ZK-027`).

**The camera.** During a call, the header offers the camera button beside mute. A press opens the
camera and sends it into the call. A second press stops it (`MURMURE-622`). How the sender takes the
camera is `1-Foundations/ambre-spec-call-media.md section 3`. The call button starts a call with the
camera off, and a press of the camera button makes it a video call.

| The camera | The button shows | Its pressed state |
|---|---|---|
| off | `MdiVideo`: turn the camera on | not pressed |
| asked of the browser | `MdiVideo`, disabled | not pressed |
| sent into the call | `MdiVideoOff`: turn the camera off | pressed |

The camera button is a button with an accessible name, and it takes the focus (`MURMURE-624`). A
press writes nothing into the channel (`MURMURE-625`).

A camera the browser does not open leaves the call on its audio. The header then shows why, until the
next press of the camera button or the end of the call (`MURMURE-623`):

| The browser answers | The header shows |
|---|---|
| a refusal of the user or of a policy | the camera is blocked, and the call goes on with its audio |
| no capture device | no camera is found, and the call goes on with its audio |
| any other failure | the camera did not open, and the call goes on with its audio |

**The screen.** During a call, the header offers the share button beside the camera button. A
press asks the browser for a screen or a window, and the browser shows its own picker. The pick
goes into the call, and a second press stops it (`MURMURE-632`). How the sender takes the screen is
`1-Foundations/ambre-spec-call-media.md section 3`.

| The share | The button shows | Its pressed state |
|---|---|---|
| off | `MdiMonitorShare`: share the screen | not pressed |
| asked of the browser | `MdiMonitorShare`, disabled | not pressed |
| sent into the call | `MdiMonitorOff`: stop sharing the screen | pressed |

The share button is a button with an accessible name, and it takes the focus (`MURMURE-633`). A
press writes nothing into the channel (`MURMURE-631`).

A capture the browser does not give leaves the call as it was. The header then shows why, until the
next press of the share button or the end of the call (`MURMURE-634`):

| The browser answers | The header shows |
|---|---|
| a refusal of the user or of a policy, or a picker the member closed | the screen is not shared: the share was refused or cancelled |
| no screen to capture | the screen is not shared: no screen is found |
| any other failure | the screen is not shared: the capture did not open |

A browser cannot tell a refusal from a picker the member closed: both read as a refusal.

The browser bar also stops a share, and so does the close of a shared window. The button then
reads off. A later share of another member takes the screen, and the member who shared stops. The
header then says that another member shares now, until the next press or the end of the call
(`MURMURE-630`).

**The video region.** While the page is in a call, the channel region shows a video region between
its message list and its composer (`MURMURE-620`). The list gives up the height of the region. A
call with every camera off shows the region too: it says who is in the call and who speaks.

Each participant of the page shown takes a tile (`MURMURE-651`). The region takes one of three
layouts, and none is a menu: the region picks it from the call.

| Layout | When | Its page of other participants | The member's own tile |
|---|---|---|---|
| band | by default, a row above the composer | 5 over the phone width, 3 under it | apart, at the end, smaller (`MURMURE-684`) |
| fill grid | after the fill button, with no screen and no pin | 11 over the phone width, 8 under it | the last cell, the size of every cell (`MURMURE-652`) |
| scene | widened, with a received screen or a pin | 4 in the strip | the last place of the strip (`MURMURE-652`) |

The page sizes follow the room each layout has. Nine tiles in a band of about 200 pixels draw
faces of 50 pixels, which nobody reads. The fill grid takes the columns that give the largest
tiles, its own tile counted (`MURMURE-686`). That is four by three on a wide desktop, and three by
three on a phone. It never sets a column aside for the member. The rules of the tiles:

1. The participants keep the order the call admitted them, with a video or without one.
2. A choice of videos moves no tile: a page shows the same members before and after its videos
   arrive.
3. The grid takes the count of columns that gives the largest tiles, and sits centred.
4. Each tile keeps the aspect of a camera, with a strip below it that names its member.
5. A tile whose member sends no video shows the initials of its member.

The page learns the member of each video slot as it learns the member of an audio slot. The call
worker names the sender of the slot at its first chunk (`CALL-084`). The Sync worker names the
member of that sender. The render runtime draws the picture inside its tile (`CALL-015`).

The SFU forwards eleven videos at most to a desktop and nine to a phone (`CALL-041`). So the
largest page of each width holds each picture a receiver gets. Past the page size of its layout,
the region shows the other participants in pages (`MURMURE-654`). An arrow at each end of the band, the
grid or the strip turns the page. The header offers the same two turns as buttons.
A participant who speaks on another page marks the arrow toward it (`MURMURE-688`).

The page names the count of videos of its width in each choice (`MURMURE-704`). A phone asks for
nine, so the fill of the SFU spends no downlink on a video the phone has no tile for.

The grid opens on the first page, and the turns stop at the first and at the last. A page past the
last, after a leave, shows the last. The list of every participant opens from the participants
button of the header. That list is in the page, so a participant the canvas draws no tile for
still reaches a screen reader (`CHROME-029`).

The page asks the SFU for the video of exactly the members whose tile the grid shows
(`MURMURE-667`). It names each one by its sender index, in the order of the grid. A member who
joined again names the sender the call admitted last. A new choice leaves at each turn of the
page, each pin and each change of the participants, and never twice the same. The order of the
grid follows the admission, so a turn of the page asks for the senders of that page alone.

The SFU fills a slot that a short last page leaves with the most recent speakers (`CALL-041`). The
grid draws no tile for them.

A press on the tile of another participant pins it, and a second press unpins it (`MURMURE-668`).
The pin button of the list does the same. In the band, the pinned tile sits at the start, larger
than each other tile, on every page. Widened, a pin lays out the scene with the pinned tile as its
subject (`MURMURE-687`). The pinned member takes a place of the page, so the choice still names
the count of the width at most.

The choice marks the pinned member large. The SFU then sends the top layer of that member that
the estimate holds, even while another member speaks (`CALL-128`). A pinned member who leaves the
call frees the pin. A turn of the page, a pin and the list write nothing into the channel. The
choice reaches the socket of the SFU alone (`MURMURE-670`, `ZK-044`).

In the band, the member's own tile sits apart, at the right end, smaller than each other tile
(`MURMURE-684`). It reads "You", and it marks a mute of the member (`MURMURE-685`). While the camera
goes into the call, the tile draws it mirrored, as a mirror shows a face (`MURMURE-656`). The page
hands the render runtime the track the video sender already carries, so the preview opens no second
capture and touches no session (`CALL-100`). With the camera off, the tile shows the initials of the
member. No other tile marks a mute: a mute sends silence, and nothing tells a receiver of it
(`ZK-025`).

The tile of a participant who speaks is outlined (`MURMURE-653`). The page reads the audio level of
each received audio slot, once per interval of the call speaker row of `ambre-spec-budgets.md
section 1`. A level past the level of speech of that row outlines the member of the slot. The
outline stays until the hold of that row after its last speech. The browser reads that level from
the clear header extension of each packet, which the SFU already reads to rank speakers
(`CALL-047`). So the outline adds nothing to what the SFU learns, and the page writes nothing of it
(`ZK-025`).

A received screen takes a tile taller than each tile of a camera (`MURMURE-629`). The first
picture of a screen widens the call into the scene (`MURMURE-692`). The screen spans the region and
the faces move to a strip beside it, or under it on a phone (`MURMURE-687`). Text on a screen
needs the room, and the faces stay in view.

A member who brings the band back keeps it until
another share starts (`MURMURE-694`). The widening writes nothing and leaves the focus where it
was, so a member who types keeps typing (`MURMURE-695`). The page shows no screen it shares
itself.

The fill button of the header widens the call to the whole height above the composer, and a
second press brings back the band (`MURMURE-657`). The list then keeps no height, and the composer
stays, so the member still writes during the call (`MURMURE-658`). The grid grows with the region,
which makes a face among nine readable. The button carries an accessible name and its pressed
state, and the keyboard reaches it (`MURMURE-659`). The band comes back at the end of the call.

| The call | The button shows | Its pressed state |
|---|---|---|
| a band | `MdiFullscreen`: fill the channel region with the call | not pressed |
| the whole region | `MdiFullscreenExit`: shrink the call to a band | pressed |

Out of a call, the region takes no height (`MURMURE-621`). The thread panel shows no video
region.

**Away from the channel of the call.** The call lives in the page, beside the one router of the
suite, so it goes on in another channel or another application (`MURMURE-701`). The page then
shows the call where the member is:

| Width | Shows | Holds |
|---|---|---|
| over the phone width | the call bubble, floating over the content | the channel, the time since the start, the faces of those who speak, mute, the camera, the share, the way back and hang up (`MURMURE-696`) |
| under the phone width | the call pill, under the header | the channel, the time since the start, the faces, mute and hang up (`MURMURE-697`) |

The way back of the bubble, and a press on the pill, show the channel of the call in Murmure
(`MURMURE-700`). Neither writes into a channel, moves a read cursor or changes the choice of
videos (`MURMURE-702`). Each control names itself and takes the focus (`MURMURE-693`).

The bubble rests in a corner of the content. A drag moves it, and its release rests it in the
nearest corner (`MURMURE-703`).
 It floats over the canvas, so it publishes the part it covers, as
each floating surface of the shell does (`MURMURE-698`, `CHROME-171`). A press on its faces would
show the video of the member who speaks, or the screen. The render runtime of the page draws a
video, never an element over the canvas (`CALL-015`). So the bubble shows no video until the
runtime of Prose, Trame and Toile draws a slot of the call (`MURMURE-699`).

**Present a document.** A screen share of an Ambre document sends pixels of what every member
could open. Present opens the document itself for each participant who is a member of it, and
each follows the view of the presenter (`MURMURE-705`). Each keeps their own panel, selection and
tools. An edit or a scroll stops following, and a press on the follow bar resumes it
(`MURMURE-707`, `MURMURE-708`).

A participant who is no member sees nothing of the document (`MURMURE-706`). The presenter is
offered to invite them as a viewer, and to share the screen meanwhile. An invite hands the key
chain of the document, so the viewer reads its whole history (`SYNC-116`, `SYNC-119`). Taking it
back owes a rotation. Present writes no membership on its own (`MURMURE-710`).

The call carries none of the document. A sender key derives from no document key (`CALL-002`), and
the call opens no data channel (`CALL-010`). The document reaches each follower through its own
sync, under its own key (`MURMURE-709`).

**Code.**

| File | Holds |
|---|---|
| `crates/ambre-murmure/src/logic/call.rs` | the call of a message, its end and who is in it |
| `crates/ambre-murmure/src/logic/delta.rs` | the check of an end under the key of its writer |
| `crates/ambre-murmure/src/logic/runtime/calls.rs` | the start, the end and the asks of a channel |
| `crates/ambre-sync/src/call_roster.rs` | the ask of the relay and the check of each signed join |
| `crates/ambre-server/src/routes/calls.rs` | the answer of the relay to an ask |
| `crates/ambre-murmure/src/ui/call_controls.rs` | the controls the header offers in each phase, and the refusal it shows |
| `crates/ambre-app/src/shell/call_bar.rs` | the view of the call controls, with their icons |
| `crates/ambre-app/src/call.rs` | the microphone, the mute and the hang-up of the page |
| `crates/ambre-murmure/src/ui/call_devices.rs` | the device panel, the fallback and the stored choice |
| `crates/ambre-app/src/call_devices.rs` | the device list, the change of a device and the stored choice of the page |
| `crates/ambre-murmure/src/ui/call_volumes.rs` | the volume of each slot, the stored volumes, the gain and the level of the meter |
| `crates/ambre-app/src/call_volumes.rs` | the volume of each element, the graph of the gain and the meter of the page |
| `crates/ambre-murmure/src/ui/call_ring.rs` | the steps of a ring, on the page of the member and on the page of the caller |
| `crates/ambre-app/src/call_ring.rs` | the ring of the page: its alert, its tone, its timer, and the join, the decline or the hang-up a step asks for |
| `crates/ambre-murmure/src/ui/call_videos.rs` | the band, the grid and the tiles of the video region, the screen included, and who speaks |
| `crates/ambre-app/src/call_stage.rs` | what the page shows of its call: the participants, the video of each, the level of each audio slot |
| `crates/ambre-canvas/src/video.rs` | the detached video element of each slot, which the render runtime copies |

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a start, from the page or the bus | a call starts, and a call message appears once the starter is seated |
| Join call, in the menu of a live call message | the member joins the call |
| the tick of a channel the view shows | each live call of the held segments is asked for, once per interval |
| the last leave | the call message shows the call ended |
| the call button in the header | the microphone opens, then a call starts in the shown channel, or the relay seats the member in its call |
| mute, during a call | the others hear silence from the member, until a second press |
| hang up, during a call | the member leaves the call |
| a refused microphone | the header shows why the call did not start |
| the devices control in the header | the device panel opens |
| a device chosen in the panel | the next call opens it, and a call in progress moves to it |
| a chosen device that goes away | the call goes on with the default one, and the header says so |
| a volume or the gain set in the panel | the call plays or sends at it, and the next call keeps it |
| the camera button, during a call | the camera goes into the call, or leaves it |
| the share button, during a call | a screen or a window goes into the call, or leaves it |
| a refused or a cancelled capture of the screen | the header shows why no screen is shared |
| a share of another member, while the page shares | the share stops, and the header says so |
| a refused camera | the call goes on with its audio, and the header shows why |
| a video received | the video region shows it in the tile of its member, above the composer |
| a call message of another member, in a direct message | the page rings, with an accept and a decline |
| accept, on the ring alert | the member joins the call |
| decline, on the ring alert | the end of the member goes into the call message, and the caller hangs up |
| the ring timeout | the ring stops, and the caller hangs up |
| a call joined or started | the video region shows a tile for each participant, and the member's own tile |
| a participant who speaks | its tile is outlined |
| the camera button, during a call | the member's own tile draws the camera mirrored, or its initials |
| the fill button, during a call | the call takes the whole channel region above the composer, or its band again |
| the participants button, during a call | the list of every participant opens, with a pin button for each |
| a press on an arrow of the grid, or a page button of the header | the grid shows the next or the previous page, and the page asks the SFU for its videos |
| a press on the tile of another participant, or its pin button | the participant is pinned large on every page, or unpinned |
| the first picture of a received screen | the call widens into the scene, the screen across it and the faces in a strip |
| another channel or another application, during a call | the call bubble on a desktop, the call pill on a phone |
| the way back of the bubble, or a press on the pill | the channel of the call opens in Murmure |
| a drag of the bubble | the bubble follows, and rests in the nearest corner |
| Present, during a call | each participant who is a member follows the document of the presenter |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A call message in the channel | a call outside any channel | the members would share a link by hand, and anyone holding it could join |
| A call message in the channel | a voice room that stays open in the channel | a room shows presence at every instant, and it holds a connection to the SFU for each member who waits in it |
| Calls are a feature of Murmure | a separate application for calls | a call has no document of its own, and it starts only from a channel. Its admission is the membership of that channel |
| One live call per channel, held by a key of the relay | two racing starts make two calls | two members who press at one instant would each wait alone in a call of their own |
| Who is in the call comes from the relay | each participant writes its join and leave into the message | every join would be a write into a message of another author, and the history of the channel would hold who was in each call |
| The end is written by the last leave | the relay keeps that a call ended | the relay would hold the call history of the channel past the time to live (`ZK-022`) |
| The microphone opens before the start or the join | a call that starts, then asks for the microphone | a refusal would leave a call message and a seat that no sound reaches, a silent call |
| Mute silences the track | mute stops the sender of the track | the SFU would read a mute apart from a silence (`ZK-025`) |
| The device choice stays in the storage of the browser | the settings document of the user | the document reaches every device of the user, where an identifier names no device, and the relay would store a list that nearly names each machine |
| The volumes stay in the storage of the browser | the settings document of the user | a volume suits one listener on one device and its speaker, and the relay would store whom each member turns down |
| The canvas draws the videos | a video element over the canvas | the page would hold an element over the canvas, which no overlay of the canvas could cover (`CALL-015`) |
| The video region sits above the composer | the region over the top of the message list | the list would hide its latest messages under the videos, or move its origin and its hit test |
| The own tile draws the camera mirrored | the camera as the others receive it | a face that moves left on the screen when its owner moves right reads as wrong, and every call client mirrors its preview |
| The fill button widens the call inside the channel region | the full screen of the browser | the composer and the header would leave the screen, and the browser asks a gesture and shows its own notice each time |
| Only the member's own tile marks a mute | each tile marks the mute of its member | a mute would travel on the wire, and the SFU would read a mute apart from a silence (`ZK-025`) |
| The page reads who speaks from the audio level of each received slot | who speaks from the slots of the SFU | a slot changes its sender only when a fourth member speaks, so the slots tell who spoke last, never who speaks now |
| A page size per layout | pages of nine in every layout | nine tiles in a band of about 200 pixels draw faces of 50 pixels, and a filled desktop leaves a column empty |
| A page size per layout | a tile for each participant, past the page | the SFU forwards eleven videos, and fifty tiles in one region leave no face readable |
| A page size per layout | a last tile that counts the others and opens their list | a member past the page would reach no tile and no video, only a name in a list |
| The own tile is the last cell of the fill grid | the own tile apart from the fill grid, smaller | it keeps a column for one small tile, so a grid of eleven loses a row |
| Three layouts the call picks | a gallery and speaker menu | the member chooses a layout the content already tells: a screen wants the room, and a pin names its subject |
| A screen widens the call | the screen waits in the band for the fill button | text in a band cannot be read, and each member would press the button at each share |
| A bubble away from the channel | the call ends when the member leaves its channel | a member who opens the document under discussion would drop out of the discussion |
| A bubble away from the channel | a bar of call controls across the bottom of every page | it costs a row of height on every page of the suite, and the controls already sit in the channel header |
| Present opens the document | a screen share of the document | the others read pixels, at the bitrate of a screen, of a document each member could open sharp, with tools of their own |
| Present for members alone | a projection of the document to a participant who is no member | the projection must draw a view with no key of the document, which no spec designs yet: a story of its own |
| The sidebar shows a count | the names of the participants in the sidebar | the sidebar is narrow, and a list of names under each channel pushes the channels out of view |
| The grid keeps the order of admission | the participants with a video first | each choice brings videos, so the tiles would move under each turn, and a page would not know its senders |
| The page asks for the tiles it shows | the page lets the SFU pick the speakers | a member on the page who does not speak would show its initials, and a speaker off the page would cost a video that no tile draws |
| The pinned tile takes a place of the page | a pinned tile beside a full page | the choice would name one sender past the count the SFU forwards |
| A phone asks for nine videos | a phone receives eleven as a desktop does | the fill would send two videos no tile of the phone draws, over the downlink of a phone |
| The call button starts a call with the camera off | a second button that starts a video call | the header would offer two buttons for one call, and a press of the camera button reaches the same call |
| A received screen takes the scene | the screen in a tile of the row of the cameras | the tile of a camera is about 160 pixels tall, where the text of a screen cannot be read |
| A share stops when another member shares | a share that goes on unseen | the member would show a screen that no other member sees |
| A reader asks while the view shows the channel | the relay pushes the roster to every member of the channel | the relay would send who is in a call to sessions that show nothing of it |
| The ring rides the sealed summary of the call message | a ring frame the relay forwards to the members of the direct message | the relay would learn whom a call rings and who declines it, beyond the call it already holds |
| A decline writes the end of the member | a decline that writes nothing | the caller would wait the whole timeout for a member who already said no |
| A call of a direct message rings | every call of a channel rings | each member of a large channel would ring for each call |
| The alert takes no focus | the alert takes the focus | a member who types would accept or decline with the next key |
| The tone is synthesized by the page | a sound file | the page would ship and fetch a file for one tone |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A removed member stays in the list until it leaves.** The reader keeps the membership log it
  read, and reads it again only when a join names a member that log does not hold.
* **A page with every tab closed rings nothing.** A ring then needs a push, which lands with the
  `Push notifications` Story (`ambre-roadmap.md` E5).
* **A page of Prose, Trame or Toile rings nothing.** The alert sits in the shell of Murmure.
* **A long-hidden tab may ring late or stop late.** A browser may slow the timers of a hidden
  tab, which delays the timeout.
* **No key turns the page of the call.** Page Up and Page Down scroll the message list, a key of
  the region (`MURMURE-541`). The arrows and the header buttons turn it.
* **The label on the screen is not chosen.** The term is `call` in the corpus; the word the
  interface shows belongs to `ambre-spec-locale.md`.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How the audio and video of a call are encrypted, signalled and carried | `1-Foundations/ambre-spec-call-media.md` |
| What the relay and the SFU learn of a call | `1-Foundations/ambre-spec-zero-knowledge.md` |
