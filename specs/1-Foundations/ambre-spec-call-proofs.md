# AMBRE -- FEATURE: THE CALL PROOFS ON REAL HARDWARE
**Type:** Horizontal (framework)

> The harnesses that measure a call on hardware no CI job runs. That hardware is a phone, a
> desktop with a GPU, or two machines on an impaired network. Each prints each figure of its
> roadmap row against its threshold, with a verdict. A person runs it and copies the dated figure
> into the budget register.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `PROOF`, and it belongs to
this document alone. These rows hold the harnesses. The budgets the harnesses measure stay rows of
`ambre-spec-budgets.md`, and no row here verifies one of them.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROOF-001` | `INV` | The proof host MUST serve the page of the build with the probe as the first script of its head. | this spec section 2, the proof host | `ambre_belt::proof::tests::the_probe_is_the_first_script_of_the_head` | none | verified |

### 1.2 Report a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROOF-002` | `POST` | Every 2 s, the probe MUST report the paints, the counters of each received video and the chunk totals of the call worker. | this spec section 2, the probe | `ambre_belt::proof::run::the_proof_host_reads_a_call_through_its_probe` -- browser tier, under Chrome and under Firefox: the paint phase holds paints and two videos, the worker phase holds chunks | none | verified |
| `PROOF-003` | `FRAME` | The timer MUST NOT change a chunk the call worker reads or writes. | this spec section 2, the timer; `CALL-001`; `CALL-022` | `ambre_belt::proof::run::the_proof_host_reads_a_call_through_its_probe` -- browser tier: the receiver decodes the sealed video of both senders through the timer | none | verified |

### 1.3 Load the Sync worker

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROOF-004` | `POST` | On the order of the host, the probe MUST keep the Sync worker sealing a file, one seal after the other, until the order ends. | this spec section 2, the phases; `DOCFILE-014` | `ambre_belt::proof::run::the_proof_host_reads_a_call_through_its_probe` -- browser tier: the worker phase holds one seal or more, and none failed | none | verified |
| `PROOF-005` | `FRAME` | Without the order of the host, the probe MUST NOT ask the Sync worker for a seal. | this spec section 2, the phases | `ambre_belt::proof::run::the_proof_host_reads_a_call_through_its_probe` -- browser tier: the paint phase holds no seal | none | verified |

### 1.4 Judge a phase

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROOF-006` | `POST` | The proof MUST read the 99th percentile of one chunk from the chunks of its phase alone. | this spec section 2, the timer | `ambre_belt::proof::tests::the_99th_percentile_reads_the_chunks_of_the_phase_alone` | none | verified |
| `PROOF-007` | `POST` | The proof MUST fail a phase whose 99th percentile of one chunk reaches 5 ms. | `ambre-spec-budgets.md section 1`, the call chunk row | `ambre_belt::proof::tests::a_chunk_below_five_ms_at_the_99th_percentile_passes_and_one_at_five_fails` -- the bin from 5.0 ms fails | none | verified |
| `PROOF-008` | `POST` | The proof MUST fail a phase whose call worker holds a chunk over a quarter of the wall clock or more. | `ambre-spec-budgets.md section 1`, the call worker load row | `ambre_belt::proof::tests::a_chunk_below_five_ms_at_the_99th_percentile_passes_and_one_at_five_fails` -- 2 500 ms held over 10 000 ms fails | none | verified |
| `PROOF-009` | `POST` | The proof MUST fail a paint phase whose 95th percentile of one paint reaches 4 ms. | `ambre-spec-budgets.md section 1`, the call video paint row | `ambre_belt::proof::tests::paints_read_their_rate_and_their_95th_percentile` -- a phase of paints at 4 ms fails | none | verified |
| `PROOF-010` | `POST` | The proof MUST fail a paint phase with fewer than 30 paints a second. | `ambre-roadmap.md`, the `Call client proofs on Android` row | `ambre_belt::proof::tests::paints_read_their_rate_and_their_95th_percentile` -- 29 paints a second fail | none | verified |
| `PROOF-011` | `POST` | The proof MUST fail a phase whose slowest received video decodes fewer pictures a second than the floor of the phase. | `ambre-roadmap.md`, the two rows of the call client proofs | `ambre_belt::proof::tests::nine_videos_at_one_top_and_eight_low_pass_the_decode_proof` -- 25 pictures a second pass a floor of 25 and fail one of 30 | none | verified |
| `PROOF-012` | `POST` | The proof MUST fail a decode phase unless one video holds the top size and eight hold the lowest. | `ambre-roadmap.md`, the `Call client proofs on iOS` row | `ambre_belt::proof::tests::nine_videos_at_one_top_and_eight_low_pass_the_decode_proof`; `::a_video_that_decodes_nothing_in_the_phase_is_not_counted` | none | verified |
| `PROOF-013` | `POST` | The proof MUST fail unless each microphone of the receiver reports its echo cancellation on. | `ambre-roadmap.md`, the `Call client proofs on iOS` row | `ambre_belt::proof::tests::echo_cancellation_passes_only_when_every_microphone_reports_it` | none | verified |
| `PROOF-014` | `FRAME` | Judging a phase MUST NOT count a paint or a seal reported before the phase. | this spec section 2, the phases | `ambre_belt::proof::tests::paints_read_their_rate_and_their_95th_percentile` -- a paint of 50 ms before the phase counts for nothing; `::the_seals_of_the_phase_count_from_its_second_report` | none | verified |
| `PROOF-015` | `FAIL` | Given a phase with no report of the call worker, the proof MUST fail the phase with a figure that names the absence. | this spec section 2, the timer | `ambre_belt::proof::tests::a_phase_with_no_report_of_the_call_worker_fails` | none | verified |
| `PROOF-016` | `FAIL` | Given a phone run with no address or no certificate, the proof MUST refuse to start and name the variable it lacks. | this spec section 3 | `ambre_belt::proof::tests::a_plan_names_what_a_phone_lacks` | none | verified |

