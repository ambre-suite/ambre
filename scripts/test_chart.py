"""Tests of `scripts/chart.py`: each fault the gate names, on objects built here.

The renders of the chart itself run in `just chart-check`, which needs Helm.
These tests need PyYAML alone.
"""

from __future__ import annotations

import copy
import unittest

import chart

DIGEST = "sha256:" + "a" * 64
STORE = "postgres:16.15-alpine@" + DIGEST


def container(name: str, image: str, **extra: object) -> dict:
    return {
        "name": name,
        "image": image,
        "securityContext": {
            "runAsNonRoot": True,
            "allowPrivilegeEscalation": False,
            "readOnlyRootFilesystem": True,
            "capabilities": {"drop": ["ALL"]},
        },
        **extra,
    }


def workload(comp: str, image: str, kind: str = "Deployment", **pod: object) -> dict:
    labels = {"app.kubernetes.io/component": comp}
    return {
        "kind": kind,
        "metadata": {"name": f"ambre-{comp}", "labels": labels},
        "spec": {
            "replicas": 1,
            "template": {
                "metadata": {"labels": labels},
                "spec": {
                    "automountServiceAccountToken": False,
                    "securityContext": {
                        "runAsNonRoot": True,
                        "runAsUser": 999,
                        "seccompProfile": {"type": "RuntimeDefault"},
                    },
                    "containers": [container(comp, image)],
                    **pod,
                },
            },
        },
    }


def sfu() -> dict:
    obj = workload("sfu", "ghcr.io/x/sfu:1.0.0", hostNetwork=True)
    obj["spec"]["template"]["spec"]["containers"][0]["ports"] = [
        {"name": "signal", "containerPort": 3480},
        {"name": "media", "containerPort": 3481, "protocol": "UDP"},
    ]
    return obj


def service(comp: str, ports: list[dict], kind: str = "ClusterIP") -> dict:
    return {
        "kind": "Service",
        "metadata": {"name": f"ambre-{comp}"},
        "spec": {"type": kind, "selector": {"app.kubernetes.io/component": comp}, "ports": ports},
    }


def stack() -> list[dict]:
    return [
        workload("relay", "ghcr.io/x/relay:1.0.0"),
        workload("postgres", STORE, kind="StatefulSet"),
        sfu(),
        service("sfu", [{"name": "signal", "port": 3480, "targetPort": "signal"}]),
    ]


COMPOSE = {"postgres": STORE}


class ObjectsTest(unittest.TestCase):
    def faults(self, objects: list[dict], release: bool = False) -> list[str]:
        return chart.check_objects(objects, COMPOSE, release)

    def test_a_sound_stack_holds(self) -> None:
        self.assertEqual(self.faults(stack()), [])

    def test_a_container_that_keeps_a_capability_fails(self) -> None:
        objects = stack()
        sc = objects[0]["spec"]["template"]["spec"]["containers"][0]["securityContext"]
        sc["capabilities"] = {"drop": ["ALL"], "add": ["NET_ADMIN"]}
        self.assertIn("keeps a capability", " ".join(self.faults(objects)))

    def test_a_writable_root_and_a_root_user_fail(self) -> None:
        objects = stack()
        objects[0]["spec"]["template"]["spec"]["containers"][0]["securityContext"].update(
            readOnlyRootFilesystem=False, runAsUser=0
        )
        text = " ".join(self.faults(objects))
        self.assertIn("writes its root filesystem", text)
        self.assertIn("may run as root", text)

    def test_a_service_account_token_fails(self) -> None:
        objects = stack()
        del objects[0]["spec"]["template"]["spec"]["automountServiceAccountToken"]
        self.assertIn("service account token", " ".join(self.faults(objects)))

    def test_a_secret_spelled_out_fails_and_a_reference_holds(self) -> None:
        objects = stack()
        c = objects[0]["spec"]["template"]["spec"]["containers"][0]
        c["env"] = [{"name": "DATABASE_URL", "value": "postgres://ambre:$(POSTGRES_PASSWORD)@db/ambre"}]
        self.assertEqual(self.faults(objects), [])
        c["env"] = [{"name": "JWT_SIGNING_SEED", "value": "00ff"}]
        self.assertIn("spells a secret out", " ".join(self.faults(objects)))

    def test_a_secret_the_chart_writes_fails(self) -> None:
        objects = [*stack(), {"kind": "Secret", "metadata": {"name": "ambre"}}]
        self.assertIn("writes the Secret", " ".join(self.faults(objects)))

    def test_an_sfu_off_the_network_of_its_node_fails(self) -> None:
        objects = stack()
        objects[2]["spec"]["template"]["spec"]["hostNetwork"] = False
        self.assertIn("off the network of its node", " ".join(self.faults(objects)))

    def test_two_sfu_pods_fail(self) -> None:
        objects = stack()
        objects[2]["spec"]["replicas"] = 2
        self.assertIn("not one pod", " ".join(self.faults(objects)))

    def test_a_load_balancer_before_the_sfu_fails(self) -> None:
        objects = [*stack(), service("sfu", [{"port": 3481, "protocol": "UDP"}], "LoadBalancer")]
        text = " ".join(self.faults(objects))
        self.assertIn("LoadBalancer Service fronts the SFU", text)
        self.assertIn("carries the media port", text)

    def test_a_store_image_off_the_compose_file_fails(self) -> None:
        objects = stack()
        objects[1]["spec"]["template"]["spec"]["containers"][0]["image"] = "postgres:16.15-alpine"
        self.assertIn("names no digest", " ".join(self.faults(objects)))
        objects[1]["spec"]["template"]["spec"]["containers"][0]["image"] = "postgres:17.1-alpine@" + DIGEST
        self.assertIn("not the image of the compose file", " ".join(self.faults(objects)))

    def test_a_release_names_each_image_of_ambre_by_digest(self) -> None:
        self.assertIn("relay/relay: ghcr.io/x/relay:1.0.0 names no digest", self.faults(stack(), True))
        objects = copy.deepcopy(stack())
        for i in (0, 2):
            name = objects[i]["metadata"]["labels"]["app.kubernetes.io/component"]
            objects[i]["spec"]["template"]["spec"]["containers"][0]["image"] = f"ghcr.io/x/{name}@{DIGEST}"
        self.assertEqual(self.faults(objects, True), [])


