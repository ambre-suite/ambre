#!/usr/bin/env python3
"""What Trunk's build leaves out, written into its staging directory.

Trunk runs this hook after every build (`Trunk.toml`, `[[hooks]]`), before it
moves the staging directory to `dist/`.

**The Sync worker, hashed, and its address.** `filehash` names each built file
by a hash of its content, so a deployment may cache it for good. Trunk 0.21
skips the three files of a worker, though: a cached loader would outlive the
release that replaced it. The hook names them by one hash of the glue and the
wasm, and rewrites the loader to match. The page then cannot spell the loader's
name, and Trunk does not inject a worker into the page. The hook writes that
name into the page's `ambre-sync-worker` meta, which
`crates/ambre-app/src/platform.rs` reads before it spawns the worker. A page
whose meta stays empty fails at boot, loudly, rather than spawning a worker
from a stale name.

**The converter module, named by its digest.** The pre-build hook builds the
module and writes its BLAKE2b-256 digest (`scripts/converter_build.py`). This
hook copies the module into the staging directory as
`ambre-convert-<16 hex digits>.wasm`, the name the Sync worker derives from
the digest it carries (`crates/ambre-sync/src/converter.rs`). A digest names
one release, so the copy caches for good like every hashed file.

**A brotli copy of each compressible file, in a release.** A static server
serves `name.br` to a browser that accepts brotli, without compressing on each
request (nginx `brotli_static`, Caddy `precompressed`). Brotli at its highest
quality takes a quarter off the wasm against gzip, and the download budget is
counted on it (`ambre-spec-budgets.md section 5`). It needs the `brotli`
command on the path.

Run by Trunk only. It reads `TRUNK_STAGING_DIR`, `TRUNK_PROFILE` and
`TRUNK_PUBLIC_URL` from the environment Trunk gives its hooks.
"""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path

# The meta the page carries for the hook to fill, exactly as the page spells it.
PLACEHOLDER = '<meta name="ambre-sync-worker" content="">'

# The three files Trunk writes for the `ambre-sync` worker, unhashed: the
# loader names the other two.
GLUE = "ambre-sync.js"
WASM = "ambre-sync_bg.wasm"
LOADER = "ambre-sync_loader.js"

# The loader once hashed, which the page's meta names.
LOADER_GLOB = "ambre-sync-*_loader.js"

# The module and its digest, as the pre-build hook writes them.
CONVERTER_DIR = Path(__file__).resolve().parent.parent / "target" / "converter"
CONVERTER_MODULE = "ambre-convert.wasm"
CONVERTER_DIGEST = "ambre-convert.digest"

# The hex digits of the digest the served name carries, as the Sync worker
# spells it.
NAME_DIGITS = 16

# What a brotli copy is written for. A font in `woff2` and an image are
# compressed already.
COMPRESSIBLE = {".wasm", ".js", ".css", ".html", ".ttf", ".otf", ".svg", ".json"}


class HookError(Exception):
    """A staging directory the hook cannot finish."""


def hash_worker(staging: Path) -> str:
    """Name the worker's three files by one hash of its content, and return the loader's name."""
    glue, wasm, loader = (staging / name for name in (GLUE, WASM, LOADER))
    for path in (glue, wasm, loader):
        if not path.is_file():
            raise HookError(f"{path} is missing: Trunk named the Sync worker otherwise")
    text = loader.read_text(encoding="utf-8")
    for name in (GLUE, WASM):
        count = text.count(f'"./{name}"')
        if count != 1:
            raise HookError(f"{loader} names `./{name}` {count} times, not once")
    digest = hashlib.sha256(glue.read_bytes() + wasm.read_bytes()).hexdigest()[:16]
    stem = f"ambre-sync-{digest}"
    text = text.replace(f'"./{GLUE}"', f'"./{stem}.js"').replace(f'"./{WASM}"', f'"./{stem}_bg.wasm"')
    glue.rename(staging / f"{stem}.js")
    wasm.rename(staging / f"{stem}_bg.wasm")
    (staging / f"{stem}_loader.js").write_text(text, encoding="utf-8")
    loader.unlink()
    return f"{stem}_loader.js"


def fill_worker_address(staging: Path, public_url: str) -> str:
    """Write the loader's address into the page, and return it."""
    loaders = sorted(staging.glob(LOADER_GLOB))
    if len(loaders) != 1:
        raise HookError(f"{len(loaders)} Sync loaders in {staging}, not one")
    address = public_url.rstrip("/") + "/" + loaders[0].name
    page = staging / "index.html"
    text = page.read_text(encoding="utf-8")
    if text.count(PLACEHOLDER) != 1:
        raise HookError(f"{page} carries {text.count(PLACEHOLDER)} empty `ambre-sync-worker` metas, not one")
    page.write_text(text.replace(PLACEHOLDER, f'<meta name="ambre-sync-worker" content="{address}">'), encoding="utf-8")
    return address


def copy_converter(staging: Path, source: Path) -> str:
    """Copy the converter module into `staging`, named by its digest, and return the name."""
    module, digest_file = source / CONVERTER_MODULE, source / CONVERTER_DIGEST
    for path in (module, digest_file):
        if not path.is_file():
            raise HookError(f"{path} is missing: the pre-build hook did not build the converter")
    hex_digest = digest_file.read_text(encoding="utf-8").strip()
    if len(hex_digest) != 64 or any(c not in "0123456789abcdef" for c in hex_digest):
        raise HookError(f"{digest_file} holds no BLAKE2b-256 digest in hex")
    if hashlib.blake2b(module.read_bytes(), digest_size=32).hexdigest() != hex_digest:
        raise HookError(f"{module} is not the module its digest names: run the pre-build hook again")
    name = f"ambre-convert-{hex_digest[:NAME_DIGITS]}.wasm"
    shutil.copyfile(module, staging / name)
    return name


def compressible(staging: Path) -> list[Path]:
    """The files of `staging` a brotli copy is written for."""
    return sorted(p for p in staging.rglob("*") if p.is_file() and p.suffix in COMPRESSIBLE)


def write_brotli(files: list[Path]) -> None:
    """A `.br` copy beside each of `files`, at the highest quality."""
    if shutil.which("brotli") is None:
        raise HookError("a release writes brotli copies, and no `brotli` command is on the path")
    for path in files:
        subprocess.run(
            ["brotli", "--quality=11", "--force", f"--output={path}.br", str(path)],
            check=True,
        )


def main() -> int:
    staging = Path(os.environ["TRUNK_STAGING_DIR"])
    try:
        hash_worker(staging)
        fill_worker_address(staging, os.environ.get("TRUNK_PUBLIC_URL", "/"))
        copy_converter(staging, CONVERTER_DIR)
        if os.environ.get("TRUNK_PROFILE") == "release":
            write_brotli(compressible(staging))
    except HookError as err:
        print(f"trunk_post_build: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
