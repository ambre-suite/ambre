#!/usr/bin/env bash
#
# run_tests.sh -- run the full local test suite (lint + native + db + wasm tiers).
#
# Wraps the `just` recipes (the single source of truth for test commands, kept
# in CI/local parity by the root justfile) with the two things they don't do on
# their own: bring the db-tier backing services up, and load an optional
# `.env`. The addresses of the local stack come from `.cargo/config.toml`;
# a `.env` only overrides them.
#
# Usage:
#   ./run_tests.sh            # lint + native + db + wasm (default: full suite)
#   ./run_tests.sh native     # host tier only (no docker)
#   ./run_tests.sh db         # db tier only (starts the services)
#   ./run_tests.sh wasm       # wasm tier only (needs wasm-bindgen-test-runner + Chrome)
#   ./run_tests.sh lint       # fmt + clippy only
#   KEEP_SERVICES=1 ./run_tests.sh   # leave the docker services running at the end
#
# The wasm tier (`cfg(target_arch = "wasm32")` chrome tests, Epic ET) runs the
# Leptos UI tests under headless Chrome via `wasm-bindgen-test-runner`; it is
# `-Z build-std`-heavy, so it comes last in the full suite.

set -euo pipefail
cd "$(dirname "$0")"

TIER="${1:-all}"

# Load an optional .env that overrides the local stack of `.cargo/config.toml`.
# `set -a` exports every assignment so the cargo subprocess inherits them.
load_env() {
    if [[ -f .env ]]; then
        set -a
        # shellcheck disable=SC1091
        source .env
        set +a
    fi
}

services_down() {
    if [[ "${KEEP_SERVICES:-0}" != "1" ]]; then
        echo ">> tearing down db-tier services"
        just services-down
    fi
}

run_lint() {
    echo ">> lint (fmt + clippy)"
    just lint
}

run_native() {
    echo ">> test-native (host tier)"
    just test-native
}

run_db() {
    echo ">> test-db (postgres/redis/garage)"
    load_env
    # Starts the Docker daemon when it is down (a cloud session), then the services.
    just docker-up
    # The proxy of a cloud session exports placeholder AWS keys, and Garage refuses
    # them. CI runs without them.
    unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY
    # Tear services down on exit (success or failure) unless KEEP_SERVICES=1.
    trap services_down EXIT
    just test-db
    # The same tier against a Redis cluster of three primaries, as CI runs it.
    echo ">> test-db-cluster (postgres/garage, a redis cluster of three primaries)"
    just cluster-up
    just test-db-cluster
}

run_wasm() {
    echo ">> test-wasm (chrome / Leptos UI, headless Chrome)"
    # cargo runs each test module through wasm-bindgen-test-runner; without it
    # the tier cannot run (same contract as the db tier needing docker). Fail
    # with an actionable hint naming the version Cargo.lock pins.
    if ! command -v wasm-bindgen-test-runner >/dev/null 2>&1; then
        version=$(awk '$0 == "name = \"wasm-bindgen\"" { getline; gsub(/"/, "", $3); print $3 }' Cargo.lock)
        echo "ERROR: wasm-bindgen-test-runner not found; install it with" >&2
        echo "       'cargo install wasm-bindgen-cli --version ${version}'" >&2
        echo "       (the wasm tier also needs Chrome and its chromedriver, see CHROMEDRIVER)" >&2
        exit 1
    fi
    just test-wasm
}

case "$TIER" in
    lint)   run_lint ;;
    native) run_native ;;
    db)     run_db ;;
    wasm)   run_wasm ;;
    all)    run_lint; run_native; run_db; run_wasm ;;
    *)
        echo "usage: $0 [all|lint|native|db|wasm]" >&2
        exit 2
        ;;
esac

echo ">> done"
