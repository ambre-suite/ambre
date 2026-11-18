# AMBRE -- FEATURE: THE MURMURE MEASURES ON A PHONE
**Type:** Vertical (feature)

> The pages that time the Murmure budgets in the browser of a phone, where no CI job runs. A
> person serves a page from a desktop, opens it on the phone and copies one line of JSON. The
> figures then replace the desktop figures and the ratio the budget register assumes today. A seed
> fills a channel of the phone stack for the measures that need the running application.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application. These rows hold the pages.
The budgets the pages measure stay rows of `ambre-spec-budgets.md`, and no row here verifies one.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-660` | `INV` | The phone page MUST time the segment that the desktop bench times. | this spec section 2, the corpus | structural -- the criterion bench and the phone page include one file, `benches/common/segment.rs` | none | verified |

### 1.2 Run a page

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-661` | `POST` | A page MUST print one line that names, for each figure, its row of the budget register, its median, its 95th percentile and its target. | this spec section 2, the line | `ambre_murmure::tests::phone_bench::every_figure_names_its_row_and_reads_a_time` -- wasm tier, at a small size; `ambre_crypto::tests::phone_bench::every_figure_names_its_row_and_reads_a_time` -- wasm tier, at a small size | none | verified |
| `MURMURE-662` | `FAIL` | Given a segment whose deltas do not rebuild it, the page MUST stop before it times a figure. | this spec section 2, the corpus | `ambre_murmure::tests::phone_bench::a_segment_missing_a_delta_stops_the_page` | none | verified |
| `MURMURE-663` | `POST` | The page MUST time an open that returns the plaintext sealed. | this spec section 2, the figures | `ambre_crypto::tests::phone_bench::every_figure_names_its_row_and_reads_a_time` -- the run compares the plaintext opened with the one sealed | none | verified |
| `MURMURE-664` | `POST` | The page MUST time wraps that the member who stays opens to the key wrapped and its anchor. | this spec section 2, the figures | `ambre_crypto::tests::phone_bench::every_figure_names_its_row_and_reads_a_time` -- the run opens one wrap as the member and compares the key and the anchor | none | verified |
| `MURMURE-665` | `POST` | The authorship figure MUST time a read that finds the operations of the delta read. | this spec section 2, the figures | `ambre_murmure::tests::phone_bench::every_figure_names_its_row_and_reads_a_time` -- each read counts the operations of the last delta, and none fails the run | none | verified |
| `MURMURE-666` | `FRAME` | A run of a page MUST NOT reach a relay. | this spec section 2, the two pages | crate-graph -- neither `ambre-murmure` nor `ambre-crypto` depends on `ambre-protocol` or `ambre-sync`, which hold every transport | none | verified |

