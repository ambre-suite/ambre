# AMBRE -- CONTRIBUTING

> Which channel is open to whom, and the two checklists no machine performs.

Read [`README.md`](README.md) first. It states what Ambre is, and the pillars a change must hold.

---

## 1. WHAT IS OPEN

| Channel | Who may open one | State |
| :--- | :--- | :--- |
| An issue that reports a reproduced bug | anyone | open |
| An issue that asks about a specification | anyone | open |
| A pull request | a repository collaborator | closed to everyone else |

The repository settings restrict pull request creation to collaborators. A fork stays available, and
the AGPL grants every right the licence names. This repository closes the pull request step, and
nothing else.

**Why the project closes it.** An open pull request commits a maintainer to a careful review. Ambre
carries a normative specification corpus, and a change that ignores the corpus costs more to review
than to write again. One maintainer cannot hold that standard on an open channel. The project
states the limit before the channel exists, rather than leaving an author to discover the limit in
a closed pull request.

**The restriction is revisable.** It holds while the corpus changes faster than an outside reader
can follow. Ask in an issue before you start work on a story.

---

## 2. OPENING AN ISSUE

The maintainer reads every issue. State:

1. the commit you ran;
2. the steps that reproduce the failure;
3. what you expected, and what happened;
4. the browser, the operating system and the render backend, for a rendering failure. The backend is
   WebGL2 by default, and WebGPU under the `?webgpu` flag.

**An automated audit is not a bug report.** The maintainer closes, without discussion, an issue
that lists the findings of a scanner or of an assistant and carries no reproduction. The project
runs its own audits, and [`specs/audits/security-audit.md`](specs/audits/security-audit.md) holds the findings.

A problem you found yourself, and can demonstrate, is a bug report. The tool that helped you find it
does not matter.

---

## 3. THE AUTHOR CHECKLIST

CI holds every rule a machine can hold. `./run_tests.sh` runs the same recipes locally, and the
`justfile` holds the exact command lines. A gate goes red on its own, so no checklist repeats one.

Enable the commit-message hook once per clone. git ships no hook inside a clone, so this step is an
adoption, not an imposition:

```bash
git config core.hooksPath .githooks
```

Install the merge driver of the generated spec files once per clone, for the same reason:

```bash
git config merge.ambre-generated.driver "python3 scripts/merge_generated.py %O %A %B %P"
```

`.gitattributes` names the contract ledger, the contract index, the contract graph and the
inventories. On a merge or a rebase, the driver writes each one from the merged sources. Two
branches that each add a row then conflict on no generated file. Without it, git merges them as text:
the merge conflicts, or it writes counts that no generator writes. A lone cherry-pick names no
other commit, so the driver keeps your side and prints the three commands that regenerate the files.
`scripts/merge_generated.py` states each case.

`just primitives` builds the rustc driver of the gate on the pinned nightly, which needs one
component more than `rust-toolchain.toml` installs. Add it once, from the root of the clone, so
rustup adds it to the pinned toolchain:

```bash
rustup component add rustc-dev
```

The gate runs its compiles without a `RUSTC_WRAPPER`, since a cache such as sccache would answer
for the driver. A unit the driver did not read fails the gate.

The table holds what no gate sees. Read it before you commit.

| Check | Why no gate holds it |
| :--- | :--- |
| Every committed fixture comes from a generator run from a seed, and never from a real document (`FIXTURE-005`, `FIXTURE-032`) | a real document and a generated one are both Loro bytes, so no check tells them apart. A real document places plaintext user content in a public repository, permanently |
| A commit that refreshes a golden artifact moves `generator_version` in the same commit | the gate compares the corpus against the code, never against the history of the corpus, so a regenerate-and-commit always passes |
| You read a snapshot diff before you accept it | `cargo insta accept` accepts blindly |
| A `fn` your change edits carries no line of `scripts/spec_code_citations.allow`: fix that defect in the same commit and delete its line. A line on a `fn` you did not edit stays | the gate refuses a new defect and a stale line, never an old one, so a listed defect lives until someone edits its `fn`. The scope is the `fn`, not the file, so a three-line change never carries a file-wide cleanup |
| Every mermaid block you touched renders locally first | `just spec-code` checks the fence tag, not the diagram. A broken diagram fails on the forge |
| A benchmark figure names what it measures, and never a frame rate (`FIXTURE-024`) | the claim is made in a pull request, not in a test |
| A contract row that binds what each application does labels its tests `Prose`, `Trame` or `Toile` (`ambre-spec-template.md`) | a row with no label reads as the shared code's own rule, and nothing tells the two apart |

**A commit that touches `specs/` has a second such list.** `ambre-spec-template.md section 1.G`
names, beside each pillar, the script that holds it and what it leaves to a reader. Three examples:
whether a kind is the right one, whether a frame names the right state, whether a gap says what is
loose. It is not copied here: a copied table goes stale.

---

## 4. THE REVIEW CHECKLIST

`ambre-spec-testing.md section 1` states what a reviewer reads: public signatures, property names,
mutant reports, snapshot diffs, the `Mechanism` column. Function bodies are not on the list.

