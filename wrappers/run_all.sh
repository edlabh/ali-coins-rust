#!/usr/bin/env bash
# Execução unificada (check-in + tarefas) com uma única notificação, no formato
# do oráculo. Com a semântica de exit do Node:
#
#  0 sucesso · 1 falha · 2 sem ação · 3 lock ativo · 4 streak quebrado · 5 2FA
#
# Retenta UMA vez (sem atraso) quando a execução falha com exit 1, como o
# run_all.sh original; demais códigos são propagados.
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"

ali-coins all "$@"
CODE=$?

if [[ "$CODE" -eq 1 ]]; then
  echo "[run_all] execução falhou (exit 1); retentando uma vez com --no-delay..." >&2
  sleep 10
  ali-coins all --no-delay "$@"
  CODE=$?
fi

# Linha de fechamento (facilita a observação da Fase 6 e auditoria no cron.log).
echo "[run_all] exit=$CODE fim: $(date -u +%Y-%m-%dT%H:%M:%SZ)"

exit "$CODE"
