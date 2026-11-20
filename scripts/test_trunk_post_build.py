#!/usr/bin/env python3
"""What `trunk_post_build.py` writes into a staging directory, and what it refuses.

The worker is tested on a scratch staging directory: its three files take one
hash, one loader and one empty meta are filled, anything else is refused. The
brotli copies are tested only where the `brotli` command is installed.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import hashlib
import io
import os
import re
import shutil
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from unittest import mock
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



class ThreadScript(unittest.TestCase):
    def page(self, tmp: str) -> Path:
        root = Path(tmp)
        (root / "index.html").write_text(f"<head>\n    {hook.THREAD_PLACEHOLDER}\n</head>\n", encoding="utf-8")
        (root / "ambre-app-0123abcd.js").write_text("export default function init() {}", encoding="utf-8")
        (root / "ambre-sync-4567ef01.js").write_text("glue", encoding="utf-8")
        return root

    def test_the_script_imports_the_main_glue_and_its_address_fills_the_meta(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self.page(tmp)
            address = hook.write_thread_script(root, "/")
            self.assertRegex(address, r"^/ambre-thread-[0-9a-f]{16}\.js$")
            script = (root / address.removeprefix("/")).read_text(encoding="utf-8")
            self.assertIn('from "./ambre-app-0123abcd.js";', script)
            self.assertIn("wasm_thread_entry_point(work);", script)
            page = (root / "index.html").read_text(encoding="utf-8")
            self.assertIn(f'<meta name="ambre-thread-worker" content="{address}">', page)

    def test_two_main_glues_are_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self.page(tmp)
            (root / "ambre-app-89abcdef.js").write_text("glue", encoding="utf-8")
            with self.assertRaises(hook.HookError):
                hook.write_thread_script(root, "/")

    def test_a_page_without_the_empty_meta_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self.page(tmp)
            (root / "index.html").write_text("<head></head>\n", encoding="utf-8")
            with self.assertRaises(hook.HookError):
                hook.write_thread_script(root, "/")


# A page as Trunk leaves it: an inline script and style of the source, a
# script of the build, the init script Trunk injects, and a comment that
# quotes a script no browser runs.
POLICY_PAGE = f"""<head>
    {hook.POLICY_PLACEHOLDER}
    <style>body {{ margin: 0; }}</style>
    <!-- <script>quoted()</script> -->
    <script src="/ambre-app-0123abcd.js" type="module"></script>
</head>
<body>
    <script type="module">alert('Hello, world.');</script>
    <script type="module">import init from '/ambre-app-0123abcd.js'; init();</script>
