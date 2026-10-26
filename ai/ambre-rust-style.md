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

  These cases take a type, and none of them is `local`, `generic` or `math`:
  * A pixel length, a width, a height or a font size that crosses a function boundary is a unit
    quantity.
  * A byte count or a byte size is a unit quantity, `ByteSize`. Opaque bytes are never `math`:
    a blob is a newtype, or `foreign-api` at the foreign call.
  * The length of a constant array stays a `usize`. It becomes a `ByteSize` when another crate
    frames bytes with it at run time.
  * A value with a parse into a type carries the parsed type, never the text.
  * A parse that needs a runtime context still types the parts that parse without it. Only the
    part that needs the context stays text, and the note of its line names that context.
  * A total parse whose result type is private to a downstream crate is still a parse. The change
    moves or exposes the type, and never keeps the text.
  * Two or more values of one type in a tuple that crosses a function boundary are swap-prone.
    The tuple becomes a struct with a named field for each value.
  * A `[T; 2]` array that crosses a function boundary is swap-prone like a tuple. So are two
    parameters of one type with different meanings. Each takes a struct, or a type per meaning.

  These cases keep the primitive, each under one category:
  * A scalar multiplier is `math`, on a public API too: a factor, a ratio or a zoom scale. A
    length, a position, a count, an index or a size is never a multiplier.
  * A formula number is `math` inside the formula engine. It is a `CellNumber` once it crosses a
    public struct or the bus.
  * A closed set stays an `enum` on every path up to its display label. The site that turns the
    `enum` into text for display or for the wire is `free-text`.
  * A SQL query, a Lua script and a whole markup document are `free-text`.
  * An error that echoes the refused raw value is `generic`.
  * A vertex layout that the GPU dictates is `foreign-api`. So is a constant that one foreign call
    alone reads.

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

  A `local` line names a private site. A site is not private when one of these holds:
  1. its item, field or `fn` is `pub`, `pub(crate)`, `pub(super)` or `pub(in path)`;
  2. it is a field of a type of `ambre-bus`;
  3. it is a field of a type that derives `Serialize`, `Deserialize` or `FromRow`.

  A field of an enum variant is as private as its `enum`. A method of a trait impl is never
  private: the trait reaches it wherever the type goes.
* **A `bool` that a caller writes as a literal is an `enum`.** At `f(true)` the reader sees `true`,
  never what it picks. Such a parameter becomes an `enum` of two variants, each named for what it
  picks. Tests are callers too: a literal in a test makes the parameter an `enum`. This holds
  whatever the category of its line, with one exception. A `trait` line keeps its `bool`, because
  the foreign signature fixes the type.
  * A predicate returns a `bool`: `is_empty()` reads as a question, and its answer stays a `bool`.
  * A `bool` that crosses a thread, crate, wire or store and names a mode or a state is an `enum`
    too. The variant that was `false` comes first, so a postcard frame reads the same.
  * A `bool` field that is not private is an `enum`, unless a foreign schema fixes it. A wrapped
    `bool` counts too: `Option<bool>`, `Cell<bool>`, `RwSignal<bool>`. A `bool` in the signature
    of a closure or a function pointer is no value the field holds: `Box<dyn Fn(bool)>` is a
    callable.
  * The one field of a tuple variant with exactly one field stays a `bool`. The variant names the
    value, as `CellValue::Boolean(bool)` does, so the `bool` selects no mode. A variant with two
    fields, a named field or an `Option<bool>` does not qualify.
  * A store column or a JSON value that records a fact maps to that `enum`. A serde or sqlx
    mapping keeps its stored bytes and its wire bytes. `ambre_core::define_flag!` writes such an
    `enum`: its serde form is a plain `bool`, and its `@sqlx` arm maps it to `BOOLEAN`.
  * A struct that mirrors a schema the workspace does not control keeps its `bool` fields. Its
    lines are `foreign-api`, and the boundary turns each value into the workspace `enum`. OIDC
    claims such as `email_verified` and the response of a third-party API are such schemas.
  * A private `bool` field stays a `bool` under `local` when two conditions hold. Only the impl
    of its own struct writes it, and the four `local` conditions hold.
  * A `bool` parameter or return at a store call or a JSON call may stay a `bool` under
    `generic`. This holds while no caller writes it as a literal. One literal at any caller makes
    it an `enum` (`ARCH-040`).
  * Any other `bool` is a fact that selects no mode. Its allowlist line is `generic`, never
    `math`: a `bool` is not arithmetic.
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
    hold. Its serde form is `#[serde(transparent)]` unless the wire says otherwise. A plaintext type
    prints only its length through `Debug`. A secret type prints nothing of its value: it holds
    an `ambre_core::Secret` and derives `Debug`, which prints `Secret([REDACTED])`.
  * **Secrets.** A key, a seed, a passphrase or a token is a domain newtype over
    `ambre_core::Secret`, which wipes the value on drop. `Secret` has no `Clone`, no serde, no
    `PartialEq` and no `Display`, so a derive of one on the newtype does not compile. A value two
    owners share is an `Arc<Secret<T>>`; `clone_secret` is the one copy. Equality is
    `ConstantTimeEq`. The value is read through `expose` in `ambre-crypto` alone: a crate outside
    it calls a typed method there (`ambre-spec-zero-knowledge.md section 2`).
  * **Two bans.** Never implement `From` in both directions, and never implement `Deref` or
    `DerefMut` to a primitive, or to a type that hands one out. Either one turns the newtype back
    into the primitive silently. A slice, an array, a reference, and a `Vec`, `Box`, `Rc`, `Arc`
    or `Cow` hand out their element. A generic `Target = T` counts, bound or not: the gate cannot
    see which instance makes `T` a primitive, so it fails closed. The gate refuses such a `Deref`
    whatever the allowlist says.
  * **Generation.** A family of similar types comes from one macro, as `define_encoded!` does.
    Hand-written copies drift.
