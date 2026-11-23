#!/usr/bin/env python3
"""Starts the Docker daemon when it is down, so the backing services can run under Docker.

Postgres, Redis and Garage run from the images `docker-compose.yml` pins, never on the
host (`ai/ambre-testing.md`). A cloud session has Docker installed with its daemon
stopped, and Docker Hub refuses its anonymous pulls (`429 Too Many Requests`). This
script is the one place that knows both:

1. a daemon that answers is left alone, so a second run changes nothing;
2. a host `postgres` or `redis-server` that holds 5432 or 6379 stops the run: the
   containers would fail to bind, and the tests would reach the host service;
3. the daemon is given Google's mirror of Docker Hub, through `/etc/docker/daemon.json`
   when the session may write it and holds none, on its command line otherwise;
4. it waits for the daemon to answer.

Where no `dockerd` can be started (a laptop with Docker Desktop, no root), it says to
start Docker and exits non-zero. It never falls back to a host service.

Run: `just docker-up`, which then starts the services.
"""

from __future__ import annotations

import json
import os
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

MIRROR = "https://mirror.gcr.io"
DAEMON_CONFIG = Path("/etc/docker/daemon.json")
DAEMON_LOG = Path("/tmp/dockerd.log")
WAIT_SECONDS = 60

# The host ports the compose services publish, each with the host service that
# would shadow it.
SERVICE_PORTS = {5432: "postgres", 6379: "redis"}

NEVER_ON_HOST = (
    "Never run postgres or redis on the host instead: the services run from the images "
    "docker-compose.yml pins (ai/ambre-testing.md)."
)


def daemon_answers() -> bool:
    """`docker info` succeeds: the daemon is up and this user may talk to it."""
    if shutil.which("docker") is None:
        return False
    return subprocess.run(["docker", "info"], capture_output=True, check=False).returncode == 0


def held_ports(ports: dict[int, str], is_open) -> list[str]:
    """The names of the host services whose port something already holds."""
    return [f"{name} on {port}" for port, name in sorted(ports.items()) if is_open(port)]


def port_open(port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.settimeout(0.5)
        return probe.connect_ex(("127.0.0.1", port)) == 0


def dockerd_command(config_exists: bool, config_writable: bool) -> tuple[list[str], bool]:
    """The command line of the daemon, and whether to write the mirror into its config first.

    An existing config is never rewritten: it may hold settings of its owner, and a
    mirror given on the command line beside a `registry-mirrors` key in the file makes
    the daemon refuse to start, so the flag goes only where the file holds no mirror.
    """
    if not config_exists and config_writable:
        return ["dockerd"], True
    return ["dockerd", f"--registry-mirror={MIRROR}"], False


def config_holds_mirror(text: str) -> bool:
    try:
        return bool(json.loads(text).get("registry-mirrors"))
    except (ValueError, AttributeError):
        return False


def start_daemon() -> int:
    if shutil.which("dockerd") is None or os.geteuid() != 0:
        print(
            "error: the Docker daemon is not running, and this shell cannot start it. "
            "Start Docker (Docker Desktop, or `sudo systemctl start docker`), then run "
            "`just docker-up` again. " + NEVER_ON_HOST,
            file=sys.stderr,
        )
        return 1
    exists = DAEMON_CONFIG.exists()
    if exists and config_holds_mirror(DAEMON_CONFIG.read_text()):
        command, write = ["dockerd"], False
    else:
        writable = os.access(DAEMON_CONFIG.parent if DAEMON_CONFIG.parent.exists() else "/etc", os.W_OK)
        command, write = dockerd_command(exists, writable)
    if write:
        DAEMON_CONFIG.parent.mkdir(parents=True, exist_ok=True)
        DAEMON_CONFIG.write_text(json.dumps({"registry-mirrors": [MIRROR]}) + "\n")
    with DAEMON_LOG.open("ab") as log:
        subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True)
    deadline = time.monotonic() + WAIT_SECONDS
    while time.monotonic() < deadline:
        if daemon_answers():
            print(f"docker daemon up (log: {DAEMON_LOG})")
            return 0
        time.sleep(1)
    print(f"error: the Docker daemon did not answer within {WAIT_SECONDS} s; see {DAEMON_LOG}. "
          + NEVER_ON_HOST, file=sys.stderr)
    return 1


def main() -> int:
    if daemon_answers():
        return 0
    held = held_ports(SERVICE_PORTS, port_open)
    if held:
        print(
            f"error: a host service holds a port of the stack ({', '.join(held)}). Stop it "
            "(`pkill redis-server`, `pkill postgres`), then run `just docker-up` again. "
            + NEVER_ON_HOST,
            file=sys.stderr,
        )
        return 1
    return start_daemon()


if __name__ == "__main__":
    sys.exit(main())