</body>
"""


class ContentPolicy(unittest.TestCase):
    def filled(self, tmp: str, page: str = POLICY_PAGE) -> tuple[str, str]:
        root = Path(tmp)
        (root / "index.html").write_text(page, encoding="utf-8")
        theme = root / "theme.css"
        theme.write_text(":root { --bg: #fff; }\n", encoding="utf-8")
        text = hook.fill_policy(root, theme)
        return text, (root / "index.html").read_text(encoding="utf-8")

    def directive(self, text: str, name: str) -> list[str]:
        found = [d.split()[1:] for d in text.split("; ") if d.split()[0] == name]
        self.assertEqual(len(found), 1, f"{name} in {text}")
        return found[0]

    def test_the_digest_is_the_one_the_csp_specification_gives(self) -> None:
        # The example of the CSP Level 3 specification, section on hash sources.
        self.assertEqual(hook.digest("alert('Hello, world.');"), "'sha256-qznLcsROx4GACP2dm0UCKCzCG+HiZ1guq6ZZDob/Tng='")

    def test_each_inline_script_is_admitted_by_its_digest_and_nothing_else(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            text, page = self.filled(tmp)
            sources = self.directive(text, "script-src")
            self.assertEqual(
                sources,
                [
                    "'self'",
                    "'wasm-unsafe-eval'",
                    hook.digest("alert('Hello, world.');"),
                    hook.digest("import init from '/ambre-app-0123abcd.js'; init();"),
                ],
            )
            self.assertNotIn(hook.digest("quoted()"), sources, "a comment ships no script")
            self.assertNotIn("'unsafe-inline'", text)
            self.assertNotIn("'unsafe-eval'", text)
            self.assertIn(f'<meta http-equiv="Content-Security-Policy" content="{text}">', page)
            self.assertNotIn(hook.POLICY_PLACEHOLDER, page)

    def test_each_inline_style_and_the_token_sheet_are_admitted_by_digest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            text, _ = self.filled(tmp)
            self.assertEqual(
                self.directive(text, "style-src"),
                ["'self'", hook.digest("body { margin: 0; }"), hook.digest(":root { --bg: #fff; }\n")],
            )

    def test_workers_come_from_the_page_origin_and_nothing_falls_open(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            text, _ = self.filled(tmp)
            self.assertEqual(self.directive(text, "worker-src"), ["'self'"])
            self.assertEqual(self.directive(text, "default-src"), ["'self'"])
            self.assertEqual(self.directive(text, "object-src"), ["'none'"])
            self.assertEqual(self.directive(text, "base-uri"), ["'none'"])
            self.assertNotIn("blob:", self.directive(text, "script-src") + self.directive(text, "worker-src"))

    def test_the_token_sheet_the_launcher_injects_is_the_shipped_file(self) -> None:
        self.assertTrue(hook.THEME_CSS.is_file(), hook.THEME_CSS)
        source = (Path(__file__).resolve().parent.parent / "crates" / "ambre-app" / "src" / "platform.rs").read_text(
            encoding="utf-8"
        )
        self.assertIn("style.set_text_content(Some(ambre_canvas::theme::THEME_CSS));", source)

    def test_the_source_page_carries_the_empty_meta_first_in_its_head(self) -> None:
        source = (Path(__file__).resolve().parent.parent / "crates" / "ambre-app" / "index.html").read_text(
            encoding="utf-8"
        )
        head = hook.COMMENT.sub("", source)
        self.assertEqual(head.count(hook.POLICY_PLACEHOLDER), 1)
        self.assertLess(head.index(hook.POLICY_PLACEHOLDER), head.index("<script"))
        self.assertLess(head.index(hook.POLICY_PLACEHOLDER), head.index("<style"))
        self.assertLess(head.index(hook.POLICY_PLACEHOLDER), head.index("<link"))

    def test_a_page_without_the_empty_meta_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(hook.HookError):
            self.filled(tmp, page="<head></head>\n")

    def test_the_reload_script_of_the_dev_server_is_admitted_as_the_server_sends_it(self) -> None:
        # The two placeholders `trunk serve` rewrites in each answer. Once the
        # hook writes them, the server finds nothing to rewrite, so the bytes
        # the digest names are the bytes the browser runs.
        reload = "const address = '{{__TRUNK_ADDRESS__}}'; const base = '{{__TRUNK_WS_BASE__}}';"
        page = POLICY_PAGE.replace("</body>", f"<script>{reload}</script>\n</body>")
        with tempfile.TemporaryDirectory() as tmp:
            text, served = self.filled(tmp, page=page)
            self.assertNotIn("{{__TRUNK", served)
            settled = "const address = window.location.host; const base = '/';"
            self.assertIn(f"<script>{settled}</script>", served)
            self.assertIn(hook.digest(settled), self.directive(text, "script-src"))


class EmbedSites(unittest.TestCase):
    """`SRV-312`: the page admits in a frame the sites the deployment names, and no frame
    when it names none."""

    def frame_src(self, tmp: str, frames: list[str] | None) -> list[str]:
        root = Path(tmp)
        (root / "index.html").write_text(POLICY_PAGE, encoding="utf-8")
        theme = root / "theme.css"
        theme.write_text(":root { --bg: #fff; }\n", encoding="utf-8")
        text = hook.fill_policy(root, theme, frames=frames)
        found = [d.split()[1:] for d in text.split("; ") if d.split()[0] == "frame-src"]
        self.assertEqual(len(found), 1, text)
        return found[0]

    def test_a_deployment_that_names_no_site_admits_no_frame(self) -> None:
        for value in ("", "  ", " , "):
            with tempfile.TemporaryDirectory() as tmp:
                self.assertEqual(self.frame_src(tmp, hook.embed_sites(value)), ["'none'"], repr(value))
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(self.frame_src(tmp, None), ["'none'"])

    def test_each_named_site_is_admitted_as_an_https_origin_and_nothing_else(self) -> None:
        sites = hook.embed_sites("www.youtube.com, https://*.Vimeo.com/\n example.org:8443 www.youtube.com")
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(
                self.frame_src(tmp, sites),
                ["https://www.youtube.com", "https://*.vimeo.com", "https://example.org:8443"],
            )

    def test_a_value_that_is_not_a_site_fails_the_build(self) -> None:
        for value in (
            "*",
            "https:",
            "https://",
            "http://example.org",
            "data:",
            "example.org/watch",
            "'unsafe-inline'",
            "'self'",
            "example.org;script-src",
            "*.",
            "-bad.example.org",
        ):
            with self.subTest(value=value), self.assertRaises(hook.HookError):
                hook.embed_sites(value)

    def test_the_hook_reads_the_sites_from_the_deployment(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            env = {"TRUNK_STAGING_DIR": tmp, hook.EMBED_SITES_VAR: "www.youtube.com"}
            seen: list[list[str] | None] = []
            with (
                mock.patch.dict(os.environ, env, clear=True),
                mock.patch.object(hook, "hash_worker"),
                mock.patch.object(hook, "fill_worker_address"),
                mock.patch.object(hook, "write_thread_script"),
                mock.patch.object(hook, "copy_converter"),
                mock.patch.object(hook, "write_headers"),
                mock.patch.object(hook, "fill_policy", side_effect=lambda *a, **k: seen.append(k["frames"])),
            ):
                self.assertEqual(hook.main(), 0)
            self.assertEqual(seen, [["https://www.youtube.com"]])
            env[hook.EMBED_SITES_VAR] = "http://www.youtube.com"
            with (
                mock.patch.dict(os.environ, env, clear=True),
                mock.patch.object(hook, "hash_worker"),
                mock.patch.object(hook, "fill_worker_address"),
                mock.patch.object(hook, "write_thread_script"),
                mock.patch.object(hook, "copy_converter"),
                mock.patch.object(hook, "write_headers"),
                redirect_stderr(io.StringIO()),
            ):
                self.assertEqual(hook.main(), 1)


class Headers(unittest.TestCase):
    """`SRV-281`, `SRV-282`: the headers file of the build refuses any frame on every answer,
    and gives the script of each worker the worker policy."""

    BUILT = ("ambre-sync-0123abcd_loader.js", "ambre-call-4567ef01_loader.js", "ambre-thread-89ab2345.js")

    def built(self, tmp: str, names: tuple[str, ...] = BUILT) -> Path:
        root = Path(tmp)
        for name in names:
            (root / name).write_text("script", encoding="utf-8")
        return root

    def rules(self, text: str) -> dict[str, list[str]]:
        """Each path of the file and the header lines under it."""
        found: dict[str, list[str]] = {}
        path = ""
        for line in text.splitlines():
            if line.startswith("  "):
                found[path].append(line.strip())
            else:
                path = line
                self.assertNotIn(path, found, f"{path} twice")
                found[path] = []
        return found

    def test_every_answer_refuses_a_frame_and_is_isolated(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            rules = self.rules(hook.write_headers(self.built(tmp)))
            self.assertEqual(
                rules["/*"],
                [
                    "Cross-Origin-Opener-Policy: same-origin",
                    "Cross-Origin-Embedder-Policy: credentialless",
                    "Content-Security-Policy: frame-ancestors 'none'",
                ],
            )

    def test_the_script_of_each_worker_carries_the_worker_policy(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self.built(tmp)
            (root / "ambre-app-0123abcd.js").write_text("glue", encoding="utf-8")
            text = hook.write_headers(root, "/app/")
            self.assertEqual((root / hook.HEADERS_FILE).read_text(encoding="utf-8"), text)
            rules = self.rules(text)
            self.assertEqual(sorted(rules), sorted(["/app/*", *(f"/app/{n}" for n in self.BUILT)]))
            for name in self.BUILT:
                self.assertEqual(rules[f"/app/{name}"], [f"Content-Security-Policy: {hook.worker_policy()}"])

    def test_the_worker_policy_runs_the_build_alone(self) -> None:
        policy = hook.worker_policy()
        directives = {d.split()[0]: d.split()[1:] for d in policy.split("; ")}
        self.assertEqual(directives["script-src"], ["'self'", "'wasm-unsafe-eval'"])
        self.assertEqual(directives["worker-src"], ["'self'"])
        self.assertEqual(directives["object-src"], ["'none'"])
        self.assertEqual(directives["frame-ancestors"], ["'none'"])
        for banned in ("'unsafe-inline'", "'unsafe-eval'", "blob:", "data:"):
            self.assertNotIn(banned, policy)

    def test_a_build_without_each_worker_script_is_refused(self) -> None:
        for missing in self.BUILT:
            with tempfile.TemporaryDirectory() as tmp, self.assertRaises(hook.HookError, msg=missing):
                hook.write_headers(self.built(tmp, tuple(n for n in self.BUILT if n != missing)))

    def test_the_hook_writes_the_file_after_the_policy(self) -> None:
        source = Path(hook.__file__).read_text(encoding="utf-8")
        main = source[source.index("def main()") :]
        self.assertLess(main.index("fill_policy("), main.index("write_headers("))
        self.assertLess(main.index("write_headers("), main.index("write_brotli("))


def wasm(*custom: bytes) -> bytes:
    """A wasm module of one type section and one custom section per name of `custom`."""
    out = b"\0asm\x01\0\0\0" + b"\x01\x01\x00"
    for name in custom:
        body = bytes([len(name)]) + name + b"x"
        out += b"\x00" + bytes([len(body)]) + body
    return out


class DevLoginGate(unittest.TestCase):
    """`ZK-045`: a release whose wasm carries the development sign-in fails."""

    def built(self, tmp: str, wasm: bytes) -> Path:
        root = Path(tmp)
        (root / "ambre-sync-0123abcd_bg.wasm").write_bytes(wasm)
        (root / "ambre-app-0123abcd_bg.wasm").write_bytes(b"\0asm")
        return root

    def test_each_mark_in_a_wasm_refuses_the_release(self) -> None:
        for mark in hook.DEV_LOGIN_MARKS:
            with tempfile.TemporaryDirectory() as tmp:
                root = self.built(tmp, b"\0asm" + mark + b"\0")
                self.assertEqual(hook.dev_login_in(root), ["ambre-sync-0123abcd_bg.wasm"])
                with self.assertRaises(hook.HookError):
                    hook.refuse_dev_login(root)

    def test_a_wasm_with_no_mark_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            hook.refuse_dev_login(self.built(tmp, b"\0asm ambre-dev"))

    def test_a_release_is_guarded_and_a_debug_build_is_not(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "ambre-app-0123abcd_bg.wasm").write_bytes(wasm(b"name", b"producers"))
            self.assertTrue(hook.guards_dev_login({"TRUNK_PROFILE": "release"}, root))
            self.assertFalse(hook.guards_dev_login({"TRUNK_PROFILE": "debug"}, root))

    def test_an_optimised_build_is_guarded_whatever_its_trunk_profile(self) -> None:
        # `ZK-049`, audit `W209`: the proofs built the profile of the release
        # without `--release`, and the gate read the Trunk profile alone.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "ambre-app-0123abcd_bg.wasm").write_bytes(wasm(b"producers"))
            self.assertEqual(hook.optimised_wasm_in(root), ["ambre-app-0123abcd_bg.wasm"])
            self.assertTrue(hook.guards_dev_login({"TRUNK_PROFILE": "debug"}, root))
            self.assertTrue(hook.guards_dev_login({}, root))

    def test_a_converter_module_marks_no_build_optimised(self) -> None:
        # A converter module is always built under the profile of the release.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "ambre-app-0123abcd_bg.wasm").write_bytes(wasm(b"name"))
            (root / "ambre-convert-pdf-0123456789abcdef.wasm").write_bytes(wasm())
            self.assertFalse(hook.guards_dev_login({"TRUNK_PROFILE": "debug"}, root))

    def test_the_hook_refuses_an_optimised_debug_build_with_the_sign_in(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "ambre-sync-0123abcd_bg.wasm").write_bytes(wasm(b"producers") + b"ambre-dev-seed")
            env = {"TRUNK_STAGING_DIR": str(root), "TRUNK_PROFILE": "debug"}
            with mock.patch.dict(os.environ, env), redirect_stderr(io.StringIO()):
                self.assertEqual(hook.main(), 1)
            self.assertEqual(len(list(root.iterdir())), 1)

    def test_the_sections_of_a_wasm_are_read_by_name(self) -> None:
        self.assertEqual(hook.wasm_sections(wasm(b"name", b"producers")), [b"name", b"producers"])
        self.assertIsNone(hook.wasm_sections(b"not a wasm"))
        # A truncated section stops the walk without an error.
        self.assertEqual(hook.wasm_sections(wasm(b"name")[:-1]), [])

    def test_the_former_escape_opens_nothing(self) -> None:
        # The call proofs once passed a release through this name. It now
        # opens nothing: the hook refuses the release all the same.
        with tempfile.TemporaryDirectory() as tmp:
            self.assertTrue(
                hook.guards_dev_login({"TRUNK_PROFILE": "release", "AMBRE_DEV_LOGIN_RELEASE": "1"}, Path(tmp))
            )
        with tempfile.TemporaryDirectory() as tmp:
            root = self.built(tmp, b"\0asm ambre-dev-seed")
            env = {"TRUNK_STAGING_DIR": str(root), "TRUNK_PROFILE": "release", "AMBRE_DEV_LOGIN_RELEASE": "1"}
            with mock.patch.dict(os.environ, env), redirect_stderr(io.StringIO()):
                self.assertEqual(hook.main(), 1)
            self.assertEqual(len(list(root.iterdir())), 2)

    def test_the_proofs_build_under_the_profile_of_the_release(self) -> None:
        # The proofs measure the shipped wasm, so they take the cargo profile
        # of the release. The gate guards that build (`ZK-049`), so the
        # development sign-in it carries fails the build.
        repo = Path(__file__).resolve().parent.parent
        page = (repo / "crates" / "ambre-app" / "index.html").read_text(encoding="utf-8")
        profiles = set(re.findall(r'data-cargo-profile-release="([^"]+)"', page))
        self.assertEqual(len(profiles), 1, profiles)
        recipe = re.search(r"^proof_build := '([^']*)'$", (repo / "justfile").read_text(encoding="utf-8"), re.M)
        assert recipe is not None
        trunk = recipe.group(1).split("&&")[0].split()
        self.assertEqual(trunk[:2], ["trunk", "build"])
        self.assertNotIn("--release", trunk)
        self.assertIn(f"--cargo-profile={profiles.pop()}", trunk)
        self.assertIn("dev-login", trunk)

    def test_the_hook_refuses_a_release_before_it_writes_a_copy(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = self.built(tmp, b"\0asm ambre-dev-seed")
            env = {"TRUNK_STAGING_DIR": str(root), "TRUNK_PROFILE": "release"}
            with mock.patch.dict(os.environ, env), redirect_stderr(io.StringIO()):
                self.assertEqual(hook.main(), 1)
            self.assertEqual(len(list(root.iterdir())), 2)

    def test_each_mark_is_spelled_by_its_source(self) -> None:
        repo = Path(__file__).resolve().parent.parent
        for mark, source in hook.DEV_LOGIN_MARKS.items():
            self.assertIn(mark.decode(), (repo / source).read_text(encoding="utf-8"), source)


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
