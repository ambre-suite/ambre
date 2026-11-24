#!/usr/bin/env sh
# Start the Ambre Blind Relay server (ambre-server).
#
# The server requires PostgreSQL, Redis, and S3 (Garage), all three under Docker,
# never on the host. The script runs `just docker-up` first: it starts the Docker
# daemon when it is down, then the three services, and creates the Garage layout,
# key and bucket, which `docker compose up -d` alone does not.
#
# The script starts the relay against the local stack, so it sets DEV_MODE=1
# unless the shell or .env sets it: the relay then takes the database password
# and the Garage key of the local stack, and `.cargo/config.toml` gives it the
# addresses. No .env is needed; one, if present, overrides any of them.
# Prerequisites: cargo, docker

set -e

DIR="$(dirname "$(realpath "$0")")"

# Export variables from .env (if it exists) so cargo picks them up
if [ -f "$DIR/.env" ]; then
    set -a
    . "$DIR/.env"
    set +a
fi

export DEV_MODE="${DEV_MODE:-1}"

(cd "$DIR" && just docker-up)
# The proxy of a cloud session exports placeholder AWS keys, and Garage refuses them.
unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY

cargo run -p ambre-server
