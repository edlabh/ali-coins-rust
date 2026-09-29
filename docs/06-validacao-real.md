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

Lacuna remanescente (Fase 4): a **execução das tarefas** continua conservadora
— o runner não porta `libs/tasks/{verifier,surprise,dispatcher,search,state}`
do oráculo, então as tarefas não são creditadas (`missões=0`, status
`Falhou/Pendente`). Evidência comparativa no mesmo dia: o oráculo (conta
`ag***`) completou 7 de 8 tarefas visíveis (`+51 moedas`), incluindo busca,
super discounts (3 rodadas), coupons e sponsored items.
