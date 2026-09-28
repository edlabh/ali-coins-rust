#!/usr/bin/env bash
# Tarefas diárias (equivalente ao run_tasks.sh do oráculo).
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"
exec ali-coins tasks "$@"
