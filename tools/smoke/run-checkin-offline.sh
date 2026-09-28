#!/usr/bin/env bash
# Smoke offline do subcomando `checkin`: servidor local + sessão fake + Chromium real.
# Não toca o AliExpress (ALI_COINS_COIN_URL aponta para 127.0.0.1).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="$ROOT/target/debug/ali-coins"
PORT="${CHECKIN_SMOKE_PORT:-38471}"

export PATH="$HOME/.cargo/bin:${PATH}"
LOCAL_LIBS="$HOME/.local/share/ali-coins-chromium-libs/usr/lib/x86_64-linux-gnu"
[[ -d "$LOCAL_LIBS" ]] && export LD_LIBRARY_PATH="$LOCAL_LIBS:${LD_LIBRARY_PATH:-}"
if [[ -z "${ALI_COINS_CHROME:-}" ]]; then
  CHROME="$(find "$HOME/.cache/ms-playwright" -maxdepth 3 -path '*chrome-linux64/chrome' -type f 2>/dev/null | head -1 || true)"
  [[ -n "$CHROME" ]] && export ALI_COINS_CHROME="$CHROME"
fi
export NO_SANDBOX="${NO_SANDBOX:-true}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"; [[ -n "${SERVER_PID:-}" ]] && kill "$SERVER_PID" 2>/dev/null || true' EXIT

# Servidor local com a página "já coletado hoje".
PORT="$PORT" python3 - <<'PY' &
import os
from http.server import BaseHTTPRequestHandler, HTTPServer

HTML = (
    "<html><body class='today-checked'>"
    "Minhas moedas 1.234 Sequ\u00eancia de 42 dias"
    "</body></html>"
).encode("utf-8")


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(HTML)))
        self.end_headers()
        self.wfile.write(HTML)

    def log_message(self, *args):
        pass


HTTPServer(("127.0.0.1", int(os.environ["PORT"])), Handler).serve_forever()
PY
SERVER_PID=$!
sleep 1

cat > "$tmp/credentials.env" <<EOF
ALI_USER="smoke@example.com"
ALI_PASSWORD="smoke"
ENCRYPT_LOCAL_SESSION=false
EOF
chmod 600 "$tmp/credentials.env"

cat > "$tmp/session.json" <<'EOF'
{"cookies":[{"name":"xman_us_t","value":"fake","domain":"127.0.0.1","path":"/"}],"origins":[]}
EOF
cat > "$tmp/session_meta.json" <<'EOF'
{"user":"smoke@example.com","savedAt":"2026-09-28T00:00:00.000Z","encrypted":false}
EOF
chmod 600 "$tmp/session.json" "$tmp/session_meta.json"

cd "$tmp"
set +e
ALI_COINS_COIN_URL="http://127.0.0.1:$PORT/" "$BIN" checkin --json >"$tmp/out.json" 2>"$tmp/err.log"
CODE=$?
set -e

echo "exit=$CODE"
python3 - "$tmp/out.json" <<'PY'
import json, sys
payload = json.load(open(sys.argv[1]))
assert payload["type"] == "unified_report", payload
assert payload["checkin"]["alreadyCollected"] is True, payload["checkin"]
assert payload["checkin"]["streakDays"] == 42, payload["checkin"]
assert payload["checkin"]["totalBalance"] == "1.234", payload["checkin"]
print("relatorio OK: já coletado, streak 42, saldo 1.234")
PY

if [[ "$CODE" -ne 2 ]]; then
  echo "ERRO: exit esperado 2 (já coletado), obtido $CODE" >&2
  tail -5 "$tmp/err.log" >&2
  exit 1
fi
echo "Smoke offline do checkin OK."