A contract row whose `Mechanism` names `human-review` leans on the reviewer: alone, by decision, or
until the Story it owes brings a mechanism. Each row lives in the spec of the component it
constrains, and the table below lists them.

<!-- human-review rows: generated by `just contracts-write`, never edit -->
| Row | What the reviewer holds | Spec |
| :--- | :--- | :--- |
| `ARCH-035` | Each line of the primitive allowlist MUST carry the category true of its site, and one site has exactly one. A `local` line MUST meet the four conditions of `ai/ambre-rust-style.md`. A site that spells an enum, or a struct that is no newtype, as a primitive MUST take its category from the reader of that spelling. A reader that decodes it back sets `newtype-inner` for a fieldless enum and `generic` for an enum that carries data or a struct that is no newtype. A person sets `free-text`. A trait method that is a type's own spelling MUST take the spelling's category, not `trait`. An error that echoes a refused value, or the count it gives of that value, MUST be `generic`, whatever shape its site has. | `ambre-spec-architecture.md` |
| `ARCH-041` | A `bool` parameter that is not private MUST NOT take a literal through a call that does not name its `fn`: through a value, a closure or a trait object, or in code a macro writes. Nor MAY it take one in a shape the gate does not read as a literal: a block of more than one statement, the argument of a call other than `Some`, a field of a struct literal, a cast, or a `view!` prop value of more than three tokens. | `ambre-spec-architecture.md` |
| `ARCH-042` | A `trait` line on a closure type or a function pointer type MUST name one that a foreign signature requires. | `ambre-spec-architecture.md` |
| `ARCH-047` | The value of a `Secret` MUST NOT leave `ambre-crypto`, the Secret module and the relay's secrets module through a door other than the three names of `ARCH-045` and the doors this row admits: a function that lends out the bare value to any caller, a trait impl that hands it out, code a macro or a derive writes, or code the gate does not read. The admitted doors are: `impl<K: WireConfidential> Serialize for Confidential<K>` with its `Deserialize` twin, which the markers `Passphrase`, `RecoveryWords` and `Plaintext` each admit by one `impl WireConfidential` line, a sealed trait; `impl<K: WireCredential> Serialize for Credential<K>` with its `Deserialize` twin, which the markers `AccessToken`, `RefreshToken`, `PresignedUrl`, `OidcCode` and `OidcState` each admit by one `impl WireCredential` line, a type that holds one deriving its serde with no gated name; the sinks of `ARCH-049`; `MasterSeed::recovery_words`; `Credential<PresignedUrl>::object_address`, the address before the query; `Confidential<Plaintext>::with_plaintext`, which lends the bytes with no copy to the code that reads them on the way to another form, while the wrapper keeps and wipes them; `Confidential<Plaintext>::into_plaintext`, which moves the bytes out with no copy into a role that keeps them in the clear by design, and leaves the wrapper empty, while a wrapper another owner shares hands out a copy, and that owner keeps and wipes the original; and each function of the relay's secrets module, which hands its copy to one foreign call and is a sink of `ARCH-049` (`ARCH-048`). | `ambre-spec-architecture.md` |
| `ARCH-063` | Each line of the newtype exceptions file MUST give a reason true of its newtype, which names why no generic family of `ambre-core` fits it. | `ambre-spec-architecture.md` |
| `ARCH-067` | A module-level `type` alias whose target holds an instance of a generic family of `ambre-core` in a tuple, a container or a closure MUST stay only where that target, written out at one of its uses, fails `clippy::type_complexity`. Its doc comment MUST say so in one line. | `ambre-spec-architecture.md` |
| `ARCH-074` | A field, parameter, return, constant, static or alias that a macro of a foreign crate writes from a type the source hands it MUST obey `ARCH-033`, `ARCH-039` and `ARCH-040` as one the source writes does. | `ambre-spec-architecture.md` |
| `ARCH-075` | A build of the workspace outside the compiles of the lint MUST read no file outside a test that those compiles do not read. | `ambre-spec-architecture.md` |
<!-- end of the human-review rows -->

The table is written from the rows by `just contracts-write`, and `just contracts` fails when it is
stale. A new `human-review` row needs no edit here.

---

## 5. WHERE THE RULES LIVE

| File | What it holds |
| :--- | :--- |
| [`README.md`](README.md) | what Ambre is: the pillars, the crate map, the map of the spec corpus |
| [`AGENTS.md`](AGENTS.md) | the reading order an assistant follows, and the file to open for each task |
| [`ai/ambre-git-strategy.md`](ai/ambre-git-strategy.md) | the author of a commit, the forbidden trailer, the pre-commit runs, the shape of a message |
| [`ai/ambre-rust-style.md`](ai/ambre-rust-style.md) | the Rust rules a change follows |
| [`ai/ambre-writing-style.md`](ai/ambre-writing-style.md) | the English every description uses, a commit message included |
| [`specs/ambre-spec-template.md`](specs/ambre-spec-template.md) | the contract every spec follows, and the grammar of a contract row |
| [`specs/1-Foundations/ambre-spec-testing.md`](specs/1-Foundations/ambre-spec-testing.md) | how Ambre is verified, and the only place the method lives |
| [`specs/ambre-roadmap.md`](specs/ambre-roadmap.md) | the only place anything is scheduled, one row per story |
