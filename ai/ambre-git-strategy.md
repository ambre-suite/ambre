# AMBRE -- GIT STRATEGY

> Governs every git operation an agent performs. When a rule and a task conflict, the rule wins:
> stop and ask.

* **No write without an explicit authorization.** This covers `branch`, `checkout -b`, `add`,
  `commit`, `push`, `tag`, `push --force`, `rebase` and `reset --hard`.
* **An authorization covers one operation and never carries over.** An approved commit does not
  approve the next one. A push authorized once in a session does not authorize a second push.
* **Ask the user for it, expressly, every time. No exception.**
* **Read-only commands are free:** `status`, `log`, `diff`, `show`, `fetch`, `branch --list`.
* **Never push to a branch the user did not name.**
* **The author is the human who owns the working copy**, read from the configured `user.name` and
  `user.email`. Never an agent identity, a bot account or a model name.
* **`Co-Authored-by` is forbidden.** The last trailer names the agent instead:
  `Assisted-by: Claude Code`.
* **Pre-commit, mandatory when the commit touches Rust.** Run `cargo fmt --all`, then clear every
  compilation warning, then every `cargo clippy` warning. Commit only once the tree is clean.
  `just lint` checks it.
* **A commit that touches only `specs/`** runs `just invariants` and `just spec-code` instead.
* **Commit message.** Imperative subject, 72 characters maximum, no final period. The body explains
  why. Rest of the rules in `ai/ambre-writing-style.md`.
