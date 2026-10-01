#!/usr/bin/env bash
# Constrói a imagem de runtime do ali-coins-rust a partir do binário já
# compilado — rápido na VPS (não recompila o workspace dentro do Docker).
#
# Uso:
#   wrappers/build-runtime-image.sh [binário] [tag]
#
# Padrões: target/release/ali-coins e ali-coins-rust:latest.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${1:-$ROOT/target/release/ali-coins}"
IMAGE="${2:-ali-coins-rust:latest}"

if [[ ! -x "$BIN" ]]; then
  echo "Binário não encontrado/executável: $BIN" >&2
  echo "Compile antes: cargo build --release -p ali-coins-cli" >&2
  exit 1
fi

CTX="$(mktemp -d)"
trap 'rm -rf "$CTX"' EXIT
cp "$ROOT/Dockerfile.runtime" "$CTX/Dockerfile"
cp "$BIN" "$CTX/ali-coins"

docker build -t "$IMAGE" "$CTX"
echo "Imagem $IMAGE pronta a partir de $BIN"
