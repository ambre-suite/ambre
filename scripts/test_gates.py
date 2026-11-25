#!/usr/bin/env python3
"""Each gate under `scripts/` fails on a breach with no flag, and each generator writes only with `--write`.

A gate that passes unless it is given `--check` passes on a red tree when a
person or a script runs it by hand. Three defects of `spec-code` reached main
that way. So each gate below runs once, with no flag, on one breach planted
through the seam its `main` reads, and must exit non-zero. Each generator runs
once, with no flag, on a stale target, and must exit non-zero and leave the
target as it was.

`SCRIPTS` names every script of the directory under one role. A new script that
no role names fails here, so a new gate cannot skip the rule.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import io
import os
import subprocess
import sys
import tempfile
import unittest
from contextlib import ExitStack, redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import ceiling_raise  # noqa: E402
import checkout_credentials  # noqa: E402
import client_digest  # noqa: E402
import commit_msg  # noqa: E402
import contract_index  # noqa: E402
import contracts  # noqa: E402
import converter_boundary  # noqa: E402
import crate_boundaries  # noqa: E402
import chart  # noqa: E402
import crate_table  # noqa: E402
import feature_inventory  # noqa: E402
import fixture_provenance  # noqa: E402
import image_provenance  # noqa: E402
import freeze_provenance  # noqa: E402
import instructions  # noqa: E402
import locale_table  # noqa: E402
import prose  # noqa: E402
import schema_freeze  # noqa: E402
import service_pins  # noqa: E402
import spec_answers  # noqa: E402
import spec_code  # noqa: E402
import spec_refs  # noqa: E402
import spec_shape  # noqa: E402
import toolchain_pin  # noqa: E402
import wasm_size  # noqa: E402
import wasm_stdio  # noqa: E402
from test_wasm_stdio import PRINT, body, module  # noqa: E402
import write_cost  # noqa: E402

GATE = "gate"
GENERATOR = "generator"
TOOL = "tool"

# Every script of `scripts/`, by role. A gate fails on a breach. A generator is
# a gate on its target, and rewrites the target only under `--write`. A tool
# prints or serves a build, and holds no rule.
SCRIPTS = {
    "ceiling_raise": GATE,
    "chart": GATE,
    "checkout_credentials": GATE,
    "ci_cache": TOOL,
    "client_digest": GATE,
    "commit_msg": GATE,
    "contract_index": GATE,
    "contracts": GENERATOR,
    "converter_boundary": GATE,
    "converter_build": TOOL,
    "corpus": TOOL,
    "crate_boundaries": GATE,
    "crate_table": GATE,
    "dev_stack": TOOL,
    "docker_up": TOOL,
    "feature_inventory": GENERATOR,
    "fixture_provenance": GATE,
    "freeze_provenance": GATE,
    "garage_init": TOOL,
    "glossary": TOOL,
    "image_provenance": GATE,
    "instructions": GATE,
    "locale_table": GENERATOR,
    "merge_generated": TOOL,
    "prose": GATE,
    "schema_freeze": GATE,
    "selfhost_wait": TOOL,
    "service_pins": GATE,
    "spec_answers": GATE,
    "spec_code": GATE,
    "spec_context": TOOL,
    "spec_read": TOOL,
    "spec_refs": GATE,
    "spec_shape": GATE,
    "template_vocabulary": TOOL,
    "term_drift": TOOL,
    "toolchain_pin": GATE,
    "trunk_post_build": TOOL,
    "wasm_driver": TOOL,
    "wasm_size": GATE,
    "wasm_stdio": GATE,
    "write_cost": GATE,
}

STALE = "stale\n"

# The three limit tables of `wasm_size.py`, with the Main ceiling left open.
LIMITS = 'CEILINGS = {{"Main": {}}}\nMODULE_BUDGETS = {{}}\nBUDGET = 4_000_000\n'


def quiet(run) -> int:
    with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
        return run()


def bare(script: str, *args: str):
    """`sys.argv` as a run by hand gives it: the script name, and no flag."""
    return mock.patch.object(sys, "argv", [f"{script}.py", *args])


class Roles(unittest.TestCase):
    def test_every_script_has_a_role(self) -> None:
        found = {p.stem for p in HERE.glob("*.py") if not p.name.startswith("test_")}
        self.assertEqual(found - SCRIPTS.keys(), set(), "a script no role names")
        self.assertEqual(SCRIPTS.keys() - found, set(), "a role names no script")

    def test_no_gate_reads_a_check_flag(self) -> None:
        for name, role in SCRIPTS.items():
            if role == TOOL:
                continue
            with self.subTest(script=name):
                self.assertNotIn('"--check"', (HERE / f"{name}.py").read_text())


class GatesFailWithNoFlag(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)

    def fails(self, script: str, patches: list, *args: str, run=None) -> None:
        with ExitStack() as stack:
            stack.enter_context(bare(script, *args))
            for patch in patches:
                stack.enter_context(patch)
            code = quiet(run or sys.modules[script].main)
        self.assertNotEqual(code, 0, f"{script} passed a breach with no flag")

    def test_contract_index(self) -> None:
        # The index and the graph are built on demand, so the gate holds the
        # edges and writes nothing: a run with no flag leaves no file behind.
        out = self.dir / "out"
        self.fails("contract_index", [
            mock.patch.object(contract_index, "INDEX", out / "index.json"),
            mock.patch.object(contract_index, "GRAPH", out / "graph.json"),
            mock.patch.object(contract_index, "build", lambda: ([], [])),
            mock.patch.object(contract_index, "edge_defects",
                              lambda records, found: ["an edge to nothing"]),
        ])
        self.assertFalse(out.exists(), "contract_index wrote with no flag")

    def test_prose(self) -> None:
        self.fails("prose", [
            mock.patch.object(prose, "glossary_breaches", lambda: ["glossary: a breach"]),
            mock.patch.object(prose, "rust_files", lambda: []),
            mock.patch.object(prose, "markdown_files", lambda: []),
        ])

    def test_spec_answers(self) -> None:
        self.fails("spec_answers", [
            mock.patch.object(spec_answers, "measured_files", lambda: [("doc.md", "text\n")]),
            mock.patch.object(spec_answers, "history_lines", lambda text: iter([(1, "history")])),
        ])

    def test_spec_shape(self) -> None:
        (self.dir / "doc.md").write_text("a spec with no title\n")
        self.fails("spec_shape", [mock.patch.object(spec_shape, "SPECS", self.dir)])

    def test_spec_code(self) -> None:
        self.fails("spec_code", [
            mock.patch.object(spec_code, "SPECS", self.dir),
            mock.patch.object(spec_code, "check_citations", lambda: 1),
        ])

    def test_spec_refs(self) -> None:
        root = spec_refs.ROOT
        shared = {"doc.md": [root / "a" / "doc.md", root / "b" / "doc.md"]}
        self.fails("spec_refs", [
            mock.patch.object(spec_refs, "corpus_files", lambda: []),
            mock.patch.object(spec_refs, "duplicates", lambda files: shared),
            mock.patch.object(spec_refs, "documents", lambda: {}),
            mock.patch.object(spec_refs, "short_names", lambda known: {}),
            mock.patch.object(spec_refs, "sources", lambda: []),
        ])

    def test_wasm_stdio(self) -> None:
        dist = self.dir / "dist"
        dist.mkdir()
        wasm = module([body(b""), body(b"\x10\x01")], {1: PRINT, 2: "lyon::log"})
        (dist / "ambre-app-0_bg.wasm").write_bytes(wasm)
        allowlist = self.dir / "stdio.tsv"
        allowlist.write_text("caller\treason\n")
        self.fails("wasm_stdio", [
            mock.patch.object(wasm_stdio, "DIST", dist),
            mock.patch.object(wasm_stdio, "ALLOWLIST", allowlist),
        ])

    def test_crate_boundaries(self) -> None:
        def breach(errors: list[str]) -> int:
            errors.append("a breach")
            return 0

        checks = [n for n in dir(crate_boundaries) if n.startswith("check_")]
        patches = [mock.patch.object(crate_boundaries, n, lambda errors: 0) for n in checks]
        patches.append(mock.patch.object(crate_boundaries, "check_instances", breach))
        self.fails("crate_boundaries", patches)

    def test_crate_table(self) -> None:
        self.fails("crate_table", [
            mock.patch.object(crate_table, "metadata", lambda: {
                "packages": [{"id": "ambre-ghost", "name": "ambre-ghost", "dependencies": []}],
                "workspace_members": ["ambre-ghost"],
            }),
        ])

    def test_converter_boundary(self) -> None:
        self.fails("converter_boundary", [
            mock.patch.object(converter_boundary, "linked", lambda crate, features=None: [crate, "krilla"]),
        ])

    def test_schema_freeze(self) -> None:
        def breach(errors: list[str]) -> int:
            errors.append("a freeze with no belt")
            return 0

        self.fails("schema_freeze", [
            mock.patch.object(schema_freeze, "released", lambda: []),
            mock.patch.object(schema_freeze, "shallow", lambda: False),
            mock.patch.object(schema_freeze, "check_belts", breach),
        ])

    def test_freeze_provenance(self) -> None:
        self.fails("freeze_provenance", [
            mock.patch.object(freeze_provenance, "resolve", lambda root, base: "base"),
            mock.patch.object(freeze_provenance, "frozen_artifacts", lambda root: ["a.bin"]),
            mock.patch.object(freeze_provenance, "blob", lambda root, base, rel: None),
            mock.patch.object(
                freeze_provenance, "check_artifact", lambda root, rel, cmd: (["refused"], [])
            ),
        ], run=lambda: freeze_provenance.main([]))

    def test_instructions(self) -> None:
        ceiling = instructions.Ceiling("crate", "bench", "op", "symbol", "row", 1)
        self.fails("instructions", [
            mock.patch.object(instructions, "CEILINGS", [ceiling]),
            mock.patch.object(instructions, "build", lambda c: "exe"),
            mock.patch.object(instructions, "count", lambda exe, c: 0),
        ])

    def test_write_cost(self) -> None:
        over = write_cost.Figures(4000, write_cost.CEILING + 1.0, 0.0, {})
        self.fails("write_cost", [mock.patch.object(write_cost, "run_once", lambda: over)])

    def test_client_digest(self) -> None:
        first, second = self.dir / "first", self.dir / "second"
        first.write_text("0" * 64 + "  index.html\n")
        second.write_text("1" * 64 + "  index.html\n")
        self.fails("client_digest", [], run=lambda: client_digest.main(["compare", str(first), str(second)]))

    def test_wasm_size(self) -> None:
        wasm = self.dir / "a.wasm.br"
        wasm.write_bytes(b"x")
        self.fails("wasm_size", [
            mock.patch.object(wasm_size, "find", lambda dist, pattern: wasm),
            mock.patch.object(wasm_size, "CEILINGS", {name: 0 for name in wasm_size.GLOBS}),
        ])

    def test_ceiling_raise(self) -> None:
        self.fails("ceiling_raise", [
            mock.patch.object(ceiling_raise, "resolve", lambda base: "HEAD"),
            mock.patch.object(ceiling_raise, "at", lambda rev, path: LIMITS.format(1)),
            mock.patch.object(ceiling_raise, "current", lambda: LIMITS.format(2)),
            mock.patch.object(ceiling_raise, "plans", lambda: [""]),
        ])

    def test_toolchain_pin(self) -> None:
        self.fails("toolchain_pin", [
            mock.patch.object(toolchain_pin, "pinned", lambda: 'channel = "nightly-2026-03-29"\n'),
            mock.patch.object(toolchain_pin, "files", lambda: [("fuzz/rust-toolchain.toml", "")]),
        ])

    def test_service_pins(self) -> None:
        self.fails("service_pins", [
            mock.patch.object(
                service_pins, "read",
                lambda path: "services:\n  redis:\n    image: redis:8-alpine\n",
            ),
        ])

    def test_chart(self) -> None:
        # A render with no SFU breaks `CALL-141`; Helm itself never runs.
        lint = subprocess.CompletedProcess([], 0, "", "")
        self.fails("chart", [
            mock.patch.object(chart.subprocess, "run", lambda *a, **k: lint),
            mock.patch.object(chart, "render", lambda *args: []),
            mock.patch.object(chart, "compose_images", lambda: {}),
        ], run=lambda: chart.main(["check"]))

    def test_image_provenance(self) -> None:
        # A tree with no release workflow attests nothing and verifies nothing.
        self.fails("image_provenance", [
            mock.patch.object(image_provenance, "WORKFLOWS", self.dir),
        ], run=lambda: image_provenance.main(["check"]))

    def test_checkout_credentials(self) -> None:
        self.fails("checkout_credentials", [
            mock.patch.object(
                checkout_credentials, "workflows",
                lambda: {"ci.yml": "jobs:\n  a:\n    steps:\n      - uses: actions/checkout@v7\n"},
            ),
        ])

    def test_fixture_provenance(self) -> None:
        # The tree holds every input, so a register emptied of its rows leaves
        # each input with no provenance line.
        real = fixture_provenance.read
        self.fails("fixture_provenance", [
            mock.patch.object(
                fixture_provenance, "read",
                lambda path: "" if path == fixture_provenance.AUTHORED else real(path),
            ),
        ])

    def test_commit_msg(self) -> None:
        message = self.dir / "COMMIT_EDITMSG"
        message.write_text("Ends on a period.\n")
        self.fails("commit_msg", [], str(message))
        message.write_text("Merge branch 'story' into main\n")
        self.fails("commit_msg", [], str(message))

    def test_commit_msg_revs(self) -> None:
        # The CI mode reads each commit of a range, the merge git words included.
        repo = self.dir / "repo"
        env = {**os.environ, "GIT_AUTHOR_NAME": "a", "GIT_AUTHOR_EMAIL": "a@a",
               "GIT_COMMITTER_NAME": "a", "GIT_COMMITTER_EMAIL": "a@a",
               "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}

        def git(*args: str) -> None:
            subprocess.run(["git", "-C", str(repo), *args], check=True,
                           capture_output=True, env=env)

        repo.mkdir()
        git("init", "-q")
        git("commit", "-q", "--allow-empty", "-m", "Old subject, before the rule")
        git("tag", "base")
        git("commit", "-q", "--allow-empty", "-m", "fix(sync): close the socket")
        with mock.patch.object(
                commit_msg, "git",
                lambda *a: subprocess.run(["git", "-C", str(repo), *a], check=True,
                                          capture_output=True, text=True).stdout):
            self.assertEqual(commit_msg.revs_defects(["HEAD", "^base"]), [])
            git("commit", "-q", "--allow-empty", "-m", "Merge branch 'x' into main")
            self.assertEqual(len(commit_msg.revs_defects(["HEAD", "^base"])), 1)


class GeneratorsWriteOnlyWithWrite(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)

    def stale(self, name: str) -> Path:
        path = self.dir / name
        path.write_text(STALE)
        return path

    def holds(self, script: str, targets: list[Path], patches: list, run=None) -> None:
        with ExitStack() as stack:
            stack.enter_context(bare(script))
            for patch in patches:
                stack.enter_context(patch)
            code = quiet(run or sys.modules[script].main)
        self.assertNotEqual(code, 0, f"{script} passed a stale target with no flag")
        for target in targets:
            self.assertEqual(target.read_text(), STALE, f"{script} wrote with no flag")

    def test_contracts(self) -> None:
        ledger = self.stale("ledger.md")
        self.holds("contracts", [ledger], [mock.patch.object(contracts, "LEDGER", str(ledger))])

    def inventory(self) -> list:
        app = self.dir / "app"
        app.mkdir()
        feature = {"rows": 0, "loose": 0}
        return [
            mock.patch.object(feature_inventory, "ROOT", self.dir),
            mock.patch.object(feature_inventory, "layers", lambda: [(app, "app", ["doc.md"])]),
            mock.patch.object(feature_inventory, "read", lambda path: feature),
            mock.patch.object(feature_inventory, "defects", lambda slug, features: []),
            mock.patch.object(feature_inventory, "render", lambda slug, features: "fresh\n"),
        ]

    def test_feature_inventory(self) -> None:
        patches = self.inventory()
        target = self.dir / "app" / "ambre-app-inventory.md"
        target.write_text(STALE)
        self.holds("feature_inventory", [target], patches)

    def test_feature_inventory_writes_under_write(self) -> None:
        patches = self.inventory()
        target = self.dir / "app" / "ambre-app-inventory.md"
        target.write_text(STALE)
        with ExitStack() as stack:
            stack.enter_context(bare("feature_inventory", "--write"))
            for patch in patches:
                stack.enter_context(patch)
            self.assertEqual(quiet(feature_inventory.main), 0)
        self.assertEqual(target.read_text(), "fresh\n")

    def test_locale_table(self) -> None:
        table = self.stale("locale_table.rs")
        self.holds("locale_table", [table], [
            mock.patch.object(locale_table, "TABLE", table),
            mock.patch.object(locale_table, "ROOT", self.dir),
        ], run=lambda: locale_table.main([]))

    def test_locale_table_writes_under_write(self) -> None:
        table = self.stale("locale_table.rs")
        with mock.patch.object(locale_table, "TABLE", table):
            self.assertEqual(quiet(lambda: locale_table.main(["--write"])), 0)
        self.assertNotEqual(table.read_text(), STALE)


if __name__ == "__main__":
    unittest.main()
