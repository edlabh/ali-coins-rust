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
| D-02 | Templates do Telegram cobrem os eventos e a mecânica (retry/truncamento/fallback), mas ainda **não têm paridade byte-a-byte** com os snapshots do oráculo. | `libs/notify.js:450-853` | Completar textos e adicionar fixture de snapshot (previsto no incremento de notificações/paridade). |
| D-03 | Diagnósticos em Rust implementam screenshot e DOM hash/dump, mas o **trace CDP** (zip do Playwright) ainda não foi portado. | `libs/ui/diagnostics.js:92-160` | Portar `Tracing.start/stop` com fixture de validação no incremento de diagnósticos avançados. |
| D-05 | `storage_state` cobre **todos os cookies**, mas o localStorage apenas do **origin atual** (o Playwright enumera todos os origins visitados). | `browser.js` (`storageState`) | Migrar para o domínio `DOMStorage` quando os fluxos multi-origin entrarem na fase 3. |
| D-06 | `close_modals` usa heurística de rótulos/classes; o oráculo tem lista exata de seletores com `:has-text`. | `libs/ui/navigation.js:59-164` | Alinhar seletores no incremento de paridade com o site real (check-in). |
| D-07 | `login` cobre preenchimento/submissão/2FA fail-fast/validação de cookies, mas o **slider humanizado** (`trySolveSlider`) e os seletores `:has-text` completos ainda não foram portados. | `libs/ui/login.js`, `libs/selectors.js:8-29` | Portar slider (easing/jitter/frames) na paridade com o site real. |
| D-08 | `checkin` não inclui a **pré-checagem desktop** (saldo/streak/ledger), a coleta de água e o parser completo do extrato; o saldo é lido por regex simplificada. | `collect.js:196-895` | Completar na paridade com o site real, reutilizando `core::report` para contabilidade. |
| D-09 | `tasks` porta status/prioridade/dispatcher, mas as **listas de palavras-chave** de app-only e detalhes de classificação serão alinhados 1:1 com `libs/tasks/state.js`. | `libs/tasks/state.js:10-25,271-332` | Alinhar keywords e snapshots de status na paridade com o site real. |

## Rígidos (cobertos por teste de paridade)

C-01…C-20 de `docs/01-avaliacao.md` §7, com exceção dos itens flexíveis acima.
