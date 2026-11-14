#!/usr/bin/env python3
"""The test stack a phone reaches: the relay and the client, served over HTTPS from the dev container.

`.devcontainer/README.md` says how a person starts it and opens it on a phone. The
backing services run beside the dev container, from the images `docker-compose.yml`
pins (`.devcontainer/compose.yml`). This script runs inside the dev container and
has three verbs:

* `cert`: writes a local certificate authority once, then a server certificate for
  `localhost`, `127.0.0.1` and the address of `--host`. A phone trusts the authority
  once and every later certificate with it. The authority may sign private
  addresses and `.local` names alone, so its key, if it leaks, vouches for no public
  site.
* `up`: `cert`, then the Garage init, then the relay in the background, then
  `trunk serve` over TLS on every address of the container. Trunk serves the page
  with the two headers of `Trunk.toml` that make it cross-origin isolated, and
  forwards `/api/` to the relay.
* `probe`: one HTTPS request for the page and one for the relay through the same
  origin, the authority above as the only trust root. It fails when an answer is
  not `200`, or when the page lacks either header with the value of `Trunk.toml`.
  The CI `devcontainer` job runs it against the container it built.

Everything the script writes sits under `target/dev-stack/`, which git ignores.

Run: `just phone-stack <host>` inside the dev container, `just phone-stack-probe`.
"""

from __future__ import annotations

import argparse
import ipaddress
import os
import shutil
import ssl
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "target" / "dev-stack"
PORT = 8443
RELAY = "http://127.0.0.1:3000"
# A route of the relay that answers with no session: the sign-in options.
RELAY_PROBE = "/api/auth/config"
RELAY_WAIT_SECONDS = 120

# What the authority may sign: the private ranges of RFC 1918, loopback, and the
# `.local` names of mDNS. A phone on the same network reaches the host through one.
NETWORKS = [
    ipaddress.IPv4Network("10.0.0.0/8"),
    ipaddress.IPv4Network("172.16.0.0/12"),
    ipaddress.IPv4Network("192.168.0.0/16"),
    ipaddress.IPv4Network("127.0.0.0/8"),
]
PERMITTED = [f"IP:{net.network_address}/{net.netmask}" for net in NETWORKS] + [
    "DNS:localhost",
    "DNS:local",
]
# The longest validity a phone accepts for a server certificate: iOS refuses more
# than 398 days.
SERVER_DAYS = 397
AUTHORITY_DAYS = 825


class StackError(Exception):
    """A step of the stack that failed, with the reason."""


@dataclass(frozen=True)
class Paths:
    """The files of the stack under one directory."""

    root: Path

    @property
    def authority(self) -> Path:
        return self.root / "ca.pem"

    @property
    def authority_key(self) -> Path:
        return self.root / "ca.key"

    @property
    def cert(self) -> Path:
        return self.root / "cert.pem"

    @property
    def key(self) -> Path:
        return self.root / "key.pem"

    @property
    def names(self) -> Path:
        """The subject names the server certificate was written for, one per line."""
        return self.root / "cert.names"

    @property
    def relay_log(self) -> Path:
        return self.root / "relay.log"


def subject_names(host: str | None) -> list[str]:
    """The subject names of the server certificate: loopback, and `host` when given.

    `host` must be a private IPv4 address or a `.local` name, which the authority may
    sign. Any other value is refused here rather than by the phone.
    """
    names = ["DNS:localhost", "IP:127.0.0.1"]
    if not host:
        return names
    try:
        address = ipaddress.ip_address(host)
    except ValueError:
        if not host.endswith(".local") or host.startswith("."):
            raise StackError(f"--host {host}: neither an IP address nor a `.local` name") from None
        return names + [f"DNS:{host}"]
    if not any(address in net for net in NETWORKS):
        raise StackError(f"--host {host}: outside the private ranges the authority may sign")
    if address.is_loopback:
        return names
    return names + [f"IP:{host}"]


def authority_extensions() -> list[str]:
    """The `-addext` values of the authority: a CA that signs servers in `PERMITTED` alone."""
    permitted = ",".join(f"permitted;{name}" for name in PERMITTED)
    return [
        "basicConstraints=critical,CA:TRUE,pathlen:0",
        "keyUsage=critical,keyCertSign,cRLSign",
        f"nameConstraints=critical,{permitted}",
    ]


