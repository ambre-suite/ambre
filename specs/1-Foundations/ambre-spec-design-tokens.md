# AMBRE -- FEATURE: THE DESIGN TOKENS

**Type:** Horizontal (framework)

> The interface is drawn twice, once by the browser and once by the graphics card. The two halves
> must not disagree about what a surface, an accent or a body size looks like. One file
> declares them, and one generator writes each half its own copy.

The two halves cannot share a value at run time: one needs a stylesheet, the other needs Rust
constants. So they share a source, and a check holds both copies to it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `UI-THREAD`, shared with
`ambre-spec-ui.md`, `ambre-spec-commands.md` and `ambre-spec-clipboard.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UI-THREAD-029` | `INV` | Every colour, spacing, radius and type size the interface uses MUST be declared in the one token source. | this spec section 2 | `ambre_canvas::theme::stylesheet_tests::the_shell_stylesheet_spells_out_no_colour_and_no_scale` -- every declaration of the shell sheet is read, and a colour, a spacing, a radius or a type size spelled out fails it with its line; `ambre_canvas::theme::stylesheet_tests::the_literal_check_catches_each_kind_and_spares_a_token` -- the check catches a literal of each kind and spares a token, a zero, a hairline and a comment | none | verified |
| `UI-THREAD-030` | `INV` | The stylesheet and the drawing constants MUST both be generated from that source by one generator, and the committed copies MUST be what it writes. | this spec section 2 | `ambre_canvas::tokens::tests::a_token_changed_without_a_rewrite_makes_both_files_stale` -- the committed files match the source, and one changed colour makes both stale; `ambre_canvas::tokens::tests::a_rerun_writes_the_same_bytes`; ci-gate -- `just tokens-check`, run by `just test-native` in the CI `test-native` job, fails when either committed file is not what `tokens.ron` generates | none | verified |
| `UI-THREAD-031` | `INV` | A stylesheet MUST NOT read a token the source does not emit and the sheet does not declare. | this spec section 2 | `ambre_canvas::theme::stylesheet_tests::the_stylesheet_reads_only_declared_tokens` -- every name the shell sheet reads is checked against the names the token source emits and the sheet declares | none | verified |
| `UI-THREAD-033` | `INV` | The unmarked stylesheet MUST carry the light set, so a page that never sets the theme attribute renders light rather than unstyled. | this spec section 2; section 4, row 4 | `ambre_canvas::theme::stylesheet_tests::the_unmarked_block_carries_the_light_set` -- reads the generated stylesheet | none | verified |

### 1.2 Switch the theme

