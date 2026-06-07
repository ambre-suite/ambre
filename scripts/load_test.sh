#!/usr/bin/env bash
#
# Ambre Blind Relay -- load test launcher.
#
# Spawns a goose load test against a running ambre-server instance.
# All goose flags are forwarded after a double-dash.
#
# Usage:
#   ./scripts/load_test.sh                           # defaults
#   ./scripts/load_test.sh -- -u 500 -t 60s           # custom
#   ./scripts/load_test.sh -- -H https://ambre.app -u 1000 -r 100 -t 300s --report-file report.html
#
# Environment variables (overridable):
#   HOST          -- target base URL  (default: http://localhost:3000)
#   USERS         -- concurrent VUs   (default: 1000)
#   HATCH_RATE    -- users/sec to start (default: 50)
#   RUN_TIME      -- test duration    (default: 120s)
#   REPORT        -- HTML report path (default: empty = no report)

set -euo pipefail

HOST="${HOST:-http://localhost:3000}"
USERS="${USERS:-1000}"
HATCH_RATE="${HATCH_RATE:-50}"
RUN_TIME="${RUN_TIME:-120s}"
REPORT="${REPORT:-}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

echo "=== Ambre Load Test ==="
echo "  Host:       $HOST"
echo "  Users:      $USERS"
echo "  Hatch rate: $HATCH_RATE"
echo "  Run time:   $RUN_TIME"
echo "  Report:     ${REPORT:-<none>}"
echo ""

FLAGS=(
  -H "$HOST"
  -u "$USERS"
  -r "$HATCH_RATE"
  -t "$RUN_TIME"
)

if [ -n "$REPORT" ]; then
  FLAGS+=(--report-file "$REPORT")
fi

# Forward extra flags from the command line (after --).
if [ "${1:-}" = "--" ]; then
  shift
  FLAGS+=("$@")
fi

cargo run --example goose_load_test -- "${FLAGS[@]}"
