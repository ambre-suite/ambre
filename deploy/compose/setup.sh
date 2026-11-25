#!/bin/sh
# Writes the `.env` of the self-hosted stack: its domain, its public address
# and a fresh secret for each service (`deploy/README.md`). Run it once, in
# this directory, before the first `docker compose up`:
#
#   ./setup.sh ambre.example.org 203.0.113.7 admin@example.org
#
# The third argument is the email of the ACME account that obtains the
# certificate, or `internal` for a certificate of Caddy's own authority, which
# no browser trusts unless told to.
#
# It needs `openssl` alone. It refuses to overwrite an existing `.env`, since a
# new secret would lock the stack out of its own database and object store.
set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <domain> <public IPv4 address> <acme email | internal>" >&2
    exit 2
fi
domain=$1
address=$2
tls=$3
dir=$(cd "$(dirname "$0")" && pwd)
# `AMBRE_ENV_FILE` names another file, for a stack under test.
env_file=${AMBRE_ENV_FILE:-"$dir/.env"}

if [ -e "$env_file" ]; then
    echo "$env_file exists: remove it first to start a new stack" >&2
    exit 1
fi
command -v openssl >/dev/null || { echo "setup.sh needs openssl" >&2; exit 1; }

hex() { openssl rand -hex "$1"; }
# Bytes on standard input, as lowercase hex on one line.
tohex() { od -An -v -tx1 | tr -d ' \n'; }

# The admission key of a call: an Ed25519 key that openssl draws. The relay
# signs each admission with its seed, the last 32 bytes of the PKCS#8
# envelope, and the SFU verifies with the public key alone
# (`ambre-spec-call-media.md`).
key=$(mktemp)
trap 'rm -f "$key"' EXIT
openssl genpkey -algorithm ed25519 -outform DER -out "$key" 2>/dev/null
admission_seed=$(tail -c 32 "$key" | tohex)
admission_key=$(openssl pkey -inform DER -in "$key" -pubout -outform DER | tail -c 32 | tohex)

umask 077
cat > "$env_file" <<EOF
# Written by setup.sh. Keep it: each secret below guards a store of the stack.
AMBRE_DOMAIN=$domain
AMBRE_PUBLIC_IP=$address
# The ACME account of Caddy, or "internal" for a certificate of its own authority.
AMBRE_TLS=$tls
POSTGRES_PASSWORD=$(hex 24)
REDIS_PASSWORD=$(hex 24)
GARAGE_RPC_SECRET=$(hex 32)
GARAGE_ADMIN_TOKEN=$(hex 32)
GARAGE_KEY_ID=GK$(hex 12)
GARAGE_KEY_SECRET=$(hex 32)
JWT_SIGNING_SEED=$(hex 32)
CALL_ADMISSION_SEED=$admission_seed
# The seed of the thread link receipts (\`SRV-363\`).
THREAD_LINK_SEED=$(hex 32)
SFU_ADMISSION_KEY=$admission_key
TURN_SECRET=$(hex 32)
# The bearer token a scrape of the relay metrics presents (\`SRV-318\`).
METRICS_TOKEN=$(hex 32)
# The provider sign-in (\`SRV-074\`). Empty turns it off.
OIDC_GOOGLE_CLIENT_ID=
OIDC_GOOGLE_CLIENT_SECRET=
# The OpenID Connect issuer of the organization, in place of Google
# (\`SRV-340\`): its https address, and the client it issued the suite.
OIDC_ISSUER=
OIDC_CLIENT_ID=
OIDC_CLIENT_SECRET=
EOF
echo "wrote $env_file"
