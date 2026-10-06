#!/usr/bin/env python3
"""What `spec_refs.py` reads, and what it refuses.

This gate resolves two namespaces, and it has been blind in both. A pattern
requiring a closing backtick after a name saw none of the 423 citations that
carry the coordinate inside the same span; a name pattern stopping at `/` read
311 prefixed siblings as no citation at all. Both reported success the whole
time, which is the shape of every defect this file is written against: the
resolver answers "no citation here" and a reader cannot tell that from "the
citation resolves".

So the tests below are mostly acceptance tests. Each names the form the
resolver must **see**, because a citation it does not see is a citation it
cannot refuse.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import spec_refs  # noqa: E402  -- the module under test


# A short-name table fixed here rather than read from the corpus: a test that
# reads the corpus reports the corpus, and the rule is what is under test.
SHORTS = {
    "architecture": "ambre-spec-architecture.md",
    "sync": "ambre-spec-sync.md",
    "ui": "ambre-spec-ui.md",
}


def doc(*lines: str) -> str:
    return "\n".join(lines) + "\n"


def claimed(candidate: str, coordinated: bool = False, in_span: bool = False):
    hit = spec_refs.resolve(candidate, SHORTS, coordinated, in_span)
    return hit[1] if hit else None


def names(text: str, own: str = "ambre-spec-frame.md") -> list[str]:
    return [claim for kind, _, _, claim, _ in spec_refs.citations(text, own, SHORTS)
            if kind == "name"]


def coords(text: str, own: str = "ambre-spec-frame.md") -> list[tuple[str | None, str]]:
    return [(target, value) for kind, _, target, value, _ in spec_refs.citations(text, own, SHORTS)
            if kind == "coord"]


class Resolve(unittest.TestCase):
    """Which backticked word is a document name, and which document it claims."""

    def test_a_full_name_claims_itself(self):
        self.assertEqual("ambre-spec-sync.md", claimed("ambre-spec-sync.md"))

    def test_the_prefix_alone_is_enough(self):
        self.assertEqual("ambre-spec-sync.md", claimed("ambre-spec-sync"))

    def test_an_application_prefix_rebuilds_the_name(self):
        self.assertEqual("ambre-spec-trame-sort.md", claimed("trame-sort"))

    def test_a_hyphenated_bare_name_is_a_document(self):
        """The roadmap, the audits and the generated ledger carry no prefix."""
        self.assertEqual("ambre-roadmap.md", claimed("ambre-roadmap.md"))

    def test_a_short_name_needs_a_coordinate(self):
        """`` `sync section 3` `` is a citation; a bare `` `sync` `` is the word."""
        self.assertIsNone(claimed("sync"))
        self.assertEqual("ambre-spec-sync.md", claimed("sync", coordinated=True))

    def test_a_crate_is_not_a_document(self):
        """The prefix is `ambre-spec-`, not `ambre-`: every crate is named
        `ambre-<something>` and backticked in half the corpus."""
        self.assertIsNone(claimed("ambre-bus"))

    def test_a_transposed_prefix_must_still_resolve(self):
        """`spec-ambre-sync` reads as a name and sat in the corpus unseen."""
        self.assertEqual("spec-ambre-sync.md", claimed("spec-ambre-sync"))

    def test_a_file_name_that_is_not_hyphenated_is_not_a_citation(self):
        self.assertIsNone(claimed("reference.md"))

    def test_a_coordinate_inside_the_span_makes_the_candidate_a_name(self):
        """Whatever `` `logic section 6` `` is, it is being read as a document,
        so it must resolve or be reported."""
        self.assertIsNone(claimed("logic"))
        self.assertEqual("logic.md", claimed("logic", coordinated=True, in_span=True))


class Names(unittest.TestCase):
    """The five forms the corpus writes, and the two that were invisible."""

    def test_a_name_carrying_its_coordinate_in_one_span_is_seen(self):
        """423 citations of this form were outside the resolver while it
        reported success."""
        self.assertEqual(["ambre-spec-trame-logic.md"], names("See `trame-logic section 9`."))

    def test_a_directory_prefix_is_dropped_and_the_name_resolves(self):
        """311 prefixed siblings read as no citation at all (finding 68)."""
        self.assertEqual(
            ["ambre-spec-toile-links.md"],
            names("See `features/ambre-spec-toile-links.md`."),
        )

    def test_a_rules_file_is_cited_through_its_directory(self):
        self.assertEqual(["ambre-writing-style.md"], names("See `ai/ambre-writing-style.md`."))

    def test_a_cargo_feature_is_not_a_document_path(self):
        """A crate name is the one prefix that cannot introduce a document."""
        self.assertEqual([], names("The feature `ambre-logic/toile-pdf` gates it."))

    def test_a_source_path_is_not_a_citation(self):
        self.assertEqual([], names("It lives in `crates/ambre-prose/src/lib.rs`."))


class Coordinates(unittest.TestCase):
    """Which document a `section N` belongs to, decided by adjacency."""

    def test_the_name_it_touches_owns_it(self):
        self.assertEqual([("ambre-spec-architecture.md", "5")], coords("See `architecture` section 5."))

    def test_the_coordinate_inside_the_span_belongs_to_the_same_name(self):
        self.assertEqual([("ambre-spec-sync.md", "3")], coords("See `sync section 3`."))

    def test_a_chain_inherits_the_document_of_the_first_coordinate(self):
        self.assertEqual(
            [("ambre-spec-sync.md", "6"), ("ambre-spec-sync.md", "7")],
            coords("See `sync section 6`/section 7."),
        )

    def test_a_bare_coordinate_belongs_to_the_citing_document(self):
        self.assertEqual([("ambre-spec-frame.md", "2")], coords("Section 2 states it."))

    def test_a_coordinate_at_the_far_end_of_a_sentence_is_not_adjacent(self):
        """A document named a paragraph earlier owns nothing: the glue is short
        by design."""
        text = "`sync` holds the envelope, the cadence and the retry, and section 4 states the rest."
        self.assertEqual([("ambre-spec-frame.md", "4")], coords(text))

    def test_adjacency_survives_one_line_break(self):
        """A citation the writer wrapped is one citation to a reader, and was
        two to the resolver -- which then found a section 5 that existed in the
        citing document and said something else."""
        self.assertEqual(
            [("ambre-spec-architecture.md", "5")],
            coords(doc("See `architecture`", "section 5.F for the boot order.")),
        )

    def test_a_comment_marker_opening_the_next_line_counts_for_nothing(self):
        self.assertEqual(
            [("ambre-spec-architecture.md", "5")],
            coords(doc("/// See `architecture`", "/// section 5 for the boot order.")),
        )

    def test_a_crate_name_is_not_an_anchor_either(self):
        """`` `ambre-chrome` (section 1) `` names a crate, so the coordinate
        belongs to the citing document."""
        self.assertEqual([("ambre-spec-frame.md", "1")], coords("`ambre-chrome` (section 1)"))

    def test_the_section_mark_is_the_same_coordinate(self):
        """`§` is a second spelling, so both are read or one hides from the
        other."""
        self.assertEqual([("ambre-spec-sync.md", "3")], coords("See `sync` §3."))

    def test_a_sub_section_is_verified_on_its_number(self):
        self.assertEqual([("ambre-spec-sync.md", "5")], coords("See `sync` section 5.B."))


class ShortNames(unittest.TestCase):
    def test_the_short_name_is_the_stem_between_the_two_fixed_halves(self):
        known = {
            "ambre-spec-sync.md": Path("a"),
            "ambre-spec-trame-sync.md": Path("b"),
            "ambre-roadmap.md": Path("c"),
        }
        self.assertEqual(
            {"sync": "ambre-spec-sync.md", "trame-sync": "ambre-spec-trame-sync.md"},
            spec_refs.short_names(known),
        )

    def test_a_document_without_the_prefix_has_no_short_name(self):
        self.assertEqual({}, spec_refs.short_names({"ambre-roadmap.md": Path("a")}))


class QuotedStories(unittest.TestCase):
    """A Story name is backticked, because a tool finds it by its backticks."""

    def test_a_quoted_name_is_reported_on_its_line(self):
        text = "row\nlands with the \"Images\" Story (`ambre-roadmap.md` E7)\n"
        self.assertEqual([(2, "Images")], list(spec_refs.quoted_stories(text)))

    def test_a_name_wrapped_onto_the_next_line_is_seen(self):
        text = "Answered by the \"A board\n  costs what it shows\" Story."
        self.assertEqual([(1, "A board\n  costs what it shows")],
                         list(spec_refs.quoted_stories(text)))

    def test_a_backticked_name_is_not_reported(self):
        text = "lands with the `Images` Story (`ambre-roadmap.md` E7)"
        self.assertEqual([], list(spec_refs.quoted_stories(text)))

    def test_an_earlier_quotation_is_not_swallowed(self):
        text = "a \"label\" names the `Images` Story, and the \"PDF export\" Story"
        self.assertEqual([(1, "PDF export")], list(spec_refs.quoted_stories(text)))


class Duplicates(unittest.TestCase):
    """One file name, one document, because a citation carries the name alone."""

    def test_two_documents_of_one_name_are_reported(self):
        """`documents` keys by file name, so one of the two is unreachable and
        which one depends on the walk order."""
        paths = [Path("specs/1-Foundations/ambre-spec-sync.md"),
                 Path("specs/3-Trame/ambre-spec-sync.md")]
        self.assertEqual({"ambre-spec-sync.md": paths}, spec_refs.duplicates(paths))

    def test_one_name_per_document_reports_nothing(self):
        self.assertEqual({}, spec_refs.duplicates([
            Path("specs/1-Foundations/ambre-spec-sync.md"),
            Path("specs/3-Trame/ambre-spec-trame-sync.md"),
        ]))

    def test_every_file_name_of_the_corpus_is_claimed_once(self):
        """The one test here that reads the corpus, because the rule is about
        the corpus and a gate that never fires is worth nothing."""
        self.assertEqual({}, spec_refs.duplicates(spec_refs.corpus_files()))

    def test_the_corpus_keeps_one_entry_per_file(self):
        """What the check protects: a collision loses a document silently, and
        this equality is where that shows."""
        self.assertEqual(len(spec_refs.corpus_files()), len(spec_refs.documents()))


def cited(text: str) -> list[tuple[str, bool]]:
    return [(path, ranged) for _, _, path, ranged in spec_refs.code_citations(text)]


class CodePaths(unittest.TestCase):
    """The third namespace: a file of the workspace, named by a document.

    329 of them sit in the corpus and this resolver used to declare each one
    "not a citation", because it is not a document. So a rename left the
    citation pointing at nothing, and the corpus said nothing: `handler.rs`
    became `handler/` and one finding of the audit walked to a file that has
    not existed since.
    """

    def test_a_path_is_seen(self):
        self.assertEqual(
            [("crates/ambre-sync/src/pipeline.rs", False)],
            cited("held in `crates/ambre-sync/src/pipeline.rs` today"),
        )

    def test_a_path_outside_backticks_is_not_a_citation(self):
        """The corpus backticks every path it cites. Prose naming a directory
        in passing is prose, and a resolver that reads it reports the sentence
        rather than the citation."""
        self.assertEqual([], cited("the crates/ambre-sync crate"))

    def test_a_line_range_is_cut_from_the_path(self):
        """Both halves are kept: the name resolves, and the range is refused on
        its own. Cutting one loses the other."""
        self.assertEqual(
            [("crates/ambre-server/src/s3.rs", True)],
            cited("`crates/ambre-server/src/s3.rs:66-72`"),
        )

    def test_the_prose_after_a_range_is_cut_too(self):
        found = cited("`crates/ambre-server/src/ws/handler.rs:271-289 (subscribe)`")
        self.assertEqual([("crates/ambre-server/src/ws/handler.rs", True)], found)

    def test_a_comma_range_is_a_range(self):
        self.assertEqual(
            [("crates/ambre-server/src/routes/sync.rs", True)],
            cited("`crates/ambre-server/src/routes/sync.rs:210,237`"),
        )

    def test_a_trailing_slash_is_dropped(self):
        self.assertEqual(
            [("crates/ambre-trame/src/render", False)],
            cited("`crates/ambre-trame/src/render/`"),
        )

    def test_a_file_the_workspace_holds_resolves(self):
        self.assertTrue(spec_refs.code_resolves("crates/ambre-prose/src/lib.rs"))

    def test_a_directory_resolves(self):
        self.assertTrue(spec_refs.code_resolves("crates/ambre-prose"))

    def test_a_file_the_workspace_lost_does_not(self):
        self.assertFalse(spec_refs.code_resolves("crates/ambre-server/src/ws/handler.rs"))

    def test_a_shape_resolves_when_it_matches(self):
        """`crates/**/*.rs` is how the roadmap names every Rust file there is."""
        self.assertTrue(spec_refs.code_resolves("crates/**/*.rs"))

    def test_a_shape_matching_nothing_is_dead(self):
        self.assertFalse(spec_refs.code_resolves("crates/ambre-gone/**/*.rs"))


class Records(unittest.TestCase):
    """A dated record quotes the corpus of the day it was written."""

    def test_the_journal_is_a_record(self):
        self.assertTrue(spec_refs.is_record(spec_refs.SPECS / "ambre-roadmap-journal.md"))

    def test_an_audit_is_a_record_by_directory(self):
        """So the next audit is a record the day it lands, with no script edit."""
        self.assertTrue(spec_refs.is_record(spec_refs.SPECS / "audits" / "security-audit.md"))

    def test_a_spec_is_not_a_record(self):
        self.assertFalse(spec_refs.is_record(spec_refs.SPECS / "1-Foundations" / "ambre-spec-sync.md"))

    def test_a_retired_name_is_carried_in_both_spellings(self):
        """A record cites either one -- finding 62 wrote `canvas section 5.B` --
        so the exemption covers both or it covers the archive by half. The
        short form is what `resolve` claims once the file is gone: it no longer
        sits in `shorts`, so the candidate falls through to `in_span`."""
        self.assertIn("ambre-spec-canvas.md", spec_refs.RETIRED)
        self.assertIn("canvas.md", spec_refs.RETIRED_SHORTS)
        self.assertEqual("canvas.md", claimed("canvas", coordinated=True, in_span=True))


if __name__ == "__main__":
    unittest.main()