def labelled(comp: str) -> dict:
    obj = workload(comp, STORE)
    obj["spec"]["template"]["metadata"]["labels"] = {
        "app.kubernetes.io/name": "ambre",
        "app.kubernetes.io/instance": "ambre",
        "app.kubernetes.io/component": comp,
    }
    return obj


def peer(comp: str) -> dict:
    return {"podSelector": {"matchLabels": labelled(comp)["spec"]["template"]["metadata"]["labels"]}}


def policy(selector: dict, types: list[str], **rules: object) -> dict:
    return {"kind": "NetworkPolicy", "spec": {"podSelector": selector, "policyTypes": types, **rules}}


def network() -> list[dict]:
    """A relay and its database: everything denied, then the one flow between them."""
    release = {"matchLabels": {"app.kubernetes.io/name": "ambre", "app.kubernetes.io/instance": "ambre"}}
    return [
        labelled("relay"),
        labelled("postgres"),
        sfu(),
        policy(release, ["Ingress", "Egress"]),
        policy(peer("relay")["podSelector"], ["Egress"], egress=[{"to": [peer("postgres")], "ports": [{"port": 5432}]}]),
        policy(
            peer("postgres")["podSelector"],
            ["Ingress"],
            ingress=[{"from": [peer("relay")], "ports": [{"protocol": "TCP", "port": 5432}]}],
        ),
    ]


TABLE = {("relay", "egress", "postgres", "TCP/5432"), ("postgres", "ingress", "relay", "TCP/5432")}


class NetworkTest(unittest.TestCase):
    def test_a_stack_that_admits_its_table_alone_holds(self) -> None:
        self.assertEqual(chart.check_network(network(), TABLE), [])

    def test_a_pod_no_policy_isolates_fails(self) -> None:
        objects = [o for o in network() if o["kind"] != "NetworkPolicy" or "Egress" not in o["spec"]["policyTypes"]]
        self.assertIn("postgres: no NetworkPolicy isolates its egress", chart.check_network(objects, TABLE))

    def test_a_database_open_to_every_pod_fails(self) -> None:
        objects = network()
        objects[5]["spec"]["ingress"][0]["from"] = [{"podSelector": {}}]
        faults = chart.check_network(objects, TABLE)
        self.assertIn("which the flow table does not name", " ".join(faults))
        self.assertIn("postgres: its ingress refuses relay on TCP/5432, which the flow table names", faults)

    def test_a_rule_with_no_peer_admits_anywhere(self) -> None:
        objects = network()
        del objects[5]["spec"]["ingress"][0]["from"]
        self.assertIn(
            "postgres: its ingress admits anywhere on TCP/5432, which the flow table does not name",
            chart.check_network(objects, TABLE),
        )

    def test_an_address_block_with_the_cluster_in_it_is_no_public_peer(self) -> None:
        objects = network()
        objects[4]["spec"]["egress"].append(
            {"to": [{"ipBlock": {"cidr": "0.0.0.0/0", "except": ["169.254.0.0/16"]}}], "ports": [{"port": 443}]}
        )
        self.assertIn(
            "relay: its egress admits ipBlock 0.0.0.0/0 on TCP/443, which the flow table does not name",
            chart.check_network(objects, TABLE),
        )

    def test_the_sfu_on_the_network_of_its_node_is_out_of_the_table(self) -> None:
        self.assertNotIn("sfu", " ".join(chart.check_network(network(), TABLE)))

    def test_the_table_of_a_render_follows_its_stores(self) -> None:
        stores = {"web", "relay", "postgres", "redis", "garage"}
        inside = chart.flows_table(stores, acme=True, external=False, nodes=chart.NODE_CIDRS)
        outside = chart.flows_table({"web", "relay"}, acme=False, external=True, nodes=chart.NODE_CIDRS)
        self.assertIn(("postgres", "ingress", "relay", "TCP/5432"), inside)
        self.assertIn(("web", "egress", "public", "TCP/443"), inside)
        self.assertNotIn(("web", "egress", "public", "TCP/443"), outside)
        self.assertIn(("relay", "egress", "ipBlock 192.0.2.10/32", "TCP/5432"), outside)
        self.assertFalse({f for f in outside if "postgres" in f})


