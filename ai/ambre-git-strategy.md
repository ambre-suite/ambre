# AMBRE -- GIT STRATEGY

> Governs every git operation an agent performs. When a rule and a task conflict, the rule wins:
> stop and ask.

* **No write without an explicit authorization.** This covers `branch`, `checkout -b`, `add`,
  `commit`, `push`, `tag`, `push --force`, `rebase` and `reset --hard`.
* **An authorization covers one operation and never carries over.** An approved commit does not
  approve the next one. A push authorized once in a session does not authorize a second push.
* **Ask the user for it, expressly, every time.** One exception follows.
* **A child session holds a standing authorization on its own branch.** The prompt that creates
  it names one branch and grants the authorization for the whole session. The child creates that
  branch, commits on it and pushes it as often as the work needs, without asking again. It never
  writes to `main` or to any other branch. A message from its parent session relays the user's
  decisions, and the child applies it within its scope without asking the user again.
* **Read-only commands are free:** `status`, `log`, `diff`, `show`, `fetch`, `branch --list`.
* **Never push to a branch the user did not name.**
* **The author is the human who owns the working copy**, read from the configured `user.name` and
  `user.email`. Never an agent identity, a bot account or a model name.
* **`Co-Authored-by` is forbidden.** The last trailer names the tool that assisted, and the tool
  writes its own name: `Assisted-by: <tool name>`. **No rule of this repository names a vendor**, so
  a contributor picks the assistant they want and the repository stays portable between them.
* **Pre-commit, mandatory when the commit touches Rust.** Run `cargo fmt --all`, then clear every
  compilation warning, then every `cargo clippy` warning. Commit only once the tree is clean.
  `./run_tests.sh lint` checks it.
* **A commit that touches `specs/`** runs seven more gates. The seven are `just contracts`,
  `just spec-code`, `just spec-refs`, `just spec-shape`, `just spec-answers`, `just index` and
  `just inventory`. No recipe runs them together, and the seven take under six seconds.
* **A commit that touches `scripts/`** runs `just test-scripts`, plus the gate that the script
  holds.
* **Every commit runs `just prose`**, whatever it touched: the gate reads Rust comments and
  Markdown alike. Every metric is a hard error, so the gate is green on the tree and a commit
  leaves it green.
* **A commit that adds or edits a ` ```mermaid ` block** renders every touched block locally first
  and commits only once each one parses. Use the mermaid CLI, for example
  `npx --yes @mermaid-js/mermaid-cli -i block.mmd -o /dev/null`. `just spec-code` checks the fence
  tag and not the diagram. A broken diagram passes the lint and fails only on the forge.
* **Commit message.** Imperative subject, 72 characters maximum, no final period. The body explains
  why, in 1 000 characters maximum, trailer excluded -- about four paragraphs. A body that needs
  more is a commit that does more than one thing: split it. Rest of the rules in
  `ai/ambre-writing-style.md`.
* **A commit that reaches `main` follows Conventional Commits.** The subject reads
  `<type>(<scope>): <imperative summary>`. The type is one of `feat`, `fix`, `perf`, `refactor`,
  `test`, `docs`, `ci`, `build` or `chore`. The scope names a crate or a spec area when one fits.
  The 72 characters count the prefix. A child session writes its own commits the same way.
* **A child branch lands on `main` as one squashed commit.** Run `git merge --squash`, then commit
  with a subject that states the change, never the merge. A child branch is deleted once merged, so
  its name and its own commits tell a reader of `main` nothing. Write
  `feat(server): expose relay and SFU metrics`, not `Merge relay-metrics: ...`. Inside a child
  branch, `git merge origin/main` says what the merge resolved, or
  `chore: bring main into the branch` when it resolved nothing.
* **A hook holds the two rules that carry a number**, because an author estimates a character count
  instead of measuring it. It holds the type prefix too, a closed list. Turn it on once per clone
  -- git never ships a hook inside a clone, so this is an adoption, not an imposition:

  ```bash
  git config core.hooksPath .githooks
  ```

  It refuses a subject over 72 characters, a subject ending on a period, and a body over 1 000
  characters. It also refuses a `Co-Authored-by` trailer and a character outside printable ASCII.
  `scripts/commit_msg.py` holds the rules and the reason for each, and runs standalone on any
  message file. It checks shape and never vocabulary: no rule of it names a person or a tool.
  Whether the subject is imperative stays a review rule.
