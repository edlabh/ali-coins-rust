#!/usr/bin/env bash
# Instala/atualiza o ali-coins-rust NA VM e agenda às 08:30 de America/Sao_Paulo.
#
# Uso (na VM):  ./install-vps.sh <caminho-do-binario> [dir-destino]
# Segurança:     só altera a região entre os marcadores do crontab;
#                todo o restante do crontab é preservado byte a byte.
set -euo pipefail

BIN_SRC="${1:?uso: install-vps.sh <caminho-do-binario> [dir-destino]}"
DEST_DIR="${2:-$HOME/ali-coins-rust}"
NODE_DIR="${ALI_COINS_NODE_DIR:-$HOME/ali-coins}"

MARK_BEGIN="# >>> ali-coins-rust (port) >>>"
MARK_END="# <<< ali-coins-rust (port) <<<"

mkdir -p "$DEST_DIR"
install -m 0755 "$BIN_SRC" "$DEST_DIR/ali-coins"

# Reaproveita credenciais/sessão do projeto Node existente (sem sobrescrever).
for file in credentials.env session.json.enc session_meta.json; do
  if [[ -f "$NODE_DIR/$file" && ! -e "$DEST_DIR/$file" ]]; then
    cp -p "$NODE_DIR/$file" "$DEST_DIR/$file"
  fi
done
chmod 600 "$DEST_DIR"/credentials.env "$DEST_DIR"/session.json.enc "$DEST_DIR"/session_meta.json 2>/dev/null || true

# Equivalente local de 08:30 BRT (fallback para crons sem CRON_TZ).
read -r FALLBACK_MIN FALLBACK_HOUR < <(python3 - <<'PY'
from datetime import datetime, date
from zoneinfo import ZoneInfo
import time

sa = ZoneInfo("America/Sao_Paulo")
today = datetime.now(sa).date()
target = datetime(today.year, today.month, today.day, 8, 30, tzinfo=sa)
# Converte para o fuso local da VM (tzlocal: agora com o offset local do sistema).
local = target.astimezone()  # usa o fuso local do processo
print(local.strftime("%M %H"))
PY
)

block=$(cat <<EOF
$MARK_BEGIN
# Horário de Brasília (sem drift de DST): CRON_TZ exige cron com suporte (Debian/Ubuntu ok).
CRON_TZ=America/Sao_Paulo
# Execução unificada (check-in + tarefas). Log append em cron.log.
30 8 * * * cd $DEST_DIR && PATH=$DEST_DIR:\$PATH ./ali-coins checkin --json >> $DEST_DIR/cron.log 2>&1 && PATH=$DEST_DIR:\$PATH ./ali-coins tasks --json >> $DEST_DIR/cron.log 2>&1
# Fallback caso CRON_TZ não seja suportado (remova a linha acima e use esta):
# $FALLBACK_MIN $FALLBACK_HOUR * * * cd $DEST_DIR && ./ali-coins checkin --json >> $DEST_DIR/cron.log 2>&1
$MARK_END
EOF
)

current="$(crontab -l 2>/dev/null || true)"
filtered="$(printf '%s\n' "$current" | sed "\|$MARK_BEGIN|,\|$MARK_END|d")"
printf '%s\n%s\n' "$filtered" "$block" | sed '/^$/N;/^\n$/D' | crontab -

echo "Instalado em: $DEST_DIR"
echo "Binário:      $(du -h "$DEST_DIR/ali-coins" | cut -f1)"
echo "Timezone VM:  $(timedatectl show -p Timezone --value 2>/dev/null || date +%Z)"
echo "Crontab (região do rust):"
crontab -l | sed -n "\|$MARK_BEGIN|,\|$MARK_END|p"
