# trusted-host-v1: caller supplies an immutable, reviewed Rust builder image digest.
ARG RUST_BUILDER_IMAGE=rust:1.85-bookworm@sha256:e51d0265072d2d9d5d320f6a44dde6b9ef13653b035098febd68cce8fa7c0bc4
FROM ${RUST_BUILDER_IMAGE} AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY public ./public
COPY containers/codex/Dockerfile.node-vite containers/codex/prepare-node-cache.mjs ./containers/codex/
RUN cargo build --locked --release --bin workboard-contained

FROM node:24.20.0-bookworm-slim@sha256:e9b5516b06baeaea9a8e65a7aec6a85fbb960a30b52b66968f2c8092b3e2a3eb
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates python3 && rm -rf /var/lib/apt/lists/*
RUN groupadd --gid 65532 workboard && useradd --no-log-init --no-create-home --uid 65532 --gid 65532 --home-dir /tmp/home --shell /usr/sbin/nologin workboard
COPY --chmod=0555 containers/codex/python3-no-bytecode /usr/local/bin/python3
WORKDIR /opt/codex
COPY containers/codex/package.json containers/codex/package-lock.json ./
RUN npm ci --ignore-scripts --omit=dev && npm cache clean --force
COPY --from=build --chmod=0555 /build/target/release/workboard-contained /usr/local/bin/workboard-contained
RUN touch /opt/workboard-contained-v1 && chmod 0444 /opt/workboard-contained-v1
USER 65532:65532
WORKDIR /tmp
ENTRYPOINT ["/usr/local/bin/workboard-contained"]
