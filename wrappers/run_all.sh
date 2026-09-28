#!/usr/bin/env bash
# Execução unificada: check-in + tarefas, com a semântica de exit do oráculo.
#
#  0 sucesso · 1 falha · 2 sem ação · 3 lock ativo · 4 streak quebrado · 5 2FA
#
# Retenta UMA vez (sem atraso) quando o check-in falha com exit 1, como o
# run_all.sh original; demais códigos são propagados.
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"

ali-coins checkin "$@"
CODE=$?

if [[ "$CODE" -eq 1 ]]; then
  echo "[run_all] check-in falhou (exit 1); retentando uma vez com --no-delay..." >&2
  ali-coins checkin --no-delay "$@"
  CODE=$?
fi

case "$CODE" in
  0|2)
    ali-coins tasks "$@"
    exit $?
    ;;
  *)
    exit "$CODE"
    ;;
esac
