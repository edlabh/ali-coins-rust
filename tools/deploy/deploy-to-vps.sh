#!/usr/bin/env bash
# Deploy do binário release para a VM (executar NA MÁQUINA LOCAL).
#
# Uso: ./deploy-to-vps.sh <usuario@host> [dir-destino-remoto]
# Requer: chave SSH configurada e o build de release já feito.
set -euo pipefail

TARGET="${1:?uso: deploy-to-vps.sh <usuario@host> [dir-destino]}"
DEST_DIR="${2:-~/ali-coins-rust}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="$ROOT/target/release/ali-coins"

if [[ ! -x "$BIN" ]]; then
  echo "Build de release ausente; rode: cargo build --release -p ali-coins-cli" >&2
  exit 1
fi

echo "-> Enviando binário para $TARGET ..."
scp "$BIN" "$TARGET:/tmp/ali-coins-rust.bin"
scp "$ROOT/tools/deploy/install-vps.sh" "$TARGET:/tmp/install-vps.sh"

echo "-> Instalando e agendando (sem tocar no restante do crontab) ..."
ssh "$TARGET" "bash /tmp/install-vps.sh /tmp/ali-coins-rust.bin '$DEST_DIR'"

echo "-> Validando dry-run na VM ..."
ssh "$TARGET" "cd '$DEST_DIR' && ./ali-coins --dry-run --json | head -5"
