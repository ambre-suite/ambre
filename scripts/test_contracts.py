#!/usr/bin/env python3
"""What `contracts.py` reads, and what it refuses.

This gate holds the row grammar of the whole corpus, and each way it goes wrong
is quiet. A row dropping the backticks around its `Kind` passes a vocabulary
check that compares the stripped word. A new column moves every cell a
position-based read follows. A status renamed under a `Counter` returns 0 and
under-reports the surface it counts.

The pattern is the same each time. The gate answers, the answer is wrong, and
nothing in the repository disagrees with it. These tests are what disagrees.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import contracts  # noqa: E402  -- the module under test
import template_vocabulary  # noqa: E402  -- where the closed sets are stated


WHERE = "ambre-spec-frame.md:12"

# A schedule, in the shape `read_schedule` returns: key -> epic, ticked, line, name.
SCHEDULE = {
    "the ui-thread belts": ("E9", False, 40, "The UI-thread belts"),
    "arch p8": ("E4", False, 51, "ARCH P8 -- persisted schema versioning"),
    "arch p8 -- persisted schema versioning": ("E4", False, 51, "ARCH P8 -- persisted schema versioning"),
    "one clipboard host": ("E7", True, 62, "One clipboard host"),
}
CLOSED = {"the delta order": ("E2", True, 8, "The delta order")}


def lands(cell: str, status: str = "untested") -> list[str]:
    return contracts.story_defects(WHERE, status, cell, SCHEDULE, CLOSED)


class Vocabulary(unittest.TestCase):
    """Every closed set is the template's, read from it exactly once."""

    def test_the_statuses_are_the_templates_own(self):
        self.assertIs(contracts.STATUSES, template_vocabulary.STATUSES)

    def test_loose_and_owes_story_name_declared_statuses(self):
        for status in contracts.LOOSE + contracts.OWES_STORY:
            with self.subTest(status=status):
                self.assertIn(status, template_vocabulary.STATUSES)

    def test_loose_covers_every_built_status_but_verified(self):
        """The summary sums over `LOOSE`, so a status missing from it is a
        silent under-count -- which is precisely what `unverified` was.
        `partial` is counted on its own: a mechanism holds part of it."""
        self.assertEqual(
            set(template_vocabulary.STATUSES) - {"verified", "partial", "unimplemented"},
            set(contracts.LOOSE),
        )

    def test_a_state_kind_and_an_operation_kind_never_overlap(self):
        self.assertEqual(set(), set(contracts.STATE_KINDS) & set(contracts.OPERATION_KINDS))
        self.assertEqual(set(contracts.KINDS), set(contracts.STATE_KINDS) | set(contracts.OPERATION_KINDS))


class Header(unittest.TestCase):
    """A cell is read by the name its own header declares, never by position."""

    def test_the_columns_are_read_in_the_order_the_header_writes_them(self):
        header = "| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |"
        self.assertEqual("Gap", contracts.header_cells(header)[5])

    def test_a_reordered_header_moves_the_cell_with_it(self):
        """Pinning a column to its place is what a new column breaks in
        silence: 'the last one before the status' names a different cell."""
        columns = contracts.header_cells("| ID | Kind | Statement | Sources | Gap | Mechanism | Status |")
        rest = " `INV` | The frame MUST land. | derived | none | `insta` | verified |"
        cells = contracts.row_cells(columns, rest)
        self.assertEqual("`insta`", cells["Mechanism"])
        self.assertEqual("none", cells["Gap"])

    def test_a_table_with_no_header_yields_no_cells(self):
        self.assertEqual({}, contracts.row_cells(None, " `INV` | x |"))


