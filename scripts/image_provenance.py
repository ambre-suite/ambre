#!/usr/bin/env python3
"""The provenance of the images of a release: its gate, and its verification.

A release pushes the images of the relay, the SFU, the client and the store
preparation (`SRV-308`). An operator who pulls one must prove which commit and
which workflow built it. Each job that pushes an image therefore attests the
digest it pushed, signed through the OIDC token of the job, and pushes the
attestation to the registry beside the image.

`check` reads each workflow under `.github/workflows/` and the `justfile`, and
fails on each of these:

* a workflow that grants `id-token` or `attestations` to every job (`SRV-355`);
* a job that pushes an image and attests no digest of it with
  `actions/attest-build-provenance`, pinned by a commit, pushed to the registry
  (`SRV-354`);
* a job that pushes an image outside `docker/build-push-action`, whose digest
  no step can name (`SRV-354`);
* a job that holds `id-token: write` or `attestations: write` and pushes no
  image, or a job that pushes one and lacks either (`SRV-355`);
* a release whose chart test does not verify the images, or a chart pushed
  before that test passes (`SRV-356`);
* a chart test of a release that loads or pulls an image before the
  verification of `pull`, or pulls one outside it (`SRV-356`).

`verify LIST --source-ref REF --source-digest SHA` verifies each image of LIST,
one `registry/repository/name@sha256:...` per line, as `ambre-images.txt` on a
release lists them. `pull` with the same arguments verifies every image first,
then pulls each one by its digest and tags it `ambre-<name>:local`, the name
the values of the chart test read. A line that names a tag and no digest fails
before any call: a tag moves, a digest does not.

The verification is `gh attestation verify`, against this repository, the
release workflow, the ref and the commit of the release. It reads the bundle
from the registry, so the check proves the attestation travels with the image.

Run: `just image-provenance` for the gate. It writes nothing.
"""

from __future__ import annotations

import re
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - the message is the whole behaviour
    sys.exit("image_provenance.py needs PyYAML: install python3-yaml")

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / ".github/workflows"
JUSTFILE = ROOT / "justfile"

# The repository and the workflow an attestation must name. They are constants
# and not read from the environment: a fork or a compromised job then fails.
REPOSITORY = "ambre-suite/ambre"
RELEASE = "release.yml"
SIGNER_WORKFLOW = f"{REPOSITORY}/.github/workflows/{RELEASE}"
ATTEST = "actions/attest-build-provenance@"
BUILD_PUSH = "docker/build-push-action@"
PINNED = re.compile(r"@[0-9a-f]{40}$")
# The recipe and the job of the release that install its images on a cluster.
RECIPE = "chart-test-release"
PULL = "scripts/image_provenance.py pull"
# The steps of a recipe that put an image on the cluster or on the runner.
LOADS = ("kind load", "docker pull", "helm install", "helm --kube-context")
SIGNING = ("id-token", "attestations")

