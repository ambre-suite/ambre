# AMBRE -- FEATURE: THE DESIGN TOKENS

**Type:** Horizontal (framework)
**Implementation:** partial -- one source generates both sides and the theme switch works. The
stylesheet still carries fifty-eight colours the source never declared.

> The interface is drawn twice, once by the browser and once by the graphics card. The two halves
> must not disagree about what a surface, an accent or a body size looks like. One file
> declares them, and a build step gives each half its own copy.

The two halves cannot share a value at run time: one needs a stylesheet, the other needs Rust
constants. So they share it at build time instead.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `UI-THREAD`, shared with
`ambre-spec-ui.md`, `ambre-spec-commands.md` and `ambre-spec-clipboard.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UI-THREAD-029` | `INV` | Every colour, spacing, radius and type size the interface uses MUST be declared in the one token source. | this spec section 2 | none | fails -- the shell stylesheet carries fifty-eight literal colours, twenty-five of them distinct, of which every shadow and every white on a coloured ground, and no check counts them; lands with the `The stylesheet reads the tokens` Story (`ambre-roadmap.md` E6) | failing |
| `UI-THREAD-030` | `INV` | The stylesheet and the drawing constants MUST both be generated from that source, in one build step. | this spec section 2 | target-build -- the drawing crate includes the generated constants and the generated sheet as files the build step writes, so neither exists unless that step ran and read the source | none | verified |
| `UI-THREAD-031` | `INV` | A stylesheet MUST NOT read a token the source does not emit and the sheet does not declare. | this spec section 2 | `ambre_canvas::theme::stylesheet_tests::the_stylesheet_reads_only_declared_tokens` -- every name the shell sheet reads is checked against the names the token source emits and the sheet declares | none | verified |
| `UI-THREAD-033` | `INV` | The unmarked stylesheet MUST carry the light set, so a page that never sets the theme attribute renders light rather than unstyled. | this spec section 2; section 4, row 4 | none | unheld -- the generator writes the light colours into the unmarked block and the dark ones into the attributed one, so the default is the order of two blocks in one build script, and nothing goes red the day they swap; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 Switch the theme

The user flips the theme control. One act must move both halves, and it must move nothing else.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UI-THREAD-032` | `POST` | A theme switch MUST reach the browser half and the drawing half together, from one act. | this spec section 2 | none | unheld -- one control carries the active theme, the launcher sets the document attribute from it and the render loop reads it on the next frame. Nothing drives a switch and observes both halves, and no test in the suite names the theme at all; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `UI-THREAD-034` | `FRAME` | A theme switch MUST NOT change a spacing, a radius or a type size. | this spec section 2; section 4, row 5 | none | unheld -- the generator emits the scales once, into the unmarked block, and the drawing palette declares twelve colours and no scale field, so a theme-dependent scale takes an edit to the build script, which compiles and which no test reads; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** none: a token is a constant resolved at build time, so it costs nothing at run time.
  A theme switch repaints the content once, which is the frame budget of
  `ambre-spec-budgets.md section 2`.
* **Boundary.** none: a token is a colour and a number, generated into the binary. Nothing here
  reaches the relay, holds a key or opens a socket, and the chosen theme is never sent anywhere.
* **Verification.** 2 `verified`, 1 `failing`, 3 `untested`. The compiler holds the half that
  matters: the two sides cannot come from two sources. A test holds that the shell sheet reads only
  declared names. What nothing holds is whether a value bypassed the source, and the failing row
  says it did.

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
accent, the text colour and the scales. Those are the entries that appear in both groups under the
same name.

**The theme is a run-time value, not a build one.** Both palettes are in the binary and the active
one is selected by a control the chrome writes and the render loop reads. An attribute on the
document root selects the browser
half. The light set is the unmarked default, the dark set the attributed one. A page
that fails to set the attribute is light rather than unstyled.

**Code.**

* `crates/ambre-canvas/tokens.ron` -- the source;
* `crates/ambre-canvas/build.rs` -- the generator;
* `crates/ambre-canvas/src/theme.rs` -- the includes, the theme value and the shared control;
* `crates/ambre-app/src/platform.rs` -- the injection of the generated sheet into the page;
* `crates/ambre-app/styles.css` -- the shell layout, which reads the properties and is where the
  fifty-eight literals are.

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

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One source, generated into two artefacts at build time | keeping a stylesheet and a Rust palette side by side, and reviewing that they match | two hand-kept lists agree until one is edited alone, which is the failure this design exists to prevent. The browser half needs a stylesheet and the drawing half needs typed constants, and no run-time value can be both, so the two artefacts are unavoidable -- only the hand-keeping is |
| The generated artefacts are included, never committed | writing the generated files into the tree so they are readable in a diff | a committed artefact is a second source that a build can silently disagree with, and reviewing a generated diff is reviewing the generator twice |
| Both palettes ship, and the theme is selected at run time | a build-time theme, one binary per palette | it doubles the binary count and makes the switch a reload. Two palettes cost the size of forty-eight colours, which is nothing against the binary budget |
| Light is the unmarked default | making dark the unmarked set | an unattributed page would be dark text on a dark ground. Light degrades to something readable instead, which is the same rule the adaptive layout uses for its narrowest width |
| A scale is theme-independent | one full token set per theme, scales included | a scale that switches with the theme reflows the interface on a colour change. A spacing and a type size are the shape of the layout, and a theme is its colour |
| The colour vocabulary differs between the two halves | one flat palette both halves index into | the chrome has surfaces and borders, the content has grid lines and carets, so a shared vocabulary names each concept for the half that does not use it. What must agree is named the same in both groups anyway |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The shell stylesheet carries fifty-eight literal colours.** Every shadow is a literal, so no theme
  changes one. Every white on a coloured ground is a literal too, so a light theme cannot lighten
  the ground under it. The file's own header still promises the token sets for later, and the sets
  are there (`UI-THREAD-029`).
* **One test reads the sheets, and it reads names only.** It holds that every token the shell sheet
  reads is declared (`UI-THREAD-031`). Neither tier constructs a palette or drives a switch, so the
  values are checked by reading.
* **Nothing rules on contrast.** The two palettes are chosen by eye. No rule states a minimum ratio
  between a text colour and the ground it is drawn on (`ambre-spec-accessibility.md`).
* **Nothing rules on a third theme, or on the system preference.** The switch has two values and
  the page never reads what the operating system asks for.
* **The scales have no unit in the source.** They are unitless integers, read as pixels by one half and as
  a floating-point length by the other. Nothing states which unit they are in.

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