class Cells(unittest.TestCase):
    """The three rules one row's own cells obey."""

    def defects(self, kind="`INV`", statement="The frame MUST land.", sources="derived"):
        return contracts.row_defects(WHERE, kind, statement, sources)

    def test_a_well_formed_row_passes(self):
        self.assertEqual([], self.defects())

    def test_a_kind_without_backticks_is_half_a_column(self):
        """519 rows dropped them and passed: a vocabulary check that forgives
        the spelling checks half a column."""
        self.assertIn("written in backticks", self.defects(kind="INV")[0])

    def test_a_kind_off_the_eight_is_refused(self):
        self.assertIn("none of the eight", self.defects(kind="`RULE`")[0])

    def test_a_statement_with_no_keyword_describes_instead_of_binding(self):
        self.assertIn("no RFC 2119 keyword", self.defects(statement="The frame lands.")[0])

    def test_every_declared_keyword_binds(self):
        for keyword in ("MUST", "MUST NOT", "SHOULD", "SHOULD NOT", "MAY"):
            with self.subTest(keyword=keyword):
                self.assertEqual([], self.defects(statement=f"The frame {keyword} land."))

    def test_a_source_nobody_can_resolve_is_refused(self):
        """`pillar "Local-First"` names a pillar no reader can grep for."""
        self.assertIn(
            "no form of the grammar accepts",
            self.defects(sources='pillar "Local-First"')[0],
        )

    def test_every_declared_source_form_resolves(self):
        for form in (
            "this spec section 2",
            'README pillar "Security"',
            "audit `H9`",
            "derived",
            "`ZK-001`",
            "`ambre-spec-sync.md`",
            "`1-Foundations/ambre-spec-message-bus.md section 4`",
        ):
            with self.subTest(form=form):
                self.assertEqual([], self.defects(sources=form))


class Conjunction(unittest.TestCase):
    """Two rules under one status: the half nothing tests breaks in green."""

    def test_a_second_keyword_behind_an_and_is_two_rules(self):
        statement = "The frame MUST paint above the content, and MUST follow the gesture."
        self.assertIsNotNone(contracts.CONJUNCTION.search(statement))

    def test_one_rule_over_two_clauses_is_one_rule(self):
        statement = "The frame MUST paint above the content and follow the gesture."
        self.assertIsNone(contracts.CONJUNCTION.search(statement))

    def test_a_sentence_break_ends_the_reach(self):
        """The measure stops at `.`, `;` and `:`, so a row stating one rule and
        then explaining it is not read as two."""
        statement = "The frame MUST paint above the content. The sink MUST follow the gesture."
        self.assertIsNone(contracts.CONJUNCTION.search(statement))

    def test_the_kinds_that_carry_a_list_by_right_are_named(self):
        """A `FAIL` binds the refusal, the notice and the rollback as one
        outcome, and a `FRAME` names everything an operation must not touch."""
        self.assertEqual(("FAIL", "FRAME"), contracts.BUNDLING_KINDS)


class PerApplication(unittest.TestCase):
    """A rule each application must hold names a test in each one.

    `KINETIC-002` read `verified` on one test of the shared crate while Toile
    broke it: the shared code was proven, never that an application called it.
    """

    APPS = ["Prose", "Trame", "Toile"]
    SHARED = "`ambre_draw::kinetic::tests::a_glide`"
    TRAME = "Trame `ambre_trame::t::a`"
    TOILE = "Toile `ambre_toile::t::b`"
    PROSE = "Prose `ambre_prose::t::c`"

    def defects(self, cell, status="verified", apps=APPS):
        return contracts.per_application_defects(WHERE, cell, status, apps)

    def test_the_shared_codes_own_rule_passes_with_no_label(self):
        self.assertEqual([], self.defects(self.SHARED))

    def test_a_test_in_each_application_is_verified(self):
        self.assertEqual([], self.defects(f"{self.TRAME}; {self.TOILE}; {self.PROSE}"))

    def test_a_missing_application_cannot_be_verified(self):
        found = self.defects(f"{self.SHARED}; {self.TRAME}; {self.TOILE}")
        self.assertEqual(1, len(found))
        self.assertIn("no test in Prose", found[0])

    def test_a_missing_application_may_stay_untested(self):
        self.assertEqual([], self.defects(self.TRAME, status="untested"))

    def test_chrome_is_no_application(self):
        cell = f"{self.TRAME}; {self.TOILE}; {self.PROSE}; chrome `ambre_trame::c::d`"
        self.assertEqual([], self.defects(cell))

    def test_a_label_needs_the_header_field(self):
        self.assertIn("declares no", self.defects(self.TRAME, apps=None)[0])

    def test_a_label_the_field_leaves_out_is_refused(self):
        found = self.defects(f"{self.TRAME}; {self.TOILE}", apps=["Trame"])
        self.assertIn("does not declare", found[0])

    def test_the_header_field_is_read_and_checked(self):
        self.assertEqual(
            (["Prose", "Trame"], []),
            contracts.declared_applications("**Applications:** Prose, Trame"),
        )
        self.assertEqual(
            (["Prose", "Murmure"], ["Murmure"]),
            contracts.declared_applications("**Applications:** Prose, Murmure"),
        )
        self.assertIsNone(contracts.declared_applications("**Type:** Horizontal"))


