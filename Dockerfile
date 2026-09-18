# Docker image for the v1 GitHub Action (ADR: Docker action for v1 distribution).
# The Action is a thin wrapper over the CLI primitive.
FROM rust:1.91-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY schemas ./schemas
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=build /src/target/release/actionlint-rs /usr/local/bin/actionlint-rs
ENTRYPOINT ["/usr/local/bin/actionlint-rs"]