The user flips the theme control. One act must move both halves, and it must move nothing else.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UI-THREAD-032` | `POST` | A theme switch MUST reach the browser half and the drawing half together, from one act. | this spec section 2 | `ambre_app::platform::tests::a_theme_switch_reaches_both_halves` -- one switch, read back from the control and the document attribute, in the browser tier | none | verified |
| `UI-THREAD-034` | `FRAME` | A theme switch MUST NOT change a spacing, a radius or a type size. | this spec section 2; section 4, row 5 | `ambre_canvas::theme::stylesheet_tests::a_theme_switch_changes_no_scale` -- the dark block declares colours and effects alone | none | verified |

* **Budget.** none: a token is a constant compiled into the binary, so it costs nothing at run time.
  A theme switch repaints the content once, which is the frame budget of
  `ambre-spec-budgets.md section 2`.
* **Boundary.** none: a token is a colour and a number, generated into the binary. Nothing here
  reaches the relay, holds a key or opens a socket. The chosen theme leaves sealed, in the user
  settings (`ZK-053`).
* **Verification.** 6 `verified`. A drift check keeps the two committed sides on one source; host
  tests read the generated sheets, and a browser test drives a switch.

---

## 2. MODEL

**CRDT containers.** none. A token is a property of the build, not of the document. The chosen
theme is a setting of the account, kept in the user settings (`ambre-spec-user-settings.md`).

**The pipeline.** One source, one generator, two committed artefacts.

1. the source declares two colour sets for the browser half, two for the drawing half, and one set
   of theme-independent scales;
2. `just tokens-write` reads it and writes a stylesheet of custom properties and a Rust module of
   constants, both committed;
3. `just tokens-check` fails when either file is not what the source generates (`UI-THREAD-030`).
   The drawing crate declares the module and embeds the sheet as text;
4. the launcher injects the stylesheet into the page at startup and sets the document's theme
   attribute;
5. a switch flips one control; the browser half re-resolves its properties and the drawing half
   reads the control on the next frame (`UI-THREAD-032`).

**What the source declares.**

| Group | Holds | Reaches |
|---|---|---|
| the chrome colours | background, surface, second surface, hover, border, text, dimmed text, accent, the four status colours, and the text drawn on the accent | the browser half, as custom properties |
| the chrome effects | three shadows, low, mid and high, and the scrim behind a modal | the browser half, as custom properties |
| the photo colours | the text, its shadow, the lines, the control and the veil drawn over a picture | the browser half, as custom properties |
| the content colours | content ground, pending ground, header ground, active and hovered header, grid line, header border, selection, cell text, header text, caret, error | the drawing half, as two palettes |
| the scales | a spacing ladder in steps of two pixels, five radii, nine type sizes | both halves |

Each colour group is declared twice, light and dark, but the photo colours. Their ground is a
picture, not the theme, so one set serves both. A scale is declared once: a spacing does not
change with the theme.

**What the shell may still spell out.** A width, a height and a position are the shape of one
component, not a shared value. A hairline of one pixel is a border width, not a spacing, and
zero is no value at all. Everything else the shell sheet sets for a colour, a padding, a margin,
a gap, a radius or a type size reads a token (`UI-THREAD-029`).

**The boot screen is drawn before any token exists.** Its few colours and sizes are written in the
page itself, and it is gone at readiness. The fatal error overlay comes after the injection, so it
reads the tokens.

**Where the two halves diverge, and why that is correct.** The chrome names a surface and the
content a grid line. They agree on the entries both groups share by name: the accent, the text
colour and the scales.

**The theme is a run-time value, not a build one.** Both palettes are in the binary. A control the
chrome writes and the render loop reads selects one, and an attribute on the document root selects
the browser half. The light set is the unmarked default (`UI-THREAD-033`).

**Code.**

* `crates/ambre-canvas/tokens.ron` -- the source;
* `crates/ambre-canvas/src/tokens.rs` -- the generator, behind the `tokens` feature;
* `crates/ambre-canvas/src/bin/ambre-tokens.rs` -- the driver of `just tokens-write` and
  `just tokens-check`;
* `crates/ambre-canvas/src/theme_gen.rs` and `src/theme.css` -- the two generated files;
* `crates/ambre-canvas/src/theme.rs` -- the re-export, the sheet, the theme value and the shared
  control;
* `crates/ambre-app/src/platform.rs` -- the injection of the generated sheet into the page;
* `crates/ambre-app/styles.css` -- the shell layout, which reads the properties and spells out
  none of its own.

---

## 3. SURFACE

The user touches one thing: the theme control. Everything else is what they see.

| Trigger | Behaviour |
|---|---|
| the application starts | the generated sheet is injected, the dark set is selected, and the content is drawn from the dark palette |
| the theme is switched | the chrome re-resolves every property at once, and the content repaints on the next frame |
| a component is added with no colour of its own | it inherits the surface and text of its region, because those are properties and not literals |
| a token is changed in the source | every use of it moves, on both halves, once `just tokens-write` rewrites the two files. A change committed without the rewrite fails `just tokens-check` |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One source, generated into two artefacts at build time | keeping a stylesheet and a Rust palette side by side, and reviewing that they match | two hand-kept lists agree until one is edited alone, which is the failure this design exists to prevent. The browser half needs a stylesheet and the drawing half needs typed constants, and no run-time value can be both, so the two artefacts are unavoidable -- only the hand-keeping is |
| The generated artefacts are committed, and a drift check holds them to the source | a build script that writes them into the build output, and a module that includes them from there | the primitive gate and the secrets check read no file an `include!` names (`ARCH-045`), so an included module escapes every gate that reads the source. A build script must not write into `src/` either. The drift check fails the day a committed file and the source disagree, as `just fixtures-check` does for the fixture corpus |
| Both palettes ship, and the theme is selected at run time | a build-time theme, one binary per palette | it doubles the binary count and makes the switch a reload. Two palettes cost the size of forty-eight colours, which is nothing against the binary budget |
| Light is the unmarked default | making dark the unmarked set | an unattributed page would be dark text on a dark ground. Light degrades to something readable instead, which is the same rule the adaptive layout uses for its narrowest width |
| A scale is theme-independent | one full token set per theme, scales included | a scale that switches with the theme reflows the interface on a colour change. A spacing and a type size are the shape of the layout, and a theme is its colour |
| The colour vocabulary differs between the two halves | one flat palette both halves index into | the chrome has surfaces and borders, the content has grid lines and carets, so a shared vocabulary names each concept for the half that does not use it. What must agree is named the same in both groups anyway |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The check reads the shell sheet alone.** The boot screen in the page is drawn before the
  tokens exist, and writes its own. A style an application sets from Rust is not read either:
  today that is one zero border, on an embedded page.
* **The scale is what the shell used, not a designed one.** The ladder keeps every spacing the
  sheet carried, rounding an odd one to its even neighbour. Three dialog headings of three sizes
  became one heading. Nothing yet rules on how many steps a ladder may hold.
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