class Mechanism(unittest.TestCase):
    """A total read: the cell is a mechanism, and nothing else."""

    def defects(self, cell):
        return contracts.mechanism_defects(WHERE, cell)

    def test_an_empty_cell_is_refused(self):
        self.assertIn("write `none`", self.defects("")[0])

    def test_a_token_of_the_closed_set_passes(self):
        self.assertEqual([], self.defects("`insta`; `proptest`"))

    def test_prose_fails_even_when_it_is_true(self):
        """What it says belongs in `Gap`, where a reader and a script can both
        find it."""
        self.assertIn("no mechanism", self.defects("a reviewer checks the margin")[0])

    def test_a_rust_path_is_a_belt(self):
        self.assertEqual([], self.defects("`ambre_toile::state::insert`"))

    def test_the_path_continuation_carries_the_module_before_it(self):
        self.assertEqual([], self.defects("`ambre_toile::state::insert`; `::remove`"))

    def test_an_application_may_precede_the_path(self):
        self.assertEqual([], self.defects("Toile `ambre_toile::state::insert`"))

    def test_a_token_naming_no_mechanism_takes_no_clause(self):
        """What follows `none` is an argument, and an argument belongs in
        `Gap`."""
        self.assertIn("names no", self.defects("none -- the belt is not written")[0])

    def test_a_token_naming_a_shape_a_reader_cannot_grep_must_locate_it(self):
        self.assertIn("on its own", self.defects("type-level")[0])

    def test_a_located_token_with_its_clause_passes(self):
        self.assertEqual([], self.defects("type-level -- the handle is `!Send`"))

    def test_every_item_of_the_list_is_read(self):
        """A cell a gate reads half of is a cell nothing holds."""
        self.assertEqual(1, len(self.defects("`insta`; a reviewer checks the rest")))


class Gap(unittest.TestCase):
    """What the mechanism leaves open, and the two rules across the columns."""

    def defects(self, cell, mechanism="`insta`", status="untested"):
        return contracts.gap_defects(WHERE, cell, mechanism, status)

    def test_an_opener_off_the_closed_set_is_refused(self):
        self.assertIn("`Gap` opens on", self.defects("nothing holds it yet")[0])

    def test_every_declared_opener_is_accepted(self):
        for opener in contracts.BARE_GAP_OPENERS:
            with self.subTest(opener=opener):
                self.assertEqual([], self.defects(opener))
        for opener in contracts.CARRIED_GAP_OPENERS:
            status = "partial" if opener == "partial -- " else "untested"
            with self.subTest(opener=opener):
                self.assertEqual(
                    [], self.defects(f"{opener}the belt is not written", status=status)
                )

    def test_a_rule_half_held_is_partial_never_untested(self):
        """`untested` beside a half-held rule reads as a row nothing tests, and
        the ledger could not tell the two apart."""
        self.assertIn("is `partial`", self.defects("partial -- the wiring is loose")[0])
        self.assertEqual([], self.defects("partial -- the wiring is loose", status="partial"))

    def test_partial_names_its_mechanism_and_the_half_left(self):
        self.assertIn(
            "names the mechanism",
            self.defects("partial -- half", mechanism="none", status="partial")[0],
        )
        self.assertIn(
            "names the mechanism", self.defects("unheld -- nothing", status="partial")[0]
        )

    def test_no_mechanism_and_no_gap_is_a_coverage_the_row_does_not_have(self):
        self.assertIn("claims a coverage", self.defects("none", mechanism="none")[0])

    def test_verified_beside_an_open_gap_is_two_answers_to_one_question(self):
        found = self.defects("partial -- the belt holds half", status="verified")
        self.assertIn("two answers", found[0])

    def test_verified_beside_a_closed_gap_passes(self):
        self.assertEqual([], self.defects("none", status="verified"))


