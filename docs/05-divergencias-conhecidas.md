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
| D-02 | Templates do Telegram agora têm **paridade byte-a-byte** com o oráculo nos eventos de produção (`dry_run`, `manual_test`, `lock_active`, `failure`, `streak_break`, `2fa_required`, `captcha_required`, `captcha_cooldown_released`) **e na mensagem consolidada multi-conta** (`buildMultiAccountMessage`), com `extractRelevantErrorMessage`/`sanitizeSensitiveQueryParams` portados e fixture de snapshot (`tools/parity/fixtures/common/notify.json`, 13 casos, teste `notify_interop`). | `libs/notify.js:450-853` | Concluído em 2026-09-30. Restante: galho de sessão importada no `failure`. |
| D-03 | **Trace CDP portado** (30/09/2026): `Tracing.start`/`Tracing.end` com coleta de `Tracing.dataCollected` e gravação em **JSON** `0600` (`scratch/<nome>-trace-<ts>.json`), com as mesmas regras do oráculo para `PW_TRACE` (default `off` em host de baixa memória, `retain-on-failure` no host normal; `on` mantém sempre; valor desconhecido inicia sem manter) e `PW_OUTPUT_DIR`. O **formato difere** do zip do Playwright (degradação aceita na v1); a paridade é de comportamento/habilitação. | `libs/ui/diagnostics.js:92-160` | Concluído em 2026-09-30, validado por testes unitários (`decide_trace`/`resolve_trace_mode`/gravação) e smoke `#[ignore]` com Chromium. |
| D-05 | `storage_state` porta o comportamento do Playwright: **todos os cookies** + localStorage de **todos os origins visitados** via CDP `DOMStorage` (lista compartilhada entre as páginas do browser, cobrindo mobile e desktop). | `browser.js` (`storageState`) | Concluído em 2026-09-30. |
| D-06 | `close_modals` foi substituído pela **lista exata do oráculo** (CSS + `:has-text`, filtro de diálogo/overlay, um round-trip CDP para os CSS e fallback por botão textual). | `libs/ui/navigation.js:59-164` | Concluído em 2026-09-29. |
| D-07 | `login` porta o **slider humanizado** (`trySolveSlider`: easing quadrático, jitter vertical, 25 frames com pausas de 10–19 ms e verificação por `detached`) e os **seletores `:has-text` completos** (Continue/Continuar/Sign in/Entrar), com as mesmas 4 chamadas do oráculo. | `libs/ui/login.js`, `libs/selectors.js:8-29` | Concluído em 2026-09-29. |
| D-08 | `checkin` porta a leitura desktop (saldo/bônus/missões), a **pré-checagem** com **reuso** (`shouldReuseEarlyDesktop`), a **coleta de água**, a **sincronização do saldo** quando o ledger atrasa e o **`resolveStreakDays`** (detectado/meta/early/extrato/confirmação por ledger). Relaitura de confirmação de quebra (`shouldConfirmStreakByStatement`) e alerta/exit 4 de streak quebrado no `all` **concluídos em 30/09**. | `collect.js:196-895` | Concluído em 2026-09-29; alinhamento contínuo na paridade com o site real. |
| D-09 | `tasks` porta status/prioridade/dispatcher **1:1** (`state.js`: keywords app-only, `findNextPendingTask`, rodadas/tentativas, `classifyTaskStatus`) e o laço do `do_tasks.js` (pausa opcional, teto de ações, segunda passada, relatório final). | `libs/tasks/state.js:10-25,271-332` | Concluído no incremento do motor de tarefas (2026-09-29); alinhamento contínuo de keywords/snapshots na paridade com o site real. |
| D-10 | Tarefa **"Browse surprise items"**: o port agora **reproduz o mecanismo do oráculo** — clique real de mouse no centro do card com **delay de 50 ms** e verificação de alvo (`elementFromPoint`, como a actionability do Playwright), **emissão de toque** (`Emulation.setEmitTouchEventsForMouse` mobile, como o contexto Playwright mobile), tratamento de **nova aba** (espera load/beacon + fecha), recuperação de navegação na mesma aba (`goBack`/`goto`), neutralização de overlays de tela cheia e diagnóstico de tracking por clique. Mesmo assim, **neste host** o site não registra os toques (`tracking` sem delta) e o oráculo Node também falha aqui; em **outro host o oráculo concluiu as duas rodadas** — indicando bloqueio do site/IP/estado da conta, não do algoritmo. | `libs/tasks/surprise.js`; logs de 30/09 (`Pós-clique: tracking=N->N`) | Validar o port no host onde o oráculo funciona (mesma conta/sessão); se necessário, comparar captura de rede entre hosts. |

## Rígidos (cobertos por teste de paridade)

C-01…C-20 de `docs/01-avaliacao.md` §7, com exceção dos itens flexíveis acima.
