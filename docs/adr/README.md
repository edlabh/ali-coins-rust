# Architecture Decision Records — ali-coins-rust

Índice das decisões da migração de `edlabh/ali-coins` (Node.js + Playwright) para Rust.

| ADR | Título | Status | Data |
|---|---|---|---|
| [0001](0001-driver-browser.md) | Driver de browser: CDP via `chromiumoxide` com trait de abstração | Accepted | 2026-09-28 |
| [0002](0002-workspace-crates.md) | Workspace em 4 crates (core, browser, flows, cli) com tokio | Accepted | 2026-09-28 |
| [0003](0003-cli-config-compat.md) | Compatibilidade de CLI/env: binário único com subcomandos e parser de env fiel | Accepted | 2026-09-28 |
| [0004](0004-cripto-sessao.md) | Cripto/sessão: interoperabilidade bit a bit com o token v3 do Node | Accepted | 2026-09-28 |
| [0005](0005-observabilidade-logs.md) | Logs/observabilidade: `tracing` JSON + redaction própria; formato humano aproximado | Accepted | 2026-09-28 |
| [0006](0006-paridade-migracao.md) | Estratégia de migração: incremental com oráculo Node e testes de contrato | Accepted | 2026-09-28 |
| [0007](0007-operacao-paralela-permanente.md) | Operação paralela permanente: Node e Rust mantidos em repositórios separados (sem corte) | Accepted | 2026-10-06 |

## Processo

1. Copiar um ADR existente, renomear para `NNNN-titulo.md` e preencher.
2. Status possível: `Proposed` → `Accepted` → `Deprecated` / `Superseded` / `Rejected`.
3. ADRs aceitos não são editados: decisões novas geram ADR que substitui.
4. Toda decisão relevante da migração deve ter ADR antes de virar código (fase correspondente do roadmap).