class Stories(unittest.TestCase):
    """A row that owes a Story lands with an open story of the roadmap."""

    def test_a_row_owing_a_story_and_citing_none_is_refused(self):
        self.assertIn("without a Story", lands("unheld -- the belt is not written")[0])

    def test_a_story_of_the_roadmap_with_its_epic_passes(self):
        self.assertEqual([], lands(
            "unheld -- lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9)"
        ))

    def test_a_phase_code_is_a_citation_key_of_its_own(self):
        """`ARCH P8 -- persisted schema versioning` is cited 13 times as
        `ARCH P8`, which section 1 of the roadmap makes a key."""
        self.assertEqual([], lands(
            "unheld -- lands with the `ARCH P8` Story (`ambre-roadmap.md` E4)"
        ))

    def test_a_story_no_row_of_the_roadmap_carries_is_refused(self):
        self.assertIn("no row of", lands(
            "unheld -- lands with the `The margin belts` Story (`ambre-roadmap.md` E9)"
        )[0])

    def test_a_story_that_shipped_cannot_be_waited_on(self):
        self.assertIn("closed under epic", lands(
            "unheld -- lands with the `The delta order` Story (`ambre-roadmap.md` E2)"
        )[0])

    def test_a_ticked_story_stays_open_until_every_row_is_verified(self):
        self.assertIn("ticks `[x]`", lands(
            "unheld -- lands with the `One clipboard host` Story (`ambre-roadmap.md` E7)"
        )[0])

    def test_a_citation_naming_no_epic_is_refused(self):
        """So a move of the story fails here."""
        self.assertIn("names no epic", lands(
            "unheld -- lands with the `The UI-thread belts` Story"
        )[0])

    def test_the_wrong_epic_is_refused(self):
        """`One clipboard host` was cited under `E6` by three rows while the
        story lived under `E7`."""
        self.assertIn("cites epic", lands(
            "unheld -- lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E4)"
        )[0])

    def test_a_note_after_the_citation_is_not_part_of_the_name(self):
        self.assertEqual([], lands(
            "unheld -- lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9);"
            " the belt is three tests"
        ))


class StoryKeys(unittest.TestCase):
    """A story name is a join key, and a key that forgives a word joins wrong."""

    def test_the_delimiters_a_citation_adds_are_stripped(self):
        self.assertEqual(contracts.story_key("`One clipboard host`"), contracts.story_key("One clipboard host"))

    def test_the_case_and_the_spacing_are_folded(self):
        self.assertEqual("one clipboard host", contracts.story_key("One   Clipboard  Host"))

    def test_a_word_is_never_forgiven(self):
        self.assertNotEqual(contracts.story_key("One clipboard host"), contracts.story_key("One clipboard"))

    def test_a_phase_code_registers_as_a_key_of_its_own(self):
        self.assertEqual(
            ["arch p8 -- persisted schema versioning", "arch p8"],
            contracts.story_keys("ARCH P8 -- persisted schema versioning"),
        )

    def test_a_name_without_a_code_registers_once(self):
        self.assertEqual(["one clipboard host"], contracts.story_keys("One clipboard host"))


