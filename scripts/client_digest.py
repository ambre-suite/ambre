#!/usr/bin/env python3
"""The digest of the client build, and the proof that two builds give it.

The relay serves the client code, and a changed page reads every key. A user
or an auditor checks the served page only against a digest that anyone can
rebuild from the source (`ZK-038`, `ZK-039`, `ZK-040` of
`ambre-spec-zero-knowledge.md`).

**The manifest.** One line per file of the release `dist/`, sorted by path:
the BLAKE2b-256 digest of the file in hex, two spaces, the path. That is the
format of `b2sum -l 256`, so an auditor who downloads the served files checks
them with `b2sum -l 256 -c ambre-client.manifest`, and needs no tool of ours.
A `.br` copy is left out: it is a brotli encoding of the file beside it, and
its bytes depend on the version of the `brotli` command. The manifest refuses
a `.br` copy that does not decode to that file, so the copy is held all the
same. The **client digest** is the BLAKE2b-256 digest of the manifest.

**Two builds.** `reproduce` extracts one commit twice with `git archive`, into
two new directories of different paths outside the checkout. A directory
inside the checkout would merge the checkout's `.cargo/config.toml` into the
build. It runs `trunk build --release` in each, with no cache, and compares the
two manifests. The environment of each build drops every variable that
changes the compiler flags, the target directory or the profile, and sets
`SOURCE_DATE_EPOCH` to the commit time. The paths of the build machine leave
the wasm through `trim-paths` of the `wasm-release` profile (`Cargo.toml`).

Commands, from the repository root:

* `python3 scripts/client_digest.py manifest DIST` prints the manifest of
  `DIST`, then the client digest.
* `python3 scripts/client_digest.py build REV DIR` extracts `REV` into the new
  directory `DIR`, builds it, and writes `DIR/ambre-client.manifest`.
* `python3 scripts/client_digest.py compare A B` compares two manifests.
* `python3 scripts/client_digest.py reproduce [--rev REV] [--keep]` builds
  twice, compares, and writes the manifest and the digest into
  `target/client-digest/`. `just reproducible` runs it.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from collections.abc import Callable
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The Trunk release that writes the published bytes. Trunk writes the loader,
# the page and the hashed names, so another release writes another digest. The
# CI jobs that build the release install this one, and a test holds the pin.
TRUNK_VERSION = "0.22.1"

# Where `reproduce` writes the two published files, and their names.
OUT = ROOT / "target" / "client-digest"
MANIFEST_NAME = "ambre-client.manifest"
DIGEST_NAME = "ambre-client.digest"

# The suffix of the brotli copy the post-build hook writes beside a file.
BROTLI_SUFFIX = ".br"

# Variables that change what the compiler or Trunk writes. A build of the
# release reads none of them from the shell of the person who runs it.
DROPPED_PREFIXES = ("CARGO_PROFILE_", "CARGO_TARGET_", "CARGO_BUILD_", "TRUNK_")
DROPPED_NAMES = frozenset(
    {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_TARGET_DIR"}
)


class DigestError(Exception):
    """A build or a manifest that the published digest cannot come from."""


def file_digest(data: bytes) -> str:
    """The BLAKE2b-256 digest of `data` in hex, as `b2sum -l 256` prints it."""
    return hashlib.blake2b(data, digest_size=32).hexdigest()


def brotli_decode(path: Path) -> bytes:
    """The bytes the `brotli` command decodes from `path`."""
    if shutil.which("brotli") is None:
        raise DigestError("a release holds brotli copies, and no `brotli` command is on the path")
    result = subprocess.run(["brotli", "--decompress", "--stdout", str(path)], capture_output=True, check=False)
    if result.returncode != 0:
        raise DigestError(f"{path} is no brotli stream")
    return result.stdout


def manifest(dist: Path, decode: Callable[[Path], bytes] = brotli_decode) -> str:
    """The manifest of `dist`: one `digest  path` line per file, sorted by path, no `.br` copy."""
    if not dist.is_dir():
        raise DigestError(f"{dist} is no directory")
    files = sorted((p for p in dist.rglob("*") if p.is_file()), key=lambda p: p.relative_to(dist).as_posix())
    lines = []
    for path in files:
        name = path.relative_to(dist).as_posix()
        if "\n" in name or "\\" in name:
            raise DigestError(f"{name!r} cannot be a line of the manifest")
        if path.suffix == BROTLI_SUFFIX:
            source = path.with_name(path.name.removesuffix(BROTLI_SUFFIX))
            if not source.is_file():
                raise DigestError(f"{name} is a brotli copy of no file")
            if decode(path) != source.read_bytes():
                raise DigestError(f"{name} does not decode to the file beside it")
            continue
        lines.append(f"{file_digest(path.read_bytes())}  {name}\n")
    if not lines:
        raise DigestError(f"{dist} holds no file")
    return "".join(lines)


def client_digest(text: str) -> str:
    """The client digest: the BLAKE2b-256 digest of the manifest."""
    return file_digest(text.encode("utf-8"))


def parse_manifest(text: str) -> dict[str, str]:
    """The digest of each path of a manifest."""
    entries: dict[str, str] = {}
    for number, line in enumerate(text.splitlines(), start=1):
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        if match is None:
            raise DigestError(f"line {number} of the manifest is no `digest  path` line")
        entries[match.group(2)] = match.group(1)
    return entries


def differences(first: str, second: str) -> list[str]:
    """The paths whose digest differs between two manifests, or that one of them lacks."""
    one, two = parse_manifest(first), parse_manifest(second)
    return sorted(path for path in one.keys() | two.keys() if one.get(path) != two.get(path))


def build_env(base: dict[str, str], epoch: str) -> dict[str, str]:
    """The environment of a release build: no flag of the shell, the commit time as the build time."""
    env = {
        name: value
        for name, value in base.items()
        if name not in DROPPED_NAMES and not name.startswith(DROPPED_PREFIXES)
    }
    env["SOURCE_DATE_EPOCH"] = epoch
    env["CARGO_INCREMENTAL"] = "0"
    env["TZ"] = "UTC"
    env["LC_ALL"] = "C"
    return env


def trunk_version() -> str:
    """The version of the `trunk` command on the path."""
    if shutil.which("trunk") is None:
        raise DigestError(f"no `trunk` command on the path: install trunk {TRUNK_VERSION}")
    out = subprocess.run(["trunk", "--version"], capture_output=True, text=True, check=False).stdout
    match = re.search(r"(\d+\.\d+\.\d+)", out)
    return match.group(1) if match else out.strip()


def commit_epoch(rev: str) -> str:
    """The commit time of `rev`, in seconds since the epoch."""
    result = subprocess.run(
        ["git", "log", "-1", "--format=%ct", rev], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if result.returncode != 0 or not result.stdout.strip().isdigit():
        raise DigestError(f"{rev} names no commit")
    return result.stdout.strip()


def extract(rev: str, into: Path) -> None:
    """The tree of `rev` alone, written into the new directory `into`."""
    if into.exists() and any(into.iterdir()):
        raise DigestError(f"{into} is not empty: a build starts from a clean tree")
    if into.resolve().is_relative_to(ROOT):
        raise DigestError(f"{into} lies inside the checkout, whose `.cargo/config.toml` the build would merge")
    into.mkdir(parents=True, exist_ok=True)
    archive = subprocess.run(["git", "archive", "--format=tar", rev], cwd=ROOT, capture_output=True, check=False)
    if archive.returncode != 0:
        raise DigestError(f"git archive refused {rev}: {archive.stderr.decode(errors='replace').strip()}")
    with tempfile.TemporaryFile() as tar:
        tar.write(archive.stdout)
        tar.seek(0)
        with tarfile.open(fileobj=tar) as tree:
            tree.extractall(into, filter="data")


def build(rev: str, into: Path) -> str:
    """Build the release of `rev` in `into`, write its manifest there, and return the manifest."""
    version = trunk_version()
    if version != TRUNK_VERSION:
        raise DigestError(f"trunk {version} is on the path, and the release is built by trunk {TRUNK_VERSION}")
    extract(rev, into)
    env = build_env(dict(os.environ), commit_epoch(rev))
    dist = into / "dist"
    print(f"client_digest: building {rev} in {into}", file=sys.stderr, flush=True)
    result = subprocess.run(["trunk", "build", "--release", "--dist", str(dist)], cwd=into, env=env, check=False)
    if result.returncode != 0:
        raise DigestError(f"the release build in {into} failed")
    text = manifest(dist)
    (into / MANIFEST_NAME).write_text(text, encoding="utf-8")
    return text


def write_published(text: str, out: Path = OUT) -> str:
    """Write the manifest and the client digest into `out`, and return the digest."""
    out.mkdir(parents=True, exist_ok=True)
    digest = client_digest(text)
    (out / MANIFEST_NAME).write_text(text, encoding="utf-8")
    (out / DIGEST_NAME).write_text(digest + "\n", encoding="utf-8")
    return digest


def reproduce(rev: str, keep: bool) -> int:
    """Build `rev` twice in two paths, and publish the digest if the two manifests match."""
    base = Path(tempfile.mkdtemp(prefix="ambre-reproduce-"))
    # Two depths, so a path that leaks into the bytes changes their length too.
    first_dir, second_dir = base / "a", base / "second" / "build-tree"
    try:
        first = build(rev, first_dir)
        second = build(rev, second_dir)
        changed = differences(first, second)
        if changed:
            print(f"client_digest: two builds of {rev} differ in {len(changed)} files:", file=sys.stderr)
            for path in changed:
                print(f"  {path}", file=sys.stderr)
            keep = True
            return 1
        digest = write_published(first)
        print(f"client digest of {rev}: {digest}")
        print(f"manifest and digest written to {OUT.relative_to(ROOT)}/")
        return 0
    finally:
        if keep:
            print(f"client_digest: the two trees stay in {base}", file=sys.stderr)
        else:
            shutil.rmtree(base, ignore_errors=True)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    one = commands.add_parser("manifest", help="print the manifest of a dist directory, then its digest")
    one.add_argument("dist", type=Path)
    two = commands.add_parser("build", help="build one commit in a new directory")
    two.add_argument("rev")
    two.add_argument("dir", type=Path)
    three = commands.add_parser("compare", help="compare two manifests")
    three.add_argument("first", type=Path)
    three.add_argument("second", type=Path)
    four = commands.add_parser("reproduce", help="build one commit twice and compare")
    four.add_argument("--rev", default="HEAD")
    four.add_argument("--keep", action="store_true", help="keep the two build trees")
    args = parser.parse_args(argv)
    try:
        if args.command == "manifest":
            text = manifest(args.dist)
            sys.stdout.write(text)
            print(f"client digest: {client_digest(text)}")
        elif args.command == "build":
            text = build(args.rev, args.dir)
            print(f"client digest of {args.rev}: {client_digest(text)}")
        elif args.command == "compare":
            changed = differences(args.first.read_text(encoding="utf-8"), args.second.read_text(encoding="utf-8"))
            for path in changed:
                print(f"differs: {path}")
            return 1 if changed else 0
        else:
            return reproduce(args.rev, args.keep)
    except DigestError as err:
        print(f"client_digest: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
