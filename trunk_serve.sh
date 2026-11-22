#!/usr/bin/env sh
# Start the Trunk dev server, with the development sign-in.
# Trunk builds the unified WASM binary (ambre-app) which serves all three
# roles: UI Thread, Logic Worker, Sync Worker.
#
# Access the app at http://localhost:8080
#
# Usage:
#   ./trunk_serve.sh [--phone] [options of trunk serve]
#
#   (no flag)  the `dev` profile: the fastest rebuild on each saved edit
#   --phone    the `wasm-phone` profile: the crates of the workspace at level 2,
#              fast enough for a call on a phone, rebuilt in seconds
#
# No release here. `dev-login` gives the `?user=` sign-in and the name field
# of a development relay, and a release build refuses it (`ZK-045`,
# `ZK-049`). Measure a release through the call proofs, which sign in through
# an identity provider (`ZK-050`).
#
# Trunk keeps serving `dist/` when a build fails, so a failed build used to
# serve the client of the last good one. The script empties `dist/` first: a
# failed build then serves nothing, and the error of Trunk is the one answer.
#
# Prerequisites: cargo, trunk

set -e

# Run from the workspace root so that Trunk.toml is picked up.
cd "$(dirname "$(realpath "$0")")"

profile=""
if [ "${1:-}" = "--phone" ]; then
    profile="--cargo-profile=wasm-phone"
    shift
fi
for arg in "$@"; do
    if [ "$arg" = "--release" ]; then
        echo "trunk_serve: no --release. A release refuses the development sign-in (ZK-045, ZK-049); use --phone for a fast build" >&2
        exit 2
    fi
done

rm -rf dist
# shellcheck disable=SC2086 # `profile` is empty or one word.
exec trunk serve --features dev-login $profile "$@"
