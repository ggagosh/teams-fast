# Build context: this directory (Coolify base directory `/relay`). Uses relay/Cargo.lock only,
# so the desktop's dependency graph never enters the server build.
FROM rust:1.95-slim-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src src
RUN CARGO_BUILD_JOBS=2 cargo build --locked --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home relay
COPY --from=build /app/target/release/teamsfast-relay /usr/local/bin/teamsfast-relay
USER relay
ENV TEAMSFAST_RELAY_BIND=0.0.0.0:8787
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=3s CMD curl -fsS http://127.0.0.1:8787/health || exit 1
ENTRYPOINT ["teamsfast-relay"]
