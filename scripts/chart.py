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
* a pod off the network of its node that a NetworkPolicy leaves open in
  either direction, or whose policies admit a flow the table of `FLOWS` does
  not name, or refuse one it names (`SRV-351`). The web server reaches the
  signalling port of the SFU on the blocks of `networkPolicy.nodeCidrs`
  alone, so a rule with no peer on that port is a fault (`SRV-361`);
* a web pod that starts Caddy before an init container of its own image
  compares the key the image pins, in `/etc/ambre/thread-link-key`, with the
  `thread-link-key` of the Secret, or that reads that key as optional
  (`SRV-430`);
* with `--release`, an image of Ambre that names no digest (`SRV-308`).

`check` also renders the chart under `NODE_REFUSALS`: no block of nodes, and
blocks as wide as an address family. Each render must fail (`SRV-362`).

`check` also reads `.github/workflows/release.yml`: a release builds and
pushes the relay, the SFU and the client, pins each digest into the chart,
checks the pinned chart and pushes it (`SRV-308`).

`pin VERSION relay=sha256:... sfu=... web=... init=...` writes the version
and the digests of a release into `Chart.yaml` and `values.yaml`.

`probe CONTEXT` runs on the cluster of `just chart-test`: two pods the flow
table names nowhere try each port of the stores and of the relay, and a
refused connection is the pass. The pod outside the release also reaches the
web server, so a refusal is the policy and not a broken network (`SRV-352`).

`link-key [--plant] CONTEXT` runs on a cluster of the chart: the web pod
stops in its init container, and its log names the key the image pins and
the key of the Secret (`SRV-430`). With `--plant`, it first writes another
key into the Secret and restarts the web server, as `just chart-test` does
on a stack that runs. Without it, the Secret already names another key, as
in the chart test of a release.

`nodes CONTEXT` runs on the same cluster before the install: each node holds
an address inside the `networkPolicy.nodeCidrs` of the test values, or the
web server cannot reach the signalling of the SFU (`SRV-361`).

