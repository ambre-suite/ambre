#!/usr/bin/env python3
"""The chart of `deploy/chart/ambre`: its gate, and the pin of a release.

`check` renders the chart through `helm template` under three sets of values:
the stores in the cluster, the stores outside it, and the stack under test of
`deploy/chart/test/values.yaml`. Each render fails on each of these:

* a pod that mounts a service account token, runs as root or under no
  seccomp profile;
* a container that may run as root, escalate, write its root filesystem, add a
  capability or keep one;
* a Secret the chart writes, or a variable of a secret that a value spells
  out: every secret comes from the Secret of the deployment;
* an SFU that is not one pod on the network of its node, or a Service that
  selects the SFU and carries its media port, or exposes it outside the
  cluster (`CALL-141`);
* a store image that names no digest, or another image than the compose file
  of `deploy/compose/` names for the same service (`ARCH-087`);
* with `--release`, an image of Ambre that names no digest (`SRV-308`).

`check` also reads `.github/workflows/release.yml`: a release builds and
pushes the relay, the SFU and the client, pins each digest into the chart,
checks the pinned chart and pushes it (`SRV-308`).

`pin VERSION relay=sha256:... sfu=... web=... init=...` writes the version
and the digests of a release into `Chart.yaml` and `values.yaml`.

The renders and the workflow are read with PyYAML, which the Python of the
CI runner ships as `python3-yaml`. Run: `just chart-check`. It writes nothing.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - the message is the whole behaviour
    sys.exit("chart.py needs PyYAML: install python3-yaml")

ROOT = Path(__file__).resolve().parent.parent
CHART = ROOT / "deploy/chart/ambre"
TEST_VALUES = ROOT / "deploy/chart/test/values.yaml"
COMPOSE = ROOT / "deploy/compose/compose.yml"
RELEASE = ROOT / ".github/workflows/release.yml"
KUBE_VERSION = "1.34.0"

# The images of Ambre, which a release publishes and pins.
AMBRE_IMAGES = ("relay", "sfu", "web", "init")
# The images a release must publish (`SRV-308`). `init` prepares the store.
PUBLISHED = ("relay", "sfu", "web")
DIGEST = re.compile(r"@sha256:[0-9a-f]{64}$")
# A variable that holds a secret: its value comes from a Secret alone.
SECRET_NAME = re.compile(r"(PASSWORD|SECRET|SEED|TOKEN|ADMISSION_KEY)$|^DATABASE_URL$|^REDIS_URL$")
# A value that reads a variable the pod got from the Secret is no secret in clear.
REFERENCE = re.compile(r"\$\([A-Z_]+\)")
BASE = ["--set", "domain=ambre.example.org,tls=admin@example.org,existingSecret=ambre"]
EXTERNAL = [
    *BASE,
    "--set",
    "postgres.enabled=false,redis.enabled=false,garage.enabled=false",
    "--set",
    "s3.endpoint=https://s3.example.org,s3.publicEndpoint=https://s3.example.org",
]


def render(*args: str) -> list[dict]:
    """The objects `helm template` writes for these values."""
    out = subprocess.run(
        ["helm", "template", "ambre", str(CHART), "--kube-version", KUBE_VERSION, *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if out.returncode != 0:
        raise SystemExit(f"helm template failed:\n{out.stderr}")
    return [doc for doc in yaml.safe_load_all(out.stdout) if doc]


def pod_of(obj: dict) -> dict | None:
    """The pod template of a workload, or None for any other object."""
    if obj.get("kind") in ("Deployment", "StatefulSet", "DaemonSet", "Job"):
        return obj["spec"]["template"]["spec"]
    return None


def component(obj: dict) -> str:
    """The component label of an object."""
    return obj.get("metadata", {}).get("labels", {}).get("app.kubernetes.io/component", "")


def check_pod(name: str, pod: dict) -> list[str]:
    """The security faults of one pod template."""
    faults = []
    if pod.get("automountServiceAccountToken") is not False:
        faults.append(f"{name}: the pod mounts a service account token")
    ctx = pod.get("securityContext", {})
    if ctx.get("runAsNonRoot") is not True or not ctx.get("runAsUser"):
        faults.append(f"{name}: the pod may run as root")
    if ctx.get("seccompProfile", {}).get("type") != "RuntimeDefault":
        faults.append(f"{name}: the pod runs under no seccomp profile")
    for c in [*pod.get("initContainers", []), *pod.get("containers", [])]:
        sc = c.get("securityContext", {})
        where = f"{name}/{c['name']}"
        if sc.get("runAsNonRoot") is not True or sc.get("runAsUser") == 0:
            faults.append(f"{where}: the container may run as root")
        if sc.get("allowPrivilegeEscalation") is not False or sc.get("privileged"):
            faults.append(f"{where}: the container may escalate")
        if sc.get("readOnlyRootFilesystem") is not True:
            faults.append(f"{where}: the container writes its root filesystem")
        caps = sc.get("capabilities", {})
        if caps.get("drop") != ["ALL"] or caps.get("add"):
            faults.append(f"{where}: the container keeps a capability")
        for env in c.get("env", []):
            value = env.get("value")
            if SECRET_NAME.search(env["name"]) and value and not REFERENCE.search(value):
                faults.append(f"{where}: {env['name']} spells a secret out")
    return faults


def check_sfu(objects: list[dict]) -> list[str]:
    """The faults of the SFU against `CALL-141`."""
    faults = []
    sfus = [o for o in objects if component(o) == "sfu" and pod_of(o) is not None]
    if len(sfus) != 1:
        return [f"the chart renders {len(sfus)} SFU workloads, not one"]
    sfu = sfus[0]
    pod = pod_of(sfu)
    if sfu["kind"] != "Deployment" or sfu["spec"].get("replicas") != 1:
        faults.append("the SFU is not one pod")
    if pod.get("hostNetwork") is not True:
        faults.append("the SFU runs off the network of its node")
    media = {
        p["containerPort"]
        for c in pod["containers"]
        for p in c.get("ports", [])
        if p.get("name") == "media"
    }
    if not media:
        faults.append("the SFU names no media port")
    labels = pod and sfu["spec"]["template"]["metadata"]["labels"]
    for svc in (o for o in objects if o.get("kind") == "Service"):
        selector = svc["spec"].get("selector", {})
        if not selector or any(labels.get(k) != v for k, v in selector.items()):
            continue
        name = svc["metadata"]["name"]
        if svc["spec"].get("type", "ClusterIP") != "ClusterIP":
            faults.append(f"{name}: a {svc['spec']['type']} Service fronts the SFU")
        for port in svc["spec"].get("ports", []):
            if port.get("targetPort") == "media" or port.get("port") in media:
                faults.append(f"{name}: a Service carries the media port of the SFU")
    return faults


def compose_images() -> dict[str, str]:
    """The image of each service of the compose file of `deploy/compose/`."""
    services = yaml.safe_load(COMPOSE.read_text())["services"]
    return {name: s["image"] for name, s in services.items() if "image" in s}


def check_images(objects: list[dict], compose: dict[str, str], release: bool) -> list[str]:
    """The faults of the images: the stores against the compose file, Ambre's by digest."""
    faults = []
    for obj in objects:
        pod = pod_of(obj)
        if pod is None:
            continue
        for c in [*pod.get("initContainers", []), *pod.get("containers", [])]:
            image, comp = c["image"], component(obj)
            ambre = comp in AMBRE_IMAGES or c["name"] == "garage-init"
            if ambre:
                if release and not DIGEST.search(image):
                    faults.append(f"{comp}/{c['name']}: {image} names no digest")
            elif not DIGEST.search(image):
                faults.append(f"{comp}: {image} names no digest")
            elif compose.get(comp) != image:
                faults.append(f"{comp}: {image} is not the image of the compose file")
    return faults


