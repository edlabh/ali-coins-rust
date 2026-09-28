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

## Credenciais locais

- `credentials.env` criado com **0600** e ignorado pelo git (contém `ALI_USER`, `ALI_PASSWORD` e `SESSION_SECRET` gerado).
- Nenhum segredo foi impresso nos logs da validação.
