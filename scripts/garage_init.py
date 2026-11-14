#!/usr/bin/env python3
"""Prepares the Garage of `docker-compose.yml`, or of CI, for the dev stack and the test tiers.

Four steps, each skipped when it is already done, so a second run changes nothing:

1. a layout that gives the one node a role, since Garage refuses every request before it;
2. the dev key, which the relay, the test helpers and the belt default to;
3. the bucket, with that key allowed to read, write and own it;
4. the CORS rule the browser needs: it puts and fetches through presigned links itself
   (`crates/ambre-server/src/s3.rs`), and Garage allows no origin until a rule says so.

Steps 1 to 3 go through the admin API of Garage over HTTP, so the script needs no
`docker exec` and runs the same from the host, from CI and from the dev container
(`.devcontainer/`), which has no Docker. Step 4 goes through the S3 API, signed by
`curl --aws-sigv4`. The admin token and the dev key guard nothing; never reuse them in a
deployment.

Environment, each with the default of the local stack:

* `GARAGE_ADMIN`: the admin API, `http://localhost:3903`;
* `GARAGE_ADMIN_TOKEN`: the token of `garage.toml`;
* `S3_ENDPOINT`: the S3 API, `http://localhost:9000`;
* `S3_BUCKET`: the bucket, `ambre-dev`.

Run: `just garage-up`, which starts the container first.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Callable

# The dev key of the local stack. Garage accepts only a key id of `GK` and 24 hex digits.
KEY_ID = "GK1c7374dda8bf9b67e1ed2022"
SECRET = "b64c22af0c1f051c482b80d9cbde6751304fd0d084a565d2a5fb0342021899ca"
KEY_NAME = "ambre-dev"
# The token of the `[admin]` section of `garage.toml`.
ADMIN_TOKEN = "ambre-dev-admin-token"
ZONE = "dev"
CAPACITY = 1_000_000_000
REGION = "us-east-1"
WAIT_SECONDS = 30

CORS = (
    "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin>"
    "<AllowedMethod>GET</AllowedMethod><AllowedMethod>PUT</AllowedMethod>"
    "<AllowedHeader>*</AllowedHeader><ExposeHeader>ETag</ExposeHeader>"
    "<MaxAgeSeconds>3600</MaxAgeSeconds></CORSRule></CORSConfiguration>"
)


class GarageError(Exception):
    """A step the store refused, with the reason it gave."""


@dataclass(frozen=True)
class Reply:
    """One answer of the admin API: its status and its decoded JSON body, or `None`."""

    status: int
    body: object


# One call of the admin API: the method, the path with its query, and the JSON body.
Call = Callable[[str, str, object], Reply]


def http_call(admin: str, token: str) -> Call:
    """The admin API at `admin`, authenticated by `token`."""

    def call(method: str, path: str, body: object) -> Reply:
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(f"{admin}{path}", data=data, method=method)
        request.add_header("Authorization", f"Bearer {token}")
        if data is not None:
            request.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                return Reply(response.status, decode(response.read()))
        except urllib.error.HTTPError as error:
            return Reply(error.code, decode(error.read()))

    return call


def quote(value: str) -> str:
    """One value of a query string, escaped."""
    return urllib.parse.quote(value, safe="")


def decode(raw: bytes) -> object:
    """The JSON of a body, or `None` when it holds none."""
    try:
        return json.loads(raw)
    except ValueError:
        return None


def expect(reply: Reply, step: str) -> object:
    """The body of a successful reply, or the error that names the step and the reason."""
    if reply.status != 200:
        raise GarageError(f"{step}: HTTP {reply.status}: {reply.body}")
    return reply.body


def field(body: object, name: str) -> object:
    """One field of a JSON object, or the error that names it."""
    if not isinstance(body, dict) or name not in body:
        raise GarageError(f"the admin API answered without `{name}`: {body}")
    return body[name]


def roles_to_assign(status: object) -> list[dict[str, object]]:
    """The role each node lacks, or nothing when every node holds one already."""
    nodes = field(status, "nodes")
    if not isinstance(nodes, list):
        raise GarageError(f"`nodes` is not a list: {nodes}")
    return [
        {"id": field(node, "id"), "zone": ZONE, "capacity": CAPACITY, "tags": []}
        for node in nodes
        if field(node, "role") is None
    ]


def wait_for_admin(call: Call, sleep: Callable[[float], None] = time.sleep) -> object:
    """The cluster status, once the admin API answers it. The container may still be starting."""
    last = "no answer"
    for _ in range(WAIT_SECONDS):
        try:
            reply = call("GET", "/v2/GetClusterStatus", None)
            if reply.status == 200:
                return reply.body
            last = f"HTTP {reply.status}"
        except OSError as error:
            last = str(error)
        sleep(1)
    raise GarageError(f"the admin API did not answer in {WAIT_SECONDS} s: {last}")


def apply_layout(call: Call, status: object) -> None:
    """Step 1: a role for each node, applied as the next layout version."""
    roles = roles_to_assign(status)
    if not roles:
        return
    expect(call("POST", "/v2/UpdateClusterLayout", {"roles": roles}), "stage the layout")
    version = field(status, "layoutVersion")
    if not isinstance(version, int):
        raise GarageError(f"`layoutVersion` is not a number: {version}")
    expect(call("POST", "/v2/ApplyClusterLayout", {"version": version + 1}), "apply the layout")


def ensure_key(call: Call) -> None:
    """Step 2: the dev key, allowed to create a bucket."""
    reply = call("GET", f"/v2/GetKeyInfo?id={quote(KEY_ID)}", None)
    if reply.status != 200:
        body = {"accessKeyId": KEY_ID, "secretAccessKey": SECRET, "name": KEY_NAME}
        expect(call("POST", "/v2/ImportKey", body), "import the dev key")
    allow = {"allow": {"createBucket": True}}
    expect(call("POST", f"/v2/UpdateKey?id={quote(KEY_ID)}", allow), "allow the dev key")


def ensure_bucket(call: Call, bucket: str) -> None:
    """Step 3: the bucket, which the dev key reads, writes and owns."""
    reply = call("GET", f"/v2/GetBucketInfo?globalAlias={quote(bucket)}", None)
    if reply.status == 200:
        info = reply.body
    else:
        info = expect(call("POST", "/v2/CreateBucket", {"globalAlias": bucket}), "create the bucket")
    grant = {
        "bucketId": field(info, "id"),
        "accessKeyId": KEY_ID,
        "permissions": {"read": True, "write": True, "owner": True},
    }
    expect(call("POST", "/v2/AllowBucketKey", grant), "allow the dev key on the bucket")


def cors_command(endpoint: str, bucket: str) -> list[str]:
    """The `curl` line that puts the CORS rule of `bucket`, signed with the dev key."""
    md5 = base64.b64encode(hashlib.md5(CORS.encode(), usedforsecurity=False).digest()).decode()
    return [
        "curl", "-sSf", "-o", "/dev/null",
        "--aws-sigv4", f"aws:amz:{REGION}:s3",
        "--user", f"{KEY_ID}:{SECRET}",
        "-X", "PUT",
        "-H", f"Content-MD5: {md5}",
        "-H", "Content-Type: application/xml",
        "--data-binary", CORS,
        f"{endpoint}/{bucket}?cors",
    ]


def put_cors(endpoint: str, bucket: str, sleep: Callable[[float], None] = time.sleep) -> None:
    """Step 4: the CORS rule. The S3 API answers once the layout is live, a moment after step 1."""
    for _ in range(WAIT_SECONDS):
        if subprocess.run(cors_command(endpoint, bucket), check=False).returncode == 0:
            return
        sleep(1)
    raise GarageError(f"the store refused the CORS rule of {bucket}")


def main() -> int:
    admin = os.environ.get("GARAGE_ADMIN", "http://localhost:3903")
    token = os.environ.get("GARAGE_ADMIN_TOKEN", ADMIN_TOKEN)
    endpoint = os.environ.get("S3_ENDPOINT", "http://localhost:9000")
    bucket = os.environ.get("S3_BUCKET", "ambre-dev")
    call = http_call(admin, token)
    try:
        apply_layout(call, wait_for_admin(call))
        ensure_key(call)
        ensure_bucket(call, bucket)
        put_cors(endpoint, bucket)
    except GarageError as error:
        print(f"garage: {error}", file=sys.stderr)
        return 1
    print(f"garage: bucket {bucket} ready at {endpoint}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
