# syntax=docker/dockerfile:1.7
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY idp idp
COPY another-login another-login
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked && cp target/release/idp target/release/another-login /usr/local/bin/

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/idp /usr/local/bin/another-login /usr/local/bin/
COPY config /config
ENV CLIENTS_FILE=/config/clients.json WELL_KNOWN_DIR=/config/well-known
USER 65534
EXPOSE 8080
CMD ["idp"]
