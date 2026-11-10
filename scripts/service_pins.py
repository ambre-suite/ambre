#!/usr/bin/env python3
"""The backing services run from one pinned image each.

`ambre-spec-testing.md section 4` lets a tier result speak only for the image
`docker-compose.yml` pins. A tag that floats changes that image under the same
text, and a CI service that names another image tests another version.
`ARCH-087` and `ARCH-088` state the two rules. The gate reads the two files and
fails on each of these:

* an image with no `@sha256:` digest of 64 hexadecimal digits;
* an image whose tag is missing, `latest`, or a version of fewer numeric
  components than the project releases, such as `8-alpine` or `16`: three,
  unless `COMPONENTS` names the repository with its count and the reason;
* a `services:` image of a CI job that differs from the compose image of the
  service of the same name, or that names a service the compose file lacks.

A tag may carry a `v` before the version and a variant after a hyphen:
`v2.3.0`, `8.10.3-alpine` and, for Postgres, `16.15-alpine` pass. Whether the digest is the one the tag named
on the day of the pin is left to review: the gate reads no registry.

The two files are read line by line, by indentation, so the gate needs no YAML
library. It reads block mappings alone, which is the form both files use.

Run: `just service-pins`. It writes nothing.
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COMPOSE = "docker-compose.yml"
WORKFLOW = ".github/workflows/ci.yml"

KEY = re.compile(r"^(\s*)([A-Za-z0-9_.-]+):\s*(.*)$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
VARIANT = r"(-[A-Za-z0-9][A-Za-z0-9._-]*)?"

# The components of an exact version, by repository: three unless the project
# releases fewer. Postgres numbers a release `major.minor` since version 10, so
# `16.15` is one release and no `16.15.0` tag exists.
COMPONENTS = {"postgres": 2, "library/postgres": 2}
EXACT = 3


@dataclass(frozen=True)
class Image:
    """One `image:` key: the file, its line, the key path above it, the value."""

    path: str
    line: int
    keys: tuple[str, ...]
    value: str


def strip_comment(value: str) -> str:
    """A value without its trailing comment and its quotes."""
    value = re.sub(r"\s+#.*$", "", value).strip()
    return value.strip("\"'")


def images(path: str, text: str) -> list[Image]:
    """Each `image:` key of one block-mapping YAML file, with the keys that hold it."""
    stack: list[tuple[int, str]] = []
    out = []
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        match = KEY.match(line)
        if not match:
            continue
        indent = len(match.group(1))
        while stack and stack[-1][0] >= indent:
            stack.pop()
        key = match.group(2)
        if key == "image":
            keys = tuple(k for _, k in stack)
            out.append(Image(path, number, keys, strip_comment(match.group(3))))
        stack.append((indent, key))
    return out


def compose_services(found: list[Image]) -> dict[str, Image]:
    """The image of each top-level compose service, by service name."""
    return {i.keys[1]: i for i in found if len(i.keys) == 2 and i.keys[0] == "services"}


def ci_services(found: list[Image]) -> list[Image]:
    """Each image a CI job declares under `services:`: `jobs.<job>.services.<name>.image`."""
    return [
        i for i in found
        if len(i.keys) == 4 and i.keys[0] == "jobs" and i.keys[2] == "services"
    ]


def exact(repository: str, tag: str) -> bool:
    """Whether a tag names one release: every version component the project gives."""
    count = COMPONENTS.get(repository, EXACT)
    return re.fullmatch(rf"v?\d+(\.\d+){{{count - 1}}}{VARIANT}", tag) is not None


def pin_breaches(image: Image) -> list[str]:
    """Whether one image names an exact version and a digest."""
    where = f"{image.path}:{image.line}"
    name, at, digest = image.value.partition("@")
    out = []
    if not at or not DIGEST.match(digest):
        out.append(f"{where}: `{image.value}` names no `@sha256:` digest")
    repository, colon, tag = name.rpartition(":")
    if not colon or "/" in tag:
        out.append(f"{where}: `{image.value}` names no tag")
    elif not exact(repository, tag):
        count = COMPONENTS.get(repository, EXACT)
        out.append(
            f"{where}: `{image.value}` floats: the tag `{tag}` is not a version of {count} components"
        )
    return out


def sync_breaches(compose: dict[str, Image], ci: list[Image]) -> list[str]:
    """Whether each CI service names the compose image of the service of the same name."""
    out = []
    for image in ci:
        job, _, service = image.keys[1], image.keys[2], image.keys[3]
        where = f"{image.path}:{image.line}"
        pinned = compose.get(service)
        if pinned is None:
            out.append(f"{where}: job `{job}` runs service `{service}`, which {COMPOSE} lacks")
        elif pinned.value != image.value:
            out.append(
                f"{where}: job `{job}` runs `{image.value}`, and {COMPOSE}:{pinned.line} "
                f"pins `{pinned.value}` for `{service}`"
            )
    return out


def breaches(compose_text: str, ci_text: str) -> tuple[list[str], int]:
    """Every breach of the two files, and the count of images read."""
    compose = compose_services(images(COMPOSE, compose_text))
    ci = ci_services(images(WORKFLOW, ci_text))
    out = []
    for image in [*compose.values(), *ci]:
        out += pin_breaches(image)
    out += sync_breaches(compose, ci)
    if not compose:
        out.append(f"{COMPOSE}: no service image read")
    return out, len(compose) + len(ci)


def read(path: str) -> str:
    """The text of one file of the repository."""
    return (ROOT / path).read_text(encoding="utf-8")


def main() -> int:
    errors, count = breaches(read(COMPOSE), read(WORKFLOW))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        print(f"service pins: {len(errors)} breach(es) in {count} images", file=sys.stderr)
        return 1
    print(f"service pins: {count} images read, each pinned to one version and one digest")
    return 0


if __name__ == "__main__":
    sys.exit(main())