def check_objects(objects: list[dict], compose: dict[str, str], release: bool) -> list[str]:
    """Every fault of one render."""
    faults = []
    for obj in objects:
        if obj.get("kind") == "Secret":
            faults.append(f"the chart writes the Secret {obj['metadata']['name']}")
        pod = pod_of(obj)
        if pod is not None:
            faults += check_pod(f"{obj['kind']} {obj['metadata']['name']}", pod)
    return faults + check_sfu(objects) + check_images(objects, compose, release)


def steps(job: dict) -> list[dict]:
    """The steps of a job of a workflow."""
    return job.get("steps", [])


def check_release(workflow: dict) -> list[str]:
    """The faults of the release workflow against `SRV-308`."""
    faults = []
    jobs = workflow.get("jobs", {})
    images = jobs.get("images", {})
    targets = images.get("strategy", {}).get("matrix", {}).get("target", [])
    for target in PUBLISHED:
        if target not in targets:
            faults.append(f"the release builds no {target} image")
    pushes = [
        s
        for s in steps(images)
        if str(s.get("uses", "")).startswith("docker/build-push-action@")
        and s.get("with", {}).get("push") is True
    ]
    if not pushes:
        faults.append("the release pushes no image")
    chart = jobs.get("chart", {})
    runs = " ".join(str(s.get("run", "")) for s in steps(chart))
    for needle, fault in (
        ("scripts/chart.py pin", "the release pins no digest into the chart"),
        ("scripts/chart.py check --release", "the release checks no pinned chart"),
        ("helm push", "the release pushes no chart"),
    ):
        if needle not in runs:
            faults.append(fault)
    if "images" not in chart.get("needs", []):
        faults.append("the chart of a release does not wait for its images")
    return faults


