#!/usr/bin/env bash
#
# run_tests.sh -- run the full local test suite (lint + native + db + wasm tiers).
#
# Wraps the `just` recipes (the single source of truth for test commands, kept
# in CI/local parity by the root justfile) with the two things they don't do on
# their own: bring the db-tier backing services up, and load `.env` so the
# `#[sqlx::test]` server tests find DATABASE_URL / REDIS_URL / S3_*.
#
# Usage:
#   ./run_tests.sh            # lint + native + db + wasm (default: full suite)
#   ./run_tests.sh native     # host tier only (no docker, no .env needed)
#   ./run_tests.sh db         # db tier only (services + .env)
#   ./run_tests.sh wasm       # wasm tier only (needs wasm-pack + Chrome)
#   ./run_tests.sh lint       # fmt + clippy only
#   KEEP_SERVICES=1 ./run_tests.sh   # leave the docker services running at the end
#
# The wasm tier (`cfg(target_arch = "wasm32")` chrome tests, Epic ET) runs the
# Trame Leptos UI tests under headless Chrome via `wasm-pack`; it is `-Z
# build-std`-heavy, so it comes last in the full suite.

set -euo pipefail
cd "$(dirname "$0")"

TIER="${1:-all}"

# Load .env (DATABASE_URL, REDIS_URL, S3_*) for the db tier. `set -a` exports
# every assignment so the cargo subprocess inherits them.
load_env() {
    if [[ -f .env ]]; then
        set -a
        # shellcheck disable=SC1091
        source .env
        set +a
    else
        echo "WARNING: .env not found; the db tier needs DATABASE_URL/REDIS_URL/S3_*" >&2
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
    echo ">> test-db (postgres/redis/minio)"
    load_env
    just services-up
    # Tear services down on exit (success or failure) unless KEEP_SERVICES=1.
    trap services_down EXIT
    just test-db
}

run_wasm() {
    echo ">> test-wasm (chrome / Leptos UI, headless Chrome)"
    # wasm-pack drives wasm-bindgen-test-runner; without it the tier cannot run
    # (same contract as the db tier needing docker). Fail with an actionable hint.
    if ! command -v wasm-pack >/dev/null 2>&1; then
        echo "ERROR: wasm-pack not found; install it with 'cargo install wasm-pack'" >&2
        echo "       (the wasm tier also needs a Chrome/Chromium binary on PATH)" >&2
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
