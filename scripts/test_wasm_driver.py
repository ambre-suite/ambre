#!/usr/bin/env python3
"""Which WebDriver server `wasm_driver.py` hands the wasm tier, case by case.

Each test passes an environment to the pure `plan` function and asserts the
environment of the command and the capabilities file. Writing the file and the
`exec` are left to the run of `just test-wasm`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import wasm_driver  # noqa: E402  -- the module under test

FILE = Path("/work/target/wasm-webdriver.json")
CHROMEDRIVER = "/opt/cft/chromedriver-linux64/chromedriver"
CHROME = "/opt/cft/chrome-linux64/chrome"


def planned(**environ: str) -> wasm_driver.Plan:
    return wasm_driver.plan({"PATH": "/usr/bin", **environ}, FILE)


class TheDriver(unittest.TestCase):
    def test_a_geckodriver_set_is_removed(self) -> None:
        decided = planned(GECKODRIVER="/usr/local/bin/geckodriver", CHROMEDRIVER=CHROMEDRIVER)
        self.assertNotIn("GECKODRIVER", decided.env)
        self.assertEqual(decided.env["CHROMEDRIVER"], CHROMEDRIVER)
        self.assertEqual(decided.env["PATH"], "/usr/bin")

    def test_a_belt_chrome_set_names_its_binary_in_the_capabilities(self) -> None:
        decided = planned(BELT_CHROME=CHROME, CHROMEDRIVER=CHROMEDRIVER)
        self.assertEqual(decided.env[wasm_driver.JSON_VAR], str(FILE))
        self.assertIsNotNone(decided.capabilities)
        self.assertEqual(json.loads(decided.capabilities or ""), {"goog:chromeOptions": {"binary": CHROME}})

    def test_neither_keeps_the_ci_behaviour(self) -> None:
        decided = planned()
        self.assertEqual(decided.env, {"PATH": "/usr/bin", "CHROMEDRIVER": "chromedriver"})
        self.assertIsNone(decided.capabilities)

    def test_the_cloud_environment_drives_its_own_chrome(self) -> None:
        decided = planned(
            GECKODRIVER="/usr/local/bin/geckodriver",
            CHROMEDRIVER=CHROMEDRIVER,
            BELT_CHROME=CHROME,
            BELT_FIREFOX="/opt/firefox/firefox",
        )
        self.assertNotIn("GECKODRIVER", decided.env)
        self.assertEqual(decided.env["CHROMEDRIVER"], CHROMEDRIVER)
        self.assertEqual(decided.env[wasm_driver.JSON_VAR], str(FILE))

    def test_a_capabilities_file_set_by_hand_wins(self) -> None:
        decided = planned(BELT_CHROME=CHROME, WASM_BINDGEN_TEST_WEBDRIVER_JSON="/mine.json")
        self.assertEqual(decided.env[wasm_driver.JSON_VAR], "/mine.json")
        self.assertIsNone(decided.capabilities)

    def test_an_empty_belt_chrome_names_no_chrome(self) -> None:
        decided = planned(BELT_CHROME="", CHROMEDRIVER="")
        self.assertNotIn(wasm_driver.JSON_VAR, decided.env)
        self.assertEqual(decided.env["CHROMEDRIVER"], "chromedriver")


if __name__ == "__main__":
    unittest.main()
