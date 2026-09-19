#!/usr/bin/env python3
"""What `spec_code.py` reads, and what it refuses.

The rule is one line of the template: a spec carries shapes and no
implementation. Making it mechanical took three readers -- one for a `fn` body,
one for a statement that belongs to no declaration, one for the fences both
depend on -- and the fence reader is the foundation of the other two. It has
already been wrong in exactly the way a gate is wrong silently: a single
untracked ```text fence desynchronised every fence after it, so the body
checker read prose as code and code as prose and reported nothing either way.

That regression is frozen below, with the line each reader must let through.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import spec_code  # noqa: E402  -- the module under test
import template_vocabulary  # noqa: E402  -- the tags it reads from the template


def doc(*lines: str) -> str:
    return "\n".join(lines) + "\n"


def signatures(text: str) -> list[str]:
    return [sig for _, _, sig in spec_code.bodies(text)]


def kinds(text: str) -> list[str]:
    return [kind for _, kind, _ in spec_code.fence_issues(text)]


class Bodies(unittest.TestCase):
    """A function body is a second, uncompiled copy of the code."""

    def test_a_body_in_a_rust_fence_is_refused(self):
        text = doc("```rust", "pub fn drop(frame: Frame) {", "    self.queue.push(frame);", "}", "```")
        self.assertEqual(["pub fn drop(frame: Frame) {"], signatures(text))

    def test_a_declaration_is_a_shape(self):
        text = doc("```rust", "pub fn drop(frame: Frame) -> bool;", "```")
        self.assertEqual([], signatures(text))

    def test_a_collapsed_body_is_the_shape_the_template_asks_for(self):
        """`{ /* ... */ }` plus a pointer to the real file is the fix the gate
        tells an author to apply, so it cannot itself be a defect."""
        text = doc("```rust", "pub fn drop(frame: Frame) {", "    /* ... */", "}", "```")
        self.assertEqual([], signatures(text))

    def test_a_body_of_comments_alone_carries_no_code(self):
        text = doc("```rust", "pub fn drop(frame: Frame) {", "    // the queue owns it", "}", "```")
        self.assertEqual([], signatures(text))

    def test_a_one_liner_closing_on_its_own_line_is_illustrative(self):
        text = doc("```rust", "pub fn len(&self) -> usize { self.queue.len() }", "```")
        self.assertEqual([], signatures(text))

    def test_a_body_under_a_wrapped_signature_is_still_a_body(self):
        """The single-line `size > 2` heuristic missed this one: the signature
        spans four lines, so the brace is not on the `fn` line."""
        text = doc(
            "```rust",
            "pub fn drop(",
            "    frame: Frame,",
            "    now: Instant,",
            ") -> bool {",
            "    self.queue.push(frame);",
            "    true",
            "}",
            "```",
        )
        self.assertEqual(1, len(signatures(text)))

    def test_a_method_is_seen_through_its_impl_wrapper(self):
        """Depth is tracked only inside a detected body, so the `impl {` brace
        does not hide the method under it."""
        text = doc(
            "```rust",
            "impl Bus {",
            "    pub fn drop(&mut self, frame: Frame) {",
            "        self.queue.push(frame);",
            "    }",
            "}",
            "```",
        )
        self.assertEqual(1, len(signatures(text)))

    def test_prose_outside_a_fence_is_not_read(self):
        self.assertEqual([], signatures(doc("The bus calls fn drop(frame) { } on every gesture.")))

    def test_a_text_fence_does_not_desynchronise_the_fences_after_it(self):
        """The regression this file exists for. Every fence toggles, whatever
        its tag; when `text` did not, the rust fence below read as prose."""
        text = doc(
            "```text",
            "frame -> queue -> sink",
            "```",
            "",
            "```rust",
            "pub fn drop(frame: Frame) {",
            "    self.queue.push(frame);",
            "}",
            "```",
        )
        self.assertEqual(1, len(signatures(text)))


class Fences(unittest.TestCase):
    def test_an_embedded_script_is_implementation(self):
        """`lua` is not an allowed tag: a script belongs in the source."""
        self.assertEqual(["bad-tag"], kinds(doc("```lua", "return 1", "```")))

    def test_an_unterminated_fence_is_refused(self):
        self.assertEqual(["unterminated"], kinds(doc("```rust", "pub struct Bus;")))

    def test_a_rust_shape_in_the_wrong_fence_is_refused(self):
        """A mis-tagged shape is Rust the body checker would never see."""
        text = doc("```text", "pub struct Bus {", "    queue: Vec<Frame>,", "}", "```")
        self.assertEqual(["rust-untagged"], kinds(text))

    def test_pseudo_code_is_what_a_text_fence_is_for(self):
        """Real Rust opens a block with `{`; `fn page() -> f32:` does not."""
        self.assertEqual([], kinds(doc("```text", "fn page() -> f32:", "    width / columns", "```")))

    def test_an_untagged_fence_is_allowed(self):
        self.assertEqual([], kinds(doc("```", "frame -> queue", "```")))

    def test_the_tag_is_the_base_language_token(self):
        self.assertEqual("rust", spec_code.fence_tag("```rust,ignore"))
        self.assertEqual("", spec_code.fence_tag("```"))

    def test_the_allowed_tags_are_the_templates_own(self):
        """A second list here is a list that drifts: the table states them."""
        self.assertEqual(
            frozenset(template_vocabulary.FENCE_TAGS), spec_code.ALLOWED_TAGS
        )


class LooseImplementation(unittest.TestCase):
    """A statement at depth 0 belongs to no declaration, so no `fn` holds it."""

    def test_a_running_statement_at_fence_top_level_is_refused(self):
        text = doc("```rust", "let caret = frame.caret();", "```")
        self.assertEqual(1, len(list(spec_code.loose_impl(text))))

    def test_a_match_at_fence_top_level_is_refused(self):
        text = doc("```rust", "match frame {", "    Frame::Empty => (),", "}", "```")
        self.assertEqual(1, len(list(spec_code.loose_impl(text))))

    def test_a_statement_inside_a_shape_is_not_at_top_level(self):
        """The body checker owns that case; counting it twice would report one
        defect as two."""
        text = doc("```rust", "pub fn drop(f: Frame) {", "    let caret = f.caret();", "}", "```")
        self.assertEqual([], list(spec_code.loose_impl(text)))

    def test_an_item_at_top_level_is_what_a_spec_fence_holds(self):
        text = doc("```rust", "use crate::bus::Frame;", "pub struct Bus;", "type Queue = Vec<Frame>;", "```")
        self.assertEqual([], list(spec_code.loose_impl(text)))

    def test_a_diagram_in_a_text_fence_is_not_read(self):
        self.assertEqual([], list(spec_code.loose_impl(doc("```text", "let x = the caret", "```"))))


if __name__ == "__main__":
    unittest.main()
