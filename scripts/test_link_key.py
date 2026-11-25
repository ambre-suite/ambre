#!/usr/bin/env python3
"""What `link_key.py` refuses, and what it lets through.

Each test plants one breach into a tree of client files, into the build of the
tree, or hands the patch one value, and asserts the fault, so a gate that
reads nothing fails here.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import copy
import gzip
import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent))

import client_digest  # noqa: E402  -- the release build that checks the slot
import link_key as lk  # noqa: E402  -- the module under test

DOCKERFILE = lk.DOCKERFILE.read_text()
RELEASE = lk.RELEASE.read_text()
COMPOSE = yaml.safe_load(lk.COMPOSE.read_text())
KEY = "11" * 32
# A wasm file around one slot: other data before it and after it.
BEFORE, AFTER = b"\x00asm\x01\x00\x00\x00" + b"\x07" * 40, b"\x09" * 50
WORKER = "ambre-sync-0123456789abcdef_bg.wasm"


def wasm(key: bytes = bytes(32)) -> bytes:
    return BEFORE + lk.MARKER + key + AFTER


def tree(root: Path, worker: bytes | None = None) -> Path:
    """A copy of client files: the page, the Main wasm, the Sync worker and its gzip copy."""
    (root / "index.html").write_text('<meta name="ambre-sync-worker" content="/ambre-sync-0123456789abcdef_loader.js">')
    (root / "ambre-app-1_bg.wasm").write_bytes(b"\x00asm" + b"\x05" * 64)
    data = wasm() if worker is None else worker
    (root / WORKER).write_bytes(data)
    (root / f"{WORKER}.gz").write_bytes(gzip.compress(data, mtime=0))
    return root


def files(root: Path) -> dict[str, bytes]:
    return {str(p.relative_to(root)): p.read_bytes() for p in sorted(root.rglob("*")) if p.is_file()}


class Key(unittest.TestCase):
    """`SRV-366`: the patch refuses no key, a malformed key and the dev key."""

    def test_the_dev_key_is_the_one_of_the_public_dev_seed(self) -> None:
        self.assertEqual(lk.check_sources(lk.ERASURE.read_text(), lk.PROTOCOL.read_text()), [])
        self.assertTrue(lk.DEV_KEY.startswith("927308f5") and len(lk.DEV_KEY) == 64)

    def test_no_key_a_malformed_key_and_the_dev_key_are_refused(self) -> None:
        for value in (None, "", "  ", "zz" * 32, KEY[2:], KEY + "1", lk.DEV_KEY, lk.DEV_KEY.upper()):
            with self.subTest(value=value):
                self.assertIsNotNone(lk.key_refusal(value, dev=False))

    def test_any_other_key_passes_and_the_dev_key_only_in_dev_mode(self) -> None:
        self.assertIsNone(lk.key_refusal(KEY, dev=False))
        self.assertIsNone(lk.key_refusal(KEY.upper() + "\n", dev=False))
        self.assertIsNone(lk.key_refusal(lk.DEV_KEY, dev=True))
        self.assertIsNotNone(lk.key_refusal("", dev=True))


class Patch(unittest.TestCase):
    """`SRV-366`, `SRV-367`: the patch writes the key alone, or nothing."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())

    def test_the_patch_changes_the_32_key_bytes_and_the_copies_alone(self) -> None:
        before = files(tree(self.root))
        path = lk.patch(self.root, KEY.upper(), dev=False)
        after = files(self.root)
        self.assertEqual(path.name, WORKER)
        self.assertEqual(after[WORKER], wasm(bytes.fromhex(KEY)))
        self.assertEqual(gzip.decompress(after[f"{WORKER}.gz"]), after[WORKER])
        changed = {name for name in before if before[name] != after[name]}
        self.assertEqual(changed, {WORKER, f"{WORKER}.gz"})
        self.assertEqual(lk.slot_bytes(after[WORKER]).hex(), KEY)

    def test_the_patch_writes_the_brotli_copy_again(self) -> None:
        tree(self.root)
        (self.root / f"{WORKER}.br").write_bytes(b"stale")
        with mock.patch.object(lk.shutil, "which", return_value="/usr/bin/brotli"), mock.patch.object(
            lk.subprocess, "run"
        ) as run:
            lk.patch(self.root, KEY, dev=False)
        args = run.call_args.args[0]
        self.assertEqual(args[0], "brotli")
        self.assertIn(f"--output={self.root / WORKER}.br", args)
        self.assertEqual(args[-1], str(self.root / WORKER))

    def test_a_brotli_copy_with_no_brotli_command_writes_nothing(self) -> None:
        tree(self.root)
        (self.root / f"{WORKER}.br").write_bytes(b"stale")
        before = files(self.root)
        with mock.patch.object(lk.shutil, "which", return_value=None):
            with self.assertRaisesRegex(lk.SlotError, "brotli"):
                lk.patch(self.root, KEY, dev=False)
        self.assertEqual(files(self.root), before)

    def test_each_refusal_writes_nothing(self) -> None:
        cases = {
            "no key": (wasm(), None, "not set"),
            "the dev key": (wasm(), lk.DEV_KEY, "development key"),
            "no slot": (BEFORE + AFTER, KEY, "no link key slot"),
            "two slots in one file": (wasm() + wasm(), KEY, "2 link key slots"),
            "a slot that holds a key": (wasm(b"\x22" * 32), KEY, "holds a key"),
            "a slot cut short": (BEFORE + lk.MARKER + bytes(31), KEY, "ends before"),
        }
        for name, (worker, value, words) in cases.items():
            with self.subTest(name), tempfile.TemporaryDirectory() as d:
                root = tree(Path(d), worker)
                before = files(root)
                with self.assertRaisesRegex(lk.SlotError, words):
                    lk.patch(root, value, dev=False)
                self.assertEqual(files(root), before)

    def test_two_slots_in_two_files_are_refused(self) -> None:
        tree(self.root)
        (self.root / "ambre-call-1_bg.wasm").write_bytes(wasm())
        with self.assertRaisesRegex(lk.SlotError, "2 link key slots"):
            lk.patch(self.root, KEY, dev=False)

    def test_an_empty_tree_is_refused(self) -> None:
        with self.assertRaisesRegex(lk.SlotError, "no link key slot"):
            lk.patch(self.root, KEY, dev=False)

    def test_the_dev_mode_takes_the_dev_key(self) -> None:
        tree(self.root)
        lk.patch(self.root, lk.DEV_KEY, dev=True)
        self.assertEqual(lk.slot_bytes((self.root / WORKER).read_bytes()).hex(), lk.DEV_KEY)

    def test_the_command_refuses_with_a_message_and_a_status(self) -> None:
        tree(self.root)
        err = io.StringIO()
        with mock.patch.dict(lk.os.environ, {lk.VARIABLE: lk.DEV_KEY}), mock.patch.object(sys, "stderr", err):
            self.assertEqual(lk.main(["patch", str(self.root)]), 1)
        self.assertIn("development key", err.getvalue())