def server_extensions(names: list[str]) -> str:
    """The extension file of a server certificate for `names`."""
    return (
        "basicConstraints=critical,CA:FALSE\n"
        "keyUsage=critical,digitalSignature\n"
        "extendedKeyUsage=serverAuth\n"
        f"subjectAltName={','.join(names)}\n"
    )


def openssl(*args: str) -> None:
    """One `openssl` command, quiet unless it fails."""
    done = subprocess.run(["openssl", *args], capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise StackError(f"openssl {args[0]}: {done.stderr.strip()}")


def write_certificates(paths: Paths, host: str | None) -> bool:
    """The authority once, then a server certificate when its names changed. True: rewritten."""
    names = subject_names(host)
    paths.root.mkdir(parents=True, exist_ok=True)
    if not (paths.authority.is_file() and paths.authority_key.is_file()):
        args = [
            "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
            "-keyout", str(paths.authority_key), "-out", str(paths.authority),
            "-days", str(AUTHORITY_DAYS), "-subj", "/CN=Ambre dev stack authority",
        ]
        for ext in authority_extensions():
            args += ["-addext", ext]
        openssl(*args)
        paths.authority_key.chmod(0o600)
    wanted = "\n".join(names) + "\n"
    current = paths.names.read_text() if paths.names.is_file() else ""
    if current == wanted and paths.cert.is_file() and paths.key.is_file():
        return False
    request = paths.root / "server.csr"
    extensions = paths.root / "server.ext"
    extensions.write_text(server_extensions(names))
    openssl(
        "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
        "-keyout", str(paths.key), "-out", str(request), "-subj", "/CN=Ambre dev stack",
    )
    openssl(
        "x509", "-req", "-in", str(request), "-CA", str(paths.authority),
        "-CAkey", str(paths.authority_key), "-CAcreateserial",
        "-days", str(SERVER_DAYS), "-extfile", str(extensions), "-out", str(paths.cert),
    )
    paths.key.chmod(0o600)
    request.unlink()
    paths.names.write_text(wanted)
    return True


def isolation_headers(trunk_toml: str) -> dict[str, str]:
    """The two headers of `[serve.headers]` in `Trunk.toml` that make the page cross-origin isolated."""
    headers = tomllib.loads(trunk_toml).get("serve", {}).get("headers", {})
    wanted = {}
    for name in ("Cross-Origin-Opener-Policy", "Cross-Origin-Embedder-Policy"):
        if name not in headers:
            raise StackError(f"Trunk.toml: `[serve.headers]` lacks {name}")
        wanted[name] = headers[name]
    return wanted


@dataclass(frozen=True)
class Answer:
    """One HTTPS answer: its status and its headers, by lowercase name."""

    status: int
    headers: dict[str, str]


def fetch(url: str, context: ssl.SSLContext) -> Answer:
    """A GET of `url` under `context`. A refused status is an answer; no answer is an error."""
    try:
        with urllib.request.urlopen(url, context=context, timeout=30) as response:
            return Answer(response.status, {k.lower(): v for k, v in response.headers.items()})
    except urllib.error.HTTPError as error:
        return Answer(error.code, {k.lower(): v for k, v in error.headers.items()})
    except (urllib.error.URLError, OSError) as error:
        raise StackError(f"{url}: {getattr(error, 'reason', error)}") from None


def page_breaches(page: Answer, wanted: dict[str, str]) -> list[str]:
    """What the answer for the page lacks: a `200`, and each isolation header with its value."""
    found = []
    if page.status != 200:
        found.append(f"the page answered HTTP {page.status}")
    for name, value in wanted.items():
        got = page.headers.get(name.lower())
        if got != value:
            found.append(f"the page carries {name}: {got!r}, not {value!r}")
    return found


def relay_breaches(relay: Answer) -> list[str]:
    """What the answer for the relay lacks: a `200` from the route that needs no session."""
    if relay.status != 200:
        return [f"the relay answered {RELAY_PROBE} with HTTP {relay.status}"]
    return []


def probe(origin: str, authority: Path, trunk_toml: str) -> list[str]:
    """Each breach of the page and of the relay at `origin`, trusting `authority` alone."""
    if not authority.is_file():
        raise StackError(f"{authority}: no authority yet; the stack writes it first")
    context = ssl.create_default_context(cafile=str(authority))
    # The stack is local: a proxy of the environment must not answer for it.
    urllib.request.install_opener(urllib.request.build_opener(urllib.request.ProxyHandler({})))
    wanted = isolation_headers(trunk_toml)
    return page_breaches(fetch(f"{origin}/", context), wanted) + relay_breaches(
        fetch(f"{origin}{RELAY_PROBE}", context)
    )


def probe_until(origin: str, authority: Path, trunk_toml: str, seconds: int) -> list[str]:
    """`probe`, again each 5 s while the origin does not answer, for `seconds` at most.

    Trunk builds the client before it serves anything, which takes minutes cold. A
    breach of an answer is final: the stack answered, and answered wrong.
    """
    deadline = time.monotonic() + seconds
    while True:
        try:
            return probe(origin, authority, trunk_toml)
        except StackError as error:
            if time.monotonic() >= deadline:
                return [str(error)]
            time.sleep(5)


def wait_for_relay(process: subprocess.Popen[bytes], log: Path) -> None:
    """Returns once the relay answers. Fails when it exits or stays silent past the wait."""
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    for _ in range(RELAY_WAIT_SECONDS):
        if process.poll() is not None:
            raise StackError(f"the relay exited with {process.returncode}; read {log}")
        try:
            with opener.open(f"{RELAY}{RELAY_PROBE}", timeout=5):
                return
        except (urllib.error.URLError, OSError):
            time.sleep(1)
    raise StackError(f"the relay did not answer in {RELAY_WAIT_SECONDS} s; read {log}")


def up(paths: Paths, host: str | None) -> int:
    """The whole stack. Returns when Trunk stops, and stops the relay with it."""
    write_certificates(paths, host)
    if subprocess.run([sys.executable, str(ROOT / "scripts" / "garage_init.py")], check=False).returncode:
        raise StackError("the Garage init failed")
    if subprocess.run(["cargo", "build", "-p", "ambre-server"], cwd=ROOT, check=False).returncode:
        raise StackError("the relay did not build")
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    with paths.relay_log.open("wb") as log:
        relay = subprocess.Popen([str(target / "debug" / "ambre-server")], cwd=ROOT, stdout=log, stderr=log)
    try:
        wait_for_relay(relay, paths.relay_log)
        print(f"relay: up, its log in {paths.relay_log}", flush=True)
        origin = f"https://{host or 'localhost'}:{PORT}"
        print(f"client: {origin} once Trunk has built; trust {paths.authority} first", flush=True)
        trunk = shutil.which("trunk") or "trunk"
        return subprocess.run(
            [
                trunk, "serve", "--address", "0.0.0.0", "--port", str(PORT), "--open", "false",
                "--tls-cert-path", str(paths.cert), "--tls-key-path", str(paths.key),
            ],
            cwd=ROOT,
            check=False,
        ).returncode
    finally:
        relay.terminate()
        relay.wait(timeout=30)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    verbs = parser.add_subparsers(dest="verb", required=True)
    for verb in ("cert", "up"):
        sub = verbs.add_parser(verb)
        sub.add_argument("--host", help="the address of this machine on the network of the phone")
    sub = verbs.add_parser("probe")
    sub.add_argument("--origin", default=f"https://localhost:{PORT}")
    sub.add_argument("--wait", type=int, default=0, help="seconds to wait for a first answer")
    args = parser.parse_args(argv)
    paths = Paths(OUT)
    try:
        if args.verb == "cert":
            rewritten = write_certificates(paths, args.host)
            print(f"{'wrote' if rewritten else 'kept'} {paths.cert}; the authority is {paths.authority}")
            return 0
        if args.verb == "up":
            return up(paths, args.host)
        trunk_toml = (ROOT / "Trunk.toml").read_text()
        breaches = probe_until(args.origin, paths.authority, trunk_toml, args.wait)
    except StackError as error:
        print(f"dev-stack: {error}", file=sys.stderr)
        return 1
    for breach in breaches:
        print(f"dev-stack: {breach}", file=sys.stderr)
    if breaches:
        return 1
    print(f"dev-stack: {args.origin} serves the page cross-origin isolated, and the relay answers")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
