# The relay image. It builds from the repo root, where the Cargo workspace lives:
#
#   docker build -f crates/relay/Dockerfile -t beans-relay .
#   docker run -p 8787:8787 -v beans-relay:/data beans-relay

FROM rust:1.96-slim-trixie AS chef
RUN cargo install cargo-chef --version 0.1.78 --locked
WORKDIR /src

# The recipe is the workspace's manifests and lockfile, so the layer that compiles the
# dependencies stays cached until one of those changes.
FROM chef AS plan
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
COPY --from=plan /src/recipe.json recipe.json
RUN cargo chef cook --release --locked -p beans-relay --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo build --release --locked -p beans-relay

# TLS roots are compiled in (webpki-roots), so the runtime image needs no CA bundle.
FROM debian:trixie-slim
COPY --from=build /src/target/release/beans-relay /usr/local/bin/beans-relay
# The SQLite database and the file ciphertext default to beans-relay.db and
# beans-relay.files in the working directory; a volume mounted here keeps them.
WORKDIR /data
EXPOSE 8787
# Railway names the port in $PORT. `[::]` takes IPv6, which Railway's private network uses,
# as well as IPv4.
CMD ["/bin/sh", "-c", "exec beans-relay --bind \"${BEANS_RELAY_BIND:-[::]:${PORT:-8787}}\""]
