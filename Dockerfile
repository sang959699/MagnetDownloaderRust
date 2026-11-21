# syntax=docker/dockerfile:1

# ---- build stage -----------------------------------------------------------
# alpine + musl produces a fully static binary, so the runtime image can be
# `scratch` (no shell, no libc, no package manager, nothing to patch).
FROM rust:1-alpine AS builder

# ring (TLS backend used by rustls) is partly C, so it needs a C toolchain.
RUN apk add --no-cache build-base

WORKDIR /src

# Compile dependencies in their own layer so editing main.rs does not trigger
# a full rebuild of the dependency tree.
COPY Cargo.toml ./
RUN mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY src ./src
# COPY preserves the source mtime, which can be older than the dummy file
# created above. Without `touch`, cargo would consider the crate up to date
# and the image would ship the `fn main() {}` stub.
#
# NOTE: do not run the binary during build -- the tool has no CLI args and
# enters its 15-minute polling loop forever, which blocks the build step.
RUN touch src/main.rs \
    && cargo build --release

# ---- runtime stage ---------------------------------------------------------
FROM scratch

LABEL org.opencontainers.image.title="magnetdownloader" \
      org.opencontainers.image.description="RSS -> aria2 magnet downloader"

COPY --from=builder /src/target/release/magnetdownloader /magnetdownloader

# The binary only reads/writes the config directory, which is bind-mounted at
# /app/config at runtime.
USER 65534:65534

ENV CONFIG_DIR=/app/config

ENTRYPOINT ["/magnetdownloader"]
