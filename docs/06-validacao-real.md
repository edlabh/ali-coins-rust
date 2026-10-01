# Validação real — check-in (2026-09-28)

Primeira execução do port Rust contra o AliExpress com conta real, a partir deste host (IP residencial).

## Evidências

| Run | Comando | Resultado | Exit |
|---|---|---|---|
| 1 | `ali-coins checkin --json` | Login in-page (e-mail → Continue → senha → botão visível) OK; `xman_us_t` presente; coleta confirmada | **0** |
| 2 | `ali-coins checkin --json` (sessão reaproveitada) | `alreadyCollected: true` — prova de que a coleta do run 1 foi registrada no site | **2** |

Relatório do run 2 (sanitizado):

```json
{
  "type": "unified_report",
  "checkin": { "alreadyCollected": true, "streakDays": 1 },
  "meta": { "checkinCoinsGained": 0, "totalCoinsGained": 0, "finalBalance": "N/D" }
}
```

Run 1: `coinsGainedToday: "10"` (Dia 1 do ciclo, +10 moedas — consistente com `streakDays: 1`).

## Correções que a validação real exigiu (`57b797c`)

1. **Latência do SPA**: o formulário in-page só aparece após um round-trip — espera por polling (15s) para usuário/senha, com fallback do botão Continue.
2. **Botões ocultos**: existem vários `button.cosmos-btn-primary`; clicar apenas no **primeiro visível** (`offsetParent`/bounding box) foi o que destravou o submit.
3. **Leitura**: saldo/streak vêm do `innerText` (o HTML não contém os rótulos).

## Pendências confirmadas

- **D-08**: saldo/streak/ledger ainda não vêm da página desktop (`mycoin`); no mobile o parser devolve `N/D`.
- **D-07**: slider anti-bot ainda não portado (não foi necessário nesta execução).
- Sessão persistida cifrada (`session.json.enc`, `ENCRYPT_LOCAL_SESSION` com `SESSION_SECRET`).

## Tasks — teste decisivo com o oráculo Node (2026-09-28)

Para separar "estado da conta" de "diferença de DOM", o bot Node original
(`do_tasks.js`) foi executado no mesmo host com a **mesma sessão exportada**:

| Métrica | Oráculo Node | Port Rust |
|---|---|---|
| Gaveta abre | ✅ | ✅ (`.e2e_task`) |
| Itens `.e2e_normal_task` | ✅ 15 ações | ❌ conteúdo preso no `common-loading-icon` |
| Ganho do dia | +56 moedas (ledger), saldo 981 | 0 (exit 2 "sem ação") |

**Conclusão**: as tarefas existem para a conta; a diferença é de fluxo/DOM. O
oráculo executa 15 ações, incluindo "Coupons & shopping credits for you!",
"Browse surprise items" (3 cards), "Items at $0.1" e navegação com scroll.

Próximos passos para alinhar o runner Rust:
1. Instrumentar a abertura da gaveta (dump do DOM **no momento em que o Node vê
   as tarefas**) para comparar com `scratch/tasks-drawer.html`.
2. Replicar a ordem exata do `verifier`/`dispatcher` (clicar o botão de tarefas
   correto, aguardar a resposta da API antes do polling de itens).
3. Revalidar preferencialmente em outro dia ou com tarefas pendentes, já que o
   run do oráculo concluiu as tarefas do dia.

## Credenciais locais

- `credentials.env` criado com **0600** e ignorado pelo git (contém `ALI_USER`, `ALI_PASSWORD` e `SESSION_SECRET` gerado).
- Nenhum segredo foi impresso nos logs da validação.

## Atualização (tarde de 2026-09-28)

- Corrigido no Rust: `wait_for_selector` agora exige **visibilidade** (semântica
  `state=visible` do Playwright) e o extrator ignora itens invisíveis — a gaveta
  estava sendo considerada aberta num container escondido (`85732de`).
- Após essa correção, novas execuções ao vivo passaram a falhar com
  **`falha de navegação: Request timed out`** no `goto` da página de moedas.
  Como o oráculo Node já havia concluído as tarefas do dia (+56 moedas) e houve
  muitas execuções no mesmo IP/sessão, a hipótese principal é **throttling/risco
  do site**, não uma regressão do port (navegação idêntica funcionava minutos antes).
