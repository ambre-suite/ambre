# AMBRE -- SPEC: NEWTYPES

**Type:** Horizontal (Foundation -- Types)
**Implementation:** implemented

> **Which generic of `ambre-core` a newtype is, and why the few that are none stay hand-written.**
> A newtype over an integer, a float, a `String` or a `Vec<u8>` is an instance of a family below.
> If none fits, it is a line of `ambre-newtypes.tsv` with its reason. The rule is `ARCH-062`,
> which the primitive gate holds, and the truth of a reason is `ARCH-063`, which a reviewer holds.

---

## 1. THE RULE

* **One family per need.** Two hand-written newtypes over one primitive grow two sets of methods,
  and the second one drifts. A family writes the methods once, and its marker names the quantity.
* **A marker opts in to a capability.** A family gives every marker the door and the reading. A
  capability that some markers need and others must not have is a trait the marker implements.
* **An exception carries its reason.** The reason is one line of the exceptions file, and a
  reviewer reads it. It names why no family fits, in one of the shapes below or another one.

| Reason | Example |
| :--- | :--- |
| An invariant no family checks | `LineSpacingMultiple` lies above zero; `FontSize` reads a value past its cap as the cap |
| An operation no sibling shares | `Rgba` reads its channels; `StackDepth` refuses a pop below zero |
| A width or a meaning a foreign definition fixes | `PeerId` is a Loro PeerID; `HttpStatus` is HTTP's |
| A door no family keeps closed | `SealedBlob` is built by the seal alone, while `Payload::new` is public |
| A crate that cannot depend on `ambre-core` | the primitive gate, which reads the workspace and links none of it |

| Element | Where | What holds it |
| :--- | :--- | :--- |
| The rule | `ambre-spec-architecture.md` section 1.1, `ARCH-062` | the primitive gate, `crates/ambre-primitives/src/newtypes.rs` |
| The exceptions | `specs/1-Foundations/ambre-newtypes.tsv`: a key, then a reason | the gate refuses a newtype no line names, a line that names none, and an empty reason. A reviewer reads each reason a change adds, `ARCH-063` |
| The census | every struct of the walk outside a test, with no generic parameter and one field once `PhantomData` is set apart, over an integer, a float, a `String` or a `Vec<u8>` | the gate reads the type as the source writes it: an alias or a macro hides it, and a reviewer holds both |
| A name | an alias that stands for a family alone, section 3 | the gate refuses it, `ARCH-066`. It reads the families from section 2, and refuses one that `ambre-core` declares as no generic struct. A reviewer and `clippy::type_complexity` hold an alias of a compound type, `ARCH-067` |
| A stale line | a line whose newtype was folded or removed | `cargo run -p ambre-primitives -- --prune` removes it, and nothing else |

---

## 2. THE FAMILIES

Each family is one generic of `ambre-core` over a marker. The marker trait is what a new marker
implements; a capability is a trait a marker adds when it needs the operation.

