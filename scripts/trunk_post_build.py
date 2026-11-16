#!/usr/bin/env python3
"""What Trunk's build leaves out, written into its staging directory.

Trunk runs this hook after every build (`Trunk.toml`, `[[hooks]]`), before it
moves the staging directory to `dist/`.

**The two workers, hashed, and their addresses.** The Sync worker and the call worker
are hashed the same way; the Sync worker stands for both below. `filehash` names each built file
by a hash of its content, so a deployment may cache it for good. Trunk 0.21
skips the three files of a worker, though: a cached loader would outlive the
release that replaced it. The hook names them by one hash of the glue and the
wasm, and rewrites the loader to match. The page then cannot spell the loader's
name, and Trunk does not inject a worker into the page. The hook writes that
name into the page's `ambre-sync-worker` meta, which
`crates/ambre-app/src/platform.rs` reads before it spawns the worker. A page
whose meta stays empty fails at boot, loudly, rather than spawning a worker
from a stale name.

**The converter modules, named by their digests.** The pre-build hook builds
one module per format family and writes each BLAKE2b-256 digest
(`scripts/converter_build.py`). This hook copies each module into the staging
directory as `ambre-convert-<family>-<16 hex digits>.wasm`, the name the Sync
worker derives from the digest it carries for that family
(`crates/ambre-sync/src/converter.rs`). A digest names one release, so each
copy caches for good like every hashed file.

**The script of each thread, shipped.** `wasm_thread` spawns each thread of
the Main instance from a module script that imports the Main glue. Left alone,
it builds that script as a `blob:` address. The hook writes the same script as
`ambre-thread-<16 hex digits>.js`, named by a hash of its content, and fills
the page's `ambre-thread-worker` meta, so the content policy admits workers of
the page's own origin alone.

**The content policy of the page.** The policy admits as a script the files of
the build, the compile of a wasm module, and each inline script the page
carries, by its SHA-256 digest (`SRV-260`). It admits each inline style the
same way, and the token stylesheet the launcher injects at boot. The hook
reads the page as the build left it, so the digests name the bytes it ships.
It writes the policy into the page's `Content-Security-Policy` meta, last of
the edits to the page.

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

import base64
import hashlib
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

# The two workers the page loads as artefacts of their own: the Sync worker at
# boot, and the call worker on the first call (`ambre-spec-architecture.md
# section 2`). Each is hashed and named in a meta of the page the same way.
WORKERS = ("ambre-sync", "ambre-call")


def placeholder(worker: str = "ambre-sync") -> str:
    """The meta the page carries for the hook to fill, exactly as the page spells it."""
    return f'<meta name="{worker}-worker" content="">'


def worker_files(worker: str = "ambre-sync") -> tuple[str, str, str]:
    """The three files Trunk writes for `worker`, unhashed: the loader names the other two."""
    return f"{worker}.js", f"{worker}_bg.wasm", f"{worker}_loader.js"


# The meta and the files of the Sync worker, as the page and Trunk spell them.
PLACEHOLDER = placeholder()
GLUE, WASM, LOADER = worker_files()


# The modules and their digests, as the pre-build hook writes them, one pair
# per family of `scripts/converter_build.py`.
CONVERTER_DIR = Path(__file__).resolve().parent.parent / "target" / "converter"
CONVERTER_FAMILIES = ("tables", "pdf", "markup", "images", "office", "boards")


def converter_module(family: str) -> str:
    """The file name of the module of `family`, as the pre-build hook writes it."""
    return f"ambre-convert-{family}.wasm"


def converter_digest(family: str) -> str:
    """The file name of the digest of the module of `family`."""
    return f"ambre-convert-{family}.digest"

# The hex digits of the digest the served name carries, as the Sync worker
# spells it.
NAME_DIGITS = 16

# What a brotli copy is written for. A font in `woff2` and an image are
# compressed already.
COMPRESSIBLE = {".wasm", ".js", ".css", ".html", ".ttf", ".otf", ".svg", ".json"}


# The glue of the Main instance, as Trunk names it with `filehash`.
MAIN_GLUE = "ambre-app-*.js"

# The meta of the thread script, as the page spells it empty.
THREAD_PLACEHOLDER = '<meta name="ambre-thread-worker" content="">'

# The module script of each thread: the template of `wasm_thread`
# (`vendor/wasm_thread/src/wasm32/js/web_worker_module.js`), with the glue named.
THREAD_SCRIPT = """import init, {{wasm_thread_entry_point}} from "./{glue}";

