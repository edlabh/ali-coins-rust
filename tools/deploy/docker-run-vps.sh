#!/usr/bin/env bash
# Execução diária do ali-coins-rust em container, espelhando o docker-run.sh do
# projeto Node (usa a mesma imagem/limites e o navegador do Playwright).
#
# Uso:
#   ./docker-run-vps.sh [--dry-run] [--json]   # args repassados ao binário
#
# Variáveis opcionais:
#   ALI_COINS_RUST_DIR   diretório do projeto Rust (padrão: $HOME/ali-coins-rust)
#   ALI_COINS_IMAGE      imagem Docker (padrão: ali-coins:latest — a do Node)
#   ALI_COINS_CHROME     binário do Chromium dentro do container
#                        (padrão: /pw/chromium-1193/chrome-linux/chrome, montado do host)
set -euo pipefail

DIR="${ALI_COINS_RUST_DIR:-$HOME/ali-coins-rust}"
IMAGE="${ALI_COINS_IMAGE:-ali-coins:latest}"
CHROME="${ALI_COINS_CHROME:-/pw/chromium-1193/chrome-linux/chrome}"
PW_CACHE="${PW_CACHE:-$HOME/.cache/ms-playwright}"

exec docker run --rm --init \
  --pids-limit=256 \
  --shm-size=256m \
  --memory="${ALI_COINS_RUST_MEM:-768m}" \
  --memory-swap="${ALI_COINS_RUST_MEM_SWAP:-1536m}" \
  --cap-drop=ALL \
  --security-opt=no-new-privileges \
  --log-opt max-size=10m --log-opt max-file=3 \
  --user "$(id -u):$(id -g)" \
  -v /etc/passwd:/etc/passwd:ro \
  -v /etc/group:/etc/group:ro \
  -v "$DIR:/data" \
  -e ALI_COINS_CHROME="$CHROME" \
  -e NO_SANDBOX=true \
  -w /data \
  "$IMAGE" /data/ali-coins "$@"
