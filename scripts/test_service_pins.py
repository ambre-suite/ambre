#!/usr/bin/env python3
"""What `service_pins.py` refuses, and what it lets through.

Each test plants one compose file and one workflow through `breaches` and
asserts the verdict. Reading the two files is left to the run on the tree.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import service_pins  # noqa: E402  -- the module under test

DIGEST = "@sha256:" + "a" * 64
POSTGRES = "postgres:16.15-alpine" + DIGEST
REDIS = "redis:8.10.3-alpine" + DIGEST


def compose(postgres: str = POSTGRES, redis: str = REDIS) -> str:
    return (
        "services:\n"
        "  postgres:\n"
        f"    image: {postgres}\n"
        "    environment:\n"
        "      POSTGRES_USER: ambre\n"
        "  redis:\n"
        f"    image: {redis}\n"
        "\n"
        "volumes:\n"
        "  pgdata:\n"
    )


def workflow(postgres: str = POSTGRES, redis: str = REDIS, name: str = "redis") -> str:
    return (
        "jobs:\n"
        "  test-db:\n"
        "    runs-on: ubuntu-latest\n"
        "    services:\n"
        "      postgres:\n"
        f"        image: {postgres}   # pinned with docker-compose.yml\n"
        "        ports:\n"
        "          - 5432:5432\n"
        f"      {name}:\n"
        f"        image: {redis}\n"
        "    steps:\n"
        "      - uses: actions/checkout@v5\n"
    )


def pin(value: str) -> list[str]:
    return service_pins.pin_breaches(service_pins.Image("x.yml", 1, ("services", "s"), value))


def refused(compose_text: str = compose(), ci_text: str = workflow()) -> list[str]:
    return service_pins.breaches(compose_text, ci_text)[0]


class APinnedImage(unittest.TestCase):
    def test_a_version_and_a_digest_pass(self) -> None:
        self.assertEqual(refused(), [])

    def test_every_image_is_read(self) -> None:
        self.assertEqual(service_pins.breaches(compose(), workflow())[1], 4)

    def test_a_v_prefix_passes(self) -> None:
        self.assertEqual(pin("dxflrs/garage:v2.3.0" + DIGEST), [])

    def test_a_registry_with_a_port_reads_its_tag(self) -> None:
        image = "registry.local:5000/redis:8.10.3-alpine" + DIGEST
        self.assertEqual(refused(compose(postgres=image), workflow(postgres=image)), [])


class AMissingDigest(unittest.TestCase):
    def test_a_tag_alone_fails(self) -> None:
        found = refused(compose(redis="redis:8.10.3-alpine"), workflow(redis="redis:8.10.3-alpine"))
        self.assertEqual(len(found), 2)
        self.assertTrue(all("names no `@sha256:` digest" in f for f in found))

    def test_a_short_digest_fails(self) -> None:
        image = "redis:8.10.3-alpine@sha256:abc"
        self.assertTrue(refused(compose(redis=image), workflow(redis=image)))


class AFloatingTag(unittest.TestCase):
    def test_latest_fails(self) -> None:
        self.assertIn("floats", pin("dpage/pgadmin4:latest" + DIGEST)[0])

    def test_a_major_alone_fails(self) -> None:
        image = "redis:8-alpine" + DIGEST
        self.assertIn("floats", refused(compose(redis=image), workflow(redis=image))[0])

    def test_two_components_fail(self) -> None:
        self.assertIn("floats", pin("redis:8.10-alpine" + DIGEST)[0])

    def test_a_postgres_major_alone_fails(self) -> None:
        self.assertIn("of 2 components", pin("postgres:16-alpine" + DIGEST)[0])

    def test_a_postgres_release_has_two_components(self) -> None:
        self.assertEqual(pin("postgres:16.15-alpine" + DIGEST), [])
        self.assertIn("floats", pin("postgres:16.15.0-alpine" + DIGEST)[0])

    def test_no_tag_fails(self) -> None:
        image = "postgres" + DIGEST
        self.assertIn("names no tag", refused(compose(postgres=image), workflow(postgres=image))[0])


class ACiService(unittest.TestCase):
    def test_another_version_than_compose_fails(self) -> None:
        image = "redis:8.10.2-alpine" + DIGEST
        found = refused(compose(), workflow(redis=image))
        self.assertEqual(len(found), 1)
        self.assertIn("pins `" + REDIS + "`", found[0])

    def test_another_digest_than_compose_fails(self) -> None:
        image = "redis:8.10.3-alpine@sha256:" + "b" * 64
        self.assertTrue(refused(compose(), workflow(redis=image)))

    def test_a_service_compose_lacks_fails(self) -> None:
        found = refused(compose(), workflow(name="valkey"))
        self.assertIn("which docker-compose.yml lacks", found[0])

    def test_a_trailing_comment_is_not_the_image(self) -> None:
        self.assertEqual(refused(compose(), workflow(postgres=POSTGRES + "  # note")), [])


class TheReader(unittest.TestCase):
    def test_a_compose_file_with_no_image_fails(self) -> None:
        self.assertIn("no service image read", refused("services: {}\n", "jobs:\n")[0])

    def test_an_image_outside_services_is_not_a_ci_service(self) -> None:
        text = "jobs:\n  build:\n    container:\n      image: node:latest\n"
        self.assertEqual(refused(compose(), text), [])


def browsers(*steps: str) -> str:
    """A workflow whose one job runs `steps`, each a block of step lines."""
    return workflow() + "".join(steps)


CHROME = (
    "      - name: Install Chrome\n"
    "        if: matrix.engine == 'chrome'\n"
    "        uses: browser-actions/setup-chrome@abc # v2.2.0\n"
    "        with:\n"
    "{version}"
    "          install-chromedriver: true\n"
)
FIREFOX = (
    "      - name: Install Firefox\n"
    "        uses: browser-actions/setup-firefox@abc # v1.7.2\n"
    "{with_}"
)
GECKO = (
    "      - uses: browser-actions/setup-geckodriver@abc # latest\n"
    "        with:\n"
    "          geckodriver-version: {version}\n"
    "          token: x\n"
)


class ABrowser(unittest.TestCase):
    def test_exact_versions_pass(self) -> None:
        text = browsers(
            CHROME.format(version="          chrome-version: 157.0.8103.78\n"),
            FIREFOX.format(with_="        with:\n          firefox-version: '160.0'\n"),
            GECKO.format(version="0.37.1"),
        )
        self.assertEqual(service_pins.browser_breaches(text), ([], 3))

    def test_a_firefox_fix_release_passes(self) -> None:
        text = browsers(FIREFOX.format(with_="        with:\n          firefox-version: 160.0.1\n"))
        self.assertEqual(service_pins.browser_breaches(text)[0], [])

    def test_a_browser_with_no_version_fails(self) -> None:
        errors = refused(ci_text=browsers(FIREFOX.format(with_="")))
        self.assertEqual(len(errors), 1)
        self.assertIn("names no `firefox-version`", errors[0])

    def test_a_channel_fails(self) -> None:
        for version in ("latest", "latest-esr", "160", "stable"):
            with self.subTest(version=version):
                text = browsers(
                    FIREFOX.format(with_=f"        with:\n          firefox-version: {version}\n")
                )
                self.assertIn("no exact version", service_pins.browser_breaches(text)[0][0])

    def test_a_chrome_major_alone_fails(self) -> None:
        text = browsers(CHROME.format(version="          chrome-version: 157\n"))
        self.assertIn("no exact version", service_pins.browser_breaches(text)[0][0])

    def test_a_chrome_with_no_version_fails(self) -> None:
        text = browsers(CHROME.format(version=""))
        self.assertIn("names no `chrome-version`", service_pins.browser_breaches(text)[0][0])

    def test_the_next_step_version_is_not_read(self) -> None:
        text = browsers(
            FIREFOX.format(with_=""),
            GECKO.format(version="0.37.1"),
        )
        errors = service_pins.browser_breaches(text)[0]
        self.assertEqual(len(errors), 1)
        self.assertIn("setup-firefox", errors[0])

    def test_a_driver_with_a_channel_fails(self) -> None:
        text = browsers(GECKO.format(version="latest"))
        self.assertIn("no exact version", service_pins.browser_breaches(text)[0][0])


if __name__ == "__main__":
    unittest.main()
