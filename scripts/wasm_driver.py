#!/usr/bin/env python3
"""The wasm tier drives Chrome, whatever the environment exports for the belt.

`just test-wasm` runs each module under `wasm-bindgen-test-runner`, which picks
its WebDriver server from the environment. `GECKODRIVER` wins over
`CHROMEDRIVER` when both are set, so an environment that exports both for the
browser tier makes the wasm tier drive Firefox. A Chrome for Testing
`chromedriver` also looks for Chrome on the default path, and opens no session
when the Chrome there is of another version or absent.

This wrapper runs one command in an environment the decision below sets:

* `GECKODRIVER` is removed: the wasm tier is Chrome-bound.
* `CHROMEDRIVER` keeps its value, or names `chromedriver` on the `PATH`.
* When `BELT_CHROME` names a Chrome, the wrapper writes a capabilities file
  that names that binary under `target/`, and points
  `WASM_BINDGEN_TEST_WEBDRIVER_JSON` at it.
* A `WASM_BINDGEN_TEST_WEBDRIVER_JSON` set by hand wins: the wrapper writes no
  file and keeps the value, since the caller asked for those capabilities.

A CI runner sets neither `GECKODRIVER` nor `BELT_CHROME`, so its command runs
with `CHROMEDRIVER` alone, as before.

Run: `python3 scripts/wasm_driver.py <command> [args...]`, as `just test-wasm`
does.
"""

from __future__ import annotations

import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CAPABILITIES = ROOT / "target" / "wasm-webdriver.json"
JSON_VAR = "WASM_BINDGEN_TEST_WEBDRIVER_JSON"


@dataclass(frozen=True)
class Plan:
    """The environment of the command, and the capabilities file to write first."""

    env: dict[str, str]
    capabilities: str | None


def plan(environ: dict[str, str], capabilities_path: Path) -> Plan:
    """Decide the driver environment from `environ`. Pure: it writes nothing."""
    env = {key: value for key, value in environ.items() if key != "GECKODRIVER"}
    env["CHROMEDRIVER"] = environ.get("CHROMEDRIVER") or "chromedriver"
    chrome = environ.get("BELT_CHROME")
    if not chrome or environ.get(JSON_VAR):
        return Plan(env, None)
    env[JSON_VAR] = str(capabilities_path)
    return Plan(env, json.dumps({"goog:chromeOptions": {"binary": chrome}}))


def main(argv: list[str]) -> int:
    if not argv:
        print("usage: wasm_driver.py <command> [args...]", file=sys.stderr)
        return 2
    decided = plan(dict(os.environ), CAPABILITIES)
    if decided.capabilities is not None:
        CAPABILITIES.parent.mkdir(parents=True, exist_ok=True)
        CAPABILITIES.write_text(decided.capabilities + "\n", encoding="utf-8")
    os.execvpe(argv[0], argv, decided.env)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
