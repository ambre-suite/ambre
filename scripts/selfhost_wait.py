#!/usr/bin/env python3
"""Waits until the self-hosted stack under test serves its page and its relay.

`just selfhost-up` starts the stack of `deploy/compose/compose.yml` with a
certificate of Caddy's own authority (`SRV-310`). This script reads that
authority out of the `web` container and trusts it alone, so every request
below checks the certificate the front serves. It then waits for two answers
through the front on port 443:

1. the page, which must carry the isolation headers of its build;
2. the configuration of the relay, through `/api`.

Usage: selfhost_wait.py "<docker compose command of the stack>" [seconds]
"""

from __future__ import annotations

import shlex
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

ORIGIN = "https://localhost"
ROOT = "/data/caddy/pki/authorities/local/root.crt"
ISOLATION = {
    "Cross-Origin-Opener-Policy": "same-origin",
    "Cross-Origin-Embedder-Policy": "credentialless",
}


class NotReady(Exception):
    """An answer the stack does not give yet, or gives wrong."""


def authority(compose: list[str]) -> str | None:
    """The root certificate of Caddy's authority, once Caddy has written it."""
    result = subprocess.run(
        [*compose, "exec", "-T", "web", "cat", ROOT],
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout if result.returncode == 0 and "BEGIN CERTIFICATE" in result.stdout else None


def opener(cafile: str) -> urllib.request.OpenerDirector:
    """An opener that trusts `cafile` alone and goes through no proxy."""
    context = ssl.create_default_context(cafile=cafile)
    return urllib.request.build_opener(
        urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=context)
    )


def check(open_url: urllib.request.OpenerDirector) -> None:
    """Raises `NotReady` until the page and the relay both answer as they must."""
    try:
        with open_url.open(f"{ORIGIN}/", timeout=5) as page:
            for name, value in ISOLATION.items():
                if page.headers.get(name) != value:
                    raise NotReady(f"the page carries {name}: {page.headers.get(name)!r}")
        with open_url.open(f"{ORIGIN}/api/auth/config", timeout=5) as relay:
            if relay.status != 200:
                raise NotReady(f"the relay answers {relay.status}")
    except (urllib.error.URLError, ConnectionError, TimeoutError, ssl.SSLError) as error:
        raise NotReady(str(error)) from error


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 3):
        print(__doc__.strip().splitlines()[-1], file=sys.stderr)
        return 2
    compose = shlex.split(argv[1])
    deadline = time.monotonic() + (int(argv[2]) if len(argv) == 3 else 300)
    last = "no answer yet"
    with tempfile.NamedTemporaryFile("w", suffix=".pem") as cafile:
        while time.monotonic() < deadline:
            root = authority(compose)
            if root is None:
                last = "Caddy has written no authority yet"
            else:
                cafile.seek(0)
                cafile.truncate()
                cafile.write(root)
                cafile.flush()
                try:
                    check(opener(cafile.name))
                    print(f"selfhost: {ORIGIN} serves the page and the relay")
                    return 0
                except NotReady as error:
                    last = str(error)
            time.sleep(2)
    print(f"selfhost: the stack is not ready: {last}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
