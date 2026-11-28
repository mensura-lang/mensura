# The toolchain image a deployment runs `mensura serve` from (ADR 0046
# decision 4).  CI publishes it as ghcr.io/mensura-lang/mensura, tagged
# `main` and `sha-<short commit>`; a deployment pins one of the latter.
#
# The runtime stage carries the `mensura` binary and nothing else: no
# shell, no package manager.  `mensura health` is the readiness check.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked --package mensura-cli \
 && mkdir -p /staging/var/lib/mensura

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/mensura /usr/local/bin/mensura
# The data volume's mount point, owned by the runtime user so that a
# fresh named volume inherits a writable directory.
COPY --from=build --chown=65532:65532 /staging/var/lib/mensura /var/lib/mensura
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["mensura"]
CMD ["--help"]