### 1.5 Judge the stall

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROOF-017` | `POST` | The stall proof MUST fail unless each received video freezes under 1 s per 10 minutes. | `ambre-spec-budgets.md section 1`, the call video stall row | `ambre_sfu::sim::browsers::tests::each_received_video_must_freeze_under_one_second_per_ten_minutes` -- 1.0 s per 10 minutes fails | none | verified |
| `PROOF-018` | `FAIL` | Given a received video whose browser counts no freeze, or no received video at all, the stall proof MUST fail. | this spec section 2, the stall proof | `ambre_sfu::sim::browsers::tests::a_video_with_no_freeze_counter_or_no_video_at_all_holds_nothing` | none | verified |
| `PROOF-019` | `FRAME` | Judging the stall MUST NOT count a video that decoded under 30 pictures in the run. | this spec section 2, the stall proof | `ambre_sfu::sim::browsers::tests::a_video_with_no_freeze_counter_or_no_video_at_all_holds_nothing` -- a video of 10 pictures is not counted | none | verified |

### 1.6 Sign a participant in

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROOF-020` | `POST` | The identity provider of the belt MUST sign each person in under an identity token the relay verifies, for the code and the verifier of that person's own authorization alone. | this spec section 2, the sign-in; `ambre-spec-session.md`, `SRV-321`; `ambre-spec-zero-knowledge.md`, `ZK-050` | `ambre_belt::provider::tests::each_person_signs_in_at_the_provider_under_a_verified_identity` -- two people at once, each token verified for its own subject, address and name, a code refused a second time and under another verifier; `ambre_belt::provider::tests::a_page_signs_in_through_the_provider_of_the_belt` -- browser tier: a relay configured with the provider signs a page in, which sets up its keys; `ambre_belt::provider::tests::the_page_of_a_phone_stores_the_state_and_opens_the_return` -- the sign-in page of a phone stores the state and opens the return | none | verified |

* **Budget.** The proofs measure four rows of `ambre-spec-budgets.md section 1`, and hold none of
  them. The four are the call chunk, the call worker load, the call video paint and the stall.
* **Boundary.** none: the proof host forwards the relay and the SFU unchanged. The probe and the
  timer read timings and counters, never a chunk or a key.
