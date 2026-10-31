# AMBRE -- BUILDING A STORY

> The order of the moves between `specs/ambre-roadmap.md` and the specs it points into. Read it
> before the first line. Nothing here is normative: every fact lives in the file it names.

## Before the first line

* **Start at the name.** `just spec-context-story "<NAME>"` prints the closure of every row the
  story lands.
* **Read what it prints, and nothing else.** A document the closure leaves out is a document no row
  of this story cites.
* **Read the documents whole when the closure costs more.** The measure on standard error names
  them.

## A story that lands no row

* **The empty output is an answer.** The corpus carries no rule for that work yet.
* **Write the rows first**, `unimplemented`, under `specs/ambre-spec-template.md` section 1, each
  landing with that story.
* **Code answers a row, never a `Touches` cell.**
* **A row you cannot write without a measure or a trial** waits for a spike, as the next section
  states.

## A spike before the rows

* **A spike is throwaway code that answers one question.** Paper cannot answer that question.
* **Two kinds of question earn a spike:** how a feature behaves under the hand, and whether a
  design holds a budget.
* **A spike never replaces a row on security or convergence.** Zero-knowledge, sealing, key
  rotation and CRDT convergence take rows first.
* **Write the question and the time box before the first line.** A spike without both is a story
  built without rows.
* **The spike code lives on a branch that never merges.** Nothing is copied out of it. The story
  starts again from the default branch.
* **A logic spike drives a state machine through the hard cases.** A scratch binary or an
  exploratory proptest is enough.
* **The spike stops when the question has an answer**, even when the code almost works. At the end
  of the time box, the spike stops and the question goes back to the user.
* **The output lands in the corpus, never in the code:**
  1. the rows, `unimplemented`, bound to the story;
  2. each measure, against its row of `specs/1-Foundations/ambre-spec-budgets.md`, with its date,
     its device and its command;
  3. each option the spike refused, in the `REJECTED ALTERNATIVES` section of the spec.

## Building, and finishing

* **A row you cannot build as written is a defect of the row.** Fix the spec first. A rule found on
  the way is one more row, written the same way.
* **A row that ships moves three cells.** `Mechanism` names the mechanism, `Gap` reads `none`,
  and `Status` becomes `verified`. The ladder of mechanisms is `ai/ambre-testing.md`.
* **A row whose mechanism does not exist becomes `untested`**, and it keeps its Story.
* **A row whose mechanism holds part of the rule is split in two.** The half held is a new row,
  `verified`. The half left keeps its Story, or its obstacle, under its own status.
* **Regenerate the three artefacts:** `just index-write`, `just contracts-write`, `just
  inventory-write`. Each one re-reads the corpus, so the order is free. The counts of each
  `Verification` answer and the `human-review` table of `CONTRIBUTING.md` are among them:
  never recount them by hand.
* **Ticking the box is the whole of it**, as `specs/ambre-roadmap.md` section 1 states.