- Próxima janela de validação: **outro dia** (ou com tarefas pendentes),
  repetindo o runner com a correção de visibilidade; se persistir, comparar o
  DOM ao vivo com o dump do oráculo (`scratch/tasks-drawer.html`).

## Execução unificada na VM em Docker — `all` (noite de 2026-09-28)

Primeira execução completa do comando **`all`** (check-in + tarefas num processo,
relatório e notificação únicos) na VM, em modo Docker:

- `./run_all.sh --json` → **exit 0**; etapas `1m 06s` (check-in) e `1m 47s`
  (tarefas) com duração total `2m 56s` no payload.
- Relatório: `checkin.alreadyCollected=true`, `streakDays=1`;
  `meta.tasksCoinsGained=0`, `meta.finalBalance="N/D"`.
- **Uma única notificação Telegram** enviada, no formato do oráculo
  (`ℹ️ ali-coins — DD/MM/AAAA`, conta mascarada, host `vm-ali-rust (v0.1.0)`,
  ganhos, sequência, saldo e duração) — commit `470e1c3`.
- Correções que a validação exigiu nesta janela: args do chromiumoxide sem
  `--` duplicado (`a82e77e`), `goto` sem depender da resposta do
  `Page.navigate` (`89f10df`) e mensagem unificada no core (`fdb0546`).

Pendências confirmadas em produção:

1. `meta.finalBalance` continua `N/D` — o parser de saldo do check-in (D-08) não
   encontra o texto na página mobile atual; sem saldo não há diferença para
   medir o ganho das tarefas (`tasksCoinsGained=0`).
2. Os status de tarefas (`Concluída (n/2)` / `Falhou (limite de 4 tentativas)`)
   são registrados no relatório normalmente.

## Extrato desktop em produção — saldo, bônus e missões (manhã de 2026-09-29)

Implementação da leitura desktop (`mycoin.html`) com as mesmas regexes do
oráculo (`libs/ui/balance.js::getBalanceDesktop`) e uso do **ledger como fonte
de verdade** para o relatório e a notificação:

- `all` ao vivo na VM (`11:57–12:04 UTC`, conta `edelanoali@gmail.com`):

  | Leitura | Valor | Cross-check |
  |---|---|---|
  | Saldo | `996` | primeiro valor real após o `N/D` |
  | Bônus (check-in) | `+15` | UI mobile do dia: `2 day streak` / `Today ✓ 15` |
  | Missões (tarefas) | `0` | nenhuma tarefa creditada nesta execução |
  | Sequência | `2` | igual à UI mobile |

- Relatório: `checkin.alreadyCollected=true`, `checkin.coinsGainedToday=15`,
  `checkin.checkinCoinsFromLedger=true`, `meta.finalBalance="996 moedas"`,
  `meta.totalCoinsGained=15` — o crédito real do dia entra no relatório mesmo
  com o check-in já coletado (regra do oráculo).
- Telegram: `🪙 Ganhas hoje: +15 moedas (check-in +15 / tarefas +0)` ·
  `💰 Saldo: 996 moedas` · `📅 Sequência: 2 dias`.
- Fluxos cobertos: `checkin` (leitura pós-check-in), `all` (pós-check-in e
  pós-tarefas) e `tasks` (saldo final + missões).

## Motor de tarefas portado — execução ao vivo (2026-09-29, 12:27–12:36 UTC)

Port de `libs/tasks/{state,verifier,dispatcher,search,prizeland,surprise}.js`
e do laço de `do_tasks.js` (`tasks.rs`, `tasks_verifier.rs`,
`tasks_dispatcher.rs`, `tasks_surprise.rs`, `tasks_runner.rs`). Execução
`all` na VM (Docker), conta `edelanoali@gmail.com`:

| Tarefa | Resultado Rust | Resultado oráculo (conta `ag***`, mesmo dia) |
|---|---|---|
| Explore sponsored items | `Concluída (2/2)` | `Concluída (2/2)` |
| Browse recently viewed items | `Concluída` | `Concluída` |
| View your "Coins Savings Recap" | `Concluída` | `Concluída` |
| View Super discounts | `Concluída (3/3)` | `Concluída (3/3)` |
| Search for what you love | `Concluída` | `Concluída` |
| Coupons & shopping credits | `Concluída` | `Concluída` |
| Browse surprise items | `Falhou (sem progresso após 3 tentativas)` | `Falhou (sem progresso após 3 tentativas)` |
| Items $0.1 / Merge Boss / Daily quiz | `Desativada (SKIP_APP_ONLY_TASKS)` | `Desativada (SKIP_APP_ONLY_TASKS)` |

