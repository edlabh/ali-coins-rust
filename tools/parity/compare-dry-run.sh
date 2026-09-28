#!/usr/bin/env bash
# Compara `--dry-run [--json]` entre o oráculo Node e o binário Rust em todos os
# cenários de tools/parity/scenarios/*.env.
# Uso: ./tools/parity/compare-dry-run.sh [--require-rust]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REF="$ROOT/reference/ali-coins"
BIN="$ROOT/target/debug/ali-coins"
SCENARIOS_DIR="$ROOT/tools/parity/scenarios"
REQUIRE_RUST=0
[[ "${1:-}" == "--require-rust" ]] && REQUIRE_RUST=1

export PATH="$HOME/.cargo/bin:${PATH}"

if [[ ! -f "$REF/security.js" ]]; then
  echo "ERRO: oráculo não encontrado em reference/ali-coins." >&2
  exit 1
fi

if command -v cargo >/dev/null 2>&1; then
  ( cd "$ROOT" && cargo build -q -p ali-coins-cli )
fi
if [[ ! -x "$BIN" ]]; then
  if [[ $REQUIRE_RUST -eq 1 ]]; then
    echo "ERRO: --require-rust usado, mas o binário não existe." >&2
    exit 3
  fi
  echo "AVISO: binário Rust indisponível; comparação ignorada."
  exit 0
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

failures=0
for scenario in "$SCENARIOS_DIR"/*.env; do
  name="$(basename "$scenario" .env)"

  set +e
  (
    export PATH HOME
    set -a
    # shellcheck disable=SC1090
    . "$scenario"
    set +a
    cd "$REF" && node all.js --dry-run --json
  ) >"$tmpdir/node.out" 2>"$tmpdir/node.err"
  node_code=$?
  (
    export PATH HOME
    set -a
    # shellcheck disable=SC1090
    . "$scenario"
    set +a
    cd "$ROOT" && "$BIN" --dry-run --json
  ) >"$tmpdir/rust.out" 2>"$tmpdir/rust.err"
  rust_code=$?
  set -e

  if [[ $node_code -ne $rust_code ]]; then
    echo "FALHA [$name]: exit node=$node_code rust=$rust_code"
    tail -3 "$tmpdir/node.err" || true
    tail -3 "$tmpdir/rust.err" || true
    failures=$((failures + 1))
    continue
  fi
  if [[ $node_code -ne 0 ]]; then
    echo "OK   [$name]: ambos falharam como esperado (exit $node_code)"
    continue
  fi

  PARITY_ROOT="$ROOT" node "$ROOT/tools/parity/node/normalize.mjs" <"$tmpdir/node.out" >"$tmpdir/node.norm"
  PARITY_ROOT="$ROOT" node "$ROOT/tools/parity/node/normalize.mjs" <"$tmpdir/rust.out" >"$tmpdir/rust.norm"
  if diff -u "$tmpdir/node.norm" "$tmpdir/rust.norm" >"$tmpdir/diff.txt"; then
    echo "OK   [$name]: exit 0 e JSON normalizado idêntico"
  else
    echo "FALHA [$name]: divergência no JSON"
    cat "$tmpdir/diff.txt"
    failures=$((failures + 1))
  fi
done

if [[ $failures -gt 0 ]]; then
  echo "$failures cenário(s) divergente(s)."
  exit 2
fi
echo "Paridade de dry-run confirmada em todos os cenários."
