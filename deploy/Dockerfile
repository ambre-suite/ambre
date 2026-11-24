# syntax=docker/dockerfile:1.7
#
# The images of a release: the relay, the SFU and the client (`SRV-308`). One
# toolchain stage builds all three, so they share the pinned nightly of
# `rust-toolchain.toml`. The build context is the root of the clone:
#
#   docker build -f deploy/Dockerfile --target relay .
#
# Targets: `relay`, `sfu`, `web`, and `init`, which prepares the object store. `deploy/compose/compose.yml`
# builds each one, and `deploy/README.md` says how to run them.
#
# A build behind a proxy that re-signs TLS passes its authority as the secret
# `ca`, for example `--secret id=ca,src=/path/ca.crt`. With it, apt fetches
# over HTTPS and checks each answer against that authority. The authority
# stays in the build stages, and no image that runs carries it.

ARG UBUNTU=ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5

# The cargo caches. The server and the client build at once, and two cargo
# runs that unpack into one shared registry collide on the same crate, so each
# run takes a registry of its own while another holds it. Each build keeps its
# own target directory, locked against a second build of the same stage.

# ---------------------------------------------------------------------------
# The toolchain: the packages, rustup and the nightly of `rust-toolchain.toml`.
FROM ${UBUNTU} AS toolchain
RUN --mount=type=secret,id=ca,required=false \
    set -eu; \
    if [ -s /run/secrets/ca ]; then \
        sed -i 's#http://#https://#g' /etc/apt/sources.list.d/ubuntu.sources; \
        install -m 0644 /run/secrets/ca /tmp/build-ca.pem; \
        echo 'Acquire::https::CAInfo "/tmp/build-ca.pem";' > /etc/apt/apt.conf.d/99build-ca; \
    fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        brotli ca-certificates clang curl git libfontconfig1-dev libssl-dev lld \
        openssl pkg-config python3 build-essential; \
    if [ -s /run/secrets/ca ]; then \
        cp /run/secrets/ca /usr/local/share/ca-certificates/build-proxy.crt; \
        update-ca-certificates; \
    fi; \
    rm -f /etc/apt/apt.conf.d/99build-ca /tmp/build-ca.pem; \
    sed -i 's#https://#http://#g' /etc/apt/sources.list.d/ubuntu.sources; \
    rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
# The rustup of `.devcontainer/Dockerfile`, checked against the same digests.
ARG RUSTUP_VERSION=1.29.3
RUN set -eu; \
    case "$(dpkg --print-architecture)" in \
        amd64) triple=x86_64-unknown-linux-gnu; sum=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55 ;; \
        arm64) triple=aarch64-unknown-linux-gnu; sum=9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282 ;; \
        *) echo "no rustup digest for $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac; \
    curl -sSfo /tmp/rustup-init "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${triple}/rustup-init"; \
    echo "${sum}  /tmp/rustup-init" | sha256sum -c -; \
    chmod +x /tmp/rustup-init; \
    /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain none; \
    rm /tmp/rustup-init
# The first cargo command installs the toolchain, its targets and its components.
COPY rust-toolchain.toml /src/rust-toolchain.toml
RUN cd /src && cargo --version

# ---------------------------------------------------------------------------
# The relay and the SFU, built together so they share one dependency build.
FROM toolchain AS server-build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=private \
    --mount=type=cache,target=/src/target,id=target-server,sharing=locked \
    set -eu; \
    cargo build --release --locked -p ambre-server -p ambre-sfu; \
    mkdir -p /out; \
    cp target/release/ambre-server target/release/ambre-sfu /out/

# ---------------------------------------------------------------------------
# The client. A release build by default. `DEV_LOGIN=1` builds the sign-in by
# bare name that the browser tier uses, under the cargo profile of the release
# and without `--release`, as the call proofs do (`justfile`, `proof_build`):
# the release hook refuses that feature in a release (`ZK-045`).
# `AMBRE_EMBED_SITES` names the sites a Toile web embed may load: the hook
# writes them into the page policy, and no frame loads without one (`SRV-312`).
# `AMBRE_OIDC_ISSUER` names the OpenID Connect issuer the client offers in place
# of Google, and the one host it follows a sign-in to (`CHROME-139`).
FROM toolchain AS web-build
# The trunk that `scripts/client_digest.py` pins: another trunk writes another page.
ARG TRUNK_VERSION=0.22.1
# In /src, where `rust-toolchain.toml` names the toolchain: rustup has no default.
WORKDIR /src
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=private \
    cargo install --locked trunk --version "${TRUNK_VERSION}"