- **Moedas**: missões do extrato `0 → 46`; saldo `996 → 1042`;
  `meta.tasksCoinsGained=46`, `totalCoinsGained=61` (check-in +15).
- **Exit 0**; etapas `1m 34s` (check-in) e `8m 21s` (tarefas).
- Uma única notificação Telegram com `🪙 +61 moedas (check-in +15 / tarefas +46)`
  e `💰 Saldo: 1042 moedas`.
- Divergência residual: a tarefa "Daily check-in" não apareceu na extração
  final (na execução anterior aparecia); investigar na próxima janela.

### Investigação da tarefa "Browse surprise items" (tarde de 2026-09-29)

A tarefa passou a ser **executada**: o feed é a grade de produtos da própria
página de moedas (`.feeds-discount-card`), com 18–19 cards visíveis; o fluxo
passou a:

- filtrar apenas cards com retângulo válido (o feed é virtualizado,
  `inscene-outside`);
- tocar o overlay `.product-click` com **toque real** de touch
  (`Input.synthesizeTapGesture`), 3s por card, e com fallback de mouse/JS;
- detectar/reabrir o feed quando o toque abre o detalhe (mesma aba) e fechar
  abas novas, com tetos de tempo por operação CDP.

Mesmo assim o contador de rodadas do card (`0/2`) **não avança** — e o
oráculo Node falhou a mesma tarefa no mesmo dia com status idêntico
(`Falhou (sem progresso após 3 tentativas)`).

Rodadas seguintes de investigação (29/09, tarde):

- Clique **trusted** no GO (mouse real) — a gaveta continua aberta
  (`Pós-GO: gaveta_aberta=true`) e o conteúdo dela entra em **loading**.
- Fechamento explícito da gaveta (elemento de fechar/ESC/canto do painel +
  neutralização do overlay) passou a funcionar: `Gaveta fechada: true |
  elemento no centro do card: product-click` — ou seja, os toques agora
  alcançam o overlay do card.
- Ainda assim, 3–4 toques touch reais em cards distintos e visíveis não movem
  o contador `0/2`. Evidências: logs `Feed de surpresas: cards=19` + toques
  sem `avançou de rodada`; screenshot `scratch/surprise-after-tap.png`.
- Hipóteses em aberto (D-10): aguardar o fim do loading da gaveta pós-GO antes
  de tocar; exigir abertura do detalhe do item (beacon no carregamento); ou um
  feed dedicado aberto pelo GO que o `find_changed_page` esteja ignorando.

Experimentos realizados (29/09, fim do dia), ambos sem mover o contador `0/2`:

- **(a) esperar o loading pós-GO:** o loading termina (1–3s) e o conteúdo é a
  própria lista de tarefas — não existe UI de "modo tocar" para aguardar.
- **(b) clique de mouse (abre o detalhe) com orçamento de 5 min:** o clique
  navega para o detalhe, mas a recuperação (`goBack`/`goto`) às vezes cai no
  layout PC (`coin-pc-index`), os tempos por toque passam de 30s e a rodada
  continua `0/2`.

Próximo passo recomendado: **captura de rede (CDP `Network`)** durante o toque
para identificar a chamada que o site espera nessa tarefa (a contabilização
parece server-side), ou uma execução do oráculo em dia em que ele conclua a
tarefa para comparar os requests. Observação: o oráculo vem falhando nessa
tarefa nos últimos dias — pode ser mudança do site, não do port.

**Decisão (29/09): aguardar o oráculo voltar a concluir a tarefa antes de
novos experimentos.**

## Atraso inicial e `--dry-run` pós-subcomando (2026-09-29)

Paridade com `all.js`/`config.js`:

- `START_DELAY_MIN_MS`/`START_DELAY_MAX_MS` (0/0 = desligado) agora são
  aplicados no comando `all`, **antes do lock/navegador**, com a mesma janela
  aleatória e a mesma mensagem do oráculo; `--no-delay` pula (manuais e
  retentativas do `run_all.sh`) e `--dry-run` nunca atrasa.
- `all --dry-run [--json]` (e `checkin`/`tasks --dry-run`) agora executam o
  dry-run global em vez do fluxo real — mesmo contrato do healthcheck do
  Docker do oráculo.
