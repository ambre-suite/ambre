#!/usr/bin/env python3
"""What `client_digest.py` writes into a manifest, and what it refuses.

The manifest is tested on a scratch `dist/`, with a fake brotli decoder, and
against `b2sum` where coreutils installs it. The pin of Trunk is tested against
every workflow that installs Trunk to build the published release.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import hashlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import client_digest as cd  # noqa: E402  -- the module under test


def identity_brotli(path: Path) -> bytes:
    """A decoder for the tests: the `.br` copy holds the bytes as they are."""
    return path.read_bytes()


def dist(tmp: str) -> Path:
    root = Path(tmp) / "dist"
    (root / "assets").mkdir(parents=True)
    (root / "index.html").write_text("<html></html>", encoding="utf-8")
    (root / "main-0123.wasm").write_bytes(b"\0asm")
    (root / "assets" / "font.woff2").write_bytes(b"font")
    return root


class Manifest(unittest.TestCase):
    def test_one_line_per_file_sorted_by_path_in_the_b2sum_form(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            text = cd.manifest(dist(tmp), identity_brotli)
        blake = lambda data: hashlib.blake2b(data, digest_size=32).hexdigest()  # noqa: E731
        self.assertEqual(
            text,
            f"{blake(b'font')}  assets/font.woff2\n"
            f"{blake(b'<html></html>')}  index.html\n"
            f"{blake(b'\0asm')}  main-0123.wasm\n",
        )

    def test_a_brotli_copy_is_left_out_of_the_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = dist(tmp)
            before = cd.manifest(root, identity_brotli)
            (root / "index.html.br").write_bytes((root / "index.html").read_bytes())
            self.assertEqual(cd.manifest(root, identity_brotli), before)

    def test_a_brotli_copy_that_decodes_to_other_bytes_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = dist(tmp)
            (root / "index.html.br").write_bytes(b"<script>steal()</script>")
            with self.assertRaisesRegex(cd.DigestError, "does not decode"):
                cd.manifest(root, identity_brotli)

    def test_a_brotli_copy_of_no_file_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = dist(tmp)
            (root / "extra.js.br").write_bytes(b"x")
            with self.assertRaisesRegex(cd.DigestError, "of no file"):
                cd.manifest(root, identity_brotli)

    def test_an_empty_or_missing_dist_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(cd.DigestError, "no file"):
                cd.manifest(Path(tmp), identity_brotli)
            with self.assertRaisesRegex(cd.DigestError, "no directory"):
                cd.manifest(Path(tmp) / "absent", identity_brotli)

    def test_one_changed_byte_changes_the_client_digest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = dist(tmp)
            before = cd.client_digest(cd.manifest(root, identity_brotli))
            (root / "main-0123.wasm").write_bytes(b"\0asn")
            self.assertNotEqual(cd.client_digest(cd.manifest(root, identity_brotli)), before)

    def test_the_same_files_in_another_directory_give_the_same_digest(self) -> None:
        with tempfile.TemporaryDirectory() as one, tempfile.TemporaryDirectory() as two:
            nested = Path(two) / "deeper" / "path"
            nested.mkdir(parents=True)
            self.assertEqual(
                cd.client_digest(cd.manifest(dist(one), identity_brotli)),
                cd.client_digest(cd.manifest(dist(str(nested)), identity_brotli)),
            )

    @unittest.skipIf(shutil.which("b2sum") is None, "needs b2sum")
    def test_b2sum_checks_the_served_files_against_the_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = dist(tmp)
            (Path(tmp) / "m").write_text(cd.manifest(root, identity_brotli), encoding="utf-8")
            ok = subprocess.run(["b2sum", "-l", "256", "-c", "--quiet", str(Path(tmp) / "m")], cwd=root, check=False)
            self.assertEqual(ok.returncode, 0)
            (root / "index.html").write_text("<html>changed</html>", encoding="utf-8")
            bad = subprocess.run(
                ["b2sum", "-l", "256", "-c", "--quiet", str(Path(tmp) / "m")],
                cwd=root,
                capture_output=True,
                check=False,
            )
            self.assertNotEqual(bad.returncode, 0)

    @unittest.skipIf(shutil.which("brotli") is None, "needs brotli")
    def test_the_brotli_command_decodes_a_real_copy(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = dist(tmp)
            page = root / "index.html"
            before = cd.manifest(root, identity_brotli)
            subprocess.run(["brotli", "--quality=11", f"--output={page}.br", str(page)], check=True)
            self.assertEqual(cd.manifest(root), before)
            (Path(f"{page}.br")).write_bytes(b"not brotli")
            with self.assertRaisesRegex(cd.DigestError, "no brotli stream"):
                cd.manifest(root)


class Compare(unittest.TestCase):
    def test_the_differences_name_a_changed_file_and_a_missing_one(self) -> None:
        a = "0" * 64 + "  index.html\n" + "1" * 64 + "  main.wasm\n"
        b = "0" * 64 + "  index.html\n" + "2" * 64 + "  main.wasm\n" + "3" * 64 + "  extra.js\n"
        self.assertEqual(cd.differences(a, b), ["extra.js", "main.wasm"])
        self.assertEqual(cd.differences(a, a), [])

    def test_a_line_off_the_form_is_refused(self) -> None:
        with self.assertRaisesRegex(cd.DigestError, "line 1"):
            cd.parse_manifest("not a manifest line\n")

    def test_the_published_files_hold_the_manifest_and_its_digest(self) -> None:
        text = "0" * 64 + "  index.html\n"
        with tempfile.TemporaryDirectory() as tmp:
            digest = cd.write_published(text, Path(tmp))
            self.assertEqual((Path(tmp) / cd.MANIFEST_NAME).read_text(encoding="utf-8"), text)
            self.assertEqual((Path(tmp) / cd.DIGEST_NAME).read_text(encoding="utf-8"), digest + "\n")
            self.assertEqual(digest, hashlib.blake2b(text.encode(), digest_size=32).hexdigest())


class Environment(unittest.TestCase):
    def test_the_build_drops_every_flag_of_the_shell(self) -> None:
        base = {
            "PATH": "/bin",
            "HOME": "/root",
            "RUSTFLAGS": "-C opt-level=0",
            "CARGO_ENCODED_RUSTFLAGS": "x",
            "CARGO_TARGET_DIR": "/elsewhere",
            "CARGO_PROFILE_WASM_RELEASE_OPT_LEVEL": "3",
            "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS": "x",
            "CARGO_BUILD_RUSTFLAGS": "x",
            "TRUNK_BUILD_PUBLIC_URL": "/other/",
            "TRUNK_BUILD_FEATURES": "dev-login",
            "RUSTC_WRAPPER": "sccache",
        }
        env = cd.build_env(base, "1700000000")
        self.assertEqual(
            env,
            {
                "PATH": "/bin",
                "HOME": "/root",
                "SOURCE_DATE_EPOCH": "1700000000",
                "CARGO_INCREMENTAL": "0",
                "TZ": "UTC",
                "LC_ALL": "C",
            },
        )

    def test_a_tree_inside_the_checkout_is_refused(self) -> None:
        with self.assertRaisesRegex(cd.DigestError, "inside the checkout"):
            cd.extract("HEAD", cd.ROOT / "target" / "client-digest-tree")

    def test_a_tree_that_is_not_empty_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "left").write_text("x", encoding="utf-8")
            with self.assertRaisesRegex(cd.DigestError, "not empty"):
                cd.extract("HEAD", Path(tmp))


def jobs(text: str) -> dict[str, str]:
    """The body of each job of a workflow, split on the two-space job keys under `jobs:`."""
    body = text.split("\njobs:\n", 1)[-1]
    found: dict[str, str] = {}
    name = ""
    for line in body.splitlines():
        if len(line) > 2 and line[:2] == "  " and line[2] not in " #" and line.endswith(":"):
            name = line.strip().removesuffix(":")
            found[name] = ""
        elif name and not line.lstrip().startswith("#"):
            found[name] += line + "\n"
    return found


class TrunkPin(unittest.TestCase):
    def test_every_job_that_builds_the_published_release_installs_the_pinned_trunk(self) -> None:
        builders = {
            f"{path.name}:{name}": body
            for path in sorted((cd.ROOT / ".github" / "workflows").glob("*.yml"))
            for name, body in jobs(path.read_text(encoding="utf-8")).items()
            if "just reproducible" in body or "just client-build" in body
        }
        self.assertTrue(builders, "no job builds the published release")
        for job, body in builders.items():
            tools = [line.strip() for line in body.splitlines() if line.strip().startswith("tool:")]
            self.assertTrue(any(f"trunk@{cd.TRUNK_VERSION}" in line for line in tools), f"{job} installs another trunk")

    def test_the_job_splitter_reads_two_jobs(self) -> None:
        text = "on: push\njobs:\n  one:\n    steps:\n      - run: a\n  # note\n  two:\n    steps: []\n"
        self.assertEqual(sorted(jobs(text)), ["one", "two"])


if __name__ == "__main__":
    unittest.main()
