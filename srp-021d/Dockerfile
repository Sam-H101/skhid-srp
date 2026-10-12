# syntax=docker/dockerfile:1
# srp, the Survarium 0.21d mock server, as one small non-root image.
#   docker build -t srp .
# Run it through docker-compose.yml (see docs/DEPLOY.md).

FROM rust:1-slim-bookworm AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev build-essential \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY servers servers
COPY scripts scripts
COPY resources/binary-config-examples resources/binary-config-examples
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p srp \
 && cp target/release/srp /srp

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends libssl3 ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin srp \
 && mkdir -p /srv/srp/data \
 && chown srp /srv/srp/data
WORKDIR /srv/srp
COPY --from=build /srp /usr/local/bin/srp
# The game's own key and certificate: the client has the certificate pinned,
# so they are not secrets (they are in the repository).
COPY certs ./certs

USER srp
# Inside the container nothing needs a privileged port: the compose file
# publishes the browser server at 80.
ENV SRP_BIND_HOST=0.0.0.0 \
    SRP_BROWSER_PORT=8080 \
    SRP_LOCAL_PEERS=off \
    SRP_WEB_BIND=0.0.0.0 \
    SRP_WEB_PORT=8088 \
    SRP_LOG_FILES=off \
    SRP_SNAPSHOT_DIR=off \
    SRP_DATA_DIR=/srv/srp/data
VOLUME /srv/srp/data
EXPOSE 8088/tcp 8080/tcp 1234/tcp 1234/udp 1235/tcp 1236/udp
STOPSIGNAL SIGTERM
ENTRYPOINT ["srp"]
