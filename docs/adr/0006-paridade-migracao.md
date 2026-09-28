# ADR-0006: Estratégia de migração — incremental com oráculo Node

**Status**: Accepted (2026-09-28)
**Data**: 2026-09-28

## Context

São 15,9k linhas de código + 16,9k de testes, com cobertura fraca justamente no ponto mais arriscado (nada é testado contra o DOM real do AliExpress). Uma reescrita "big bang" só seria validada em produção, com risco de quebrar streaks e credenciais de usuários reais. O projeto tem 166 commits e um CHANGELOG ativo — vai continuar evoluindo durante a migração.

## Decision Drivers

- Zero regressão em comportamento observável (exit codes, relatórios, sessões, notificações).
- Validação contínua a cada fase, não só no final.
- Custo de manter duas implementações deve ser limitado no tempo.

## Considered Options

### Opção 1: Reescrita big bang
- Prós: sem código duplicado.
- Contras: 3–4 meses sem feedback real; risco máximo concentrado no corte; impossível isolar regressões.

### Opção 2: Migração incremental tipo "strangler" com oráculo Node — **escolhida**
- Prós: cada fase entrega valor verificável; o Node continua sendo a referência executável; corte final com evidência de paridade.
- Contras: duas implementações coexistem durante ~3 meses; exige harness de comparação.

### Opção 3: Congelar o Node e só migrar depois
- Prós: menos sincronização.
- Contras: projeto upstream ativo; congelar na prática significa manter dois backports.

## Decision

Migrar de forma incremental, em 6 fases (ver `docs/03-roadmap.md`), mantendo o clone Node como **oráculo executável** até o corte final:
1. Toda fase tem critérios de aceite mensuráveis e rollback.
2. Comportamento de referência é **congelado na v1.7.1** (commit `c05bf07`); mudanças do upstream entram como itens explícitos de sincronização, nunca silenciosamente.
3. Testes de contrato comparam Node × Rust (normalizando timestamps/durações) para C-01…C-20.
4. O corte final exige janela de execução em paralelo com **zero divergências** por 14 dias.

## Rationale

O único jeito de provar que "o bot continua coletando moedas" sem testes E2E do site real é executar as duas implementações lado a lado, com o mesmo estado, e comparar resultados. O harness de comparação é mais barato que qualquer tentativa de replicar o DOM do AliExpress em teste.

## Consequences

### Positive
- Risco distribuído; regressões localizadas na fase que as introduziu.
- Usuários podem optar pela versão Rust por subcomando/wrapper antes do corte.
- Fixtures geradas do Node (tokens, relatórios, meta) passam a ser um ativo de teste.

### Negative
- ~3 meses de manutenção dupla para correções críticas que afetem paridade.
- Harness de comparação precisa de cuidado com não-determinismo (tempo, moedas, ordem de tarefas do site).

### Risks
- **Divergência de ambiente do site**: variabilidade real do AliExpress pode mascarar diferenças. Mitigação: fixtures gravadas + janelas de teste lado a lado no mesmo período.
- **Drift do upstream**: mitigação por congelamento + triagem mensal do CHANGELOG.
- **Fadiga de paridade**: tempo gasto polindo diferenças irrelevantes (ex.: texto de log humano). Mitigação: classificar cada contrato como **rígido** (C-01, C-05…C-10, C-12, C-14) ou **flexível** (formato de log humano, mensagens internas).

## Implementation Notes

- Harness: `tools/parity/` com scripts que rodam Node e Rust sobre o mesmo `credentials.env`/fixtures e geram `diff` normalizado + exit code.
- Tipos de teste: (a) unitário puro; (b) contrato Node×Rust; (c) golden/fixture; (d) smoke real controlado (fase 3+).
- Não usar contas reais em CI: smoke real só manual/opt-in, com flag explícita.

## Related Decisions

- ADR-0001 (oráculo sidecar), ADR-0003/0004/0005 (contratos testáveis).
