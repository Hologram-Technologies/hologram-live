# syntax=docker/dockerfile:1
# Hologram Registry: a drop-in for the Docker Registry image (registry:3).
# Build from the repository root:
#   docker build -f apps/registry/Dockerfile -t hologram-registry .
# Entry point, command, port, volume and environment equal the reference
# image's (distribution/distribution-library-image); gate F compares them.

FROM rust:1.97.1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --package hologram-live --features oci --bin hologram \
 && install -D target/release/hologram /out/usr/local/bin/hologram \
 && install -d /out/bin /out/var/lib/registry /out/etc/distribution \
 # Hard links, one copy of the binary: argv[0] picks the command line.
 && ln /out/usr/local/bin/hologram /out/bin/registry \
 && ln /out/usr/local/bin/hologram /out/entrypoint.sh \
 && install -m 0644 apps/registry/config.yml /out/etc/distribution/config.yml \
 # FR-S14: both licence texts inside the image, where an operator looks.
 && install -m 0644 -D LICENSE-MIT /out/licenses/LICENSE-MIT \
 && install -m 0644 -D LICENSE-APACHE /out/licenses/LICENSE-APACHE

# Root, as the reference, so a volume's ownership is the same. The data inside
# is not: a reference volume is imported, not opened (README, `oci import`).
FROM gcr.io/distroless/cc-debian12
COPY --from=build /out/ /
ENV OTEL_TRACES_EXPORTER=none
VOLUME ["/var/lib/registry"]
EXPOSE 5000
ENTRYPOINT ["/entrypoint.sh"]
CMD ["/etc/distribution/config.yml"]
