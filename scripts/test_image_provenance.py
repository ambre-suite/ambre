#!/usr/bin/env python3
"""What `image_provenance.py` refuses, and what it lets through.

Each test plants one breach into the release workflow or the recipe of the tree
and asserts the fault, so a gate that reads nothing fails here. The
verification runs against a recording runner: no test calls `gh` or `docker`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent))

import image_provenance as ip  # noqa: E402  -- the module under test

RELEASE = yaml.safe_load((ip.WORKFLOWS / ip.RELEASE).read_text())
JUSTFILE = ip.JUSTFILE.read_text()
DIGEST = "sha256:" + "a" * 64
COMMIT = "b" * 40
TAG = "refs/tags/v0.1.0"
LIST = [(f"ghcr.io/ambre-suite/ambre/{t}@{DIGEST}", t) for t in ("relay", "sfu", "web", "init")]


def release() -> dict:
    return copy.deepcopy(RELEASE)


def attest_step(workflow: dict) -> dict:
    return next(s for s in workflow["jobs"]["images"]["steps"] if ip.uses(s).startswith(ip.ATTEST))


def faults(workflow: dict, justfile: str = JUSTFILE) -> list[str]:
    return ip.check_workflow("release.yml", workflow) + ip.check_install(workflow, justfile)


class Recorder:
    """A runner that records each command and fails the ones it is told to."""

    def __init__(self, failing: str = "") -> None:
        self.calls: list[list[str]] = []
        self.failing = failing

    def __call__(self, cmd: list[str]) -> int:
        self.calls.append(cmd)
        return 1 if self.failing and any(self.failing in part for part in cmd) else 0


class TheTree(unittest.TestCase):
    def test_the_workflows_and_the_recipe_of_the_tree_hold(self) -> None:
        self.assertEqual(ip.check(), 0)

    def test_the_release_holds_with_no_fault(self) -> None:
        self.assertEqual(faults(release()), [])


class Attestation(unittest.TestCase):
    def test_an_image_pushed_with_no_attestation_fails(self) -> None:
        w = release()
        w["jobs"]["images"]["steps"].remove(attest_step(w))
        self.assertTrue(any("no attestation of its digest" in f for f in faults(w)))

    def test_an_attestation_of_another_digest_fails(self) -> None:
        w = release()
        attest_step(w)["with"]["subject-digest"] = "${{ steps.other.outputs.digest }}"
        self.assertTrue(any("no attestation of its digest" in f for f in faults(w)))

    def test_an_attestation_kept_off_the_registry_fails(self) -> None:
        w = release()
        del attest_step(w)["with"]["push-to-registry"]
        self.assertTrue(any("stays off the registry" in f for f in faults(w)))

    def test_an_attestation_action_pinned_by_a_tag_fails(self) -> None:
        w = release()
        attest_step(w)["uses"] = "actions/attest-build-provenance@v4"
        self.assertTrue(any("names no commit" in f for f in faults(w)))

    def test_a_push_outside_build_push_action_fails(self) -> None:
        w = release()
        w["jobs"]["chart"]["steps"].append({"run": "docker push ghcr.io/x/y:z"})
        self.assertTrue(any("outside docker/build-push-action" in f for f in faults(w)))

    def test_a_pushing_step_with_no_id_fails(self) -> None:
        w = release()
        build = next(s for s in w["jobs"]["images"]["steps"] if ip.pushes(s))
        del build["id"]
        self.assertTrue(any("has no id" in f for f in faults(w)))


class SigningToken(unittest.TestCase):
    def test_a_pushing_job_with_no_signing_token_fails(self) -> None:
        w = release()
        del w["jobs"]["images"]["permissions"]["id-token"]
        self.assertTrue(any("cannot sign: no id-token" in f for f in faults(w)))

    def test_a_job_that_pushes_no_image_and_signs_fails(self) -> None:
        w = release()
        w["jobs"]["chart"]["permissions"]["id-token"] = "write"
        self.assertTrue(any("job chart: the job pushes no image and holds id-token" in f for f in faults(w)))

    def test_a_signing_grant_to_every_job_fails(self) -> None:
        w = release()
        w["permissions"] = "write-all"
        self.assertTrue(any("to every job" in f for f in faults(w)))


class InstallPath(unittest.TestCase):
    def test_a_release_with_no_chart_test_fails(self) -> None:
        w = release()
        del w["jobs"]["chart-test"]
        self.assertTrue(any("no chart test verifies" in f for f in faults(w)))

    def test_a_chart_pushed_before_the_chart_test_fails(self) -> None:
        w = release()
        w["jobs"]["chart"]["needs"] = ["client-digest", "images"]
        self.assertTrue(any("pushed before the chart test" in f for f in faults(w)))

    def test_a_job_step_that_loads_an_image_itself_fails(self) -> None:
        w = release()
        w["jobs"]["chart-test"]["steps"].append({"run": "docker pull ghcr.io/x/y@" + DIGEST})
        self.assertTrue(any("loads an image outside" in f for f in faults(w)))

    def test_a_recipe_that_loads_before_it_verifies_fails(self) -> None:
        body = "\n".join(ip.recipe(JUSTFILE, ip.RECIPE))
        load = next(line for line in body.splitlines() if "kind load" in line)
        planted = body.replace(load + "\n", "").replace("    set -euo pipefail\n", "    set -euo pipefail\n" + load + "\n")
        self.assertTrue(any("before" in f for f in faults(release(), JUSTFILE.replace(body, planted))))

    def test_a_recipe_that_pulls_by_itself_fails(self) -> None:
        body = "\n".join(ip.recipe(JUSTFILE, ip.RECIPE))
        planted = body + "\n    docker pull ghcr.io/x/y:latest"
        self.assertTrue(any("outside" in f for f in faults(release(), JUSTFILE.replace(body, planted))))

    def test_a_recipe_that_never_verifies_fails(self) -> None:
        body = "\n".join(ip.recipe(JUSTFILE, ip.RECIPE))
        planted = body.replace(ip.PULL, "scripts/other.py")
        self.assertTrue(any("never runs" in f for f in faults(release(), JUSTFILE.replace(body, planted))))


class Verification(unittest.TestCase):
    def test_the_call_pins_the_repository_the_workflow_the_ref_and_the_commit(self) -> None:
        cmd = ip.verify_command(LIST[0][0], TAG, COMMIT)
        self.assertEqual(cmd[:4], ["gh", "attestation", "verify", f"oci://{LIST[0][0]}"])
        pairs = dict(zip(cmd[4::2], cmd[5::2]))
        self.assertEqual(pairs["--repo"], "ambre-suite/ambre")
        self.assertEqual(pairs["--signer-workflow"], "ambre-suite/ambre/.github/workflows/release.yml")
        self.assertEqual(pairs["--source-ref"], TAG)
        self.assertEqual(pairs["--source-digest"], COMMIT)
        self.assertIn("--bundle-from-oci", cmd)
        self.assertIn("--deny-self-hosted-runners", cmd)

    def test_a_tag_with_no_digest_fails_before_any_call(self) -> None:
        with self.assertRaises(SystemExit):
            ip.images("ghcr.io/ambre-suite/ambre/relay:v0.1.0\n")

    def test_an_empty_list_fails(self) -> None:
        with self.assertRaises(SystemExit):
            ip.images("\n")

    def test_the_list_names_each_image(self) -> None:
        text = "".join(ref + "\n" for ref, _ in LIST)
        self.assertEqual(ip.images(text), LIST)

    def test_one_refusal_pulls_no_image(self) -> None:
        run = Recorder(failing="oci://ghcr.io/ambre-suite/ambre/web@")
        self.assertEqual(ip.pull(LIST, TAG, COMMIT, run), 1)
        self.assertFalse([c for c in run.calls if c[0] == "docker"])
        self.assertEqual(len(run.calls), len(LIST))

    def test_every_image_verifies_before_the_first_pull(self) -> None:
        run = Recorder()
        self.assertEqual(ip.pull(LIST, TAG, COMMIT, run), 0)
        first_pull = next(i for i, c in enumerate(run.calls) if c[0] == "docker")
        self.assertEqual([c[0] for c in run.calls[:first_pull]], ["gh"] * len(LIST))
        self.assertIn(["docker", "pull", LIST[0][0]], run.calls)
        self.assertIn(["docker", "tag", LIST[0][0], "ambre-relay:local"], run.calls)

    def test_a_ref_other_than_a_release_tag_fails_before_any_call(self) -> None:
        run = Recorder()
        with self.assertRaises(SystemExit):
            ip.pull(LIST, "refs/heads/main", COMMIT, run)
        self.assertEqual(run.calls, [])

    def test_a_short_commit_fails_before_any_call(self) -> None:
        run = Recorder()
        with self.assertRaises(SystemExit):
            ip.verify(LIST, TAG, "b" * 7, run)
        self.assertEqual(run.calls, [])

    def test_a_failed_verification_exits_non_zero_from_the_command_line(self) -> None:
        import tempfile

        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(LIST[0][0] + "\n")
        run = Recorder(failing="gh")
        self.assertEqual(ip.main(["verify", f.name, "--source-ref", TAG, "--source-digest", COMMIT], run), 1)
        Path(f.name).unlink()


if __name__ == "__main__":
    unittest.main()
