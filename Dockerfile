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
# The build writes the bundled packages' licence notices into dist
# (third-party-licenses.txt), checked against deny.toml's licence list.
COPY deny.toml ./
COPY scripts/third-party-licenses.mjs scripts/
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
# The one library the binary needs beyond glibc: libgcc_s (stack unwinding).
# The runtime base has glibc but no libgcc_s, so it is taken from this stage -
# the same Debian release - together with its copyright file and its dpkg
# record, so that the SBOM and the image scan still see the package.
RUN multiarch="$(dpkg-architecture -qDEB_HOST_MULTIARCH)" \
    && install -D -m 0644 "/usr/lib/${multiarch}/libgcc_s.so.1" "/out/rootfs/usr/lib/${multiarch}/libgcc_s.so.1" \
    && install -D -m 0644 /usr/share/doc/libgcc-s1/copyright /out/rootfs/usr/share/doc/libgcc-s1/copyright \
    && install -d /out/rootfs/var/lib/dpkg/status.d \
    && dpkg-query --status libgcc-s1 > /out/rootfs/var/lib/dpkg/status.d/libgcc-s1
# The licence notices of everything the binary contains: the crates linked into
# it (from cargo metadata and the crate sources this build downloaded) and the
# embedded web client's packages. Node.js comes from the web stage (same Debian
# release) only to run the generator; it is not part of any output.
COPY --from=web /usr/local/bin/node /usr/local/bin/node
COPY deny.toml LICENSE ./
COPY scripts/third-party-licenses.mjs scripts/
RUN node scripts/third-party-licenses.mjs server --web src/server/web/dist/third-party-licenses.txt \
        > /out/THIRD_PARTY_LICENSES \
    && install -D -m 0644 /out/THIRD_PARTY_LICENSES /out/rootfs/usr/share/doc/capsulebin/THIRD_PARTY_LICENSES \
    && install -D -m 0644 LICENSE /out/rootfs/usr/share/doc/capsulebin/LICENSE

# --- binary only -------------------------------------------------------------
# The release binary and its licence notices, for the release tarballs:
#   docker buildx build --target binary --output type=local,dest=out .
# It is the same build the image ships. Built against glibc 2.36, it runs on
# Debian 12, Ubuntu 24.04, RHEL 10 and newer.
FROM scratch AS binary
COPY --from=server /out/capsulebin /capsulebin
COPY --from=server /out/THIRD_PARTY_LICENSES /THIRD_PARTY_LICENSES

# --- runtime -----------------------------------------------------------------
# distroless "base-nossl": glibc, CA certificates and tzdata, but no OpenSSL.
# CapsuleBin never loads OpenSSL (TLS ends at the reverse proxy), and an unused
# library in the image is still something to patch and to explain in every
# scan. The "cc" variant would add OpenSSL and libstdc++, neither of which the
# binary links; its one real need, libgcc_s, is copied in below.
FROM gcr.io/distroless/base-nossl-debian12:nonroot@sha256:be40c00dfabd86576d92666e87e406714d5618342de1a0c213ad232de255172e

LABEL org.opencontainers.image.title="CapsuleBin" \
      org.opencontainers.image.description="Accountless, end-to-end encrypted paste service" \
      org.opencontainers.image.source="https://github.com/capsulebin/capsulebin" \
      org.opencontainers.image.licenses="AGPL-3.0-only"

COPY --from=server /out/rootfs/ /
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
