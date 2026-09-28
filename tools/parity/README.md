# Harness de paridade Node × Rust

Ferramentas da estratégia de migração (ADR-0006): geram fixtures a partir do
oráculo Node (`reference/ali-coins`) e comparam o comportamento do binário Rust
com o Node, contrato a contrato (`docs/01-avaliacao.md` §7).

## Estrutura

```
tools/parity/
├── node/
│   ├── gen_tokens.mjs    # gera fixtures de cripto (v1/v2/v3 + tokens malformados)
│   └── normalize.mjs     # normaliza JSON (tempos/durações) para diff justo
├── fixtures/             # gerado localmente (não versionado por padrão)
│   ├── crypto/
│   ├── config/
│   └── reports/
├── generate-fixtures.sh  # roda os geradores Node
└── compare-dry-run.sh    # Node × Rust para `--dry-run [--json]`
```

## Pré-requisitos

- Node.js ≥ 22.
- Oráculo clonado em `reference/ali-coins` (veja o README do repositório).
- Rust: `cargo` em `~/.cargo/bin` (se não estiver no `PATH`:
  `export PATH="$HOME/.cargo/bin:$PATH"`).

## Uso

```bash
# 1. Gerar/atualizar fixtures a partir do oráculo
./tools/parity/generate-fixtures.sh

# 2. Comparar dry-run (fase 1+ exige o binário Rust)
./tools/parity/compare-dry-run.sh            # tolerante: avisa se o Rust ainda não implementa
./tools/parity/compare-dry-run.sh --require-rust
```

## Regras de normalização

- `startTime`/`endTime`/`savedAt`/`importedAt`/durations → placeholder `T`.
- Caminhos absolutos da máquina → `<ROOT>`.
- Moedas do site (saldo/streak) não são normalizadas; a comparação é por faixa
  quando a execução for real.
- Logs vão para stderr e não entram no diff de stdout.
