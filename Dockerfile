# Build multi-stage do ali-coins-rust (imagem completa; usada pelo CI).
#
# Na VPS (1 vCPU), prefira `Dockerfile.runtime` + `wrappers/build-runtime-image.sh`:
# o binário é compilado uma vez no host (target incremental) e a imagem de
# runtime fica pronta em segundos, sem recompilar o workspace.
#
# Args de build:
#   CARGO_BUILD_JOBS                    (padrão 1 — baixa memória)
#   CARGO_PROFILE_RELEASE_LTO           (padrão false — o perfil do workspace usa
#                                        LTO thin, caro em 1 vCPU)
#   CARGO_PROFILE_RELEASE_CODEGEN_UNITS (padrão 16 — o perfil usa 1, o que
#                                        serializa/atrasa o codegen)
#
# O cache do BuildKit mantém registry/git/target entre builds: quando só o
# código muda, o cargo recompila apenas o que mudou (builds incrementais).
FROM rust:1.85-bookworm AS builder
ARG CARGO_BUILD_JOBS=1
ARG CARGO_PROFILE_RELEASE_LTO=false
ARG CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} \
    CARGO_PROFILE_RELEASE_LTO=${CARGO_PROFILE_RELEASE_LTO} \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=${CARGO_PROFILE_RELEASE_CODEGEN_UNITS}
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
# Copia o binário para fora do cache mount (fica na camada da imagem).
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release -p ali-coins-cli \
 && cp target/release/ali-coins /usr/local/bin/ali-coins

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
COPY --from=builder /usr/local/bin/ali-coins /usr/local/bin/ali-coins

ENV ALI_COINS_HOME=/app \
    SCRATCH_DIR=/app/scratch \
    NO_SANDBOX=true

USER appuser

# Healthcheck usa o dry-run (valida config sem abrir navegador).
HEALTHCHECK --interval=10m --timeout=30s --start-period=90s --retries=5 \
    CMD ["ali-coins", "--dry-run", "--json"]

STOPSIGNAL SIGTERM
CMD ["ali-coins", "--dry-run", "--json"]