* **Verification.** 19 `verified`. No row here verifies a budget: a person reads each figure on real
  hardware into the budget register.

---

## 2. MODEL

**CRDT containers.** none. A proof holds no document state.

**The four proofs.**

| Proof | Roadmap row | Receiver | What it prints |
|---|---|---|---|
| Android | `Call client proofs on Android` | a Pixel 9a under Chrome | the paint phase and the worker phase |
| iOS | `Call client proofs on iOS` | an iPhone under Safari | the two phases of Android, then a decode phase of 30 minutes |
| Desktop paint | the `Call video paint` row of the budget register | a Chrome with a window and its GPU on the desktop that runs the senders | the paint phase and the worker phase |
| Stall | `Call video stall on real hardware` | two Chrome and two Firefox on a second machine | the freezes of each received video |

**The proof host.** One process of the belt starts the relay, the SFU and the proof host.

| Path | The host |
|---|---|
| a file of the build | serves the page with the probe, built under the cargo profile of the release |
| `/api` | forwards it to the relay, as a deployment does |
| `/sfu/call` | forwards it to the signalling of the SFU |
| `/proof/sign-in` | signs the phone in through the identity provider of the belt (`PROOF-020`) |
| the media of the SFU | listens on the address of the machine on the network of the phone |
| the scheme | HTTPS under a certificate the phone trusts, since a camera needs a secure context; the loopback for a receiver on the desktop |

**The senders.** Nine headless Chrome pages of the belt run on the machine of the host. Each signs
in through the relay and joins one channel. Each joins one video call with a fake camera at 30 fps,
and seals through its own call worker. The receiver then opens real sealed chunks. Eight senders
stay muted, so the top size stays on the video of the ninth.

**The sign-in.** The belt runs an identity provider on the loopback, and a relay started with its
variables discovers it as its provider (`SRV-321`). The provider signs an identity token for each
person it authorizes (`PROOF-020`). A page never follows its address, because the client follows
the host of Google alone. The belt asks the relay for the address and takes the code at the
provider. It then opens the return on the page, which runs the exchange of a release build. The
belt links the provider crate, an RSA key and a clock for it, and the relay already links all three.

Each participant signs in that way, and none by a bare name (`ZK-050`). So the build of a proof
holds no development sign-in, and the release gate passes it (`ZK-049`). A sender opens Murmure and
follows an invite link by a reload. The session the Sync worker kept survives each reload
(`ambre-spec-session.md`, `SRV-322`). A phone signs in at `/proof/sign-in` on the host, which stores
the state of the sign-in and opens the return, as the belt does through WebDriver.

**The probe.** The host injects one script at the head of the page (`PROOF-001`). It runs before
the application boots, and every 2 s it reports to the host:

| Field | Read from |
|---|---|
| the paints | the `ambre:video-paint` measures of the UI thread since the last report |
| the received videos | `getStats()` of the peer connection: size, pictures decoded, freezes |
| the sent encodings | `getStats()`: size and pictures encoded of each of the three sizes |
| the call worker | the totals of the timer, below |
| the seals | the seals the Sync worker answered since the last report, with the time of each |
| the microphone | the echo cancellation setting of each local audio track |

**The timer.** The probe starts the call worker through a wrapper script.

* The wrapper imports the timer, then the shipped loader: the shipped worker runs unchanged
  (`PROOF-003`).
* The timer marks the time the worker reads a chunk, and the time it writes the chunk on.
* Each span falls in a bin of 0.05 ms, which gives the 99th percentile of one chunk.
* The union of the spans over the wall clock is the held share. The worker has one thread, so the
  held share bounds from above the share of one core its transforms take.

**The phases.** Once the receiver decodes nine videos and sends its camera, the run waits 10 s.

1. The paint phase, 60 s: the probe asks for no seal.
2. The worker phase, 60 s: the host orders seals. The probe posts the command of a sealed file
   export to the Sync worker, as the Logic thread does. It posts the next one when the worker
   answers, so the Sync worker seals during the whole phase (`PROOF-004`).
3. The decode phase, 30 minutes, for iOS alone: no seal.