- Validação na VM: `all --dry-run --json` → exit 0 com `dryRun: true`;
  janela fixa de 15s → `Início atrasado em 15s (janela 15–15s; início previsto
  às 11:13:16 PDT)`; com `--no-delay` a mensagem não aparece.

## Paridade fina do check-in (D-08) — concluída (2026-09-29)

Port de `collect.js`: pré-checagem desktop antes do mobile, reuso da leitura
quando nada foi coletado (`shouldReuseEarlyDesktop`), coleta de água, sincronização
do saldo quando o ledger atrasa e `resolveStreakDays` (móvel + meta anterior +
early desktop + extrato + confirmação por ledger).

Validação ao vivo (`checkin` avulso e `all` na VM):

- `checkin --json` → `Extrato desktop: saldo=1042 bônus=15 missões=46 streak=2`
  (pré-checagem), **"Saldo/streak reutilizados da checagem inicial do desktop"**
  e relatório `alreadyCollected=true`, `coinsGainedToday=15`, `streakDays=2`,
  `totalBalance=1042` (exit 0 pelo crédito do dia no extrato).
- `all --json` → etapas `1m 13s` (check-in) e `7m 35s` (tarefas);
  `meta`: `checkinCoinsGained=15`, `tasksCoinsGained=46`,
  `totalCoinsGained=61`, `finalBalance="1042 moedas"`; notificação única enviada.

## D-06/D-07 — `close_modals` exato e slider humanizado (2026-09-29)

- `close_modals` porta a lista exata do oráculo (CSS + `:has-text`), com filtro
  de diálogo/overlay e um único round-trip CDP para os seletores CSS.
- `login` ganhou o **slider humanizado** (`trySolveSlider`: easing quadrático,
  jitter vertical, 25 frames com pausas de 10–19 ms, verificação por `detached`)
  com primitivas de mouse reais no driver (`mouse_move/down/up`) e recebeu os
  seletores `:has-text` completos (Continue/Continuar/Sign in/Entrar), com as
  4 chamadas do oráculo no fluxo.
- Testes: `slider_trajectory` (easing/jitter/monotonicidade) e
  `split_has_text`; 45 testes do crate de fluxos verdes.
- Regressão ao vivo (`all` na VM): 6 tarefas `Concluída`, surprise ainda
  `Falhou (sem progresso…)` (D-10 parado), `meta` idêntico
  (`+61 moedas`, saldo `1042`) e duração total `6m 45s`.

## Incidente e correção — CDP pendurado em página lenta (30/09/2026)

- **Sintoma**: o `all` agendado (11:30 UTC) travou após ~14 min de execução
  (último log às 11:44, container ainda "up" 33 min depois, sem novos logs).
- **Causa**: com o site lento/throttlado (navegações de 20–46 s e container do
  projeto Node rodando no mesmo IP), uma chamada CDP (`Runtime.evaluate` /
  consultas de elemento / storage) ficou sem resposta e **sem timeout**,
  bloqueando o laço de tarefas (que só limita o despacho por tarefa, não as
  leituras de DOM).
- **Correção**: todas as chamadas CDP do driver passaram a ter teto —
  `eval`/mouse/tap/scroll/`go_back`/screenshot em **20 s**, consultas de
  elemento em **4 s**, storage/perfil em **30 s** — retornando erro de timeout
  em vez de pendurar. `wait_for_selector` agora respeita o deadline mesmo com
  consultas lentas.
- **Validação**: CI verde no commit `8c22396`; reexecução ao vivo na VM
  concluiu em **12m 03s** (etapas `1m 18s` + `10m 02s`), **0 erros**, sem
  locks órfãos, 6 tarefas `Concluída` (surprise segue `Falhou…`, D-10).
- **Observação de paridade**: a mensagem do dia saiu `check-in +0 / tarefas
  +46` porque a leitura inicial do desktop (reutilizada) não trouxe a seção do
  extrato (`ledger disponível: false`) naquele momento; o oráculo tem o mesmo
  atalho de reuso e o mesmo comportamento nesse cenário.

## Valores do extrato na mensagem (30/09/2026)

Ajuste para que a mensagem exiba **sempre** o valor real creditado no extrato
desktop, tanto no check-in quanto nas tarefas (como o oráculo):