self.onmessage = event => {{
    let [ module, memory, work ] = event.data;
    init(module, memory).catch(err => {{
        console.log(err);
        setTimeout(() => {{ throw err; }});
        throw err;
    }}).then(() => {{
        wasm_thread_entry_point(work);
        close();
    }});
}};
"""

# The meta of the content policy, as the page spells it empty.
POLICY_PLACEHOLDER = '<meta http-equiv="Content-Security-Policy" content="">'

# The token stylesheet the launcher injects into the page at boot, as an inline
# style (`crates/ambre-app/src/platform.rs`, `inject_theme`).
THEME_CSS = Path(__file__).resolve().parent.parent / "crates" / "ambre-canvas" / "src" / "theme.css"

COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)
INLINE_SCRIPT = re.compile(r"<script\b([^>]*)>(.*?)</script\s*>", re.DOTALL | re.IGNORECASE)
INLINE_STYLE = re.compile(r"<style\b[^>]*>(.*?)</style\s*>", re.DOTALL | re.IGNORECASE)


class HookError(Exception):
    """A staging directory the hook cannot finish."""


def hash_worker(staging: Path, worker: str = "ambre-sync") -> str:
    """Name the worker's three files by one hash of its content, and return the loader's name."""
    names = worker_files(worker)
    glue_name, wasm_name, _ = names
    glue, wasm, loader = (staging / name for name in names)
    for path in (glue, wasm, loader):
        if not path.is_file():
            raise HookError(f"{path} is missing: Trunk named the {worker} worker otherwise")
    text = loader.read_text(encoding="utf-8")
    for name in (glue_name, wasm_name):
        count = text.count(f'"./{name}"')
        if count != 1:
            raise HookError(f"{loader} names `./{name}` {count} times, not once")
    digest = hashlib.sha256(glue.read_bytes() + wasm.read_bytes()).hexdigest()[:16]
    stem = f"{worker}-{digest}"
    text = text.replace(f'"./{glue_name}"', f'"./{stem}.js"').replace(f'"./{wasm_name}"', f'"./{stem}_bg.wasm"')
    glue.rename(staging / f"{stem}.js")
    wasm.rename(staging / f"{stem}_bg.wasm")
    (staging / f"{stem}_loader.js").write_text(text, encoding="utf-8")
    loader.unlink()
    return f"{stem}_loader.js"


def fill_worker_address(staging: Path, public_url: str, worker: str = "ambre-sync") -> str:
    """Write the loader's address into the page, and return it."""
    loaders = sorted(staging.glob(f"{worker}-*_loader.js"))
    if len(loaders) != 1:
        raise HookError(f"{len(loaders)} {worker} loaders in {staging}, not one")
    address = public_url.rstrip("/") + "/" + loaders[0].name
    page = staging / "index.html"
    text = page.read_text(encoding="utf-8")
    empty = placeholder(worker)
    if text.count(empty) != 1:
        raise HookError(f"{page} carries {text.count(empty)} empty `{worker}-worker` metas, not one")
    page.write_text(text.replace(empty, f'<meta name="{worker}-worker" content="{address}">'), encoding="utf-8")
    return address


def copy_converter(staging: Path, source: Path, family: str) -> str:
    """Copy the module of `family` into `staging`, named by its digest, and return the name."""
    module, digest_file = source / converter_module(family), source / converter_digest(family)
    for path in (module, digest_file):
        if not path.is_file():
            raise HookError(f"{path} is missing: the pre-build hook did not build the {family} module")
    hex_digest = digest_file.read_text(encoding="utf-8").strip()
    if len(hex_digest) != 64 or any(c not in "0123456789abcdef" for c in hex_digest):
        raise HookError(f"{digest_file} holds no BLAKE2b-256 digest in hex")
    if hashlib.blake2b(module.read_bytes(), digest_size=32).hexdigest() != hex_digest:
        raise HookError(f"{module} is not the module its digest names: run the pre-build hook again")
    name = f"ambre-convert-{family}-{hex_digest[:NAME_DIGITS]}.wasm"
    shutil.copyfile(module, staging / name)
    return name


def write_thread_script(staging: Path, public_url: str) -> str:
    """Write the thread script beside the Main glue, fill the page's meta, and return the address."""
    glues = sorted(p for p in staging.glob(MAIN_GLUE) if not p.name.endswith("_bg.js"))
    if len(glues) != 1:
        raise HookError(f"{len(glues)} Main glues in {staging}, not one")
    text = THREAD_SCRIPT.format(glue=glues[0].name)
    name = f"ambre-thread-{hashlib.sha256(text.encode('utf-8')).hexdigest()[:16]}.js"
    (staging / name).write_text(text, encoding="utf-8")
    address = public_url.rstrip("/") + "/" + name
    page = staging / "index.html"
    html = page.read_text(encoding="utf-8")
    if html.count(THREAD_PLACEHOLDER) != 1:
        raise HookError(f"{page} carries {html.count(THREAD_PLACEHOLDER)} empty `ambre-thread-worker` metas, not one")
    page.write_text(html.replace(THREAD_PLACEHOLDER, f'<meta name="ambre-thread-worker" content="{address}">'), encoding="utf-8")
    return address


