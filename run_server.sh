#!/usr/bin/env sh
# Start the Ambre Blind Relay server (ambre-server).
#
# The server requires PostgreSQL, Redis, and S3 (Garage) to be running.
# Run `docker compose up -d` first to provision these services.
#
# Environment variables are read from .env if present.
# Prerequisites: cargo, docker

set -e

DIR="$(dirname "$(realpath "$0")")"

# Export variables from .env (if it exists) so cargo picks them up
if [ -f "$DIR/.env" ]; then
    set -a
    . "$DIR/.env"
    set +a
fi

cargo run -p ambre-server
