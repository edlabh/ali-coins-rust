# Build multi-stage do ali-coins-rust.
# Estágio 1: compila o binário com o workspace completo.
FROM rust:1.85-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
# Build de release (LTO thin definido no perfil do workspace).
RUN cargo build --release -p ali-coins-cli

# Estágio 2: runtime mínimo, sem toolchain, com as libs do Chromium.
FROM debian:bookworm-slim AS runtime
ARG APP_UID=10001
ARG APP_GID=10001

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libnss3 libnspr4 libasound2 \
        libatk1.0-0 libatk-bridge2.0-0 libcups2 \
        libdrm2 libxkbcommon0 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 \
        libgbm1 libpango-1.0-0 libcairo2 libatspi2.0-0 \
        libx11-6 libxcb1 libxext6 \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid "${APP_GID}" appuser \
    && useradd --uid "${APP_UID}" --gid "${APP_GID}" --create-home --shell /usr/sbin/nologin appuser \
    && mkdir -p /app/scratch \
    && chown -R appuser:appuser /app

WORKDIR /app
COPY --from=builder /src/target/release/ali-coins /usr/local/bin/ali-coins

ENV ALI_COINS_HOME=/app \
    SCRATCH_DIR=/app/scratch \
    NO_SANDBOX=true

USER appuser

# Healthcheck usa o dry-run (valida config sem abrir navegador).
HEALTHCHECK --interval=10m --timeout=30s --start-period=90s --retries=5 \
    CMD ["ali-coins", "--dry-run", "--json"]

STOPSIGNAL SIGTERM
CMD ["ali-coins", "--dry-run", "--json"]
