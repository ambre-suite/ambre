# AMBRE -- RUST CODE RULES

> Applies to every line of Rust an agent writes or modifies.

* **Idiomatic.** Prefer iterators and combinators over manual loops.
* **Data-oriented design, no Java-like OOP.** A closed `enum` plus a `match` beats a trait object:
  the compiler proves the dispatch is exhaustive.
* **A newtype for every domain value.** An identifier, a quantity with a unit, a secret and a
  validated value each get their own type, for example `MessageId(Uuid)`, `Timestamp(u64)` or
  `Emoji(String)`. A closed set of values is an `enum`, never a `String`. A bare `String`, `Uuid`,
  integer or `Vec<u8>` is allowed for free text with no rule, and inside the newtype that wraps it.
  The compiler then refuses a message id passed as a document id (`ambre-spec-testing.md section 2`).
* **Minimal allocations.** Borrow before you clone. Never allocate on a hot path.
* **No `unsafe` in our code.** A third-party crate with C or C++ bindings is allowed, but native
  Rust is preferred. A crate that carries no `unsafe` declares `#![forbid(unsafe_code)]`.
* **No panic.** `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` and
  out-of-bounds indexing are forbidden outside tests. Return `Result` or `Option` instead. The
  client is WASM: a panic kills the worker, so the user loses the editing session, not one
  operation.
* **Compilation-ready.** Every `use` statement included. No `todo!()`, no stub left behind.
* **A new dependency needs a written justification**, in the spec that pulls it in. Pure Rust wins
  over a C binding, and the standard library wins over a crate. `just deny` gates licences and
  advisories.
* **Commented.** Every non-trivial item carries one, written per `ai/ambre-writing-style.md`.