The renders and the workflow are read with PyYAML, which the Python of the
CI runner ships as `python3-yaml`. Run: `just chart-check`. It writes nothing.
"""

from __future__ import annotations

import base64
import ipaddress
import json
import re
import secrets
import subprocess
import sys
import time
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
# The address blocks of the nodes in the two renders outside the test values.
NODE_CIDRS = ["192.0.2.0/24", "2001:db8::/64"]
BASE = [
    "--set",
    "domain=ambre.example.org,tls=admin@example.org,existingSecret=ambre",
    "--set-json",
    "networkPolicy.nodeCidrs=" + json.dumps(NODE_CIDRS),
]
# Blocks of nodes the chart must refuse to render (`SRV-362`): none, or one
# that reopens the signalling port of the SFU on the internet.
NODE_REFUSALS = ([], ["0.0.0.0/0"], ["::/0"], ["10.0.0.0/7"], ["fc00::/31"], ["10.0.0.0"])
# The addresses of the stores outside the cluster, in the render that names them.
EXTERNAL_STORES = [{"to": [{"ipBlock": {"cidr": "192.0.2.10/32"}}], "ports": [{"port": 5432}, {"port": 6379}]}]
EXTERNAL = [
    *BASE,
    "--set",
    "postgres.enabled=false,redis.enabled=false,garage.enabled=false",
    "--set",
    "s3.endpoint=https://s3.example.org,s3.publicEndpoint=https://s3.example.org",
    "--set-json",
    "networkPolicy.externalStores=" + json.dumps(EXTERNAL_STORES),
]

# A peer or a port a rule leaves out: every one.
ANYWHERE = "anywhere"
# The private, shared, link-local and loopback ranges a public peer excepts.
PRIVATE = {
    "0.0.0.0/0": {"10.0.0.0/8", "100.64.0.0/10", "127.0.0.0/8", "169.254.0.0/16", "172.16.0.0/12", "192.168.0.0/16"},
    "::/0": {"::1/128", "fc00::/7", "fe80::/10"},
}
DNS = {"kubernetes.io/metadata.name": "kube-system"}, {"k8s-app": "kube-dns"}


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


def rendered(*args: str) -> bool:
    """Whether `helm template` renders the chart under these values."""
    out = subprocess.run(
        ["helm", "template", "ambre", str(CHART), "--kube-version", KUBE_VERSION, *args],
        capture_output=True,
        text=True,
        check=False,
    )
    return out.returncode == 0


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


def flows_table(
    present: set[str], acme: bool, external: bool, nodes: list[str]
) -> set[tuple[str, str, str, str]]:
    """The flow table of `ambre-spec-server.md` section 3, for the stores a render runs.

    Each flow reads (pod, direction, peer, port). `public` is every address
    but the private ranges of `PRIVATE`, and `dns` the resolver of the cluster.
    `nodes` holds the address blocks of the nodes that the render names.
    """
    flows = {
        ("web", "ingress", ANYWHERE, "TCP/8080"),
        ("web", "ingress", ANYWHERE, "TCP/8443"),
        ("web", "egress", "relay", "TCP/3000"),
        ("relay", "ingress", "web", "TCP/3000"),
        ("relay", "egress", "public", "TCP/443"),
    }
    # The SFU listens on the address of its node, which no pod selector names.
    flows |= {("web", "egress", f"ipBlock {cidr}", "TCP/3480") for cidr in nodes}
    for pod in ("web", "relay"):
        flows |= {(pod, "egress", "dns", "UDP/53"), (pod, "egress", "dns", "TCP/53")}
    if acme:
        flows.add(("web", "egress", "public", "TCP/443"))
    for store, port in (("postgres", "TCP/5432"), ("redis", "TCP/6379")):
        if store in present:
            flows |= {("relay", "egress", store, port), (store, "ingress", "relay", port)}
    if "garage" in present:
        for port in ("TCP/3900", "TCP/3903"):
            flows |= {("relay", "egress", "garage", port), ("garage", "ingress", "relay", port)}
        flows |= {("web", "egress", "garage", "TCP/3900"), ("garage", "ingress", "web", "TCP/3900")}
    if external:
        for rule in EXTERNAL_STORES:
            for peer in rule["to"]:
                for port in rule["ports"]:
                    flows.add(("relay", "egress", f"ipBlock {peer['ipBlock']['cidr']}", f"TCP/{port['port']}"))
    return flows


def selects(selector: dict, labels: dict) -> bool:
    """Whether a label selector of a NetworkPolicy matches these labels."""
    if set(selector) - {"matchLabels"}:
        return False
    return all(labels.get(k) == v for k, v in selector.get("matchLabels", {}).items())


def peer_name(peer: dict, pods: dict[str, dict]) -> str:
    """The name the flow table gives a peer of a rule."""
    block = peer.get("ipBlock")
    if block:
        if set(block.get("except", [])) == PRIVATE.get(block["cidr"]):
            return "public"
        return f"ipBlock {block['cidr']}"
    ns, pod = peer.get("namespaceSelector"), peer.get("podSelector")
    if ns is not None:
        if pod is not None and ns.get("matchLabels") == DNS[0] and pod.get("matchLabels") == DNS[1]:
            return "dns"
        return f"namespaces {ns}"
    matched = sorted(name for name, labels in pods.items() if selects(pod, labels))
    if len(matched) == 1 and pod.get("matchLabels", {}).get("app.kubernetes.io/component") == matched[0]:
        return matched[0]
    return f"pods {pod}"


def check_network(objects: list[dict], expected: set[tuple[str, str, str, str]]) -> list[str]:
    """The faults of the NetworkPolicies of one render against its flow table (`SRV-351`)."""
    pods = {
        component(o): o["spec"]["template"]["metadata"]["labels"]
        for o in objects
        if pod_of(o) is not None and not pod_of(o).get("hostNetwork")
    }
    policies = [o["spec"] for o in objects if o.get("kind") == "NetworkPolicy"]
    faults, found = [], set()
    for name, labels in pods.items():
        mine = [p for p in policies if selects(p.get("podSelector", {}), labels)]
        for direction, key, side in (("ingress", "from", "Ingress"), ("egress", "to", "Egress")):
            if not any(side in p.get("policyTypes", []) for p in mine):
                faults.append(f"{name}: no NetworkPolicy isolates its {direction}")
                continue
            for policy in mine:
                if side not in policy.get("policyTypes", []):
                    continue
                for rule in policy.get(direction, []):
                    peers = [peer_name(p, pods) for p in rule.get(key, [])] or [ANYWHERE]
                    ports = [
                        f"{p.get('protocol', 'TCP')}/{p['port']}" if "port" in p else f"{p.get('protocol', 'TCP')}/any"
                        for p in rule.get("ports", [])
                    ] or ["any port"]
                    found |= {(name, direction, peer, port) for peer in peers for port in ports}
    for flow in sorted(found - expected):
        faults.append("{}: its {} admits {} on {}, which the flow table does not name".format(*flow))
    for flow in sorted(expected - found):
        if flow[0] in pods:
            faults.append("{}: its {} refuses {} on {}, which the flow table names".format(*flow))
    return faults


# The init container of the web pod, the file of its image and the key of
# the Secret it compares (`SRV-430`).
LINK_INIT = "link-key"
LINK_FILE = "/etc/ambre/thread-link-key"
LINK_SECRET_KEY = "thread-link-key"


def check_link_key(objects: list[dict]) -> list[str]:
    """The faults of the web pod against `SRV-430`."""
    webs = [o for o in objects if component(o) == "web" and pod_of(o) is not None]
    if len(webs) != 1:
        return [f"{len(webs)} web workloads, not one"]
    pod = pod_of(webs[0])
    images = {c.get("image") for c in pod.get("containers", [])}
    init = next((c for c in pod.get("initContainers", []) if c.get("name") == LINK_INIT), None)
    if init is None:
        return [f"the web pod starts with no init container {LINK_INIT}, so it serves a client of another key"]
    faults = []
    if {init.get("image")} != images:
        faults.append(f"web/{LINK_INIT}: runs another image than the web server, so it reads another pinned key")
    if LINK_FILE not in " ".join(map(str, init.get("command", []) + init.get("args", []))):
        faults.append(f"web/{LINK_INIT}: does not read {LINK_FILE}")
    refs = [
        e.get("valueFrom", {}).get("secretKeyRef", {})
        for e in init.get("env", [])
        if e.get("name") == "THREAD_LINK_KEY"
    ]
    if not refs or refs[0].get("key") != LINK_SECRET_KEY:
        faults.append(f"web/{LINK_INIT}: THREAD_LINK_KEY does not come from {LINK_SECRET_KEY} of the Secret")
    elif refs[0].get("optional"):
        faults.append(f"web/{LINK_INIT}: {LINK_SECRET_KEY} is optional, so a Secret without it starts the pod")
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
    test_nodes = yaml.safe_load(TEST_VALUES.read_text())["networkPolicy"]["nodeCidrs"]
    nodes = {"stores in the cluster": NODE_CIDRS, "stores outside it": NODE_CIDRS, "the stack under test": test_nodes}
    renders = {
        "stores in the cluster": render(*BASE),
        "stores outside it": render(*EXTERNAL),
        "the stack under test": render("-f", str(TEST_VALUES)),
    }
    for name, objects in renders.items():
        under_test = name == "the stack under test"
        faults += [f"{name}: {f}" for f in check_objects(objects, compose, release and not under_test)]
        faults += [f"{name}: {f}" for f in check_link_key(objects)]
        present = {component(o) for o in objects if pod_of(o) is not None}
        table = flows_table(present, acme=not under_test, external=name == "stores outside it", nodes=nodes[name])
        faults += [f"{name}: {f}" for f in check_network(objects, table)]
    for cidrs in NODE_REFUSALS:
        if rendered(*BASE, "--set-json", "networkPolicy.nodeCidrs=" + json.dumps(cidrs)):
            faults.append(f"the chart renders under networkPolicy.nodeCidrs={json.dumps(cidrs)}")
    if not release:
        faults += [f"release.yml: {f}" for f in check_release(yaml.safe_load(RELEASE.read_text()))]
    for fault in faults:
        print(f"chart: {fault}", file=sys.stderr)
    if faults:
        return 1
    print(f"chart: {sum(len(r) for r in renders.values())} objects in {len(renders)} renders hold")
    return 0


# The release of `just chart-test`, and the kubectl that reaches its cluster.
PROBE_RELEASE = "ambre"
PROBE_KUBECTL = ["kubectl", "--context", "kind-ambre-chart"]
# What a pod the flow table names nowhere tries and must not reach (`SRV-352`).
REFUSED = (("postgres", 5432), ("redis", 6379), ("garage", 3900), ("garage", 3903), ("relay", 3000), ("relay", 9100))
# The one entry every pod reaches: the web server on the port of its Service.
OPEN = ("web", 443)
# The image of the probe, which `just chart-test` loads, and its program: one
# line per target, `host port 1` when a connection opened and `0` otherwise.
PROBE_IMAGE = "ambre-init:local"
PROBE_SCRIPT = """
import socket, sys
for target in sys.argv[1:]:
    host, port = target.rsplit(":", 1)
    try:
        socket.create_connection((host, int(port)), timeout=5).close()
        print(host, port, 1)
    except OSError:
        print(host, port, 0)
