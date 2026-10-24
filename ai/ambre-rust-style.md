# AMBRE -- RUST CODE RULES

> Applies to every line of Rust an agent writes or modifies.

* **Idiomatic.** Prefer iterators and combinators over manual loops.
* **Data-oriented design, no Java-like OOP.** A closed `enum` plus a `match` beats a trait object:
  the compiler proves the dispatch is exhaustive.
* **A newtype for every domain value, inside a function too.** An identifier, a quantity with a
  unit, an index or offset, a secret and a validated value each get their own type. Examples:
  `MessageId(Uuid)`, `Timestamp(u64)`, `Row(u32)`, `Emoji(String)`. A closed set of values is
  an `enum`, never a `String`. A `bool` that selects a mode is an `enum` too. A bare `String`,
  `Uuid`, integer, float or `Vec<u8>` is allowed in four places only:
  * free text with no rule;
  * the inside of the newtype that wraps it;
  * the call into a foreign API that takes the primitive;
  * a local value, which stays inside one function or one private struct.

  The type is unwrapped at that call and nowhere else. The compiler then refuses a message id
  passed as a document id, or a row passed as a column (`ambre-spec-testing.md section 2`).

  A new concept gets its newtype in the change that introduces it, never later. If no type
  exists, the change adds one.
* **A local value keeps its primitive only when four conditions hold.** The allowlist names it
  `local`, and its note names the reason.
  1. The value crosses no thread, crate, wire, store or public API.
  2. No other value of the same type in its scope means something else.
  3. The value is not a closed set, a validated value, a unit quantity, an id, a key, a secret
     or a version.
  4. Its name, or the single call that reads it, states its meaning.

  When one condition fails, the value takes a type. When you are not sure, give it a type.
* **A newtype is complete, so its callers never unwrap it.**
  * **Validation.** A value with a rule has a fallible constructor, and no other way in.
    Deserialisation goes through that constructor.
  * **Arithmetic.** It is limited to the operations the domain allows. A position, a count and a
    difference are three types: `Row + RowCount -> Row` and `Row - Row -> RowDelta`, while
    `Row + Row` does not compile.
  * **Iteration.** `Range<Row>` cannot iterate, because `core::iter::Step` is unstable
    (tracking issue rust-lang/rust#73121). So an index type ships a span type that implements
    `IntoIterator`. When `Step` reaches stable, implement it for every index type. Then replace
    each span type with `Range` and delete it. The macro that generates the index types carries
    the same note.
  * **Indexing.** A collection indexed by an index type is a typed collection.
  * **Derives.** Each type derives what the code needs: `Ord`, `Hash` and `Default` where they
    hold. Its serde form is `#[serde(transparent)]` unless the wire says otherwise. A secret or
    plaintext type prints only its length through `Debug`.
  * **Two bans.** Never implement `Deref` to the primitive, and never implement `From` in both
    directions. Either one turns the newtype back into the primitive silently.
  * **Generation.** A family of similar types comes from one macro, as `define_encoded!` does.
    Hand-written copies drift.
* **The primitive gate holds the rule.** `just primitives` finds every site outside a test whose
  type holds a primitive. Each site is typed, or `specs/1-Foundations/ambre-primitives.tsv` names
  it with a category and a reason (`ambre-spec-testing.md section 2`). A reviewer reads every new
  line of that file by hand (`ARCH-035`). The gate checks the category word, and never whether it
  is true.

  The categories are a closed set, and no category means "to type later". A value still to type
  has no line, so the gate fails until the change types it.
* **Minimal allocations.** Borrow before you clone. Never allocate on a hot path.
* **No `unsafe` in our code.** A third-party crate with C or C++ bindings is allowed, but native
  Rust is preferred. The workspace lint table forbids `unsafe_code`, and every crate inherits it.
* **No panic.** `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` and
  out-of-bounds indexing are forbidden outside tests. Return `Result` or `Option` instead. The
  client is WASM: a panic kills the worker, so the user loses the editing session, not one
  operation.
* **Compilation-ready.** Every `use` statement included. No `todo!()`, no stub left behind.
* **A new dependency needs a written justification**, in the spec that pulls it in. Pure Rust wins
  over a C binding, and the standard library wins over a crate. `just deny` gates licences and
  advisories.
* **Commented.** Every non-trivial item carries one, written per `ai/ambre-writing-style.md`.
