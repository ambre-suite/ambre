#!/usr/bin/env python3
"""What `garage_init.py` asks the admin API of Garage, and what it skips.

Each test answers the calls through a fake admin API and asserts the calls the
script made. The run against a real Garage is `just garage-up`, which CI runs
before the db tier.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import base64
import hashlib
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import garage_init  # noqa: E402  -- the module under test
from garage_init import GarageError, Reply  # noqa: E402

NODE = "3ddd59181389e7b6a10a1ebeb7506c62b3a0c90c65cc201432617c89ba172b7b"


class FakeAdmin:
    """An admin API that answers each path from `answers`, `200` and `{}` otherwise."""

    def __init__(self, answers: dict[str, Reply] | None = None) -> None:
        self.answers = answers or {}
        self.calls: list[tuple[str, str, object]] = []

    def __call__(self, method: str, path: str, body: object) -> Reply:
        self.calls.append((method, path, body))
        return self.answers.get(path, Reply(200, {"id": "bucket-id"}))

    def paths(self) -> list[str]:
        return [path for _, path, _ in self.calls]


def status(role: object, version: int = 0) -> dict[str, object]:
    return {"layoutVersion": version, "nodes": [{"id": NODE, "role": role}]}


class TheLayout(unittest.TestCase):
    def test_a_node_with_no_role_gets_one(self) -> None:
        roles = garage_init.roles_to_assign(status(None))
        self.assertEqual(roles, [{"id": NODE, "zone": "dev", "capacity": 1_000_000_000, "tags": []}])

    def test_a_node_with_a_role_keeps_it(self) -> None:
        self.assertEqual(garage_init.roles_to_assign(status({"zone": "dev"})), [])

    def test_the_next_version_is_applied(self) -> None:
        admin = FakeAdmin()
        garage_init.apply_layout(admin, status(None, version=3))
        self.assertEqual(admin.paths(), ["/v2/UpdateClusterLayout", "/v2/ApplyClusterLayout"])
        self.assertEqual(admin.calls[1][2], {"version": 4})

    def test_a_live_layout_is_left_alone(self) -> None:
        admin = FakeAdmin()
        garage_init.apply_layout(admin, status({"zone": "dev"}, version=1))
        self.assertEqual(admin.calls, [])

    def test_a_status_with_no_nodes_fails(self) -> None:
        with self.assertRaises(GarageError):
            garage_init.roles_to_assign({"layoutVersion": 0})


class TheKey(unittest.TestCase):
    def test_a_missing_key_is_imported_then_allowed(self) -> None:
        admin = FakeAdmin({f"/v2/GetKeyInfo?id={garage_init.KEY_ID}": Reply(404, {})})
        garage_init.ensure_key(admin)
        self.assertEqual(
            admin.paths(),
            [
                f"/v2/GetKeyInfo?id={garage_init.KEY_ID}",
                "/v2/ImportKey",
                f"/v2/UpdateKey?id={garage_init.KEY_ID}",
            ],
        )
        self.assertEqual(admin.calls[1][2]["secretAccessKey"], garage_init.SECRET)

    def test_a_present_key_is_allowed_alone(self) -> None:
        admin = FakeAdmin()
        garage_init.ensure_key(admin)
        self.assertNotIn("/v2/ImportKey", admin.paths())


class TheBucket(unittest.TestCase):
    def test_a_missing_bucket_is_created_then_granted(self) -> None:
        admin = FakeAdmin({
            "/v2/GetBucketInfo?globalAlias=ambre-dev": Reply(404, {}),
            "/v2/CreateBucket": Reply(200, {"id": "new-id"}),
        })
        garage_init.ensure_bucket(admin, "ambre-dev")
        self.assertEqual(admin.paths()[1:], ["/v2/CreateBucket", "/v2/AllowBucketKey"])
        self.assertEqual(admin.calls[2][2]["bucketId"], "new-id")
        self.assertEqual(
            admin.calls[2][2]["permissions"], {"read": True, "write": True, "owner": True}
        )

    def test_a_present_bucket_is_granted_alone(self) -> None:
        admin = FakeAdmin()
        garage_init.ensure_bucket(admin, "ambre-dev")
        self.assertNotIn("/v2/CreateBucket", admin.paths())

    def test_a_refused_creation_names_the_step(self) -> None:
        admin = FakeAdmin({
            "/v2/GetBucketInfo?globalAlias=ambre-dev": Reply(404, {}),
            "/v2/CreateBucket": Reply(500, {"message": "no quorum"}),
        })
        with self.assertRaisesRegex(GarageError, "create the bucket: HTTP 500"):
            garage_init.ensure_bucket(admin, "ambre-dev")


class TheWait(unittest.TestCase):
    def test_a_container_that_starts_late_is_waited_for(self) -> None:
        answers = iter([OSError("refused"), Reply(503, None), Reply(200, status(None))])

        def call(method: str, path: str, body: object) -> Reply:
            answer = next(answers)
            if isinstance(answer, OSError):
                raise answer
            return answer

        self.assertEqual(garage_init.wait_for_admin(call, sleep=lambda _: None), status(None))

    def test_a_silent_admin_api_fails(self) -> None:
        def call(method: str, path: str, body: object) -> Reply:
            raise OSError("refused")

        with self.assertRaisesRegex(GarageError, "did not answer"):
            garage_init.wait_for_admin(call, sleep=lambda _: None)


class TheCorsRule(unittest.TestCase):
    def test_the_rule_is_signed_and_carries_its_digest(self) -> None:
        line = garage_init.cors_command("http://garage:3900", "ambre-dev")
        md5 = base64.b64encode(hashlib.md5(garage_init.CORS.encode()).digest()).decode()
        self.assertIn(f"Content-MD5: {md5}", line)
        self.assertIn("aws:amz:us-east-1:s3", line)
        self.assertEqual(line[-1], "http://garage:3900/ambre-dev?cors")

    def test_the_rule_allows_a_put_and_a_get_from_any_origin(self) -> None:
        for part in ("<AllowedOrigin>*</AllowedOrigin>", "<AllowedMethod>PUT</AllowedMethod>",
                     "<AllowedMethod>GET</AllowedMethod>", "<ExposeHeader>ETag</ExposeHeader>"):
            self.assertIn(part, garage_init.CORS)


if __name__ == "__main__":
    unittest.main()
