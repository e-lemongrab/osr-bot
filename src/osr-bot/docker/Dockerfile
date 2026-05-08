FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 10001 --home-dir /nonexistent --shell /usr/sbin/nologin osr-bot

COPY dist/osr-bot/osr-bot /usr/local/bin/osr-bot

USER 10001:10001

ENTRYPOINT ["/usr/local/bin/osr-bot"]
