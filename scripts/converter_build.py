#!/usr/bin/env python3
"""The converter modules, built before Trunk builds the two instances.

Trunk runs this hook before every build (`Trunk.toml`, `[[hooks]]`, stage
`pre_build`). The client fetches one converter module per format family, each
on the first conversion of its family, and the Logic thread instantiates it
(`specs/1-Foundations/ambre-spec-architecture.md section 2`, the converter
modules). Each module is one build of `ambre-convert` with its family's
feature alone, so a user who exports a CSV never downloads the PDF writer.

**Built apart, without threads.** A module shares no memory with anything,
so it is built without the atomics and the shared memory that
`.cargo/config.toml` sets for every wasm build: `RUSTFLAGS` replaces those
flags for these commands. Their own target directory keeps the two flag sets
from rebuilding each other's caches. The profile is always `wasm-release`: a
module is fetched rarely, and the size gate measures the bytes a user fetches.

**Named by its digest.** The hook writes each module and its BLAKE2b-256
digest in hex into `target/converter/`, as `ambre-convert-<family>.wasm` and
`ambre-convert-<family>.digest`. The Sync worker's `build.rs` reads each
digest, so the worker refuses any other bytes (`ARCH-080`), and the
post-build hook names each served copy by it (`scripts/trunk_post_build.py`).

Run by Trunk, or by hand: `python3 scripts/converter_build.py`.
"""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Where the modules and their digests land, read by
# `crates/ambre-sync/build.rs` and by the post-build hook.
OUT = ROOT / "target" / "converter"

# The families of `ambre_core::ConverterFamily`, by the name each file
# carries, which is also the Cargo feature of `ambre-convert` that builds it.
FAMILIES = ("tables", "pdf", "markup", "images", "office")

# The cargo output of each command below.
BUILT = OUT / "wasm32-unknown-unknown" / "wasm-release" / "ambre_convert.wasm"


def module(family: str) -> Path:
    """Where the module of `family` lands."""
    return OUT / f"ambre-convert-{family}.wasm"


def digest_file(family: str) -> Path:
    """Where the digest of the module of `family` lands."""
    return OUT / f"ambre-convert-{family}.digest"


def command(family: str) -> list[str]:
    """The build of one family: its engine and the exports, no loader."""
    return [
        "cargo",
        "build",
        "--quiet",
        "-p",
        "ambre-convert",
        "--features",
        family,
        "--lib",
        "--target",
        "wasm32-unknown-unknown",
        "--profile",
        "wasm-release",
    ]


def digest(data: bytes) -> str:
    """The BLAKE2b-256 digest of `data` in hex, as `ambre_crypto::hash::module_digest` gives it."""
    return hashlib.blake2b(data, digest_size=32).hexdigest()


def build_env(base: dict[str, str]) -> dict[str, str]:
    """The environment of the build: no threading flags, its own target directory."""
    env = dict(base)
    env["RUSTFLAGS"] = "-C debuginfo=0"
    env["CARGO_TARGET_DIR"] = str(OUT)
    return env


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    env = build_env(dict(os.environ))
    # One after the other: each build writes the same cargo output, which is
    # copied out before the next one starts.
    for family in FAMILIES:
        result = subprocess.run(command(family), cwd=ROOT, env=env, check=False)
        if result.returncode != 0:
            print(f"converter_build: the {family} module did not build", file=sys.stderr)
            return result.returncode
        shutil.copyfile(BUILT, module(family))
        hex_digest = digest(module(family).read_bytes())
        # Written only when it changed, so the Sync worker rebuilds only then.
        known = digest_file(family)
        if not known.is_file() or known.read_text(encoding="utf-8").strip() != hex_digest:
            known.write_text(hex_digest + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
