# Manual de Instalação — Linux

Port Rust do `ali-coins` (Node.js + Playwright). Requer **Linux x86_64** (Ubuntu 22.04+/Debian 12+).

## 1. Pré-requisitos

- Rust **1.85+** (edition 2024) — instale com [rustup](https://rustup.rs):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
  ```
- **Chromium** e bibliotecas de sistema. Duas opções:
  - **Com sudo:** `npx playwright install-deps chromium` e depois aponte `ALI_COINS_CHROME` para o binário do Playwright (`~/.cache/ms-playwright/chromium-*/chrome-linux/chrome`);
  - **Sem sudo:** baixe e extraia apenas as libs que faltarem:
    ```bash
    mkdir -p ~/.local/share/ali-coins-chromium-libs && cd /tmp
    apt-get download libnspr4 libnss3 libasound2t64
    for d in *.deb; do dpkg-deb -x "$d" ~/.local/share/ali-coins-chromium-libs; done
    export LD_LIBRARY_PATH="$HOME/.local/share/ali-coins-chromium-libs/usr/lib/x86_64-linux-gnu:$LD_LIBRARY_PATH"
    ```

## 2. Compilar

```bash
git clone <repo> ali-coins-rust && cd ali-coins-rust
cargo build --release -p ali-coins-cli
# binário: target/release/ali-coins
```

> ⚠️ Em hosts com poucos recursos (1 vCPU / ~1 GB de RAM) a compilação completa pode
> levar **horas** e ser interrompida por falta de memória. Nesses casos, prefira
> **compilar em uma máquina de desenvolvimento** e copiar o binário pronto para o
> servidor — o uso direto do binário deve ser priorizado, pelo menos no início.
>
> Se for compilar no próprio host: `CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 cargo build --release -p ali-coins-cli`,
> com swap extra (2–4 GB).

## 3. Credenciais

```bash
cp credentials.env.example credentials.env   # se disponível; senão crie
chmod 600 credentials.env
```

Conteúdo mínimo:

```env
ALI_USER="seu_email@exemplo.com"
ALI_PASSWORD="sua_senha"
SESSION_SECRET="<32+ caracteres aleatórios>"
```

- Gere o segredo com `openssl rand -base64 32` (ou `head -c 32 /dev/urandom | base64`).
- Opcionais: `ENCRYPT_LOCAL_SESSION`, `HEADLESS`, `NAV_TIMEOUT`, `SELECTOR_TIMEOUT`,
  `ELEMENT_TIMEOUT`, `SCROLL_WAIT_SECONDS`, `TASK_MAX_ACTIONS`, `TASK_MAX_ATTEMPTS`,
  `LOCK_STALE_TIMEOUT_MS`, `NOTIFY_HOST_LABEL`, `TELEGRAM_*`, `HEARTBEAT_*`.
- O arquivo é carregado do diretório atual (rode sempre de dentro do projeto).

## 4. Primeira execução

```bash
./target/release/ali-coins --dry-run --json   # valida config sem abrir navegador
ALI_COINS_CHROME=/caminho/chrome ./target/release/ali-coins checkin --json
```

No primeiro login o app pode pedir 2FA; em ambiente interativo o fluxo orienta, em cron ele **falha rápido com exit 5** (use export/import de sessão — veja `CLOUD_SESSIONS.md`).

## 5. Wrappers

| Script | Ação |
|---|---|
| `wrappers/run_checkin.sh` | check-in diário |
| `wrappers/run_tasks.sh` | tarefas diárias |
| `wrappers/run_all.sh` | check-in + tarefas (retenta exit 1 uma vez) |

## 6. Códigos de saída

`0` sucesso · `1` falha · `2` sem ação/já coletado · `3` lock ativo ·
`4` streak quebrado · `5` 2FA não-interativo · `6` crash

## 7. Docker

Há **dois** caminhos, para não recompilar o workspace a cada atualização:

**VPS com poucos recursos (recomendado): imagem de runtime + binário pré-compilado.**

1. Compile o binário uma vez (o `target/` fica incremental):
   ```bash
   cargo build --release -p ali-coins-cli
   ```
2. Gere a imagem de runtime (segundos — só copia o binário e reaproveita a
   camada do apt):
   ```bash
   wrappers/build-runtime-image.sh          # vira ali-coins-rust:latest
   ```
3. Rode:
   ```bash
   docker run --rm --init --shm-size=256m --memory=768m --memory-swap=1536m \
     -v "$PWD:/data" -w /data -v "$HOME/.cache/ms-playwright:/pw:ro" \
     -e ALI_COINS_CHROME=/pw/chromium-1193/chrome-linux/chrome -e NO_SANDBOX=true \
     ali-coins-rust:latest checkin --json
   ```

**Imagem completa (`Dockerfile`)**: usada pelo CI e quando se quer construir tudo
em container. O build já aplica `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16` e
`CARGO_PROFILE_RELEASE_LTO=false` (o perfil do workspace usa `codegen-units = 1`
+ LTO thin, que em 1 vCPU multiplica o tempo) e usa **cache do BuildKit** para
`registry`/`git`/`target` — a segunda build é incremental.

```bash
DOCKER_BUILDKIT=1 docker build -t ali-coins-rust:full .
docker run --rm --init --shm-size=256m --memory=768m --memory-swap=1536m \
  -v "$PWD:/data" -w /data \
  -e ALI_COINS_CHROME=/caminho/no/container/chrome -e NO_SANDBOX=true \
  ali-coins-rust:full checkin --json
```

> Em hosts com 1 vCPU/~1 GB, evite o build completo (pode levar dezenas de
> minutos e ser morto por falta de memória). Use `Dockerfile.runtime`.

> Windows/macOS ainda **não** são suportados pelo port (decisão Linux-first no roadmap).
