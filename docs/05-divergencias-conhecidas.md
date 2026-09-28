# Divergências conhecidas e decisões de flexibilização

Contratos classificados como **rígidos** (devem ser idênticos) ou **flexíveis**
(aproximações aceitas e documentadas), conforme ADR-0006.

## Flexíveis (aceitas)

| Item | Oráculo (Node) | Port (Rust) | Justificativa |
|---|---|---|---|
| Formato de log humano | `pino` + `pino-pretty` | `tracing` (formato próprio) | Nenhum consumidor depende do layout; o contrato é stdout/stderr + redaction (ADR-0005) |
| Mensagens de erro internas | Texto exato do Zod/pino | Texto PT-BR equivalente | Apenas mensagens de `config` são contrato (C-04); demais são diagnósticas |
| Gravação de vídeo Playwright | `recordVideo` | Degradação para trace+screenshot na v1 (decisão aprovada) | Custo alto no CDP (`Page.startScreencast`) |
| `PRINT`/emoji no Telegram | Idênticos | Idênticos (snapshot) | — |

## Bugs do oráculo replicados por paridade

| ID | Descrição | Evidência | Ação no port |
|---|---|---|---|
| D-01 | `CAPTCHA_COOLDOWN_HOURS` é **sempre 12**: a variável existe no schema (`config.js:144-150`), mas não é incluída no `rawEnv` (`config.js:636-682`). O valor do `credentials.env` é ignorado por `collect.js:357` e `all.js:62`. | `CAPTCHA_COOLDOWN_HOURS=6 node all.js --dry-run --json` → `captchaCooldownHours: 12` | Replicado em `Config` com comentário. Quando o upstream corrigir, ajustar `core::config` e o cenário `tuned` das fixtures. |

## Rígidos (cobertos por teste de paridade)

C-01…C-20 de `docs/01-avaliacao.md` §7, com exceção dos itens flexíveis acima.
