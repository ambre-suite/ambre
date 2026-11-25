#!/usr/bin/env python3
"""The link key slot of the client: the patch of an install, and its gates.

The relay signs a receipt of each thread link under a key of one purpose,
and a census refuses to erase a thread under a receipt that the client's key
verifies (`SYNC-330`, `SRV-363`). The published client pins no key. Its Sync
worker reserves one slot instead: 16 marker bytes, then 32 key bytes, which a
release publishes as zeros (`ZK-057`, `ZK-058`). Each install writes the
public key of its own relay into that slot, with no rebuild. A client whose
slot stays empty counts no receipt and names no thread.

Commands:

* `patch DIR [--dev]` writes the key that `THREAD_LINK_KEY` names into the
  one slot of the wasm files under `DIR`, a writable copy of the served
  files. It refuses each of these, and writes nothing then (`SRV-366`):
  * no key, or an empty one;
  * a key that is not 64 hexadecimal digits;
  * the development key, unless `--dev` is given;
  * no slot, or more than one, under `DIR`;
  * a slot that already holds a key, or that ends before its 32 bytes;
  * a `_headers` under `DIR` that does not serve the patched file under
    `Cache-Control: no-cache`, so a browser would keep an older key
    (`SRV-372`).
  It then writes again the `.br` and `.gz` copies of the patched file. No
  other byte of `DIR` changes (`SRV-367`).
* `empty DIST` fails unless `DIST` holds exactly one slot, all zero: the gate
  of a release build (`ZK-057`).
* `read FILE` prints the key the slot of `FILE` holds, or `empty`.
* `zero FILE OUT` writes `FILE` with its slot zeroed to `OUT`. A reader then
  compares the digest of `OUT` with the release manifest (`ZK-059`).
* `served ORIGIN` fetches the Sync worker that `ORIGIN` serves and fails
  unless its slot holds the key of `THREAD_LINK_KEY`. It also fails unless
  the answer carries `Cache-Control: no-cache` and an `ETag` (`SRV-373`). It
  trusts any certificate, so it serves the tests of a stack under an
  internal authority.
* `lint` reads the build, the release, the compose file and the Rust sources,
  and fails on each of these:
  * the marker or the development key of this script differs from the Rust
    source that declares it;
  * `deploy/Dockerfile` or `.github/workflows/release.yml` names
    `AMBRE_THREAD_LINK_KEY` or `vars.THREAD_LINK_KEY`, so a release pins a
    key at build time;
  * a `trunk build` of the `web-build` stage that no `empty /out/dist`
    follows in its `RUN`;
  * an `init` stage that does not carry this script and the `brotli` command;
  * a compose `web` service that serves another volume than the one a
    `link-key` service of the init image patches first (`SRV-369`).

The key is public, and the seed stays with the relay of the install. The
patch needs only the Python standard library, and `brotli` for a `.br` copy,
since the init image carries no more. Run: `just link-key` for the gate.
"""

from __future__ import annotations

import gzip
import os
import re
import shutil
import ssl
import subprocess
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCKERFILE = ROOT / "deploy/Dockerfile"
RELEASE = ROOT / ".github/workflows/release.yml"
COMPOSE = ROOT / "deploy/compose/compose.yml"
PROTOCOL = ROOT / "crates/ambre-protocol/src/thread_erasure.rs"
ERASURE = ROOT / "crates/ambre-sync/src/erasure.rs"

VARIABLE = "THREAD_LINK_KEY"
# The two spellings of a key named at build time: the old build argument, and
# the repository variable a release once read it from.
BUILD_TIME = ("AMBRE_THREAD_LINK_KEY", "vars.THREAD_LINK_KEY")
# The 16 bytes that open the slot, `LINK_KEY_SLOT_MARKER` of
# `crates/ambre-sync/src/erasure.rs`. The init image carries no source, so the
# script holds a copy, and `lint` compares the two.
MARKER = bytes.fromhex("461176fb8d6aafeb989288040c4591f9")
KEY_LEN = 32
# The public half of the dev seed of `0x4c` bytes, `DEV_LINK_KEY` of
# `crates/ambre-protocol/src/thread_erasure.rs`, held the same way.
DEV_KEY = "927308f53d6765503d2b64c103ab5aa47206ab39502cfe8f98442fdb543100b7"
HEX = re.compile(r"[0-9a-fA-F]{64}")
EMPTY_GATE = "scripts/link_key.py empty /out/dist"
# The headers file of the build (`scripts/trunk_post_build.py`), and the cache
# rule it gives the patched file: the browser asks again before each use, so a
# rotated key reaches it at the next load (`SRV-371`, `SRV-372`).
HEADERS = "_headers"
REVALIDATE = "no-cache"


