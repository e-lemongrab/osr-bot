FROM rust:1.91-bookworm AS builder

WORKDIR /workspace

COPY Cargo.toml ./
COPY src/osr-bot/rust/Cargo.toml ./src/osr-bot/rust/Cargo.toml
COPY src/osr-bot/rust/src ./src/osr-bot/rust/src

RUN cargo build -p osr-bot --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin osr-bot

COPY --from=builder /workspace/target/release/osr-bot /usr/local/bin/osr-bot

USER 10001:10001

ENTRYPOINT ["/usr/local/bin/osr-bot"]