def signalling(rule: dict) -> list[dict]:
    """A web server whose policy opens the signalling port of the SFU under `rule`."""
    web = {"matchLabels": labelled("web")["spec"]["template"]["metadata"]["labels"]}
    return [
        labelled("web"),
        sfu(),
        policy(web, ["Ingress", "Egress"], egress=[{**rule, "ports": [{"protocol": "TCP", "port": 3480}]}]),
    ]


SIGNAL = {("web", "egress", "ipBlock 192.0.2.0/24", "TCP/3480")}


class SignallingTest(unittest.TestCase):
    """`SRV-410`: the web server reaches the signalling of the SFU on the nodes alone."""

    def test_the_blocks_of_the_nodes_alone_hold(self) -> None:
        objects = signalling({"to": [{"ipBlock": {"cidr": "192.0.2.0/24"}}]})
        self.assertEqual(chart.check_network(objects, SIGNAL), [])

    def test_the_port_open_on_every_address_fails(self) -> None:
        faults = chart.check_network(signalling({}), SIGNAL)
        self.assertIn("web: its egress admits anywhere on TCP/3480, which the flow table does not name", faults)
        self.assertIn("web: its egress refuses ipBlock 192.0.2.0/24 on TCP/3480, which the flow table names", faults)

    def test_the_port_open_on_the_internet_fails(self) -> None:
        public = [{"ipBlock": {"cidr": cidr, "except": sorted(ex)}} for cidr, ex in chart.PRIVATE.items()]
        faults = chart.check_network(signalling({"to": public}), SIGNAL)
        self.assertIn("web: its egress admits public on TCP/3480, which the flow table does not name", faults)

    def test_a_block_wider_than_the_nodes_fails(self) -> None:
        faults = chart.check_network(signalling({"to": [{"ipBlock": {"cidr": "192.0.0.0/8"}}]}), SIGNAL)
        self.assertIn("web: its egress admits ipBlock 192.0.0.0/8 on TCP/3480, which the flow table does not name", faults)

    def test_the_table_names_each_block_of_the_nodes(self) -> None:
        table = chart.flows_table({"web", "relay"}, acme=False, external=False, nodes=chart.NODE_CIDRS)
        ports = {f for f in table if f[3] == "TCP/3480"}
        self.assertEqual(ports, {("web", "egress", f"ipBlock {c}", "TCP/3480") for c in chart.NODE_CIDRS})

    def test_the_refusals_name_no_block_and_each_whole_family(self) -> None:
        self.assertIn([], chart.NODE_REFUSALS)
        self.assertIn(["0.0.0.0/0"], chart.NODE_REFUSALS)
        self.assertIn(["::/0"], chart.NODE_REFUSALS)


class NodesTest(unittest.TestCase):
    def test_a_node_inside_the_blocks_holds(self) -> None:
        self.assertEqual(chart.nodes_outside(["172.16.0.0/12"], ["172.18.0.2"]), [])

    def test_a_node_outside_the_blocks_fails(self) -> None:
        self.assertEqual(
            chart.nodes_outside(["172.16.0.0/12"], ["192.168.49.2"]),
            ["node address 192.168.49.2 lies outside networkPolicy.nodeCidrs ['172.16.0.0/12']"],
        )

    def test_a_cluster_with_no_node_address_fails(self) -> None:
        self.assertEqual(chart.nodes_outside(["172.16.0.0/12"], []), ["the cluster lists no address of a node"])

    def test_the_test_values_name_a_block_of_nodes(self) -> None:
        values = chart.yaml.safe_load(chart.TEST_VALUES.read_text())
        self.assertTrue(values["networkPolicy"]["nodeCidrs"])


