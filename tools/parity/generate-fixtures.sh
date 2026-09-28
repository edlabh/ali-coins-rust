#!/usr/bin/env bash
# Gera fixtures do oráculo Node para os testes de paridade (fase 0+).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REF="$ROOT/reference/ali-coins"

if [[ ! -f "$REF/security.js" ]]; then
  echo "ERRO: oráculo não encontrado em reference/ali-coins." >&2
  echo "Clone com: git clone --depth 1 https://github.com/edlabh/ali-coins.git reference/ali-coins" >&2
  exit 1
fi

export ORACLE_COMMIT="$(git -C "$REF" rev-parse HEAD 2>/dev/null || true)"

node "$ROOT/tools/parity/node/gen_tokens.mjs"
node "$ROOT/tools/parity/node/gen_dry_run.mjs"
node "$ROOT/tools/parity/node/gen_storage_filter.mjs"
node "$ROOT/tools/parity/node/gen_time_url_guard.mjs"
node "$ROOT/tools/parity/node/gen_report.mjs"

echo "Fixtures geradas em tools/parity/fixtures/ (oráculo: ${ORACLE_COMMIT:-desconhecido})"
