#!/usr/bin/env bash
# Interop Rust -> Node: gera tokens com a implementação Rust e decifra com o
# oráculo Node (security.js). Falha se qualquer token não for aceito.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export PATH="$HOME/.cargo/bin:${PATH}"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

(
  cd "$ROOT"
  cargo run -q -p ali-coins-core --example gen-tokens
) >"$tmp"

node "$ROOT/tools/parity/node/verify_rust_tokens.mjs" <"$tmp"
