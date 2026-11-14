# The dev container of Ambre (`.devcontainer/README.md`): the toolchain of
# `rust-toolchain.toml`, Trunk and the tools the stack script calls. The
# backing services are not here: they run from the images `docker-compose.yml`
# pins, beside this container (`.devcontainer/compose.yml`).
#
# The base is the release of Ubuntu the CI runners run, so a build that passes
# in CI passes here. It names that release and the digest of its multi-arch
# index, as each image of the compose file does (`just service-pins`).
FROM ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5

# `libfontconfig1-dev`: the native build links fontconfig, as CI installs it.
# `clang` and `lld`: a C dependency compiled for wasm32. `openssl`: the stack
# script writes the certificates with it. `brotli`: the release hook of Trunk.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        brotli ca-certificates clang curl git libfontconfig1-dev libssl-dev lld \
        openssl pkg-config python3 build-essential \
    && rm -rf /var/lib/apt/lists/*

# rustup, one exact release, checked against the digest of its own build per
# architecture. The toolchain comes from `rust-toolchain.toml` alone: rustup
# installs the pinned nightly on the first `cargo` call in the directory that
# holds the file, so no other file names a toolchain (`ARCH-075`).
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
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

COPY rust-toolchain.toml /opt/ambre/rust-toolchain.toml
RUN cd /opt/ambre \
    && cargo --version \
    && rustup component add rustfmt clippy

# Trunk and just, exact releases from crates.io, built with their own lock
# files. Trunk fetches the `wasm-bindgen` that `Cargo.lock` names on its first
# build, into the cache of the user.
ARG TRUNK_VERSION=0.22.1
ARG JUST_VERSION=1.59.0
RUN cd /opt/ambre \
    && cargo install --locked trunk --version "${TRUNK_VERSION}" \
    && cargo install --locked just --version "${JUST_VERSION}" \
    && rm -rf "${CARGO_HOME}/registry" "${CARGO_HOME}/git"

# The container runs as the user of the clone, whatever its id
# (`DEV_UID` in `.devcontainer/compose.yml`), so the toolchain and the home
# are writable by every user: the registry, the components and Trunk's cache.
RUN chmod -R a+rwX "${RUSTUP_HOME}" "${CARGO_HOME}" \
    && mkdir -p /home/dev && chmod 1777 /home/dev
ENV HOME=/home/dev
WORKDIR /workspace
