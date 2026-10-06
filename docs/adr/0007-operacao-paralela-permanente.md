# 0007 — Operação paralela permanente (Node e Rust em repositórios separados)

- **Status**: Accepted
- **Data**: 2026-10-06
- **Contexto**: o projeto Node (`edlabh/ali-coins`) e o port Rust
  (`edlabh/ali-coins-rust`) são projetos independentes. Decisão do operador:
  **ambos permanecem ativos em caráter permanente**, em repositórios separados,
  sem corte ou descomissionamento do Node.

## Decisão

1. Os dois agendamentos continuam ativos na VM (Node `09:30 UTC`, Rust
   `11:30 UTC`), cada um na sua conta — operação **paralela permanente**.
2. Não existe "desligar o Node": a Fase 6 passa a ser **observação paralela +
   primeira release estável do port**, sem etapa de corte.
3. O clone do oráculo em `reference/` é **ferramenta de paridade** (fixado pelo
   commit registrado nas fixtures, `ORACLE_COMMIT`), não runtime do port.
4. Os contratos de paridade (`docs/01-avaliacao.md` §7) seguem como referência
   do port; o upstream pode evoluir por conta própria e a paridade é avaliada
   contra o commit pinado, com upgrades deliberados.
5. Os releases do port seguem SemVer próprio e são independentes dos releases
   do Node.

## Consequências

- **Positive**: sem custo/risco de corte; o Node permanece como referência viva
  e fallback natural; cada projeto evolui no seu ritmo.
- **Negative**: manutenção dos dois projetos é permanente (não há "economia"
  por desligamento); mudanças de site exigem atenção nos dois.
- **Mitigação**: heartbeat e auditoria de logs nos dois crons (o incidente de
  02–06/10 do cron comentado mostrou a importância do monitoramento); fixtures
  regeneradas apenas em upgrades deliberados do oráculo.

## Referências

- Fase 6 redefinida em `docs/03-roadmap.md`.
- Observação: `docs/07-fase6-observacao.md`.