The first report of a phase closes the time before it, so no earlier figure enters it
(`PROOF-014`).

**The figures of a phone.**

| Figure | Phase | Threshold | Source of the threshold |
|---|---|---|---|
| chunks per second | worker | read only; the row names about 1 500 | `ambre-spec-budgets.md section 1`, the call worker load row |
| one chunk held, 99th percentile | worker | under 5 ms | the call chunk row |
| call worker held, share of one core | worker | under 25 % | the call worker load row |
| files the Sync worker sealed | worker | one or more, none failed | the call worker load row: "while the Sync worker seals a file" |
| video paints per second | paint | 30 or more | `Call client proofs on Android` |
| UI thread per paint, 95th percentile | paint | under 4 ms | the call video paint row |
| videos decoding | paint, decode | 9 or more | the call videos forwarded row |
| slowest video, pictures decoded a second | paint, decode | 30 in the paint phase, 25 in the decode phase | the two roadmap rows |
| video sizes | decode | one at 1280 wide, eight at 320 | `Call client proofs on iOS` |
| echo cancellation of the microphone | end | on | `Call client proofs on iOS` |

**What no page reads.** The battery and the crackle of the audio have no figure in a page.
Section 3 gives the procedure a person follows for each.

**The stall proof.**

| Element | Setting |
|---|---|
| the SFU machine | the SFU and 46 synthetic participants, in the load run of the SFU simulation |
| the receivers' machine | two Chrome and two Firefox, each behind a WebDriver server there |
| a browser | sends its fake camera in three sizes, and decodes the three other browsers |
| the network | `tc netem` on the receivers' machine: 75 ms of delay and 2 % of loss each way, so 150 ms of round trip |
| the length | 10 minutes after a warm-up |
| a received video | one that decoded 30 pictures or more (`PROOF-019`) |
| the figure | `totalFreezesDuration` of `getStats()`, per received video |

**Code.**

| Path | Holds |
|---|---|
| `crates/ambre-belt/src/proof.rs` | the figures, the verdicts, the plan and the proof host |
| `crates/ambre-belt/src/proof/run.rs` | the run: the services, the senders, the receiver and the phases |
| `crates/ambre-belt/src/proof/probe.js`, `crates/ambre-belt/src/proof/call_shim.js` | the probe and the timer |
| `crates/ambre-belt/src/provider.rs` | the identity provider of the belt, the sign-in of a page through it, and the sign-in page of a phone |
| `crates/ambre-sfu/src/sim/load.rs`, `crates/ambre-sfu/src/sim/browsers.rs` | the stall run, its remote browsers and its verdict |
| `scripts/call_stall_receivers.sh` | the netem and the WebDriver servers of the receivers' machine |

---

## 3. SURFACE

**Before any proof.** The machine of the host has the toolchain of `README.md`, `trunk`, Chrome for
Testing with its chromedriver (`CHROMEDRIVER`, `BELT_CHROME`), and Docker. It starts the services
with `just services-up`. Nine senders take about one core each, so the machine has 8 cores or more.
The result names each sender short of CPU, whose figures then understate the load of the receiver.
`PROOF_RECEIVER=headless` with the variables of `call-proof-desktop` runs the whole proof against a
headless receiver, with no phone, to try the setup.

### 3.1 Android

Prerequisites: the Pixel 9a and the desktop on one network, which lets the phone reach the
desktop on TCP 8443 and on UDP. `mkcert` on the desktop.

1. Run `just call-proof-cert <address of the desktop>`, once. Install the root it names on the
   phone, in Settings, Security, Install a certificate, CA certificate.
2. Run `just call-proof-android <address of the desktop>`.
3. On the phone, open the address it prints in Chrome: it ends in `/proof/sign-in`. Set a
   passphrase of your choice, then confirm the recovery words. The list of the name it prints shows.
4. When the run says so, open Murmure, then the channel `proof` of the space `Proof`. Press Join on
   its call message, then the camera button. Keep the screen on and the page in front.
5. Wait for the verdict, about 4 minutes.

Paste back the table it prints and its `PROOF-RESULT` line.

