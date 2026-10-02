# Manual de Instalação — Linux

Guia detalhado para Ubuntu 22.04/24.04 LTS e Debian (VPS ou máquina local).
Para o fluxo em nuvem (sessão/exportação, cron, heartbeat), veja o
[Manual de Execução em Nuvem](CLOUD_SESSIONS.md).

## Sumário

1. [Requisitos mínimos](#1-requisitos-mínimos)
2. [Método A: binário da release (recomendado)](#2-método-a-binário-da-release-recomendado)
3. [Método B: compilar do zero](#3-método-b-compilar-do-zero)
4. [Configuração de credenciais](#4-configuração-de-credenciais)
5. [Primeira execução](#5-primeira-execução)
6. [Resolução de problemas frequentes (troubleshooting)](#6-resolução-de-problemas-frequentes-troubleshooting)
7. [Docker](#7-docker)
8. [Agendamento diário (cron)](#8-agendamento-diário-cron)
9. [Códigos de saída](#9-códigos-de-saída)
10. [Atualização e rollback](#10-atualização-e-rollback)
11. [Referências cruzadas](#11-referências-cruzadas)

## 1. Requisitos mínimos

| Item | Recomendado | Mínimo |
|---|---|---|
| CPU | 2 vCPU | 1 vCPU |
| RAM | 2 GB | 1 GB (com `CHROMIUM_LOW_MEMORY=true`, padrão) |
| Disco | 5 GB livres | 2 GB |
| SO | Ubuntu 24.04 LTS / Debian 12 | Ubuntu 22.04 LTS |
| Chromium | Playwright `chromium-1243` ou sistema | qualquer ≥ 120 |
| Rust (para compilar) | stable (pinada em `rust-toolchain.toml`) | 1.85+ |

O port roda **sem Node.js**. O Node só é necessário para as ferramentas de
paridade (`tools/parity/`, opcional) e para o oráculo em `reference/`.

## 2. Método A: binário da release (recomendado)

```bash
# 1. Baixe o tarball da release (GitHub Releases) e valide o checksum
tar -xzf ali-coins-rust-<versão>-linux-x86_64.tar.gz
sha256sum -c ali-coins-rust-<versão>-linux-x86_64.tar.gz.sha256

# 2. Instale
install -m 755 ali-coins ~/.local/bin/ali-coins    # ou /usr/local/bin

# 3. Chromium (Playwright)
npx playwright install chromium
```

## 3. Método B: compilar do zero

### Passo 1: Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"
rustc --version        # a toolchain é fixada por rust-toolchain.toml
```

### Passo 2: Clonar e compilar

```bash
git clone <repo> ali-coins-rust && cd ali-coins-rust
cargo build --release -p ali-coins-cli      # ~1-5 min (deps cacheadas)
install -m 755 target/release/ali-coins ~/.local/bin/ali-coins   # opcional
```

> Em VPS de 1 vCPU/~1 GB, compile com:
> `CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 cargo build --release -p ali-coins-cli`.
> O perfil já reduz `codegen-units` do crate gerado do CDP (veja `Cargo.toml`).

### Passo 3: Chromium e dependências nativas

```bash
# Opção 1 (recomendada): Chromium do Playwright
npx playwright install chromium
npx playwright install-deps chromium        # requer sudo (libs do sistema)

# Opção 2: Chromium do sistema
sudo apt-get install -y chromium
export ALI_COINS_CHROME="$(command -v chromium)"
```

O binário procura o Chromium em `ALI_COINS_CHROME`; sem a variável, usa o cache
do Playwright (`~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome`).

### Passo 4: Validar o Chromium

```bash
"${ALI_COINS_CHROME:-$HOME/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome}" --version
```

Se faltar alguma lib (ex.: `libnspr4.so`), veja
[6.A Libs do Chromium ausentes](#a-libs-do-chromium-ausentes-libnspr4-libnss3-libasound).

## 4. Configuração de Credenciais

```bash
cd ali-coins-rust
cp credentials.env.example credentials.env
chmod 600 credentials.env
```

```bash
ALI_USER="seu_usuario@exemplo.com"
ALI_PASSWORD="sua_senha"
SESSION_SECRET="$(openssl rand -base64 32)"   # 32+ caracteres

# Opcional (multi-conta)
# ALI_USER_2="segunda_conta@exemplo.com"
# ALI_PASSWORD_2="senha_segunda_conta"

# Opcional (Telegram) — veja TELEGRAM.md
# TELEGRAM_ENABLED=true
# TELEGRAM_BOT_TOKEN="123:ABC..."
# TELEGRAM_CHAT_ID="123456789"
```

Segurança:

- o arquivo **nunca** deve ser versionado (já consta no `.gitignore`);
- mantenha modo `0600`;
- `SESSION_SECRET` protege a sessão at-rest e o token exportado — guarde-o com cuidado;
- webhooks/heartbeat para destinos privados exigem `ALLOW_PRIVATE_WEBHOOKS=true`
  (proteção SSRF).

Lista completa de variáveis e defaults: contrato C-04 em
[`docs/04-matriz-rastreabilidade.md`](../04-matriz-rastreabilidade.md) e o exemplo
comentado no [README](../../README.md#credenciais-credentialsenv).

## 5. Primeira execução

```bash
# 1. Validação sem navegador (config + credenciais)
ali-coins --dry-run --json

# 2. Check-in (abre o navegador, faz login/sessão e coleta)
ali-coins checkin --json

# 3. Tarefas (painel "Ganhe mais moedas")
ali-coins tasks --json

# 4. Fluxo unificado (recomendado para produção)
ali-coins all --json
./wrappers/run_all.sh --json          # com retentativa única em exit 1
```

Na primeira execução com login, o 2FA/captcha pode exigir interação: rode
manualmente numa máquina local e depois **exporte a sessão** para a VPS
([CLOUD_SESSIONS.md](CLOUD_SESSIONS.md#3-delegação-de-sessão-gerar-local--exportar-para-nuvem)).

## 6. Resolução de Problemas Frequentes (Troubleshooting)

### A. Libs do Chromium ausentes (`libnspr4`, `libnss3`, `libasound`)

Sintoma: `error while loading shared libraries: libnspr4.so: cannot open shared object file`.

```bash
# Com sudo (recomendado)
sudo npx playwright install-deps chromium

# Sem sudo (extrai os .deb num diretório local e usa LD_LIBRARY_PATH)
mkdir -p ~/.local/chromium-libs && cd ~/.local/chromium-libs
apt-get download libnspr4 libnss3 libasound2t64
for d in *.deb; do dpkg-deb -x "$d" .; done
export LD_LIBRARY_PATH="$HOME/.local/chromium-libs/usr/lib/x86_64-linux-gnu:$LD_LIBRARY_PATH"
```

### B. Sandbox do Chromium bloqueado (Ubuntu 23.10+/AppArmor)

Sintoma: o Chromium não inicia em VPS/container. O AppArmor do Ubuntu restringe
*user namespaces*. Soluções (em ordem):

1. `NO_SANDBOX=true` no `credentials.env` (é o que a imagem Docker já usa);
2. perfil AppArmor dedicado para o binário do Chromium;
3. `sysctl kernel.apparmor_restrict_unprivileged_userns=0` (ajuste global do host).

### C. Captcha ou bloqueio no login em nuvem

IPs de datacenter (Oracle/AWS/GCP) costumam receber captcha no login. **Não** force
login na VPS: gere a sessão na sua rede residencial e importe-a
([CLOUD_SESSIONS.md](CLOUD_SESSIONS.md#1-o-desafio-de-ips-de-datacenter-anti-bot)).
O port já pausa novas tentativas por `CAPTCHA_COOLDOWN_HOURS` (12 h, replicado do oráculo).

### D. VPS com pouca RAM (512 MB–1 GB)

- mantênha `CHROMIUM_LOW_MEMORY=true` (padrão) e `ALLOW_MEDIA=false`;
- evite `PW_VIDEO`; trace/screenshot ficam em `scratch/` (0600);
- prefira a **imagem de runtime** + binário pré-compilado
  ([7. Docker](#7-docker)) em vez de compilar no container;
- se o Chromium morrer por OOM, reduza `CHROMIUM_JS_HEAP_MB` (ex.: 128).

### E. "Gaveta de tarefas não encontrada" / painel fechado

Sintomas nos logs: `painel de tarefas fechado ou não detectado` ou
`Gaveta de tarefas não encontrada`. O runner já tenta recarregar a central e
recriar a página; para investigar, o diagnóstico grava URL + trecho do corpo +
screenshot `0600` em `scratch/` (`tasks-drawer-falha-*.png`). Se persistir:

- confirme que a conta não está bloqueada/sem a tarefa disponível;
- tente `PW_EMIT_TOUCH` **não** definir (a conversão mouse→toque é desligada por
  padrão, igual ao Playwright);
- rode `ali-coins tasks --json` manualmente e observe o painel.

### F. Cache de navegadores (`PLAYWRIGHT_BROWSERS_PATH`)

O Playwright instala em `~/.cache/ms-playwright`. Para mudar:

```bash
export PLAYWRIGHT_BROWSERS_PATH="$HOME/pw-browsers"
npx playwright install chromium
export ALI_COINS_CHROME="$PLAYWRIGHT_BROWSERS_PATH/chromium-1243/chrome-linux64/chrome"
```

### G. 2FA/captcha em execução automática (cron/CI)

- **2FA não-interativo** encerra com `exit 5` e notifica `2fa_required`;
- **captcha** encerra com `exit 1` e notifica `captcha_required`; novas tentativas
  ficam pausadas por 12 h (`CAPTCHA_COOLDOWN_HOURS`, replicado do oráculo);
- solução: resolver localmente, reexportar a sessão e importar na VPS
  ([TELEGRAM.md](TELEGRAM.md#aviso-de-sessão-remota-expirada)).

### H. Lock ativo (`exit 3`)

Outra execução está em andamento (ou o lock ficou stale). O lock expira em
`LOCK_STALE_TIMEOUT_MS` (padrão 30 min); `--force` remove lock stale. Em
multi-conta, cada conta tem o próprio lock.

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
     --entrypoint /data/ali-coins ali-coins-rust:latest all --json
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

## 8. Agendamento Diário (cron)

```bash
# Editar com `crontab -e` (08:30 America/Sao_Paulo = 11:30 UTC na VPS)
30 11 * * * /home/ubuntu/ali-coins-rust/run_all.sh --json >> /home/ubuntu/ali-coins-rust/cron.log 2>&1
```

Passo a passo completo (incluindo o clássico "cron não encontra o binário/PATH",
rotação de logs e heartbeat): [CLOUD_SESSIONS.md](CLOUD_SESSIONS.md#8-agendamento-no-cron).

## 9. Códigos de Saída

| Código | Significado | Ação recomendada no cron |
|---|---|---|
| `0` | Sucesso | — |
| `1` | Falha de execução | alerta (o wrapper tenta 1 vez) |
| `2` | Sem ação / já coletado | sucesso silencioso |
| `3` | Lock ativo | ignorar (outra execução) |
| `4` | Streak quebrado | alerta dedicado |
| `5` | 2FA não-interativo | alerta + reexportar sessão |
| `6` | Crash | alerta + investigar `scratch/` |

## 10. Atualização e Rollback

```bash
# Atualizar (fonte)
git pull && cargo build --release -p ali-coins-cli
install -m 755 target/release/ali-coins ~/.local/bin/ali-coins
# (ou, na VPS com Docker) reconstrua a imagem de runtime:
wrappers/build-runtime-image.sh

# Rollback
# 1. mantenha o binário anterior (ex.: ali-coins.bak-<data>);
# 2. restaure o arquivo e/ou a tag da imagem (docker tag);
# 3. sessões e formato de token permanecem compatíveis (ADR-0004).
```

## 11. Referências cruzadas

- [Índice dos manuais](README.md)
- [Instalação no macOS](INSTALL_MACOS.md)
- [Instalação no Windows](INSTALL_WINDOWS.md)
- [Execução em nuvem / sessões](CLOUD_SESSIONS.md)
- [Telegram](TELEGRAM.md)
- [Releases](RELEASING.md)
- [Divergências conhecidas](../05-divergencias-conhecidas.md)
- [Validação real](../06-validacao-real.md)
