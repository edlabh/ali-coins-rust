#!/usr/bin/env bash
# Compara `--dry-run --json` entre o oráculo Node e o binário Rust.
# Uso: ./tools/parity/compare-dry-run.sh [--require-rust]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REF="$ROOT/reference/ali-coins"
BIN="$ROOT/target/debug/ali-coins"
REQUIRE_RUST=0
[[ "${1:-}" == "--require-rust" ]] && REQUIRE_RUST=1

SECRET="parity-test-secret-0123456789abcdef"
export ALI_USER="parity-fixture@example.com"
export ALI_PASSWORD="parity-fixture-password"
export SESSION_SECRET="$SECRET"
export ENCRYPT_LOCAL_SESSION="false"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

echo "== Node (oráculo) =="
set +e
( cd "$REF" && node all.js --dry-run --json ) >"$tmpdir/node.out" 2>"$tmpdir/node.err"
NODE_CODE=$?
set -e
echo "exit=$NODE_CODE"

if [[ ! -x "$BIN" ]]; then
  echo "Binário Rust ainda não compilado; rode: cargo build -p ali-coins-cli" >&2
fi

if [[ -x "$BIN" ]]; then
  echo "== Rust =="
  set +e
  ( cd "$ROOT" && "$BIN" --dry-run --json ) >"$tmpdir/rust.out" 2>"$tmpdir/rust.err"
  RUST_CODE=$?
  set -e
  echo "exit=$RUST_CODE"

  if [[ $NODE_CODE -ne $RUST_CODE ]]; then
    echo "DIVERGÊNCIA de exit code: node=$NODE_CODE rust=$RUST_CODE"
    echo "--- node.err (tail) ---"; tail -5 "$tmpdir/node.err" || true
    echo "--- rust.err (tail) ---"; tail -5 "$tmpdir/rust.err" || true
    exit 2
  fi

  PARITY_ROOT="$ROOT" node "$ROOT/tools/parity/node/normalize.mjs" <"$tmpdir/node.out" >"$tmpdir/node.norm"
  PARITY_ROOT="$ROOT" node "$ROOT/tools/parity/node/normalize.mjs" <"$tmpdir/rust.out" >"$tmpdir/rust.norm"

  if diff -u "$tmpdir/node.norm" "$tmpdir/rust.norm" >"$tmpdir/diff.txt"; then
    echo "OK: dry-run JSON idêntico (normalizado)."
    exit 0
  fi
  echo "DIVERGÊNCIA no JSON normalizado:"
  cat "$tmpdir/diff.txt"
  exit 2
fi

if [[ $REQUIRE_RUST -eq 1 ]]; then
  echo "ERRO: --require-rust usado, mas o binário não existe." >&2
  exit 3
fi

echo "AVISO: Rust ainda não compilado/implementado (fase 1). Comparação ignorada."
