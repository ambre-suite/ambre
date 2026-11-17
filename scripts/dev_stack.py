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
  Then it puts one object through a link the relay presigns, and reads it back
  through another: both links must lead under `/s3/` of the same origin, where
  Trunk forwards them to Garage. The CI `devcontainer` job runs it against the
  container it built.

Everything the script writes sits under `target/dev-stack/`, which git ignores.

Run: `just phone-stack <host>` inside the dev container, `just phone-stack-probe`.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import ipaddress
import json
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
# The path of the origin that Trunk forwards to Garage (`Trunk.toml`). The relay
# moves each presigned link under it (`S3_PUBLIC_ENDPOINT`, `SRV-265`).
STORE_PATH = "/s3"
# The account the media probe signs in as, through the login of development mode.
PROBE_USER = "dev-stack-probe"
# A sealed document key holds a nonce, a key and a tag: 24, 32 and 16 bytes.
WRAPPED_KEY_BYTES = 72
PROBE_OBJECT_BYTES = 64

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
    """One HTTPS answer: its status, its headers by lowercase name, and its body."""

    status: int
    headers: dict[str, str]
    body: bytes = b""


def send(
    url: str,
    context: ssl.SSLContext,
    method: str = "GET",
    body: bytes | None = None,
    headers: dict[str, str] | None = None,
) -> Answer:
    """One request to `url` under `context`. A refused status is an answer; no answer is an error."""
    request = urllib.request.Request(url, data=body, method=method, headers=headers or {})
    try:
        with urllib.request.urlopen(request, context=context, timeout=30) as response:
            return Answer(
                response.status, {k.lower(): v for k, v in response.headers.items()}, response.read()
            )
    except urllib.error.HTTPError as error:
        # The error is the response too: close it once read.
        with error:
            return Answer(
                error.code, {k.lower(): v for k, v in error.headers.items()}, error.read()
            )
    except (urllib.error.URLError, OSError) as error:
        raise StackError(f"{url}: {getattr(error, 'reason', error)}") from None


def fetch(url: str, context: ssl.SSLContext) -> Answer:
    """A GET of `url` under `context`."""
    return send(url, context)


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


class Breach(Exception):
    """A step of the media probe that answered wrong. The steps after it depend on it."""


def call_relay(origin: str, context: ssl.SSLContext, route: str, payload: dict, token: str | None = None) -> dict:
    """A JSON `POST` to the relay at `route`, through the origin. Any status but `200` is a breach."""
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    answer = send(f"{origin}/api{route}", context, "POST", json.dumps(payload).encode(), headers)
    if answer.status != 200:
        raise Breach(f"the relay answered POST /api{route} with HTTP {answer.status}: {answer.body[:200]!r}")
    try:
        return json.loads(answer.body)
    except ValueError:
        raise Breach(f"the relay answered POST /api{route} with no JSON") from None


def link_under(origin: str, link: object, kind: str) -> str:
    """`link`, once it leads under the store path of `origin`, where a phone reaches it."""
    base = f"{origin}{STORE_PATH}/"
    if not isinstance(link, str) or not link.startswith(base):
        raise Breach(f"the relay signs the {kind} link {str(link).split('?')[0]!r}, not under {base}")
    return link


def media_steps(origin: str, context: ssl.SSLContext) -> None:
    """Signs in, makes a document, puts one object through a presigned link and reads it back.

    The object is random bytes, as a sealed blob is to the relay: the probe needs no key.
    """
    login = call_relay(
        origin,
        context,
        "/auth/dev/login",
        {
            "username": PROBE_USER,
            "signing_pub_key": os.urandom(32).hex(),
            "exchange_pub_key": os.urandom(32).hex(),
        },
    )
    token = login.get("access_token")
    if not isinstance(token, str):
        raise Breach("the dev login answered no access token")
    sealed_key = base64.urlsafe_b64encode(os.urandom(WRAPPED_KEY_BYTES)).decode()
    document = call_relay(origin, context, "/documents", {"doc_type": "prose", "doc_key_enc": sealed_key}, token)
    doc = document.get("document_id")
    blob = os.urandom(PROBE_OBJECT_BYTES)
    named = {"content_hash": hashlib.sha256(blob).hexdigest(), "size_bytes": len(blob)}
    upload = call_relay(origin, context, f"/documents/{doc}/media/upload-url", named, token)
    put = send(link_under(origin, upload.get("upload_url"), "upload"), context, "PUT", blob)
    if put.status != 200:
        raise Breach(f"the store refused the upload through the presigned link: HTTP {put.status}: {put.body[:300]!r}")
    call_relay(origin, context, f"/documents/{doc}/media", named, token)
    download = call_relay(
        origin, context, f"/documents/{doc}/media/download-url", {"content_hash": named["content_hash"]}, token
    )
    got = send(link_under(origin, download.get("download_url"), "download"), context)
    if got.status != 200:
        raise Breach(f"the store refused the download through the presigned link: HTTP {got.status}")
    if got.body != blob:
        raise Breach(f"the store gave back {len(got.body)} bytes that differ from the {len(blob)} put")


def media_breaches(origin: str, authority: Path) -> list[str]:
    """The first breach of the media probe at `origin`, trusting `authority` alone; none when it passes."""
    context = ssl.create_default_context(cafile=str(authority))
    urllib.request.install_opener(urllib.request.build_opener(urllib.request.ProxyHandler({})))
    try:
        media_steps(origin, context)
    except Breach as breach:
        return [str(breach)]
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
    origin = f"https://{host or 'localhost'}:{PORT}"
    # The relay signs for the store at `S3_ENDPOINT`, and hands each link out
    # under the origin of the page, where Trunk forwards it (`SRV-265`).
    env = {**os.environ, "S3_PUBLIC_ENDPOINT": os.environ.get("S3_PUBLIC_ENDPOINT", f"{origin}{STORE_PATH}")}
    with paths.relay_log.open("wb") as log:
        relay = subprocess.Popen(
            [str(target / "debug" / "ambre-server")], cwd=ROOT, stdout=log, stderr=log, env=env
        )
    try:
        wait_for_relay(relay, paths.relay_log)
        print(f"relay: up, its log in {paths.relay_log}", flush=True)
        print(f"client: {origin} once Trunk has built; trust {paths.authority} first", flush=True)
        trunk = shutil.which("trunk") or "trunk"
        return subprocess.run(
            [
                # The stack signs in by a bare name: the development sign-in is a
                # feature no release build turns on (`ZK-045`).
                trunk, "serve", "--features", "dev-login",
                "--address", "0.0.0.0", "--port", str(PORT), "--open", "false",
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
        if not breaches:
            breaches = media_breaches(args.origin, paths.authority)
    except StackError as error:
        print(f"dev-stack: {error}", file=sys.stderr)
        return 1
    for breach in breaches:
        print(f"dev-stack: {breach}", file=sys.stderr)
    if breaches:
        return 1
    print(
        f"dev-stack: {args.origin} serves the page cross-origin isolated, the relay answers,"
        " and an object goes through its presigned links"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
