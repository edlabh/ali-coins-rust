# Fase 6 — Observação paralela Node × Rust

**Janela proposta:** 2026-10-01 a 2026-10-14 (14 dias), com os dois crons
intactos:

| Projeto | Cron | Horário | Log |
|---|---|---|---|
| Node (oráculo) | `30 9 * * * /home/ubuntu/ali-coins/docker-run.sh` | 09:30 UTC (06:30 BRT) | `/home/ubuntu/ali-coins/cron.log` |
| Rust | `30 11 * * * /home/ubuntu/ali-coins-rust/run_all.sh --json` | 11:30 UTC (08:30 BRT) | `/home/ubuntu/ali-coins-rust/cron.log` |

O Rust roda ~2 h depois do Node, então **a conta já teve o check-in do dia
coletado** quando o Rust executa: o esperado é o Rust reportar
`already_collected` no check-in e executar as tarefas que o Node deixou (quando
houver), com o saldo final lido do extrato desktop.

## Como registrar cada dia

No log do Rust, a linha de fechamento agora traz o exit code:

```bash
ssh ubuntu@136.248.83.112 \
  'tail -80 /home/ubuntu/ali-coins-rust/cron.log | grep -E "exit=|Extrato desktop|Notificação|STREAK"'
```

No log do Node:

```bash
ssh ubuntu@136.248.83.112 \
  'tail -300 /home/ubuntu/ali-coins/cron.log | grep -E "exit=|Saldo Total Atualizado|Notificação Telegram"'
```

## Baseline (2026-09-30)

| Métrica | Node | Rust |
|---|---|---|
| Execução do dia | OK (`exit=0`, 1m29s, saldo 3217 moedas às ~16:07 UTC) | **Cron travou** às 11:44 (container removido 12:03, sem relatório). Correção de timeouts CDP aplicada depois; **re-execução manual OK** (12m03s, 0 erros, saldo 1118, check-in +20 / tarefas +46) |
| Binário na VM | — | atualizado 2026-09-30 19:26 UTC (multi-conta/trace opt-in; cron usa o fluxo de conta única) |
| Pendências do dia | — | `run_all.sh` agora grava `[run_all.sh] exit=N fim: ...` no `cron.log` |

> **Nota importante (01/10):** os dois crons usam **contas diferentes** — o Node
> roda `ag***@gmail.com` e o Rust roda `edelanoali@gmail.com`. Cada sistema
> coleta o **próprio** check-in diário (o Rust não encontra "já coletado" do
> Node). A comparação da janela é de **comportamento/estabilidade/paridade
> funcional**, não de saldo entre contas.
>
> Observação do dia de incidente (30/09): os valores de saldo do Node (3217) e
> do Rust (1118) são de execuções/hosts diferentes; a comparação válida começa
> em 2026-10-01, com os dois crons regulares.

## Planilha de acompanhamento

| Data | Node exit | Node saldo | Rust exit | Rust saldo | Check-in (Node/Rust) | Tarefas (Node/Rust) | Divergências |
|---|---|---|---|---|---|---|---|
| 2026-10-01 | 0 (12m23s) | 3321 (+91: check-in +40 / tarefas +51) | 0 (12m22s) | 1130 (+12: check-in +1 / tarefas +11) | Node: coletou próprio; Rust: `alreadyCollected=false` (coletou próprio) | Node: `Daily check-in +1`; Rust: 3 tarefas app-only desativadas, +11 | Nenhuma; ambos `exit=0`, sem travas |
| 2026-10-02 | | | | | | | |
| 2026-10-03 | | | | | | | |
| 2026-10-04 | | | | | | | |
| 2026-10-05 | | | | | | | |
| 2026-10-06 | | | | | | | |
| 2026-10-07 | | | | | | | |
| 2026-10-08 | | | | | | | |
| 2026-10-09 | | | | | | | |
| 2026-10-10 | | | | | | | |
| 2026-10-11 | | | | | | | |
| 2026-10-12 | | | | | | | |
| 2026-10-13 | | | | | | | |
| 2026-10-14 | | | | | | | |

## Critérios de encerramento (tag `v2.0.0` e desligamento do Node)

1. 14 dias consecutivos com `exit=0` (ou `exit=2` justificado por "sem ação
   nova") no Rust.
2. Saldo final do Rust igual ao saldo do extrato desktop no fim da execução.
3. Check-in: Rust sempre `already_collected` (Node roda antes) e streak
   consistente com o Node; nenhum `STREAK QUEBRADO` espúrio.
4. Tarefas: zero falhas de frame/loop; contagem de ações compatível com o que o
   Node deixou pendente.
5. Nenhum travamento do cron (toda execução termina e grava `exit=`).
6. Relatório final de paridade anexado a `docs/06-validacao-real.md` e tag
   `v2.0.0` publicada; só então o cron do Node é desativado (com confirmação do
   operador).

## Fora de escopo da janela

- **D-10** (surpresa): segue pendente por decisão do operador (host atual não
  registra os toques; o oráculo tem o mesmo comportamento aqui).
- Multi-conta e trace CDP: opt-in (`--all` / `PW_TRACE`), não usados pelo cron.