### 3.2 iOS

Prerequisites: those of Android, with an iPhone under Safari.

1. Install the root of `mkcert` on the iPhone. Then turn on its full trust in Settings, General,
   About, Certificate Trust Settings.
2. Charge the iPhone, unplug it, set the screen brightness to half, and turn off Auto-Lock.
3. Run `just call-proof-ios <address of the desktop>`, then steps 3 and 4 of Android.
4. When the run says "decode phase", read the battery level in Settings, Battery. Read it again
   when the run says "decode phase over", 30 minutes later.
5. During the decode phase, listen to the call on the loudspeaker. At the start, then every 10
   minutes, listen for 1 minute for a click, a crackle or a cut.
6. For the echo, join the same call from the desktop in Chrome, with headphones, and speak. The
   iPhone plays the voice on its loudspeaker. Its echo cancellation is on when the desktop hears
   no echo of the voice.

Paste back the table, the `PROOF-RESULT` line, the two battery levels with their times, and what
steps 5 and 6 heard. The battery proof holds at 15 % or less.

### 3.3 The desktop paint

Prerequisites: a desktop with a GPU, a display, and the toolchain above.

1. Run `just call-proof-desktop`. A Chrome window opens, signs in, and joins the call by itself.
2. Leave the window in front until the verdict.

Paste back the table and the `PROOF-RESULT` line. Its `renderer` names the GPU the page drew
with: a software renderer such as SwiftShader voids the figure.

### 3.4 The stall

Prerequisites: two Linux machines on one network. The SFU machine has the toolchain. The receivers'
machine has Chrome and its chromedriver, Firefox and geckodriver, `tc`, and the `ifb` module.

1. On the receivers' machine, run
   `sudo scripts/call_stall_receivers.sh <address of the SFU machine>`. It prints a line.
2. On the SFU machine, run the line it printed: `LOAD_DRIVERS=... just call-stall-proof <address
   of the SFU machine>`. A Chrome or a Firefox that is not on the path of the receivers' machine
   is named by `LOAD_REMOTE_CHROME` or `LOAD_REMOTE_FIREFOX`.
3. After about 12 minutes the run prints its verdict. Stop the script with Ctrl-C: it prints what
   netem held, then removes it.

Paste back the date line, the lines from "browser" to "stall verdict", and what the script printed
on exit.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The timer wraps the shipped worker | a timer inside the call worker, in its Rust | each chunk of every user would pay for a measure that one person reads once per release |
| The timer wraps the shipped worker | a bench of the chunk seal on the phone | it times the cipher alone, without the copies and the streams the worker pays for each chunk |
| The probe posts the seal command to the Sync worker | a binding of the page that seals a file | a new door into the Sync worker in every build, for a proof |
| The senders are pages of the application | the synthetic participants of the SFU simulation | they send no chunk the receiver can open, so its call worker would open nothing |
| The stall run drives the browsers through WebDriver servers on the second machine | a second run on the second machine that reports to the first | two clocks and two processes to start in step, for four sessions one process can drive |
| netem impairs the path of the browsers | the impairment relay of the simulation, on the SFU machine | the relay shares the CPU of the SFU machine, which the proof keeps apart from the receivers |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The held share bounds the load from above.** A share over 25 % does not prove a load over 25 %.
  A profile of the phone through the remote inspector of its browser then decides.
* **The battery and the crackle have no mechanism.** Section 3.2 is the procedure.
* **No proof chooses the receiver's network.** A phone on a busy network can lose pictures the SFU
  sent. The run prints the freezes of each video, so a reader sees it.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| each budget a proof measures, and its dated figure | `ambre-spec-budgets.md` |
| which proofs a release needs, and what a failed proof reopens | `ambre-roadmap.md` |
| the call worker, the seal of a chunk and the sizes of a video | `ambre-spec-call-media.md` |
| the paint of a video on the canvas | `ambre-spec-call-media.md` |
| the load run of the SFU and its synthetic participants | `ambre-spec-call-media.md` |
| how a rule is verified, and the obstacle of a figure on a device | `ambre-spec-testing.md` |
