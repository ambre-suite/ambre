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