class Counts(unittest.TestCase):
    """The `Verification` answer opens on the counts of the table above it."""

    def test_the_counts_are_read_off_the_head_of_the_bullet(self):
        self.assertEqual(
            {"verified": 7, "untested": 1, "unimplemented": 1},
            contracts.stated_counts("7 `verified`, 1 `untested`, and 1 `unimplemented`."),
        )

    def test_a_figure_after_the_counts_is_a_review_act_not_a_parse(self):
        """"two of the four are rules the code answers with the opposite" is
        prose, and it carries figures of its own."""
        self.assertEqual(
            {"verified": 4},
            contracts.stated_counts("4 `verified`. Two of the four are held by 1 belt."),
        )

    def test_a_count_cut_at_the_end_of_a_line_is_still_read(self):
        """The bullet wraps, and the gate once read its first line alone."""
        self.assertTrue(contracts.continues_bullet("  `untested`."))
        self.assertFalse(contracts.continues_bullet("   "))
        self.assertFalse(contracts.continues_bullet("* next"))
        self.assertEqual(
            {"verified": 1, "untested": 1},
            contracts.stated_counts("1 `verified`, 1 `untested`."),
        )

    def test_the_counts_are_written_from_the_table(self):
        self.assertEqual(
            "2 `verified`, 1 `failing`",
            contracts.render_counts(Counter({"failing": 1, "verified": 2})),
        )

    def test_a_rewrite_replaces_the_counts_and_keeps_the_prose(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            spec = Path(tmp) / "spec.md"
            spec.write_text("x\n* **Verification.** 9 `verified`, 1\n  `untested`. The gap is X.\n* next\n")
            self.assertTrue(contracts.rewrite_answer(spec, 2, Counter({"verified": 3})))
            self.assertEqual(
                "x\n* **Verification.** 3 `verified`. The gap is X.\n* next\n", spec.read_text()
            )

    def test_an_answer_opening_on_no_count_is_refused(self):
        found = contracts.answer_defects("spec.md", (9, "The belt holds it."), Counter({"verified": 1}))
        self.assertIn("states no count", found[0])

    def test_an_answer_that_disagrees_with_its_table_is_refused(self):
        found = contracts.answer_defects("spec.md", (9, "2 `verified`."), Counter({"verified": 3}))
        self.assertIn("says 2 `verified` and the table holds 3", found[0])

    def test_an_answer_counting_a_status_off_the_six_is_refused(self):
        found = contracts.answer_defects("spec.md", (9, "2 `unverified`."), Counter({"verified": 2}))
        self.assertIn("no status of the seven", found[0])

    def test_a_missing_answer_belongs_to_the_other_gate(self):
        """One rule, one home: `spec_shape.py` holds whether the label exists."""
        self.assertEqual([], contracts.answer_defects("spec.md", None, Counter({"verified": 1})))

    def test_an_answer_that_matches_its_table_passes(self):
        found = contracts.answer_defects("spec.md", (9, "3 `verified`, 1 `untested`."),
                                         Counter({"verified": 3, "untested": 1}))
        self.assertEqual([], found)


class Tables(unittest.TestCase):
    """Two rules about the table as a whole."""

    def table(self, *kinds: str) -> list[tuple[int, str, str]]:
        return [(10 + i, f"SPEC-{i:03}", kind) for i, kind in enumerate(kinds)]

    def test_a_table_mixing_the_two_families_is_the_layout_collapsing(self):
        """One list again, which is what hid the frame conditions."""
        found = contracts.table_defects("spec.md", self.table("INV", "POST", "FRAME"))
        self.assertIn("operation carries a table of its own", found[0])

    def test_a_post_with_no_frame_names_what_it_writes_and_not_what_it_spares(self):
        found = contracts.table_defects("spec.md", self.table("PRE", "POST"))
        self.assertIn("declares a `POST` and no `FRAME`", found[0])

    def test_a_complete_operation_table_passes(self):
        self.assertEqual([], contracts.table_defects("spec.md", self.table("PRE", "POST", "FRAME", "FAIL")))

    def test_a_state_table_passes(self):
        self.assertEqual([], contracts.table_defects("spec.md", self.table("INV", "CONC", "OBS", "IO")))

    def test_an_unknown_kind_is_reported_once_by_the_cell_check(self):
        """A typo must not also read as a family clash."""
        self.assertEqual([], contracts.table_defects("spec.md", self.table("RULE")))


class IdRanges(unittest.TestCase):
    """The index that joins an id to the file declaring it."""

    def test_adjacent_numbers_fold_into_one_run(self):
        self.assertEqual("TRAME-151..154", contracts.id_ranges(
            ["TRAME-151", "TRAME-152", "TRAME-153", "TRAME-154"]))

    def test_a_hole_opens_a_second_run(self):
        self.assertEqual("TRAME-017,121", contracts.id_ranges(["TRAME-121", "TRAME-017"]))

    def test_the_three_digits_are_kept(self):
        """An id is written `TRAME-017` everywhere else, and a run that printed
        `17` would be a second spelling of the same key."""
        self.assertEqual("TRAME-017", contracts.id_ranges(["TRAME-017"]))

    def test_two_prefixes_are_two_cells(self):
        """One document declares from its application's family and from the
        sub-families beside it, and the runs never cross a prefix."""
        self.assertEqual(
            "TRAME-019..020; TRAME-UI-009",
            contracts.id_ranges(["TRAME-019", "TRAME-UI-009", "TRAME-020"]),
        )

    def test_a_repeated_id_is_listed_once(self):
        self.assertEqual("ARCH-001", contracts.id_ranges(["ARCH-001", "ARCH-001"]))


DECLARED = {
    "ZK-001": "1-Foundations/ambre-spec-zero-knowledge.md",
    "SYNC-050": "1-Foundations/ambre-spec-media.md",
    "UI-THREAD-001": "1-Foundations/ambre-spec-ui.md",
    "UI-THREAD-050": "1-Foundations/ambre-spec-ui.md",
}


def cites(cell: str) -> list[str]:
    return contracts.source_defects(WHERE, cell, DECLARED)


class SourceIds(unittest.TestCase):
    """The rule a `Sources` cell cites, and whether it still exists.

    The corpus carries 174 such citations and nothing resolved one. A row is
    deleted or its id is renumbered, and every citation of it points at nothing
    while the cell still reads like a join.
    """

    def test_an_id_no_document_declares_fails(self):
        self.assertEqual(1, len(cites("this spec section 3; `ZK-999`")))

    def test_a_declared_id_passes_bare(self):
        """An id is unique in the corpus, so a bare one resolves through the
        ledger index without naming its file."""
        self.assertEqual([], cites("this spec section 3; `ZK-001`"))

    def test_the_named_document_must_declare_the_id(self):
        defects = cites("`ambre-spec-server.md`, `ZK-001`")
        self.assertEqual(1, len(defects))
        self.assertIn("ambre-spec-zero-knowledge.md", defects[0])

    def test_the_right_document_passes(self):
        self.assertEqual([], cites("`ambre-spec-zero-knowledge.md`, `ZK-001`"))

    def test_a_path_matches_the_bare_name(self):
        """The corpus writes both `ambre-spec-media.md` and the path to it, and
        the two name one file."""
        self.assertEqual([], cites("`1-Foundations/ambre-spec-media.md`, `SYNC-050`"))

    def test_each_source_pairs_with_its_own_document(self):
        """One cell lists several sources. Pairing an id with the document of
        the source beside it reads a true citation as a lie, which is what a
        whole-cell read did to five rows of the corpus."""
        self.assertEqual([], cites("`ZK-001`; `ambre-spec-media.md`, `SYNC-050`"))

    def test_two_ids_under_one_document_both_resolve(self):
        self.assertEqual(
            [], cites("`ambre-spec-ui.md section 2` (`UI-THREAD-001`, `UI-THREAD-050`)")
        )

    def test_a_cell_citing_no_id_is_silent(self):
        self.assertEqual([], cites('README pillar "Zero-Knowledge"; audit `H9`'))


FIELDS_ANCHOR = "**The seven fields.**"


class SourceGrammar(unittest.TestCase):
    """What a `Sources` cell may say, form by form.

    What this replaced accepted a cell when any one of its sources matched any
    one of five loose patterns, anywhere inside it. A bare `` `ZK-001` `` rode
    on the section citation beside it 95 times and no gate read it once.
    """

    def parsed(self, source: str) -> contracts.Source:
        found = contracts.parse_source(source)
        self.assertIsNotNone(found, f"the grammar refused {source!r}")
        return found

    def test_derived_names_nothing_outside_the_row(self):
        found = self.parsed("derived")
        self.assertEqual(("derived", "", ""), (found.kind, found.target, found.locator))

    def test_a_pillar_carries_the_pillar_it_names(self):
        found = self.parsed('README pillar "Zero-Knowledge"')
        self.assertEqual(("pillar", "Zero-Knowledge"), (found.kind, found.target))

    def test_an_audit_finding_carries_its_number(self):
        found = self.parsed("audit `H9`")
        self.assertEqual(("audit", "`H9`"), (found.kind, found.target))

    def test_several_findings_ride_one_source(self):
        self.assertEqual("audit", self.parsed("audit `H9`, `H12`").kind)

    def test_a_bare_id_is_a_source_of_its_own(self):
        """The form the corpus writes most after a section, and the one the
        shape list never declared."""
        found = self.parsed("`ZK-001`")
        self.assertEqual(("rule", ("ZK-001",)), (found.kind, found.rules))

    def test_a_document_carries_its_section_inside_the_ticks(self):
        found = self.parsed("`ambre-spec-ui.md section 2`")
        self.assertEqual(
            ("doc", "ambre-spec-ui.md", "2"), (found.kind, found.target, found.locator)
        )

    def test_a_document_carries_its_section_outside_the_ticks(self):
        found = self.parsed("`ambre-spec-ui.md` section 2")
        self.assertEqual(
            ("doc", "ambre-spec-ui.md", "2"), (found.kind, found.target, found.locator)
        )

    def test_a_path_is_a_document_name(self):
        found = self.parsed("`1-Foundations/ambre-spec-media.md`")
        self.assertEqual("1-Foundations/ambre-spec-media.md", found.target)

    def test_a_short_name_is_a_citation_and_not_prose(self):
        """`spec_refs.py` resolves the short name, and reads the number of the
        section and no more. Twenty sources write it, and the shape list read
        every one of them as a section of the citing document."""
        found = self.parsed("architecture section 2.D")
        self.assertEqual(
            ("doc", "architecture", "2.D"), (found.kind, found.target, found.locator)
        )

    def test_this_spec_locates_a_section_of_the_citing_document(self):
        found = self.parsed("this spec section 3")
        self.assertEqual(("own", "", "3"), (found.kind, found.target, found.locator))

    def test_the_section_stands_without_this_spec(self):
        self.assertEqual("own", self.parsed("section 3").kind)

    def test_a_parenthesis_holds_the_rules_a_document_cites(self):
        found = self.parsed("`ambre-spec-ui.md section 2` (`UI-THREAD-001`, `UI-THREAD-050`)")
        self.assertEqual(("UI-THREAD-001", "UI-THREAD-050"), found.rules)

    def test_an_ids_only_clause_is_the_citation_and_not_a_note(self):
        """56 sources name the rule they point at this way, and reading that
        clause as prose loses every one of them."""
        found = self.parsed("`ambre-spec-zero-knowledge.md`, `ZK-002`")
        self.assertEqual((("ZK-002",), ""), (found.rules, found.note))

    def test_a_clause_that_is_prose_stays_a_note(self):
        found = self.parsed("`ambre-spec-ui.md section 2`, the token table")
        self.assertEqual(((), "the token table"), (found.rules, found.note))

    def test_an_id_the_note_mentions_in_passing_is_prose(self):
        """A clause is the citation when it is ids and nothing else. Anything
        around them is a sentence, and a sentence cites nothing."""
        found = self.parsed("`ambre-spec-ui.md`, as `ZK-001` requires")
        self.assertEqual((), found.rules)

    def test_a_source_that_names_nothing_is_refused(self):
        self.assertIsNone(contracts.parse_source("the design"))

    def test_a_document_named_in_prose_is_refused(self):
        self.assertIsNone(contracts.parse_source("see ambre-spec-ui.md"))

    def test_a_section_without_a_number_is_refused(self):
        self.assertIsNone(contracts.parse_source("this spec section"))

    def test_each_source_of_a_cell_is_read_on_its_own(self):
        """The looseness this replaced: one entry matched and the cell passed,
        so the entry beside it was never read."""
        defects = contracts.grammar_defects(WHERE, "this spec section 3; the design")
        self.assertEqual(1, len(defects))
        self.assertIn("'the design'", defects[0])

    def test_a_cell_the_grammar_reads_whole_is_silent(self):
        self.assertEqual(
            [], contracts.grammar_defects(WHERE, "this spec section 3; `ZK-001`; derived")
        )

    def test_a_bare_id_names_no_document(self):
        self.assertEqual([("ZK-001", [])], contracts.sources_cited("`ZK-001`"))

    def test_an_id_pairs_with_the_document_of_its_own_source(self):
        self.assertEqual(
            [("ZK-001", []), ("SYNC-050", ["ambre-spec-media.md"])],
            contracts.sources_cited("`ZK-001`; `ambre-spec-media.md`, `SYNC-050`"),
        )

    def test_a_source_the_grammar_refuses_cites_nothing(self):
        """One defect per source: the refusal names it, and the citation check
        does not name it a second time."""
        self.assertEqual([], contracts.sources_cited("the design"))

    def test_the_template_announces_one_form_per_kind(self):
        """The template is what a contributor reads, and `SOURCE_KINDS` is what
        the gate holds. A form declared in one and missing from the other is
        the drift the shape list carried for the bare id."""
        fields = {
            template_vocabulary._token(cells[0], FIELDS_ANCHOR): cells[1]
            for cells in template_vocabulary._counted(FIELDS_ANCHOR)
        }
        word = template_vocabulary.ANNOUNCED_COUNT.search(fields["Sources"]).group(1)
        self.assertEqual(len(contracts.SOURCE_KINDS), template_vocabulary.NUMBERS[word])


class SplitCandidates(unittest.TestCase):
    """The second keyword the refusal does not reach.

    `CONJUNCTION` asks for the keyword right after the `and`, so four words of
    subject hide 40 rows from it. This measure allows the subject and reports
    what it finds, because the reading is what decides each row.
    """

    def test_a_subject_between_the_and_and_the_keyword_is_reached(self):
        self.assertTrue(contracts.SPLIT_CANDIDATE.search(
            "A departure MUST drop the entry, and the others MUST be told."))

    def test_the_refusal_does_not_reach_it(self):
        """The reason the report exists: this row passes the gate today."""
        self.assertIsNone(contracts.CONJUNCTION.search(
            "A departure MUST drop the entry, and the others MUST be told."))

    def test_a_noun_list_after_the_keyword_is_left_alone(self):
        self.assertIsNone(contracts.SPLIT_CANDIDATE.search(
            "A command MUST carry its label, its icon and its chord."))

    def test_a_second_sentence_is_left_alone(self):
        """A full stop ends the rule. What follows is a second sentence, and the
        row grammar reads sentences, never paragraphs."""
        self.assertIsNone(contracts.SPLIT_CANDIDATE.search(
            "The menu MUST close. A reopened menu MUST start empty."))

    def test_the_report_is_empty_when_nothing_matches(self):
        self.assertEqual([], contracts.split_report([]))

    def test_the_report_counts_the_rows_it_prints(self):
        lines = contracts.split_report([(WHERE, "A MUST b, and c MUST d.")])
        self.assertIn("1 rows to read", lines[1])
        self.assertIn(WHERE, lines[2])


if __name__ == "__main__":
    unittest.main()