def pin_values(text: str, digests: dict[str, str]) -> str:
    """`values.yaml` with the digest of each image of Ambre written in."""
    out, current, inside = [], None, False
    for line in text.splitlines(keepends=True):
        if re.match(r"^images:\s*$", line):
            inside = True
        elif inside and re.match(r"^\S", line):
            inside = False
        if inside:
            m = re.match(r"^  ([a-z]+):\s*$", line)
            if m:
                current = m.group(1)
            m = re.match(r"^(    digest:)\s*.*$", line)
            if m and current in digests:
                line = f'{m.group(1)} "{digests[current]}"\n'
                digests = {k: v for k, v in digests.items() if k != current}
        out.append(line)
    if digests:
        raise SystemExit(f"values.yaml holds no digest field for {sorted(digests)}")
    return "".join(out)


def pin_chart(text: str, version: str) -> str:
    """`Chart.yaml` at the version of a release."""
    text = re.sub(r"(?m)^version: .*$", f"version: {version}", text)
    return re.sub(r"(?m)^appVersion: .*$", f'appVersion: "{version}"', text)


def pin(version: str, pairs: list[str]) -> int:
    """Writes a release into the chart."""
    if not re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?", version):
        raise SystemExit(f"{version}: no semantic version")
    digests = dict(p.split("=", 1) for p in pairs)
    if sorted(digests) != sorted(AMBRE_IMAGES):
        raise SystemExit(f"pin names {sorted(digests)}, not {sorted(AMBRE_IMAGES)}")
    for name, digest in digests.items():
        if not DIGEST.search("@" + digest):
            raise SystemExit(f"{name}: {digest} is no sha256 digest")
    values = CHART / "values.yaml"
    values.write_text(pin_values(values.read_text(), digests))
    chart = CHART / "Chart.yaml"
    chart.write_text(pin_chart(chart.read_text(), version))
    return 0


def check(release: bool) -> int:
    """Runs the gate."""
    lint = subprocess.run(
        ["helm", "lint", str(CHART), "--kube-version", KUBE_VERSION, *BASE],
        capture_output=True,
        text=True,
        check=False,
    )
    faults = [] if lint.returncode == 0 else [f"helm lint failed:\n{lint.stdout}{lint.stderr}"]
    compose = compose_images()
    renders = {
        "stores in the cluster": render(*BASE),
        "stores outside it": render(*EXTERNAL),
        "the stack under test": render("-f", str(TEST_VALUES)),
    }
    for name, objects in renders.items():
        under_test = name == "the stack under test"
        faults += [f"{name}: {f}" for f in check_objects(objects, compose, release and not under_test)]
    if not release:
        faults += [f"release.yml: {f}" for f in check_release(yaml.safe_load(RELEASE.read_text()))]
    for fault in faults:
        print(f"chart: {fault}", file=sys.stderr)
    if faults:
        return 1
    print(f"chart: {sum(len(r) for r in renders.values())} objects in {len(renders)} renders hold")
    return 0


def main(argv: list[str]) -> int:
    if argv[:1] == ["check"]:
        return check(release=argv[1:] == ["--release"])
    if argv[:1] == ["pin"] and len(argv) >= 2:
        return pin(argv[1], argv[2:])
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