| Need | Family | Marker trait | Capabilities a marker opts in to |
| :--- | :--- | :--- | :--- |
| An identifier of an entity | `Id<E>` | `Entity`, sealed, with its `Repr` | `DocumentScoped`, `TextKeyed` |
| A fixed number of bytes: a nonce, a key, a digest | `FixedBytes<R>` | `FixedRole`, sealed | -- |
| A bearer value that a log never prints | `Credential<K>` | `CredentialKind`, sealed | `WireCredential` |
| A secret that crosses a thread | `Confidential<K>` | `ConfidentialKind` | `WireConfidential`, sealed |
| Bytes that cross the threads | `Payload<K>` | `PayloadKind` | `Becomes<B>` |
| Sealed or signed bytes the relay stores and reads none of | `Encoded<K>` | `EncodedKind`, sealed | -- |
| A key that a `const` table names | `StaticKey<K>` | `KeyKind`, sealed | -- |
| A length in a pixel space | `Px<S>` | `Space`, with its `Repr` | `ScalesTo<B>`, sealed |
| The scale between two pixel spaces | `Scale<A, B>` | `ScalesTo<B>` | -- |
| A print length: points, millimetres | `PrintLength<U>` | `PrintUnit`, sealed | -- |
| A span of time in a unit | `TimeSpan<U>` | `TimeUnit`, sealed | `ClockUnit` |
| An instant on a time base | `Instant<C>` | `TimeBase`, sealed | -- |
| A position, a length, a difference and a range in an index space | `Pos<A>`, `Len<A>`, `Delta<A>`, `Span<A>`, `IdxVec<A, T>` | `Axis` | `Numbered`, `Shown`, `Circular`, `MeasuresAs<B>`, `MapsTo<B>` |
| How many things, addressing nothing | `Count<K>` | `CountKind`, with its `Repr` of `u32` or `u64` | `Recounts<J>`, `Divisor`, `CountColumn`, `IndexedAs` |
| A number one issuer hands out going up, each once | `Seq<K>` | `SeqKind`, with its `IssueRight` | `Wire`, `Before`, `CountsSince`, `SeqColumn` |
| A one-based number a reader sees beside an item | `Ordinal<K>` | `OrdinalKind` | -- |
| Where an item falls in an order, zero included, never counted and handed out by no issuer | `Rank<K>` | `RankKind`, with its `Repr` of `u32` or `i64` | -- |
| A format or a schema version | `Version<K>` | `VersionKind`, with its `Repr` | `Current`, `OpenRange` |
| A number between two bounds | `Bounded<K>` | `Bounds`, with its `Repr` | `Defaulted`, `Saturates` |
| The error of a number outside its bounds, which echoes it | `OutOfRange<K>` | `Bounds`, the marker of the `Bounded<K>` it failed | -- |
| The error of a refused value, which echoes it | `Refused<K>` | `Refuses`, with its `Raw` and its message | -- |
| A text a rule checked once, at the door | `Validated<V>` | `Rule`, with its `LOGGED_AS` | `Displayed` |
| A text of one kind that crosses or is stored, read by no code that carries it, and that a log never prints | `Opaque<K>` | `OpaqueKind`, with its `LENGTH` | `WireOpaque`, `SealedAtRest` |
| Text a log never prints | `Redacted<T>` | -- | -- |
| A small set of named bits, built in a `const` table | `Flags<K>` | `FlagsKind`, with its `Repr` of `u8` and its `ALL`, the mask a const door checks at compile time | -- |

---

## 3. THE MARKER GRAMMAR

A marker is written one way, so a reader meets no variant of it.

| Element | Form | Why |
| :--- | :--- | :--- |
| The marker | an empty `enum` | no value of it exists, so none is built by mistake |
| The phantom field | `PhantomData<fn() -> M>` | the family stays `Send` and `Sync` whatever the marker, and owns no `M` |
| The derived traits | `Clone`, `Copy`, `Eq`, `Ord`, `Hash` and `Debug` written by hand on the family | a derive bounds the marker, which implements none of them |
| The `Debug` name | a `NAME` constant of the marker trait. A `Rule` names its text in `LOGGED_AS`, and the default `None` masks the text | a log, a test and a ZK rule keep the name the hand-written type printed. A rule that forgets the opt-in leaks no text |
| The serde form | the bare representation, transparent | the wire and the store keep the bytes of the type it replaced |
| A closed set | a sealed marker trait | a crate downstream adds no marker where the set is a decision |
| A capability | a trait the marker implements, one line per marker | a method that some markers must not have is never inherent |
| A name | the family and its marker in full: `Count<EditUpdates>` | a type alias hides the family, so none stands for a family instance alone. An alias stays only where the compound type it names -- a tuple, a container, a closure -- fails `clippy::type_complexity` once written out at its uses |
| The fields | private, read through `new` and `get` | a family that checks nothing still has one door |

---

## 4. WHAT THIS DOCUMENT DOES NOT OWN

| Subject | Home |
| :--- | :--- |
| The exceptions, one line each with its reason | `specs/1-Foundations/ambre-newtypes.tsv` |
| Which sites may keep a bare primitive | `specs/1-Foundations/ambre-primitives.tsv`, `ARCH-033` |
| The newtype rule of the style guide | `ai/ambre-rust-style.md` |
| How the gate itself is verified | `ambre-spec-testing.md` section 2 |
