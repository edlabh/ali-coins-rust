# ali-coins-rust

Port em Rust do [ali-coins](https://github.com/edlabh/ali-coins) (Node.js + Playwright):
automação de check-in diário e tarefas de moedas do AliExpress com emulação mobile
(Google Pixel 7) via CDP (`chromiumoxide`).

O projeto Node original é mantido em `reference/ali-coins/` como **oráculo de
paridade** (não versionado) e o comportamento é validado contra ele por fixtures
geradas em `tools/parity/`.

## Recursos Principais

- **Check-in diário** com leitura do extrato desktop (saldo/bônus/missões), coleta
  de água, sincronização de ledger e resolução de streak (inclusive quebra).
- **Tarefas** do painel "Ganhe mais moedas": busca por palavra-chave, Prize Land,
  navegação com scroll, surpresa ("toque em 3 itens") e verificação de conclusão.
- **Sessão persistente** com criptografia at-rest (v1/v2/v3), exportação/importação
  de token, rotação de chave e pruning de backups.
- **Multi-conta sequencial** (até N contas) com lock por conta, pausas, backoff e
  relatório/notificação consolidados.
- **Notificações**: Telegram (todos os eventos + mensagem consolidada multi-conta)
  e webhook genérico; heartbeat (*dead man's switch*) opcional.
- **Diagnósticos**: screenshot em falha, hash/dump de DOM e trace CDP em `scratch/`.
- **Wrappers prontos para cron** (`wrappers/run_*.sh`) com exit codes do oráculo.
- **Docker**: imagem completa (CI) e imagem de runtime enxuta para VPS de 1 vCPU.

## Modos de Execução

| Comando | O que faz | Notificação |
|---|---|---|
| `ali-coins all` | Check-in + tarefas numa execução (relatório/notificação únicos) | `success`, `already_collected`, `streak_break`, `failure`, `2fa_required`, `captcha_required` |
| `ali-coins checkin` | Apenas o check-in diário | relatório do check-in |
| `ali-coins tasks` | Apenas o painel "Ganhe mais moedas" | relatório das tarefas |
| `ali-coins all --all` | Todas as contas em sequência (multi-conta) | consolidada multi-conta |
| `ali-coins --dry-run` | Valida configuração/credenciais sem abrir navegador | `dry_run` (com `--notify`) |
| `ali-coins notify-test` | Envia mensagem de teste no Telegram | `manual_test` |

## Opções de Linha de Comando (CLI)

| Flag | Descrição |
|---|---|
| `-d, --dry-run` | Valida config/ambiente sem navegador (também aceito após subcomandos) |
| `--json` | Relatório em JSON no stdout (contrato C-09) |
| `--notify` / `--no-notify` | Força ligar/desligar a notificação (o último vence) |
| `--heartbeat` / `--no-heartbeat` | Força ligar/desligar o heartbeat (o último vence) |
| `-f, --force` | Ignora o cooldown pós-captcha e o lock stale |
| `--no-delay` | Pula o atraso inicial aleatório (`START_DELAY_*`) |
| `--account <id>` | Seleciona a conta por índice (1-based) ou usuário |
| `--all` | Executa todas as contas (multi-conta sequencial) |
| `-h, --help` / `-V, --version` | Ajuda / versão |

> `export-session` e `import-session` têm flags próprias — veja a seção
> [Sessões](#sessões-exportação-e-importação) e o
> [manual de nuvem](docs/manual/CLOUD_SESSIONS.md#3-delegação-de-sessão-gerar-local--exportar-para-nuvem).

## Códigos de Saída (Exit Codes)

| Código | Significado |
|---|---|
| `0` | Sucesso |
| `1` | Falha de execução |
| `2` | Sem ação / já coletado |
| `3` | Lock ativo (outra instância) |
| `4` | Streak quebrado (alerta) |
| `5` | 2FA não-interativo (cron/CI) |
| `6` | Crash (panic handler) |

O cron deve tratar `0`/`2` como sucesso; `4`/`5` geram alerta dedicado no Telegram.
Veja [`## Códigos de saída`](docs/manual/CLOUD_SESSIONS.md#11-códigos-de-saída-e-alertas)
para o detalhamento.

## Início Rápido (3 Passos)

### 1. Compilar (ou baixar o binário)

```bash
# Opção A — compilar do zero (Rust stable; ver docs/manual/INSTALL_LINUX.md)
cargo build --release -p ali-coins-cli      # binário: target/release/ali-coins

# Opção B — baixar o tarball da release (GitHub Releases)
# ali-coins-rust-<versão>-linux-x86_64.tar.gz + .sha256
```

### 2. Configurar credenciais

```bash
cp credentials.env.example credentials.env
chmod 600 credentials.env                    # ALI_USER / ALI_PASSWORD / SESSION_SECRET
```

### 3. Validar e executar

```bash
ali-coins --dry-run --json                   # valida sem navegador
ali-coins all                                # check-in + tarefas (recomendado)
./wrappers/run_all.sh                        # execução unificada com retentativa (cron)
```

## Credenciais (`credentials.env`)

O arquivo é lido do diretório atual (nunca versionado; modo `0600`). Exemplo
completo com as principais opções:

```bash
# --- Conta principal (obrigatório) ---
ALI_USER="seu_usuario@exemplo.com"
ALI_PASSWORD="sua_senha"
SESSION_SECRET="<32+ caracteres>"            # openssl rand -base64 32

# --- Multi-conta (opcional; até N contas) ---
# ALI_USER_2="segunda_conta@exemplo.com"
# ALI_PASSWORD_2="senha_segunda_conta"

# --- Sessão ---
# ENCRYPT_LOCAL_SESSION=true                 # criptografia at-rest (padrão true)
# SESSION_BACKUP_RETENTION_DAYS=7            # retenção de backups
# SESSION_STRICT_STORAGE=false               # allowlist rígida de chaves do localStorage

# --- Notificações ---
# TELEGRAM_ENABLED=true
# TELEGRAM_BOT_TOKEN="123:ABC..."
# TELEGRAM_CHAT_ID="123456789"
# TELEGRAM_CHAT_ID_2="987654321"             # chat por conta (opcional)
# TELEGRAM_SILENT=false
# TELEGRAM_PER_ACCOUNT=false
# NOTIFY_HOST_LABEL="vm-ali-rust"            # rótulo do host nas mensagens
# NOTIFY_WEBHOOK_URL="https://discord.com/api/webhooks/..."   # webhook genérico
# HEARTBEAT_URL="https://hc-ping.com/<uuid>" # dead man's switch (opcional)

# --- Rede/segurança ---
# ALLOW_PRIVATE_WEBHOOKS=true                # destrava loopback/privados (testes locais)

# --- Browser ---
# ALI_COINS_CHROME="/caminho/para/chrome"    # binário do Chromium (ou use o do Playwright)
# HEADLESS=true
# NO_SANDBOX=true                            # necessário em container/root
# CHROMIUM_LOW_MEMORY=true                   # economia de RAM (padrão true)
# CHROMIUM_JS_HEAP_MB=192                    # teto do heap JS (low-memory)
# ALLOW_MEDIA=false                          # bloqueia imagens/mídia

# --- Tarefas ---
# SKIP_APP_ONLY_TASKS=true                   # ignora Prize Land/minigames/quizzes/avaliações
# TASK_MAX_ACTIONS=25
# TASK_MAX_ATTEMPTS=4
# TASK_MAX_DURATION_MS=180000
# SCROLL_WAIT_SECONDS=10
# TASK_RETRY_UNFINISHED=false                # 2ª passada só nas incompletas
# TASK_PAUSE_MIN_MS=0 / TASK_PAUSE_MAX_MS=0  # pausa aleatória entre tarefas

# --- Multi-conta / anti-detecção ---
# ACCOUNT_DELAY_MIN_MS=0 / ACCOUNT_DELAY_MAX_MS=0
# ACCOUNT_BACKOFF_BASE_MS=30000
# START_DELAY_MIN_MS=0 / START_DELAY_MAX_MS=0   # atraso inicial (--no-delay pula)

# --- Diagnósticos ---
# PW_SCREENSHOT=only-on-failure              # off | on | only-on-failure
# PW_TRACE=off                               # off | on | retain-on-failure
# PW_OUTPUT_DIR=scratch
# PW_VIDEO=off
```

> Lista completa e defaults: `docs/04-matriz-rastreabilidade.md` (contrato C-04) e
> [`docs/manual/INSTALL_LINUX.md#4-configuração-de-credenciais`](docs/manual/INSTALL_LINUX.md#4-configuração-de-credenciais).

## Execução Multi-Conta

```bash
ali-coins all --all                 # todas as contas em sequência
ali-coins all --account 2           # apenas a conta de índice 2
ali-coins all --account user3@exemplo.com
```

Cada conta roda em um processo filho isolado (`all --account <user> --json`), com
lock por conta, pausa/backoff entre contas e notificação consolidada. Contas sem
`TELEGRAM_CHAT_ID_<n>` herdam o chat principal. Detalhes e boas práticas em
[`docs/manual/CLOUD_SESSIONS.md#6-troca-de-contas-na-nuvem`](docs/manual/CLOUD_SESSIONS.md#6-troca-de-contas-na-nuvem).

## Sessões (exportação e importação)

```bash
# Exportar no PC (rede residencial) após um login válido
ali-coins export-session --show-token > session_token.txt

# Importar na nuvem (token cifrado no stdin)
ali-coins import-session < session_token.txt

# Utilitários
ali-coins export-session --all --json                # todas as contas
ali-coins export-session --rotate --new-secret-from-env=SESSION_SECRET_NEW
ali-coins import-session --from-file token.txt       # arquivo em vez de stdin
ali-coins import-session --plaintext                 # sem criptografia at-rest
ali-coins import-session --keep-tokens               # não remove o token após importar
ali-coins import-session --migrate                   # converte session.json legado em .enc
```

> Fluxo completo (por que o login falha em VPS e como delegar a sessão):
> [`docs/manual/CLOUD_SESSIONS.md`](docs/manual/CLOUD_SESSIONS.md).

## Notificações

- **Telegram**: criação do bot, `CHAT_ID`, eventos e troubleshooting em
  [`docs/manual/TELEGRAM.md`](docs/manual/TELEGRAM.md).
- **Webhook genérico**: `NOTIFY_WEBHOOK_URL` (Discord/Slack/HTTP POST) com payload
  JSON do relatório; destinos privados exigem `ALLOW_PRIVATE_WEBHOOKS=true`.
- **Heartbeat**: `HEARTBEAT_URL` + `HEARTBEAT_ENABLED` (ações `start`/`success`/`fail`);
  monitore se o cron morreu em
  [`docs/manual/CLOUD_SESSIONS.md#9-heartbeat-dead-mans-switch`](docs/manual/CLOUD_SESSIONS.md#9-heartbeat-dead-mans-switch).

## Docker

Dois caminhos (detalhes em [`docs/manual/INSTALL_LINUX.md#7-docker`](docs/manual/INSTALL_LINUX.md#7-docker)):

```bash
# VPS de poucos recursos (recomendado): binário pré-compilado + imagem de runtime
cargo build --release -p ali-coins-cli
wrappers/build-runtime-image.sh              # ali-coins-rust:latest (segundos)

# Imagem completa (CI/validação do fonte)
DOCKER_BUILDKIT=1 docker build -t ali-coins-rust:full .
```

## Agendamento Diário (cron)

```bash
# 08:30 America/Sao_Paulo (11:30 UTC) — executa e registra o exit code no log
30 11 * * * /caminho/ali-coins-rust/run_all.sh --json >> /caminho/ali-coins-rust/cron.log 2>&1
```

Passo a passo (inclui o clássico "cron sem PATH/HOME"), rotação de logs e
heartbeat em [`docs/manual/CLOUD_SESSIONS.md`](docs/manual/CLOUD_SESSIONS.md).

## Desenvolvimento

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Fixtures de paridade contra o oráculo Node
./tools/parity/generate-fixtures.sh
./tools/parity/compare-dry-run.sh
./tools/parity/verify-rust-tokens.sh

# Smokes reais (opt-in)
ALI_COINS_CHROME=/caminho/chrome ./tools/smoke/run-cdp.sh
./tools/smoke/run-checkin-offline.sh
```

## Documentação

### Manuais

- [`docs/manual/INSTALL_LINUX.md`](docs/manual/INSTALL_LINUX.md) — instalação no Linux
  (Rust, Chromium, credenciais, cron e [troubleshooting](docs/manual/INSTALL_LINUX.md#6-resolução-de-problemas-frequentes-troubleshooting))
- [`docs/manual/CLOUD_SESSIONS.md`](docs/manual/CLOUD_SESSIONS.md) — VPS, delegação de
  sessão, cron, [heartbeat](docs/manual/CLOUD_SESSIONS.md#9-heartbeat-dead-mans-switch)
  e [logrotate](docs/manual/CLOUD_SESSIONS.md#10-higiene-e-rotação-de-logs-logrotate)
- [`docs/manual/TELEGRAM.md`](docs/manual/TELEGRAM.md) — bot, `CHAT_ID`, eventos e
  [troubleshooting](docs/manual/TELEGRAM.md#7-resolução-de-problemas-troubleshooting)
- [`docs/manual/RELEASING.md`](docs/manual/RELEASING.md) — versionamento, tags e artefatos
- [Índice dos manuais](docs/manual/README.md)

### Técnica

- [`docs/01-avaliacao.md`](docs/01-avaliacao.md) — avaliação do projeto Node
- [`docs/02-arquitetura.md`](docs/02-arquitetura.md) — arquitetura do port
- [`docs/03-roadmap.md`](docs/03-roadmap.md) — fases e critérios de aceite
- [`docs/04-matriz-rastreabilidade.md`](docs/04-matriz-rastreabilidade.md) — contratos e testes
- [`docs/05-divergencias-conhecidas.md`](docs/05-divergencias-conhecidas.md) — divergências e flexibilizações
- [`docs/06-validacao-real.md`](docs/06-validacao-real.md) — evidências das execuções com conta real
- [`docs/07-fase6-observacao.md`](docs/07-fase6-observacao.md) — janela de observação Node × Rust
- [`docs/security.md`](docs/security.md) — política de segurança
- [ADRs](docs/adr/README.md) — decisões de arquitetura
