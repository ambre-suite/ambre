#!/bin/sh
# Writes the Secret of a new deployment of the chart to standard output: a
# fresh secret for each service (`deploy/chart/README.md`). Apply it once,
# before the first `helm install`:
#
#   ./secret.sh ambre | kubectl apply -n ambre -f -
#
# It needs `openssl` alone. Keep the Secret: each value guards a store of the
# deployment, and a new value locks the deployment out of its own data. An
# external store, or a sign-in provider, adds its keys to the same Secret
# (`deploy/chart/README.md`).
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <name of the Secret>" >&2
    exit 2
fi
name=$1
command -v openssl >/dev/null || { echo "secret.sh needs openssl" >&2; exit 1; }

hex() { openssl rand -hex "$1"; }
# Bytes on standard input, as lowercase hex on one line.
tohex() { od -An -v -tx1 | tr -d ' \n'; }

# The admission key of a call, as `deploy/compose/setup.sh` draws it: the
# relay signs with the seed, and the SFU verifies with the public key alone.
key=$(mktemp)
trap 'rm -f "$key"' EXIT
openssl genpkey -algorithm ed25519 -outform DER -out "$key" 2>/dev/null
admission_seed=$(tail -c 32 "$key" | tohex)
admission_key=$(openssl pkey -inform DER -in "$key" -pubout -outform DER | tail -c 32 | tohex)
# The link key of a thread: the relay signs each link with the seed, and the
# web pod writes the public key into the client (`SRV-364`, `SRV-365`). A
# stack that already drew the pair, as the chart test does, passes both in
# the environment.
if [ -n "${THREAD_LINK_SEED:-}" ] && [ -n "${THREAD_LINK_KEY:-}" ]; then
    link_seed=$THREAD_LINK_SEED
    link_key=$THREAD_LINK_KEY
else
    openssl genpkey -algorithm ed25519 -outform DER -out "$key" 2>/dev/null
    link_seed=$(tail -c 32 "$key" | tohex)
    link_key=$(openssl pkey -inform DER -in "$key" -pubout -outform DER | tail -c 32 | tohex)
fi
echo "secret.sh: the web pod writes the link key $link_key into the client" >&2

cat <<YAML
apiVersion: v1
kind: Secret
metadata:
  name: $name
type: Opaque
stringData:
  postgres-password: "$(hex 24)"
  redis-password: "$(hex 24)"
  garage-rpc-secret: "$(hex 32)"
  garage-admin-token: "$(hex 32)"
  s3-access-key-id: "GK$(hex 12)"
  s3-secret-access-key: "$(hex 32)"
  jwt-signing-seed: "$(hex 32)"
  call-admission-seed: "$admission_seed"
  sfu-admission-key: "$admission_key"
  thread-link-seed: "$link_seed"
  thread-link-key: "$link_key"
  metrics-token: "$(hex 32)"
YAML