class ProbeTest(unittest.TestCase):
    TARGETS = {"ambre-postgres:5432": False, "ambre-web:443": True}

    def test_a_refusal_beside_an_open_entry_holds(self) -> None:
        out = "ambre-postgres 5432 0\nambre-web 443 1\n"
        self.assertEqual(chart.probe_faults("p", out, self.TARGETS), [])

    def test_a_database_the_probe_reaches_fails(self) -> None:
        out = "ambre-postgres 5432 1\nambre-web 443 1\n"
        self.assertEqual(
            chart.probe_faults("p", out, self.TARGETS),
            ["p: reached ambre-postgres:5432, which the flow table does not name"],
        )

    def test_a_refusal_with_a_closed_entry_proves_nothing(self) -> None:
        out = "ambre-postgres 5432 0\nambre-web 443 0\n"
        self.assertIn("so a refusal proves nothing", " ".join(chart.probe_faults("p", out, self.TARGETS)))

    def test_a_target_with_no_answer_fails(self) -> None:
        self.assertIn("no answer for ambre-postgres:5432", " ".join(chart.probe_faults("p", "", self.TARGETS)))


RELEASE = {
    "jobs": {
        "images": {
            "strategy": {"matrix": {"target": ["relay", "sfu", "web", "init"]}},
            "steps": [{"uses": "docker/build-push-action@abc", "with": {"push": True}}],
        },
        "chart": {
            "needs": ["client-digest", "images"],
            "steps": [
                {"run": "python3 scripts/chart.py pin 1.0.0 relay=x"},
                {"run": "python3 scripts/chart.py check --release"},
                {"run": "helm push target/chart/ambre-1.0.0.tgz oci://ghcr.io/x/charts"},
            ],
        },
    }
}


class ReleaseTest(unittest.TestCase):
    def test_the_release_workflow_holds(self) -> None:
        self.assertEqual(chart.check_release(RELEASE), [])

    def test_a_release_without_the_sfu_image_fails(self) -> None:
        wf = copy.deepcopy(RELEASE)
        wf["jobs"]["images"]["strategy"]["matrix"]["target"] = ["relay", "web"]
        self.assertEqual(chart.check_release(wf), ["the release builds no sfu image"])

    def test_a_release_that_pushes_nothing_fails(self) -> None:
        wf = copy.deepcopy(RELEASE)
        wf["jobs"]["images"]["steps"][0]["with"]["push"] = False
        self.assertEqual(chart.check_release(wf), ["the release pushes no image"])

    def test_a_chart_pushed_unpinned_fails(self) -> None:
        wf = copy.deepcopy(RELEASE)
        del wf["jobs"]["chart"]["steps"][0]
        self.assertEqual(chart.check_release(wf), ["the release pins no digest into the chart"])

    def test_the_workflow_of_the_repository_holds(self) -> None:
        workflow = chart.yaml.safe_load(chart.RELEASE.read_text())
        self.assertEqual(chart.check_release(workflow), [])


VALUES = """images:
  registry: ghcr.io/x
  relay:
    repository: relay
    digest: ""
  sfu:
    repository: sfu
    digest: ""
relay:
  digest: keep
"""


class PinTest(unittest.TestCase):
    def test_each_digest_lands_under_its_image_alone(self) -> None:
        out = chart.pin_values(VALUES, {"relay": "sha256:1", "sfu": "sha256:2"})
        self.assertIn('  relay:\n    repository: relay\n    digest: "sha256:1"\n', out)
        self.assertIn('  sfu:\n    repository: sfu\n    digest: "sha256:2"\n', out)
        self.assertIn("relay:\n  digest: keep\n", out)

    def test_an_image_without_a_digest_field_fails(self) -> None:
        with self.assertRaises(SystemExit):
            chart.pin_values(VALUES, {"web": "sha256:3"})

    def test_the_chart_takes_the_version_of_the_release(self) -> None:
        out = chart.pin_chart('name: ambre\nversion: 0.0.0\nappVersion: "0.0.0"\n', "1.2.3")
        self.assertEqual(out, 'name: ambre\nversion: 1.2.3\nappVersion: "1.2.3"\n')

    def test_the_values_of_the_repository_pin_every_image(self) -> None:
        digests = {name: DIGEST for name in chart.AMBRE_IMAGES}
        out = chart.pin_values((chart.CHART / "values.yaml").read_text(), digests)
        self.assertEqual(out.count(f'digest: "{DIGEST}"'), len(chart.AMBRE_IMAGES))


if __name__ == "__main__":
    unittest.main()