### 1.3 Seed the phone stack

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-676` | `POST` | The seed MUST fill a new channel with the count of messages it is given, and the relay MUST deliver each one to another member. | this spec section 2, the seed | `ambre_belt::murmure_seed::tests::a_seed_fills_its_channel_to_the_count` -- browser tier, at 25 messages: a second member counts each one unread | none | verified |
| `MURMURE-677` | `FRAME` | A seed MUST NOT write into a channel that an earlier seed of the same account made. | this spec section 2, the seed | `ambre_belt::murmure_seed::tests::two_seeds_of_one_count_name_two_channels` -- each seed names a new channel, and opens the sidebar entry of that name alone | none | verified |

* **Budget.** The pages measure six rows of `ambre-spec-budgets.md` and hold none of them. The six
  are the segment replay, the snapshot import, the authorship check, the inbound open, the space
  leave and the outbound seal. Section 3 reads the rest of the roadmap row by hand.
* **Boundary.** none: a page holds fixed test keys and a synthetic segment, and reaches no relay. The
  seed sends through the application itself, so the relay receives sealed deltas alone.
* **Verification.** 9 `verified`. No row here verifies a budget: a person reads each figure on the
  phone into the budget register.

---

## 2. MODEL

**CRDT containers.** none. A page builds its segment in memory and keeps nothing.

* **The two pages.** Each is a test file of the crate it measures, run by `wasm-bindgen-test-runner`.
  The runner serves the page on the network, and the phone runs it on load.

| Page | File | What it times | Row of `ambre-spec-budgets.md section 1` |
|---|---|---|---|
| `murmure` | `crates/ambre-murmure/tests/phone_bench.rs` | each delta of a segment of 5 000 messages imported alone | Murmure segment, Loro replay of one delta past its snapshot |
| `murmure` | the same | the snapshot of that segment imported into an empty document | Murmure segment, Loro import of its snapshot, 5000 messages |
| `murmure` | the same | the last delta read as a Loro JSON update, its operations counted | Murmure authorship check |
| `crypto` | `crates/ambre-crypto/tests/phone_bench.rs` | one seal of 1 KB | Outbound encryption 1 KB |
| `crypto` | the same | one open of 1 KB | Inbound decryption 1 KB |
| `crypto` | the same | 51 keys wrapped for 199 members, 10 149 wraps through two wrappers | Murmure space leave |

* **The corpus.** The criterion bench `segment` and the `murmure` page include one file:
  `crates/ambre-murmure/benches/common/segment.rs`. It builds a segment of 20 writers, one delta per
  gesture. One message in five draws a reaction, and one in twenty an edit. A segment whose deltas do
  not rebuild it stops the page (`MURMURE-662`).

* **The figures.** A figure is the time of one operation, in microseconds or milliseconds.
  * The page doubles the runs of one sample until the sample lasts 20 ms. Chrome on a phone rounds
    its clock to 100 us, and Safari to 1 ms. A sample of 20 ms then errs by 5 % at most.
  * The page takes 30 samples, and 5 for the space leave. Each sample gives the mean of its runs.
  * The median and the 95th percentile are those of the 30 means, by the nearest rank.
  * `holds` compares the 95th percentile with the target of the row.

* **The space leave.** It times the wraps alone: one per remaining member for the space key and for
  each of the 50 public channel keys. The space key takes a wrapper of its own, and the channel keys
  share one, as the Sync worker does (`MURMURE-671`). Each sample derives every pair key again. The
  page leaves out the relay round trips and the reading of the membership log.

* **The line.** Each page prints `PHONE-MEASURE` and one JSON object on the page. The object carries
  the user agent, the clock step the page read, the sizes measured and one entry per figure.

* **The build.** A page builds under the `wasm-release` profile, which Trunk ships, so the code is the
  code users run. It builds without the threading flags of the product. The page needs no shared
  memory, so it runs over plain HTTP on the local network.

* **What no page reads.** Four figures of the roadmap row need the running application.

| Figure | Row of `ambre-spec-budgets.md` | Read from |
|---|---|---|
| the color emoji font, as the network served it | section 6, Color emoji font | `scripts/phone_measures.js`: the resource timing of the font |
| the local store of the origin | section 6, Local store, per client | `scripts/phone_measures.js`: the storage estimate of the browser |
| the memory of the documents held | section 6, Documents held in memory, per client | `scripts/phone_measures.js` on Chrome; the Web Inspector on Safari |
| a cold open of a channel, relay read included | section 3, Document cold start | a trace of the remote inspector, by hand |

* `scripts/phone_measures.js` reads timings and estimates, and changes nothing in the page. Pasted
  into the console of the phone stack, it prints one `PHONE-MEASURE` line.

* **The seed.** A cold open of a full segment needs a channel of 5 000 messages on the relay of the
  phone stack. The recipe `phone-seed` fills one.
  * A headless Chrome opens the stack and signs in with the development name the phone uses. A
    development name derives the same keys on every device, so the phone opens the space as its
    owner.
  * The seed creates the space `Seed` and a new channel named after the count. It types each
    message as a person types it. Enter only asks to send, and the next paint reads the draft. So
    the seed waits for two paints before the next message, or two messages would join.
  * A second member, `<name>-watcher`, joins the space and goes back from the channel. The seed
    ends once that member counts every message unread (`MURMURE-676`).
  * The seed prints one `PHONE-SEED` line of JSON: the origin, the name, the channel, the count and
    the seconds it took.

* **Code.**

| Path | Holds |
|---|---|
| `crates/ambre-murmure/benches/common/segment.rs` | the corpus, shared with the criterion bench |
| `crates/ambre-murmure/tests/phone_bench.rs` | the `murmure` page and its smoke test |
| `crates/ambre-crypto/tests/phone_bench.rs` | the `crypto` page and its smoke test |
| `scripts/phone_measures.js` | the figures of the running application |
| `justfile`, recipe `phone-measures` | the build and the server of a page |
| `crates/ambre-belt/src/murmure_seed.rs` | the seed and its test |
| `justfile`, recipe `phone-seed` | the seed of the phone stack |

---

## 3. SURFACE

* **The phone.** The owner measures on a Pixel 9a under Chrome, and on an iPhone under Safari, its
  model to name. The README calibrates every target for a mid-range phone of 2024.

* **Before any measure.** The desktop has the toolchain of `README.md`, and `wasm-bindgen-test-runner`
  at the version that `Cargo.lock` pins. The phone and the desktop share one network, and the phone
  reaches the desktop on TCP 8000. The phone is charged above 50 %, on its charger, with its battery
  saver off. Close every other tab and application.

### 3.1 The two pages

1. Run `just phone-measures murmure <address of the desktop>`. The first build takes about 10
   minutes, and the next a few seconds.
2. On the phone, open the address it prints. Keep the screen on and the page in front.
3. Wait for `test result`, about 2 minutes. Copy the `PHONE-MEASURE` line.
4. Stop the server with Ctrl-C. Run `just phone-measures crypto <address of the desktop>`, then
   steps 2 and 3 again. The space leave takes about 6 leaves, so allow 3 minutes.
5. Run each page three times, with the phone cooled down between two runs. Keep the slowest run.

### 3.2 The running application

1. Run `just phone-stack <address of the desktop>` in the dev container. Trust its authority on
   the phone, as `.devcontainer/README.md` states.
2. On the desktop, run `just phone-seed <name>` once the stack answers. The desktop needs a Chrome,
   its chromedriver and the toolchain. The seed sends about 6 messages per second.
3. On the desktop, open the remote inspector: `chrome://inspect` for Chrome on Android, the Develop
   menu of Safari for an iPhone.