class SlotError(Exception):
    """A tree or a key that no patch may write."""


def key_refusal(value: str | None, dev: bool) -> str | None:
    """Why a patch refuses `value` as the link key, or `None` when it takes it."""
    if not value or not value.strip():
        return f"{VARIABLE} is not set: name the public link key of the relay of this install"
    value = value.strip()
    if not HEX.fullmatch(value):
        return f"{VARIABLE} must be 64 hex digits"
    if value.lower() == DEV_KEY and not dev:
        return f"{VARIABLE} names the development key, under which any member signs a receipt"
    return None


def wasm_files(tree: Path) -> list[Path]:
    """The wasm files under `tree`, by path."""
    return sorted(p for p in tree.rglob("*.wasm") if p.is_file())


def find_slot(tree: Path) -> tuple[Path, int]:
    """The one wasm file under `tree` that holds a slot, and the offset of its key bytes."""
    hits = [(path, m.end()) for path in wasm_files(tree) for m in re.finditer(re.escape(MARKER), path.read_bytes())]
    if not hits:
        raise SlotError(f"no link key slot under {tree}: the client is not a build of Ambre, or not a copy of one")
    if len(hits) > 1:
        where = ", ".join(f"{p.relative_to(tree)}@{at}" for p, at in hits)
        raise SlotError(f"{len(hits)} link key slots under {tree}, not one: {where}")
    path, at = hits[0]
    if at + KEY_LEN > path.stat().st_size:
        raise SlotError(f"the slot of {path.relative_to(tree)} ends before its {KEY_LEN} key bytes")
    return hits[0]


def slot_bytes(data: bytes) -> bytes:
    """The 32 key bytes of the one slot of `data`."""
    count = data.count(MARKER)
    if count != 1:
        raise SlotError(f"{count} link key slots, not one")
    at = data.index(MARKER) + len(MARKER)
    if at + KEY_LEN > len(data):
        raise SlotError(f"the slot ends before its {KEY_LEN} key bytes")
    return data[at : at + KEY_LEN]


def with_slot(data: bytes, key: bytes) -> bytes:
    """`data` with the key bytes of its one slot replaced by `key`."""
    slot_bytes(data)
    at = data.index(MARKER) + len(MARKER)
    return data[:at] + key + data[at + KEY_LEN :]


def empty_refusal(dist: Path) -> str | None:
    """Why `dist` is no release: a slot missing, doubled or holding a key (`ZK-057`)."""
    try:
        path, at = find_slot(dist)
    except SlotError as e:
        return str(e)
    if path.read_bytes()[at : at + KEY_LEN] != bytes(KEY_LEN):
        return f"the slot of {path.relative_to(dist)} holds a key: a release publishes it empty"
    return None


def revalidated(headers: str, name: str) -> bool:
    """Whether `headers`, a `_headers` text, serves the file `name` under `REVALIDATE`."""
    rule: str | None = None
    for line in headers.splitlines():
        if line and not line[0].isspace():
            rule = line.strip()
            continue
        header, _, value = line.strip().partition(":")
        if rule is not None and rule.endswith("/" + name) and header.strip().lower() == "cache-control":
            if REVALIDATE in [v.strip().lower() for v in value.split(",")]:
                return True
    return False


def headers_refusal(tree: Path, path: Path) -> str | None:
    """Why a browser could keep an older copy of `path` under `tree`, or `None` (`SRV-372`)."""
    name = path.relative_to(tree).as_posix()
    headers = tree / HEADERS
    if not headers.is_file():
        return f"no {HEADERS} under {tree}: the web server would let a browser keep the older key"
    if not revalidated(headers.read_text(encoding="utf-8"), name):
        return f"{HEADERS} does not serve {name} under Cache-Control: {REVALIDATE}, so a browser keeps its older key"
    return None


def recompress(path: Path) -> list[Path]:
    """Write again each `.br` and `.gz` copy beside `path`, and return them."""
    written = []
    br, gz = Path(f"{path}.br"), Path(f"{path}.gz")
    if br.exists():
        subprocess.run(["brotli", "--quality=11", "--force", f"--output={br}", str(path)], check=True)
        written.append(br)
    if gz.exists():
        gz.write_bytes(gzip.compress(path.read_bytes(), compresslevel=9, mtime=0))
        written.append(gz)
    return written


