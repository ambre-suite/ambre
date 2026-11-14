#!/usr/bin/env python3
"""What a CI job restores of `target/`, and what lets cargo reuse it.

`.cargo/config.toml` builds `std` from source on every target (`-Z build-std`):
multi-threaded wasm needs it, and cargo applies it to the host build too. Two
facts then made each restored cache useless, and every job compiled every
dependency again (run 43574018216 of 2026-11-14):

* **`rust-cache` deletes `std`.** Before it saves `target/`, it keeps only the
  artifacts of the packages `cargo metadata` names. `std`, `core`, `alloc` and
  the rest of the sysroot are not among them. Cargo then rebuilds them, and
  each crate above them is newer than its dependent, so every crate rebuilds.
* **rustup dates the `std` sources at install.** Each job installs the pinned
  toolchain, so each source of `std` carries the time of that job. Cargo reads
  a path dependency as stale when a source is newer than its last build, and
  `std` is a path dependency under `build-std`.

The two subcommands remove both facts:

* `pin-std` dates every source of `std` at `PINNED`, so a build restored from
  another job reads `std` as fresh. Run it before the first build.
* `prune` removes, from each profile of `target/`, the artifacts of the
  workspace's own crates, and every file beside `build/`, `.fingerprint/` and
  `deps/`. It keeps everything else, `std` included. It reads both layouts of
  cargo: `deps/libserde-<hash>.rlib`, and `build/serde/<hash>/` of the
  build directory that the pinned nightly writes. A fresh checkout dates
  every workspace source at the clone, so a workspace artifact never survives
  into a later job: the cache would carry it for nothing. Run it last, before
  the cache saves.

A job that runs both caches `target/` through `cache-directories`, with
`cache-targets: false`, so that `rust-cache` saves what `prune` left.

Run: `just ci-cache pin-std`, `just ci-cache prune`.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Any instant before the first build of a job. A fixed one keeps a restored
# `std` fresh in every later job.
PINNED = 946684800  # 2000-01-01T00:00:00Z

# What cargo keeps in a profile directory between two builds. The rest of a
# profile, such as `incremental/` or the binaries, is rebuilt or relinked.
KEPT = {"build", ".fingerprint", "deps"}

# A cargo artifact name: the crate, a hyphen, sixteen hexadecimal digits, then
# an extension or nothing.
HASHED = re.compile(r"^(?P<name>.+)-[0-9a-f]{16}(\..*)?$")


def library_sources() -> Path:
    """The `std` sources of the active toolchain, which `build-std` compiles."""
    sysroot = subprocess.run(
        ["rustc", "--print", "sysroot"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    return Path(sysroot) / "lib" / "rustlib" / "src" / "rust" / "library"


def pin(tree: Path, instant: int = PINNED) -> int:
    """Date every file and directory under `tree` at `instant`; the count dated.

    Raises `SystemExit` when `tree` is not a directory: without `rust-src`,
    `build-std` cannot run at all."""
    if not tree.is_dir():
        raise SystemExit(f"{tree}: no std sources; is the `rust-src` component installed?")
    count = 0
    for parent, dirs, files in os.walk(tree):
        for name in [*files, *dirs]:
            os.utime(Path(parent) / name, (instant, instant), follow_symlinks=False)
            count += 1
    return count


def workspace_names(metadata: dict) -> set[str]:
    """Every name a workspace crate's artifacts carry: each package and each target,
    under its own spelling, with `-` turned to `_`, and with `lib` before it."""
    names = set()
    for package in metadata["packages"]:
        for name in [package["name"], *(t["name"] for t in package["targets"])]:
            under = name.replace("-", "_")
            names |= {name, under, f"lib{under}"}
    return names


def crate_of(entry: str) -> str:
    """The crate an artifact entry belongs to: the name before its hash."""
    match = HASHED.match(entry)
    return match.group("name") if match else entry


def is_profile(path: Path) -> bool:
    """Whether a directory of `target/` is a profile: it holds a cargo artifact directory."""
    return any((path / kept).is_dir() for kept in KEPT)


def remove(path: Path) -> None:
    """Remove one file, link or directory tree."""
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    else:
        path.unlink()


def prune_profile(profile: Path, workspace: set[str]) -> int:
    """Remove the workspace's artifacts and every entry outside `KEPT`; the count removed."""
    count = 0
    for entry in profile.iterdir():
        if entry.name not in KEPT:
            remove(entry)
            count += 1
    for kept in KEPT:
        directory = profile / kept
        if not directory.is_dir():
            continue
        for entry in directory.iterdir():
            if crate_of(entry.name) in workspace:
                remove(entry)
                count += 1
    return count


def prune(target: Path, workspace: set[str]) -> int:
    """Prune each profile under `target`, at any depth; the count removed.

    A target triple directory, such as `wasm32-unknown-unknown/`, holds profiles
    of its own. A loose file beside them is removed, `CACHEDIR.TAG` aside."""
    count = 0
    for entry in target.iterdir():
        if entry.is_dir() and not entry.is_symlink():
            if is_profile(entry):
                count += prune_profile(entry, workspace)
            else:
                count += prune(entry, workspace)
        elif entry.name != "CACHEDIR.TAG":
            remove(entry)
            count += 1
    return count


def metadata(workspace: Path) -> dict:
    """`cargo metadata` of the workspace's own packages."""
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=workspace,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def main(argv: list[str]) -> int:
    if argv[:1] == ["pin-std"] and len(argv) == 1:
        tree = library_sources()
        print(f"ci-cache: {pin(tree)} std sources dated 2000-01-01 under {tree}")
        return 0
    if argv[:1] == ["prune"] and len(argv) <= 2:
        workspace = ROOT / (argv[1] if len(argv) == 2 else ".")
        meta = metadata(workspace)
        target = Path(meta["target_directory"])
        if not target.is_dir():
            print(f"ci-cache: no {target}, nothing to prune")
            return 0
        removed = prune(target, workspace_names(meta))
        print(f"ci-cache: {removed} workspace entries removed from {target}")
        return 0
    print("usage: ci_cache.py pin-std | prune [workspace]", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
