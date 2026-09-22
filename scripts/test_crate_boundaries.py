"""Each check of `crate_boundaries.py`, against a tree that breaches it.

The green run proves nothing on its own: a check that cannot fail holds no
rule. Each case here moves one line of the real tree and asserts the breach is
named.
"""

import unittest
from pathlib import Path

import crate_boundaries as cb


class Tree:
    """The module pointed at a scratch tree, restored on exit."""

    def __init__(self, root: Path):
        self.root = root
        self.was = cb.ROOT

    def __enter__(self):
        cb.ROOT = self.root
        return self

    def __exit__(self, *_):
        cb.ROOT = self.was


def write(root: Path, rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def scratch(tmp: str) -> Path:
    """A minimal tree that passes every check."""
    root = Path(tmp)
    write(root, cb.CLIENT, '[dependencies]\nambre-logic = { path = "../ambre-logic" }\n')
    write(root, cb.CHROME, '[dependencies]\nambre-commands = { path = "../ambre-commands" }\n')
    for rel in cb.FIXTURE_MODULES:
        write(root, rel, "impl S {\n    fn read(&self) { self.doc.get_map(X); }\n}\n\nfn build_a(s: &mut S) {\n    s.handle_command(C);\n}\n")
    return root


class CrateBoundaries(unittest.TestCase):
    def run_checks(self, root: Path) -> list[str]:
        with Tree(root):
            errors: list[str] = []
            cb.check_client(errors)
            cb.check_generators(errors)
            cb.check_chrome(errors)
            return errors

    def test_the_real_tree_is_clean(self):
        self.assertEqual(self.run_checks(cb.ROOT), [])

    def test_a_client_enabling_the_fixture_feature_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, cb.CLIENT, '[dependencies]\nambre-logic = { path = "..", features = ["fixtures"] }\n')
            errors = self.run_checks(root)
        self.assertTrue(any("enables `fixtures`" in e for e in errors), errors)

    def test_a_default_feature_set_folding_in_fixtures_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, "crates/ambre-logic/Cargo.toml", '[features]\ndefault = ["fixtures"]\n')
            errors = self.run_checks(root)
        self.assertTrue(any("`default` feature set" in e for e in errors), errors)

    def test_a_generator_writing_to_the_crdt_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(
                root,
                cb.FIXTURE_MODULES[0],
                "fn build_a(s: &mut S) {\n    s.doc.get_map(CELLS).insert(k, v);\n}\n",
            )
            errors = self.run_checks(root)
        self.assertTrue(any("reaches the CRDT" in e for e in errors), errors)

    def test_the_reading_half_of_a_fixture_module_stays_free(self):
        """`fixture_shape` counts the CRDT it was handed. Only free functions are read."""
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            errors = self.run_checks(root)
        self.assertEqual([e for e in errors if "reaches the CRDT" in e], [])

    def test_a_chrome_that_depends_on_an_app_crate_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, cb.CHROME, '[dependencies]\nambre-bus = { path = "../ambre-bus" }\n')
            errors = self.run_checks(root)
        self.assertTrue(any("depends on `ambre-bus`" in e for e in errors), errors)

    def test_a_missing_client_manifest_is_an_error_not_a_pass(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            (root / cb.CLIENT).unlink()
            errors = self.run_checks(root)
        self.assertTrue(any("gone from the tree" in e for e in errors), errors)


if __name__ == "__main__":
    unittest.main()
