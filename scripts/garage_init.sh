#!/usr/bin/env bash
# Prepares the Garage of `docker-compose.yml`, or of CI, for the dev stack and
# the test tiers: a layout for its one node, the dev key, the bucket, and the
# CORS rule the browser needs. Each step skips what is already there, so a
# second run changes nothing.
set -euo pipefail

container="${GARAGE_CONTAINER:-ambre-garage}"
endpoint="${S3_ENDPOINT:-http://localhost:9000}"
bucket="${S3_BUCKET:-ambre-dev}"
# The dev key of the local stack, which the relay, the test helpers and the
# belt default to. It guards nothing; never reuse it in a deployment. Garage
# accepts only a key id of `GK` and 24 hex digits.
key_id="GK1c7374dda8bf9b67e1ed2022"
secret="b64c22af0c1f051c482b80d9cbde6751304fd0d084a565d2a5fb0342021899ca"

garage() { docker exec -e RUST_LOG=warn "$container" /garage "$@"; }

for _ in $(seq 1 30); do
    garage status >/dev/null 2>&1 && break
    sleep 1
done
garage status >/dev/null

# One node holds every block: the layout must name it before any request.
if garage layout show | grep -q "No nodes currently have a role"; then
    node="$(garage node id -q | cut -d@ -f1)"
    garage layout assign -z dev -c 1G "$node" >/dev/null
    garage layout apply --version 1 >/dev/null
fi

garage key info "$key_id" >/dev/null 2>&1 \
    || garage key import --yes -n ambre-dev "$key_id" "$secret" >/dev/null
garage key allow --create-bucket "$key_id" >/dev/null
garage bucket info "$bucket" >/dev/null 2>&1 || garage bucket create "$bucket" >/dev/null
garage bucket allow --read --write --owner "$bucket" --key "$key_id" >/dev/null

# The browser puts and fetches through presigned links itself, so the bucket
# must answer a cross-origin PUT and GET (`crates/ambre-server/src/s3.rs`).
# Garage allows no origin until a rule says so.
cors='<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod><AllowedMethod>PUT</AllowedMethod><AllowedHeader>*</AllowedHeader><ExposeHeader>ETag</ExposeHeader><MaxAgeSeconds>3600</MaxAgeSeconds></CORSRule></CORSConfiguration>'
md5="$(printf '%s' "$cors" | openssl dgst -md5 -binary | base64)"
for _ in $(seq 1 30); do
    if curl -sSf -o /dev/null --aws-sigv4 "aws:amz:us-east-1:s3" \
        --user "$key_id:$secret" -X PUT \
        -H "Content-MD5: $md5" -H "Content-Type: application/xml" \
        --data-binary "$cors" "$endpoint/$bucket?cors"; then
        echo "garage: bucket $bucket ready at $endpoint"
        exit 0
    fi
    sleep 1
done
echo "garage: the store refused the CORS rule of $bucket" >&2
exit 1
