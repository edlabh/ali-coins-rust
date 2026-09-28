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
