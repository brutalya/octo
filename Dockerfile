# syntax=docker/dockerfile:1
FROM node:20-alpine AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1-bookworm AS rust
WORKDIR /src
# Dependency layer: manifests only, stub sources.
COPY Cargo.toml Cargo.lock ./
COPY crates/octo-core/Cargo.toml crates/octo-core/Cargo.toml
COPY crates/octo-net/Cargo.toml crates/octo-net/Cargo.toml
COPY crates/octo-server/Cargo.toml crates/octo-server/Cargo.toml
COPY crates/octo-desktop/Cargo.toml crates/octo-desktop/Cargo.toml
RUN mkdir -p crates/octo-core/src crates/octo-net/src crates/octo-server/src crates/octo-desktop/src \
 && touch crates/octo-core/src/lib.rs crates/octo-net/src/lib.rs crates/octo-server/src/lib.rs crates/octo-desktop/src/lib.rs \
 && echo 'fn main() {}' | tee crates/octo-server/src/main.rs > crates/octo-desktop/src/main.rs \
 && cargo build --release -p octo-server || true
COPY crates/ crates/
RUN find crates -name '*.rs' -exec touch {} + && cargo build --release -p octo-server

FROM debian:bookworm-slim AS base
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 10001 --create-home --home-dir /home/octo --shell /usr/sbin/nologin octo
COPY --from=rust /src/target/release/octo-server /usr/local/bin/octo-server

FROM base AS relay
USER octo
EXPOSE 7443
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD ["octo-server", "--healthcheck", "127.0.0.1:7443"]
ENTRYPOINT ["octo-server", "--relay-only", "--relay-addr", "0.0.0.0:7443"]

# Full node: web UI + API + Claude Code CLI. Antigravity (agy) is optional, see docs/deploy.md.
FROM node:20-bookworm-slim AS node
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git \
 && rm -rf /var/lib/apt/lists/* \
 && npm install -g @anthropic-ai/claude-code \
 && npm cache clean --force \
 && useradd --uid 10001 --create-home --home-dir /home/octo --shell /usr/sbin/nologin octo \
 && mkdir -p /data /home/octo/.claude \
 && chown -R octo:octo /data /home/octo
COPY --from=rust /src/target/release/octo-server /usr/local/bin/octo-server
COPY --from=web /src/web/dist /opt/octo/web
USER octo
ENV HOME=/home/octo
WORKDIR /data
VOLUME ["/data", "/home/octo/.claude"]
EXPOSE 7878
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["octo-server", "--healthcheck", "127.0.0.1:7878"]
ENTRYPOINT ["octo-server", "--addr", "0.0.0.0:7878", "--data-dir", "/data", "--web-dir", "/opt/octo/web", "--token-file", "/data/token"]
