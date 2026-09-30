# ali-coins-rust

Port em Rust do [ali-coins](https://github.com/edlabh/ali-coins) (Node.js + Playwright):
automação de check-in diário e tarefas de moedas do AliExpress com emulação mobile
(Google Pixel 7) via CDP (`chromiumoxide`).

O projeto Node original é mantido em `reference/ali-coins/` como **oráculo de
paridade** (não versionado) e o comportamento é validado contra ele por fixtures
geradas em `tools/parity/`.

## Estado atual

| Área | Status |
|---|---|
| Núcleo (config, cripto v1/v2/v3, sessão, lock, relatórios, notificações) | ✅ com paridade em fixtures |
| Driver de browser (launch, emulação, bloqueio de recursos, diagnósticos, storage) | ✅ smoke real validado |
| **Check-in** (`checkin`) | ✅ **validado com conta real** |
| Tarefas (`tasks`) | 🔧 runner conservador (claims/busca/navegação); extração em alinhamento |
| Divergências conhecidas | `docs/05-divergencias-conhecidas.md` |
| Avaliação/arquitetura/roadmap | `docs/` |

## Início rápido

```bash
# 1. Compilar
cargo build --release

# 2. Credenciais (0600, ignorado pelo git)
cp credentials.env.example credentials.env   # ajuste ALI_USER/ALI_PASSWORD/SESSION_SECRET
chmod 600 credentials.env

# 3. Validar sem abrir navegador
ali-coins --dry-run --json

# 4. Executar
ali-coins all                # check-in + tarefas (notificação única; recomendado)
ali-coins checkin            # apenas o check-in diário
ali-coins tasks              # apenas o painel "Ganhe mais moedas"
./wrappers/run_all.sh        # execução unificada com retentativa (usa `all`)
```

### CLI

| Comando | Descrição |
|---|---|
| `ali-coins --dry-run [--json]` | Valida configuração sem navegador |
| `ali-coins all [--json] [--force] [--account <id>] [--no-delay] [--all]` | Check-in + tarefas numa execução (relatório e notificação únicos); `--all` roda todas as contas em sequência |
| `ali-coins checkin [--json] [--force] [--account <id>]` | Check-in diário |
| `ali-coins tasks [--json] [--force]` | Tarefas diárias |
| `ali-coins notify-test` | Envia uma mensagem de teste no Telegram |
| `ali-coins export-session [--all] [--account <id>] [--show-token] [--rotate] [--new-secret-from-env <VAR>]` | Exporta sessão cifrada (token v3); `--rotate` gira a chave at-rest |
| `ali-coins import-session [--all] [--from-file <path>] [--plaintext] [--keep-tokens] [--migrate]` | Importa sessão (stdin/arquivo ≤ 2 MiB); `--migrate` converte `session.json` legado em `.enc` |

### Códigos de saída

`0` sucesso · `1` falha · `2` sem ação/já coletado · `3` lock ativo ·
`4` streak quebrado · `5` 2FA não-interativo · `6` crash

## Docker (experimental)

> ⚠️ **Experimental.** Compilar dentro de um container pode levar **horas** em hosts com
> poucos recursos (1 vCPU / ~1 GB de RAM) e ser morto por falta de memória. **Prefira o
> binário direto** (`cargo build --release -p ali-coins-cli`), pelo menos no início; veja
> `docs/manual/INSTALL_LINUX.md` e `docs/manual/CLOUD_SESSIONS.md`.

```bash
docker build -t ali-coins-rust .
docker run --rm --init --shm-size=256m \
  -v "$PWD/credentials.env:/app/credentials.env:ro" \
  -v "$PWD/session.json.enc:/app/session.json.enc" \
  -v "$PWD/session_meta.json:/app/session_meta.json" \
  -v "$PWD/scratch:/app/scratch" \
  ali-coins-rust checkin --json
```

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

- `docs/manual/INSTALL_LINUX.md` — instalação no Linux (Rust, Chromium, primeiros passos)
- `docs/manual/CLOUD_SESSIONS.md` — VPS, cron, export/import de sessão, Docker e low-memory
- `docs/manual/TELEGRAM.md` — bot, configuração e testes de notificação
- `docs/manual/RELEASING.md` — versionamento, tags e artefatos

### Técnica

- `docs/01-avaliacao.md` — avaliação do projeto Node
- `docs/02-arquitetura.md` — arquitetura do port
- `docs/03-roadmap.md` — fases e critérios de aceite
- `docs/05-divergencias-conhecidas.md` — divergências e flexibilizações
- `docs/06-validacao-real.md` — evidências das execuções com conta real
