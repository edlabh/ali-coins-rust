# Manual de Instalação — macOS

Guia para macOS em **Apple Silicon (arm64)** e **Intel (x86_64)**.
Para Linux/VPS, veja [INSTALL_LINUX.md](INSTALL_LINUX.md); para sessões e nuvem,
[CLOUD_SESSIONS.md](CLOUD_SESSIONS.md).

## Sumário

1. [Requisitos mínimos](#1-requisitos-mínimos)
2. [Método A: binário da release (recomendado)](#2-método-a-binário-da-release-recomendado)
3. [Método B: compilar do zero](#3-método-b-compilar-do-zero)
4. [Configuração de credenciais](#4-configuração-de-credenciais)
5. [Primeira execução](#5-primeira-execução)
6. [Agendamento automático diário no macOS](#6-agendamento-automático-diário-no-macos)
7. [Resolução de problemas frequentes no macOS](#7-resolução-de-problemas-frequentes-no-macos)
8. [Variáveis de ambiente (opcional)](#8-variáveis-de-ambiente-opcional)
9. [Atualização e desinstalação](#9-atualização-e-desinstalação)
10. [Referências cruzadas](#10-referências-cruzadas)

## 1. Requisitos mínimos

| Item | Recomendado | Mínimo |
|---|---|---|
| macOS | 13+ | 11 (Big Sur) |
| RAM | 8 GB | 4 GB |
| Disco | 2 GB livres | 1 GB |
| Chromium | Playwright `chromium-*` ou Chrome/Edge | qualquer ≥ 120 |
| Rust (para compilar) | stable (pinada em `rust-toolchain.toml`) | 1.85+ |

> O port roda **sem Node.js**. O Node só é necessário se você optar por instalar o
> Chromium via `npx playwright install`.

## 2. Método A: binário da release (recomendado)

```bash
# Apple Silicon → macos-aarch64 | Intel → macos-x86_64
tar -xzf ali-coins-rust-<versão>-macos-aarch64.tar.gz
shasum -a 256 -c ali-coins-rust-<versão>-macos-aarch64.tar.gz.sha256

install -m 755 ali-coins ~/.local/bin/ali-coins     # ou /usr/local/bin
# Se o Gatekeeper reclamar: xattr -d com.apple.quarantine ./ali-coins
```

O tarball já inclui o binário universal? Não: baixe o artefato da sua arquitetura
(`aarch64` para Apple Silicon, `x86_64` para Intel).

## 3. Método B: compilar do zero

### Passo 1: Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"
rustc --version
```

### Passo 2: Clonar e compilar

```bash
git clone <repo> ali-coins-rust && cd ali-coins-rust
cargo build --release -p ali-coins-cli      # binário: target/release/ali-coins
```

### Passo 3: Chromium e dependências

```bash
# Opção 1 (recomendada): Chromium do Playwright
npx playwright install chromium

# Opção 2: Chrome/Edge instalados no sistema
export ALI_COINS_CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
```

Sem `ALI_COINS_CHROME`, o port procura automaticamente o cache do Playwright em
`~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/Chromium.app/Contents/MacOS/Chromium`.

### Passo 4: Validar o Chromium

```bash
"${ALI_COINS_CHROME:-$HOME/Library/Caches/ms-playwright/chromium-1243/chrome-mac-arm64/Chromium.app/Contents/MacOS/Chromium}" --version
```

## 4. Configuração de Credenciais

```bash
cd ali-coins-rust
cp credentials.env.example credentials.env
chmod 600 credentials.env
# Gere o segredo com OpenSSL (>= 32 caracteres)
openssl rand -base64 32
```

Edite o `credentials.env` com `ALI_USER`, `ALI_PASSWORD` e `SESSION_SECRET`.
Variáveis opcionais (Telegram, multi-conta, tarefas, diagnósticos): veja o
[README](../../README.md#credenciais-credentialsenv).

## 5. Primeira execução

```bash
./ali-coins --dry-run --json
./ali-coins checkin --json
./ali-coins all --json
./wrappers/run_all.sh --json
```

## 6. Agendamento Automático Diário no macOS

### Opção 1: cron (mais simples)

```bash
crontab -e
```

```cron
# 08:30 no fuso local — o caminho precisa ser absoluto (o cron tem PATH mínimo)
30 8 * * * cd "$HOME/ali-coins-rust" && ./wrappers/run_all.sh --json >> cron.log 2>&1
```

> No macOS, o cron pode exigir permissão de Acesso Total ao Disco para o binário
> (Ajustes do Sistema → Privacidade e Segurança).

### Opção 2: launchd (nativo e recomendado)

Crie `~/Library/LaunchAgents/com.ali-coins.rust.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.ali-coins.rust</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/SEU_USUARIO/ali-coins-rust/wrappers/run_all.sh</string>
    <string>--json</string>
  </array>
  <key>WorkingDirectory</key><string>/Users/SEU_USUARIO/ali-coins-rust</string>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>8</integer><key>Minute</key><integer>30</integer></dict>
  <key>StandardOutPath</key><string>/Users/SEU_USUARIO/ali-coins-rust/cron.log</string>
  <key>StandardErrorPath</key><string>/Users/SEU_USUARIO/ali-coins-rust/cron.log</string>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/com.ali-coins.rust.plist
launchctl list | grep ali-coins       # confirma o agendamento
```

## 7. Resolução de Problemas Frequentes no macOS

### A. `command not found` (cargo/ali-coins)

O `zsh` não carrega o `~/.cargo/env` automaticamente em scripts. Use caminho
absoluto nos agendamentos e, na sessão, `source "$HOME/.cargo/env"`.

### B. `permission denied: ./run_all.sh`

```bash
chmod +x wrappers/*.sh
```

### C. Gatekeeper / "desenvolvedor não identificado"

```bash
xattr -d com.apple.quarantine ./ali-coins        # após baixar o binário
```

### D. Macs com pouca RAM (VMs/4 GB)

- mantenha `CHROMIUM_LOW_MEMORY=true` (padrão) e `ALLOW_MEDIA=false`;
- `CHROMIUM_JS_HEAP_MB=128` se o Chromium morrer por memória;
- evite rodar mais de uma conta ao mesmo tempo em máquinas pequenas.

### E. Chromium não encontrado / bibliotecas

- defina `ALI_COINS_CHROME` para o binário completo
  (`.../Chromium.app/Contents/MacOS/Chromium` ou Google Chrome);
- se instalou via Playwright, confira o cache em `~/Library/Caches/ms-playwright`;
- não é preciso instalar libs do sistema (o app é assinado pela Playwright).

## 8. Variáveis de Ambiente (opcional)

O port lê tudo do `credentials.env`; variáveis do shell vencem quando definidas.

```bash
export ALI_COINS_CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
export CHROMIUM_LOW_MEMORY=false
```

Para persistir no `zsh` (padrão do macOS), adicione os `export` em `~/.zshrc`.

## 9. Atualização e Desinstalação

### Atualizar

1. Baixe o novo tarball (mesma plataforma) e valide o `.sha256`;
2. substitua o binário:
   ```bash
   install -m 755 ali-coins ~/.local/bin/ali-coins
   ```
3. Se usar launchd, **nada muda** (o plist aponta para o wrapper, não para uma
   versão fixa do binário).

### Desinstalar

```bash
launchctl unload ~/Library/LaunchAgents/com.ali-coins.rust.plist 2>/dev/null
rm -f ~/Library/LaunchAgents/com.ali-coins.rust.plist
rm -f ~/.local/bin/ali-coins
rm -rf ~/ali-coins-rust                  # sessão/segredos: apague com cuidado
rm -rf ~/Library/Caches/ms-playwright    # opcional (Chromium do Playwright)
```

> A sessão (`session.json.enc`), os backups e o `credentials.env` ficam **fora**
> do binário; remova-os apenas se quiser apagar as credenciais de vez.

## 10. Referências Cruzadas

- [Índice dos manuais](README.md)
- [Instalação Linux](INSTALL_LINUX.md)
- [Execução em nuvem / sessões](CLOUD_SESSIONS.md#3-delegação-de-sessão-gerar-local--exportar-para-nuvem)
- [Telegram](TELEGRAM.md)
- [Releases](RELEASING.md)
