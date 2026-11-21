"""Tests of the two scripts of the self-hosted stack: the header translation and the wait."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


caddy_headers = load("caddy_headers", ROOT / "deploy/compose/caddy_headers.py")
selfhost_wait = load("selfhost_wait", ROOT / "scripts/selfhost_wait.py")


HEADERS = """/*
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: credentialless
  Content-Security-Policy: frame-ancestors 'none'
/ambre-thread-0123456789abcdef.js
  Content-Security-Policy: default-src 'none'; script-src 'self'
"""


class CaddyHeaders(unittest.TestCase):
    def test_every_answer_and_each_named_script_get_their_headers(self) -> None:
        out = caddy_headers.directives(caddy_headers.parse(HEADERS))
        self.assertEqual(
            out.splitlines(),
            [
                'header +Cross-Origin-Opener-Policy "same-origin"',
                'header +Cross-Origin-Embedder-Policy "credentialless"',
                "header +Content-Security-Policy \"frame-ancestors 'none'\"",
                "@ambre_headers_1 path /ambre-thread-0123456789abcdef.js",
                "header @ambre_headers_1 +Content-Security-Policy "
                "\"default-src 'none'; script-src 'self'\"",
            ],
        )

    def test_a_header_before_any_path_is_refused(self) -> None:
        with self.assertRaises(caddy_headers.HeadersError):
            caddy_headers.parse("  X-Thing: one\n")

    def test_a_pattern_other_than_every_path_is_refused(self) -> None:
        with self.assertRaises(caddy_headers.HeadersError):
            caddy_headers.directives(caddy_headers.parse("/assets/*\n  X-Thing: one\n"))

    def test_a_value_caddy_cannot_quote_is_refused(self) -> None:
        with self.assertRaises(caddy_headers.HeadersError):
            caddy_headers.directives([("/*", [("X-Thing", 'a "quoted" word')])])


class Wait(unittest.TestCase):
    def test_a_page_without_isolation_is_not_ready(self) -> None:
        class Answer:
            def __init__(self, headers: dict[str, str]) -> None:
                self.headers = headers
                self.status = 200

            def __enter__(self):
                return self

            def __exit__(self, *_: object) -> None:
                return None

        class Opener:
            def open(self, url: str, timeout: int):
                return Answer({} if url.endswith("/") else {"x": "y"})

        with self.assertRaises(selfhost_wait.NotReady):
            selfhost_wait.check(Opener())


if __name__ == "__main__":
    unittest.main()
