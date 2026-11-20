#!/usr/bin/env python3
"""Writes the `_headers` of a client build as Caddy directives.

The release hook writes `_headers` beside the page, in the format Netlify and
Cloudflare Pages read (`scripts/trunk_post_build.py`). Caddy reads no such
file, so the image of the client carries the same headers as directives. One
source keeps them: the build, and never a copy typed by hand. Each header is
added with `+`, so an answer that two rules cover carries both, as the belt
server and Netlify give it (`crates/ambre-belt/src/serve.rs`, `apply_rules`).

Usage: caddy_headers.py <_headers>, which prints the directives.
"""

from __future__ import annotations

import sys
from pathlib import Path


class HeadersError(Exception):
    """A line of `_headers` this script does not read."""


def parse(text: str) -> list[tuple[str, list[tuple[str, str]]]]:
    """Each rule of `text`: its path, then its headers in order."""
    rules: list[tuple[str, list[tuple[str, str]]]] = []
    for number, line in enumerate(text.splitlines(), start=1):
        if not line.strip():
            continue
        if not line[0].isspace():
            rules.append((line.strip(), []))
            continue
        if not rules:
            raise HeadersError(f"line {number}: a header before any path")
        name, sep, value = line.strip().partition(":")
        if not sep or not name or not value.strip():
            raise HeadersError(f"line {number}: no `Name: value`")
        rules[-1][1].append((name.strip(), value.strip()))
    return rules


def quote(value: str) -> str:
    """`value` as one Caddyfile token."""
    if '"' in value or "\n" in value:
        raise HeadersError(f"a value Caddy cannot quote: {value!r}")
    return f'"{value}"'


def directives(rules: list[tuple[str, list[tuple[str, str]]]]) -> str:
    """The Caddy directives that give each answer the headers of its rules."""
    out: list[str] = []
    for index, (path, headers) in enumerate(rules):
        if path == "/*":
            matcher = ""
        else:
            if "*" in path:
                raise HeadersError(f"a pattern this script does not map: {path}")
            name = f"@ambre_headers_{index}"
            out.append(f"{name} path {path}")
            matcher = f"{name} "
        for header, value in headers:
            out.append(f"header {matcher}+{header} {quote(value)}")
    return "\n".join(out) + "\n"


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: caddy_headers.py <_headers>", file=sys.stderr)
        return 2
    try:
        sys.stdout.write(directives(parse(Path(argv[1]).read_text(encoding="utf-8"))))
    except (OSError, HeadersError) as error:
        print(f"caddy_headers: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
