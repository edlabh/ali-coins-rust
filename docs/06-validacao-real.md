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
