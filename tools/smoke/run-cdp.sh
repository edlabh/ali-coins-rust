#!/usr/bin/env bash
# Roda o smoke real do CdpDriver (Chromium de verdade + emulação Pixel 7).
#
# Requisitos:
#   1. Chromium: usa $ALI_COINS_CHROME ou o cache do Playwright
#      (~/.cache/ms-playwright/**/chrome-linux64/chrome).
#   2. Libs de sistema: em hosts sem sudo, extraia os .deb para um diretório local:
#        mkdir -p /tmp/libs && cd /tmp/libs
#        apt-get download libnspr4 libnss3 libasound2t64
#        for d in *.deb; do dpkg-deb -x "$d" "$HOME/.local/share/ali-coins-chromium-libs"; done
#      (com sudo: `npx playwright install-deps chromium`)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export PATH="$HOME/.cargo/bin:${PATH}"

LOCAL_LIBS="$HOME/.local/share/ali-coins-chromium-libs/usr/lib/x86_64-linux-gnu"
if [[ -d "$LOCAL_LIBS" ]]; then
  export LD_LIBRARY_PATH="$LOCAL_LIBS:${LD_LIBRARY_PATH:-}"
fi

if [[ -z "${ALI_COINS_CHROME:-}" ]]; then
  CHROME="$(find "$HOME/.cache/ms-playwright" -maxdepth 3 -path '*chrome-linux64/chrome' -type f 2>/dev/null | head -1 || true)"
  if [[ -n "$CHROME" ]]; then
    export ALI_COINS_CHROME="$CHROME"
  fi
fi

if [[ -z "${ALI_COINS_CHROME:-}" ]]; then
  echo "ERRO: defina ALI_COINS_CHROME ou instale um Chromium no cache do Playwright." >&2
  exit 1
fi

export NO_SANDBOX="${NO_SANDBOX:-true}"

cd "$ROOT"
echo "Chromium: $ALI_COINS_CHROME"
cargo test -p ali-coins-browser --test cdp_smoke -- --ignored --nocapture
