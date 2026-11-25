#!/usr/bin/env python3
"""The link key a release of the client pins: its check, and its gate.

The relay signs a receipt of each thread link under a key of one purpose,
and a census refuses to erase a thread under a receipt that the pinned key
verifies (`SYNC-370`, `SRV-390`). A client that pins no key counts no
receipt. A client that pins the development key counts a receipt any member
signs with the public development seed, so any member blocks every removal.
A release therefore pins one key, and never that one (`ZK-062`).

`check` reads `AMBRE_THREAD_LINK_KEY` from the environment and fails on each
of these:

* no value, or an empty one;
* a value that is not 64 hexadecimal digits;
* the development key, `DEV_LINK_KEY` of
  `crates/ambre-protocol/src/thread_erasure.rs`, read from that file.

`lint` reads `deploy/Dockerfile` and `.github/workflows/release.yml`, and
fails on each of these:

* a `web-build` stage that declares no `ARG AMBRE_THREAD_LINK_KEY`;
* a `trunk build --release` of that stage that no `check` precedes in its
  `RUN`;
* a job of the release that runs `just reproducible` with no `check` step
  before it, or whose `AMBRE_THREAD_LINK_KEY` is not `vars.THREAD_LINK_KEY`;
* an image build of the release that passes no `AMBRE_THREAD_LINK_KEY` from
  `vars.THREAD_LINK_KEY` as a build argument.

The key is public, so a repository variable holds it, never a secret. The
seed stays with the deployment. The build of the crate refuses a malformed
key or the development key as well, and `check` adds the refusal of no key.

Run: `just link-key` for the gate. It writes nothing.
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - the message is the whole behaviour
    sys.exit("link_key.py needs PyYAML: install python3-yaml")

ROOT = Path(__file__).resolve().parent.parent
DOCKERFILE = ROOT / "deploy/Dockerfile"
RELEASE = ROOT / ".github/workflows/release.yml"
PROTOCOL = ROOT / "crates/ambre-protocol/src/thread_erasure.rs"

VARIABLE = "AMBRE_THREAD_LINK_KEY"
SOURCE = "${{ vars.THREAD_LINK_KEY }}"
CHECK = "scripts/link_key.py check"
HEX = re.compile(r"[0-9a-fA-F]{64}")


def dev_key(source: str) -> str:
    """The development link key that `source`, the protocol file, declares, in hex."""
    found = re.search(r"pub const DEV_LINK_KEY\b[^\[]*\[(.*?)\]", source, re.S)
    if found is None:
        raise ValueError("no DEV_LINK_KEY array in the protocol file")
    digits = re.findall(r"0x([0-9a-fA-F]{2})", found.group(1))
    if len(digits) != 32:
        raise ValueError(f"DEV_LINK_KEY holds {len(digits)} bytes, not 32")
    return "".join(digits).lower()


def refusal(value: str | None, dev: str) -> str | None:
    """Why a release refuses `value` as its link key, or `None` when it pins it."""
    if not value:
        return f"{VARIABLE} is not set: a release pins the link key of its relay"
    if not HEX.fullmatch(value):
        return f"{VARIABLE} must be 64 hex digits"
    if value.lower() == dev:
        return f"{VARIABLE} names the development key, under which any member signs a receipt"
    return None


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
    """The faults of the client build of `deploy/Dockerfile` against `ZK-062`."""
    lines = stage(dockerfile, "web-build")
    if not lines:
        return ["Dockerfile: no web-build stage"]
    faults = []
    if not any(re.match(rf"\s*ARG\s+{VARIABLE}\b", line) for line in lines):
        faults.append(f"Dockerfile web-build: no ARG {VARIABLE}, so a build argument never reaches the client")
    releases = [line for line in lines if "trunk build --release" in line]
    if not releases:
        faults.append("Dockerfile web-build: no `trunk build --release`, so nothing builds the release")
    for line in releases:
        before = line[: line.index("trunk build --release")]
        if CHECK not in before:
            faults.append(f"Dockerfile web-build: a release builds before `{CHECK}`")
    return faults


def steps(job: dict) -> list[dict]:
    return [s for s in job.get("steps") or [] if isinstance(s, dict)]


def check_release(workflow: dict) -> list[str]:
    """The faults of the release workflow against `ZK-062`."""
    faults = []
    jobs = {n: j for n, j in (workflow.get("jobs") or {}).items() if isinstance(j, dict)}
    digests = [n for n, j in jobs.items() if any("just reproducible" in str(s.get("run", "")) for s in steps(j))]
    if not digests:
        faults.append("release.yml: no job runs `just reproducible`")
    for name in digests:
        job = jobs[name]
        env = {**(workflow.get("env") or {}), **(job.get("env") or {})}
        if env.get(VARIABLE) != SOURCE:
            faults.append(f"release.yml job {name}: {VARIABLE} is not {SOURCE}")
        runs = [str(s.get("run", "")) for s in steps(job)]
        first = next(i for i, r in enumerate(runs) if "just reproducible" in r)
        if not any(CHECK in r for r in runs[:first]):
            faults.append(f"release.yml job {name}: `just reproducible` runs before `{CHECK}`")
    builds = [
        (n, s)
        for n, j in jobs.items()
        for s in steps(j)
        if str(s.get("uses", "")).startswith("docker/build-push-action@")
    ]
    if not builds:
        faults.append("release.yml: no job builds an image")
    for name, step in builds:
        args = str((step.get("with") or {}).get("build-args", ""))
        if f"{VARIABLE}={SOURCE}" not in (line.strip() for line in args.splitlines()):
            faults.append(f"release.yml job {name}: the image build passes no {VARIABLE}={SOURCE}")
    return faults


def main(args: list[str]) -> int:
    if args == ["check"]:
        why = refusal(os.environ.get(VARIABLE), dev_key(PROTOCOL.read_text()))
        if why:
            print(f"link_key: {why} (ZK-062)", file=sys.stderr)
            return 1
        return 0
    if args == ["lint"]:
        faults = check_dockerfile(DOCKERFILE.read_text()) + check_release(yaml.safe_load(RELEASE.read_text()))
        for fault in faults:
            print(f"link_key: {fault} (ZK-062)", file=sys.stderr)
        return 1 if faults else 0
    print("usage: link_key.py check | lint", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