- `all`: o bônus do check-in usa a leitura pós-check-in **ou** a pós-tarefas
  (a seção do dia pode não ter renderizado na primeira leitura reutilizada);
  as missões das tarefas vêm do extrato **quando há lançamentos**
  (`todayMissionsCount > 0`, igual ao `do_tasks.js`), senão cai na diferença de
  saldo.
- `checkin`: se a leitura escolhida (inclusive a reutilizada) não trouxe a
  seção do dia, faz **uma leitura fresca** para exibir o valor real.
- Evidência da execução ao vivo (30/09): `meta.checkinCoinsGained=20`,
  `meta.tasksCoinsGained=46`, `meta.totalCoinsGained=66`,
  `finalBalance="1108 moedas"`; o check-in foi coletado nesta execução
  (`alreadyCollected=false`, `streakDays 3→4`) e o saldo pós-check-in foi
  sincronizado (`1128`) enquanto o extrato não refletia o crédito.

## Fase 5 — hardening do CI (01/10/2026)

- **Auditoria bloqueante**: `cargo audit` sem `|| true` (0 vulnerabilidades; aviso
  de versão yanked apenas), com ferramenta pré-compilada via `taiki-e/install-action`.
- **Cobertura**: job `coverage` com `cargo llvm-cov --workspace --summary-only` e
  gate progressivo em **61%** — medido em 01/10: **61,74% de linhas** (14.473
  linhas, 8.936 cobertas). Meta de 80% registrada como pendência: as maiores
  lacunas são o glue de CLI (`run_all`/`run_checkin`/`run_tasks`, que dependem de
  browser real) e o CDP/tarefas DOM (smoke `#[ignore]`).
- **SBOM**: job `sbom` gera CycloneDX JSON (`cargo cyclonedx --format json`) e
  publica `*.cdx.json` como artefato do workflow.
- **Dependabot**: `.github/dependabot.yml` para `cargo`, `github-actions` e
  `docker` (semanal, PRs agrupados para minor/patch).
- **Dockerfile**: `ARG CARGO_BUILD_JOBS=1` e `CARGO_PROFILE_RELEASE_LTO=false`
  por padrão, permitindo build da imagem na VPS de baixa memória.
- **Imagem própria no runner da VM (01/10)**: `ali-coins-rust:latest` (201 MB) gerada a
  partir do binário pré-compilado (`Dockerfile.runtime` + `wrappers/build-runtime-image.sh`);
  `docker-run.sh`/`run.sh` da VM usam a imagem com `--entrypoint /data/ali-coins` (binário
  montado) e `ENTRYPOINT ["ali-coins"]` na imagem. Tempos medidos na VPS (1 vCPU):
  binário incremental **5m47s**, imagem de runtime **26s** (primeira) e **4s** (seguinte);
  o build completo (CI) usa `codegen-units=16`, `LTO=false` e cache do BuildKit.
  O smoke do CI foi corrigido para não mascarar o exit code do container (`| head`).

## Recuperação da página principal nas tarefas (01/10/2026)

A execução do cron de 01/10 mostrou a gaveta de tarefas inacessível **após** a
tarefa "Browse surprise items" (o site não registrou os toques — D-10), com
"painel de tarefas fechado ou não detectado" até encerrar a etapa. As tarefas
executadas antes (check-in +1, sponsored +5+5) concluíram; o relatório ficou só
com as 3 tarefas app-only desativadas porque a extração final da gaveta falhou
(mesma lógica do oráculo nesse cenário).

Correções aplicadas:

- **`ensure_main_page`** (port fiel de `ensureMainPage`): recria a página
  principal quando ela está **fechada** (com device profile Pixel 7 e navegação
  para a central mobile) e navega para a central quando a URL não é dela.
- **`get_drawer_tasks_with_retry` devolve a página ativa**; o runner passa a
  substituir o handle quando a original fecha (antes o retry repetia no mesmo
  estado, sem recuperação).
- **Diagnóstico em falha da gaveta**: URL, título, trecho do corpo e screenshot
  `0600` em `scratch/` — explica a causa na próxima ocorrência.
- **Proteção extra**: `close_new_tabs`/`close_orphan_pages` nunca fecham páginas
  da central de moedas (o matching por URL podia fechar a página principal).
- **Testes**: 2 novos no crate de flows (página fechada recriada na central;
  página viva com URL errada navegada sem recriar) — 47 testes no crate,
  workspace e clippy verdes.

## D-02 residual — sessão importada expirada (01/10/2026) — validado com fixtures

