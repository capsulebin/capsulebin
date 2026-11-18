# CapsuleBin container image.
#
#   docker build -t capsulebin .
#   docker run --rm capsulebin token            # a creation token and its hash
#   docker run -d -p 8080:8080 -v capsulebin-data:/data \
#     -e CAPSULEBIN__SERVER__PUBLIC_URL=https://paste.example \
#     -e CAPSULEBIN__CREATION__TOKEN_HASHES='["sha256:..."]' \
#     capsulebin
#
# Configuration comes from CAPSULEBIN__ environment variables (or a file mounted
# and passed with --config). The database lives on the /data volume. The runtime
# image is distroless: no shell, no package manager, a non-root user.
#
# Base images are pinned by digest; the tags next to them say what the digest
# was taken from.

# --- web client --------------------------------------------------------------
# Built first: the server embeds src/server/web/dist at compile time.
FROM docker.io/library/node:24-bookworm-slim@sha256:dc26ae92c9fb15cf6b1d63e20f0944b2efe0f115b20e00be48fb23fceb061c54 AS web

WORKDIR /build
# The manifests alone first, so the dependency layer is reused while only
# source changes. npm ci needs every workspace's manifest.
COPY package.json package-lock.json ./
COPY src/core/package.json src/core/
COPY src/cli/package.json src/cli/
COPY src/server/web/package.json src/server/web/
RUN npm ci --no-audit --no-fund

COPY src/core src/core
COPY src/server/web src/server/web
RUN npm run build -w @capsulebin/web

# --- server ------------------------------------------------------------------
FROM docker.io/library/rust:1.99.1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c AS server

WORKDIR /build
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY migrations migrations
COPY src/server src/server
COPY --from=web /build/src/server/web/dist src/server/web/dist
RUN cargo build --release --locked -p capsulebin-server \
    && install -D -m 0755 target/release/capsulebin /out/capsulebin \
    && install -d -m 0700 /out/data

# --- binary only -------------------------------------------------------------
# The release binary on its own, for the release tarballs:
#   docker buildx build --target binary --output type=local,dest=out .
# It is the same build the image ships. Built against glibc 2.36, it runs on
# Debian 12, Ubuntu 24.04, RHEL 10 and newer.
FROM scratch AS binary
COPY --from=server /out/capsulebin /capsulebin

# --- runtime -----------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

LABEL org.opencontainers.image.title="CapsuleBin" \
      org.opencontainers.image.description="Accountless, end-to-end encrypted paste service" \
      org.opencontainers.image.source="https://github.com/capsulebin/capsulebin" \
      org.opencontainers.image.licenses="AGPL-3.0-only"

COPY --from=server /out/capsulebin /usr/local/bin/capsulebin
# Owned by the image's non-root user, so a fresh named volume inherits it.
COPY --from=server --chown=65532:65532 /out/data /data

ENV CAPSULEBIN__SERVER__BIND=0.0.0.0:8080 \
    CAPSULEBIN__DATABASE__PATH=/data/capsulebin.db

USER 65532:65532
VOLUME ["/data"]
EXPOSE 8080

# /health/ready: assets, database and schema are usable. An orchestrator that
# restarts unhealthy containers should probe with `healthcheck --live` instead.
HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 \
    CMD ["/usr/local/bin/capsulebin", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/capsulebin"]
CMD ["serve"]
