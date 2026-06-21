#!/usr/bin/env bash
#
# run_tests.sh -- run the full local test suite (lint + native + db tiers).
#
# Wraps the `just` recipes (the single source of truth for test commands, kept
# in CI/local parity by the root justfile) with the two things they don't do on
# their own: bring the db-tier backing services up, and load `.env` so the
# `#[sqlx::test]` server tests find DATABASE_URL / REDIS_URL / S3_*.
#
# Usage:
#   ./run_tests.sh            # lint + native + db (default: full suite)
#   ./run_tests.sh native     # host tier only (no docker, no .env needed)
#   ./run_tests.sh db         # db tier only (services + .env)
#   ./run_tests.sh lint       # fmt + clippy only
#   KEEP_SERVICES=1 ./run_tests.sh   # leave the docker services running at the end
#
# The wasm tier is intentionally excluded: it has no wasm-bindgen-test targets
# yet (pending the Leptos UI story) and is `-Z build-std`/Chrome-heavy.

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

case "$TIER" in
    lint)   run_lint ;;
    native) run_native ;;
    db)     run_db ;;
    all)    run_lint; run_native; run_db ;;
    *)
        echo "usage: $0 [all|lint|native|db]" >&2
        exit 2
        ;;
esac

echo ">> done"