4. On the phone, sign in with the name of the seed, open Murmure, and post one message that holds an
   emoji.
5. Open each channel and each document of the workload, then paste `scripts/phone_measures.js` into
   the console. Copy the `PHONE-MEASURE` line.
6. On Safari, read the memory in the Timelines tab of the Web Inspector instead.
7. For the cold open, close the tab and record a trace in the inspector. Open the channel of the
   `PHONE-SEED` line from the sidebar. Read the time from the click to the first frame that shows
   the messages.

* The memory of the page never shrinks, so the figure bounds the documents held from above. Read it
  once after the boot and once with the workload open: the difference is the figure.

### 3.3 Writing the figures down

* Each figure goes into the `Method` cell of its row in `ambre-spec-budgets.md`. The cell names the
  date, the phone, its browser and its version, and the command. The desktop figure stays beside it,
  with its own date. The ratio of 7 that the authorship row assumes then goes.

| Figure of the line | Row |
|---|---|
| `Murmure segment, Loro replay of one delta past its snapshot` | section 1, the same name; then section 6, Murmure active segment, whose 250 deltas it prices |
| `Murmure segment, Loro import of its snapshot, 5000 messages` | section 1, the same name |
| `Murmure authorship check, one received delta read as a Loro JSON update` | section 1, the same name |
| `Outbound encryption 1 KB` and `Inbound decryption 1 KB` | section 1, the same names |
| `Murmure space leave` | section 1, the same name |
| `emoji_font` | section 6, Color emoji font |
| `local_store_bytes` and `memory` | section 6, the two rows of residency |
| the cold open of the trace | section 3, Document cold start |

* A figure past its target is a finding, not a failure of the measure. Write it down with its date,
  then bring it to the owner of the row: the space leave row names what follows a breach.

### 3.4 The call proofs

* The three call rows of the roadmap have their own harness, in `ambre-spec-call-proofs.md section
  3`. Their figures go into the call rows of `ambre-spec-budgets.md section 1`, the stall included,
  by the same rule. The stall names both machines.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A page is a test file of the crate it measures | a crate of its own, built by Trunk | a crate more in the graph and in the primitive gate, for a page one person opens once per release |
| A page is a test file of the crate it measures | a page inside the application, behind a flag | every user would download the corpus and the timers |
| The `murmure` page and the bench include one corpus | a corpus written again for the page | two corpora drift, and a phone figure then answers another question than the desktop one |
| The space leave times the wraps | the whole rotation through the Sync worker and a relay | the phone would then time the network of the room, not the phone |
| The page builds under `wasm-release` | the test profile | the figures of a build users never run price nothing |
| The seed drives the application in Chrome | a host client that seals and posts the deltas | a second copy of the sealing, the membership and the delta order of the Sync worker, which drifts from the copy the phone runs |
| The seed drives the application in Chrome | rows written into the database of the relay | the relay holds no key, so rows it did not receive open on no client |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A seed of 5 000 messages loses messages past the first 4 000.** On 2026-11-17, in a cloud
  container, `a_seed_fills_its_channel_to_the_count` held 25, 300, 1 000 and 3 000 messages. A run of
  5 000 delivered 4 738 to the relay, and the watcher counted 4 501. The cause is not yet known.
* **No measure marks the cold open in the application.** Section 3.2 reads it from a trace by hand.
* **The memory of the documents held is an upper bound.** The memory of a page never shrinks.
* **The 95th percentile is that of means.** A page times samples of many runs, since the clock of a
  phone cannot time one wrap.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| each budget a page measures, and its dated figure | `ambre-spec-budgets.md` |
| the call proofs on a phone, a desktop and two machines | `ambre-spec-call-proofs.md` |
| which measures a release needs | `ambre-roadmap.md` |
| how a rule is verified, and the obstacle of a figure on a device | `ambre-spec-testing.md` |
| the phone stack and its certificates | `ambre-spec-testing.md section 4` |