IMAGE = re.compile(r"^(?P<repo>[a-z0-9.-]+(?::[0-9]+)?(?:/[a-z0-9._-]+)+?)/(?P<name>[a-z0-9-]+)@sha256:[0-9a-f]{64}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
GIT_REF = re.compile(r"^refs/tags/v[0-9A-Za-z.+-]+$")

Run = Callable[[list[str]], int]


def steps(job: dict) -> list[dict]:
    """The steps of a job."""
    return [s for s in job.get("steps", []) if isinstance(s, dict)]


def uses(step: dict) -> str:
    """The action a step uses, or the empty string."""
    return str(step.get("uses", ""))


def grants(permissions: object) -> set[str]:
    """The signing scopes a `permissions` value writes."""
    if permissions == "write-all":
        return set(SIGNING)
    if not isinstance(permissions, dict):
        return set()
    return {scope for scope in SIGNING if permissions.get(scope) == "write"}


def pushes(step: dict) -> bool:
    """Whether a step of `docker/build-push-action` pushes its image."""
    push = step.get("with", {}).get("push")
    return uses(step).startswith(BUILD_PUSH) and (push is True or str(push).lower() == "true")


def check_job(where: str, job: dict) -> list[str]:
    """The faults of one job against `SRV-354` and `SRV-355`."""
    faults = []
    all_steps = steps(job)
    pushed = [(i, s) for i, s in enumerate(all_steps) if pushes(s)]
    for s in all_steps:
        if "docker push" in str(s.get("run", "")):
            faults.append(f"{where}: a step pushes an image outside {BUILD_PUSH[:-1]}, so no step attests its digest")
    for index, step in pushed:
        name = step.get("id")
        if not name:
            faults.append(f"{where}: a step pushes an image and has no id, so no step can name its digest")
            continue
        digest = f"${{{{ steps.{name}.outputs.digest }}}}"
        attests = [
            s
            for s in all_steps[index + 1 :]
            if uses(s).startswith(ATTEST) and s.get("with", {}).get("subject-digest") == digest
        ]
        if not attests:
            faults.append(f"{where}: the image of step {name} is pushed with no attestation of its digest")
        for s in attests:
            if not PINNED.search(uses(s)):
                faults.append(f"{where}: {uses(s)} names no commit")
            if s.get("with", {}).get("push-to-registry") is not True:
                faults.append(f"{where}: the attestation of step {name} stays off the registry")
            if not s.get("with", {}).get("subject-name"):
                faults.append(f"{where}: the attestation of step {name} names no image")
    held = grants(job.get("permissions"))
    if pushed and held != set(SIGNING):
        missing = sorted(set(SIGNING) - held)
        faults.append(f"{where}: the job pushes an image and cannot sign: no {', '.join(missing)} write")
    if not pushed and held:
        faults.append(f"{where}: the job pushes no image and holds {', '.join(sorted(held))} write")
    return faults


def check_workflow(name: str, workflow: dict) -> list[str]:
    """The faults of one workflow against `SRV-354` and `SRV-355`."""
    faults = []
    held = grants(workflow.get("permissions"))
    if held:
        faults.append(f"{name}: the workflow grants {', '.join(sorted(held))} write to every job")
    for job_name, job in (workflow.get("jobs") or {}).items():
        if isinstance(job, dict):
            faults += check_job(f"{name} job {job_name}", job)
    return faults


def recipe(justfile: str, name: str) -> list[str]:
    """The body lines of one recipe of the justfile, or an empty list."""
    lines = justfile.splitlines()
    head = re.compile(rf"^{re.escape(name)}(\s[^:]*)?:")
    for i, line in enumerate(lines):
        if head.match(line):
            body = []
            for line in lines[i + 1 :]:
                if line and not line[0].isspace():
                    break
                body.append(line)
            return body
    return []


def check_install(release: dict, justfile: str) -> list[str]:
    """The faults of the install path of a release against `SRV-356`."""
    faults = []
    jobs = release.get("jobs") or {}
    testers = [
        name
        for name, job in jobs.items()
        if any(f"just {RECIPE}" in str(s.get("run", "")) for s in steps(job))
    ]
    if not testers:
        faults.append(f"{RELEASE}: no job runs `just {RECIPE}`, so no chart test verifies the images")
    for name in testers:
        for s in steps(jobs[name]):
            run = str(s.get("run", ""))
            if any(load in run for load in LOADS):
                faults.append(f"{RELEASE} job {name}: a step loads an image outside `just {RECIPE}`")
    needs = (jobs.get("chart") or {}).get("needs", [])
    needs = [needs] if isinstance(needs, str) else needs
    if testers and not set(testers) & set(needs):
        faults.append(f"{RELEASE}: the chart is pushed before the chart test verifies its images")
    body = recipe(justfile, RECIPE)
    if not body:
        return faults + [f"justfile: no recipe `{RECIPE}`"]
    verify = next((i for i, line in enumerate(body) if PULL in line), None)
    if verify is None:
        faults.append(f"justfile: `{RECIPE}` never runs `{PULL}`")
    for i, line in enumerate(body):
        if "docker pull" in line:
            faults.append(f"justfile: `{RECIPE}` pulls an image outside `{PULL}`")
        elif any(load in line for load in LOADS) and (verify is None or i < verify):
            faults.append(f"justfile: `{RECIPE}` loads an image before `{PULL}` verifies it")
    return faults


def check() -> int:
    """Runs the gate."""
    faults = []
    found = sorted([*WORKFLOWS.glob("*.yml"), *WORKFLOWS.glob("*.yaml")])
    for path in found:
        faults += check_workflow(path.name, yaml.safe_load(path.read_text()) or {})
    release = WORKFLOWS / RELEASE
    if release.exists():
        faults += check_install(yaml.safe_load(release.read_text()) or {}, JUSTFILE.read_text())
    else:
        faults.append(f"no {RELEASE} under {WORKFLOWS}")
    for fault in faults:
        print(f"image-provenance: {fault}", file=sys.stderr)
    if faults:
        return 1
    print(f"image-provenance: {len(found)} workflows hold")
    return 0


def images(text: str) -> list[tuple[str, str]]:
    """The reference and the name of each image of a list, or an exit on a tag."""
    out = []
    for line in text.splitlines():
        ref = line.strip()
        if not ref:
            continue
        m = IMAGE.match(ref)
        if not m:
            raise SystemExit(f"image-provenance: {ref} names no sha256 digest")
        out.append((ref, m.group("name")))
    if not out:
        raise SystemExit("image-provenance: the list names no image")
    return out


def verify_command(ref: str, source_ref: str, source_digest: str) -> list[str]:
    """The `gh attestation verify` call that proves where one image comes from."""
    return [
        "gh", "attestation", "verify", f"oci://{ref}",
        "--repo", REPOSITORY,
        "--signer-workflow", SIGNER_WORKFLOW,
        "--source-ref", source_ref,
        "--source-digest", source_digest,
        "--deny-self-hosted-runners",
        "--bundle-from-oci",
    ]


def verify(refs: list[tuple[str, str]], source_ref: str, source_digest: str, run: Run) -> int:
    """Verifies every image, and names each one that fails."""
    if not GIT_REF.match(source_ref):
        raise SystemExit(f"image-provenance: {source_ref} is no release tag ref")
    if not COMMIT.match(source_digest):
        raise SystemExit(f"image-provenance: {source_digest} is no commit")
    failed = [ref for ref, _ in refs if run(verify_command(ref, source_ref, source_digest)) != 0]
    for ref in failed:
        print(f"image-provenance: {ref} carries no provenance of {source_ref}", file=sys.stderr)
    return 1 if failed else 0


def pull(refs: list[tuple[str, str]], source_ref: str, source_digest: str, run: Run) -> int:
    """Pulls each image by its digest, once every image of the list verifies."""
    if verify(refs, source_ref, source_digest, run) != 0:
        return 1
    for ref, name in refs:
        if run(["docker", "pull", ref]) != 0 or run(["docker", "tag", ref, f"ambre-{name}:local"]) != 0:
            print(f"image-provenance: {ref} did not pull", file=sys.stderr)
            return 1
    return 0


def system(cmd: list[str]) -> int:
    """Runs one command and returns its exit code."""
    return subprocess.run(cmd, check=False).returncode


def main(argv: list[str], run: Run = system) -> int:
    if argv[:1] == ["check"]:
        return check()
    if argv[:1] in (["verify"], ["pull"]) and len(argv) == 6 and argv[2] == "--source-ref" and argv[4] == "--source-digest":
        refs = images(Path(argv[1]).read_text())
        act = verify if argv[0] == "verify" else pull
        return act(refs, argv[3], argv[5], run)
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