Fechado o último item funcional de notificação: o `failure` de conta única passa a
exibir o bloco **"Aviso de Sessão Remota"** nas mesmas condições do oráculo.

- `check_imported_session_expired` (puro, em `core::notify::telegram`): flags no
  erro/relatório/por conta → regex `/sessão.*(importada|remota).*expir/i` ou
  `/node export_session\.js/i` → fallback pelo `session_meta.json`
  (`isImported`/`importedAt` + erro de autenticação), desligado em multi-conta.
- Produtores replicados (`collect.js`/`do_tasks.js`): `detect_imported_session_expired`
  marca sessão importada + erro de login/navegação; `run_multi` grava o flag por
  conta no payload; `tasks` avulso agora notifica falha/2FA/captcha como o `do_tasks.js`.
- **Fixtures**: `notify.json` subiu de 13 para **17 casos** (flag estruturada, regex,
  `export_session.js` e multi-conta com conta importada) — `notify_interop` verde
  byte-a-byte contra o oráculo.
- Testes unitários: `check`/produtor/aviso no `failure` + leitura do meta.

## D-03 — trace CDP (30/09/2026) — validado com testes e smoke opt-in

Implementado o trace CDP (`Tracing.start/end`) com as regras do oráculo para
`PW_TRACE` (default `off` em host de baixa memória, `retain-on-failure` no host
normal), retenção por modo/erro e arquivo `0600` em `PW_OUTPUT_DIR` (padrão
`scratch/`), com o **mesmo nome** `<nome>-trace-<epoch_ms>` mas extensão `.json`
(trace nativo do CDP; o zip do Playwright fica como divergência flexível).

- Puro/testável: `decide_trace`, `resolve_trace_mode`, `diagnostics_dir`,
  `trace_file_name` e gravação `0600` (5 testes no crate de browser).
- Smoke `#[ignore]` `grava_trace_cdp_em_json` (requer Chromium; rodar com
  `--ignored`) valida o ciclo real `start → navegação → stop` com eventos.
- Ligado no fluxo `all` atrás de `PW_TRACE`; **na VM o default é desligado**
  (`CHROMIUM_LOW_MEMORY` ligado), então o cron não captura trace por padrão.

## Multi-conta sequencial (30/09/2026) — validado apenas com mocks

Implementado o modo multi-conta do `all` (paridade com o fluxo multi do
`all.js`), **sem nenhuma execução em contas reais** (decisão do operador):

- Orquestração pura e testável: `MultiFlags` (sucesso/ação nova/streak/2FA/
  captcha/lock/falhas), seleção de evento e exit code agregados (3 todas
  bloqueadas, 4 streak, 5 2FA, 1 falha, 2 sem ação, 0 sucesso).
- Execução: contas em sequência, cada uma em processo filho
  (`all --account <user> --json`, isolamento de lock/sessão/browser do fluxo
  de conta única), com backoff exponencial com jitter + pausa aleatória
  (`ACCOUNT_DELAY_MIN/MAX_MS`) composta pela maior espera.
- Agregação: `build_multi_account_report_payload` + mensagem consolidada
  **byte-a-byte** com o oráculo (`buildMultiAccountMessage`; fixtures
  `multi_success`, `multi_failure`, `multi_already_collected`).
- Testes (sem contas reais): flags/event/exit, sequência e isolamento do loop
  com runner fake, extração do relatório do stdout e agregação do payload.

## D-05 — localStorage multi-origin (30/09/2026)

`storage_state` agora rastreia os origins http(s) visitados por **qualquer
página** do browser (lista compartilhada no `CdpBrowserHandle`) e lê/grava o
localStorage por origin com CDP `DOMStorage` — como o `context.storageState()`
do Playwright (cobre `m.aliexpress.com` e `www.aliexpress.com` no mesmo run).
Cada chamada CDP segue com teto de tempo.

## Heartbeat na CLI (30/09/2026)

O comando `all` passou a enviar o **dead man's switch** como o `all.js`:
`start` (após o lock), `success` (com o payload do relatório), `fail` (lock
ativo, erro de lock e falhas gerais) e `fail` no streak quebrado antes do
exit 4. Envio best-effort, respeitando `HEARTBEAT_ENABLED`/`HEARTBEAT_URL`
(na VM não está configurado — sem impacto no cron atual).

## D-02/D-08 — Telegram byte-a-byte e streak quebrado (30/09/2026)

