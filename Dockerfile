# Multi-stage build for the request-based deobfuscation service.
# Cloudflare Containers run linux/amd64 only, so pin the platform on both stages.

# ---- builder -------------------------------------------------------------
FROM --platform=linux/amd64 rust:1-bookworm AS builder
WORKDIR /app
COPY . .
# Build both binaries: `deob` (the deobfuscator, run per-request as a child) and
# `server` (the HTTP front end). `--bins --workspace` covers both crates.
RUN cargo build --release --workspace --bins

# ---- runtime -------------------------------------------------------------
# distroless/cc provides glibc + libgcc the (dynamically linked) Rust binaries
# need, with no shell/package manager — minimal attack surface, tiny image.
FROM --platform=linux/amd64 gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /app/target/release/deob   /usr/local/bin/deob
COPY --from=builder /app/target/release/server /usr/local/bin/server

# The server reads these; see server/src/main.rs for the full set.
ENV PORT=8080 \
    DEOB_BIN=/usr/local/bin/deob \
    TIMEOUT_SECS=30 \
    MAX_BODY_BYTES=5242880

EXPOSE 8080
USER nonroot
ENTRYPOINT ["/usr/local/bin/server"]