* **The primitive gate holds the rule.** `just primitives` finds every site outside a test whose
  type holds a primitive. Each site is typed, or `specs/1-Foundations/ambre-primitives.tsv` names
  it with a category and a reason (`ambre-spec-testing.md section 2`). A reviewer reads every new
  line of that file by hand (`ARCH-035`).

  The gate reads a `type` alias a module of the workspace declares as its target. Such an alias
  hides no primitive and no `bool` from a check. The gate reads aliases and `use` items by module
  only. So a site whose path starts with an alias or a rename that a block declares fails the scan.
  So does a site whose path starts with a block import of an alias, a rename or a primitive. An
  alias of a foreign crate, and a site a macro writes, are a reviewer's to read (`ARCH-043`,
  `ARCH-044`).

  The gate checks the category word. It also refutes a `newtype-inner` or a `trait` line whose
  site lacks the shape stated below (`ARCH-036`). It refuses six more defects, whatever the line
  says:
  1. a `Deref` or a `DerefMut` to a primitive (`ARCH-037`);
  2. a `local` line on a site that is not private (`ARCH-038`);
  3. a `bool` field that is not private, unless its line is `foreign-api` (`ARCH-039`);
  4. a `bool` parameter that is not private and that a caller writes as a literal, unless its
     line is `trait` (`ARCH-040`);
  5. `expose`, `expose_mut` or `clone_secret` named outside `ambre-crypto` and the Secret module
     (`ARCH-045`);
  6. a name of the `zeroize` crate outside the Secret module (`ARCH-046`).

  The categories are a closed set, and no category means "to type later". A value still to type
  has no line, so the gate fails until the change types it.
  * A `foreign-api` line names the site of the foreign call, never an accessor that wraps it.
  * A `newtype-inner` line names a site in the newtype's own impl. The newtype is a struct with
    exactly one field, or a type that a workspace `define_*!` macro writes. A `PhantomData`
    marker is not a field in that count. The orphan rule may put the impl of a workspace trait
    for the newtype in the crate of the trait. That impl is the newtype's own too. The site is one
    of three:
    1. that field;
    2. a parameter of an associated function of the newtype that returns `Self`, `Option<Self>`
       or `Result<Self, _>`;
    3. the return of a method of the newtype that takes `self` or `&self`.

    An `enum`, a constant and a struct of two fields are never `newtype-inner`.
  * A `trait` line names a signature that the workspace cannot change. A trait the workspace does
    not own fixes it, or a generic bound the workspace cannot change. A closure or function
    pointer type that a foreign signature requires is `trait` too. A trait the workspace declares
    is ordinary code, and its signatures take types. The workspace chooses a primitive that the
    impl writes as a generic argument of the trait, or as the type it implements for. So
    `impl Add<u32> for Px` and `impl Add<Px> for u32` both take a type at the site.
  * The note of a line describes the value truthfully. A reviewer reads the note as code.
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
