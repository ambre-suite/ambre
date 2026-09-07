# AMBRE -- WRITING STYLE

> Every English description uses concise Simplified Technical English: specifications, analyses,
> audits, rustdoc, Rust comments, commit messages, pull request bodies. Discussion with the user
> stays in the user's language and is out of scope.
> Reference: <https://en.wikipedia.org/wiki/Simplified_Technical_English> (ASD-STE100), minus its
> approved dictionary of 900 general words, which cannot express "CRDT convergence". The project's
> technical vocabulary is the dictionary; the rules below are what is kept.

* **One sentence, one idea.** 20 words maximum for a rule, 25 for descriptive text. 6 sentences
  maximum per paragraph.
* **Active voice, named actor.** "The Sync worker seals the frame", not "the frame is sealed".
  Present tense for behaviour, imperative for an instruction. One negative per sentence.
* **One word, one meaning, and one meaning, one word.** Never vary a term for style. If it is a
  `frame`, it is a `frame` everywhere.
* **The simplest exact word wins.** "use", not "utilize". No idiom, no metaphor, no humour, no
  Latin abbreviation: write "for example", "that is".
* **No ambiguous pronoun.** "it" and "this" must have one possible antecedent; repeat the noun when
  in doubt. Articles are mandatory. No noun cluster longer than three words.
* **Delete empty openers:** "It should be noted that", "In order to", "basically", "obviously".
* **A list for a sequence, a table for a mapping.** Both beat a sentence chaining "and", "or" and
  "except". Digits then a space then an SI unit: `15 ms`. Expand an acronym on first use.
* **RFC 2119 keywords stay uppercase** in a contract statement, and keep their normative force.
  This overrides the simplest-word rule.
* **Printable ASCII only, code points 32 to 126**, everywhere: prose, code, comments, commit
  messages, file names. Write `--`, `->`, and "section 4".
* **A Rust comment explains why, never what.** A public item states its contract and its failure
  modes.

No script checks this file, unlike `just contracts` and `just spec-code`. It is a review rule, and
review rules drift.