def digest(source: str) -> str:
    """The CSP source that admits the inline text `source`, by its SHA-256 digest."""
    return "'sha256-" + base64.b64encode(hashlib.sha256(source.encode("utf-8")).digest()).decode("ascii") + "'"


def inline_digests(html: str) -> tuple[list[str], list[str]]:
    """The digests of the inline scripts and of the inline styles of `html`, in page order."""
    html = COMMENT.sub("", html)
    scripts = [digest(body) for attrs, body in INLINE_SCRIPT.findall(html) if not re.search(r"\bsrc\s*=", attrs)]
    styles = [digest(body) for body in INLINE_STYLE.findall(html)]
    return scripts, styles


def policy(scripts: list[str], styles: list[str]) -> str:
    """The content policy that admits the shipped files and the inline texts named by digest.

    No `unsafe-inline` and no `unsafe-eval`: an injected script or an evaluated
    string runs nowhere. `wasm-unsafe-eval` admits the compile of a wasm module
    alone. The workers come from the page's own origin, never from `blob:`. A
    web embed is a sandboxed frame of any `https` page (`TOILE-088`). A picture
    the page shows from memory is a `blob:` or `data:` address.
    """
    return "; ".join(
        [
            "default-src 'self'",
            " ".join(["script-src 'self' 'wasm-unsafe-eval'", *dict.fromkeys(scripts)]),
            " ".join(["style-src 'self'", *dict.fromkeys(styles)]),
            "img-src 'self' blob: data:",
            "media-src 'self' blob:",
            "frame-src https:",
            "worker-src 'self'",
            "object-src 'none'",
            "base-uri 'none'",
            "form-action 'none'",
        ]
    )


def settle_autoreload(html: str, public_url: str) -> str:
    """The page with the address of the reload socket of `trunk serve` written in.

    `trunk serve` injects an inline reload script, then rewrites two
    placeholders of it in each answer: the host the request names, and the
    base of the socket. A digest of the script before that rewrite admits
    nothing. So the hook writes the two values the script would compute, the
    page's own host and the public address, and the server finds nothing to
    rewrite. A build outside `trunk serve` carries no reload script.
    """
    base = public_url.rstrip("/") + "/"
    return (
        html.replace("'{{__TRUNK_ADDRESS__}}'", "window.location.host")
        .replace("`{{__TRUNK_ADDRESS__}}`", "window.location.host")
        .replace("{{__TRUNK_WS_BASE__}}", base)
    )


def fill_policy(staging: Path, theme_css: Path = THEME_CSS, public_url: str = "/") -> str:
    """Write the content policy of the page as it stands into its meta, and return it."""
    if not theme_css.is_file():
        raise HookError(f"{theme_css} is missing: the policy cannot name the token stylesheet")
    page = staging / "index.html"
    html = settle_autoreload(page.read_text(encoding="utf-8"), public_url)
    if html.count(POLICY_PLACEHOLDER) != 1:
        raise HookError(f"{page} carries {html.count(POLICY_PLACEHOLDER)} empty content policy metas, not one")
    scripts, styles = inline_digests(html)
    text = policy(scripts, [*styles, digest(theme_css.read_text(encoding="utf-8"))])
    page.write_text(
        html.replace(POLICY_PLACEHOLDER, f'<meta http-equiv="Content-Security-Policy" content="{text}">'),
        encoding="utf-8",
    )
    return text


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
        for worker in WORKERS:
            hash_worker(staging, worker)
            fill_worker_address(staging, os.environ.get("TRUNK_PUBLIC_URL", "/"), worker)
        write_thread_script(staging, os.environ.get("TRUNK_PUBLIC_URL", "/"))
        for family in CONVERTER_FAMILIES:
            copy_converter(staging, CONVERTER_DIR, family)
        fill_policy(staging, public_url=os.environ.get("TRUNK_PUBLIC_URL", "/"))
        if os.environ.get("TRUNK_PROFILE") == "release":
            write_brotli(compressible(staging))
    except HookError as err:
        print(f"trunk_post_build: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