"""
# The two probes: outside the release, and inside it under no component the
# table names. The labels of the second one put it under the deny policy.
PROBES = {
    "ambre-probe-outside": "app.kubernetes.io/name=probe",
    "ambre-probe-inside": f"app.kubernetes.io/name=ambre,app.kubernetes.io/instance={PROBE_RELEASE},app.kubernetes.io/component=probe",
}


def probe_faults(probe: str, lines: str, targets: dict[str, bool]) -> list[str]:
    """The faults of one probe: a target it reached that it must not, or the reverse.

    `targets` maps each `host:port` to whether the probe must reach it.
    """
    seen = {}
    for line in lines.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[2] in ("0", "1"):
            seen[f"{parts[0]}:{parts[1]}"] = parts[2] == "1"
    faults = []
    for target, must in targets.items():
        if target not in seen:
            faults.append(f"{probe}: no answer for {target}")
        elif seen[target] and not must:
            faults.append(f"{probe}: reached {target}, which the flow table does not name")
        elif must and not seen[target]:
            faults.append(f"{probe}: did not reach {target}, so a refusal proves nothing")
    return faults


def kube(kubectl: list[str], *args: str) -> str:
    """The output of one kubectl command, or an exit on its failure."""
    out = subprocess.run([*kubectl, *args], capture_output=True, text=True, check=False)
    if out.returncode != 0:
        raise SystemExit(f"kubectl {' '.join(args)} failed:\n{out.stderr}")
    return out.stdout


def probe(kubectl: list[str]) -> int:
    """Runs the two probes on the cluster of `just chart-test` (`SRV-352`)."""
    def name(svc: str) -> str:
        return f"{PROBE_RELEASE}-{svc}"

    # The inside probe resolves no name, since its policy admits no resolver.
    ip = {svc: kube(kubectl, "get", "svc", name(svc), "-o", "jsonpath={.spec.clusterIP}").strip() for svc, _ in REFUSED}
    plans = {
        "ambre-probe-outside": {
            **{f"{name(svc)}:{port}": False for svc, port in REFUSED},
            f"{name(OPEN[0])}:{OPEN[1]}": True,
        },
        "ambre-probe-inside": {f"{ip[svc]}:{port}": False for svc, port in REFUSED},
    }
    faults = []
    for pod, targets in plans.items():
        kube(kubectl, "delete", "pod", pod, "--ignore-not-found", "--wait=true")
        kube(
            kubectl, "run", pod, "--image", PROBE_IMAGE, "--image-pull-policy", "Never",
            "--restart", "Never", "--labels", PROBES[pod], "--command", "--",
            "python3", "-c", PROBE_SCRIPT, *targets,
        )
        kube(kubectl, "wait", f"pod/{pod}", "--for=jsonpath={.status.phase}=Succeeded", "--timeout=180s")
        faults += probe_faults(pod, kube(kubectl, "logs", pod), targets)
        kube(kubectl, "delete", "pod", pod, "--wait=false")
    for fault in faults:
        print(f"chart: {fault}", file=sys.stderr)
    if faults:
        return 1
    print(f"chart: {sum(len(t) for t in plans.values())} connections of two probes hold the flow table")
    return 0


def link_key_faults(pods: dict, log: str, pinned: str, named: str) -> list[str]:
    """The faults of the web pods of a cluster against `SRV-430`.

    `pods` is the list of the pods of the web server, as `kubectl get -o json`
    prints it, and `log` the log of the init container of the refused pod.
    The pass is a pod whose init container failed, whose web server never
    ran, and whose log names `pinned`, the key of the image, and `named`, the
    key of the Secret.
    """
    if pinned == named:
        return [f"the Secret names {named}, the key the image pins, so a refusal proves nothing"]
    items = pods.get("items", [])
    if not items:
        return ["the cluster runs no web pod"]
    faults = []
    refused = False
    for pod in items:
        name = pod.get("metadata", {}).get("name", "?")
        status = pod.get("status", {})
        for c in status.get("containerStatuses", []):
            if "running" in c.get("state", {}) or c.get("ready"):
                faults.append(f"{name}: the web server runs under another key than the image pins")
        for c in status.get("initContainerStatuses", []):
            if c.get("name") != LINK_INIT:
                continue
            ends = [c.get(k, {}).get("terminated", {}) for k in ("state", "lastState")]
            refused |= any(e and e.get("exitCode", 0) != 0 for e in ends)
    if not refused:
        faults.append(f"no web pod shows {LINK_INIT} failed")
    for key, what in ((pinned, "the key the image pins"), (named, "the key of the Secret")):
        if key not in log:
            faults.append(f"the log of {LINK_INIT} does not name {what}, {key}")
    return faults


def link_key(kubectl: list[str], plant: bool) -> int:
    """Finds the web pod refused under a key the image does not pin (`SRV-430`)."""
    secret = yaml.safe_load(TEST_VALUES.read_text())["existingSecret"]
    deployment = f"deployment/{PROBE_RELEASE}-web"
    image = yaml.safe_load(TEST_VALUES.read_text())["images"]["web"]
    out = subprocess.run(
        ["docker", "run", "--rm", "--entrypoint", "cat", f"{image['repository']}:{image['tag']}", LINK_FILE],
        capture_output=True,
        text=True,
        check=False,
    )
    if out.returncode != 0:
        raise SystemExit(f"chart: the web image holds no {LINK_FILE}:\n{out.stderr}")
    pinned = out.stdout.strip().lower()
    if plant:
        # Any 32 bytes but the pinned ones: the pod compares the text alone.
        named = secrets.token_hex(32)
        patch = json.dumps({"stringData": {LINK_SECRET_KEY: named}})
        kube(kubectl, "patch", "secret", secret, "--type", "merge", "-p", patch)
        kube(kubectl, "rollout", "restart", deployment)
    else:
        path = "jsonpath={.data." + LINK_SECRET_KEY + "}"
        named = base64.b64decode(kube(kubectl, "get", "secret", secret, "-o", path)).decode().strip().lower()
    selector = f"app.kubernetes.io/instance={PROBE_RELEASE},app.kubernetes.io/component=web"
    faults: list[str] = []
    # The pod runs its init container once per start, so the verdict waits
    # for the first failure, or for the restart to leave the old pod.
    for _ in range(36):
        time.sleep(5)
        pods = json.loads(kube(kubectl, "get", "pods", "-l", selector, "-o", "json"))
        failed = [
            p["metadata"]["name"]
            for p in pods.get("items", [])
            if any(
                c.get(k, {}).get("terminated", {}).get("exitCode", 0) != 0
                for c in p.get("status", {}).get("initContainerStatuses", [])
                for k in ("state", "lastState")
            )
        ]
        if not failed or len(pods.get("items", [])) != 1:
            continue
        log = subprocess.run(
            [*kubectl, "logs", failed[0], "-c", LINK_INIT, "--previous"], capture_output=True, text=True, check=False
        )
        if log.returncode != 0:
            log = subprocess.run([*kubectl, "logs", failed[0], "-c", LINK_INIT], capture_output=True, text=True, check=False)
        faults = link_key_faults(pods, log.stdout + log.stderr, pinned, named)
        break
    else:
        faults = ["no web pod failed its link key within 180 s"]
    for fault in faults:
        print(f"chart: {fault}", file=sys.stderr)
    if faults:
        return 1
    print(f"chart: the web pod refused {named}, as its image pins {pinned}")
    return 0


def nodes_outside(cidrs: list[str], addresses: list[str]) -> list[str]:
    """The faults of the nodes against the blocks of the test values (`SRV-361`)."""
    if not addresses:
        return ["the cluster lists no address of a node"]
    blocks = [ipaddress.ip_network(c) for c in cidrs]
    return [
        f"node address {a} lies outside networkPolicy.nodeCidrs {cidrs}"
        for a in addresses
        if not any(ipaddress.ip_address(a) in b for b in blocks)
    ]


def nodes(kubectl: list[str]) -> int:
    """Checks the nodes of the cluster of `just chart-test` against the test values."""
    cidrs = yaml.safe_load(TEST_VALUES.read_text())["networkPolicy"]["nodeCidrs"]
    path = '{range .items[*]}{.status.addresses[?(@.type=="InternalIP")].address}{"\\n"}{end}'
    addresses = kube(kubectl, "get", "nodes", "-o", f"jsonpath={path}").split()
    faults = nodes_outside(cidrs, addresses)
    for fault in faults:
        print(f"chart: {fault}", file=sys.stderr)
    if faults:
        return 1
    print(f"chart: {len(addresses)} node addresses lie inside {cidrs}")
    return 0


def main(argv: list[str]) -> int:
    if argv[:1] == ["check"]:
        return check(release=argv[1:] == ["--release"])
    if argv[:1] == ["probe"]:
        return probe(argv[1:] or PROBE_KUBECTL)
    if argv[:1] == ["link-key"]:
        plant = argv[1:2] == ["--plant"]
        return link_key(argv[1 + plant :] or PROBE_KUBECTL, plant)
    if argv[:1] == ["nodes"]:
        return nodes(argv[1:] or PROBE_KUBECTL)
    if argv[:1] == ["pin"] and len(argv) >= 2:
        return pin(argv[1], argv[2:])
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