class Empty(unittest.TestCase):
    """`ZK-057`: a release holds one slot, and that slot is empty."""

    def test_a_build_with_one_empty_slot_passes(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            self.assertIsNone(lk.empty_refusal(tree(Path(d))))

    def test_a_build_with_no_slot_two_slots_or_a_key_fails(self) -> None:
        for worker in (BEFORE + AFTER, wasm() + wasm(), wasm(bytes.fromhex(KEY))):
            with self.subTest(worker=len(worker)), tempfile.TemporaryDirectory() as d:
                self.assertIsNotNone(lk.empty_refusal(tree(Path(d), worker)))

    def test_the_client_digest_refuses_a_release_that_holds_a_key(self) -> None:
        def build(cmd: list[str], **_: object) -> mock.Mock:
            dist = Path(cmd[cmd.index("--dist") + 1])
            dist.mkdir(parents=True)
            tree(dist, wasm(bytes.fromhex(KEY)))
            return mock.Mock(returncode=0)

        with tempfile.TemporaryDirectory() as d, mock.patch.multiple(
            client_digest,
            trunk_version=lambda: client_digest.TRUNK_VERSION,
            extract=lambda rev, into: into.mkdir(parents=True),
            commit_epoch=lambda rev: "0",
        ), mock.patch.object(client_digest.subprocess, "run", side_effect=build):
            with self.assertRaisesRegex(client_digest.DigestError, "holds a key"):
                client_digest.build("HEAD", Path(d) / "a")


class Zero(unittest.TestCase):
    """`ZK-059`: a patched file with its slot zeroed is the published file."""

    def test_a_zeroed_copy_has_the_digest_of_the_release(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            root = tree(Path(d))
            published = client_digest.file_digest((root / WORKER).read_bytes())
            lk.patch(root, KEY, dev=False)
            served = root / WORKER
            self.assertNotEqual(client_digest.file_digest(served.read_bytes()), published)
            self.assertEqual(lk.main(["zero", str(served), str(root / "zeroed.wasm")]), 0)
            self.assertEqual(client_digest.file_digest((root / "zeroed.wasm").read_bytes()), published)


class Served(unittest.TestCase):
    """`SRV-369`: the check reads the Sync worker the page names."""

    def test_the_worker_comes_from_the_address_of_the_page(self) -> None:
        asked: list[str] = []
        page = b'<meta name="ambre-sync-worker" content="/ambre-sync-0123456789abcdef_loader.js">'

        def urlopen(request: object, **_: object) -> mock.MagicMock:
            asked.append(request.full_url)
            answer = mock.MagicMock()
            answer.__enter__.return_value.read.return_value = page if len(asked) == 1 else wasm(bytes.fromhex(KEY))
            return answer

        with mock.patch.object(lk.urllib.request, "urlopen", side_effect=urlopen), mock.patch.dict(
            lk.os.environ, {lk.VARIABLE: KEY}
        ):
            self.assertEqual(lk.main(["served", "https://localhost"]), 0)
            asked.clear()
            with mock.patch.dict(lk.os.environ, {lk.VARIABLE: "22" * 32}), mock.patch.object(sys, "stderr", io.StringIO()):
                self.assertEqual(lk.main(["served", "https://localhost"]), 1)
        self.assertEqual(asked, ["https://localhost/", "https://localhost/ambre-sync-0123456789abcdef_bg.wasm"])


class Lint(unittest.TestCase):
    """`ZK-057`, `SRV-369`: the build, the release and the compose file."""

    def test_the_tree_passes(self) -> None:
        self.assertEqual(lk.check_dockerfile(DOCKERFILE), [])
        self.assertEqual(lk.check_release(RELEASE), [])
        self.assertEqual(lk.check_compose(COMPOSE), [])

    def test_a_marker_or_a_dev_key_apart_from_the_sources_fails(self) -> None:
        erasure, protocol = lk.ERASURE.read_text(), lk.PROTOCOL.read_text()
        self.assertTrue(lk.check_sources(erasure.replace("0x46, 0x11", "0x47, 0x11", 1), protocol))
        self.assertTrue(lk.check_sources(erasure, protocol.replace("0x92, 0x73", "0x93, 0x73", 1)))

    def test_a_key_named_at_build_time_fails(self) -> None:
        planted = DOCKERFILE.replace("ARG DEV_LOGIN=\n", "ARG DEV_LOGIN=\nARG AMBRE_THREAD_LINK_KEY=\n")
        self.assertNotEqual(planted, DOCKERFILE)
        self.assertTrue(any("AMBRE_THREAD_LINK_KEY" in f for f in lk.check_dockerfile(planted)))
        planted = RELEASE + "\n# build-args: AMBRE_THREAD_LINK_KEY=${{ vars.THREAD_LINK_KEY }}\n"
        self.assertEqual(len(lk.check_release(planted)), 2)

    def test_a_build_with_no_empty_gate_fails(self) -> None:
        planted = DOCKERFILE.replace(f"python3 {lk.EMPTY_GATE}; \\", "true; \\")
        self.assertNotEqual(planted, DOCKERFILE)
        self.assertTrue(any("no `" in f for f in lk.check_dockerfile(planted)))

    def test_an_init_image_without_the_patch_or_brotli_fails(self) -> None:
        for old, words in (
            ("COPY scripts/link_key.py /opt/ambre/link_key.py\n", "no COPY"),
            ("brotli curl python3", "no brotli"),
        ):
            with self.subTest(old=old):
                planted = DOCKERFILE.replace(old, "curl python3" if "curl" in old else "")
                self.assertNotEqual(planted, DOCKERFILE)
                self.assertTrue(any(words in f for f in lk.check_dockerfile(planted)))

    def test_each_compose_breach_fails(self) -> None:
        def drop_service(c: dict) -> None:
            del c["services"]["link-key"]

        def serve_the_image(c: dict) -> None:
            c["services"]["web"]["volumes"] = [v for v in c["services"]["web"]["volumes"] if "/srv/ambre" not in v]

        def patch_from_web(c: dict) -> None:
            c["services"]["link-key"]["image"] = "ambre-web:local"

        def no_patch(c: dict) -> None:
            c["services"]["link-key"]["command"] = ["empty", "/work"]

        def key_in_clear(c: dict) -> None:
            c["services"]["link-key"]["environment"]["THREAD_LINK_KEY"] = KEY

        def start_first(c: dict) -> None:
            c["services"]["web"]["depends_on"]["link-key"] = {"condition": "service_started"}

        def build_arg(c: dict) -> None:
            c["services"]["web"]["build"]["args"]["AMBRE_THREAD_LINK_KEY"] = KEY

        for plant in (drop_service, serve_the_image, patch_from_web, no_patch, key_in_clear, start_first, build_arg):
            with self.subTest(plant.__name__):
                planted = copy.deepcopy(COMPOSE)
                plant(planted)
                self.assertTrue(lk.check_compose(planted))


if __name__ == "__main__":
    unittest.main()
