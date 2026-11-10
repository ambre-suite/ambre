#!/usr/bin/env python3
"""What `trunk_post_build.py` writes into a staging directory, and what it refuses.

The worker is tested on a scratch staging directory: its three files take one
hash, one loader and one empty meta are filled, anything else is refused. The
brotli copies are tested only where the `brotli` command is installed.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import hashlib
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import trunk_post_build as hook  # noqa: E402  -- the module under test

PAGE = f"<head>\n    {hook.PLACEHOLDER}\n</head>\n"

# The loader as Trunk 0.21 writes it.
LOADER = 'importScripts("./ambre-sync.js");wasm_bindgen("./ambre-sync_bg.wasm");'


def unhashed(tmp: str, loader: str = LOADER) -> Path:
    root = Path(tmp)
    (root / "index.html").write_text(PAGE, encoding="utf-8")
    (root / hook.GLUE).write_text("glue", encoding="utf-8")
    (root / hook.WASM).write_bytes(b"\0asm")
    (root / hook.LOADER).write_text(loader, encoding="utf-8")
    return root


class HashWorker(unittest.TestCase):
    def test_the_three_files_take_one_hash_and_the_loader_names_the_other_two(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = unhashed(tmp)
            name = hook.hash_worker(root)
            stem = name.removesuffix("_loader.js")
            self.assertRegex(stem, r"^ambre-sync-[0-9a-f]{16}$")
            self.assertEqual(
                sorted(p.name for p in root.iterdir()),
                sorted(["index.html", f"{stem}.js", f"{stem}_bg.wasm", name]),
            )
            self.assertEqual(
                (root / name).read_text(encoding="utf-8"),
                f'importScripts("./{stem}.js");wasm_bindgen("./{stem}_bg.wasm");',
            )

    def test_another_wasm_takes_another_hash(self) -> None:
        with tempfile.TemporaryDirectory() as one, tempfile.TemporaryDirectory() as two:
            first = hook.hash_worker(unhashed(one))
            root = unhashed(two)
            (root / hook.WASM).write_bytes(b"\0asm\1")
            self.assertNotEqual(first, hook.hash_worker(root))

    def test_a_missing_file_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = unhashed(tmp)
            (root / hook.WASM).unlink()
            with self.assertRaises(hook.HookError):
                hook.hash_worker(root)

    def test_a_loader_that_names_its_files_otherwise_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(hook.HookError):
            hook.hash_worker(unhashed(tmp, loader='importScripts("./ambre-sync.js");wasm_bindgen();'))


def staging(tmp: str, page: str = PAGE, loaders: tuple[str, ...] = ("ambre-sync-0123abcd_loader.js",)) -> Path:
    root = Path(tmp)
    (root / "index.html").write_text(page, encoding="utf-8")
    for name in loaders:
        (root / name).write_text("importScripts();", encoding="utf-8")
    return root


class FillWorkerAddress(unittest.TestCase):
    def test_the_loader_address_is_written_into_the_meta(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = staging(tmp)
            address = hook.fill_worker_address(root, "/")
            self.assertEqual(address, "/ambre-sync-0123abcd_loader.js")
            page = (root / "index.html").read_text(encoding="utf-8")
            self.assertIn('<meta name="ambre-sync-worker" content="/ambre-sync-0123abcd_loader.js">', page)
            self.assertNotIn(hook.PLACEHOLDER, page)

    def test_the_public_url_prefixes_the_address(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            address = hook.fill_worker_address(staging(tmp), "/app/")
            self.assertEqual(address, "/app/ambre-sync-0123abcd_loader.js")

    def test_no_loader_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(hook.HookError):
            hook.fill_worker_address(staging(tmp, loaders=()), "/")

    def test_two_loaders_are_refused(self) -> None:
        loaders = ("ambre-sync-0123abcd_loader.js", "ambre-sync-4567ef01_loader.js")
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(hook.HookError):
            hook.fill_worker_address(staging(tmp, loaders=loaders), "/")

    def test_a_page_without_the_empty_meta_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(hook.HookError):
            hook.fill_worker_address(staging(tmp, page="<head></head>\n"), "/")

class CallWorker(unittest.TestCase):
    def test_the_call_worker_takes_a_hash_and_a_meta_of_its_own(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            page = f"<head>\n    {hook.PLACEHOLDER}\n    {hook.placeholder('ambre-call')}\n</head>\n"
            (root / "index.html").write_text(page, encoding="utf-8")
            glue, wasm, loader = hook.worker_files("ambre-call")
            (root / glue).write_text("glue", encoding="utf-8")
            (root / wasm).write_bytes(b"\0asm")
            (root / loader).write_text(f'importScripts("./{glue}");wasm_bindgen("./{wasm}");', encoding="utf-8")
            name = hook.hash_worker(root, "ambre-call")
            self.assertRegex(name, r"^ambre-call-[0-9a-f]{16}_loader\.js$")
            address = hook.fill_worker_address(root, "/", "ambre-call")
            self.assertEqual(address, f"/{name}")
            text = (root / "index.html").read_text(encoding="utf-8")
            self.assertIn(f'<meta name="ambre-call-worker" content="/{name}">', text)
            self.assertIn(hook.PLACEHOLDER, text, "the Sync meta waits for its own loader")



class Compressible(unittest.TestCase):
    def test_text_and_wasm_are_chosen_and_compressed_formats_are_not(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = staging(tmp)
            (root / "fonts").mkdir()
            for name in ("ambre-app-0123abcd_bg.wasm", "fonts/Noto.ttf", "fonts/Noto.woff2", "backdrop.png"):
                (root / name).write_bytes(b"x")
            chosen = {path.relative_to(root).as_posix() for path in hook.compressible(root)}
            self.assertEqual(
                chosen,
                {"index.html", "ambre-sync-0123abcd_loader.js", "ambre-app-0123abcd_bg.wasm", "fonts/Noto.ttf"},
            )


@unittest.skipIf(shutil.which("brotli") is None, "no `brotli` command on the path")
class WriteBrotli(unittest.TestCase):
    def test_a_copy_is_written_beside_each_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = staging(tmp)
            hook.write_brotli(hook.compressible(root))
            self.assertTrue((root / "index.html.br").is_file())
            self.assertTrue((root / "ambre-sync-0123abcd_loader.js.br").is_file())


class CopyConverter(unittest.TestCase):
    def built(self, tmp: str, module: bytes = b"\0asm module") -> Path:
        source = Path(tmp) / "converter"
        source.mkdir()
        (source / hook.converter_module("markup")).write_bytes(module)
        digest = hashlib.blake2b(module, digest_size=32).hexdigest()
        (source / hook.converter_digest("markup")).write_text(digest + "\n", encoding="utf-8")
        return source

    def test_the_module_is_named_by_the_first_digits_of_its_digest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = self.built(tmp)
            staging = Path(tmp) / "staging"
            staging.mkdir()
            name = hook.copy_converter(staging, source, "markup")
            digest = hashlib.blake2b(b"\0asm module", digest_size=32).hexdigest()
            self.assertEqual(name, f"ambre-convert-markup-{digest[:16]}.wasm")
            self.assertEqual((staging / name).read_bytes(), b"\0asm module")

    def test_a_module_its_digest_does_not_name_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = self.built(tmp)
            (source / hook.converter_module("markup")).write_bytes(b"other bytes")
            with self.assertRaises(hook.HookError):
                hook.copy_converter(Path(tmp), source, "markup")

    def test_a_missing_module_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(hook.HookError):
                hook.copy_converter(Path(tmp), Path(tmp) / "nowhere", "markup")

    def test_a_family_whose_module_the_build_skipped_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source = self.built(tmp)
            with self.assertRaises(hook.HookError):
                hook.copy_converter(Path(tmp), source, "pdf")

    def test_the_families_are_the_ones_the_build_writes(self) -> None:
        import converter_build

        self.assertEqual(hook.CONVERTER_FAMILIES, converter_build.FAMILIES)


if __name__ == "__main__":
    unittest.main()