def patch(tree: Path, value: str | None, dev: bool) -> Path:
    """Write the key `value` names into the one slot under `tree`, and return the file (`SRV-366`)."""
    why = key_refusal(value, dev)
    if why:
        raise SlotError(why)
    path, at = find_slot(tree)
    data = path.read_bytes()
    if data[at : at + KEY_LEN] != bytes(KEY_LEN):
        raise SlotError(f"the slot of {path.relative_to(tree)} holds a key: patch a copy of the published files")
    stale = headers_refusal(tree, path)
    if stale:
        raise SlotError(stale)
    if Path(f"{path}.br").exists() and shutil.which("brotli") is None:
        raise SlotError(f"{path.name}.br needs the brotli command, and none is on the path")
    path.write_bytes(data[:at] + bytes.fromhex(value.strip()) + data[at + KEY_LEN :])
    recompress(path)
    return path


def sync_worker(origin: str) -> bytes:
    """The wasm of the Sync worker that `origin` serves, found through its page.

    It fails unless the answer makes the browser ask again before each use
    (`SRV-373`): `Cache-Control: no-cache`, and an `ETag` the web server
    changes with the file.
    """
    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE

    def get(url: str) -> tuple[bytes, object]:
        request = urllib.request.Request(url, headers={"Accept-Encoding": "identity"})
        with urllib.request.urlopen(request, context=context, timeout=30) as answer:
            return answer.read(), answer.headers

    page = get(origin.rstrip("/") + "/")[0].decode()
    found = re.search(r'<meta name="ambre-sync-worker" content="([^"]+)"', page)
    if found is None or not found.group(1).endswith("_loader.js"):
        raise SlotError(f"the page of {origin} names no Sync worker")
    loader = found.group(1)
    url = loader if loader.startswith("http") else origin.rstrip("/") + "/" + loader.lstrip("/")
    wasm, headers = get(url.removesuffix("_loader.js") + "_bg.wasm")
    control = [v.strip().lower() for v in (headers.get("Cache-Control") or "").split(",")]
    if REVALIDATE not in control or not headers.get("ETag"):
        raise SlotError(
            f"the served Sync worker answers Cache-Control {headers.get('Cache-Control')!r} and ETag "
            f"{headers.get('ETag')!r}: a browser may keep an older key"
        )
    return wasm


def dev_key_of(source: str) -> str:
    """The development link key that `source`, the protocol file, declares, in hex."""
    found = re.search(r"pub const DEV_LINK_KEY\b[^\[]*\[(.*?)\]", source, re.S)
    if found is None:
        raise ValueError("no DEV_LINK_KEY array in the protocol file")
    return "".join(re.findall(r"0x([0-9a-fA-F]{2})", found.group(1))).lower()


def marker_of(source: str) -> bytes:
    """The marker that `source`, the erasure file of the Sync worker, declares."""
    found = re.search(r"const LINK_KEY_SLOT_MARKER\b[^=]*=\s*\[(.*?)\]", source, re.S)
    if found is None:
        raise ValueError("no LINK_KEY_SLOT_MARKER array in the erasure file")
    return bytes.fromhex("".join(re.findall(r"0x([0-9a-fA-F]{2})", found.group(1))))


def check_sources(erasure: str, protocol: str) -> list[str]:
    """The faults of the copies this script holds against the Rust sources."""
    faults = []
    if marker_of(erasure) != MARKER:
        faults.append("MARKER differs from LINK_KEY_SLOT_MARKER of crates/ambre-sync/src/erasure.rs")
    if dev_key_of(protocol) != DEV_KEY:
        faults.append("DEV_KEY differs from DEV_LINK_KEY of crates/ambre-protocol/src/thread_erasure.rs")
    return faults


def stage(dockerfile: str, name: str) -> list[str]:
    """The lines of the stage `name` of `dockerfile`, joined over each line break escape."""
    joined = re.sub(r"\\\n", " ", dockerfile)
    lines: list[str] = []
    inside = False
    for line in joined.splitlines():
        head = re.match(r"\s*FROM\s+\S+\s+AS\s+(\S+)", line, re.I)
        if head:
            inside = head.group(1) == name
            continue
        if inside:
            lines.append(line)
    return lines


def check_dockerfile(dockerfile: str) -> list[str]:
    """The faults of `deploy/Dockerfile` against `ZK-057` and `SRV-365`."""
    faults = []
    for name in BUILD_TIME:
        if name in dockerfile:
            faults.append(f"Dockerfile: names {name}, so a build pins a key the published slot must not hold")
    lines = stage(dockerfile, "web-build")
    builds = [line for line in lines if "trunk build" in line]
    if not builds:
        faults.append("Dockerfile web-build: no `trunk build`, so nothing builds the client")
    for line in builds:
        if EMPTY_GATE not in line[line.rindex("trunk build") :]:
            faults.append(f"Dockerfile web-build: a client builds with no `{EMPTY_GATE}` after it")
    init = " ".join(stage(dockerfile, "init"))
    if not re.search(r"COPY\s+scripts/link_key\.py\s+/opt/ambre/link_key\.py", init):
        faults.append("Dockerfile init: no COPY of scripts/link_key.py to /opt/ambre/link_key.py, so no pod patches the slot")
    if not re.search(r"apt-get install[^;]*\bbrotli\b", init):
        faults.append("Dockerfile init: installs no brotli, so a patch cannot write the .br copy again")
    return faults


