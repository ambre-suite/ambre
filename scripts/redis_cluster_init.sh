#!/usr/bin/env bash
# Joins the three Redis nodes of the `cluster` profile of `docker-compose.yml`
# into one cluster of three primaries, and waits until every hash slot is
# served. A cluster already joined is left as it is, so a second run changes
# nothing.
set -euo pipefail

nodes=(7001 7002 7003)
cli() { docker compose exec -T redis-cluster-1 redis-cli "$@"; }

for port in "${nodes[@]}"; do
    for _ in $(seq 1 30); do
        cli -p "$port" ping >/dev/null 2>&1 && break
        sleep 1
    done
    cli -p "$port" ping >/dev/null
done

# The reply is read whole first: under `pipefail`, a `grep -q` that stops
# early could kill the writer and read as false.
info="$(cli -p 7001 cluster info)"
if ! grep -q "cluster_state:ok" <<<"$info"; then
    cli --cluster create 127.0.0.1:7001 127.0.0.1:7002 127.0.0.1:7003 \
        --cluster-replicas 0 --cluster-yes >/dev/null
fi

for _ in $(seq 1 30); do
    info="$(cli -p 7001 cluster info)"
    grep -q "cluster_state:ok" <<<"$info" && break
    sleep 1
done
grep -q "cluster_state:ok" <<<"$info"
echo "redis cluster: three primaries ready at 127.0.0.1:7001-7003"
