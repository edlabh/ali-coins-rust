#!/usr/bin/env bash
# Check-in diário (equivalente ao run.sh do oráculo).
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"
exec ali-coins checkin "$@"
