#!/usr/bin/env python3
"""What `docker_up.py` decides before it starts the Docker daemon.

The decisions are pure: which host services shadow the stack, and how the daemon
gets the mirror. The run against a real daemon is `just docker-up`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import docker_up  # noqa: E402  -- the module under test


class TheHostServices(unittest.TestCase):
    def test_a_free_stack_names_nothing(self) -> None:
        self.assertEqual(docker_up.held_ports(docker_up.SERVICE_PORTS, lambda port: False), [])

    def test_a_host_redis_is_named(self) -> None:
        held = docker_up.held_ports(docker_up.SERVICE_PORTS, lambda port: port == 6379)
        self.assertEqual(held, ["redis on 6379"])

    def test_both_host_services_are_named_in_port_order(self) -> None:
        held = docker_up.held_ports(docker_up.SERVICE_PORTS, lambda port: True)
        self.assertEqual(held, ["postgres on 5432", "redis on 6379"])


class TheMirror(unittest.TestCase):
    def test_a_writable_absent_config_gets_the_mirror(self) -> None:
        self.assertEqual(docker_up.dockerd_command(False, True), (["dockerd"], True))

    def test_an_existing_config_is_never_rewritten(self) -> None:
        command, write = docker_up.dockerd_command(True, True)
        self.assertFalse(write)
        self.assertIn(f"--registry-mirror={docker_up.MIRROR}", command)

    def test_a_read_only_etc_takes_the_mirror_on_the_command_line(self) -> None:
        command, write = docker_up.dockerd_command(False, False)
        self.assertFalse(write)
        self.assertIn(f"--registry-mirror={docker_up.MIRROR}", command)

    def test_a_config_that_holds_a_mirror_is_read_as_such(self) -> None:
        self.assertTrue(docker_up.config_holds_mirror('{"registry-mirrors": ["https://m"]}'))
        self.assertFalse(docker_up.config_holds_mirror('{"debug": true}'))
        self.assertFalse(docker_up.config_holds_mirror("not json"))
        self.assertFalse(docker_up.config_holds_mirror("[]"))


if __name__ == "__main__":
    unittest.main()