COPY . .
ARG DEV_LOGIN=
ARG AMBRE_EMBED_SITES=
ARG AMBRE_OIDC_ISSUER=
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=private \
    --mount=type=cache,target=/src/target,id=target-web,sharing=locked \
    set -eu; \
    if [ -n "${DEV_LOGIN}" ]; then \
        trunk build --cargo-profile=wasm-release --features dev-login --dist /out/dist; \
    else \
        trunk build --release --dist /out/dist; \
    fi; \
    python3 deploy/compose/caddy_headers.py /out/dist/_headers > /out/headers.caddy

# ---------------------------------------------------------------------------
# The base the two server images run on: the system of the build, its
# certificate store and one user without privileges.
FROM ${UBUNTU} AS runtime
RUN --mount=type=secret,id=ca,required=false \
    set -eu; \
    if [ -s /run/secrets/ca ]; then \
        sed -i 's#http://#https://#g' /etc/apt/sources.list.d/ubuntu.sources; \
        install -m 0644 /run/secrets/ca /tmp/build-ca.pem; \
        echo 'Acquire::https::CAInfo "/tmp/build-ca.pem";' > /etc/apt/apt.conf.d/99build-ca; \
    fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates libssl3t64 libfontconfig1; \
    rm -f /etc/apt/apt.conf.d/99build-ca /tmp/build-ca.pem; \
    sed -i 's#https://#http://#g' /etc/apt/sources.list.d/ubuntu.sources; \
    rm -rf /var/lib/apt/lists/*; \
    useradd --system --no-create-home --shell /usr/sbin/nologin ambre
USER ambre

# The relay. It listens on 3000, and on 9100 for its metrics (`SRV-257`).
FROM runtime AS relay
COPY --from=server-build /out/ambre-server /usr/local/bin/ambre-server
ENV BIND_ADDR=0.0.0.0:3000 \
    METRICS_ADDR=0.0.0.0:9100
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/ambre-server"]

# The SFU. Its media address must be the public address of the host, so the
# compose file runs it on the network of the host (`CALL-141`).
FROM runtime AS sfu
COPY --from=server-build /out/ambre-sfu /usr/local/bin/ambre-sfu
ENTRYPOINT ["/usr/local/bin/ambre-sfu"]

# The client: the built files behind Caddy, with the headers of `_headers`.
FROM caddy:2.10.2-alpine@sha256:4c6e91c6ed0e2fa03efd5b44747b625fec79bc9cd06ac5235a779726618e530d AS web
# The binary carries the file capability to bind a low port. A process that
# holds no capability cannot run such a file, so the copy drops it: the chart
# runs Caddy as a user without privileges on high ports (`deploy/chart/`). The
# compose file runs it as root, which binds port 80 without the file capability.
RUN cp /usr/bin/caddy /tmp/caddy && mv /tmp/caddy /usr/bin/caddy
COPY --from=web-build /out/dist /srv/ambre
COPY --from=web-build /out/headers.caddy /etc/caddy/headers.caddy
COPY deploy/compose/Caddyfile /etc/caddy/Caddyfile

# The one-shot preparation of the object store: its layout, the key of the
# stack and the bucket (`scripts/garage_init.py`).
FROM ${UBUNTU} AS init
RUN --mount=type=secret,id=ca,required=false \
    set -eu; \
    if [ -s /run/secrets/ca ]; then \
        sed -i 's#http://#https://#g' /etc/apt/sources.list.d/ubuntu.sources; \
        install -m 0644 /run/secrets/ca /tmp/build-ca.pem; \
        echo 'Acquire::https::CAInfo "/tmp/build-ca.pem";' > /etc/apt/apt.conf.d/99build-ca; \
    fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends curl python3; \
    rm -f /etc/apt/apt.conf.d/99build-ca /tmp/build-ca.pem; \
    sed -i 's#https://#http://#g' /etc/apt/sources.list.d/ubuntu.sources; \
    rm -rf /var/lib/apt/lists/*
COPY scripts/garage_init.py /opt/ambre/garage_init.py
USER nobody
ENTRYPOINT ["python3", "/opt/ambre/garage_init.py"]
