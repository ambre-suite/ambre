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
  service of the same name, or that names a service the compose file lacks;
* an image of the dev container that floats as above (`ARCH-109`): each `FROM`
  of `.devcontainer/Dockerfile`, and each image of `.devcontainer/compose.yml`,
  which must also be the image `docker-compose.yml` names for a service of the
  same name;
* a step of a CI job that installs a browser or a WebDriver server through a
  `browser-actions` action and names no exact version of it (`ARCH-105`).
  `BROWSERS` gives the input each action reads and the form of its version.
  A browser release changes a belt result under the same commit, as a new
  image does.

A tag may carry a `v` before the version and a variant after a hyphen:
`v2.3.0`, `8.10.3-alpine` and, for Postgres, `16.15-alpine` pass. Whether the digest is the one the tag named
on the day of the pin is left to review: the gate reads no registry.

The YAML files are read line by line, by indentation, so the gate needs no YAML
library. It reads block mappings alone, which is the form each file uses.

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
DEV_COMPOSE = ".devcontainer/compose.yml"
DOCKERFILE = ".devcontainer/Dockerfile"

KEY = re.compile(r"^(\s*)([A-Za-z0-9_.-]+):\s*(.*)$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
VARIANT = r"(-[A-Za-z0-9][A-Za-z0-9._-]*)?"

# The components of an exact version, by repository: three unless the project
# releases fewer. Postgres numbers a release `major.minor` since version 10, so
# `16.15` is one release and no `16.15.0` tag exists. Ubuntu numbers a release
# `year.month`, and tags no point release: `24.04` is the release.
COMPONENTS = {"postgres": 2, "library/postgres": 2, "ubuntu": 2, "library/ubuntu": 2}
EXACT = 3

# Each action that installs a browser or a driver, the input that names its
# version, and the form of an exact one. Chrome numbers a build with four
# components. Firefox numbers a release `major.minor`, and a fix release
# `major.minor.patch`. Each input otherwise defaults to a moving channel:
# `stable` for Chrome, `latest` for Firefox and geckodriver.
BROWSERS = {
    "browser-actions/setup-chrome": ("chrome-version", r"\d+\.\d+\.\d+\.\d+"),
    "browser-actions/setup-firefox": ("firefox-version", r"\d+\.\d+(\.\d+)?"),
    "browser-actions/setup-geckodriver": ("geckodriver-version", r"\d+\.\d+\.\d+"),
}
USES = re.compile(r"^(\s*)(- )?uses:\s*([^@\s]+)@")
# A `FROM` of a Dockerfile: the image, then an optional stage name.
FROM = re.compile(r"^\s*FROM\s+(?:--platform=\S+\s+)?(\S+)(?:\s+AS\s+(\S+))?", re.IGNORECASE)


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


def dockerfile_images(path: str, text: str) -> list[Image]:
    """Each image a `FROM` of one Dockerfile names. A `FROM` of an earlier stage names none."""
    stages: set[str] = set()
    out = []
    for number, line in enumerate(text.splitlines(), 1):
        match = FROM.match(line)
        if not match:
            continue
        value, stage = match.group(1), match.group(2)
        if value.lower() not in stages and value != "scratch":
            out.append(Image(path, number, ("FROM",), value))
        if stage:
            stages.add(stage.lower())
    return out


def built_services(text: str) -> set[str]:
    """The services of a compose file that name a `build:`: their image is the local build."""
    out = set()
    service = None
    for line in text.splitlines():
        match = KEY.match(line)
        if not match or line.lstrip().startswith("#"):
            continue
        indent = len(match.group(1))
        if indent == 2:
            service = match.group(2)
        elif indent == 4 and match.group(2) == "build" and service is not None:
            out.add(service)
    return out


def dev_breaches(compose: dict[str, Image], dev_text: str, dockerfile_text: str) -> list[str]:
    """Whether each image of the dev container is pinned, and names the compose image of its service."""
    out = []
    bases = dockerfile_images(DOCKERFILE, dockerfile_text)
    if not bases:
        out.append(f"{DOCKERFILE}: no `FROM` image read")
    for image in bases:
        out += pin_breaches(image)
    built = built_services(dev_text)
    for service, image in compose_services(images(DEV_COMPOSE, dev_text)).items():
        if service in built:
            continue
        out += pin_breaches(image)
        pinned = compose.get(service)
        if pinned is not None and pinned.value != image.value:
            out.append(
                f"{DEV_COMPOSE}:{image.line}: service `{service}` runs `{image.value}`, and "
                f"{COMPOSE}:{pinned.line} pins `{pinned.value}`"
            )
    return out


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


def indent_of(line: str) -> int:
    """The column of the first character that is not a space."""
    return len(line) - len(line.lstrip())


def step_of(lines: list[str], index: int) -> list[str]:
    """The lines of the list item that holds line `index`: from its dash down
    to the next line, not blank and not a comment, at the dash's column or less."""
    head = index
    while head > 0 and not lines[head].lstrip().startswith("- "):
        head -= 1
    dash = indent_of(lines[head])
    step = [lines[head]]
    for line in lines[head + 1:]:
        if line.strip() and not line.lstrip().startswith("#") and indent_of(line) <= dash:
            break
        step.append(line)
    return step


def browser_breaches(ci_text: str) -> tuple[list[str], int]:
    """Whether each step that installs a browser or a driver names its exact
    version; the breaches and the count of such steps."""
    lines = ci_text.splitlines()
    out = []
    count = 0
    for index, line in enumerate(lines):
        match = USES.match(line)
        if not match or match.group(3) not in BROWSERS:
            continue
        count += 1
        action = match.group(3)
        key, form = BROWSERS[action]
        value = None
        for other in step_of(lines, index):
            found = KEY.match(other.replace("- ", "  ", 1))
            if found and found.group(2) == key:
                value = strip_comment(found.group(3))
        where = f"{WORKFLOW}:{index + 1}"
        if value is None:
            out.append(f"{where}: `{action}` names no `{key}`, so it installs a moving channel")
        elif not re.fullmatch(form, value):
            out.append(f"{where}: `{action}` installs `{value}`, which is no exact version")
    return out, count


def breaches(
    compose_text: str, ci_text: str, dev_text: str | None = None, dockerfile_text: str | None = None
) -> tuple[list[str], int]:
    """Every breach of the four files, and the count of images read.

    A caller that plants no Dockerfile reads the compose file and the workflow alone.
    """
    compose = compose_services(images(COMPOSE, compose_text))
    ci = ci_services(images(WORKFLOW, ci_text))
    out = []
    for image in [*compose.values(), *ci]:
        out += pin_breaches(image)
    out += sync_breaches(compose, ci)
    if not compose:
        out.append(f"{COMPOSE}: no service image read")
    browsers, steps = browser_breaches(ci_text)
    out += browsers
    dev = 0
    if dockerfile_text is not None:
        dev_text = dev_text or ""
        out += dev_breaches(compose, dev_text, dockerfile_text)
        dev = len(dockerfile_images(DOCKERFILE, dockerfile_text))
        dev += len(set(compose_services(images(DEV_COMPOSE, dev_text))) - built_services(dev_text))
    return out, len(compose) + len(ci) + steps + dev


def read(path: str) -> str:
    """The text of one file of the repository."""
    return (ROOT / path).read_text(encoding="utf-8")


def main() -> int:
    errors, count = breaches(read(COMPOSE), read(WORKFLOW), read(DEV_COMPOSE), read(DOCKERFILE))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        print(f"service pins: {len(errors)} breach(es) in {count} images and browsers", file=sys.stderr)
        return 1
    print(f"service pins: {count} images and browsers read, each pinned to one version")
    return 0


if __name__ == "__main__":
    sys.exit(main())
