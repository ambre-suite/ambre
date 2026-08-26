# AMBRE -- FEATURE: THE DESIGN TOKENS

**Type:** Horizontal (framework)
**Implementation:** partial -- one source generates both sides and the theme switch works; the
stylesheet still carries fifty colours the source never declared.

> The interface is drawn twice, once by the browser and once by the graphics card, and the two
> halves must not disagree about what a surface, an accent or a body size looks like. One file
> declares them, and a build step gives each half its own copy.

The two halves cannot share a value at run time: one needs a stylesheet, the other needs Rust
constants. So they share it at build time instead.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `UI-THREAD`, shared with
`ambre-spec-ui.md`, `ambre-spec-commands.md` and `ambre-spec-clipboard.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `UI-THREAD-029` | Every colour, spacing, radius and type size the interface uses MUST be declared in the one token source. | this spec section 2 | none, and **the rule fails today**: the shell stylesheet carries fifty literal colours, of which every shadow and every white on a coloured ground, and no check counts them. The repair is the `The stylesheet reads the tokens` Story (`ambre-roadmap.md` E6) | unverified |
| `UI-THREAD-030` | The stylesheet and the drawing constants MUST both be generated from that source, in one build step. | this spec section 2 | type-level: the drawing crate includes the generated constants and the generated sheet as files the build step writes, so neither exists unless that step ran and read the source | verified |
| `UI-THREAD-031` | A stylesheet MUST NOT read a token the source does not emit and the sheet does not declare. | this spec section 2 | none, and **the rule fails today**: five names are read and defined nowhere, and one of the five carries no fallback, so a label on the home screen has no colour of its own. The repair is the `The stylesheet reads the tokens` Story (`ambre-roadmap.md` E6) | unverified |
| `UI-THREAD-032` | A theme switch MUST reach the browser half and the drawing half together, from one act. | this spec section 2 | none. One control carries the active theme, the launcher sets the document attribute from it and the render loop reads it on the next frame. Nothing drives a switch and observes both halves, and no test in the suite names the theme at all | unverified |

* **Budget.** none: a token is a constant resolved at build time, so it costs nothing at run time.
  A theme switch repaints the content once, which is the frame budget of
  `ambre-spec-budgets.md section 2`, and the generated sheet is a few hundred bytes inside
  the binary budget of `ambre-spec-budgets.md section 5`.
* **Boundary.** none: a token is a colour and a number, generated into the binary. Nothing here
  reaches the relay, holds a key or opens a socket, and the chosen theme is never sent anywhere.
* **Verification.** 1 `verified`, 3 `unverified`, 0 `unimplemented`. The one held rule is held by
  the compiler and holds the half that matters most: the two sides cannot come from two sources.
  What nothing holds is whether a value bypassed the source, and both failing rows say it did.

---

## 2. MODEL

**CRDT containers.** none. A token is a property of the build, not of the document, and the chosen
theme is a property of the machine.

**The pipeline.** One source, one build step, two artefacts.

1. the source declares two colour sets for the browser half, two for the drawing half, and one set
   of theme-independent scales;
2. the build step reads it and writes a stylesheet of custom properties and a Rust module of
   constants;
3. the drawing crate includes both, so neither can exist without the source (`UI-THREAD-030`);
4. the launcher injects the stylesheet into the page at startup and sets the document's theme
   attribute;
5. a switch flips one control; the browser half re-resolves its properties and the drawing half
   reads the control on the next frame (`UI-THREAD-032`).

**What the source declares.**

| Group | Holds | Reaches |
|---|---|---|
| the chrome colours | background, surface, second surface, hover, border, text, dimmed text, accent, and the four status colours | the browser half, as custom properties |
| the content colours | content ground, pending ground, header ground, active and hovered header, grid line, header border, selection, cell text, header text, caret, error | the drawing half, as two palettes |
| the scales | four spacings, one radius, three type sizes | both halves |

Each colour group is declared twice, light and dark. A scale is declared once: a spacing does not
change with the theme.

**Where the two halves diverge, and why that is correct.** The chrome names a surface and the
content names a grid line, and neither needs the other's vocabulary. What they must agree on is the
accent, the text colour and the scales, and those are the entries that appear in both groups under
the same name.

**The theme is a run-time value, not a build one.** Both palettes are in the binary and the active
one is selected by a control the chrome writes and the render loop reads. The browser half is
selected by an attribute on the document root: the light set is the unmarked default and the dark
set is the attributed one, so a page that fails to set the attribute is light rather than
unstyled.

**Code.**

* `crates/ambre-canvas/tokens.ron` -- the source;
* `crates/ambre-canvas/build.rs` -- the generator;
* `crates/ambre-canvas/src/theme.rs` -- the includes, the theme value and the shared control;
* `crates/ambre-app/src/platform.rs` -- the injection of the generated sheet into the page;
* `crates/ambre-app/styles.css` -- the shell layout, which reads the properties and is where the
  fifty literals are.

---

## 3. SURFACE

The user touches one thing: the theme control. Everything else is what they see.

| Trigger | Behaviour |
|---|---|
| the application starts | the generated sheet is injected, the dark set is selected, and the content is drawn from the dark palette |
| the theme is switched | the chrome re-resolves every property at once, and the content repaints on the next frame |
| a component is added with no colour of its own | it inherits the surface and text of its region, because those are properties and not literals |
| a token is changed in the source | every use of it moves, on both halves, on the next build |

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One source, generated into two artefacts at build time | the browser half needs a stylesheet and the drawing half needs typed constants, and no run-time value can be both. Generating them removes the failure this design exists to prevent: two hand-kept lists that agree until one is edited alone | keeping a stylesheet and a Rust palette side by side, and reviewing that they match |
| The generated artefacts are included, never committed | a committed artefact is a second source that a build can silently disagree with, and reviewing a generated diff is reviewing the generator twice | writing the generated files into the tree so they are readable in a diff |
| Both palettes ship, and the theme is selected at run time | a build per theme doubles the binary count and makes the switch a reload. Two palettes cost the size of forty-eight colours, which is nothing against the binary budget | a build-time theme, one binary per palette |
| Light is the unmarked default | a page that has not yet applied the attribute renders light rather than unstyled, which is the failure that degrades to something readable. It is also the same rule the adaptive layout uses for its narrowest width | making dark the unmarked set, so an unattributed page is dark text on a dark ground |
| A scale is theme-independent | a spacing and a type size are the shape of the layout, and a theme is its colour. Making a scale switch with the theme means the interface reflows on a colour change | one full token set per theme, scales included |
| The colour vocabulary differs between the two halves | the chrome has surfaces and borders, the content has grid lines and carets, and a shared vocabulary would name each concept for the half that does not use it. What must agree is named the same in both groups | one flat palette both halves index into |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The shell stylesheet carries fifty literal colours.** Every shadow is a literal, so no theme
  changes one, and every white on a coloured ground is a literal, so a light theme cannot lighten
  the ground under it. The file's own header still says the token sets will land later, and they
  landed (`UI-THREAD-029`).
* **Five tokens are read and declared nowhere:** the content ground, a danger colour, a foreground,
  a shadow and a dimmed text. Four have a fallback and degrade quietly; the foreground has none, so
  a label on the home screen takes whatever it inherits (`UI-THREAD-031`).
* **The danger colour is a token that does not exist under a name that does.** The status group
  declares an error colour, and the sheet asks for a danger one and falls back to a red of its own.
  Two names, one meaning, and the one the source emits is unused.
* **Two accent fallbacks contradict the accent.** Where the sheet writes the accent with a
  fallback, the fallback is a blue and the token is amber, so the two disagree about the product's
  own colour.
* **No test names the theme.** Neither tier constructs a palette, reads the generated sheet or
  drives a switch, so every row here except the compiler-held one is checked by reading.
* **Nothing rules on contrast.** The two palettes are chosen by eye and no rule states a minimum
  ratio between a text colour and the ground it is drawn on
  (`ambre-spec-accessibility.md`).
* **Nothing rules on a third theme, or on the system preference.** The switch has two values and
  the page never reads what the operating system asks for.
* **The scales have no unit in the source.** They are unitless integers read as pixels by one half
  and as a floating-point length by the other, and nothing states which unit they are in.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How the drawing half consumes a palette, and what a switch repaints | `ambre-spec-frame.md`, `CANVAS-015` |
| The three widths a layout adapts at, and what moves between them | `ambre-spec-adaptive-layout.md` |
| What a dense display multiplies a scale by | `ambre-spec-high-density.md` |
| The components the properties are applied to | `ambre-spec-chrome.md` |
| Whether a colour is legible, and the contrast a mandate would require | `ambre-spec-accessibility.md` |
| Where the theme control sits in the interface | `ambre-spec-chrome.md` |
| Which colours an application's own render code may name | each application's rendering feature |