def check_release(text: str) -> list[str]:
    """The faults of the release workflow against `ZK-057`."""
    return [
        f"release.yml: names {name}, so a release pins a key the published slot must not hold"
        for name in BUILD_TIME
        if name in text
    ]


def check_compose(compose: dict) -> list[str]:
    """The faults of the compose file against `SRV-369`."""
    services = compose.get("services") or {}
    web, patcher = services.get("web") or {}, services.get("link-key")
    faults = []
    if any(VARIABLE in str(k) for k in ((web.get("build") or {}).get("args") or {})):
        faults.append(f"compose web: a build argument names {VARIABLE}")
    if not isinstance(patcher, dict):
        return [*faults, "compose: no link-key service, so the web server serves an empty slot"]
    served = [str(v).split(":")[0] for v in web.get("volumes") or [] if str(v).split(":")[1:2] == ["/srv/ambre"]]
    patched = [str(v).split(":")[0] for v in patcher.get("volumes") or []]
    if not served or served[0] not in patched:
        faults.append("compose web: /srv/ambre is not the volume the link-key service patches")
    if "ambre-init" not in str(patcher.get("image", "")):
        faults.append("compose link-key: runs no init image, which alone carries the patch")
    command = " ".join(map(str, patcher.get("command") or [])) + str(patcher.get("entrypoint", ""))
    if "link_key.py" not in command or "patch" not in command:
        faults.append("compose link-key: runs no `link_key.py patch`")
    if f"${{{VARIABLE}" not in str((patcher.get("environment") or {}).get(VARIABLE, "")):
        faults.append(f"compose link-key: {VARIABLE} does not come from the .env")
    after = (web.get("depends_on") or {}).get("link-key") if isinstance(web.get("depends_on"), dict) else None
    if not after or after.get("condition") != "service_completed_successfully":
        faults.append("compose web: starts before the link-key service completes")
    return faults


def lint() -> list[str]:
    """Every fault of the tree against the rules of this script."""
    try:
        import yaml
    except ImportError:  # pragma: no cover - the message is the whole behaviour
        sys.exit("link_key.py lint needs PyYAML: install python3-yaml")
    return (
        check_sources(ERASURE.read_text(), PROTOCOL.read_text())
        + check_dockerfile(DOCKERFILE.read_text())
        + check_release(RELEASE.read_text())
        + check_compose(yaml.safe_load(COMPOSE.read_text()))
    )


def main(args: list[str]) -> int:
    try:
        if args[:1] == ["patch"] and len(args) in (2, 3) and args[2:] in ([], ["--dev"]):
            path = patch(Path(args[1]), os.environ.get(VARIABLE), args[2:] == ["--dev"])
            print(f"link_key: wrote {os.environ[VARIABLE].strip().lower()} into the slot of {path.name}")
            return 0
        if args[:1] == ["empty"] and len(args) == 2:
            why = empty_refusal(Path(args[1]))
            if why:
                raise SlotError(why)
            return 0
        if args[:1] == ["read"] and len(args) == 2:
            key = slot_bytes(Path(args[1]).read_bytes())
            print("empty" if key == bytes(KEY_LEN) else key.hex())
            return 0
        if args[:1] == ["zero"] and len(args) == 3:
            Path(args[2]).write_bytes(with_slot(Path(args[1]).read_bytes(), bytes(KEY_LEN)))
            return 0
        if args[:1] == ["served"] and len(args) == 2:
            named = (os.environ.get(VARIABLE) or "").strip().lower()
            held = slot_bytes(sync_worker(args[1])).hex()
            if held != named:
                raise SlotError(f"the served client holds {held}, and {VARIABLE} names {named or 'none'}")
            print(f"link_key: the served client holds {held}")
            return 0
        if args == ["lint"]:
            faults = lint()
            for fault in faults:
                print(f"link_key: {fault}", file=sys.stderr)
            return 1 if faults else 0
    except SlotError as e:
        print(f"link_key: {e}", file=sys.stderr)
        return 1
    print("usage: link_key.py patch DIR [--dev] | empty DIST | read FILE | zero FILE OUT | served ORIGIN | lint", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
