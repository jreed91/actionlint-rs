# Docker image for the GitHub Action (a thin wrapper over the CLI).
#
# We build a fully static musl binary so the image has NO glibc dependency. This avoids the
# GLIBC-version mismatch that a dynamically-linked binary hits when the build base (rust
# image, tracking Debian trixie) has a newer glibc than the runtime base. A static binary
# runs on any base, including a minimal one.
FROM rust:1.91-slim AS build
WORKDIR /src
RUN rustup target add x86_64-unknown-linux-musl
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY schemas ./schemas
# Link the musl target with rust-lld (bundled with the toolchain) so no external musl-gcc /
# `cc` is needed — avoids the `cc: unrecognized option '-m64'` failure and keeps the build
# self-contained.
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
RUN cargo build --release --target x86_64-unknown-linux-musl

# Minimal runtime: the static binary needs no shared libraries. Use a slim base (rather than
# scratch) so `ca-certificates` and a shell are available for the Action environment.
FROM debian:bookworm-slim
COPY --from=build \
    /src/target/x86_64-unknown-linux-musl/release/actionlint-rs \
    /usr/local/bin/actionlint-rs
ENTRYPOINT ["/usr/local/bin/actionlint-rs"]