- **D-02**: eventos de produção do Telegram portados byte-a-byte
  (`dry_run`, `manual_test`, `lock_active`, `failure`, `streak_break`,
  `2fa_required`, `captcha_required`, `captcha_cooldown_released`) com
  `extractRelevantErrorMessage` e `sanitizeSensitiveQueryParams` fiéis.
  Fixture de snapshot gerada do oráculo (`gen_notify.mjs`, relógio congelado)
  e teste `notify_interop` comparando as 10 mensagens byte-a-byte.
- **D-08**: releitura de confirmação de quebra de streak
  (`shouldConfirmStreakByStatement`) quando a tela lê 1 com histórico > 1 e o
  extrato ainda não veio; `all` agora detecta a quebra (`is_streak_break`),
  envia a mensagem dedicada `🚨 STREAK QUEBRADO` e sai com **exit 4** (como o
  `all.js`).
- Falhas de 2FA/captcha passam a enviar os eventos dedicados (não mais o
  `failure` genérico).

## Tarefa de surpresa — reprodução do mecanismo do oráculo (30/09/2026)

O oráculo concluiu as duas rodadas da tarefa **em outro host**, então o port
passou a reproduzir o mecanismo dele em `tasks_surprise.rs`:

1. **Clique real de mouse** no centro do card (`.feeds-discount-card`) com
   **delay de 50 ms** entre press/release — como `card.click({ delay: 50 })` —
   precedido de `scrollIntoView` e **verificação de alvo** (`elementFromPoint`
   precisa devolver o card/descendente; caso contrário tenta de novo).
2. **Emissão de toque**: `Emulation.setEmitTouchEventsForMouse(mobile)` no
   perfil de device (o Playwright usa isso em contextos mobile), para que o
   clique real chegue ao handler de "tap" do site.
3. **Nova aba primeiro**: se o clique abrir a página do item em nova aba,
   espera o load curto + 1 s (beacon), fecha a aba e mantém a feed intacta.
4. **Mesma aba**: `goBack` com espera e, se necessário, `goto(feed)`; depois
   aguarda os cards voltarem (até 45 s).
5. **Overlays**: antes dos toques, neutraliza gaveta/máscaras/overlays de tela
   cheia que interceptam o ponto de clique.
6. **Diagnóstico**: cada clique loga `Pós-clique: url=… cards=… tracking=N->M`
   (requests de tracking/carregamento antes/depois), expondo se o handler do
   site reagiu.

**Resultado neste host**: mesmo com o mecanismo fiel, os cliques **não geram
tracking** (`42->42`) e a rodada não avança; o oráculo Node também falha neste
host e conclui em outro. Evidência de que o bloqueio é do site/IP/estado da
conta neste ambiente, não do algoritmo. Próximo passo: validar o port no host
onde o oráculo funciona.

## Fase 1 fechada — `--rotate`/`--migrate` e relatório em texto (2026-09-29)

- `export-session --rotate [--all] [--account <id>] [--new-secret-from-env=VAR]`
  (port de `rotateSessionSecret`): chave antiga = `SESSION_SECRET_OLD` (ou
  `SESSION_SECRET`); nova = env do `VAR` (padrão `SESSION_SECRET_NEW`) ou
  `SESSION_SECRET` quando há `SESSION_SECRET_OLD`; backup versionado em
  `scratch/session.bak-*.json.enc`, re-cifra com roundtrip antes de gravar,
  remove o texto claro e atualiza `lastRotatedAt`/`encrypted` no meta.
- `import-session --migrate [--all] [--account <id>] [--json]` (port de
  `migrateLegacySession`): migra `session.json` legado para `.enc` com schema
  validado, preserva `.enc` existente como backup e só remove o texto claro
  após verificação do roundtrip.
- Relatório em texto (sem `--json`): `all` imprime o **RELATÓRIO CONSOLIDADO
  FINAL** (conta, sequência, check-in, tarefas com moedas, total do dia, saldo,
  horários e durações); `checkin` e `tasks` imprimem seus resumos próprios.
- Validação na VM (diretório temporário, sem tocar a sessão real):
  `import-session --migrate --json` → `{cookiesCount:1, migrated:true,
  encrypted:true}`; `export-session --rotate` com `SESSION_SECRET_NEW` →
  `[SUCESSO] Rotação de chave concluída` + backup versionado.
