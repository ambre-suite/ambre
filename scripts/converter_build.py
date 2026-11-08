#!/usr/bin/env python3
"""The converter module, built before Trunk builds the two instances.

Trunk runs this hook before every build (`Trunk.toml`, `[[hooks]]`, stage
`pre_build`). The converter module is the third wasm artifact of the client:
every file format Ambre reads or writes, fetched by the Sync worker on the
first conversion and instantiated by the Logic thread
(`specs/1-Foundations/ambre-spec-architecture.md section 2`, the converter
module).

**Built apart, without threads.** The module shares no memory with anything,
so it is built without the atomics and the shared memory that
`.cargo/config.toml` sets for every wasm build: `RUSTFLAGS` replaces those
flags for this one command. Its own target directory keeps the two flag sets
from rebuilding each other's caches. The profile is always `wasm-release`: the
module is fetched rarely, and the size gate measures the bytes a user fetches.

**Named by its digest.** The hook writes the module and its BLAKE2b-256
digest in hex into `target/converter/`. The Sync worker's `build.rs` reads the
digest, so the worker refuses any other bytes (`ARCH-080`), and the post-build
hook names the served copy by it (`scripts/trunk_post_build.py`).

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

# Where the module and its digest land, read by `crates/ambre-sync/build.rs`
# and by the post-build hook.
OUT = ROOT / "target" / "converter"
MODULE = OUT / "ambre-convert.wasm"
DIGEST = OUT / "ambre-convert.digest"

# The cargo output of the command below.
BUILT = OUT / "wasm32-unknown-unknown" / "wasm-release" / "ambre_convert.wasm"

# The one command: the engine and its exports, no loader.
COMMAND = [
    "cargo",
    "build",
    "--quiet",
    "-p",
    "ambre-convert",
    "--features",
    "engine",
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
    result = subprocess.run(COMMAND, cwd=ROOT, env=build_env(dict(os.environ)), check=False)
    if result.returncode != 0:
        print("converter_build: the converter module did not build", file=sys.stderr)
        return result.returncode
    shutil.copyfile(BUILT, MODULE)
    hex_digest = digest(MODULE.read_bytes())
    # Written only when it changed, so the Sync worker rebuilds only then.
    if not DIGEST.is_file() or DIGEST.read_text(encoding="utf-8").strip() != hex_digest:
        DIGEST.write_text(hex_digest + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
